//! What any tool's row opens to, rendered for its kind (ds/tool_open): the
//! tool's whole call and result as the device that ran it keeps them
//! (`ReadToolExchange`, from its run journal, bounded and redacted there).
//! The chat's doc keeps only a call's identifiers and a line of its result
//! (chat2-sync), so a script, an MCP tool's arguments, a read's lines or
//! picture, an edit's blocks come from the host when the row opens.
//!
//!   chat.tool_exchange { chat, id }   { sections = { … }, failed, done }
//!                                     or one word, as chat.tool_output's
//!   files.tool_image { chat, id }     the picture a read read, kept here
//!                                     as a file: { src, size } (src/files.rs)
//!
//! A section is what one part of the opened row shows, in order:
//!
//!   { kind = "script", label, text, lang }   code the tool ran (codemode's
//!                                            script, a command, a patch)
//!   { kind = "fields", rows = { { key, text, block, json } } }
//!                                            a structured input, key by key
//!                                            (block: JSON or many lines)
//!   { kind = "json", text }                  an input that isn't an object
//!   { kind = "image", chat, id, path }       a read's picture
//!   { kind = "code", text, path, start, more }
//!                                            a read's lines
//!   (a diff: src/tool_details.rs diff_value, kind = "diff")
//!   { kind = "output", text, exit_code, trimmed }
//!                                            any other result, as a
//!                                            command's output shows
//!   { kind = "prose", text }                 a web search's or fetch's answer
//!   { kind = "note", text }                  what isn't shown: running, withheld

use serde_json::{Value as Json, json};
use zgx_script::{ScriptHost, Value};

use crate::engine;
use crate::tool_details::diff_value;

/// Lines a read's code shows, at most.
pub const CODE_MAX_LINES: usize = 600;
/// Characters of one input value a field shows on its key's line before
/// it's a block of its own.
const FIELD_LINE_MAX: usize = 120;

pub fn install(host: &ScriptHost) {
    host.async_action("chat.tool_exchange", |arg| async move {
        let doc = arg.field("chat").map(crate::auth::text).unwrap_or_default();
        let id = arg.field("id").map(crate::auth::text).unwrap_or_default();
        if doc.is_empty() || id.is_empty() {
            return Err("failed".into());
        }
        let mut params = json!({ "chatId": doc, "toolId": id });
        if let Some(device) = crate::chat::host_of(&doc) {
            params["targetDeviceId"] = Json::String(device);
        }
        let reply = engine::get()
            .call("ReadToolExchange", params)
            .await
            .map_err(|e| crate::tool_details::output_error(&e))?;
        Ok(exchange_value(&reply, &doc, &id))
    });
}

/// A ToolExchangeReply as the row opened shows it.
pub fn exchange_value(reply: &Json, doc: &str, id: &str) -> Value {
    let call = &reply["call"];
    let mut sections: Vec<Value> = Vec::new();
    let output = reply["output"].as_str().filter(|o| !o.trim().is_empty());
    let failed = reply["isError"].as_bool() == Some(true);
    let withheld = reply["withheld"].as_bool() == Some(true);
    let mut output_shown = false;
    let text = |k: &str| call[k].as_str().filter(|s| !s.is_empty());
    match call["kind"].as_str().unwrap_or_default() {
        "readFile" => {
            let path = text("path").unwrap_or_default();
            if reply["image"].is_string() && !failed {
                sections.push(Value::record([
                    ("kind", Value::from("image")),
                    ("chat", Value::from(doc)),
                    ("id", Value::from(id)),
                    ("path", Value::from(path)),
                ]));
                // ("Read image file [image/png]": the picture says it.)
                output_shown = true;
            } else if !failed && !withheld && reply["done"].as_bool() != Some(false) && is_picture(path) {
                // A picture its host kept no bytes of (read before Zend
                // kept what a read gave the agent, or by a harness that
                // doesn't say): its path, and why there's nothing to see.
                // Never the file at that path now.
                sections.push(fields(&[("path", &Json::String(path.to_owned()))]));
                sections.push(note("nopicture"));
                output_shown = true;
            } else if let Some(out) = output.filter(|_| !failed && !withheld) {
                sections.push(code_section(out, path));
                output_shown = true;
            }
        }
        "writeFile" | "editFile" | "applyPatch" => {
            let shown = reply["diff"]
                .is_object()
                .then(|| diff_value(&reply["diff"]))
                .flatten()
                .or_else(|| {
                    // No diff from the harness: the call's own blocks.
                    let new = text("newString").or(text("content"))?;
                    let old = text("oldString").unwrap_or("");
                    diff_value(&json!({ "path": text("path").unwrap_or(""), "oldText": old, "newText": new }))
                });
            if let Some(diff) = shown {
                sections.push(diff);
                // (A success's "Wrote 3 lines" says what the diff does.)
                output_shown = !failed;
            }
        }
        "webSearch" => {
            if let Some(out) = output.filter(|_| !failed) {
                sections.push(prose(out));
                output_shown = true;
            }
        }
        "webFetch" => {
            if let Some(prompt) = text("prompt") {
                sections.push(fields(&[("prompt", &Json::String(prompt.to_owned()))]));
            }
            if let Some(out) = output.filter(|_| !failed) {
                sections.push(prose(out));
                output_shown = true;
            }
        }
        "mcp" | "unknown" => {
            let name = text("name").or(text("tool")).unwrap_or_default();
            sections.extend(input_sections(name, &call["input"]));
            if let Some(out) = output.filter(|_| !failed && !withheld)
                && let Ok(parsed) = serde_json::from_str::<Json>(out.trim())
                && (parsed.is_object() || parsed.is_array())
            {
                // A JSON answer, pretty and coloured.
                let pretty = serde_json::to_string_pretty(&parsed).unwrap_or_else(|_| out.to_owned());
                sections.push(script("result", &pretty, "json"));
                output_shown = true;
            }
        }
        _ => {}
    }
    if let Some(out) = output.filter(|_| !output_shown) {
        let mut fields = vec![
            ("kind", Value::from("output")),
            ("text", Value::from(out)),
            ("trimmed", Value::Bool(reply["outputTrimmed"].as_bool() == Some(true))),
        ];
        if let Some(code) = reply["exitCode"].as_i64() {
            fields.push(("exit_code", Value::Num(code as f64)));
        }
        sections.push(Value::record(fields));
        output_shown = true;
    }
    let done = reply["done"].as_bool() != Some(false);
    if !done && sections.is_empty() {
        sections.push(note("running"));
    }
    if withheld && !output_shown {
        sections.push(note("withheld"));
    }
    Value::record([
        ("sections", Value::List(sections.into())),
        ("failed", Value::Bool(failed)),
        ("done", Value::Bool(done)),
    ])
}

