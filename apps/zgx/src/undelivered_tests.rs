//! A prompt that didn't go (plans/live-visuals, undelivered): it says why,
//! offers Send again and Discard, and stays where it was sent.
//!
//! The drawing is the page's (ui/chat_fixture.luau, chat `uf`); where the
//! host puts it in the list, and what Discard does to it, is the view's
//! (src/chat_pending_tests.rs).

use zgx_script::{ScriptHost, testing::TestApp};

fn mount() -> TestApp {
    let entry = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/chat_fixture.luau");
    let mut app = TestApp::new(|runtime| ScriptHost::new(runtime, entry));
    app.ui.scene.borrow_mut().resize(900., 900.);
    app.frames(10);
    app.host.run_command("go_uf");
    app.frames(30);
    let drawn = app.wait_until(|app| app.find("#undelivered-why").is_some());
    assert!(drawn, "the failed prompt isn't drawn:\n{}", app.tree());
    assert_eq!(app.error(), None, "the fixture failed");
    app
}

fn calls(app: &TestApp) -> String {
    let found = app.host.states().into_iter().find(|(name, _)| name == "calls");
    found.map_or_else(String::new, |(_, v)| v.to_json())
}

#[test]
fn a_failed_prompt_says_why_and_offers_send_again_and_discard() {
    let mut app = mount();
    let tree = app.tree();
    assert!(tree.contains("Too large: 2 files are over the limit of 24 MB per file"), "{tree}");
    // (Held to the message's width, wrapping, not run past it.)
    let (why, bubble) = (app.bounds("#undelivered-why"), app.bounds("#uf1"));
    assert!(why.width <= bubble.width, "{why:?} wider than {bubble:?}");
    assert!(app.find("#undelivered-retry").is_some(), "no Send again");
    assert!(app.find("#undelivered-discard").is_some(), "no Discard");
}

#[test]
fn send_again_and_discard_ask_the_host_for_that_prompt() {
    let mut app = mount();
    app.click("#undelivered-retry");
    app.frames(5);
    assert!(calls(&app).contains("retry:uf:uf1"), "{}", calls(&app));
    app.click("#undelivered-discard");
    app.frames(10);
    assert!(calls(&app).contains("discard:uf:uf1"), "{}", calls(&app));
    assert!(app.find("#undelivered-why").is_none(), "still there after Discard");
    assert_eq!(app.error(), None);
}
