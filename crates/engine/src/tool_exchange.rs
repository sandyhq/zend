//! `ReadToolExchange {chatId, toolId}`: one tool's call and result, read
//! from the run journal of the device that ran it, for its row opened in
//! the transcript (`ReadToolImageChunk {chatId, toolId, offset}`: the
//! picture a read named).
//!
//! The chat's doc keeps little of a tool ([`zend_doc::sanitize_tool_call`]:
//! a call's short identifiers, a line of its result); its whole input and
//! result stay in the host's journal. A reader who opens a row asks the
//! host, which answers from there, made safe to leave it as a command's
//! whole output is ([`crate::tool_output`]):
//!
//! - every string of the input redacted ([`redact::redact_output`]) and cut
//!   to [`TOOL_EXCHANGE_INPUT_STRING_MAX_BYTES`]; the value of a key naming
//!   a secret ([`redact::input_key_is_secret`]) replaced; the whole input at
//!   most [`TOOL_EXCHANGE_INPUT_MAX_BYTES`] of JSON;
//! - its result redacted as a command's is ([`redact::redact_command_output`];
//!   a command's through [`zend_doc::command_output_end`], so one that
//!   prints secrets by nature answers only the withheld note), cut to
//!   [`TOOL_EXCHANGE_OUTPUT_MAX_BYTES`]: a command's end, another tool's
//!   start;
//! - a file whose contents are secrets by its name (`.env`, a key:
//!   [`redact::file_holds_secrets`]) read, written or edited: nothing of its
//!   contents (`withheld`);
//! - a diff whose sides fit [`TOOL_EXCHANGE_DIFF_MAX_BYTES`], redacted.
//!
//! A read of a picture says its type (`image`); its bytes come from
//! `ReadToolImageChunk`, from the path the journal recorded (never one a
//! client names), a PNG, JPEG, WebP or GIF whose first bytes say so
//! ([`crate::uploads::read_image_file_chunk`]).

use std::path::PathBuf;

use serde_json::Value;
use zend_proto::{
    TOOL_EXCHANGE_DIFF_MAX_BYTES, TOOL_EXCHANGE_INPUT_MAX_BYTES,
    TOOL_EXCHANGE_INPUT_STRING_MAX_BYTES, TOOL_EXCHANGE_OUTPUT_MAX_BYTES, ToolCall, ToolDiff,
    ToolExchangeReply, redact,
};

use crate::run_journal::{RunJournal, ToolResultRecord};
use crate::tool_output::{ToolOutputError, journal_chat};

/// Tool `tool_id` of doc `doc_id` (a chat, or a subagent's doc in it), as
/// a row opened shows it.
pub fn read(
    journal: &RunJournal,
    doc_id: &str,
    tool_id: &str,
) -> Result<ToolExchangeReply, ToolOutputError> {
    let exchange = journal
        .tool_exchange(journal_chat(doc_id), tool_id)?
        .ok_or(ToolOutputError::NotAvailable)?;
    Ok(reply(&exchange.call, exchange.result.as_ref()))
}

/// The picture tool `tool_id` read: its path, made absolute against the
/// chat's folder `cwd` when the harness named it relative.
pub fn image_path(
    journal: &RunJournal,
    doc_id: &str,
    tool_id: &str,
    cwd: Option<&str>,
) -> Result<PathBuf, ToolOutputError> {
    let exchange = journal
        .tool_exchange(journal_chat(doc_id), tool_id)?
        .ok_or(ToolOutputError::NotAvailable)?;
    let ToolCall::ReadFile { path } = &exchange.call else {
        return Err(ToolOutputError::NotAPicture);
    };
    if image_type(path).is_none() {
        return Err(ToolOutputError::NotAPicture);
    }
    let path = PathBuf::from(crate::repos::expand_home(path.trim()).map_err(|_| ToolOutputError::NotAPicture)?);
    if path.is_absolute() {
        return Ok(path);
    }
    let cwd = cwd
        .and_then(|cwd| crate::repos::expand_home(cwd).ok())
        .map(PathBuf::from)
        .filter(|cwd| cwd.is_absolute())
        .ok_or(ToolOutputError::NotAPicture)?;
    Ok(cwd.join(path))
}

