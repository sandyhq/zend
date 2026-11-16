//! Every tool's row opens to what it did (ds/tool_open), for the tools the
//! owner's pi chat couldn't open: a read of a PNG, an edit, codemode, an
//! MCP tool the app knows nothing of. The rows are what src/chat.rs builds
//! (`built_entry`) of an engine-shaped reply holding those calls as the
//! doc keeps them (identifiers only, chat2-sync's `sanitize_tool_call`);
//! what each did comes as the host serves it, the engine's
//! `ToolExchangeReply` shaped by src/tool_open.rs (`exchange_value`), the picture
//! as files.tool_image keeps one. Mounted headless (ui/tool_open_fixture.luau).

use std::cell::RefCell;
use std::rc::Rc;

use serde_json::{Value as Json, json};
use zgx_script::{ScriptHost, Value, testing::TestApp};

const PNG_1X1: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8DwHwAFBQIAX8jx0gAAAABJRU5ErkJggg==";

/// The reply as the engine's doc has it: pi's calls, their inputs gone.
fn reply() -> Json {
    json!({
        "id": "a1", "role": "assistant", "status": "complete", "createdAt": 1_762_000_000_000_i64,
        "parts": [
            { "id": "r1", "kind": "tool", "resolved": true, "call": { "kind": "readFile", "path": "/home/me/shots/f19.png" }, "output": "Read image file [image/png]" },
            { "id": "e1", "kind": "tool", "resolved": true, "call": { "kind": "editFile", "path": "/home/me/src/app.rs" }, "output": "Successfully replaced 1 block(s) in /home/me/src/app.rs." },
            { "id": "c1", "kind": "tool", "resolved": true, "call": { "kind": "unknown", "name": "codemode" }, "output": "Script completed" },
            { "id": "m1", "kind": "tool", "resolved": true, "call": { "kind": "mcp", "server": "acme", "tool": "lookup_order" }, "output": "{\"status\":\"shipped\"}" },
            { "id": "t1", "kind": "text", "text": "Done." },
        ],
    })
}

/// What the host's journal answers for each (ReadToolExchange).
fn exchange(id: &str) -> Json {
    match id {
        "r1" => json!({ "call": { "kind": "readFile", "path": "/home/me/shots/f19.png" }, "done": true, "output": "Read image file [image/png]", "image": "image/png" }),
        "e1" => json!({
            "call": { "kind": "editFile", "path": "/home/me/src/app.rs" }, "done": true,
            "output": "Successfully replaced 1 block(s) in /home/me/src/app.rs.",
            "diff": { "path": "/home/me/src/app.rs", "oldText": "fn main() {\n    old_call();\n}\n", "newText": "fn main() {\n    new_call();\n    more();\n}\n" },
        }),
        "c1" => json!({
            "call": { "kind": "unknown", "name": "codemode", "input": { "code": "const shots = await tools.ls({path: 'shots'});\nreturn shots;" } },
            "done": true, "output": "Script completed\nOutput:\nf19.png\nm1.png",
        }),
        "m1" => json!({
            "call": { "kind": "mcp", "server": "acme", "tool": "lookup_order", "input": { "order": "A-1042", "region": "eu" } },
            "done": true, "output": "{\"status\":\"shipped\",\"eta\":\"2026-11-20\"}",
        }),
        _ => Json::Null,
    }
}

struct Fixture {
    app: TestApp,
    asked: Rc<RefCell<Vec<String>>>,
}

