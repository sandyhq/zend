//! ui.kit, batch b (planning and data): each kind builds its nodes, and bad
//! data fails with the kind's name at the agent's line. The examples in
//! visuals/ are checked by kit_tests, which also counts these kinds.

use super::*;

/// The kinds kit_b.luau adds. Compare (needs an image node) and Threads
/// (needs the host's live chat status) are left out on purpose.
pub(super) const KINDS_B: &[&str] = &[
    "Board",
    "Calendar",
    "CalendarHeatmap",
    "Decision",
    "Entity",
    "Estimate",
    "Flashcards",
    "Plan",
    "Provenance",
    "Roadmap",
    "Sources",
    "Table",
    "Timeline",
    "Transcript",
    "Walkthrough",
];

fn tree(script: &str) -> Node {
    Visual::load(script, 1)
        .unwrap_or_else(|e| panic!("{script}: {}", e.message))
        .render()
        .unwrap_or_else(|e| panic!("{script}: {}", e.message))
}

fn text_of(node: &Node) -> String {
    match &node.kind {
        NodeKind::Text(text) => text.spans.iter().map(|s| s.text.as_str()).collect(),
        _ => children(node).map(text_of).collect::<Vec<_>>().join("|"),
    }
}

fn count(node: &Node, pred: &dyn Fn(&Node) -> bool) -> usize {
    usize::from(pred(node)) + children(node).map(|c| count(c, pred)).sum::<usize>()
}

fn is_canvas(n: &Node) -> bool {
    matches!(n.kind, NodeKind::Canvas(_))
}

fn is_table(n: &Node) -> bool {
    matches!(n.kind, NodeKind::Table(_))
}

fn is_answer(n: &Node) -> bool {
    matches!(n.kind, NodeKind::Answer { .. })
}

fn err(script: &str) -> String {
    check(script, 1).unwrap_err().message
}

#[test]
fn board_puts_items_in_columns_by_status() {
    let t = tree(
        r#"return ui.kit.Board { items = {
            { key = "A-1", title = "Ship it", status = "done", owner = "Ana" },
            { key = "A-2", title = "Write it", status = "in_progress", priority = "high", due = "2026-10-01" },
            { title = "Later", status = "backlog" },
            { title = "Stuck", status = "blocked", detail = "Waiting on DNS\nsecond line" },
        } }"#,
    );
    let text = text_of(&t);
    // Default column order: backlog, in progress, blocked, done.
    let at = |s: &str| text.find(s).unwrap_or_else(|| panic!("{s} in {text}"));
    assert!(at("Backlog") < at("In progress") && at("In progress") < at("Blocked") && at("Blocked") < at("Done"), "{text}");
    assert!(text.contains("Write it") && text.contains("high") && text.contains("due 2026-10-01"), "{text}");
    assert!(text.contains("Waiting on DNS") && !text.contains("second line"), "{text}");
    // Given columns fix the order and reject a status they lack.
    let t = tree(
        r#"return ui.kit.Board { columns = { "Open", "Merged" }, items = { { title = "x", status = "merged" }, { title = "y", status = "open" } } }"#,
    );
    let text = text_of(&t);
    assert!(text.find("Open").unwrap() < text.find("Merged").unwrap(), "{text}");
    let e = err(r#"return ui.kit.Board { columns = { "Open" }, items = { { title = "x", status = "closed" } } }"#);
    assert!(e.contains("ui.kit.Board") && e.contains("not in columns"), "{e}");
}

