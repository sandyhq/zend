use serde_json::Value;
use std::collections::HashMap;
use zend_proto::{AgentEvent, DoneStatus, ToolCall, ToolDiff};

#[derive(Default)]
pub(super) struct Normalizer {
    text: HashMap<usize, String>,
    thinking: HashMap<usize, String>,
    tools: HashMap<String, (String, Value)>,
    pub error: Option<String>,
    pub aborted: bool,
    pub window: Option<u64>,
}
impl Normalizer {
    pub fn reset(&mut self) {
        self.error = None;
        self.aborted = false;
        self.text.clear();
        self.thinking.clear();
    }
    pub fn status(&self) -> DoneStatus {
        if self.aborted {
            DoneStatus::Interrupted
        } else if self.error.is_some() {
            DoneStatus::Errored
        } else {
            DoneStatus::Completed
        }
    }
    pub fn map(&mut self, frame: &Value) -> Vec<AgentEvent> {
        let mut events = Vec::new();
        match string(frame, "type") {
            "message_start" if frame["message"]["role"] == "assistant" => {
                self.text.clear();
                self.thinking.clear();
            }
            "message_update" => {
                let update = &frame["assistantMessageEvent"];
                let index = update["contentIndex"].as_u64().unwrap_or(0) as usize;
                let delta = string(update, "delta");
                match string(update, "type") {
                    "text_delta" => {
                        self.text.entry(index).or_default().push_str(delta);
                        events.push(AgentEvent::TextDelta { text: delta.into() });
                    }
                    "thinking_delta" => {
                        self.thinking.entry(index).or_default().push_str(delta);
                        events.push(AgentEvent::ReasoningDelta { text: delta.into() });
                    }
                    _ => {}
                }
            }
            "message_end" if frame["message"]["role"] == "assistant" => {
                let message = &frame["message"];
                self.error = (message["stopReason"] == "error").then(|| {
                    message["errorMessage"]
                        .as_str()
                        .unwrap_or("Pi provider failed")
                        .to_owned()
                });
                self.aborted = message["stopReason"] == "aborted";
                if let Some(content) = message["content"].as_array() {
                    for (index, part) in content.iter().enumerate() {
                        let (cache, value, reasoning) = match string(part, "type") {
                            "text" => (&mut self.text, string(part, "text"), false),
                            "thinking" => (&mut self.thinking, string(part, "thinking"), true),
                            _ => continue,
                        };
                        let previous = cache.entry(index).or_default();
                        if let Some(suffix) = value
                            .strip_prefix(previous.as_str())
                            .filter(|s| !s.is_empty())
                        {
                            events.push(if reasoning {
                                AgentEvent::ReasoningDelta {
                                    text: suffix.into(),
                                }
                            } else {
                                AgentEvent::TextDelta {
                                    text: suffix.into(),
                                }
                            });
                        }
                        *previous = value.into();
                    }
                }
                let usage = &message["usage"];
                if usage.is_object() {
                    let n = |key| usage[key].as_u64().unwrap_or(0);
                    let input = n("input") + n("cacheRead") + n("cacheWrite");
                    events.push(AgentEvent::Usage {
                        input_tokens: input,
                        output_tokens: n("output"),
                    });
                    events.push(AgentEvent::ContextUsage {
                        tokens: Some(input + n("output")),
                        window: self.window,
                    });
                }
            }
            "tool_execution_start" => {
                let id = string(frame, "toolCallId").to_owned();
                let name = string(frame, "toolName");
                if !self.tools.contains_key(&id) {
                    let args = frame["args"].clone();
                    events.push(AgentEvent::ToolCall {
                        id: id.clone(),
                        call: tool(name, &args),
                    });
                    self.tools.insert(id, (name.into(), args));
                }
            }
            "tool_execution_end" => {
                let id = string(frame, "toolCallId").to_owned();
                let info = self.tools.remove(&id);
                let result = &frame["result"];
                let output = result["content"].as_array().map(|items| {
                    items
                        .iter()
                        .filter_map(|v| v["text"].as_str())
                        .collect::<Vec<_>>()
                        .join("\n")
                });
                let diff = info.as_ref().and_then(|(name, args)| {
                    if frame["isError"] == true {
                        return None;
                    }
                    match name.as_str() {
                        "edit" => edit_diff(args, &result["details"]),
                        "write" => Some(ToolDiff {
                            path: string(args, "path").into(),
                            old_text: None,
                            new_text: cap(string(args, "content"), 65536),
                        }),
                        _ => None,
                    }
                });
                // The first picture the result gave the agent (a read of a
                // PNG), for its row: the engine keeps its bytes, bounded,
                // and journals only their file's name.
                let image = result["content"].as_array().and_then(|items| {
                    items.iter().find(|v| v["type"] == "image" && v["data"].is_string())
                });
                if let Some(image) = image.filter(|_| frame["isError"] != true) {
                    let data = string(image, "data");
                    if data.len() <= zend_proto::TOOL_IMAGE_MAX_BYTES.div_ceil(3) * 4 {
                        events.push(AgentEvent::ToolImage {
                            id: id.clone(),
                            mime_type: string(image, "mimeType").into(),
                            data: Some(data.into()),
                            stored: None,
                        });
                    }
                }
                events.push(AgentEvent::ToolResult {
                    id,
                    is_error: frame["isError"].as_bool().unwrap_or(false),
                    output: output.map(|s| cap(&s, 16384)),
                    diff,
                    exit_code: None,
                });
            }
            "compaction_end" => {
                if let Some(tokens) = frame["result"]["estimatedTokensAfter"].as_u64() {
                    events.push(AgentEvent::ContextUsage {
                        tokens: Some(tokens),
                        window: self.window,
                    });
                }
            }
            // A throwing extension handler is the extension's failure, not
            // the turn's: the model run goes on, so it is shown, not fatal.
            "extension_error" => {
                // Name the extension and event: with several loaded, the bare
                // error text does not say which one threw.
                let path = string(frame, "extensionPath");
                let event = string(frame, "event");
                let origin = match (path.is_empty(), event.is_empty()) {
                    (true, true) => String::new(),
                    (false, true) => format!(" ({path})"),
                    (true, false) => format!(" ({event})"),
                    (false, false) => format!(" ({path}, {event})"),
                };
                events.push(AgentEvent::Error {
                    message: format!("Pi extension{origin}: {}", string(frame, "error")),
                });
            }
            _ => {}
        }
        events
    }
}
pub(super) fn string<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap_or("")
}
fn cap(text: &str, limit: usize) -> String {
    if text.len() <= limit {
        return text.into();
    }
    let mut at = limit;
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    format!("{}\n… [truncated]", &text[..at])
}
/// An edit's diff, from what Pi says it changed: its own numbered diff
/// (`details.diff`, the file's lines round each change), else the blocks
/// it was asked to replace (`edits: [{oldText, newText}]`), else an older
/// Pi's single `oldText` / `newText`. `None` when none of them says
/// anything (a diff of two empty texts would count nothing and show
/// nothing).
fn edit_diff(args: &Value, details: &Value) -> Option<ToolDiff> {
    let path = string(args, "path").to_owned();
    let (old, new) = details["diff"]
        .as_str()
        .and_then(numbered_diff_texts)
        .or_else(|| {
            let (old, new) = edit_texts(args)?;
            (!old.is_empty() || !new.is_empty()).then_some((old, new))
        })?;
    Some(ToolDiff {
        path,
        old_text: Some(cap(&old, 65536)),
        new_text: cap(&new, 65536),
    })
}

