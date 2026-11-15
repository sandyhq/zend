use super::*;

fn text_of(node: &Node) -> String {
    match &node.kind {
        NodeKind::Text(text) => text.spans.iter().map(|s| s.text.as_str()).collect(),
        _ => children(node).map(text_of).collect::<Vec<_>>().join("|"),
    }
}

fn find<'a>(node: &'a Node, pred: &dyn Fn(&Node) -> bool) -> Option<&'a Node> {
    if pred(node) {
        return Some(node);
    }
    children(node).find_map(|child| find(child, pred))
}

#[test]
fn a_view_renders_and_handlers_change_state() {
    let mut visual = Visual::load(
        r#"
        local s = ui.state { tab = "p95" }
        return function()
          return ui.column { gap = 12,
            ui.text { "Latency ", ui.span { s.tab, weight = 600, color = "accent" }, size = "lg" },
            ui.tabs { options = { "p95", "p99" }, value = s.tab, on_change = function(v) s.tab = v end },
          }
        end
        "#,
        1,
    )
    .unwrap();
    let tree = visual.render().unwrap();
    assert_eq!(text_of(&tree), "Latency p95|");
    let tabs = find(&tree, &|n| matches!(n.kind, NodeKind::Tabs { .. })).unwrap();
    let NodeKind::Tabs { on_change, .. } = tabs.kind else {
        unreachable!()
    };
    visual
        .call(on_change, HandlerArg::Text("p99".into()))
        .unwrap();
    assert_eq!(text_of(&visual.render().unwrap()), "Latency p99|");
}