/// A PNG, JPEG, WebP or GIF by its name.
fn is_picture(path: &str) -> bool {
    path.trim().rsplit_once('.').is_some_and(|(_, ext)| {
        matches!(ext.to_ascii_lowercase().as_str(), "png" | "jpg" | "jpeg" | "webp" | "gif")
    })
}

fn note(word: &str) -> Value {
    Value::record([("kind", Value::from("note")), ("text", Value::from(word))])
}

fn prose(text: &str) -> Value {
    Value::record([("kind", Value::from("prose")), ("text", Value::from(text))])
}

fn script(label: &str, text: &str, lang: &str) -> Value {
    Value::record([
        ("kind", Value::from("script")),
        ("label", Value::from(label)),
        ("text", Value::from(text)),
        ("lang", Value::from(lang)),
    ])
}

/// Input keys that hold code the tool runs, and its language.
fn code_key(tool: &str, key: &str) -> Option<&'static str> {
    let tool = tool.rsplit("__").next().unwrap_or(tool).to_ascii_lowercase();
    match key {
        "code" | "script" | "source" | "program" | "js" | "javascript" => Some(
            if tool.contains("python") || tool.contains("jupyter") {
                "python"
            } else if tool.contains("luau") || tool.contains("visual") || tool.contains("lua") {
                "luau"
            } else if tool.contains("sql") {
                "sql"
            } else {
                "javascript"
            },
        ),
        "command" | "cmd" | "shell" | "bash" => Some("shell"),
        "patch" | "diff" => Some("diff"),
        "sql" => Some("sql"),
        "query" if tool.contains("sql") => Some("sql"),
        _ => None,
    }
}

/// A tool's input: the code it ran as code, the rest key by key.
fn input_sections(tool: &str, input: &Json) -> Vec<Value> {
    let mut out = Vec::new();
    match input {
        Json::Null => {}
        Json::Object(map) => {
            let mut rest: Vec<(&str, &Json)> = Vec::new();
            for (key, value) in map {
                match (code_key(tool, key), value.as_str()) {
                    (Some(lang), Some(code)) if !code.trim().is_empty() => out.push(script(key, code, lang)),
                    _ => rest.push((key.as_str(), value)),
                }
            }
            if !rest.is_empty() {
                out.push(fields(&rest));
            }
        }
        // (Past its cap the host sends the input's JSON's start as one
        // string.)
        Json::String(s) => out.push(Value::record([("kind", Value::from("json")), ("text", Value::from(s.as_str()))])),
        other => out.push(Value::record([
            ("kind", Value::from("json")),
            ("text", Value::from(serde_json::to_string_pretty(other).unwrap_or_default().as_str())),
        ])),
    }
    out
}