/// Pi's `edit` input as one old and one new text: each block of
/// `edits` (today's schema) one after the other, a blank line between, or
/// an older Pi's `oldText` / `newText`.
fn edit_texts(args: &Value) -> Option<(String, String)> {
    if let Some(edits) = args["edits"].as_array().filter(|e| !e.is_empty()) {
        let join = |key: &str| {
            edits
                .iter()
                .map(|e| string(e, key).trim_end_matches('\n'))
                .collect::<Vec<_>>()
                .join("\n\n")
        };
        return Some((join("oldText"), join("newText")));
    }
    let old = args["oldText"].as_str();
    let new = args["newText"].as_str();
    (old.is_some() || new.is_some())
        .then(|| (old.unwrap_or("").to_owned(), new.unwrap_or("").to_owned()))
}

/// The file before and after, as far as Pi's numbered diff shows it: each
/// line is its sign (` `, `-`, `+`), the line's number padded, a space and
/// its text; a gap between hunks is `...` alone. Context goes into both,
/// a removed line into the old, an added into the new, a gap into both as
/// `⋯`. `None` when it isn't such a diff.
fn numbered_diff_texts(diff: &str) -> Option<(String, String)> {
    let (mut old, mut new) = (Vec::new(), Vec::new());
    let mut changed = false;
    for line in diff.lines() {
        let mut chars = line.chars();
        let sign = chars.next()?;
        let rest = chars.as_str().trim_start();
        if rest == "..." {
            old.push("⋯");
            new.push("⋯");
            continue;
        }
        let digits = rest.len() - rest.trim_start_matches(|c: char| c.is_ascii_digit()).len();
        if digits == 0 {
            return None;
        }
        let text = rest[digits..].strip_prefix(' ').unwrap_or(&rest[digits..]);
        match sign {
            ' ' => {
                old.push(text);
                new.push(text);
            }
            '-' => {
                old.push(text);
                changed = true;
            }
            '+' => {
                new.push(text);
                changed = true;
            }
            _ => return None,
        }
    }
    changed.then(|| (old.join("\n") + "\n", new.join("\n") + "\n"))
}