/// The reply for a call and (once settled) its result.
pub fn reply(call: &ToolCall, result: Option<&ToolResultRecord>) -> ToolExchangeReply {
    let withheld = withholds(call);
    let image = match call {
        ToolCall::ReadFile { path } => image_type(path).map(str::to_owned),
        _ => None,
    };
    let (output, output_trimmed) = match result.and_then(|r| r.output.as_deref()) {
        None => (None, false),
        Some(text) if text.trim().is_empty() => (None, false),
        Some(_) if withheld => (Some(redact::SECRET_OUTPUT_WITHHELD.to_owned()), false),
        Some(text) => {
            let (text, trimmed) = shown_output(call, text);
            (Some(text).filter(|t| !t.is_empty()), trimmed)
        }
    };
    let diff = if withheld {
        None
    } else {
        result.and_then(|r| r.diff.as_ref()).and_then(shown_diff)
    };
    ToolExchangeReply {
        call: shown_call(call, withheld),
        done: result.is_some(),
        output,
        output_trimmed,
        is_error: result.is_some_and(|r| r.is_error),
        exit_code: result.and_then(|r| r.exit_code),
        diff,
        image,
        withheld,
    }
}

/// A picture's type by its file's name.
fn image_type(path: &str) -> Option<&'static str> {
    let ext = path.trim().rsplit_once('.')?.1.to_ascii_lowercase();
    match ext.as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "webp" => Some("image/webp"),
        "gif" => Some("image/gif"),
        _ => None,
    }
}

/// Whether what the tool touched is a secret by nature.
fn withholds(call: &ToolCall) -> bool {
    match call {
        ToolCall::ReadFile { path }
        | ToolCall::WriteFile { path, .. }
        | ToolCall::EditFile { path, .. } => redact::file_holds_secrets(path),
        ToolCall::ApplyPatch { path: Some(path), .. } => redact::file_holds_secrets(path),
        ToolCall::Exec { command } => redact::command_prints_secrets(command),
        _ => false,
    }
}

/// The call with its input made safe to show.
fn shown_call(call: &ToolCall, withheld: bool) -> ToolCall {
    let text = |s: &Option<String>| {
        if withheld {
            None
        } else {
            s.as_deref().map(|s| cut_start(&redact::redact_output(s), TOOL_EXCHANGE_INPUT_STRING_MAX_BYTES).0)
        }
    };
    match call {
        ToolCall::WriteFile { path, content } => ToolCall::WriteFile {
            path: path.clone(),
            content: text(content),
        },
        ToolCall::EditFile {
            path,
            old_string,
            new_string,
        } => ToolCall::EditFile {
            path: path.clone(),
            old_string: text(old_string),
            new_string: text(new_string),
        },
        ToolCall::WebFetch { url, prompt } => ToolCall::WebFetch {
            url: url.clone(),
            prompt: text(prompt),
        },
        ToolCall::Mcp {
            server,
            tool,
            input,
        } => ToolCall::Mcp {
            server: server.clone(),
            tool: tool.clone(),
            input: input.as_ref().map(shown_input),
        },
        ToolCall::Unknown { name, input } => ToolCall::Unknown {
            name: name.clone(),
            input: input.as_ref().map(shown_input),
        },
        other => other.clone(),
    }
}