#[test]
fn a_script_may_return_a_static_node() {
    let mut visual = Visual::load(r#"return ui.text "hello""#, 1).unwrap();
    assert_eq!(text_of(&visual.render().unwrap()), "hello");
}

#[test]
fn style_rules_are_enforced_with_reasons() {
    for (script, needle) in [
        (
            r##"return ui.text { "x", color = "#ff0000" }"##,
            "theme tokens",
        ),
        (
            r#"return ui.column { background = "accent" }"#,
            "no containers",
        ),
        (
            r#"return ui.text { "x", font = "Comic Sans" }"#,
            "interface font",
        ),
        (r#"return ui.text { "x", size = "40px" }"#, "type scale"),
        (r#"return ui.row { colour = "x" }"#, "has no 'colour'"),
        (
            r#"return ui.chart { type = "pie", series = {} }"#,
            "ui.canvas",
        ),
        (
            r#"return ui.canvas { draw = function() end }"#,
            "needs height",
        ),
    ] {
        let err = Visual::load(script, 1)
            .and_then(|mut v| v.render())
            .unwrap_err();
        assert!(err.message.contains(needle), "{script}: {}", err.message);
    }
}

#[test]
fn the_sandbox_has_no_io_and_bounds_time_and_memory() {
    for script in [
        "return os.execute('true')",
        "return io.open('/etc/passwd')",
        "return require('x')",
    ] {
        assert!(Visual::load(script, 1).is_err(), "{script}");
    }
    let started = Instant::now();
    let err = Visual::load("while true do end", 1).err().unwrap();
    assert!(err.message.contains("time budget"), "{}", err.message);
    assert!(started.elapsed() < Duration::from_secs(2));

    let err = Visual::load(
        "local t = {} for i = 1, 1e9 do t[i] = string.rep('x', 1024) .. i end",
        1,
    )
    .err()
    .unwrap();
    assert!(
        err.message.contains("MB") || err.message.contains("time budget"),
        "{}",
        err.message
    );
    // The ui library can't be replaced by a script.
    assert!(Visual::load("ui.text = nil return ui.text 'x'", 1).is_err());
}

/// R5: Luau doesn't validate bytecode, so a script that is a binary chunk
/// (precompiled, or hand-made to corrupt the VM) is refused, never run.
#[test]
fn a_bytecode_script_is_refused() {
    // A real compiled chunk whose bytes happen to be UTF-8 (as a script's
    // must): before R5 it ran.
    let script = ["return { kind = 'text' }", "return 7", "return nil", "return {}", "return 'x'"]
        .iter()
        .find_map(|source| {
            for level in 0..=2 {
                let bytes = mlua::chunk::Compiler::new().set_optimization_level(level).compile(source).unwrap();
                if let Ok(text) = String::from_utf8(bytes) {
                    return Some(text);
                }
            }
            None
        })
        .expect("a chunk whose bytecode is UTF-8");
    let err = Visual::load(&script, 1).err().expect("bytecode was run");
    assert!(err.message.contains("binary chunk"), "{}", err.message);
    // A hand-made chunk too: a version byte, then garbage.
    let err = Visual::load("\u{6}\u{3}\u{1}garbage", 1).err().expect("garbage bytecode was loaded");
    assert!(err.message.contains("binary chunk"), "{}", err.message);
}

#[test]
fn random_data_is_stable_across_renders() {
    let script = r#"
        return function() return ui.text { tostring(math.random(1, 1000000)) } end
    "#;
    let mut a = Visual::load(script, 7).unwrap();
    let first = text_of(&a.render().unwrap());
    assert_eq!(first, text_of(&a.render().unwrap()));
    let mut b = Visual::load(script, 7).unwrap();
    assert_eq!(first, text_of(&b.render().unwrap()));
}

#[test]
fn canvases_record_theme_colored_commands() {
    let mut visual = Visual::load(
        r#"
        return ui.canvas { height = 120, draw = function(g, w, h)
          g:line(0, h, w, 0, { color = "border" })
          g:rect(10, 10, 40, 20, { fill = "chart2", radius = 4 })
          g:circle(w / 2, h / 2, 8, { color = ui.color("accent", 0.5) })
          g:arc(60, 60, 30, 0, math.pi, { fill = "chart3", inner = 20 })
          g:text(w, 0, "label", { align = "end", color = "muted", size = "xs" })
        end }
        "#,
        1,
    )
    .unwrap();
    let tree = visual.render().unwrap();
    let NodeKind::Canvas(canvas) = &tree.kind else {
        panic!()
    };
    assert_eq!(canvas.height, 120.0);
    let commands = visual
        .draw_canvas(canvas.draw, 300.0, 120.0, 0.0, None)
        .unwrap();
    assert_eq!(commands.len(), 5);
    assert!(matches!(&commands[0], DrawCmd::Stroke { color, .. } if color.token == Token::Border));
    assert!(matches!(&commands[2], DrawCmd::Fill { color, .. } if color.alpha == 0.5));
    let DrawCmd::Text(text) = &commands[4] else {
        panic!()
    };
    assert_eq!((text.x, text.align), (300.0, Align::End));
}

#[test]
fn scenes_project_lit_depth_sorted_geometry() {
    let mut visual = Visual::load(
        r#"
        return ui.scene { height = 300, spin = 0.3, draw = function(s)
          s:grid { size = 4, step = 1 }
          s:surface { x = { -2, 2 }, z = { -2, 2 }, detail = 12,
            colors = { "chart1", "chart3" },
            f = function(x, z) return math.sin(x) * math.cos(z) end }
          s:box { at = { 0, 1.5, 0 }, size = 0.5, color = "chart2" }
          s:point({ 1, 1, 1 }, { hint = "peak" })
          s:point { at = { -1, 1, -1 }, color = "chart4" }
          s:label { at = { 0, 2, 0 }, text = "top" }
        end }
        "#,
        1,
    )
    .unwrap();
    let tree = visual.render().unwrap();
    let NodeKind::Scene(node) = &tree.kind else {
        panic!()
    };
    let scene = visual.draw_scene(node.draw, 0.0).unwrap();
    assert_eq!(scene.face_count(), 12 * 12 + 6);
    let projection = scene.project(
        Camera {
            yaw: node.yaw,
            pitch: node.pitch,
            zoom: node.zoom,
        },
        640.0,
        300.0,
    );
    assert!(matches!(projection.prims.last(), Some(Prim::Label { .. })));
    assert_eq!(projection.hints.len(), 1);
    // A closed box shows at most three faces.
    let box_faces = projection
        .prims
        .iter()
        .filter(|p| matches!(p, Prim::Face { color, .. } if color.token == Token::Chart(1)))
        .count();
    assert!((1..=3).contains(&box_faces), "{box_faces}");
}

#[test]
fn charts_and_tables_parse() {
    let mut visual = Visual::load(
        r#"
        return ui.column {
          ui.chart { type = "line", height = 200, x = { "Mon", "Tue", "Wed" }, unit = "ms",
            series = { { name = "p95", values = { 120, 180, 140 } }, { name = "p99", values = { 200, nil, 260 } } } },
          ui.table { columns = { "Region", "p95" }, rows = { { "us-east", 182 }, { "eu-west", 140 } } },
          ui.row { ui.stat { value = "182 ms", label = "p95" }, ui.stat { value = 99.2, label = "uptime", tone = "success" } },
        }
        "#,
        1,
    )
    .unwrap();
    let tree = visual.render().unwrap();
    let chart = find(&tree, &|n| matches!(n.kind, NodeKind::Chart(_))).unwrap();
    let NodeKind::Chart(chart) = &chart.kind else {
        unreachable!()
    };
    assert_eq!(chart.series[1].values, [Some(200.0), None, Some(260.0)]);
    assert_eq!(chart.series[1].color, Color::token(Token::Chart(1)));
    let table = find(&tree, &|n| matches!(n.kind, NodeKind::Table(_))).unwrap();
    let NodeKind::Table(table) = &table.kind else {
        unreachable!()
    };
    // Numeric columns right-align.
    assert_eq!(table.columns[1].align, Align::End);
}

// show_visual's hover advice, as written there, checks clean: a line kept
// for what a pick shows, the same height before and after.
#[test]
fn the_hover_advice_example_checks_clean() {
    check(
        r#"
        local s = ui.state { picked = false }
        return function()
          return ui.column { gap = 8,
            ui.row { hover = true, on_click = function() s.picked = "us-east, 182 ms" end, ui.text "us-east" },
            ui.text { s.picked or " " },
          }
        end
        "#,
        1,
    )
    .unwrap();
}

#[test]
fn check_exercises_every_control_and_reports_where_it_broke() {
    let report = check(
        r#"
        local s = ui.state { tab = "a", on = false, n = 1 }
        return function()
          return ui.column {
            ui.tabs { options = { "a", "b" }, value = s.tab, on_change = function(v) s.tab = v end },
            ui.toggle { label = "Log scale", value = s.on, on_change = function(v) s.on = v end },
            ui.slider { label = "Window", value = s.n, min = 1, max = 10, on_change = function(v) s.n = v end },
            ui.canvas { height = 100, animate = true, draw = function(g, w, h, t) g:line(0, 0, w * t, h) end },
          }
        end
        "#,
        1,
    )
    .unwrap();
    assert_eq!(report.interactions, 5);

    let err = check(
        r#"
        local s = ui.state { tab = "a" }
        return function()
          local body = s.tab == "a" and ui.text "fine" or ui.text { nil .. "boom" }
          return ui.column {
            ui.tabs { options = { "a", "b" }, value = s.tab, on_change = function(v) s.tab = v end },
            body,
          }
        end
        "#,
        1,
    )
    .unwrap_err();
    assert!(
        err.message.starts_with("after selecting tab 'b':"),
        "{}",
        err.message
    );
    assert!(err.message.contains("visual:4"), "{}", err.message);
}

#[test]
fn ticks_are_round() {
    assert_eq!(
        nice_ticks(0.0, 100.0, 5),
        [0.0, 20.0, 40.0, 60.0, 80.0, 100.0]
    );
    assert_eq!(nice_ticks(0.0, 0.9, 4), [0.0, 0.2, 0.4, 0.6, 0.8]);
    assert_eq!(nice_ticks(5.0, 5.0, 4), [5.0]);
}

#[test]
fn shaders_bind_state_as_inputs_and_fail_with_their_line() {
    let report = check(
        r#"
        local s = ui.state { speed = 1 }
        return function()
          return ui.column {
            ui.slider { label = "Speed", value = s.speed, min = 0, max = 4, on_change = function(v) s.speed = v end },
            ui.shader { height = 240, orbit = true, inputs = { speed = s.speed }, code = [[
              void mainImage(out vec4 fragColor, in vec2 fragCoord) {
                vec2 uv = fragCoord / iResolution.xy;
                fragColor = vec4(uv, 0.5 + 0.5 * sin(iTime * speed + iCamera.x), 1.0);
              }
            ]] },
          }
        end
        "#,
        1,
    )
    .unwrap();
    assert!(report.interactions >= 1);

    let err = check(
        r#"
        return function()
          return ui.shader { height = 200, code = [[
void mainImage(out vec4 fragColor, in vec2 fragCoord) {
  fragColor = vec4(missing, 1.0);
}
          ]] }
        end
        "#,
        1,
    )
    .unwrap_err();
    // Lua drops the newline after `[[`, so mainImage is line 1.
    assert!(err.message.contains("shader line 2:"), "{}", err.message);
}

#[test]
fn shaders_default_to_light_and_keep_the_old_transparent_flag() {
    use crate::shader::Blend;
    let blend_of = |props: &str| {
        let mut visual = Visual::load(
            &format!(
                "return function() return ui.shader {{ height = 100, {props} code = [[void mainImage(out vec4 c, in vec2 p) {{ c = vec4(1.0); }}]] }} end"
            ),
            1,
        )
        .unwrap();
        match visual.render().unwrap().kind {
            NodeKind::Shader(shader) => shader.blend,
            _ => unreachable!(),
        }
    };
    assert_eq!(blend_of(""), Blend::Light);
    assert_eq!(blend_of("transparent = true,"), Blend::Alpha);
    assert_eq!(blend_of("transparent = false,"), Blend::Opaque);
    assert_eq!(blend_of("blend = \"opaque\","), Blend::Opaque);
}

fn png(width: u32, height: u32) -> Vec<u8> {
    let mut out = Vec::new();
    ::image::RgbaImage::from_fn(width, height, |x, y| {
        ::image::Rgba([(x * 30) as u8, (y * 30) as u8, 200, 255])
    })
    .write_to(
        &mut std::io::Cursor::new(&mut out),
        ::image::ImageFormat::Png,
    )
    .unwrap();
    out
}

#[test]
fn scenes_place_models_textures_environments_and_sprites_from_assets() {
    let assets = Assets::new(
        [
            (
                "tri.glb".to_owned(),
                crate::model::tests::triangle_glb().into(),
            ),
            ("wood.png".to_owned(), png(4, 4).into()),
            ("sky.png".to_owned(), png(8, 4).into()),
        ]
        .into_iter()
        .collect(),
    );
    let source = r##"
        return ui.scene { height = 300, draw = function(s)
          s:environment { asset = "sky.png", intensity = 1.5 }
          s:model { asset = "tri.glb", fit = 1, rotate = { 0, 90, 0 } }
          s:model { asset = "tri.glb", at = { 3, 0, 0 }, color = "#00ff00", metal = 1 }
          s:box { at = { 0, -1, 0 }, texture = "wood.png" }
          s:sprite { at = { 0, 2, 0 }, size = 64, color = "#ffeedd", glow = 2 }
        end }
    "##;
    let mut visual = Visual::load(source, 1).unwrap().with_assets(assets.clone());
    let tree = visual.render().unwrap();
    let NodeKind::Scene(node) = &tree.kind else {
        panic!()
    };
    let scene = visual.draw_scene(node.draw, 0.0).unwrap();
    assert_eq!(scene.face_count(), 2 + 6);
    assert_eq!(scene.textures().len(), 2);
    assert_eq!(scene.environment().map(|e| e.intensity), Some(1.5));
    assert_eq!(scene.sprites().len(), 1);
    let faces: Vec<FaceView> = scene.face_views().collect();
    // `fit` centers the model and sizes its longest side; the rotation turns
    // the triangle's plane from z = 0 onto x = 0.
    let fitted = &faces[0];
    for corner in fitted.corners {
        assert!(corner[0].abs() < 1e-5, "{corner:?}");
        assert!(corner[1].abs() <= 0.5 + 1e-5 && corner[2].abs() <= 0.5 + 1e-5);
    }
    assert_eq!(fitted.color.token, Token::Rgb(0xff0000));
    assert!(fitted.two_sided);
    assert_eq!(faces[1].color.token, Token::Rgb(0x00ff00));
    assert_eq!(faces[1].material.metalness, 1.0);
    assert_eq!(faces[1].material.roughness, 0.25);
    let boxed = &faces[2];
    assert_eq!(boxed.texture, Some(1));
    assert_eq!(boxed.uvs.map(|uv| uv.len()), Some(4));
    // The model is parsed once per visual, however often the scene draws.
    visual.draw_scene(node.draw, 1.0).unwrap();
    assert_eq!(assets.models.lock().unwrap().len(), 1);
    check_with_assets(source, 1, assets).unwrap();
}

#[test]
fn missing_and_broken_assets_fail_the_check_with_their_names() {
    let script =
        |body: &str| format!("return ui.scene {{ height = 200, draw = function(s) {body} end }}");
    let error = check(&script(r#"s:model { asset = "prism.glb" }"#), 1).unwrap_err();
    assert!(
        error.message.contains("no asset named 'prism.glb'"),
        "{}",
        error.message
    );
    let assets = Assets::new(
        [
            ("bad.png".to_owned(), b"nope".to_vec().into()),
            ("bad.glb".to_owned(), b"nope".to_vec().into()),
        ]
        .into_iter()
        .collect(),
    );
    let error = check_with_assets(
        &script(r#"s:box { at = {0,0,0}, texture = "bad.png" }"#),
        1,
        assets.clone(),
    )
    .unwrap_err();
    assert!(error.message.contains("bad.png"), "{}", error.message);
    let error =
        check_with_assets(&script(r#"s:model { asset = "bad.glb" }"#), 1, assets).unwrap_err();
    assert!(
        error.message.contains("model 'bad.glb'"),
        "{}",
        error.message
    );
}

// ---- named inputs and kept entries (plans/live-visuals G1) ---------------

const FORM: &str = r#"
    return function()
      return ui.column {
        ui.field { name = "goal", label = "Goal", placeholder = "One line", value = "ship it" },
        ui.area { name = "notes", label = "Notes" },
        ui.check { name = "agree", label = "Agree" },
        ui.checklist { name = "steps", label = "Steps", options = { "build", "test" }, value = { "build" } },
        ui.choice { name = "size", options = { "S", "M", "L" }, value = "M" },
        ui.choice { name = "tags", many = true, options = { { value = "a", label = "Alpha" }, "b" } },
        ui.select { name = "who", options = { "me", "you" } },
        ui.text { "goal is ", ui.entry("goal") or "-" },
      }
    end
"#;

fn input<'a>(tree: &'a Node, name: &str) -> &'a Input {
    inputs(tree)
        .into_iter()
        .find(|i| i.name == name)
        .unwrap_or_else(|| panic!("no input {name}"))
}

fn text(value: &str) -> EntryValue {
    EntryValue::Text(value.into())
}

#[test]
fn every_input_kind_parses_with_its_start_value() {
    let mut visual = Visual::load(FORM, 1).unwrap();
    let tree = visual.render().unwrap();
    let kinds: Vec<_> = inputs(&tree).iter().map(|i| (i.name.as_str(), i.kind)).collect();
    assert_eq!(
        kinds,
        [
            ("goal", InputKind::Field),
            ("notes", InputKind::Area),
            ("agree", InputKind::Check),
            ("steps", InputKind::Checklist),
            ("size", InputKind::Choice { many: false }),
            ("tags", InputKind::Choice { many: true }),
            ("who", InputKind::Select),
        ]
    );
    assert_eq!(input(&tree, "goal").value, text("ship it"));
    assert_eq!(input(&tree, "goal").placeholder.as_deref(), Some("One line"));
    assert_eq!(input(&tree, "agree").value, EntryValue::Bool(false));
    assert_eq!(input(&tree, "steps").value, EntryValue::List(vec!["build".into()]));
    assert_eq!(input(&tree, "tags").options[0].label, "Alpha");
    assert_eq!(input(&tree, "tags").value, EntryValue::List(vec![]));
    assert!(!input(&tree, "goal").kept);
    check(FORM, 1).unwrap();
}

#[test]
fn a_kept_entry_beats_the_scripts_value() {
    let mut visual = Visual::load(FORM, 1).unwrap();
    visual.render().unwrap();
    visual.set_entry("goal", Some(text("fix the bug")));
    visual.set_entry("steps", Some(EntryValue::List(vec!["test".into()])));
    let tree = visual.render().unwrap();
    let goal = input(&tree, "goal");
    assert_eq!(goal.value, text("fix the bug"));
    assert_eq!(goal.start, text("ship it"));
    assert!(goal.kept);
    assert_eq!(input(&tree, "steps").value, EntryValue::List(vec!["test".into()]));
    // The script reads it, read-only.
    assert!(text_of(&tree).contains("goal is fix the bug"));
    // Cleared, the script's value shows again.
    visual.set_entry("goal", None);
    assert_eq!(input(&visual.render().unwrap(), "goal").value, text("ship it"));
}

#[test]
fn an_entry_survives_a_script_swap_and_a_dropped_input_keeps_its_own() {
    let mut visual = Visual::load(FORM, 1).unwrap();
    visual.render().unwrap();
    visual.set_entry("goal", Some(text("mine")));
    visual.set_entry("notes", Some(text("typed")));
    // The agent's new script gives goal another value, drops notes, adds due.
    visual
        .swap(
            r#"return function() return ui.column {
                ui.field { name = "goal", label = "Goal", value = "the agent's" },
                ui.field { name = "due", label = "Due" },
            } end"#,
        )
        .unwrap();
    let tree = visual.render().unwrap();
    assert_eq!(input(&tree, "goal").value, text("mine"));
    assert!(!input(&tree, "goal").fresh);
    assert!(input(&tree, "due").fresh, "new since the last script");
    assert_eq!(visual.entries().get("notes"), Some(&text("typed")));
    // Touched, it isn't new any more; renders don't make it new again.
    visual.set_entry("due", Some(text("Friday")));
    assert!(!input(&visual.render().unwrap(), "due").fresh);
    // The dropped input comes back with its entry.
    visual.swap(FORM).unwrap();
    let tree = visual.render().unwrap();
    assert_eq!(input(&tree, "notes").value, text("typed"));
    assert!(!input(&tree, "notes").fresh, "known from before");
    // A broken script leaves the old one running, entries untouched.
    assert!(visual.swap("return 5").is_err());
    assert_eq!(input(&visual.render().unwrap(), "goal").value, text("mine"));
}

#[test]
fn the_focused_input_keeps_what_it_shows_across_a_swap() {
    let mut visual = Visual::load(FORM, 1).unwrap();
    visual.render().unwrap();
    // The reader is in "goal" but hasn't typed yet.
    visual.focus(Some("goal"));
    visual
        .swap(r#"return function() return ui.field { name = "goal", value = "changed" } end"#)
        .unwrap();
    let tree = visual.render().unwrap();
    assert_eq!(input(&tree, "goal").value, text("ship it"), "focused input untouched");
    assert!(!input(&tree, "goal").kept, "not an entry of the reader's");
    // Focus gone, the script's value shows.
    visual.focus(None);
    assert_eq!(input(&visual.render().unwrap(), "goal").value, text("changed"));
}

#[test]
fn entries_round_trip_as_json_for_a_restart() {
    let mut visual = Visual::load(FORM, 1).unwrap();
    visual.render().unwrap();
    visual.set_entry("agree", Some(EntryValue::Bool(true)));
    visual.set_entry("tags", Some(EntryValue::List(vec!["a".into(), "b".into()])));
    visual.set_entry("goal", Some(text("x")));
    let saved = visual.entries().to_json();
    let mut again = Visual::load(FORM, 1).unwrap().with_entries(Entries::from_json(&saved));
    let tree = again.render().unwrap();
    assert_eq!(input(&tree, "agree").value, EntryValue::Bool(true));
    assert_eq!(input(&tree, "tags").value, EntryValue::List(vec!["a".into(), "b".into()]));
    assert_eq!(input(&tree, "goal").value, text("x"));
    assert_eq!(Entries::from_json("not json"), Entries::default());
}

#[test]
fn an_entry_of_the_wrong_shape_is_not_shown() {
    let mut visual = Visual::load(FORM, 1).unwrap();
    visual.set_entry("agree", Some(text("yes")));
    assert_eq!(input(&visual.render().unwrap(), "agree").value, EntryValue::Bool(false));
}

#[test]
fn inputs_need_a_name_of_their_own() {
    let missing = check(r#"return ui.field { label = "Goal" }"#, 1).unwrap_err();
    assert!(missing.message.contains("ui.field needs name"), "{}", missing.message);
    let twice = check(
        r#"return ui.column { ui.field { name = "a" }, ui.check { name = "a" } }"#,
        1,
    )
    .unwrap_err();
    assert!(twice.message.contains("name 'a' is used twice"), "{}", twice.message);
    let no_options = check(r#"return ui.choice { name = "c" }"#, 1).unwrap_err();
    assert!(no_options.message.contains("needs options"), "{}", no_options.message);
    let unknown = check(r#"return ui.check { name = "c", options = { "x" } }"#, 1).unwrap_err();
    assert!(unknown.message.contains("has no 'options'"), "{}", unknown.message);
}

#[test]
fn an_answered_input_carries_its_answer() {
    let mut visual = Visual::load(
        r#"return ui.field { name = "db", label = "Database", answered = "Postgres (child session)" }"#,
        1,
    )
    .unwrap();
    let tree = visual.render().unwrap();
    assert_eq!(input(&tree, "db").answered.as_deref(), Some("Postgres (child session)"));
}

const ANSWER_FORM: &str = r#"return ui.column {
    ui.field { name = "goal", label = "Goal" },
    ui.area { name = "notes", label = "Notes" },
    ui.check { name = "downtime", label = "Downtime is fine" },
    ui.checklist { name = "steps", label = "Steps", options = { "Back up", { value = "verify", label = "Verify" }, "Announce" } },
    ui.choice { name = "when", label = "When", options = { "Tonight", "This weekend" }, value = "Tonight" },
    ui.choice { name = "who", label = "Who", many = true, options = { "Ana", "Ben", "Cy" } },
    ui.select { name = "region", options = { { value = "eu", label = "Europe" }, { value = "us", label = "US" } } },
    ui.field { name = "db", label = "Database", answered = "Postgres 16 (from the research chat)" },
    ui.answer {},
}"#;

#[test]
fn the_answer_text_is_the_title_then_a_line_per_input() {
    let mut visual = Visual::load(ANSWER_FORM, 1).unwrap();
    visual.set_entry("goal", Some(EntryValue::Text("Move the database".into())));
    visual.set_entry("notes", Some(EntryValue::Text("after 22:00\nnot on Friday".into())));
    visual.set_entry("downtime", Some(EntryValue::Bool(true)));
    visual.set_entry("steps", Some(EntryValue::List(vec!["Back up".into(), "verify".into()])));
    visual.set_entry("who", Some(EntryValue::List(vec!["Ana".into(), "Cy".into()])));
    visual.set_entry("region", Some(EntryValue::Text("eu".into())));
    let tree = visual.render().unwrap();
    assert!(find(&tree, &|n| matches!(n.kind, NodeKind::Answer { label: None })).is_some());
    assert_eq!(
        answer_text("Migration plan", &tree),
        [
            "### Migration plan",
            "Goal: Move the database",
            "Notes: after 22:00",
            "  not on Friday",
            "Downtime is fine: yes",
            "Steps:",
            "- [x] Back up",
            "- [x] Verify",
            "- [ ] Announce",
            "When: Tonight",
            "Who: Ana, Cy",
            "region: Europe",
            "Database: Postgres 16 (from the research chat)",
        ]
        .join("\n")
    );
}

#[test]
fn an_empty_answer_says_so_and_the_reader_overrules_a_decided_one() {
    let mut visual = Visual::load(ANSWER_FORM, 1).unwrap();
    visual.set_entry("db", Some(EntryValue::Text("MySQL".into())));
    let text = answer_text("  ", &visual.render().unwrap());
    assert!(text.starts_with("### Answers\nGoal: (no answer)\n"), "{text}");
    assert!(text.contains("Downtime is fine: no\n"), "{text}");
    assert!(text.ends_with("Database: MySQL"), "{text}");
}

#[test]
fn the_answer_text_stops_at_4000_characters() {
    let mut visual = Visual::load(ANSWER_FORM, 1).unwrap();
    visual.set_entry("notes", Some(EntryValue::Text("é".repeat(5000))));
    let text = answer_text("Long", &visual.render().unwrap());
    assert_eq!(text.chars().count(), MAX_ANSWER_CHARS);
    assert!(text.ends_with('…'));
}

#[test]
fn an_answer_button_takes_only_a_label() {
    let tree = Visual::load(r#"return ui.answer { label = "Use these" }"#, 1).unwrap().render().unwrap();
    assert!(matches!(&tree.kind, NodeKind::Answer { label: Some(l) } if l == "Use these"));
    let wrong = check(r#"return ui.answer { on_click = function() end }"#, 1).unwrap_err();
    assert!(wrong.message.contains("on_click"), "{}", wrong.message);
}
