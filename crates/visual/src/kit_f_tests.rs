//! ui.image and ui.kit, batch f (plans/live-visuals G10): Image, Gallery,
//! Compare and Hotspots, all built on the picture node. Their examples are
//! checked with the rest in kit_tests.rs (with visuals/assets).

use super::*;
use std::sync::Arc;

/// The kinds this batch adds; kit_tests.rs wants an example of each.
pub(crate) const KINDS_F: &[&str] = &["Image", "Gallery", "Compare", "Hotspots"];

fn png(width: u32, height: u32) -> Arc<[u8]> {
    let mut out = Vec::new();
    ::image::RgbaImage::from_pixel(width, height, ::image::Rgba([40, 90, 200, 255]))
        .write_to(&mut std::io::Cursor::new(&mut out), ::image::ImageFormat::Png)
        .unwrap();
    out.into()
}

fn assets(names: &[&str]) -> Assets {
    Assets::new(names.iter().map(|n| (n.to_string(), png(32, 20))).collect())
}

fn render(script: &str, names: &[&str]) -> (Visual, Node) {
    let mut visual = Visual::load(script, 1)
        .unwrap_or_else(|e| panic!("{script}: {}", e.message))
        .with_assets(assets(names));
    let tree = visual.render().unwrap_or_else(|e| panic!("{script}: {}", e.message));
    (visual, tree)
}

fn pictures(node: &Node, out: &mut Vec<Picture>) {
    if let NodeKind::Image(p) = &node.kind {
        out.push((**p).clone());
    }
    children(node).for_each(|c| pictures(c, out));
}

fn only_picture(script: &str, names: &[&str]) -> Picture {
    let (_, tree) = render(script, names);
    let mut found = Vec::new();
    pictures(&tree, &mut found);
    assert_eq!(found.len(), 1, "{script}");
    found.remove(0)
}

fn texts(node: &Node, out: &mut Vec<String>) {
    if let NodeKind::Text(t) = &node.kind {
        out.push(t.spans.iter().map(|s| s.text.as_str()).collect());
    }
    children(node).for_each(|c| texts(c, out));
}

fn check_err(script: &str, names: &[&str]) -> String {
    check_with_assets(script, 1, assets(names)).map(|_| ()).unwrap_err().message
}