/// A tool's input made safe to show: strings redacted and cut, secret
/// keys' values replaced; past [`TOOL_EXCHANGE_INPUT_MAX_BYTES`] of JSON,
/// its pretty JSON's start as one string.
pub fn shown_input(input: &Value) -> Value {
    fn walk(value: &Value) -> Value {
        match value {
            Value::String(s) => Value::String(
                cut_start(&redact::redact_output(s), TOOL_EXCHANGE_INPUT_STRING_MAX_BYTES).0,
            ),
            Value::Array(items) => Value::Array(items.iter().map(walk).collect()),
            Value::Object(map) => Value::Object(
                map.iter()
                    .map(|(k, v)| {
                        let v = if redact::input_key_is_secret(k) && !v.is_object() && !v.is_array() && !v.is_null() {
                            Value::String("[redacted]".into())
                        } else {
                            walk(v)
                        };
                        (k.clone(), v)
                    })
                    .collect(),
            ),
            other => other.clone(),
        }
    }
    let shown = walk(input);
    let json = serde_json::to_string(&shown).unwrap_or_default();
    if json.len() <= TOOL_EXCHANGE_INPUT_MAX_BYTES {
        return shown;
    }
    let pretty = serde_json::to_string_pretty(&shown).unwrap_or(json);
    Value::String(cut_start(&pretty, TOOL_EXCHANGE_INPUT_MAX_BYTES).0)
}

/// A result's text as it may leave the host, and whether it was cut.
fn shown_output(call: &ToolCall, text: &str) -> (String, bool) {
    if let ToolCall::Exec { command } = call {
        return match zend_doc::command_output_end(command, text, usize::MAX, TOOL_EXCHANGE_OUTPUT_MAX_BYTES) {
            Some(end) => (end.text, end.truncated),
            None => (String::new(), false),
        };
    }
    let redacted = redact::redact_command_output(text);
    cut_start(redacted.trim_end(), TOOL_EXCHANGE_OUTPUT_MAX_BYTES)
}

/// A diff as it may leave the host: both sides redacted alike (so their
/// lines still pair up); none when a side is past the cap.
fn shown_diff(diff: &ToolDiff) -> Option<ToolDiff> {
    let old = diff.old_text.as_deref().unwrap_or("");
    if old.len() > TOOL_EXCHANGE_DIFF_MAX_BYTES || diff.new_text.len() > TOOL_EXCHANGE_DIFF_MAX_BYTES {
        return None;
    }
    if old.is_empty() && diff.new_text.is_empty() {
        return None;
    }
    Some(ToolDiff {
        path: diff.path.clone(),
        old_text: diff.old_text.as_deref().map(redact::redact_output),
        new_text: redact::redact_output(&diff.new_text),
    })
}