fn tool(name: &str, args: &Value) -> ToolCall {
    match name {
        "bash" => ToolCall::Exec {
            command: string(args, "command").into(),
        },
        "read" => ToolCall::ReadFile {
            path: string(args, "path").into(),
        },
        "write" => ToolCall::WriteFile {
            path: string(args, "path").into(),
            content: args["content"].as_str().map(str::to_owned),
        },
        "edit" => {
            let (old, new) = edit_texts(args).unzip();
            ToolCall::EditFile {
                path: string(args, "path").into(),
                old_string: old,
                new_string: new,
            }
        }
        "grep" => ToolCall::Search {
            pattern: string(args, "pattern").into(),
            path: args["path"].as_str().map(str::to_owned),
        },
        "find" => ToolCall::Glob {
            pattern: string(args, "pattern").into(),
        },
        name if name.starts_with("zend_") => ToolCall::Mcp {
            server: "zend".into(),
            tool: name["zend_".len()..].into(),
            input: Some(args.clone()),
        },
        _ => ToolCall::Unknown {
            name: name.into(),
            input: Some(args.clone()),
        },
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn final_message_reconciles_without_duplicate_text_and_retry_clears_error() {
        let mut n = Normalizer::default();
        n.map(&json!({"type":"message_update","assistantMessageEvent":{"type":"text_delta","contentIndex":0,"delta":"hel"}}));
        let events = n.map(&json!({"type":"message_end","message":{"role":"assistant","stopReason":"error","errorMessage":"temporary","content":[{"type":"text","text":"hello"}]}}));
        assert!(matches!(&events[0], AgentEvent::TextDelta {text} if text == "lo"));
        assert_eq!(n.status(), DoneStatus::Errored);
        n.map(&json!({"type":"message_end","message":{"role":"assistant","stopReason":"stop","content":[]}}));
        assert_eq!(n.status(), DoneStatus::Completed);
    }
    #[test]
    fn extension_errors_show_without_failing_the_turn() {
        let mut n = Normalizer::default();
        let events = n.map(&json!({"type":"extension_error","extensionPath":"/x.ts","event":"tool_call","error":"boom"}));
        assert!(
            matches!(&events[0], AgentEvent::Error {message} if message == "Pi extension (/x.ts, tool_call): boom")
        );
        assert_eq!(n.status(), DoneStatus::Completed);
    }
    #[test]
    fn tools_preserve_identity_and_edit_diff() {
        let mut n = Normalizer::default();
        let start = json!({"type":"tool_execution_start","toolCallId":"t","toolName":"edit","args":{"path":"a","oldText":"old","newText":"new"}});
        assert_eq!(n.map(&start).len(), 1);
        assert!(n.map(&start).is_empty());
        let events=n.map(&json!({"type":"tool_execution_end","toolCallId":"t","isError":false,"result":{"content":[{"type":"text","text":"ok"}]}}));
        assert!(!events.iter().any(|e| matches!(e, AgentEvent::ToolImage { .. })));
        assert!(
            matches!(&events[0],AgentEvent::ToolResult{id,diff:Some(diff),..} if id=="t" && diff.new_text=="new")
        );
    }

    /// Today's Pi edits by blocks (`edits`) and answers its own numbered
    /// diff (`details.diff`): the diff is that one, its context in both
    /// texts, so the row counts and shows what really changed; without
    /// details, the blocks themselves. (Before, both texts were empty: Pi
    /// no longer sends `oldText` at the top.)
    #[test]
    fn a_pi_edit_by_blocks_diffs_from_its_own_numbered_diff_or_its_blocks() {
        let args = json!({"path":"a.md","edits":[{"oldText":"two\n","newText":"2\n"},{"oldText":"nine","newText":"9"}]});
        let details = json!({"diff":"    ...\n  1 one\n- 2 two\n+ 2 2\n  3 three\n    ...\n- 9 nine\n+ 9 9"});
        let diff = edit_diff(&args, &details).unwrap();
        assert_eq!(diff.old_text.as_deref(), Some("⋯\none\ntwo\nthree\n⋯\nnine\n"));
        assert_eq!(diff.new_text, "⋯\none\n2\nthree\n⋯\n9\n");
        let diff = edit_diff(&args, &json!({})).unwrap();
        assert_eq!((diff.old_text.as_deref(), diff.new_text.as_str()), (Some("two\n\nnine"), "2\n\n9"));
        // The call keeps the blocks too, as its input.
        assert!(matches!(tool("edit", &args),
            ToolCall::EditFile { old_string: Some(o), new_string: Some(n), .. } if o == "two\n\nnine" && n == "2\n\n9"));
        // Nothing said: no diff, rather than an empty one.
        assert!(edit_diff(&json!({"path":"a"}), &json!({})).is_none());
        assert!(edit_diff(&json!({"path":"a"}), &json!({"diff":"not a diff"})).is_none());
        // Through the normalizer, as Pi's frames come.
        let mut n = Normalizer::default();
        n.map(&json!({"type":"tool_execution_start","toolCallId":"e","toolName":"edit","args":args}));
        let events = n.map(&json!({"type":"tool_execution_end","toolCallId":"e","isError":false,
            "result":{"content":[{"type":"text","text":"Successfully replaced 2 block(s) in a.md."}],"details":details}}));
        assert!(matches!(&events[0], AgentEvent::ToolResult { diff: Some(d), .. } if d.new_text.contains("three")));
    }

    /// A read of a picture: Pi's result carries the image the model was
    /// given (`{type:"image",data,mimeType}`); it goes on, before the
    /// result, for the engine to keep. Not after a failure.
    #[test]
    fn a_reads_picture_goes_on_with_its_result() {
        let mut n = Normalizer::default();
        let end = |id: &str, is_error: bool| json!({"type":"tool_execution_end","toolCallId":id,"isError":is_error,
            "result":{"content":[{"type":"text","text":"Read image file [image/png]"},
                {"type":"image","data":"iVBORw0KGgo=","mimeType":"image/png"}]}});
        n.map(&json!({"type":"tool_execution_start","toolCallId":"r","toolName":"read","args":{"path":"/home/me/.cache/sandy-phys/m1.png"}}));
        let events = n.map(&end("r", false));
        assert!(matches!(&events[0], AgentEvent::ToolImage { id, mime_type, data: Some(d), stored: None }
            if id == "r" && mime_type == "image/png" && d == "iVBORw0KGgo="), "{events:?}");
        assert!(matches!(&events[1], AgentEvent::ToolResult { id, output: Some(o), .. }
            if id == "r" && o == "Read image file [image/png]"));
        n.map(&json!({"type":"tool_execution_start","toolCallId":"f","toolName":"read","args":{"path":"x.png"}}));
        assert!(!n.map(&end("f", true)).iter().any(|e| matches!(e, AgentEvent::ToolImage { .. })));
    }

    #[test]
    fn zend_tools_keep_their_whole_name() {
        assert!(matches!(tool("zend_whoami", &json!({})),
            ToolCall::Mcp { server, tool, .. } if server == "zend" && tool == "whoami"));
        assert!(matches!(tool("zend_create_chats", &json!({})),
            ToolCall::Mcp { server, tool, .. } if server == "zend" && tool == "create_chats"));
    }
}
