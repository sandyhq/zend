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
    // (In its place in the transcript, in view: not pinned.)
    let settled = app.wait_until(|app| app.find(&format!("#transcript >> {ANSWER}")).is_some());
    if !settled {
        let tree = app.tree();
        let lines: Vec<&str> = tree.lines().filter(|l| l.contains("visual") || l.contains("Add to message")).collect();
        panic!("not drawn in place: {:?}\n{}", app.target(ANSWER), lines.join("\n"));
    }
    assert_eq!(app.error(), None, "the fixture failed");
    if std::env::var("G4_DEBUG").is_ok() {
        let b = app.bounds(ANSWER);
        let p = app.bounds("#chat-pinned-visual");
        eprintln!("ANSWER {b:?} PIN {p:?}\n{}", app.tree().lines().filter(|l| l.contains("pinned") || l.contains("Add to") || l.contains("scroll")).collect::<Vec<_>>().join("\n"));
    }
    app
}

/// The chat moves on (the fixture's `sv_more`): the visual's place
/// scrolls out of view and it is pinned above the composer.
fn pin(app: &mut TestApp) {
    app.host.run_command("sv_more");
    app.frames(30);
    let pinned = app.wait_until(|app| {
        app.find(&format!("#chat-pinned-visual >> {ANSWER}")).is_some() && app.find(".visual-pinned-marker").is_some()
    });
    assert!(pinned, "not pinned once scrolled away:\n{}", visual_lines(app));
}

/// The transcript scrolled back up to the visual's place.
fn back(app: &mut TestApp) {
    for _ in 0..10 {
        if app.find("#chat-pinned-visual").is_none() {
            break;
        }
        app.scroll("#transcript", -4000.);
        app.frames(10);
    }
    let returned = app.wait_until(|app| app.find("#chat-pinned-visual").is_none() && app.find(&format!("#transcript >> {ANSWER}")).is_some());
    assert!(returned, "not back in its place:\n{}", visual_lines(app));
}

fn visual_lines(app: &mut TestApp) -> String {
    let tree = app.tree();
    tree.lines().filter(|l| l.contains("visual") || l.contains("textbox") || l.contains("Add to message")).collect::<Vec<_>>().join("\n")
}

/// The tree line of what has the keys.
fn line_of_focus(app: &mut TestApp) -> String {
    app.tree().lines().find(|l| l.split(' ').any(|w| w == "focused")).map(|l| l.trim().to_owned()).unwrap_or_default()
}

fn has_keys(app: &mut TestApp, target: &str) -> bool {
    let id = app.target(target).ok();
    id.is_some() && app.ui.input.focused() == id
}

fn goal(app: &TestApp) -> Option<String> {
    crate::visuals::entries_of("v:fixture-answer").field("goal").map(crate::auth::text)
}

