//! ui.kit, batch e (plans/live-visuals G7e): Slider, Stepper, Rating,
//! ListEditor, EditableTable, Quiz and Timer, and the three named inputs
//! under them (`ui.number`, `ui.items`, `ui.rows`). Their examples are
//! checked with the rest in kit_tests.rs.

use super::*;

fn visual(script: &str) -> Visual {
    Visual::load(script, 1).unwrap_or_else(|e| panic!("{script}: {}", e.message))
}

fn render(visual: &mut Visual) -> Node {
    visual.render().unwrap_or_else(|e| panic!("{}", e.message))
}

fn only_input(script: &str) -> Input {
    let t = render(&mut visual(script));
    let found = inputs(&t);
    assert_eq!(found.len(), 1, "{script}");
    found[0].clone()
}

fn text_of(node: &Node) -> String {
    match &node.kind {
        NodeKind::Text(text) => text.spans.iter().map(|s| s.text.as_str()).collect(),
        _ => children(node).map(text_of).collect::<Vec<_>>().join("|"),
    }
}

fn buttons(node: &Node, out: &mut Vec<(String, HandlerId)>) {
    if let NodeKind::Button { label, on_click, .. } = &node.kind {
        out.push((label.clone(), *on_click));
    }
    children(node).for_each(|c| buttons(c, out));
}

fn canvas(node: &Node) -> Option<HandlerId> {
    if let NodeKind::Canvas(c) = &node.kind {
        return Some(c.draw);
    }
    children(node).find_map(canvas)
}