fn mount() -> Fixture {
    let entry = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/tool_open_fixture.luau");
    let picture = std::env::temp_dir().join(format!("zgx-tool-open-{}.png", std::process::id()));
    {
        use base64::Engine as _;
        let bytes = base64::engine::general_purpose::STANDARD.decode(PNG_1X1).unwrap();
        std::fs::write(&picture, bytes).unwrap();
    }
    let asked = Rc::new(RefCell::new(Vec::new()));
    let heard = asked.clone();
    let mut app = TestApp::new(move |runtime| {
        let host = ScriptHost::new(runtime, entry);
        let heard_exchange = heard.clone();
        host.async_action("chat.tool_exchange", move |arg| {
            let chat = crate::auth::text(&arg.field("chat").cloned().unwrap_or(Value::Nil));
            let id = crate::auth::text(&arg.field("id").cloned().unwrap_or(Value::Nil));
            heard_exchange.borrow_mut().push(format!("exchange:{chat}:{id}"));
            async move { Ok(crate::tool_open::exchange_value(&exchange(&id), &chat, &id)) }
        });
        let heard_image = heard.clone();
        let src = picture.to_string_lossy().into_owned();
        host.async_action("files.tool_image", move |arg| {
            let id = crate::auth::text(&arg.field("id").cloned().unwrap_or(Value::Nil));
            heard_image.borrow_mut().push(format!("image:{id}"));
            let src = src.clone();
            async move { Ok(Value::record([("src", Value::from(src.as_str())), ("size", Value::Num(70.))])) }
        });
        host
    });
    app.ui.scene.borrow_mut().resize(900., 1400.);
    app.frames(5);
    let row = crate::chat::built_entry(&reply(), "doc-7", None, Some("Opus")).expect("a row");
    let tools = row.field("tools").cloned().expect("the reply's tools");
    app.host.shared("test.tool_open.tools", Value::Nil).set(tools);
    app.frames(20);
    assert_eq!(app.error(), None, "the fixture failed");
    // (A settled turn's work is folded under its line: opened, as the
    // reader opens it.)
    let group = r#"role:button "Read 1 file · ran 2 commands · edited 1 file""#;
    assert!(app.find(group).is_some(), "no group:
{}", app.tree());
    app.click(group);
    app.frames(30);
    Fixture { app, asked }
}

fn lines_with(app: &mut TestApp, words: &[&str]) -> String {
    app.tree().lines().filter(|l| words.iter().any(|w| l.contains(w))).collect::<Vec<_>>().join("\n")
}

/// Opens the row labelled `label` and waits for `found` in it.
fn open(f: &mut Fixture, label: &str, found: &str) {
    let row = format!("role:button \"{label}\"");
    assert!(f.app.find(&row).is_some(), "no row {label:?}:\n{}", lines_with(&mut f.app, &["button"]));
    f.app.click(&row);
    let came = f.app.wait_until(|app| app.find(found).is_some());
    assert!(came, "{label:?} didn't open to {found}; asked {:?}\n{}", f.asked.borrow(), f.app.tree());
    assert_eq!(f.app.error(), None);
}

#[test]
fn a_pi_read_of_a_png_opens_to_the_picture() {
    let mut f = mount();
    open(&mut f, "Read f19.png", ".tool-open-image");
    // (The reply's picture frame, ds/reply_picture: a press opens it in the
    // lightbox; the picture drawn in it.)
    let shown = f.app.wait_until(|app| {
        let tree = app.tree();
        let mut lines = tree.lines().skip_while(|l| !l.contains(r#"button "Open f19.png""#));
        lines.next().is_some() && lines.next().is_some_and(|l| l.trim_start().split(' ').nth(1) == Some("img"))
    });
    assert!(shown, "no image node:\n{}", f.app.tree());
    assert!(f.asked.borrow().contains(&"exchange:doc-7:r1".to_owned()), "{:?}", f.asked.borrow());
    assert!(f.asked.borrow().contains(&"image:r1".to_owned()), "{:?}", f.asked.borrow());
}

#[test]
fn a_pi_edit_opens_to_a_diff_with_lines_added_and_removed() {
    let mut f = mount();
    open(&mut f, "Edited app.rs", ".tool-diff");
    let diff = lines_with(&mut f.app, &["old_call", "new_call", "more()"]);
    assert!(diff.contains("old_call") && diff.contains("new_call") && diff.contains("more()"), "{diff}");
    let tree = f.app.tree();
    let signs = |s: &str| tree.lines().filter(|l| l.trim_start().contains(&format!("\"{s}\""))).count();
    assert!(signs("+") >= 2 && signs("-") >= 1, "+{} -{}\n{}", signs("+"), signs("-"), lines_with(&mut f.app, &["tool-diff", "\"+\"", "\"-\""]));
}

#[test]
fn codemode_opens_to_its_script_as_code_then_its_result() {
    let mut f = mount();
    open(&mut f, "Used codemode", ".tool-open-script");
    let script = lines_with(&mut f.app, &["tool-open-script", "tools.ls", "javascript"]);
    assert!(script.contains("tools.ls"), "the script isn't shown:\n{script}");
    assert!(script.contains("javascript"), "not as JavaScript:\n{script}");
    let result = lines_with(&mut f.app, &["m1.png"]);
    assert!(result.contains("m1.png"), "its result isn't shown:\n{}", f.app.tree());
}

#[test]
fn an_unknown_mcp_tool_opens_to_its_args_and_its_result() {
    let mut f = mount();
    open(&mut f, "Used acme.lookup_order", ".tool-open-fields");
    let args = lines_with(&mut f.app, &["tool-open-field", "A-1042", "region"]);
    assert!(args.contains("A-1042") && args.contains("region"), "its args aren't shown:\n{args}");
    let result = lines_with(&mut f.app, &["shipped", "2026-11-20"]);
    assert!(result.contains("shipped") && result.contains("2026-11-20"), "its result isn't shown:\n{}", f.app.tree());
}

/// One tool's row alone (ui/tool_open_one_fixture.luau), its host's answers
/// the test's: `exchange` answers each ReadToolExchange in turn, the first
/// held until `release` (as a slow host does), `image` each picture read.
struct One {
    app: TestApp,
    asked: Rc<RefCell<Vec<String>>>,
    release: Rc<RefCell<Option<tokio::sync::oneshot::Sender<()>>>>,
}

fn mount_one(exchanges: Vec<Json>, hold_first: bool, images: Vec<Result<(), String>>) -> One {
    let entry = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/tool_open_one_fixture.luau");
    let picture = std::env::temp_dir().join(format!("zgx-tool-open-one-{}.png", std::process::id()));
    {
        use base64::Engine as _;
        std::fs::write(&picture, base64::engine::general_purpose::STANDARD.decode(PNG_1X1).unwrap()).unwrap();
    }
    let asked = Rc::new(RefCell::new(Vec::new()));
    let release = Rc::new(RefCell::new(None));
    let (heard, held) = (asked.clone(), release.clone());
    let app = TestApp::new(move |runtime| {
        let host = ScriptHost::new(runtime, entry);
        let answers = Rc::new(RefCell::new(std::collections::VecDeque::from(exchanges)));
        let heard_exchange = heard.clone();
        host.async_action("chat.tool_exchange", move |arg| {
            let chat = crate::auth::text(&arg.field("chat").cloned().unwrap_or(Value::Nil));
            let id = crate::auth::text(&arg.field("id").cloned().unwrap_or(Value::Nil));
            let first = heard_exchange.borrow().iter().all(|a: &String| !a.starts_with("exchange"));
            heard_exchange.borrow_mut().push(format!("exchange:{id}"));
            let answer = answers.borrow_mut().pop_front().unwrap_or(Json::Null);
            let gate = (first && hold_first).then(|| {
                let (tx, rx) = tokio::sync::oneshot::channel::<()>();
                *held.borrow_mut() = Some(tx);
                rx
            });
            async move {
                if let Some(gate) = gate {
                    let _ = gate.await;
                }
                Ok(crate::tool_open::exchange_value(&answer, &chat, &id))
            }
        });
        let reads = Rc::new(RefCell::new(std::collections::VecDeque::from(images)));
        let heard_image = heard.clone();
        let src = picture.to_string_lossy().into_owned();
        host.async_action("files.tool_image", move |_arg| {
            heard_image.borrow_mut().push("image".into());
            let answer = reads.borrow_mut().pop_front().unwrap_or(Ok(()));
            let src = src.clone();
            async move {
                answer?;
                Ok(Value::record([("src", Value::from(src.as_str())), ("size", Value::Num(70.))]))
            }
        });
        host
    });
    app.ui.scene.borrow_mut().resize(900., 1400.);
    app.frames(5);
    assert_eq!(app.error(), None, "the fixture failed");
    One { app, asked, release }
}

fn set_tool(app: &TestApp, live: bool) {
    let tool = Value::record([
        ("id", Value::from("x1")),
        ("chat", Value::from("doc-7")),
        ("kind", Value::from("tool")),
        ("call_kind", Value::from("unknown")),
        ("live", Value::Bool(live)),
    ]);
    app.host.shared("test.tool_open.tool", Value::Nil).set(tool);
}

/// Opened while the tool runs; it settles while that first request is still
/// out, whose answer then says it runs: the row asks again and shows the
/// result, not "hasn't finished yet" forever.
#[test]
fn a_tool_that_settles_while_its_first_request_is_out_is_asked_again() {
    let running = json!({ "call": { "kind": "unknown", "name": "slow_tool" }, "done": false });
    let finished = json!({ "call": { "kind": "unknown", "name": "slow_tool" }, "done": true, "output": "finished-output-42" });
    let mut one = mount_one(vec![running, finished], true, vec![]);
    set_tool(&one.app, true);
    one.app.frames(10);
    assert_eq!(*one.asked.borrow(), ["exchange:x1"], "asked as it opens");
    // It settles; the first answer hasn't come.
    set_tool(&one.app, false);
    one.app.frames(10);
    let _ = one.release.borrow_mut().take().expect("the first request is held").send(());
    let shown = one.app.wait_until(|app| app.find(r#""finished-output-42""#).is_some());
    assert!(shown, "asked {:?}\n{}", one.asked.borrow(), one.app.tree());
    assert_eq!(*one.asked.borrow(), ["exchange:x1", "exchange:x1"]);
}

/// A picture whose first read failed (the host was busy, offline a moment)
/// is read again when its retry is due, and then shows.
#[test]
fn a_picture_whose_read_failed_is_read_again() {
    let read = json!({ "call": { "kind": "readFile", "path": "/home/me/m1.png" }, "done": true, "output": "Read image file [image/png]", "image": "image/png" });
    let mut one = mount_one(vec![read], false, vec![Err("the device is busy".into())]);
    set_tool(&one.app, false);
    let failed = one.app.wait_until(|app| app.tree().contains("the device is busy") || app.find(".tool-open-image").is_some());
    assert!(failed, "{}", one.app.tree());
    assert_eq!(one.asked.borrow().iter().filter(|a| *a == "image").count(), 1);
    // Past the first wait (3 s, on the clock `ui.now()` reads, the wall's),
    // it's read again.
    std::thread::sleep(std::time::Duration::from_millis(3100));
    one.app.skip_frames(4 * 60);
    let shown = one.app.wait_until(|app| {
        let tree = app.tree();
        let mut lines = tree.lines().skip_while(|l| !l.contains(r#"button "Open m1.png""#));
        lines.next().is_some() && lines.next().is_some_and(|l| l.trim_start().split(' ').nth(1) == Some("img"))
    });
    assert!(shown, "asked {:?}\n{}", one.asked.borrow(), one.app.tree());
    assert_eq!(one.asked.borrow().iter().filter(|a| *a == "image").count(), 2);
}
