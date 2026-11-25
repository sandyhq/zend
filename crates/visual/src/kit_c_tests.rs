//! ui.kit batch c: charts, drawing, media and 3D kinds.

use super::*;

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

fn canvases(n: &Node) -> usize {
    count(n, &|n| matches!(n.kind, NodeKind::Canvas(_)))
}

fn charts(n: &Node) -> usize {
    count(n, &|n| matches!(n.kind, NodeKind::Chart(_)))
}

/// Renders a node's canvases through the real draw path: a draw error fails.
fn draw_all(script: &str) -> Node {
    let mut visual = Visual::load(script, 1).unwrap_or_else(|e| panic!("{script}: {}", e.message));
    let node = visual.render().unwrap();
    let mut ids = Vec::new();
    collect_canvases(&node, &mut ids);
    for id in ids {
        visual
            .draw_canvas(id, 600.0, 300.0, 0.0, None)
            .unwrap_or_else(|e| panic!("{script}: {}", e.message));
    }
    node
}

fn collect_canvases(node: &Node, out: &mut Vec<HandlerId>) {
    if let NodeKind::Canvas(c) = &node.kind {
        out.push(c.draw);
    }
    for c in children(node) {
        collect_canvases(c, out);
    }
}

#[test]
fn chart_types_build_a_chart_node_or_a_canvas() {
    for kind in ["bar", "line", "area"] {
        let n = tree(&format!(
            r#"return ui.kit.Chart {{ type = "{kind}", labels = {{ "a", "b" }}, series = {{ Web = {{ 1, 2 }}, iOS = {{ 3, 4 }} }} }}"#
        ));
        assert_eq!(charts(&n), 1, "{kind}");
    }
    let scatter = tree(
        r#"return ui.kit.Chart { type = "scatter", labels = { 1, 2, 3 }, series = { { name = "p", values = { 5, 6, 7 } } } }"#,
    );
    assert_eq!(charts(&scatter), 1);
    let histogram = tree(
        r#"return ui.kit.Chart { type = "histogram", labels = {}, series = { { 1, 2, 2, 3, 3, 3, 4, 9 } } }"#,
    );
    assert_eq!(charts(&histogram), 1);
    for script in [
        r#"return ui.kit.Chart { type = "pie", labels = { "a", "b" }, series = { 3, 1 }, unit = "h" }"#,
        r#"return ui.kit.Chart { type = "radar", labels = { "a", "b", "c" }, series = { X = { 1, 2, 3 }, Y = { 3, 2, 1 } } }"#,
        r#"return ui.kit.Chart { type = "funnel", labels = { "a", "b" }, series = { 100, 40 } }"#,
        r#"return ui.kit.Chart { type = "heatmap", labels = { "x", "y" }, series = { Mon = { 1, 0 }, Tue = { 2, false } } }"#,
        r#"return ui.kit.Chart { type = "box", series = { A = { 1, 2, 3, 4, 50 }, B = { 2, 3, 4 } } }"#,
    ] {
        let n = draw_all(script);
        assert!(canvases(&n) >= 1, "{script}");
    }
    // pie draws one legend line per slice
    let pie = draw_all(r#"return ui.kit.Chart { type = "pie", labels = { "a", "b" }, series = { 3, 1 } }"#);
    assert_eq!(canvases(&pie), 1);
}

#[test]
fn small_charts_draw() {
    let spark = draw_all(r#"return ui.kit.Sparkline { values = { 1, 3, 2 }, tone = "good" }"#);
    assert_eq!(canvases(&spark), 1);
    let bar = draw_all(
        r#"return ui.kit.SegmentedBar { unit = "h", parts = { { label = "A", value = 3 }, { label = "B", value = 1, tone = "neutral" } } }"#,
    );
    assert_eq!(canvases(&bar), 1);
    assert_eq!(text_of(&bar), "|●|A|3 h  75%|●|B|1 h  25%");
    let wf = draw_all(
        r#"return ui.kit.Waterfall { unit = "ms", spans = { { name = "root", start = 0, duration = 10 }, { name = "q", start = 2, duration = 5, depth = 1, tone = "warn", detail = "slow" } } }"#,
    );
    assert!(text_of(&wf).ends_with("q: slow"), "{}", text_of(&wf));
    let tm = draw_all(
        r#"return ui.kit.Treemap { unit = "bytes", items = { { path = "a/x.ts", size = 3000 }, { path = "a/y.ts", size = 1000 }, { path = "b/z.ts", size = 2048 } } }"#,
    );
    assert_eq!(canvases(&tm), 1);
}

#[test]
fn tree_lists_paths_and_nodes() {
    let t = tree(
        r#"return ui.kit.Tree { items = { "src/a.ts", "src/b.ts", { label = "db", children = { "m.sql" }, note = "new" } } }"#,
    );
    assert_eq!(
        text_of(&t),
        "├─ |src/|│  ├─ |a.ts|│  └─ |b.ts|└─ |db/|   new|   └─ |m.sql"
    );
}

#[test]
fn icon_carousel_and_map() {
    assert_eq!(text_of(&tree(r#"return ui.kit.Icon { name = "Check", tone = "good" }"#)), "✓");
    let mut visual = Visual::load(
        r#"return function() return ui.kit.Carousel { key = "t", ui.text "one", ui.text "two" } end"#,
        1,
    )
    .unwrap();
    let first = visual.render().unwrap();
    assert_eq!(text_of(&first), "one|‹|1 / 2|›");
    let NodeKind::Stack(outer) = &first.kind else { unreachable!() };
    let NodeKind::Stack(bar) = &outer.children[1].kind else { unreachable!() };
    let NodeKind::Stack(next) = &bar.children[2].kind else { unreachable!() };
    visual.call(next.on_click.unwrap(), HandlerArg::None).unwrap();
    assert_eq!(text_of(&visual.render().unwrap()), "two|‹|2 / 2|›");

    let map = draw_all(
        r#"return ui.kit.Map { route = true, places = { { name = "A", lat = 38.7, lng = -9.2, note = "n", day = 1 }, { name = "B", lat = 38.8, lng = -9.4, day = 1 } } }"#,
    );
    assert_eq!(canvases(&map), 1);
    assert!(text_of(&map).contains("A|n"), "{}", text_of(&map));
}

#[test]
fn diagram_draws_flowcharts_and_sequences() {
    let flow = draw_all(
        r#"return ui.kit.Diagram { mermaid = "flowchart TD\n a[Start] --> b{Ok?}\n b -->|yes| c((Done))\n b -.-> a" }"#,
    );
    assert_eq!(canvases(&flow), 1);
    let lr = draw_all(
        r#"return ui.kit.Diagram { mermaid = "graph LR\n a --> b --> c\n c ==> a", details = { a = "first" } }"#,
    );
    assert_eq!(canvases(&lr), 1);
    assert!(text_of(&lr).contains("Click a node"));
    let seq = draw_all(
        r#"return ui.kit.Diagram { mermaid = "sequenceDiagram\n participant A as Alice\n A->>B: hi\n B-->>A: yo\n Note over A,B: ok" }"#,
    );
    assert_eq!(canvases(&seq), 1);

    // A click on a node shows its details.
    let mut visual = Visual::load(
        r#"return function() return ui.kit.Diagram { mermaid = "flowchart LR\n a[Alpha] --> b[Beta]", details = { a = "about alpha" } } end"#,
        1,
    )
    .unwrap();
    let node = visual.render().unwrap();
    let mut ids = Vec::new();
    collect_canvases(&node, &mut ids);
    visual.draw_canvas(ids[0], 600.0, 100.0, 0.0, None).unwrap();
    let click = children(&node)
        .find_map(|c| match &c.kind {
            NodeKind::Canvas(c) => c.on_click,
            _ => None,
        })
        .expect("a canvas with on_click");
    // The chart is centred in 600 px: node a (64 wide) sits near x = 238, y = 21.
    visual.call(click, HandlerArg::Point(238.0, 21.0)).unwrap();
    let after = text_of(&visual.render().unwrap());
    assert!(after.contains("Alpha|about alpha"), "{after}");
}

/// A flowchart wider than the column, as an agent wrote it: five nodes in a
/// row with long labels and two labelled edges.
const WIDE_FLOW: &str = r#"return ui.kit.Diagram { mermaid = [=[flowchart LR
  b1["browser on laptop"] --> ls["local fork server 127.0.0.1"]
  ls --> hub["hub space: journal/ zen/ anexia/ (symlinks)"]
  hub --> jr["~/Projects/journal"]
  jr -- "sync: the open question" --> gh[("GitHub private: nesyeklaf/journal")]
  ai["AI fork server: space journal"] -- "built-in Git Sync" --> gh
  b2["phone or browser elsewhere"] --> ai
]=] }"#;

/// Node boxes (fills larger than an arrow head) and node labels (sm text).
fn flow_boxes_and_labels(cmds: &[DrawCmd]) -> (Vec<[f32; 4]>, Vec<(f32, f32, String)>) {
    let mut boxes = Vec::new();
    let mut labels = Vec::new();
    for cmd in cmds {
        match cmd {
            DrawCmd::Fill { points, .. } => {
                let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
                for p in points {
                    x0 = x0.min(p[0]);
                    y0 = y0.min(p[1]);
                    x1 = x1.max(p[0]);
                    y1 = y1.max(p[1]);
                }
                if x1 - x0 > 20.0 && y1 - y0 > 20.0 {
                    boxes.push([x0, y0, x1, y1]);
                }
            }
            DrawCmd::Text(t) if t.size == crate::style::TextSize::Sm => {
                labels.push((t.x, t.y, t.text.clone()));
            }
            _ => {}
        }
    }
    (boxes, labels)
}

#[test]
fn a_wide_flowchart_fits_the_column_without_overlaps() {
    for width in [685.0_f32, 360.0] {
        let mut visual = Visual::load(WIDE_FLOW, 1).unwrap();
        let node = visual.render().unwrap();
        let canvas = match &node.kind {
            NodeKind::Canvas(c) => c.clone(),
            _ => panic!("a canvas"),
        };
        let cmds = visual
            .draw_canvas(canvas.draw, width, canvas.height, 0.0, None)
            .unwrap();
        let (boxes, labels) = flow_boxes_and_labels(&cmds);
        assert_eq!(boxes.len(), 7, "{width}: one box per node");
        for (i, a) in boxes.iter().enumerate() {
            assert!(
                a[0] >= 0.0 && a[2] <= width && a[1] >= 0.0 && a[3] <= canvas.height,
                "{width}: box {a:?} outside {width}x{}",
                canvas.height
            );
            for b in &boxes[i + 1..] {
                let apart = a[2] <= b[0] || b[2] <= a[0] || a[3] <= b[1] || b[3] <= a[1];
                assert!(apart, "{width}: boxes overlap: {a:?} and {b:?}");
            }
        }
        // Every label line sits inside a box, at the kit's width estimate.
        for (x, y, text) in &labels {
            let half = text.len() as f32 * 6.6 / 2.0;
            let inside = boxes
                .iter()
                .any(|b| x - half >= b[0] && x + half <= b[2] && *y >= b[1] && *y <= b[3]);
            assert!(
                inside,
                "{width}: label {text:?} at ({x}, {y}) spills out of its box"
            );
        }
    }
}

#[test]
fn scene_and_shader_wrap_the_3d_nodes() {
    let scene = tree(
        r#"return ui.kit.Scene { objects = { { shape = "box", at = { 0, 0, 0 } }, { shape = "point", at = { 1, 1, 1 }, hint = "h" }, { shape = "label", at = { 0, 2, 0 }, text = "t" }, { shape = "line", from = { 0, 0, 0 }, to = { 1, 1, 1 } }, { shape = "path", points = { { 0, 0, 0 }, { 1, 0, 0 } } }, { shape = "grid" } } }"#,
    );
    assert!(matches!(scene.kind, NodeKind::Scene(_)));
    check(
        r#"return ui.kit.Scene { objects = function(t) return { { shape = "sphere", at = { math.sin(t), 0, 0 }, radius = 0.5 } } end }"#,
        1,
    )
    .unwrap();
    let shader = tree(
        r#"return ui.kit.Shader { inputs = { k = 1 }, code = "void mainImage(out vec4 c, in vec2 p) { c = vec4(k); }" }"#,
    );
    assert!(matches!(shader.kind, NodeKind::Shader(_)));
}

#[test]
fn errors_name_the_kind_and_the_line() {
    for (script, needle) in [
        (r#"return ui.kit.Chart { type = "pie", series = { 1 }, oops = 1 }"#, "ui.kit.Chart has no prop 'oops'"),
        (r#"return ui.kit.Chart { type = "donut", series = { 1 } }"#, "ui.kit.Chart has unknown type 'donut'"),
        (r#"return ui.kit.Chart { type = "bar" }"#, "ui.kit.Chart needs series"),
        (r#"return ui.kit.Sparkline { values = { 1 } }"#, "ui.kit.Sparkline needs values"),
        (r#"return ui.kit.SegmentedBar { parts = { { label = "a", value = 0 } } }"#, "ui.kit.SegmentedBar needs values above zero"),
        (r#"return ui.kit.Waterfall { spans = { { name = "a" } } }"#, "ui.kit.Waterfall span 1 duration"),
        (r#"return ui.kit.Treemap { items = { { path = "a" } } }"#, "ui.kit.Treemap item 1 size"),
        (r#"return ui.kit.Tree { items = {} }"#, "ui.kit.Tree needs items"),
        (r#"return ui.kit.Diagram { mermaid = "pie\n a" }"#, "ui.kit.Diagram draws flowcharts"),
        (r#"return ui.kit.Diagram { mermaid = "flowchart LR\n subgraph x" }"#, "ui.kit.Diagram does not support subgraph"),
        (r#"return ui.kit.Map { places = { { name = "A" } } }"#, "ui.kit.Map place 'A' needs lat and lng"),
        (r#"return ui.kit.Scene { objects = { { shape = "cube" } }, bogus = 1 }"#, "ui.kit.Scene has no prop 'bogus'"),
        (r#"return ui.kit.Shader { height = 100 }"#, "ui.kit.Shader needs code"),
        (r#"return ui.kit.Carousel { }"#, "ui.kit.Carousel needs its slides"),
        (r#"return ui.kit.Icon { name = "Nope" }"#, "ui.kit.Icon has no icon 'Nope'"),
    ] {
        let err = Visual::load(script, 1).and_then(|mut v| v.render()).unwrap_err();
        assert!(err.message.contains(needle), "{script}: {}", err.message);
    }
    // The error carries the agent's line.
    let err = Visual::load("local x = 1\nreturn ui.kit.Icon { name = \"Nope\" }", 1)
        .and_then(|mut v| v.render())
        .unwrap_err();
    assert!(err.message.contains(":2"), "{}", err.message);
    // A shape the scene does not know fails when drawn, naming the kind.
    let err = check(r#"return ui.kit.Scene { objects = { { shape = "cube" } } }"#, 1).unwrap_err();
    assert!(err.message.contains("ui.kit.Scene has unknown shape 'cube'"), "{}", err.message);
}