/// Typed as the keyboard types, into what has the keys.
fn text(app: &mut TestApp, s: &str) {
    app.ui.dispatch(zgx::input::InputEvent::Text(s.into()));
    app.settle();
    app.frames(10);
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
    pin(&mut app);
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
    pin(&mut app);
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

/// Pinned only once its place has scrolled out of view: in view it stays
/// where it is, out of view it's over the composer (its place a line, as
/// tall as it was, seen in the stand-in), and scrolled back it returns (plans/live-visuals fix1).
#[test]
fn an_open_visual_is_pinned_only_while_its_place_is_out_of_view() {
    let mut app = mount();
    app.frames(20);
    assert!(app.find("#chat-pinned-visual").is_none(), "pinned while in view:\n{}", visual_lines(&mut app));
    assert!(app.find(".visual-pinned-marker").is_none());
    pin(&mut app);
    back(&mut app);
    assert!(app.find(".visual-pinned-marker").is_none());
    assert_eq!(app.error(), None);
}

/// The first keystroke into a visual's field keeps the keys there, in its
/// place and pinned. (The first entry made the input "kept", which the
/// visual's shape counted: the whole visual was built again, the field
/// went, and the composer took the keys: plans/live-visuals fix1.)
#[test]
fn the_first_keystroke_into_a_visual_keeps_the_keys_there() {
    let goal_box = "role:textbox \"Goal\"";
    let mut app = mount();
    let before = app.target(&format!("#transcript >> {goal_box}")).unwrap();
    app.click(&format!("#transcript >> {goal_box}"));
    app.frames(5);
    assert!(has_keys(&mut app, goal_box), "{}", line_of_focus(&mut app));
    text(&mut app, "M");
    app.wait_until(|app| goal(app).as_deref() == Some("M"));
    app.frames(10);
    assert!(has_keys(&mut app, goal_box), "the keys left the field for {}:\n{}", line_of_focus(&mut app), visual_lines(&mut app));
    assert!(!has_keys(&mut app, COMPOSER), "the composer took the keys");
    assert_eq!(app.target(goal_box).unwrap(), before, "the field was built again");
    text(&mut app, "ove");
    app.wait_until(|app| goal(app).as_deref() == Some("Move"));
    assert_eq!(goal(&app).as_deref(), Some("Move"));

    // Pinned, a first entry into another field.
    let mut app = mount();
    pin(&mut app);
    let pinned_goal = format!("#chat-pinned-visual >> {goal_box}");
    app.click(&pinned_goal);
    app.frames(5);
    text(&mut app, "N");
    app.wait_until(|app| goal(app).as_deref() == Some("N"));
    app.frames(10);
    assert!(has_keys(&mut app, &pinned_goal), "the keys left the pinned field:\n{}", visual_lines(&mut app));
    assert!(!has_keys(&mut app, COMPOSER), "the composer took the keys");
    assert_eq!(app.error(), None);
}

/// Moving between its place and the pin (scrolled away, scrolled back), the
/// field being typed in keeps the keys and the caret, and the entry stays.
#[test]
fn moving_between_place_and_pin_keeps_the_keys_and_the_caret() {
    let goal_box = "role:textbox \"Goal\"";
    let mut app = mount();
    app.click(&format!("#transcript >> {goal_box}"));
    app.frames(5);
    text(&mut app, "abc");
    app.keys("Left");
    app.frames(5);
    app.wait_until(|app| goal(app).as_deref() == Some("abc"));
    pin(&mut app);
    app.frames(10);
    let pinned_goal = format!("#chat-pinned-visual >> {goal_box}");
    assert!(has_keys(&mut app, &pinned_goal), "the pinned field hasn't the keys:\n{}", visual_lines(&mut app));
    text(&mut app, "X");
    app.wait_until(|app| goal(app).as_deref() == Some("abXc"));
    assert_eq!(goal(&app).as_deref(), Some("abXc"), "the caret moved");
    back(&mut app);
    app.frames(10);
    let placed_goal = format!("#transcript >> {goal_box}");
    assert!(has_keys(&mut app, &placed_goal), "the field in its place hasn't the keys:\n{}", visual_lines(&mut app));
    text(&mut app, "Y");
    app.wait_until(|app| goal(app).as_deref() == Some("abXYc"));
    assert_eq!(goal(&app).as_deref(), Some("abXYc"), "the caret moved");
    assert_eq!(app.error(), None);
}

/// Whether the host has the visual as sent (its place may be out of view
/// after a send, so what it draws can't say).
fn sent(app: &TestApp) -> bool {
    app.host.shared("visuals", Value::Nil).get().field("v:fixture-answer").and_then(|v| v.field("sent")).is_some_and(|v| v.truthy())
}

/// The composer's draft, rewritten as the reader would (the words changed,
/// lines gone): what the next send holds.
fn rewrite_draft(app: &mut TestApp, words: &str) {
    app.host.shared("chat.composer.text", Value::Nil).set(Value::Str(words.into()));
    app.frames(10);
}

/// A message holding the visual's heading line is its answer, however the
/// answers under it were rewritten (the reader's words, an answer changed,
/// lines struck); take the heading out and it isn't (plans/live-visuals
/// fix2).
#[test]
fn a_message_with_the_heading_line_answers_the_visual_whatever_else_was_rewritten() {
    let mut app = mount();
    pin(&mut app);
    app.type_text("role:textbox \"Goal\"", "Move the db");
    app.frames(10);
    app.wait_until(|app| goal(app).is_some());
    app.click(ANSWER);
    app.frames(10);
    assert!(draft(&app).contains("### Migration plan\nGoal: Move the db"), "{:?}", draft(&app));
    // Rewritten: the answer changed, a line struck, words of the reader's own.
    rewrite_draft(&mut app, "Hm, let me put it my way.\n\n### Migration plan\nGoal: not the db after all\nand more words");
    app.click(COMPOSER);
    app.keys("Enter");
    app.wait_until(|app| calls(app).contains("send:sv:"));
    assert!(calls(&app).contains("not the db after all"), "{}", calls(&app));
    app.wait_until(|app| sent(app));
    assert!(sent(&app), "the edited answers didn't answer it: {}", calls(&app));
    assert!(app.find("#chat-pinned-visual").is_none(), "still pinned");
    assert_eq!(app.error(), None);
}

#[test]
fn a_message_without_the_heading_line_leaves_the_visual_open_even_with_its_answers() {
    let mut app = mount();
    pin(&mut app);
    app.type_text("role:textbox \"Goal\"", "Move the db");
    app.frames(10);
    app.wait_until(|app| goal(app).is_some());
    app.click(ANSWER);
    app.frames(10);
    // The heading struck; the answers under it stay.
    rewrite_draft(&mut app, "Goal: Move the db\nSteps: none\n(### Migration plan, said in a sentence)");
    app.click(COMPOSER);
    app.keys("Enter");
    app.wait_until(|app| calls(app).contains("send:sv:"));
    app.frames(20);
    assert!(!sent(&app), "it was marked sent");
    assert!(app.find("#chat-pinned-visual").is_some(), "no longer pinned");
}

/// Two visuals under one title: each heading line in the message answers
/// one, the one whose whole block it holds first, else the newest put
/// (the fixture's `sv_dup`, noted in `calls`).
#[test]
fn two_visuals_with_one_title_are_told_apart_by_how_many_headings_a_message_holds() {
    let mut app = mount();
    app.host.run_command("sv_dup");
    app.frames(10);
    let seen = calls(&app);
    for want in [
        "one:b",
        "two:a+b",
        "trimmed:b",
        "inline:",
        "whole:a",
        "none:",
    ] {
        assert!(seen.contains(&format!("dup:{want}")), "{want} missing:\n{seen}");
    }
}

/// The reader's own ×: it is answered all the same (marked sent with the
/// part's path and device, though no answers ever went into the draft),
/// unpinned, nothing sent.
#[test]
fn the_x_on_the_pinned_visual_marks_it_sent_by_the_parts_path() {
    let mut app = mount();
    app.host.run_command("sv_spy");
    pin(&mut app);
    assert!(draft(&app).is_empty(), "{:?}", draft(&app));
    app.click("#chat-pinned-visual >> role:button \"Close\"");
    app.frames(10);
    app.wait_until(|app| calls(app).contains("mark:"));
    assert!(calls(&app).contains("mark:v:fixture-answer:/fixture/answer.luau:fixture-device"), "{}", calls(&app));
    app.wait_until(|app| app.find("#chat-pinned-visual").is_none());
    app.frames(10);
    assert!(app.find("#chat-pinned-visual").is_none(), "still pinned");
    assert!(app.find(".visual-pinned-marker").is_none(), "still a line");
    assert!(!calls(&app).contains("send:"), "something was sent: {}", calls(&app));
    assert!(draft(&app).is_empty(), "{:?}", draft(&app));
    assert_eq!(app.error(), None);
}

fn height_of(app: &mut TestApp, selector: &str) -> f32 {
    let node = app.find(selector).unwrap_or_else(|| panic!("no {selector}"));
    app.ui.scene.borrow().transformed_bounds(node).height
}

/// Minimised, the pinned visual is its title line alone; the line or its
/// button opens it again, what was entered kept, and nothing is marked
/// sent by folding it.
#[test]
fn the_pinned_visual_folds_to_its_title_line_and_opens_again() {
    let goal_box = "#chat-pinned-visual >> role:textbox \"Goal\"";
    let mut app = mount();
    app.host.run_command("sv_spy");
    pin(&mut app);
    let open_h = height_of(&mut app, "#chat-pinned-visual");
    app.click(goal_box);
    text(&mut app, "Keep me");
    app.wait_until(|app| goal(app).as_deref() == Some("Keep me"));

    app.click("#chat-pinned-visual >> role:button \"Minimise\"");
    app.frames(15);
    assert!(app.find(goal_box).is_none(), "the visual still shows:\n{}", visual_lines(&mut app));
    assert!(app.find("#chat-pinned-visual >> .visual-pinned-title").is_some(), "no title line");
    let folded_h = height_of(&mut app, "#chat-pinned-visual");
    assert!(folded_h < 40. && folded_h < open_h, "folded {folded_h} open {open_h}");
    assert_eq!(goal(&app).as_deref(), Some("Keep me"));
    assert!(!calls(&app).contains("mark:"), "folding marked it: {}", calls(&app));

    app.click("#chat-pinned-visual >> role:button \"Expand\"");
    app.frames(15);
    assert!(app.find(goal_box).is_some(), "not open again:\n{}", visual_lines(&mut app));
    assert_eq!(goal(&app).as_deref(), Some("Keep me"));
    assert!(height_of(&mut app, "#chat-pinned-visual") > folded_h);

    // The line itself opens it too.
    app.click("#chat-pinned-visual >> role:button \"Minimise\"");
    app.frames(15);
    assert!(app.find(goal_box).is_none());
    app.click("#chat-pinned-visual >> .visual-pinned-title");
    app.frames(15);
    assert!(app.find(goal_box).is_some(), "the line didn't open it");
    assert_eq!(goal(&app).as_deref(), Some("Keep me"));
    assert!(!calls(&app).contains("mark:"), "folding marked it: {}", calls(&app));
    assert_eq!(app.error(), None);
}