#[test]
fn a_picture_names_one_of_the_visuals_files_and_its_props() {
    let p = only_picture(
        r#"return ui.image { src = "a.png", alt = "Shot", height = 200, fit = "cover" }"#,
        &["a.png"],
    );
    assert_eq!((p.src.as_str(), p.alt.as_deref(), p.height, p.fit), ("a.png", Some("Shot"), Some(200.0), Fit::Cover));
    assert_eq!((p.before, p.split, p.marks.len(), p.selected, p.on_click.is_some()), (None, 0.5, 0, None, false));
    // The name alone, as ui.text takes its words.
    assert_eq!(only_picture(r#"return ui.image "a.png""#, &["a.png"]).src, "a.png");
    // A wipe: before left of split, src right of it.
    let wipe = only_picture(r#"return ui.image { src = "b.png", before = "a.png", split = 0.3 }"#, &["a.png", "b.png"]);
    assert_eq!((wipe.before.as_deref(), wipe.split), (Some("a.png"), 0.3));
    // Marks: boxes and points in shares of the picture.
    let marked = only_picture(
        r#"return ui.image { src = "a.png", marks = { { x = 0.1, y = 0.2, w = 0.5, h = 0.3, label = "Box", tone = "warning" }, { x = 0.9, y = 0.9 } }, selected = 2, on_click = function(i) end }"#,
        &["a.png"],
    );
    assert_eq!(marked.marks.len(), 2);
    assert_eq!((marked.marks[0].w, marked.marks[0].label.as_deref()), (0.5, Some("Box")));
    assert!(marked.marks[0].tone.is_some());
    assert_eq!((marked.marks[1].w, marked.marks[1].h), (0.0, 0.0));
    assert_eq!(marked.selected, Some(2));
    assert!(marked.on_click.is_some());
}

#[test]
fn a_missing_or_wrong_picture_is_an_error_that_names_it() {
    let missing = check_err(r#"return ui.image { src = "shot.png" }"#, &[]);
    assert!(missing.contains("no asset named 'shot.png'"), "{missing}");
    let before = check_err(r#"return ui.image { src = "a.png", before = "old.png" }"#, &["a.png"]);
    assert!(before.contains("no asset named 'old.png'"), "{before}");
    // Words in a .png, and an HDR (a scene's, not a picture's).
    let words = Assets::new([("notes.png".to_owned(), Arc::from(&b"just words"[..]))].into_iter().collect());
    let e = check_with_assets(r#"return ui.image "notes.png""#, 1, words).unwrap_err().message;
    assert!(e.contains("'notes.png'") && e.contains("is not a picture"), "{e}");
    let hdr = Assets::new([("sky.hdr".to_owned(), Arc::from(&b"#?RADIANCE\nFORMAT=32-bit_rle_rgbe\n\n-Y 1 +X 1\n\x80\x80\x80\x80"[..]))].into_iter().collect());
    let e = check_with_assets(r#"return ui.image "sky.hdr""#, 1, hdr).unwrap_err().message;
    assert!(e.contains("'sky.hdr'") && e.contains("Radiance HDR"), "{e}");
    // Never a path or a link: the file comes with the visual.
    for src in ["/tmp/a.png", "https://example.com/a.png", "shots/a.png"] {
        let e = check_err(&format!(r#"return ui.image {{ src = "{src}" }}"#), &["a.png"]);
        assert!(e.contains("not a path or a link"), "{src}: {e}");
    }
    let cases = [
        (r#"ui.image {}"#, "needs src"),
        (r#"ui.image { src = "a.png", fit = "fill" }"#, "use contain or cover"),
        (r#"ui.image { src = "a.png", split = 2 }"#, "0 to 1"),
        (r#"ui.image { src = "a.png", marks = { { x = 0.5 } } }"#, "needs x and y"),
        (r#"ui.image { src = "a.png", marks = { { x = 0.5, y = 0.5, w = 0.2 } } }"#, "both w and h"),
        (r#"ui.image { src = "a.png", marks = { { x = 0.8, y = 0.5, w = 0.5, h = 0.1 } } }"#, "runs off"),
        (r#"ui.image { src = "a.png", marks = { { x = 0.5, y = 0.5, size = 3 } } }"#, "has no 'size'"),
        (r#"ui.image { src = "a.png", width_px = 3 }"#, "width_px"),
    ];
    for (node, want) in cases {
        let e = check_err(&format!("return {node}"), &["a.png"]);
        assert!(e.contains(want), "{node}: {e}");
    }
}

#[test]
fn clicking_a_mark_hands_the_script_its_number() {
    let script = r#"
        local s = ui.state { picked = 0 }
        return function()
          return ui.column {
            ui.image { src = "a.png", marks = { { x = 0.2, y = 0.2, label = "One" }, { x = 0.7, y = 0.6, label = "Two" } },
              on_click = function(i) s.picked = i end },
            ui.text { "picked " .. s.picked },
          }
        end
    "#;
    let (mut visual, tree) = render(script, &["a.png"]);
    let mut found = Vec::new();
    pictures(&tree, &mut found);
    let handler = found[0].on_click.unwrap();
    visual.call(handler, HandlerArg::Number(2.0)).unwrap();
    let tree = visual.render().unwrap();
    let mut words = Vec::new();
    texts(&tree, &mut words);
    assert!(words.contains(&"picked 2".to_owned()), "{words:?}");
    // The check clicks each mark, so a handler that fails is caught.
    let bad = script.replace("s.picked = i", "if i == 2 then error('boom') end");
    assert!(check_err(&bad, &["a.png"]).contains("boom"));
}

#[test]
fn image_is_a_picture_with_its_caption() {
    let (_, tree) = render(r#"return ui.kit.Image { src = "a.png", alt = "The page", caption = "Figure 1" }"#, &["a.png"]);
    let mut found = Vec::new();
    pictures(&tree, &mut found);
    assert_eq!((found.len(), found[0].alt.as_deref()), (1, Some("The page")));
    let mut words = Vec::new();
    texts(&tree, &mut words);
    assert_eq!(words, ["Figure 1"]);
    assert!(check_err(r#"return ui.kit.Image { alt = "x" }"#, &[]).contains("ui.kit.Image needs src"));
    assert!(check_err(r#"return ui.kit.Image { src = "a.png", width = 3 }"#, &["a.png"]).contains("has no prop 'width'"));
}

#[test]
fn gallery_lays_pictures_out_in_columns_cut_to_one_height() {
    let (_, tree) = render(
        r#"return ui.kit.Gallery { columns = 2, items = { { src = "a.png", caption = "A" }, "b.png", { src = "c.png" } } }"#,
        &["a.png", "b.png", "c.png"],
    );
    let NodeKind::Grid { columns, .. } = &tree.kind else { panic!("{tree:?}") };
    assert_eq!(*columns, 2);
    let mut found = Vec::new();
    pictures(&tree, &mut found);
    let srcs: Vec<_> = found.iter().map(|p| (p.src.as_str(), p.fit, p.height)).collect();
    assert_eq!(srcs, [("a.png", Fit::Cover, Some(140.0)), ("b.png", Fit::Cover, Some(140.0)), ("c.png", Fit::Cover, Some(140.0))]);
    assert_eq!(found[0].alt.as_deref(), Some("A"));
    assert!(check_err(r#"return ui.kit.Gallery { items = {} }"#, &[]).contains("needs items"));
    let e = check_err(r#"return ui.kit.Gallery { items = { "a.png", "gone.png" } }"#, &["a.png"]);
    assert!(e.contains("no asset named 'gone.png'"), "{e}");
    let e = check_err(r#"return ui.kit.Gallery { items = { { caption = "x" } } }"#, &[]);
    assert!(e.contains("item 1") && e.contains("needs src"), "{e}");
}

#[test]
fn compare_wipes_from_before_to_after() {
    let p = only_picture(
        r#"return ui.kit.Compare { before = "old.png", after = "new.png", split = 0.25, alt = "The header" }"#,
        &["old.png", "new.png"],
    );
    assert_eq!((p.src.as_str(), p.before.as_deref(), p.split), ("new.png", Some("old.png"), 0.25));
    let (_, tree) = render(r#"return ui.kit.Compare { before = "a.png", after = "b.png", labels = { "v1", "v2" } }"#, &["a.png", "b.png"]);
    let mut words = Vec::new();
    texts(&tree, &mut words);
    assert_eq!(words, ["◀ v1", "v2 ▶"]);
    assert!(check_err(r#"return ui.kit.Compare { before = "a.png" }"#, &["a.png"]).contains("needs before"));
    let e = check_err(r#"return ui.kit.Compare { before = "a.png", after = "b.png" }"#, &["a.png"]);
    assert!(e.contains("no asset named 'b.png'"), "{e}");
}

#[test]
fn hotspots_number_the_spots_and_a_click_picks_one() {
    let script = r#"
        local s = ui.state { pick = 0 }
        return function()
          return ui.kit.Hotspots { src = "a.png", selected = s.pick > 0 and s.pick or nil, on_select = function(i) s.pick = i end, spots = {
            { x = 0.1, y = 0.1, w = 0.3, h = 0.2, title = "Header", detail = "Logo and search" },
            { x = 0.6, y = 0.7, title = "Chart", tone = "warning" },
          } }
        end
    "#;
    let (mut visual, tree) = render(script, &["a.png"]);
    let mut found = Vec::new();
    pictures(&tree, &mut found);
    let p = &found[0];
    let labels: Vec<_> = p.marks.iter().map(|m| m.label.as_deref()).collect();
    assert_eq!(labels, [Some("Header"), Some("Chart")]);
    assert!(p.marks[1].tone.is_some());
    let mut words = Vec::new();
    texts(&tree, &mut words);
    assert!(words.contains(&"Click a number to read about it.".to_owned()), "{words:?}");
    visual.call(p.on_click.unwrap(), HandlerArg::Number(1.0)).unwrap();
    let tree = visual.render().unwrap();
    let mut found = Vec::new();
    pictures(&tree, &mut found);
    assert_eq!(found[0].selected, Some(1));
    let mut words = Vec::new();
    texts(&tree, &mut words);
    assert!(words.contains(&"Logo and search".to_owned()) && !words.contains(&"Chart".to_owned()), "{words:?}");
    // Without on_select every spot's details are listed.
    let (_, tree) = render(r#"return ui.kit.Hotspots { src = "a.png", spots = { { x = 0.1, y = 0.1, title = "A" }, { x = 0.5, y = 0.5, title = "B" } } }"#, &["a.png"]);
    let mut words = Vec::new();
    texts(&tree, &mut words);
    assert!(words.contains(&"A".to_owned()) && words.contains(&"B".to_owned()), "{words:?}");
    assert!(check_err(r#"return ui.kit.Hotspots { src = "a.png", spots = {} }"#, &["a.png"]).contains("needs spots"));
    assert!(check_err(r#"return ui.kit.Hotspots { src = "a.png", spots = { { x = 0.1, y = 0.1 } } }"#, &["a.png"]).contains("spot 1 needs title"));
    assert!(check_err(r#"return ui.kit.Hotspots { src = "a.png", selected = 5, spots = { { x = 0.1, y = 0.1, title = "A" } } }"#, &["a.png"]).contains("selected"));
    assert!(check_err(r#"return ui.kit.Hotspots { src = "a.png", spots = { { x = 2, y = 0.1, title = "A" } } }"#, &["a.png"]).contains("0 to 1"));
}
