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
//! - every string of every kind of call cut to
//!   [`TOOL_EXCHANGE_INPUT_STRING_MAX_BYTES`] and redacted as a command's
//!   output is ([`redact::redact_command_output`]: private key blocks,
//!   env-style assignments too); all a key naming a secret holds
//!   ([`redact::input_key_is_secret`]) replaced, whatever its shape; the whole input at
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
//! A tool whose result gave the agent a picture (a read of a PNG) says its
//! type (`image`); its bytes come from `ReadToolImageChunk`: the very bytes
//! the agent was given, kept beside the journal as they came
//! ([`keep_image`], at most [`TOOL_IMAGE_MAX_BYTES`]), never a file read at
//! the path when the row opens. No picture for a tool still running, one
//! that failed, or one whose file holds secrets by its name; none for a
//! tool journaled before pictures were kept.

use std::path::PathBuf;

use serde_json::Value;
use zend_proto::{
    TOOL_EXCHANGE_DIFF_MAX_BYTES, TOOL_EXCHANGE_INPUT_MAX_BYTES,
    TOOL_EXCHANGE_INPUT_STRING_MAX_BYTES, TOOL_EXCHANGE_OUTPUT_MAX_BYTES, TOOL_IMAGE_MAX_BYTES,
    AgentEvent, ToolCall, ToolDiff, ToolExchangeReply, redact,
};

use crate::run_journal::{KeptImage, RunJournal, ToolExchange};
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
    Ok(reply(&exchange))
}

/// The picture the exchange may show: its result's, kept, when the tool
/// settled without failing and touched no secret file.
fn shown_image(exchange: &ToolExchange) -> Option<&KeptImage> {
    let settled = exchange.result.as_ref().is_some_and(|r| !r.is_error);
    (settled && !withholds(&exchange.call)).then_some(exchange.image.as_ref()).flatten()
}

/// What a harness's [`AgentEvent::ToolImage`] becomes before it's journaled
/// or published: its bytes kept beside the journal ([`RunJournal::keep_tool_image`])
/// and the event naming them. `None`, dropped, for one with no bytes, past
/// [`TOOL_IMAGE_MAX_BYTES`], or not a PNG, JPEG, WebP or GIF by its first
/// bytes (whatever type the harness said).
pub fn keep_image(
    journal: &RunJournal,
    chat_id: &str,
    key: &str,
    id: String,
    data: Option<&str>,
) -> Option<AgentEvent> {
    use base64::Engine as _;
    let data = data?.trim();
    if data.len() > TOOL_IMAGE_MAX_BYTES.div_ceil(3) * 4 {
        return None;
    }
    let bytes = base64::engine::general_purpose::STANDARD.decode(data).ok()?;
    if bytes.len() > TOOL_IMAGE_MAX_BYTES {
        return None;
    }
    let (ext, mime) = crate::uploads::raster_signature(&bytes)?;
    let name = journal
        .keep_tool_image(chat_id, key, &bytes, ext)
        .inspect_err(|err| tracing::warn!(chat = %chat_id, error = %err, "keeping a tool's picture failed"))
        .ok()?;
    Some(AgentEvent::ToolImage {
        id,
        mime_type: mime.into(),
        data: None,
        stored: Some(name),
    })
}

/// The file of the picture tool `tool_id`'s result gave the agent, kept
/// beside the journal ([`shown_image`] says when there is one). Resolved
/// once: the chunks after the first don't scan the journal again.
pub fn image_file(journal: &RunJournal, doc_id: &str, tool_id: &str) -> Result<PathBuf, ToolOutputError> {
    let chat = journal_chat(doc_id);
    if let Some(path) = journal.remembered_picture(chat, tool_id) {
        return Ok(path);
    }
    let exchange = journal.tool_exchange(chat, tool_id)?.ok_or(ToolOutputError::NotAvailable)?;
    let kept = shown_image(&exchange).ok_or(ToolOutputError::NotAPicture)?;
    let path = journal.tool_image_file(chat, &kept.name).ok_or(ToolOutputError::NotAPicture)?;
    journal.remember_picture(chat, tool_id, path.clone());
    Ok(path)
}

