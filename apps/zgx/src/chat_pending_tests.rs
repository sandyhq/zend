//! Prompts waiting for the engine must outlive navigation between chats.

use super::*;
use std::{
    future::Future,
    pin::Pin,
    task::{Context, Waker},
};
use zgx_script::testing::TestApp;

type Sending = Pin<Box<dyn Future<Output = Result<(), Outcome>>>>;

fn pending_chat() -> (TestApp, Rc<View>, Sending) {
    let app = TestApp::new(|runtime| {
        ScriptHost::new(
            runtime,
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/activity_fixture.luau"),
        )
    });
    let view = Rc::new(View {
        session: app.host.shared("test.session", Value::Nil),
        messages: app.host.shared("test.messages", Value::Nil),
        draft: app.host.shared("test.draft", Value::Nil),
        rpc: RefCell::default(),
        open: RefCell::default(),
        pending: RefCell::default(),
        subagents: app.host.shared("test.subagents", Value::Nil),
        generation: Cell::new(0),
        snapshot: Cell::new(false),
    });
    CHATS.with(|chats| {
        chats.made(
            "new-chat",
            json!({ "id": "new-chat", "config": { "harness": "codex" } }),
        );
        chats.made("other-chat", json!({ "id": "other-chat" }));
    });
    view.open_chat(Some("new-chat".into()));
    let sender = view.clone();
    let mut sending: Sending = Box::pin(async move {
        sender
            .send(Value::record([
                ("session", Value::from("new-chat")),
                ("message_id", Value::from("first-message")),
                ("text", Value::from("Find the startup bug")),
                ("fresh", Value::Bool(true)),
                ("harness", Value::from("codex")),
                (
                    "attachments",
                    Value::List(
                        vec![Value::record([
                            ("path", Value::from("/tmp/startup.txt")),
                            ("on_host", Value::Bool(true)),
                        ])]
                        .into(),
                    ),
                ),
            ]))
            .await
    });
    // No engine connection in this test: stop at the first asynchronous
    // handoff, with the real sender's prompt waiting to be persisted.
    assert!(
        sending
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    view.heard_frame(json!({ "reset": [] }));
    (app, view, sending)
}

fn messages(view: &View) -> Json {
    view.messages
        .with_untracked(|value| serde_json::from_str(&value.to_json()).unwrap())
}

#[test]
fn first_prompt_survives_switching_chats_before_the_engine_saves_it() {
    let (_app, view, _sending) = pending_chat();
    let original = messages(&view);
    assert_eq!(original[0]["id"], "first-message");

    view.open_chat(Some("other-chat".into()));
    view.heard_frame(json!({ "reset": [] }));
    assert_eq!(
        messages(&view),
        json!([]),
        "the prompt belongs only to its chat"
    );

    view.open_chat(Some("new-chat".into()));
    assert_eq!(
        messages(&view),
        original,
        "restore the prompt before the next transcript frame"
    );
    view.heard_frame(json!({ "reset": [] }));
    assert_eq!(
        messages(&view),
        original,
        "the first prompt and its attachments must survive navigation"
    );

    view.open_chat(None);
    view.open_chat(Some("new-chat".into()));
    view.heard_frame(json!({ "reset": [] }));
    assert_eq!(
        messages(&view),
        original,
        "visiting the new-chat page must also preserve the prompt"
    );

    let saved = json!({ "id": "first-message", "role": "user", "createdAt": now_seconds() * 1000,
        "parts": [{ "id": "text", "kind": "text", "text": "Find the startup bug" }] });
    view.heard_frame(json!({ "reset": [saved] }));
    assert_eq!(
        messages(&view).as_array().unwrap().len(),
        1,
        "the engine's copy replaces the local prompt"
    );
    assert!(!view.pending.borrow().contains_key("new-chat"));
    // Once acknowledged, a later empty transcript must not resurrect it.
    view.heard_frame(json!({ "reset": [] }));
    assert_eq!(messages(&view), json!([]));
}

#[test]
fn a_send_that_fails_while_away_keeps_its_prompt_and_retry() {
    let (_app, view, _sending) = pending_chat();
    view.open_chat(Some("other-chat".into()));
    let run = json!({ "kind": "run", "messageId": "first-message", "request": { "prompt": "Find the startup bug" } });
    view.mark_failed("new-chat", "first-message", Some(run.clone()));
    assert_eq!(messages(&view), json!([]));

    view.open_chat(Some("new-chat".into()));
    view.heard_frame(json!({ "reset": [] }));
    assert_eq!(messages(&view)[0]["waiting"], "failed");
    assert_eq!(view.pending.borrow()["new-chat"][0].retry, Some(run));

    let mut retry = Box::pin(view.retry(Value::record([
        ("id", Value::from("new-chat")),
        ("message", Value::from("first-message")),
    ])));
    assert!(
        retry
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    assert_eq!(messages(&view)[0]["waiting"], "sending");
}

#[test]
fn a_saved_first_prompt_stays_visible_after_the_sending_indicator_expires() {
    let (_app, view, _sending) = pending_chat();
    view.open_chat(Some("other-chat".into()));
    view.open_chat(Some("new-chat".into()));
    view.heard_frame(json!({ "reset": [{
        "id": "first-message", "role": "user",
        "createdAt": (now_seconds() - SEND_GRACE_SECONDS as i64 - 1) * 1000,
        "parts": [{ "id": "text", "kind": "text", "text": "Find the startup bug" }]
    }] }));
    assert_eq!(messages(&view)[0]["text"], "Find the startup bug");
    assert!(messages(&view)[0]["waiting"].is_null());
    assert!(!view.pending.borrow().contains_key("new-chat"));
}

fn held_send(id: &str, failure: Option<&str>) -> Json {
    json!({
        "id": format!("send-{id}"),
        "chatId": "other-chat",
        "messageId": id,
        "prompt": "look at this\n\nAttached images (local files \u{2014} open them to view):\n- pending://u1/a.png",
        "createdAt": 1_000,
        "failure": failure,
    })
}

#[test]
fn a_send_in_the_engines_outbox_shows_even_after_a_restart() {
    let (_app, view, _sending) = pending_chat();
    view.open_chat(Some("other-chat".into()));
    view.heard_frame(json!({ "reset": [] }));

    // Nothing here sent it: this window only hears of it from the engine.
    view.follow_held(&[held_send("m1", None)]);
    let shown = messages(&view);
    assert_eq!(shown[0]["id"], "m1");
    assert_eq!(shown[0]["text"], "look at this");
    assert_eq!(shown[0]["waiting"], "sending");

    // Hearing the same again adds nothing.
    view.follow_held(&[held_send("m1", None)]);
    assert_eq!(messages(&view).as_array().unwrap().len(), 1);
}

#[test]
fn a_failed_send_in_the_outbox_reads_failed_and_comes_back() {
    let (_app, view, _sending) = pending_chat();
    view.open_chat(Some("other-chat".into()));
    view.heard_frame(json!({ "reset": [] }));
    view.follow_held(&[held_send("m1", Some("the file is too large"))]);
    assert_eq!(messages(&view)[0]["waiting"], "failed");
    {
        let pending = view.pending.borrow();
        let prompt = &pending["other-chat"][0];
        assert!(prompt.held, "the engine retries it, not a command from here");
        assert!(prompt.retry.is_none());
    }
    view.follow_held(&[held_send("m1", None)]);
    assert_eq!(messages(&view)[0]["waiting"], "sending");
}

#[test]
fn a_send_leaving_the_outbox_waits_for_the_transcript() {
    let (_app, view, _sending) = pending_chat();
    view.open_chat(Some("other-chat".into()));
    view.heard_frame(json!({ "reset": [] }));
    view.follow_held(&[held_send("m1", None)]);
    view.follow_held(&[]);
    assert_eq!(messages(&view)[0]["waiting"], "sending", "it has only just gone out");
    assert!(!view.pending.borrow()["other-chat"][0].held);
    let saved = json!({ "id": "m1", "role": "user", "createdAt": now_seconds() * 1000,
        "parts": [{ "id": "text", "kind": "text", "text": "look at this" }] });
    view.heard_frame(json!({ "reset": [saved] }));
    assert!(!view.pending.borrow().contains_key("other-chat"));
}

#[test]
fn a_row_leaving_the_outbox_is_the_queues_to_show() {
    let (_app, view, _sending) = pending_chat();
    view.open_chat(Some("other-chat".into()));
    view.heard_frame(json!({ "reset": [] }));
    let mut row = held_send("r1", None);
    row["queueRow"] = json!(true);
    view.follow_held(&[row]);
    assert_eq!(messages(&view)[0]["id"], "r1");
    view.follow_held(&[]);
    assert_eq!(messages(&view), json!([]));
}

#[test]
fn only_a_cloud_chat_on_an_engine_that_knows_the_store_goes_through_it() {
    let knows = json!(["message-queue-v1", "attachment-store-v1"]);
    let older = json!(["message-queue-v1"]);
    assert!(store_takes_files(Some("cloud-machine"), &knows));
    assert!(!store_takes_files(Some("cloud-machine"), &older));
    assert!(!store_takes_files(Some("cloud-machine"), &Json::Null));
    assert!(!store_takes_files(Some("desktop-at-home"), &knows));
    assert!(!store_takes_files(None, &knows));
}

#[test]
fn a_stored_file_is_named_the_way_the_engine_resolves_it() {
    let (reference, transfer) = stored_file("u1", "/Users/me/shots/a b.png");
    assert_eq!(reference, "pending://u1/a b.png");
    assert_eq!(transfer, json!({ "uploadId": "u1", "fileName": "a b.png" }));
    let prompt = with_attachments("look", std::slice::from_ref(&reference), 1);
    assert_eq!(split_attachments(&prompt), ("look".to_owned(), vec![reference]));
}

// --- A prompt that didn't go (undelivered) ---------------------------------

fn poll_once<T>(future: impl Future<Output = T>) -> std::task::Poll<T> {
    let mut future = Box::pin(future);
    future.as_mut().poll(&mut Context::from_waker(Waker::noop()))
}

fn ids(view: &View) -> Vec<String> {
    messages(view).as_array().unwrap().iter().map(|m| m["id"].as_str().unwrap().to_owned()).collect()
}

/// What the engine has of a chat that went on after the failed prompt: one
/// that was delivered, and its answer, both sent after it.
fn went_on() -> Vec<Json> {
    let later = now_millis() + 60_000;
    vec![
        json!({ "id": "second", "role": "user", "createdAt": later,
            "parts": [{ "id": "text", "kind": "text", "text": "Here is the folder path instead" }] }),
        json!({ "id": "answer", "role": "assistant", "status": "done", "createdAt": later + 1000,
            "parts": [{ "id": "text", "kind": "text", "text": "I read the folder." }] }),
    ]
}

#[test]
fn a_failed_prompt_stays_where_it_was_sent_not_at_the_end() {
    let (_app, view, _sending) = pending_chat();
    view.mark_failed("new-chat", "first-message", None);
    view.heard_frame(json!({ "reset": went_on() }));
    assert_eq!(ids(&view), ["first-message", "second", "answer"], "newer messages go below the failed one");
    assert_eq!(messages(&view)[0]["waiting"], "failed");
    assert!(messages(&view)[1]["waiting"].is_null());
}

#[test]
fn a_prompt_still_on_its_way_stays_the_newest() {
    let (_app, view, _sending) = pending_chat();
    view.heard_frame(json!({ "reset": went_on() }));
    assert_eq!(ids(&view), ["second", "answer", "first-message"]);
}

#[test]
fn discarding_a_failed_prompt_takes_it_away_for_good() {
    let (_app, view, _sending) = pending_chat();
    view.mark_failed("new-chat", "first-message", None);
    view.heard_frame(json!({ "reset": went_on() }));
    let args = || {
        Value::record([("id", Value::from("new-chat")), ("message", Value::from("first-message"))])
    };
    // One still on its way isn't a failed one: nothing to discard.
    assert!(matches!(poll_once(view.discard(Value::record([("id", Value::from("new-chat")), ("message", Value::from("second"))]))), std::task::Poll::Ready(Err(_))));
    assert!(matches!(poll_once(view.discard(args())), std::task::Poll::Ready(Ok(()))));
    assert_eq!(ids(&view), ["second", "answer"]);
    // The chat opened again (away and back, or its transcript heard anew):
    // it doesn't come back.
    view.open_chat(Some("other-chat".into()));
    view.open_chat(Some("new-chat".into()));
    view.heard_frame(json!({ "reset": went_on() }));
    assert_eq!(ids(&view), ["second", "answer"]);
    assert!(!view.pending.borrow().contains_key("new-chat"));
}

#[test]
fn a_failed_send_held_by_the_engine_is_found_again_to_take_out() {
    let (_app, view, _sending) = pending_chat();
    view.open_chat(Some("other-chat".into()));
    view.heard_frame(json!({ "reset": [] }));
    view.follow_held(&[held_send("m1", Some("the file is too large"))]);
    assert_eq!(view.pending.borrow()["other-chat"][0].item.as_deref(), Some("send-m1"));
}

fn big_file(name: &str, bytes: u64) -> String {
    let path = std::env::temp_dir().join(format!("zgx-undelivered-{}-{name}", std::process::id()));
    std::fs::File::create(&path).unwrap().set_len(bytes).unwrap();
    path.to_string_lossy().into_owned()
}

fn send_of(message: &str, text: &str, files: &[(&str, &str)]) -> Value {
    Value::record([
        ("session", Value::from("new-chat")),
        ("message_id", Value::from(message)),
        ("text", Value::from(text)),
        ("fresh", Value::Bool(true)),
        ("harness", Value::from("codex")),
        (
            "attachments",
            Value::List(
                files
                    .iter()
                    .map(|(path, name)| Value::record([("path", Value::from(*path)), ("name", Value::from(*name))]))
                    .collect::<Vec<_>>()
                    .into(),
            ),
        ),
    ])
}

#[test]
fn a_send_with_a_file_over_the_cap_fails_at_once_naming_it() {
    let (_app, view, _sending) = pending_chat();
    let mb = 1024 * 1024;
    let (small, large, larger) = (big_file("small.json", 2 * mb), big_file("large.json", 32 * mb), big_file("larger.json", 45 * mb));
    let sent = poll_once(view.send(send_of(
        "big",
        "all my conversations",
        &[(&small, "small.json"), (&large, "large.json"), (&larger, "larger.json")],
    )));
    for path in [&small, &large, &larger] {
        let _ = std::fs::remove_file(path);
    }
    assert!(matches!(sent, std::task::Poll::Ready(Err("too_large"))), "{sent:?}");
    let shown = messages(&view);
    let failed = shown.as_array().unwrap().iter().find(|m| m["id"] == "big").unwrap();
    assert_eq!(failed["waiting"], "failed");
    assert_eq!(
        failed["failure"],
        json!({ "kind": "too_large", "count": 2, "name": "larger.json", "size": 45, "limit": 24 })
    );
}

#[test]
fn a_file_that_cant_be_read_is_named_too() {
    let (_app, view, _sending) = pending_chat();
    let sent = poll_once(view.send(send_of("gone", "look", &[("/nonexistent/zgx/notes.txt", "notes.txt")])));
    assert!(matches!(sent, std::task::Poll::Ready(Err("failed"))));
    let shown = messages(&view);
    let failed = shown.as_array().unwrap().iter().find(|m| m["id"] == "gone").unwrap();
    assert_eq!(failed["failure"], json!({ "kind": "unreadable", "name": "notes.txt" }));
}

#[test]
fn send_again_on_a_prompt_whose_files_failed_runs_the_send_again() {
    let (_app, view, _sending) = pending_chat();
    let large = big_file("again.json", 30 * 1024 * 1024);
    let first = poll_once(view.send(send_of("big", "files", &[(&large, "again.json")])));
    assert!(matches!(first, std::task::Poll::Ready(Err("too_large"))));
    let args = Value::record([("id", Value::from("new-chat")), ("message", Value::from("big"))]);
    let again = poll_once(view.retry(args));
    let _ = std::fs::remove_file(&large);
    // (It ran again, and failed the same way, saying so: the row isn't
    // thrown away by asking for it to be sent.)
    assert!(matches!(again, std::task::Poll::Ready(Err("too_large"))), "{again:?}");
    let shown = messages(&view);
    let failed = shown.as_array().unwrap().iter().find(|m| m["id"] == "big").unwrap();
    assert_eq!(failed["waiting"], "failed");
    assert_eq!(failed["failure"]["kind"], "too_large");
}

#[test]
fn sending_the_same_words_again_replaces_the_failed_prompt() {
    let (_app, view, _sending) = pending_chat();
    view.mark_failed("new-chat", "first-message", None);
    // The words as they were, the files taken off: sent again.
    let again = view.send(send_of("second-try", "Find the startup bug", &[]));
    let _ = poll_once(again);
    assert_eq!(ids(&view), ["second-try"], "the failed one is gone");
}

#[test]
fn other_words_leave_the_failed_prompt_in_place() {
    let (_app, view, _sending) = pending_chat();
    view.mark_failed("new-chat", "first-message", None);
    let _ = poll_once(view.send(send_of("other", "Something else", &[])));
    assert_eq!(ids(&view), ["first-message", "other"]);
}