/// `text`'s first `max` bytes, ending on a whole line where one ends inside
/// them (on a character otherwise), and whether anything was cut.
fn cut_start(text: &str, max: usize) -> (String, bool) {
    if text.len() <= max {
        return (text.to_owned(), false);
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    if let Some(nl) = text[..end].rfind('\n').filter(|nl| *nl > 0) {
        end = nl;
    }
    (format!("{}\n…", &text[..end]), true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use zend_proto::AgentEvent;

    fn journal(events: &[AgentEvent]) -> (tempfile::TempDir, RunJournal) {
        let dir = tempfile::tempdir().unwrap();
        let journal = RunJournal::open(dir.path()).unwrap();
        for event in events {
            journal.append("chat-1", event).unwrap();
        }
        (dir, journal)
    }

    fn call(id: &str, call: ToolCall) -> AgentEvent {
        AgentEvent::ToolCall { id: id.into(), call }
    }

    fn result(id: &str, output: &str, diff: Option<ToolDiff>) -> AgentEvent {
        AgentEvent::ToolResult {
            id: id.into(),
            is_error: false,
            output: Some(output.into()),
            diff,
            exit_code: None,
        }
    }

    /// Pi's codemode: its whole script as the agent wrote it (the doc kept
    /// none of it), then its result; nested in a subagent's doc too.
    #[test]
    fn a_scripts_whole_input_and_its_result_come_from_the_journal() {
        let code = "const r = await tools.read({ path: \"a.rs\" });\n\tif (r) {\n\t\treturn r.slice(0, 10);\n\t}\n";
        let (_dir, journal) = journal(&[
            call("c1", ToolCall::Unknown { name: "codemode".into(), input: Some(json!({ "code": code })) }),
            result("c1", "Script completed\nOutput:\nfn main() {}", None),
        ]);
        let reply = read(&journal, "chat-1", "c1").unwrap();
        let ToolCall::Unknown { input: Some(input), .. } = &reply.call else { panic!("{reply:?}") };
        assert_eq!(input["code"], code, "the script as written, whitespace and all");
        assert!(reply.done && !reply.is_error && !reply.withheld);
        assert_eq!(reply.output.as_deref(), Some("Script completed\nOutput:\nfn main() {}"));
        assert_eq!(read(&journal, "chat-1--sub--x", "c1").unwrap(), reply);
        // The doc kept nothing of that input: this is the only way to it.
        let doc = zend_doc::sanitize_tool_call(&ToolCall::Unknown { name: "codemode".into(), input: Some(json!({ "code": code })) });
        assert!(matches!(doc, ToolCall::Unknown { input: None, .. }));
    }

    #[test]
    fn an_mcp_tools_args_keep_their_shape_with_secrets_out() {
        let input = json!({
            "chat": "149a9c2c",
            "text": "deploy with GITHUB_TOKEN=ghp_abcdefghijklmnopqrstuvwxyz0123456789",
            "api_key": "abc123",
            "headers": { "Authorization": "Bearer xyz", "Accept": "json" },
            "limit": 5,
            "tags": ["a", "b"],
        });
        let (_dir, journal) = journal(&[
            call("m1", ToolCall::Mcp { server: "zend".into(), tool: "send_message".into(), input: Some(input) }),
            result("m1", "{\"ok\":true,\"token\":\"sk-ant-api03-abcdefghijklmnop\"}", None),
        ]);
        let reply = read(&journal, "chat-1", "m1").unwrap();
        let ToolCall::Mcp { input: Some(shown), tool, .. } = &reply.call else { panic!() };
        assert_eq!(tool, "send_message");
        assert_eq!(shown["chat"], "149a9c2c");
        assert_eq!(shown["limit"], 5);
        assert_eq!(shown["tags"], json!(["a", "b"]));
        assert_eq!(shown["api_key"], "[redacted]");
        assert_eq!(shown["headers"]["Authorization"], "[redacted]");
        assert_eq!(shown["headers"]["Accept"], "json");
        let text = shown["text"].as_str().unwrap();
        assert!(text.starts_with("deploy with") && !text.contains("ghp_abcdef"), "{text}");
        let output = reply.output.unwrap();
        assert!(output.contains("\"ok\":true") && !output.contains("sk-ant-api03"), "{output}");
    }

    #[test]
    fn an_edit_brings_its_diff_and_a_read_its_text_and_whether_it_is_a_picture() {
        let diff = ToolDiff { path: "src/a.rs".into(), old_text: Some("one\ntwo\n".into()), new_text: "one\n2\n".into() };
        let (_dir, journal) = journal(&[
            call("e1", ToolCall::EditFile { path: "src/a.rs".into(), old_string: Some("two".into()), new_string: Some("2".into()) }),
            result("e1", "Successfully replaced 1 block(s) in src/a.rs.", Some(diff.clone())),
            call("r1", ToolCall::ReadFile { path: "src/a.rs".into() }),
            result("r1", "one\n2\n", None),
            call("p1", ToolCall::ReadFile { path: "shots/G11.PNG".into() }),
            result("p1", "Read image file [image/png]", None),
        ]);
        let edit = read(&journal, "chat-1", "e1").unwrap();
        assert_eq!(edit.diff, Some(diff));
        assert!(matches!(&edit.call, ToolCall::EditFile { old_string: Some(o), .. } if o == "two"));
        let text = read(&journal, "chat-1", "r1").unwrap();
        assert_eq!((text.output.as_deref(), text.image.as_deref()), (Some("one\n2"), None));
        let picture = read(&journal, "chat-1", "p1").unwrap();
        assert_eq!(picture.image.as_deref(), Some("image/png"));
        // Its bytes come from where the journal says, against the chat's
        // folder when relative; another tool has no picture.
        assert_eq!(image_path(&journal, "chat-1", "p1", Some("/w/app")).unwrap(), PathBuf::from("/w/app/shots/G11.PNG"));
        assert!(matches!(image_path(&journal, "chat-1", "p1", None), Err(ToolOutputError::NotAPicture)));
        assert!(matches!(image_path(&journal, "chat-1", "r1", Some("/w")), Err(ToolOutputError::NotAPicture)));
        assert!(matches!(image_path(&journal, "chat-1", "e1", Some("/w")), Err(ToolOutputError::NotAPicture)));
        assert!(matches!(read(&journal, "chat-1", "nope"), Err(ToolOutputError::NotAvailable)));
    }

    #[test]
    fn a_secret_files_contents_and_a_secret_commands_output_stay_on_the_host() {
        let (_dir, journal) = journal(&[
            call("r1", ToolCall::ReadFile { path: "/w/.env.local".into() }),
            result("r1", "STRIPE_KEY=sk_live_abc", None),
            call("w1", ToolCall::WriteFile { path: "/w/id_ed25519".into(), content: Some("-----BEGIN".into()) }),
            result("w1", "ok", Some(ToolDiff { path: "/w/id_ed25519".into(), old_text: None, new_text: "-----BEGIN".into() })),
            call("x1", ToolCall::Exec { command: "cat .env".into() }),
            result("x1", "DB=postgres://u:p@h/db", None),
        ]);
        for id in ["r1", "w1", "x1"] {
            let reply = read(&journal, "chat-1", id).unwrap();
            assert!(reply.withheld, "{id}");
            assert_eq!(reply.output.as_deref(), Some(redact::SECRET_OUTPUT_WITHHELD), "{id}");
            assert!(reply.diff.is_none(), "{id}");
            let json = serde_json::to_string(&reply).unwrap();
            for secret in ["sk_live", "BEGIN", "postgres"] {
                assert!(!json.contains(secret), "{id}: {json}");
            }
        }
    }

    #[test]
    fn big_inputs_outputs_and_diffs_are_bounded() {
        let long = "x = 1\n".repeat(TOOL_EXCHANGE_OUTPUT_MAX_BYTES);
        let many: Vec<Value> = (0..400).map(|i| json!(format!("{i}: {}", "y".repeat(400)))).collect();
        let big = ToolDiff { path: "a".into(), old_text: Some(long.clone()), new_text: long.clone() + "z\n" };
        let (_dir, journal) = journal(&[
            call("r1", ToolCall::ReadFile { path: "big.txt".into() }),
            result("r1", &long, None),
            call("u1", ToolCall::Unknown { name: "codemode".into(), input: Some(json!({ "code": long, "items": many })) }),
            result("u1", "", Some(big)),
        ]);
        let read_reply = read(&journal, "chat-1", "r1").unwrap();
        let output = read_reply.output.unwrap();
        assert!(read_reply.output_trimmed && output.len() <= TOOL_EXCHANGE_OUTPUT_MAX_BYTES + 4);
        assert!(output.starts_with("x = 1\nx = 1\n"), "a read keeps its start");
        let unknown = read(&journal, "chat-1", "u1").unwrap();
        assert!(unknown.output.is_none() && unknown.diff.is_none(), "a diff past the cap isn't sent");
        let ToolCall::Unknown { input: Some(input), .. } = &unknown.call else { panic!() };
        // Its JSON over the cap: the start of it, as one string.
        let s = input.as_str().expect("one string");
        assert!(s.len() <= TOOL_EXCHANGE_INPUT_MAX_BYTES + 4 && s.starts_with('{'));
        assert!(serde_json::to_vec(&unknown).unwrap().len() < 3 * TOOL_EXCHANGE_INPUT_MAX_BYTES);
    }

    #[test]
    fn a_running_tool_answers_its_call_alone() {
        let (_dir, journal) = journal(&[call("x1", ToolCall::Exec { command: "sleep 9".into() })]);
        let reply = read(&journal, "chat-1", "x1").unwrap();
        assert!(!reply.done && reply.output.is_none());
        assert_eq!(reply.call, ToolCall::Exec { command: "sleep 9".into() });
    }
}