fn drawn_text(visual: &mut Visual, tree: &Node, time: f64) -> String {
    let draw = canvas(tree).expect("a canvas");
    visual
        .draw_canvas(draw, 300.0, 64.0, time, None)
        .unwrap()
        .into_iter()
        .filter_map(|c| match c {
            DrawCmd::Text(t) => Some(t.text),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("|")
}

#[test]
fn slider_stepper_and_rating_are_one_named_number_each() {
    let slider = only_input(
        r#"return ui.kit.Slider { name = "budget", label = "Budget", min = 0, max = 50, step = 5, value = 20 }"#,
    );
    assert_eq!(slider.kind, InputKind::Number { style: NumberStyle::Slider });
    assert_eq!(slider.range, Some(NumberRange { min: Some(0.0), max: Some(50.0), step: 5.0 }));
    assert_eq!(slider.value, EntryValue::Text("20".into()));

    // A slider without a value starts at min; fractional ends get 100 stops.
    let fine = only_input(r#"return ui.kit.Slider { name = "s", min = 0.5, max = 1.5 }"#);
    assert_eq!(fine.value, EntryValue::Text("0.5".into()));
    assert!((fine.range.unwrap().step - 0.01).abs() < 1e-9);

    let stepper = only_input(r#"return ui.kit.Stepper { name = "seats", label = "Seats", min = 1 }"#);
    assert_eq!(stepper.kind, InputKind::Number { style: NumberStyle::Stepper });
    assert_eq!(stepper.range, Some(NumberRange { min: Some(1.0), max: None, step: 1.0 }));
    assert_eq!(stepper.value, EntryValue::Text("1".into()));

    let rating = only_input(r#"return ui.kit.Rating { name = "fit", label = "How well does it fit?" }"#);
    assert_eq!(rating.kind, InputKind::Number { style: NumberStyle::Rating });
    assert_eq!(rating.range.unwrap().max, Some(5.0));
    // Unrated until the reader picks.
    assert_eq!(rating.value, EntryValue::Text(String::new()));
}

#[test]
fn list_editor_and_editable_table_hold_lists_and_rows() {
    let list = only_input(
        r#"return ui.kit.ListEditor { name = "risks", label = "Risks", value = { "Vendor late", "Scope creep" }, placeholder = "Add a risk" }"#,
    );
    assert_eq!(list.kind, InputKind::Items);
    assert_eq!(list.placeholder.as_deref(), Some("Add a risk"));
    assert_eq!(list.value, EntryValue::List(vec!["Vendor late".into(), "Scope creep".into()]));

    // Rows keyed by column key or in column order; a boolean column is
    // "true" or "false".
    let table = only_input(
        r#"return ui.kit.EditableTable {
            name = "budget", label = "Budget",
            columns = { "item", { key = "cost", label = "Cost (€)", type = "number" }, { key = "ok", label = "Approved", type = "boolean" } },
            value = { { item = "Laptops", cost = 4200, ok = true }, { "Desks", 900, false } },
        }"#,
    );
    assert_eq!(table.kind, InputKind::Rows);
    let kinds: Vec<_> = table.columns.iter().map(|c| (c.key.as_str(), c.label.as_str(), c.kind)).collect();
    assert_eq!(
        kinds,
        [
            ("item", "item", ColumnKind::Text),
            ("cost", "Cost (€)", ColumnKind::Number),
            ("ok", "Approved", ColumnKind::Bool)
        ]
    );
    assert_eq!(
        table.value,
        EntryValue::Rows(vec![
            vec!["Laptops".into(), "4200".into(), "true".into()],
            vec!["Desks".into(), "900".into(), "false".into()],
        ])
    );
}

#[test]
fn what_the_reader_enters_is_kept_by_name_and_goes_into_the_answer() {
    let script = r#"return function() return ui.kit.Form { title = "Kickoff",
        ui.kit.Rating { name = "fit", label = "Fit" },
        ui.kit.Stepper { name = "seats", label = "Seats", min = 1, max = 9 },
        ui.kit.ListEditor { name = "risks", label = "Risks" },
        ui.kit.EditableTable { name = "budget", label = "Budget", columns = { "Item", { key = "ok", label = "Approved", type = "boolean" } } },
    } end"#;
    let mut v = visual(script);
    // Untouched: no answer for each (a stepper starts at its min).
    assert_eq!(
        answer_text("Kickoff", &render(&mut v)),
        "### Kickoff\nFit: (no answer)\nSeats: 1\nRisks: (no answer)\nBudget: (no answer)"
    );
    v.set_entry("fit", Some(EntryValue::Text("4".into())));
    v.set_entry("seats", Some(EntryValue::Text("3".into())));
    v.set_entry("risks", Some(EntryValue::List(vec!["Vendor late".into(), "A | pipe".into()])));
    v.set_entry(
        "budget",
        Some(EntryValue::Rows(vec![
            vec!["Laptops".into(), "true".into()],
            vec!["Desks | chairs".into(), "false".into()],
        ])),
    );
    let answer = answer_text("Kickoff", &render(&mut v));
    assert_eq!(
        answer,
        "### Kickoff\nFit: 4 of 5\nSeats: 3\nRisks:\n- Vendor late\n- A | pipe\nBudget:\n\
         | Item | Approved |\n|---|---|\n| Laptops | yes |\n| Desks \\| chairs | no |"
    );
    // ui.entry reads the rows back as a list of lists.
    let mut probe = visual(
        r#"return function() local r = ui.entry("t") or {}
        return ui.column { ui.rows { name = "t", columns = { "a", "b" } }, ui.text { #r .. ":" .. (r[1] and r[1][2] or "-") } } end"#,
    );
    probe.set_entry("t", Some(EntryValue::Rows(vec![vec!["x".into(), "y".into()]])));
    assert!(text_of(&render(&mut probe)).ends_with("1:y"));

    // Kept across a restart (JSON), even no rows, and across a new script.
    v.set_entry("budget", Some(EntryValue::Rows(Vec::new())));
    let saved = Entries::from_json(&v.entries().to_json());
    let mut again = visual(script).with_entries(saved);
    let t = render(&mut again);
    let found = inputs(&t);
    assert_eq!(found[0].value, EntryValue::Text("4".into()));
    assert_eq!(found[2].value, EntryValue::List(vec!["Vendor late".into(), "A | pipe".into()]));
    assert_eq!(found[3].value, EntryValue::Rows(Vec::new()));
    assert!(found[3].kept);
    v.swap(r#"return function() return ui.kit.Rating { name = "fit", max = 10 } end"#).unwrap();
    let t = render(&mut v);
    assert_eq!(inputs(&t)[0].value_text(), "4 of 10");
}

#[test]
fn a_quiz_says_after_each_pick_whether_it_was_right() {
    let script = r#"return function() return ui.kit.Quiz { name = "q", label = "Rust ownership", questions = {
        { q = "Who frees a Box?", options = { "The caller", "Its owner, when it goes out of scope", "The GC" }, answer = 2, why = "Drop runs when the owner goes out of scope." },
        { q = "Can two &mut borrows overlap?", options = { "Yes", "No" }, answer = "No" },
    } } end"#;
    let mut v = visual(script);
    let t = render(&mut v);
    let names: Vec<_> = inputs(&t).iter().map(|i| (i.name.clone(), i.kind)).collect();
    assert_eq!(
        names,
        [("q.1".to_owned(), InputKind::Choice { many: false }), ("q.2".to_owned(), InputKind::Choice { many: false })]
    );
    assert!(text_of(&t).contains("0 of 2 answered"));
    v.set_entry("q.1", Some(EntryValue::Text("1".into())));
    let t = render(&mut v);
    let shown = text_of(&t);
    assert!(shown.contains("✕ Not quite: Its owner, when it goes out of scope"), "{shown}");
    assert!(shown.contains("Drop runs when"), "{shown}");
    v.set_entry("q.2", Some(EntryValue::Text("2".into())));
    let t = render(&mut v);
    let shown = text_of(&t);
    assert!(shown.contains("✓ Right"), "{shown}");
    assert!(shown.contains("1 of 2 right"), "{shown}");
    // The picks go into the answer as the options' words.
    assert_eq!(
        answer_text("Quiz", &t),
        "### Quiz\n1. Who frees a Box?: The caller\n2. Can two &mut borrows overlap?: No"
    );
}

#[test]
fn a_timer_counts_down_from_start_and_pauses() {
    let mut v = visual(r#"return function() return ui.kit.Timer { seconds = 90, label = "Standup" } end"#);
    let t = render(&mut v);
    assert_eq!(drawn_text(&mut v, &t, 0.0), "01:30");
    let mut found = Vec::new();
    buttons(&t, &mut found);
    assert_eq!(found.iter().map(|(l, _)| l.as_str()).collect::<Vec<_>>(), ["Start", "Reset"]);
    v.call(found[0].1, HandlerArg::None).unwrap();
    let t = render(&mut v);
    assert_eq!(drawn_text(&mut v, &t, 10.0), "01:30");
    assert_eq!(drawn_text(&mut v, &t, 40.0), "01:00");
    let mut found = Vec::new();
    buttons(&t, &mut found);
    assert_eq!(found[0].0, "Pause");
    v.call(found[0].1, HandlerArg::None).unwrap();
    let t = render(&mut v);
    assert_eq!(drawn_text(&mut v, &t, 45.0), "00:55");
    assert_eq!(drawn_text(&mut v, &t, 500.0), "00:55");
    // Started again it runs out, done.
    let mut found = Vec::new();
    buttons(&t, &mut found);
    v.call(found[0].1, HandlerArg::None).unwrap();
    let t = render(&mut v);
    drawn_text(&mut v, &t, 500.0);
    assert_eq!(drawn_text(&mut v, &t, 600.0), "✓ 00:00");
    // A stopwatch counts up.
    let mut w = visual(r#"return function() return ui.kit.Timer {} end"#);
    let t = render(&mut w);
    let mut found = Vec::new();
    buttons(&t, &mut found);
    w.call(found[0].1, HandlerArg::None).unwrap();
    let t = render(&mut w);
    drawn_text(&mut w, &t, 1.0);
    assert_eq!(drawn_text(&mut w, &t, 66.0), "01:05");
}

#[test]
fn batch_e_errors_name_the_kind_and_the_agents_line() {
    let cases = [
        (r#"ui.kit.Slider { name = "s", colour = "red" }"#, "ui.kit.Slider has no prop 'colour'"),
        (r#"ui.kit.Stepper { label = "Seats" }"#, "ui.kit.Stepper needs name"),
        (r#"ui.kit.Slider { name = "s", min = 5, max = 1 }"#, "needs max above min"),
        (r#"ui.kit.Slider { name = "s", value = "lots" }"#, "ui.kit.Slider needs value = <number>"),
        (r#"ui.kit.Rating { name = "r", max = 20 }"#, "whole max from 2 to 10"),
        (r#"ui.kit.Rating { name = "r", min = 1 }"#, "ui.kit.Rating has no prop 'min'"),
        (r#"ui.kit.ListEditor { name = "l", value = "a" }"#, "ui.kit.ListEditor needs value"),
        (r#"ui.kit.EditableTable { name = "t" }"#, "ui.kit.EditableTable needs columns"),
        (
            r#"ui.kit.EditableTable { name = "t", columns = { { key = "a", type = "date" } } }"#,
            "column of type 'date'",
        ),
        (r#"ui.kit.Quiz { name = "q", questions = {} }"#, "ui.kit.Quiz needs questions"),
        (
            r#"ui.kit.Quiz { name = "q", questions = { { q = "x", options = { "a", "b" }, answer = 3 } } }"#,
            "ui.kit.Quiz question 1 needs answer",
        ),
        (r#"ui.kit.Timer { seconds = 0 }"#, "ui.kit.Timer needs seconds above 0"),
        (r#"ui.kit.Timer { secs = 5 }"#, "ui.kit.Timer has no prop 'secs'"),
    ];
    for (call, expected) in cases {
        let script = format!("local x = 1\n\nreturn function() return {call} end");
        let error = check(&script, 1).unwrap_err();
        assert!(error.message.contains(expected), "{call}: {}", error.message);
        assert!(error.message.contains("visual:3"), "{call}: {}", error.message);
    }
}
