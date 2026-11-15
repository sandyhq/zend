//! A visual's answer button (`ui.answer`, plans/live-visuals G2) in a
//! chat (ui/chat_fixture.luau, session `sv`), on the real visuals host:
//! the inputs go into that chat's composer as words (crates/visual
//! `answer_text`, design D8), under what the reader wrote, nothing is
//! sent, and a second press replaces its own block instead of adding one.

use zgx_script::{ScriptHost, Value, testing::TestApp};

const COMPOSER: &str = "#chat-composer >> role:textbox \"Message\"";
const ANSWER: &str = "role:button \"Add to message\"";

fn mount() -> TestApp {
    let entry = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/chat_fixture.luau");
    let mut app = TestApp::new(|runtime| {
        let host = ScriptHost::new(runtime, entry);
        crate::visuals::register(&host);
        host
    });
    app.ui.scene.borrow_mut().resize(900., 1400.);
    app.frames(10);
    app.host.run_command("go_sv");
    app.frames(30);
    // (Pinned over the composer once it has drawn: once there, only there.)
    let settled = app.wait_until(|app| app.target(ANSWER).is_ok() && app.find(".visual-pinned-marker").is_some());
    if !settled {
        let tree = app.tree();
        let lines: Vec<&str> = tree.lines().filter(|l| l.contains("visual") || l.contains("Add to message")).collect();
        panic!("not pinned: {:?}\n{}", app.target(ANSWER), lines.join("\n"));
    }
    assert_eq!(app.error(), None, "the fixture failed");
    if std::env::var("G4_DEBUG").is_ok() {
        let b = app.bounds(ANSWER);
        let p = app.bounds("#chat-pinned-visual");
        eprintln!("ANSWER {b:?} PIN {p:?}\n{}", app.tree().lines().filter(|l| l.contains("pinned") || l.contains("Add to") || l.contains("scroll")).collect::<Vec<_>>().join("\n"));
    }
    app
}

fn draft(app: &TestApp) -> String {
    match app.host.shared("chat.composer.text", Value::Nil).get() {
        Value::Str(s) => s.to_string(),
        other => crate::auth::text(&other),
    }
}

/// What the page asked of the host (the fixture's `calls`).
fn calls(app: &TestApp) -> String {
    let found = app.host.states().into_iter().find(|(name, _)| name == "calls");
    found.map_or_else(String::new, |(_, v)| v.to_json())
}

/// Open (it asks, nothing sent), the visual is pinned above the composer
/// and its place in the transcript is a line; sent in a message holding its
/// block, it is answered: it says so, asks no more and goes back into the
/// transcript (plans/live-visuals G4).
#[test]
fn an_open_visual_is_pinned_and_a_message_holding_its_answers_marks_it_sent() {
    let mut app = mount();
    app.wait_until(|app| app.find("#chat-pinned-visual >> role:button \"Add to message\"").is_some());
    assert!(app.find(".visual-pinned-marker").is_some(), "no line in the transcript");
    app.type_text("role:textbox \"Goal\"", "Move the db");
    app.frames(10);
    app.wait_until(|app| crate::visuals::entries_of("v:fixture-answer").field("goal").is_some());
    app.click(ANSWER);
    app.frames(10);
    assert!(draft(&app).contains("Goal: Move the db"), "{:?}", draft(&app));
    app.click(COMPOSER);
    app.keys("Enter");
    app.frames(20);
    app.wait_until(|app| calls(app).contains("send:sv:"));
    assert!(calls(&app).contains("Goal: Move the db"), "{}", calls(&app));
    app.wait_until(|app| app.find(".visual-sent").is_some());
    app.frames(10);
    assert!(app.find("#chat-pinned-visual").is_none(), "still pinned");
    assert!(app.find(".visual-pinned-marker").is_none(), "still a line");
    assert!(app.find(ANSWER).is_none(), "it still asks");
    assert!(
        app.find("#chat-composer >> .visual-sent").is_none(),
        "sent, it shows in the transcript, not over the composer"
    );
    assert_eq!(app.error(), None);
}

/// A message that doesn't hold the block (the reader deleted it) leaves the
/// visual open.
#[test]
fn a_message_without_the_block_leaves_it_open() {
    let mut app = mount();
    app.type_text(COMPOSER, "Something else.");
    app.frames(5);
    app.keys("Enter");
    app.frames(20);
    app.wait_until(|app| calls(app).contains("send:sv:"));
    app.frames(10);
    assert!(app.find(".visual-sent").is_none());
    assert!(app.find("#chat-pinned-visual").is_some());
}

#[test]
fn the_answers_go_under_the_draft_unsent_and_a_second_press_replaces_them() {
    let mut app = mount();
    app.type_text(COMPOSER, "My own words first.");
    app.frames(5);
    app.type_text("role:textbox \"Goal\"", "Move the db");
    app.click("role:checkbox \"Back up\"");
    app.click("role:radio \"This weekend\"");
    app.frames(10);
    app.wait_until(|app| crate::visuals::entries_of("v:fixture-answer").field("when").is_some());

    app.click(ANSWER);
    app.frames(10);
    let block = |when: &str| {
        format!("### Migration plan\nGoal: Move the db\nSteps:\n- [x] Back up\n- [ ] Verify\nWhen: {when}")
    };
    let first = format!("My own words first.\n\n{}", block("This weekend"));
    assert_eq!(draft(&app), first);
    assert!(!calls(&app).contains("send:"), "something was sent: {}", calls(&app));

    // Again, nothing changed: the same draft, not a second block.
    app.click(ANSWER);
    app.frames(10);
    assert_eq!(draft(&app), first);

    // A changed answer replaces its own block; the reader's words stay.
    app.click("role:radio \"Tonight\"");
    app.wait_until(|app| {
        crate::visuals::entries_of("v:fixture-answer").field("when").map(crate::auth::text).as_deref() == Some("Tonight")
    });
    app.click(ANSWER);
    app.frames(10);
    assert_eq!(draft(&app), format!("My own words first.\n\n{}", block("Tonight")));

    // Words the reader adds after it stay too: the block is still there as
    // it was, so it's the one replaced.
    app.click(COMPOSER);
    app.keys("cmd+End");
    app.ui.dispatch(zgx::input::InputEvent::Text(" Thanks.".into()));
    app.settle();
    app.frames(5);
    let edited = draft(&app);
    assert!(edited.ends_with("When: Tonight Thanks."), "{edited:?}");
    app.click(ANSWER);
    app.frames(10);
    assert_eq!(draft(&app), edited);
    assert!(!calls(&app).contains("send:"), "something was sent: {}", calls(&app));
    assert_eq!(app.error(), None);
}