/// Inputs key by key: a short value on its key's line, a long, many-line
/// or structured one as a block of its own.
fn fields(rows: &[(&str, &Json)]) -> Value {
    let rows: Vec<Value> = rows
        .iter()
        .map(|(key, value)| {
            let (text, structured) = match value {
                Json::String(s) => (s.clone(), false),
                Json::Null => ("null".into(), false),
                Json::Bool(_) | Json::Number(_) => (value.to_string(), false),
                _ => (serde_json::to_string_pretty(value).unwrap_or_default(), true),
            };
            let block = structured || text.contains('\n') || text.chars().count() > FIELD_LINE_MAX;
            Value::record([
                ("key", Value::from(*key)),
                ("text", Value::from(text.as_str())),
                ("block", Value::Bool(block)),
                ("json", Value::Bool(structured)),
            ])
        })
        .collect();
    Value::record([("kind", Value::from("fields")), ("rows", Value::List(rows.into()))])
}

/// A read's lines as code: a cat -n gutter some harnesses add
/// ("     1→text", "1\ttext") taken off and its first number kept, at most
/// [CODE_MAX_LINES].
fn code_section(text: &str, path: &str) -> Value {
    let lines: Vec<&str> = text.lines().collect();
    let numbered = |l: &str| -> Option<(usize, usize)> {
        let t = l.trim_start();
        let digits = t.len() - t.trim_start_matches(|c: char| c.is_ascii_digit()).len();
        if digits == 0 {
            return None;
        }
        let rest = &t[digits..];
        let sep = if rest.starts_with('→') {
            '→'.len_utf8()
        } else if rest.starts_with('\t') {
            1
        } else {
            return None;
        };
        Some((t[..digits].parse().ok()?, l.len() - rest.len() + sep))
    };
    let gutter = !lines.is_empty() && lines.iter().filter(|l| !l.is_empty()).all(|l| numbered(l).is_some());
    let start = if gutter { lines.first().and_then(|l| numbered(l)).map_or(1, |(n, _)| n) } else { 1 };
    let body: Vec<&str> = lines
        .iter()
        .map(|l| if gutter { numbered(l).map_or(*l, |(_, at)| &l[at..]) } else { l })
        .collect();
    let more = body.len().saturating_sub(CODE_MAX_LINES);
    let shown = body[..body.len().min(CODE_MAX_LINES)].join("\n");
    Value::record([
        ("kind", Value::from("code")),
        ("text", Value::from(shown.as_str())),
        ("path", Value::from(path)),
        ("start", Value::Num(start as f64)),
        ("more", Value::Num(more as f64)),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn str_of(v: Option<&Value>) -> &str {
        match v {
            Some(Value::Str(s)) => s,
            _ => "",
        }
    }

    fn kinds(v: &Value) -> Vec<String> {
        let Some(Value::List(list)) = v.field("sections") else { panic!() };
        list.iter().map(|s| str_of(s.field("kind")).to_owned()).collect()
    }

    fn section<'a>(v: &'a Value, kind: &str) -> &'a Value {
        let Some(Value::List(list)) = v.field("sections") else { panic!() };
        list.iter().find(|s| str_of(s.field("kind")) == kind).unwrap_or_else(|| panic!("no {kind}"))
    }

    fn s<'a>(v: &'a Value, k: &str) -> &'a str {
        str_of(v.field(k))
    }

    #[test]
    fn a_codemode_call_opens_to_its_script_then_its_result() {
        let reply = json!({
            "call": { "kind": "unknown", "name": "codemode", "input": { "code": "const r = await tools.read({path:'a'});\nreturn r;" } },
            "done": true, "output": "Script completed\nOutput:\nhi",
        });
        let v = exchange_value(&reply, "chat-1", "c1");
        assert_eq!(kinds(&v), ["script", "output"]);
        let script = section(&v, "script");
        assert_eq!((s(script, "lang"), s(script, "label")), ("javascript", "code"));
        assert!(s(script, "text").starts_with("const r = await"));
        assert_eq!(s(section(&v, "output"), "text"), "Script completed\nOutput:\nhi");
    }

    #[test]
    fn an_mcp_tool_opens_to_its_args_key_by_key_and_its_json_result() {
        let reply = json!({
            "call": { "kind": "mcp", "server": "zend", "tool": "send_message", "input": { "chat": "149a", "text": "line one\nline two", "limit": 5, "opts": { "wait": false } } },
            "done": true, "output": "{\"ok\":true}",
        });
        let v = exchange_value(&reply, "chat-1", "m1");
        assert_eq!(kinds(&v), ["fields", "script"]);
        let Some(Value::List(rows)) = section(&v, "fields").field("rows") else { panic!() };
        let row = |k: &str| rows.iter().find(|r| s(r, "key") == k).unwrap();
        assert_eq!((s(row("chat"), "text"), row("chat").field("block")), ("149a", Some(&Value::Bool(false))));
        assert_eq!(row("text").field("block"), Some(&Value::Bool(true)));
        assert_eq!(s(row("limit"), "text"), "5");
        assert_eq!(row("opts").field("json"), Some(&Value::Bool(true)));
        let result = section(&v, "script");
        assert_eq!((s(result, "label"), s(result, "lang")), ("result", "json"));
        assert_eq!(s(result, "text"), "{\n  \"ok\": true\n}");
    }

    #[test]
    fn a_read_opens_to_its_picture_or_its_lines_as_code() {
        let picture = exchange_value(
            &json!({ "call": { "kind": "readFile", "path": "shots/a.png" }, "done": true, "output": "Read image file [image/png]", "image": "image/png" }),
            "chat-1",
            "p1",
        );
        assert_eq!(kinds(&picture), ["image"]);
        let image = section(&picture, "image");
        assert_eq!((s(image, "chat"), s(image, "id"), s(image, "path")), ("chat-1", "p1", "shots/a.png"));
        // Claude Code's cat -n gutter: off, its first number kept.
        let text = exchange_value(
            &json!({ "call": { "kind": "readFile", "path": "src/a.rs" }, "done": true, "output": "    41→fn a() {\n    42→\tb()\n    43→}" }),
            "chat-1",
            "r1",
        );
        let code = section(&text, "code");
        assert_eq!(s(code, "text"), "fn a() {\n\tb()\n}");
        assert_eq!(code.field("start"), Some(&Value::Num(41.0)));
        // Pi's plain lines stay as they are.
        let plain = exchange_value(&json!({ "call": { "kind": "readFile", "path": "a.md" }, "output": "# T\n2026 notes" }), "c", "r");
        assert_eq!(s(section(&plain, "code"), "text"), "# T\n2026 notes");
        assert_eq!(kinds(&plain), ["code"]);
    }

    /// A read of a picture its host kept no bytes of (an old journal):
    /// its path and a quiet line why, no picture asked for.
    #[test]
    fn a_read_picture_with_nothing_kept_shows_its_path_and_why() {
        let old = exchange_value(
            &json!({ "call": { "kind": "readFile", "path": "/home/me/.cache/sandy-phys/m1.png" }, "done": true, "output": "Read image file [image/png]" }),
            "chat-1",
            "p1",
        );
        assert_eq!(kinds(&old), ["fields", "note"]);
        assert_eq!(s(section(&old, "note"), "text"), "nopicture");
        let failed = exchange_value(
            &json!({ "call": { "kind": "readFile", "path": "a.png" }, "done": true, "isError": true, "output": "ENOENT" }),
            "c",
            "p",
        );
        assert_eq!(kinds(&failed), ["output"]);
    }

    #[test]
    fn an_edit_opens_to_its_diff_from_the_result_or_the_call() {
        let reply = json!({
            "call": { "kind": "editFile", "path": "a.rs" },
            "done": true, "output": "Successfully replaced 1 block(s)",
            "diff": { "path": "a.rs", "oldText": "a\nb\n", "newText": "a\nc\n" },
        });
        let v = exchange_value(&reply, "c", "e");
        assert_eq!(kinds(&v), ["diff"]);
        assert_eq!(section(&v, "diff").field("added"), Some(&Value::Num(1.0)));
        let from_call = exchange_value(&json!({ "call": { "kind": "editFile", "path": "a.rs", "oldString": "b", "newString": "c" }, "done": true }), "c", "e");
        assert_eq!(section(&from_call, "diff").field("removed"), Some(&Value::Num(1.0)));
        // A failed edit says why, under what it tried.
        let failed = exchange_value(&json!({ "call": { "kind": "editFile", "path": "a.rs", "oldString": "b", "newString": "c" }, "isError": true, "output": "Could not find the exact text" }), "c", "e");
        assert_eq!(kinds(&failed), ["diff", "output"]);
    }

    #[test]
    fn a_running_or_withheld_tool_says_so() {
        let running = exchange_value(&json!({ "call": { "kind": "unknown", "name": "x" }, "done": false }), "c", "x");
        assert_eq!(kinds(&running), ["note"]);
        assert_eq!(s(section(&running, "note"), "text"), "running");
        let secret = exchange_value(
            &json!({ "call": { "kind": "readFile", "path": ".env" }, "done": true, "withheld": true, "output": "(output not kept: may contain secrets)" }),
            "c",
            "r",
        );
        assert_eq!(kinds(&secret), ["output"]);
        let search = exchange_value(&json!({ "call": { "kind": "webSearch", "query": "q" }, "done": true, "output": "**Answer** text" }), "c", "w");
        assert_eq!(kinds(&search), ["prose"]);
    }
}