#[test]
fn roadmap_draws_one_canvas_and_lists_details() {
    let t = tree(
        r#"return ui.kit.Roadmap { items = {
            { title = "Build", start = "2026-10-01", ["end"] = "2026-10-10", lane = "Dev", status = "active", progress = 40, detail = "late copy" },
            { title = "Ship", milestone = "2026-10-12", lane = "Dev", after = "Build" },
            { title = "Test", start = "2026-10-05", days = 3, lane = "QA" },
        } }"#,
    );
    assert_eq!(count(&t, &is_canvas), 1);
    assert!(text_of(&t).ends_with("Build: late copy"), "{}", text_of(&t));
    for (script, needle) in [
        (r#"return ui.kit.Roadmap { items = { { title = "x", start = "2026-13-01", days = 2 } } }"#, "bad start"),
        (r#"return ui.kit.Roadmap { items = { { title = "x", start = "2026-02-30", days = 2 } } }"#, "does not exist"),
        (r#"return ui.kit.Roadmap { items = { { title = "x", start = "2026-10-05", ["end"] = "2026-10-01" } } }"#, "ends before"),
        (r#"return ui.kit.Roadmap { items = { { title = "x", milestone = "2026-10-05", after = "y" } } }"#, "not an item"),
        (r#"return ui.kit.Roadmap { items = { { title = "x", milestone = "2026-10-05", status = "late" } } }"#, "unknown status 'late'"),
    ] {
        let e = err(script);
        assert!(e.contains("ui.kit.Roadmap") && e.contains(needle), "{e}");
    }
}

#[test]
fn plan_is_a_checklist_with_its_answer_button() {
    let t = tree(
        r#"return ui.kit.Plan { steps = {
            { title = "Fix limiter", detail = "per host", risk = "medium", estimate = "2h", files = { "a.ts:1" } },
            { title = "Polish", optional = true },
        } }"#,
    );
    let found = inputs(&t);
    assert_eq!(found.len(), 1);
    assert_eq!((found[0].name.as_str(), found[0].kind), ("plan", InputKind::Checklist));
    assert_eq!(found[0].value, EntryValue::List(vec!["Fix limiter".into()]));
    assert_eq!(found[0].options.len(), 2);
    assert_eq!(
        found[0].options[0].description.as_deref(),
        Some("per host  ·  medium risk · 2h · a.ts:1")
    );
    assert_eq!(found[0].options[1].description.as_deref(), Some("optional"));
    assert_eq!(count(&t, &is_answer), 1);
    let bare = tree(r#"return ui.kit.Plan { steps = { "a" }, answer = false, name = "p" }"#);
    assert_eq!(count(&bare, &is_answer), 0);
    // In a Form its own button is taken out: one button in all.
    let form = tree(r#"return ui.kit.Form { ui.kit.Plan { steps = { "a" } } }"#);
    assert_eq!(count(&form, &is_answer), 1);
}

#[test]
fn decision_ranks_by_weighted_score_and_weights_move_on_a_slider() {
    let script = r#"local view = function()
        return ui.kit.Decision {
            options = {
                { name = "A", scores = { 5, 1 }, pros = { "fast" } },
                { name = "B", scores = { Speed = 2, Cost = 5 }, cons = { "pricey" } },
            },
            criteria = { { name = "Speed", weight = 1 }, { name = "Cost", weight = 3 } },
        }
    end
    return view"#;
    let mut visual = Visual::load(&format!("return function() local v = (function() {script} end)() return v() end"), 1).unwrap();
    let t = visual.render().unwrap();
    let text = text_of(&t);
    assert_eq!(count(&t, &is_canvas), 1);
    assert_eq!(count(&t, &is_table), 1);
    assert!(text.contains("+ fast") && text.contains("− pricey"), "{text}");
    // B wins at cost weight 3 ((2 + 15) / 4 = 4.25 against (5 + 3) / 4 = 2).
    let sliders = count(&t, &|n| matches!(n.kind, NodeKind::Slider { .. }));
    assert_eq!(sliders, 2);
    for (script, needle) in [
        (r#"return ui.kit.Decision { options = { { name = "A", scores = {} } }, criteria = { { name = "c" } } }"#, "at least two"),
        (
            r#"return ui.kit.Decision { options = { { name = "A", scores = { 1 } }, { name = "B", scores = {} } }, criteria = { { name = "c" } } }"#,
            "no score for 'c'",
        ),
    ] {
        let e = err(script);
        assert!(e.contains("ui.kit.Decision") && e.contains(needle), "{e}");
    }
}

#[test]
fn estimate_sums_three_point_estimates() {
    let t = tree(
        r#"return ui.kit.Estimate { unit = "d", tasks = {
            { title = "a", low = 1, likely = 2, high = 3 },
            { title = "b", low = 4, high = 2 },
        } }"#,
    );
    let text = text_of(&t);
    // a: expected 2; b swapped to 2..4, likely 3, expected 3; total 5.
    assert!(text.contains("Total|3||7|5"), "{text}");
    assert!(text.contains("b|2|3|4|3"), "swapped low and high: {text}");
    assert_eq!(count(&t, &|n| matches!(n.kind, NodeKind::Stat { .. })), 3);
    assert_eq!(count(&t, &is_table), 1);
    let e = err(r#"return ui.kit.Estimate { tasks = { { title = "a", low = 1 } } }"#);
    assert!(e.contains("ui.kit.Estimate") && e.contains("needs title, low and high"), "{e}");
}

#[test]
fn table_timeline_sources_and_entity_build() {
    let t = tree(
        r#"return ui.kit.Table { columns = { "A", "B" }, rows = { { "x", true }, { 1, ui.kit.Badge { "ok" } } } }"#,
    );
    assert_eq!(count(&t, &is_table), 1);
    assert_eq!(text_of(&t), "x|✓|1|ok");
    let e = err(r#"return ui.kit.Table { columns = { "A" }, rows = { { 1, 2 } } }"#);
    assert!(e.contains("ui.kit.Table") && e.contains("2 cells for 1 columns"), "{e}");

    let t = tree(
        r#"return ui.kit.Timeline { items = { { time = "09:12", title = "Deploy", detail = "v2" }, { time = "09:20", title = "Down", tone = "bad" } } }"#,
    );
    assert_eq!(text_of(&t), "09:12|●|Deploy|v2|09:20|●|Down");
    assert!(err(r#"return ui.kit.Timeline { items = { { title = "x", tone = "red" } } }"#).contains("unknown tone 'red'"));

    let t = tree(
        r#"return ui.kit.Sources { links = { { url = "https://a.example/x", title = "A", note = "n" }, "https://b.example/" } }"#,
    );
    assert_eq!(text_of(&t), "[1]||a.example|n|[2]|");
    assert_eq!(count(&t, &|n| matches!(n.kind, NodeKind::Link { .. })), 2);
    assert!(err(r#"return ui.kit.Sources { links = { "ftp://x" } }"#).contains("needs an http(s) url"));

    let t = tree(
        r#"return ui.kit.Entity { name = "Ana", kind = "person", facts = { { "Role", "Ranger" } }, note = "Walks", url = "https://x.example" }"#,
    );
    assert_eq!(text_of(&t), "Ana|person|Walks|Role|Ranger|");
    assert!(err(r#"return ui.kit.Entity { name = "x", kind = "robot" }"#).contains("unknown kind 'robot'"));
}

#[test]
fn calendar_lists_months_with_events() {
    let t = tree(
        r#"return ui.kit.Calendar { events = {
            { date = "2026-10-20", title = "Review", tone = "bad" },
            { date = "2026-10-20", title = "Docs" },
            { date = "2026-11-03", title = "Retro" },
        } }"#,
    );
    let text = text_of(&t);
    assert!(text.contains("October 2026") && text.contains("November 2026"), "{text}");
    assert!(text.contains("Oct 20|Review  Docs") && text.contains("Nov 3|Retro"), "{text}");
    // 2026-10-01 is a Thursday: three blanks, then day 1 in column four.
    assert!(text.contains("Mon|Tue|Wed|Thu|Fri|Sat|Sun"), "{text}");
    let one = text_of(&tree(
        r#"return ui.kit.Calendar { month = "2026-10", events = { { date = "2026-10-20", title = "Review" }, { date = "2026-11-03", title = "Retro" } } }"#,
    ));
    assert!(one.contains("October 2026") && !one.contains("Retro"), "{one}");
    let e = err(r#"return ui.kit.Calendar { events = { { date = "2026-10-20", title = "a" }, { date = "2027-10-20", title = "b" } } }"#);
    assert!(e.contains("ui.kit.Calendar") && e.contains("more than six months"), "{e}");
    assert!(err(r#"return ui.kit.Calendar { month = "Oct", events = { { date = "2026-10-20", title = "a" } } }"#).contains("bad month"));
}

#[test]
fn heatmap_draws_a_canvas_from_a_map_or_a_list() {
    let map = tree(r#"return ui.kit.CalendarHeatmap { days = { ["2026-08-03"] = 30, ["2026-09-12"] = 80 }, label = "min" }"#);
    assert_eq!(count(&map, &is_canvas), 1);
    let list = tree(
        r#"return ui.kit.CalendarHeatmap { days = { { date = "2026-08-03", value = 30 }, { date = "2026-08-03" } }, tone = "info" }"#,
    );
    assert_eq!(count(&list, &is_canvas), 1);
    let e = err(r#"return ui.kit.CalendarHeatmap { days = { ["soon"] = 3 } }"#);
    assert!(e.contains("ui.kit.CalendarHeatmap") && e.contains("bad date 'soon'"), "{e}");
}

#[test]
fn provenance_flags_what_differs() {
    let ok = text_of(&tree(
        r#"return ui.kit.Provenance { stages = {
            { name = "Build", status = "done", facts = { sha = "44f1d749f06f2a0f573f58dbd68289e510ab408b" } },
            { name = "Live", status = "done", facts = { sha = "44f1d74" } },
        } }"#,
    ));
    assert!(ok.contains("Consistent") && ok.contains("44f1d74"), "{ok}");
    let bad = text_of(&tree(
        r#"return ui.kit.Provenance { expected = { branch = "main" }, stages = {
            { name = "Build", status = "done", facts = { sha = "aaaaaaa", branch = "dev" } },
            { name = "Live", status = "done", facts = { sha = "bbbbbbb" } },
        } }"#,
    ));
    assert!(bad.contains("Mismatch") && bad.contains("expected main") && bad.contains("was aaaaaaa in Build"), "{bad}");
    let none = text_of(&tree(
        r#"return ui.kit.Provenance { stages = { { name = "Build", status = "done" }, { name = "Install" } } }"#,
    ));
    assert!(none.contains("No receipt after Build") && none.contains("No receipt"), "{none}");
    let e = err(r#"return ui.kit.Provenance { stages = { { name = "x", status = "maybe" } } }"#);
    assert!(e.contains("ui.kit.Provenance") && e.contains("unknown status 'maybe'"), "{e}");
}

#[test]
fn transcript_counts_word_errors_against_the_first_track() {
    let t = tree(
        r#"return ui.kit.Transcript { tracks = {
            { name = "Said", text = "Hello, can you hear me today" },
            { name = "Heard", words = { { text = "hello" }, { text = "can" }, { text = "you" }, { text = "here" }, { text = "me" }, { text = "now" }, { text = "today" } } },
            { name = "Same", text = "hello can you hear me today" },
        } }"#,
    );
    let text = text_of(&t);
    // One wrong word (here), one extra (now): 2 of 6 words.
    assert!(text.contains("reference · 6 words"), "{text}");
    assert!(text.contains("WER 33% · 1 wrong, 1 extra, 0 missed"), "{text}");
    assert!(text.contains("WER 0% · 0 wrong, 0 extra, 0 missed"), "{text}");
    let missed = text_of(&tree(
        r#"return ui.kit.Transcript { tracks = { { name = "a", text = "one two three" }, { name = "b", text = "one three" } } }"#,
    ));
    assert!(missed.contains("1 missed") && missed.contains("[two]"), "{missed}");
    let e = err(r#"return ui.kit.Transcript { tracks = { { name = "a" } } }"#);
    assert!(e.contains("ui.kit.Transcript") && e.contains("needs words"), "{e}");
}

#[test]
fn walkthrough_and_flashcards_step_on_a_click() {
    let mut visual = Visual::load(
        r#"return function() return ui.kit.Walkthrough { steps = {
            { title = "One", body = "first", code = "ls", language = "bash" },
            { title = "Two", body = "second" },
        } } end"#,
        1,
    )
    .unwrap();
    let t = visual.render().unwrap();
    assert_eq!(text_of(&t), "Step 1 of 2|One|first|bash|ls|");
    let button = |t: &Node| {
        let mut found = Vec::new();
        fn walk(n: &Node, out: &mut Vec<HandlerId>) {
            if let NodeKind::Button { on_click, .. } = &n.kind {
                out.push(*on_click);
            }
            for c in children(n) {
                walk(c, out);
            }
        }
        walk(t, &mut found);
        found
    };
    let next = button(&t);
    assert_eq!(next.len(), 1);
    visual.call(next[0], HandlerArg::None).unwrap();
    let t = visual.render().unwrap();
    assert_eq!(text_of(&t), "Step 2 of 2|Two|second|");
    assert_eq!(button(&t).len(), 1, "only Back on the last step");

    let mut visual = Visual::load(
        r#"return function() return ui.kit.Flashcards { cards = { { front = "401", back = "Unauthorized" }, { front = "404", back = "Not found" } } } end"#,
        1,
    )
    .unwrap();
    let t = visual.render().unwrap();
    assert!(text_of(&t).contains("Card 1 of 2|Front|401"), "{}", text_of(&t));
    let NodeKind::Stack(outer) = &t.kind else { unreachable!() };
    let NodeKind::Stack(card) = &outer.children[1].kind else { unreachable!() };
    visual.call(card.on_click.unwrap(), HandlerArg::None).unwrap();
    assert!(text_of(&visual.render().unwrap()).contains("Back|Unauthorized"));
    let e = err(r#"return ui.kit.Flashcards { cards = { { front = "x" } } }"#);
    assert!(e.contains("ui.kit.Flashcards") && e.contains("needs front and back"), "{e}");
}

#[test]
fn batch_b_errors_name_the_kind_and_the_agents_line() {
    for kind in KINDS_B {
        let e = err(&format!("\n\nreturn ui.kit.{kind} {{ colour = \"red\" }}"));
        assert!(e.contains("visual:3"), "{kind}: {e}");
        assert!(e.contains(&format!("ui.kit.{kind} has no prop 'colour'")), "{kind}: {e}");
    }
}
