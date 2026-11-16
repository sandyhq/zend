//! Words are never cut off where they are for reading (ui/wrap_fixture.luau):
//! a field's label and a visual form's question longer than their column wrap
//! onto more lines with all their words; a tool's row shows up to two lines
//! of its command, fading after, and a press opens the whole call.
//! `cargo test --bin zend-zgx wrap_tests`; `ZEND_ZGX_TEST_UI=<another ui/>`
//! runs it on another copy of the interface (to see it fail without a fix).

use zgx::scene::NodeKind;
use zgx_script::{ScriptHost, testing::TestApp};

const QUESTION: &str = "Welche der Projekte gov-sess, chatgpt-intercom, zend-wrap und pi-setup sind für die Migration vorgesehen?";
const FIELD_LABEL: &str = "Warum gerade diese Projekte jetzt und nicht erst nach dem Release im Herbst?";
const TWO: &str = "cd ~/Projects/zend-wrap && git log --oneline -5 && git status";

fn mount() -> TestApp {
    let ui = std::env::var_os("ZEND_ZGX_TEST_UI")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("ui"));
    let path = ui.join("wrap_fixture.luau");
    let mut app = TestApp::new(|runtime| {
        let host = ScriptHost::new(runtime, path);
        crate::visuals::register(&host);
        host
    });
    app.ui.scene.borrow_mut().resize(420., 1400.);
    app.frames(10);
    app.wait_until(|app| app.find("~\"Loading \"").is_none());
    app.frames(30);
    assert_eq!(app.error(), None, "the fixture failed");
    app
}

/// Each text drawn under `root` (its words and its box's height), with the
/// node's id, in order.
fn texts(app: &mut TestApp, root: &str) -> Vec<(String, f32, f32)> {
    let root = app.target(root).unwrap_or_else(|e| panic!("{e}"));
    let scene = app.ui.scene.borrow();
    let mut out = Vec::new();
    let mut stack = vec![root];
    while let Some(n) = stack.pop() {
        stack.extend(scene.children(n).iter().rev());
        if let NodeKind::Text { text, .. } = scene.kind(n) {
            let b = scene.bounds(n);
            out.push((text.to_string(), b.width, b.height));
        }
    }
    out
}

/// A line of the interface's small text is 16 points here; a text of more than
/// one and a half lines runs over lines.
const LINE: f32 = 16.;

fn the_text(app: &mut TestApp, root: &str, words: &str) -> (f32, f32) {
    let all = texts(app, root);
    let found = all.iter().find(|(t, ..)| t == words).unwrap_or_else(|| panic!("{words:?} is not drawn whole under {root}: {all:#?}"));
    (found.1, found.2)
}

/// A field's label (ds/field `M.label`, every field's) longer than its
/// column wraps, its words whole.
#[test]
fn a_field_label_longer_than_its_column_wraps() {
    let mut app = mount();
    let (w, h) = the_text(&mut app, "#wrap-field", QUESTION);
    assert!(w <= 360., "wider than its column: {w}");
    assert!(h > LINE * 1.5, "one line, cut off: {h} tall");
}

/// A visual form's question and its field's label wrap, their words whole.
#[test]
fn a_visual_form_s_question_and_labels_wrap() {
    let mut app = mount();
    let (w, h) = the_text(&mut app, "#wrap-visual", QUESTION);
    assert!(w <= 360., "the question runs past its column: {w}");
    assert!(h > LINE * 1.5, "the question is one line, cut off: {h} tall");
    let (w, h) = the_text(&mut app, "#wrap-visual", FIELD_LABEL);
    assert!(w <= 360., "the field's label runs past its column: {w}");
    assert!(h > LINE * 1.5, "the field's label is one line, cut off: {h} tall");
}

/// Where a tool's row is: the one saying `words` (its box's height).
fn row_height(app: &mut TestApp, words: &str) -> f32 {
    let all = texts(app, "#wrap-activity");
    all.iter().find(|(t, ..)| t.starts_with(words)).map(|t| t.2).unwrap_or_else(|| panic!("{words:?}: {all:#?}"))
}

/// The command's own text node: its height is its lines.
#[test]
fn a_tool_row_shows_two_lines_of_a_long_command_and_a_press_opens_the_call() {
    let mut app = mount();
    // A short call: one line.
    let short = row_height(&mut app, "cargo test");
    assert!(short < LINE * 1.5, "a short call is one line: {short}");
    // A command that runs to two lines: the row is two lines, whole.
    let two = row_height(&mut app, "cd ~/Projects/zend-wrap && git log --oneline");
    assert!((two - 2. * short).abs() < 1., "two lines tall: {two}, a line is {short}");
    // One that runs to more: the box that clips its words is two lines,
    // the words laid out whole behind it.
    let all = texts(&mut app, "#wrap-activity");
    let many = all.iter().find(|(t, ..)| t.contains("cargo nextest run")).unwrap_or_else(|| panic!("{all:#?}"));
    assert!(many.2 > 2.5 * short, "the words run to more than two lines: {}", many.2);
    // (The row's own height: its text's box clipped at two lines.)
    let clip = {
        let id = app.target("#wrap-activity").unwrap();
        let scene = app.ui.scene.borrow();
        let mut stack = vec![id];
        let mut found = None;
        while let Some(n) = stack.pop() {
            stack.extend(scene.children(n).iter().copied());
            if let NodeKind::Text { text, .. } = scene.kind(n)
                && text.contains("cargo nextest run")
            {
                found = scene.ancestors(n).find(|&a| scene.style(a).clip).map(|a| scene.bounds(a).height);
            }
        }
        found.expect("its clip")
    };
    assert!((clip - 2. * short).abs() < 1., "clipped at two lines ({}), not three: {clip}", 2. * short);
    // The row itself: the button, two lines of words and the room round them.
    let button = app.bounds("role:button \"Ran cd ~/Projects/zend-wrap && git log --oneline -5 && git status && cargo check -p zend-engine && cargo nextest run --no-fail-fast -p zend-engine && cargo test --doc -p zend-engine && git diff --stat\"");
    assert!(button.height < 4. * short, "the row is two lines and its padding: {}", button.height);
    assert!(button.height > 2. * short, "the row holds both lines: {}", button.height);
    // A press on the row opens the whole call under it.
    let before = texts(&mut app, "#wrap-activity").iter().filter(|(t, ..)| t == TWO).count();
    app.click("role:button \"Ran cd ~/Projects/zend-wrap && git log --oneline -5 && git status\"");
    app.frames(20);
    let after = texts(&mut app, "#wrap-activity").iter().filter(|(t, ..)| t == TWO).count();
    assert!(after > before, "the call opens whole: {before} → {after}");
}
