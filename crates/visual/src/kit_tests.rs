//! ui.kit: each kind builds from the plain nodes, and every example in the
//! repo's visuals/ passes the check show_visual runs.

use super::*;
use std::path::{Path, PathBuf};

/// Every kind the kit has. A new kind goes here, gets a case in
/// each_kind_builds_its_nodes and an example in visuals/.
const KINDS: &[&str] = &[
    "Badge",
    "Callout",
    "Card",
    "Checklist",
    "Choice",
    "DateInput",
    "Details",
    "Form",
    "FollowUps",
    "Gauge",
    "Input",
    "KeyValue",
    "List",
    "Metric",
    "OptionCards",
    "Progress",
    "Radio",
    "Select",
    "Status",
    "Steps",
    "TextArea",
    "Toggle",
    // Batch c (G7c): charts, drawing, media, 3D.
    "Carousel",
    "Chart",
    "Diagram",
    "Icon",
    "Map",
    "Scene",
    "SegmentedBar",
    "Shader",
    "Sparkline",
    "Tree",
    "Treemap",
    "Waterfall",
    // Batch e (kit_e.luau, G7e).
    "EditableTable",
    "ListEditor",
    "Quiz",
    "Rating",
    "Slider",
    "Stepper",
    "Timer",
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

fn is_answer(n: &Node) -> bool {
    matches!(n.kind, NodeKind::Answer { .. })
}

fn is_canvas(n: &Node) -> bool {
    matches!(n.kind, NodeKind::Canvas(_))
}

#[test]
fn the_kit_has_exactly_the_listed_kinds() {
    let listed = text_of(&tree(
        r#"local names = {}
        for name in pairs(ui.kit) do table.insert(names, name) end
        table.sort(names)
        return ui.text { table.concat(names, ",") }"#,
    ));
    let mut kinds = KINDS.to_vec();
    kinds.extend(super::kit_a_tests::KINDS_A); // batch a
    kinds.extend(super::kit_b_tests::KINDS_B);
    kinds.sort();
    assert_eq!(listed, kinds.join(","));
}

#[test]
fn each_kind_builds_its_nodes() {
    let one_input = |script: &str, kind: InputKind| {
        let t = tree(script);
        let found = inputs(&t);
        assert_eq!(found.len(), 1, "{script}");
        assert_eq!(found[0].kind, kind, "{script}");
        found[0].clone()
    };
    let i = one_input(
        r#"return ui.kit.Input { name = "p", label = "Project", placeholder = "my-app", value = "x" }"#,
        InputKind::Field,
    );
    assert_eq!((i.name.as_str(), i.label.as_deref()), ("p", Some("Project")));
    assert_eq!(i.placeholder.as_deref(), Some("my-app"));
    assert_eq!(i.value, EntryValue::Text("x".into()));
    one_input(r#"return ui.kit.TextArea { name = "n" }"#, InputKind::Area);
    let d = one_input(r#"return ui.kit.DateInput { name = "d" }"#, InputKind::Field);
    assert_eq!(d.placeholder.as_deref(), Some("yyyy-mm-dd"));
    let t = one_input(r#"return ui.kit.Toggle { name = "t", value = true }"#, InputKind::Check);
    assert_eq!(t.value, EntryValue::Bool(true));
    one_input(
        r#"return ui.kit.Checklist { name = "c", options = { "a", "b" } }"#,
        InputKind::Checklist,
    );
    one_input(
        r#"return ui.kit.Choice { name = "c", options = { "a", "b" } }"#,
        InputKind::Choice { many: false },
    );
    one_input(
        r#"return ui.kit.Choice { name = "c", options = { "a", "b" }, many = true }"#,
        InputKind::Choice { many: true },
    );
    let r = one_input(
        r#"return ui.kit.Radio { name = "r", options = { { value = "m", detail = "Cancel any time" } } }"#,
        InputKind::Choice { many: false },
    );
    assert_eq!(r.options[0].description.as_deref(), Some("Cancel any time"));
    one_input(
        r#"return ui.kit.Select { name = "s", options = { "a", "b" }, placeholder = "Pick" }"#,
        InputKind::Select,
    );
    let o = one_input(
        r#"return ui.kit.OptionCards { name = "o", value = { "b" },
            options = { { value = "a", title = "A", detail = "first" }, { value = "b", title = "B" } } }"#,
        InputKind::Choice { many: true },
    );
    assert_eq!(
        (o.options[0].label.as_str(), o.options[0].description.as_deref()),
        ("A", Some("first"))
    );
    one_input(
        r#"return ui.kit.OptionCards { name = "o", options = { { value = "a", title = "A" } } }"#,
        InputKind::Choice { many: false },
    );

    // FollowUps: a choice and its own answer button, or none.
    let f = tree(r#"return ui.kit.FollowUps { prompts = { "Why?", "How?" } }"#);
    assert_eq!(inputs(&f)[0].name, "follow_up");
    assert_eq!(count(&f, &is_answer), 1);
    let f = tree(r#"return ui.kit.FollowUps { prompts = { "Why?" }, answer = false }"#);
    assert_eq!(count(&f, &is_answer), 0);

    let card = tree(r#"return ui.kit.Card { title = "Deploy", subtitle = "web", "body" }"#);
    assert_eq!(text_of(&card), "Deploy|web|body");
    let callout = tree(
        r#"return ui.kit.Callout { title = "Careful", text = "Back up first", tone = "warn" }"#,
    );
    assert_eq!(text_of(&callout), "!|Careful|Back up first");
    assert_eq!(
        text_of(&tree(r#"return ui.kit.Badge { text = "2 failing", tone = "bad" }"#)),
        "2 failing"
    );
    let kv = tree(
        r#"return ui.kit.KeyValue { pairs = { { "Branch", "fix" }, { "Checks", ui.kit.Badge { "ok" } } } }"#,
    );
    assert_eq!(text_of(&kv), "Branch|fix|Checks|ok");
    let sorted = tree(r#"return ui.kit.KeyValue { pairs = { b = 2, a = 1 } }"#);
    assert_eq!(text_of(&sorted), "a|1|b|2");
    let list = tree(
        r#"return ui.kit.List { items = { "one", { title = "two", detail = "d", value = "9", tone = "good" } } }"#,
    );
    assert_eq!(text_of(&list), "•|one|●|two|d|9");
    let steps = tree(
        r#"return ui.kit.Steps { items = { { title = "Freeze", status = "done" }, { title = "QA", status = "current" }, "Ship" } }"#,
    );
    assert_eq!(text_of(&steps), "✓|Freeze|●|QA|○|Ship");
    let status = tree(
        r#"return ui.kit.Status { items = { { name = "Site", status = "at-risk", owner = "Ana", progress = 40 } } }"#,
    );
    assert_eq!(text_of(&status), "●|Site|At risk|Ana · 40%");
    let metric = tree(
        r#"return ui.kit.Metric { label = "CSS", value = "22 KB", note = "gzipped", tone = "good" }"#,
    );
    assert_eq!(count(&metric, &|n| matches!(n.kind, NodeKind::Stat { .. })), 1);
    assert_eq!(text_of(&metric), "|gzipped");
    let progress = tree(r#"return ui.kit.Progress { value = 3, max = 12, label = "Tables" }"#);
    assert_eq!(count(&progress, &is_canvas), 1);
    assert_eq!(count(&tree(r#"return ui.kit.Gauge { value = 72 }"#), &is_canvas), 1);

    // Details: closed shows only its header; a click opens it.
    let mut visual = Visual::load(
        r#"return function() return ui.kit.Details { title = "More", "hidden" } end"#,
        1,
    )
    .unwrap();
    let t = visual.render().unwrap();
    assert_eq!(text_of(&t), "▸|More");
    let NodeKind::Stack(outer) = &t.kind else {
        unreachable!()
    };
    let NodeKind::Stack(header) = &outer.children[0].kind else {
        unreachable!()
    };
    visual.call(header.on_click.unwrap(), HandlerArg::None).unwrap();
    assert_eq!(text_of(&visual.render().unwrap()), "▾|More|hidden");
}

#[test]
fn a_form_ends_in_its_one_answer_button() {
    let form = tree(
        r#"return ui.kit.Form { title = "Q", submit = "Add answers",
            ui.kit.Input { name = "a" },
            ui.kit.FollowUps { prompts = { "x" } },
            ui.answer {},
        }"#,
    );
    assert_eq!(count(&form, &is_answer), 1);
    let NodeKind::Stack(stack) = &form.kind else {
        unreachable!()
    };
    let NodeKind::Answer { label } = &stack.children.last().unwrap().kind else {
        panic!("the answer button is not last")
    };
    assert_eq!(label.as_deref(), Some("Add answers"));
    // The answer text is the inputs', as for plain nodes.
    let mut visual = Visual::load(
        r#"return function() return ui.kit.Form { ui.kit.Input { name = "goal", label = "Goal" } } end"#,
        1,
    )
    .unwrap();
    visual.set_entry("goal", Some(EntryValue::Text("ship".into())));
    assert_eq!(
        answer_text("Plan", &visual.render().unwrap()),
        "### Plan\nGoal: ship"
    );
}

#[test]
fn kit_entries_are_kept_by_name_like_plain_inputs() {
    let mut visual = Visual::load(
        r#"return function() return ui.kit.Form { ui.kit.Checklist { name = "c", options = { "a", "b" } } } end"#,
        1,
    )
    .unwrap();
    visual.render().unwrap();
    visual.set_entry("c", Some(EntryValue::List(vec!["b".into()])));
    visual
        .swap(r#"return function() return ui.kit.Form { ui.kit.Checklist { name = "c", options = { "a", "b" } }, ui.kit.Input { name = "n" } } end"#)
        .unwrap();
    let t = visual.render().unwrap();
    let found = inputs(&t);
    assert_eq!(found[0].value, EntryValue::List(vec!["b".into()]));
    assert!(found[1].fresh);
}

#[test]
fn kit_errors_name_the_kind_and_the_agents_line() {
    let unknown =
        check("\n\nreturn ui.kit.Card { title = \"x\", colour = \"red\" }", 1).unwrap_err();
    assert!(unknown.message.contains("visual:3"), "{}", unknown.message);
    assert!(
        unknown.message.contains("ui.kit.Card has no prop 'colour'"),
        "{}",
        unknown.message
    );
    let name = check(r#"return ui.kit.Input { label = "Goal" }"#, 1).unwrap_err();
    assert!(name.message.contains("ui.kit.Input needs name"), "{}", name.message);
    assert!(name.message.contains("visual:1"), "{}", name.message);
    let tone = check(r#"return ui.kit.Badge { text = "x", tone = "red" }"#, 1).unwrap_err();
    assert!(tone.message.contains("unknown tone 'red'"), "{}", tone.message);
    let status = check(
        r#"return ui.kit.Steps { items = { { title = "x", status = "late" } } }"#,
        1,
    )
    .unwrap_err();
    assert!(status.message.contains("unknown status 'late'"), "{}", status.message);
    let empty = check(r#"return ui.kit.Form { title = "x" }"#, 1).unwrap_err();
    assert!(empty.message.contains("needs its questions"), "{}", empty.message);
    // The kit is read-only, like the rest of ui.
    assert!(check(r#"ui.kit.Card = nil return ui.text "x""#, 1).is_err());
}

fn visuals_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../visuals")
}

fn examples(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            examples(&path, out);
        } else if path.extension().is_some_and(|e| e == "luau") {
            out.push(path);
        }
    }
}

/// The header G5's gallery reads: title, category (its folder), description
/// and tags, as the file's leading comments.
fn header(source: &str) -> Vec<(&str, &str)> {
    source
        .lines()
        .map_while(|l| l.strip_prefix("-- "))
        .filter_map(|l| l.split_once(": "))
        .collect()
}

#[test]
fn every_example_passes_the_show_visual_check_and_each_kind_has_one() {
    let mut files = Vec::new();
    examples(&visuals_dir(), &mut files);
    files.sort();
    assert!(!files.is_empty(), "no examples under {}", visuals_dir().display());
    let mut used = std::collections::BTreeSet::new();
    for file in &files {
        let source = std::fs::read_to_string(file).unwrap();
        let shown = file.strip_prefix(visuals_dir()).unwrap().display().to_string();
        let head = header(&source);
        let keys: Vec<_> = head.iter().map(|(k, _)| *k).collect();
        assert_eq!(keys, ["title", "category", "description", "tags"], "{shown}: header");
        assert_eq!(
            Some(head[1].1),
            file.parent().and_then(|p| p.file_name()).and_then(|n| n.to_str()),
            "{shown}: category is its folder"
        );
        // The same check show_visual runs, with a visual id's seed.
        check(&source, seed_for(&shown)).unwrap_or_else(|e| panic!("{shown}: {}", e.message));
        for kind in KINDS.iter().chain(super::kit_a_tests::KINDS_A).chain(super::kit_b_tests::KINDS_B) {
            if source.contains(&format!("ui.kit.{kind} ")) {
                used.insert(*kind);
            }
        }
    }
    let missing: Vec<_> = KINDS.iter().chain(super::kit_a_tests::KINDS_A).chain(super::kit_b_tests::KINDS_B).filter(|k| !used.contains(*k)).collect();
    assert!(missing.is_empty(), "kinds without an example: {missing:?}");
}