/// The reply for a call and (once settled) its result and picture.
pub fn reply(exchange: &ToolExchange) -> ToolExchangeReply {
    let (call, result) = (&exchange.call, exchange.result.as_ref());
    let withheld = withholds(call);
    let image = shown_image(exchange).map(|kept| kept.mime_type.clone());
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

/// How much more than its cap a text is redacted of before it's cut to
/// it: a secret cut in two at the first cut sits past the second.
const REDACT_MARGIN: usize = 4096;

/// A text as it may leave the host: cut to `max` and redacted as a
/// command's output is ([`redact::redact_command_output`]: private key
/// blocks dropped, an env-style assignment's value hidden). Cut first, so
/// a huge input costs no more to redact than one at the cap.
fn shown_text(text: &str, max: usize) -> (String, bool) {
    let (head, cut) = cut_start(text, max + REDACT_MARGIN);
    let (shown, cut_again) = cut_start(&redact::redact_command_output(&head), max);
    (shown, cut || cut_again)
}

/// The call with its input made safe to show: every string of every kind
/// cut and redacted, a secret file's contents none, and the whole at most
/// [`TOOL_EXCHANGE_INPUT_MAX_BYTES`] (each of two fields half of it).
fn shown_call(call: &ToolCall, withheld: bool) -> ToolCall {
    const ONE: usize = TOOL_EXCHANGE_INPUT_MAX_BYTES / 2;
    let s = |s: &str, max: usize| shown_text(s, max.min(TOOL_EXCHANGE_INPUT_STRING_MAX_BYTES)).0;
    let contents = |c: &Option<String>| if withheld { None } else { c.as_deref().map(|c| s(c, ONE)) };
    match call {
        ToolCall::Exec { command } => ToolCall::Exec { command: s(command, usize::MAX) },
        ToolCall::ReadFile { path } => ToolCall::ReadFile { path: s(path, usize::MAX) },
        ToolCall::WriteFile { path, content } => ToolCall::WriteFile {
            path: s(path, ONE / 4),
            content: contents(content),
        },
        ToolCall::EditFile {
            path,
            old_string,
            new_string,
        } => ToolCall::EditFile {
            path: s(path, ONE / 4),
            old_string: contents(old_string),
            new_string: contents(new_string),
        },
        ToolCall::ApplyPatch { path } => ToolCall::ApplyPatch { path: path.as_deref().map(|p| s(p, usize::MAX)) },
        ToolCall::Search { pattern, path } => ToolCall::Search {
            pattern: s(pattern, ONE),
            path: path.as_deref().map(|p| s(p, ONE)),
        },
        ToolCall::Glob { pattern } => ToolCall::Glob { pattern: s(pattern, usize::MAX) },
        ToolCall::WebFetch { url, prompt } => ToolCall::WebFetch {
            url: s(url, ONE),
            prompt: prompt.as_deref().map(|p| s(p, ONE)),
        },
        ToolCall::WebSearch { query } => ToolCall::WebSearch { query: s(query, usize::MAX) },
        ToolCall::Todo { items } => {
            // As many items as fit, each its text cut.
            let mut left = TOOL_EXCHANGE_INPUT_MAX_BYTES;
            let mut kept = Vec::new();
            for item in items {
                let text = s(&item.text, 1024);
                let cost = text.len() + 64;
                if cost > left {
                    break;
                }
                left -= cost;
                kept.push(zend_proto::TodoItem { text, ..item.clone() });
            }
            ToolCall::Todo { items: kept }
        }
        ToolCall::Mcp {
            server,
            tool,
            input,
        } => ToolCall::Mcp {
            server: s(server, 256),
            tool: s(tool, 256),
            input: input.as_ref().map(shown_input),
        },
        ToolCall::Unknown { name, input } => ToolCall::Unknown {
            name: s(name, 512),
            input: input.as_ref().map(shown_input),
        },
    }
}

/// A tool's input made safe to show: strings cut and redacted, all a
/// secret key holds replaced (a list, an object as much as a string); past [`TOOL_EXCHANGE_INPUT_MAX_BYTES`] of JSON,
/// its pretty JSON's start as one string.
pub fn shown_input(input: &Value) -> Value {
    fn walk(value: &Value) -> Value {
        match value {
            Value::String(s) => Value::String(shown_text(s, TOOL_EXCHANGE_INPUT_STRING_MAX_BYTES).0),
            Value::Array(items) => Value::Array(items.iter().map(walk).collect()),
            Value::Object(map) => Value::Object(
                map.iter()
                    .map(|(k, v)| {
                        let v = if redact::input_key_is_secret(k) && !v.is_null() {
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
    let (shown, cut) = shown_text(text.trim_end(), TOOL_EXCHANGE_OUTPUT_MAX_BYTES);
    (shown.trim_end().to_owned(), cut)
}

/// A diff as it may leave the host: both sides redacted alike, as a
/// command's output is (so their lines still pair up); none when a side is
/// past the cap.
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
        old_text: diff.old_text.as_deref().map(redact::redact_command_output),
        new_text: redact::redact_command_output(&diff.new_text),
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
    fn an_edit_brings_its_diff_and_a_read_its_text() {
        let diff = ToolDiff { path: "src/a.rs".into(), old_text: Some("one\ntwo\n".into()), new_text: "one\n2\n".into() };
        let (_dir, journal) = journal(&[
            call("e1", ToolCall::EditFile { path: "src/a.rs".into(), old_string: Some("two".into()), new_string: Some("2".into()) }),
            result("e1", "Successfully replaced 1 block(s) in src/a.rs.", Some(diff.clone())),
            call("r1", ToolCall::ReadFile { path: "src/a.rs".into() }),
            result("r1", "one\n2\n", None),
        ]);
        let edit = read(&journal, "chat-1", "e1").unwrap();
        assert_eq!(edit.diff, Some(diff));
        assert!(matches!(&edit.call, ToolCall::EditFile { old_string: Some(o), .. } if o == "two"));
        let text = read(&journal, "chat-1", "r1").unwrap();
        assert_eq!((text.output.as_deref(), text.image.as_deref()), (Some("one\n2"), None));
        assert!(matches!(read(&journal, "chat-1", "nope"), Err(ToolOutputError::NotAvailable)));
    }

    /// A PNG's first bytes and a little more: what `raster_signature` takes.
    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDRpicture";

    fn image(journal: &RunJournal, id: &str, bytes: &[u8]) -> Option<AgentEvent> {
        use base64::Engine as _;
        let data = base64::engine::general_purpose::STANDARD.encode(bytes);
        keep_image(journal, "chat-1", &format!("chat-1\0{id}"), id.into(), Some(&data))
    }

    /// No picture without the bytes the tool gave the agent: a read of a
    /// `.png` journaled before pictures were kept (or by a harness that
    /// doesn't send them) says no image, and nothing is read at its path.
    #[test]
    fn a_read_of_a_picture_without_its_kept_bytes_shows_no_picture() {
        let (_dir, journal) = journal(&[
            call("p1", ToolCall::ReadFile { path: "/home/me/.cache/sandy-phys/m1.png".into() }),
            result("p1", "Read image file [image/png]", None),
        ]);
        let reply = read(&journal, "chat-1", "p1").unwrap();
        assert_eq!(reply.image, None, "{reply:?}");
        assert!(reply.done && !reply.is_error);
    }

    /// The owner's case: pi reads a PNG outside the project; the row's
    /// picture is the bytes pi gave the agent, kept beside the journal and
    /// served from there, whatever is at the path now.
    #[test]
    fn a_reads_picture_is_the_bytes_its_result_gave_the_agent() {
        let dir = tempfile::tempdir().unwrap();
        let journal = RunJournal::open(dir.path()).unwrap();
        journal.append("chat-1", &call("p1", ToolCall::ReadFile { path: "~/.cache/sandy-phys/m1.png".into() })).unwrap();
        let kept = image(&journal, "p1", PNG).expect("kept");
        let AgentEvent::ToolImage { data, stored: Some(name), mime_type, .. } = &kept else { panic!("{kept:?}") };
        assert!(data.is_none(), "the journal names the file, never holds the bytes");
        assert_eq!(mime_type, "image/png");
        journal.append("chat-1", &kept).unwrap();
        journal.append("chat-1", &result("p1", "Read image file [image/png]", None)).unwrap();

        let reply = read(&journal, "chat-1", "p1").unwrap();
        assert_eq!(reply.image.as_deref(), Some("image/png"));
        let file = image_file(&journal, "chat-1", "p1").unwrap();
        assert_eq!(file, dir.path().join("chat-1.images").join(name));
        assert_eq!(std::fs::read(&file).unwrap(), PNG);
        let chunk = crate::uploads::read_image_file_chunk(&file, 0).unwrap();
        assert_eq!(chunk.mime_type, "image/png");
        // A subagent's doc reads its parent's journal, as for the call.
        assert_eq!(image_file(&journal, "chat-1--sub--x", "p1").unwrap(), file);
        // Another tool has no picture.
        assert!(matches!(image_file(&journal, "chat-1", "nope"), Err(ToolOutputError::NotAvailable)));
    }

    /// The engine, not the window, holds back a picture: none while the read
    /// runs, none when it failed, none for a file that holds secrets by its
    /// name, even with bytes kept.
    #[test]
    fn no_picture_for_a_running_failed_or_secret_read() {
        let dir = tempfile::tempdir().unwrap();
        let journal = RunJournal::open(dir.path()).unwrap();
        for (id, path) in [("run", "/w/a.png"), ("err", "/w/b.png"), ("sec", "/w/.ssh/id_ed25519")] {
            journal.append("chat-1", &call(id, ToolCall::ReadFile { path: path.into() })).unwrap();
            journal.append("chat-1", &image(&journal, id, PNG).unwrap()).unwrap();
        }
        journal
            .append("chat-1", &AgentEvent::ToolResult { id: "err".into(), is_error: true, output: Some("EACCES".into()), diff: None, exit_code: None })
            .unwrap();
        journal.append("chat-1", &result("sec", "Read image file [image/png]", None)).unwrap();
        for id in ["run", "err", "sec"] {
            let reply = read(&journal, "chat-1", id).unwrap();
            assert_eq!(reply.image, None, "{id}: {reply:?}");
            assert!(matches!(image_file(&journal, "chat-1", id), Err(ToolOutputError::NotAPicture)), "{id}");
        }
    }

    /// Only a real picture, at most 10 MB, is kept; its type is what its
    /// first bytes say, not what the harness said.
    #[test]
    fn only_a_bounded_real_picture_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let journal = RunJournal::open(dir.path()).unwrap();
        assert!(image(&journal, "t", b"<svg onload=alert(1)>").is_none());
        let mut big = PNG.to_vec();
        big.resize(TOOL_IMAGE_MAX_BYTES + 1, 0);
        assert!(image(&journal, "b", &big).is_none(), "past the cap");
        big.truncate(TOOL_IMAGE_MAX_BYTES);
        assert!(image(&journal, "m", &big).is_some(), "at the cap");
        assert!(keep_image(&journal, "chat-1", "k", "n".into(), None).is_none());
        assert!(keep_image(&journal, "chat-1", "k", "n".into(), Some("not base64!")).is_none());
        let jpeg = image(&journal, "j", b"\xff\xd8\xff\xe0rest").unwrap();
        assert!(matches!(jpeg, AgentEvent::ToolImage { mime_type, .. } if mime_type == "image/jpeg"));
        // A line can't name a file outside the pictures' folder.
        assert!(journal.tool_image_file("chat-1", "../../etc/passwd").is_none());
        assert!(journal.tool_image_file("chat-1", &format!("{}.svg", "a".repeat(64))).is_none());
    }

    /// A picture's chunks after the first don't scan the journal again.
    #[test]
    fn a_pictures_later_chunks_dont_rescan_the_journal() {
        let dir = tempfile::tempdir().unwrap();
        let journal = RunJournal::open(dir.path()).unwrap();
        journal.append("chat-1", &call("p1", ToolCall::ReadFile { path: "/w/a.png".into() })).unwrap();
        journal.append("chat-1", &image(&journal, "p1", PNG).unwrap()).unwrap();
        journal.append("chat-1", &result("p1", "ok", None)).unwrap();
        let first = image_file(&journal, "chat-1", "p1").unwrap();
        // With the journal gone, only what the first resolved can answer.
        std::fs::remove_file(dir.path().join("chat-1.jsonl")).unwrap();
        assert_eq!(image_file(&journal, "chat-1", "p1").unwrap(), first);
        // Discarding the chat forgets it, and its pictures with it.
        journal.discard("chat-1").unwrap();
        assert!(!first.exists());
        assert!(image_file(&journal, "chat-1", "p1").is_err());
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

    /// A key naming a secret hides all it holds, whatever its shape.
    #[test]
    fn a_secret_keys_whole_value_is_redacted_whatever_its_shape() {
        let input = json!({
            "password": ["hunter2", "hunter3"],
            "credentials": { "value": "s3cr3t-value", "user": "ann" },
            "nested": { "api_key": { "primary": "pk-0042" } },
            "token": 424242,
            "note": "kept",
        });
        let (_dir, journal) = journal(&[
            call("m1", ToolCall::Mcp { server: "acme".into(), tool: "login".into(), input: Some(input) }),
            result("m1", "ok", None),
        ]);
        let reply = read(&journal, "chat-1", "m1").unwrap();
        let json = serde_json::to_string(&reply).unwrap();
        for secret in ["hunter2", "hunter3", "s3cr3t-value", "pk-0042", "424242"] {
            assert!(!json.contains(secret), "{secret}: {json}");
        }
        let ToolCall::Mcp { input: Some(shown), .. } = &reply.call else { panic!() };
        assert_eq!(shown["password"], "[redacted]");
        assert_eq!(shown["credentials"], "[redacted]");
        assert_eq!(shown["nested"]["api_key"], "[redacted]");
        assert_eq!(shown["note"], "kept");
    }

    /// A file's contents leave as a command's output does: a private key
    /// block dropped whole, an env-style assignment's value hidden.
    #[test]
    fn written_and_edited_contents_and_diffs_are_redacted_as_a_commands_output_is() {
        let pem = "-----BEGIN RSA PRIVATE KEY-----
MIIEowIBAAKCAQEA
qrstuvwx
-----END RSA PRIVATE KEY-----
";
        let env = "ROLLOUT_SALT=plainvalue1
";
        let content = format!("fn a() {{}}
{pem}{env}");
        let (_dir, journal) = journal(&[
            call("w1", ToolCall::WriteFile { path: "config/app.rs".into(), content: Some(content.clone()) }),
            result("w1", "ok", Some(ToolDiff { path: "config/app.rs".into(), old_text: None, new_text: content.clone() })),
            call("e1", ToolCall::EditFile { path: "config/app.rs".into(), old_string: Some(pem.into()), new_string: Some(env.into()) }),
            result("e1", "ok", Some(ToolDiff { path: "config/app.rs".into(), old_text: Some(content.clone()), new_text: format!("fn a() {{}}
{env}") })),
            call("u1", ToolCall::Unknown { name: "codemode".into(), input: Some(json!({ "code": format!("await write(`\n{content}`);") })) }),
            result("u1", "ok", None),
        ]);
        for id in ["w1", "e1", "u1"] {
            let reply = read(&journal, "chat-1", id).unwrap();
            assert!(!reply.withheld, "{id}");
            let json = serde_json::to_string(&reply).unwrap();
            for secret in ["MIIEowIBAAKCAQEA", "qrstuvwx", "plainvalue1"] {
                assert!(!json.contains(secret), "{id} {secret}: {json}");
            }
            assert!(json.contains("fn a()"), "{id}: the rest stays: {json}");
        }
    }

    /// Every kind of call leaves bounded and redacted, not only the ones
    /// with free-form input.
    #[test]
    fn every_kind_of_call_is_bounded_and_redacted() {
        let huge = format!("ghp_abcdefghijklmnopqrstuvwxyz0123456789 {}", "q".repeat(3 * TOOL_EXCHANGE_INPUT_STRING_MAX_BYTES));
        let items: Vec<zend_proto::TodoItem> = (0..2000)
            .map(|i| zend_proto::TodoItem { text: format!("{i} {}", "t".repeat(1000)), done: false, status: None })
            .collect();
        let calls = [
            ToolCall::Exec { command: huge.clone() },
            ToolCall::ReadFile { path: huge.clone() },
            ToolCall::Search { pattern: huge.clone(), path: Some(huge.clone()) },
            ToolCall::Glob { pattern: huge.clone() },
            ToolCall::WebSearch { query: huge.clone() },
            ToolCall::WebFetch { url: huge.clone(), prompt: Some(huge.clone()) },
            ToolCall::ApplyPatch { path: Some(huge.clone()) },
            ToolCall::Todo { items },
        ];
        let events: Vec<AgentEvent> = calls.iter().enumerate().map(|(i, c)| call(&format!("k{i}"), c.clone())).collect();
        let (_dir, journal) = journal(&events);
        for i in 0..calls.len() {
            let reply = read(&journal, "chat-1", &format!("k{i}")).unwrap();
            let json = serde_json::to_string(&reply.call).unwrap();
            assert!(json.len() <= TOOL_EXCHANGE_INPUT_MAX_BYTES + 1024, "k{i}: {} bytes", json.len());
            assert!(!json.contains("ghp_abcdef"), "k{i}");
        }
    }

    #[test]
    fn a_running_tool_answers_its_call_alone() {
        let (_dir, journal) = journal(&[call("x1", ToolCall::Exec { command: "sleep 9".into() })]);
        let reply = read(&journal, "chat-1", "x1").unwrap();
        assert!(!reply.done && reply.output.is_none());
        assert_eq!(reply.call, ToolCall::Exec { command: "sleep 9".into() });
    }
}
