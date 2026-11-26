//! Projects on a paired server (plan step 6c), in the window: New project
//! browses and adds on a server that grants projects.manage (its calls
//! sent to it), and says plainly, with the command to run there, that a
//! server doesn't grant it (nothing sent, no error toast); Edit project is
//! the same for a project of one. Mounted headless on the shell fixture
//! (ui/shell_fixture.luau: fixture.paired_servers), whose host answers as
//! the engine would and notes each call in its "calls" state.

use zgx_script::{ScriptHost, testing::TestApp};

fn app() -> TestApp {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("ui/shell_fixture.luau");
    let app = TestApp::new(|runtime| ScriptHost::new(runtime, path));
    app.ui.scene.borrow_mut().resize(1200., 900.);
    app.frames(30);
    assert_eq!(app.error(), None, "the shell failed");
    assert!(app.host.run_command("fixture.paired_servers"));
    app.frames(10);
    app
}

fn calls(app: &TestApp) -> String {
    app.host
        .states()
        .into_iter()
        .find(|(n, _)| n == "calls" || n.ends_with("::calls"))
        .map(|(_, v)| match v {
            zgx_script::Value::Str(s) => s.to_string(),
            _ => String::new(),
        })
        .unwrap_or_default()
}

fn option(label: &str) -> String {
    format!("#palette-list >> role:option \"{label}\"")
}

/// Its row chosen in New project's palette, once it's there.
fn pick(app: &mut TestApp, label: &str) {
    let target = option(label);
    assert!(app.wait_until(|app| app.find(&target).is_some()), "no row {label}:\n{}", app.tree());
    app.click(&target);
    app.frames(20);
}

fn new_project() -> TestApp {
    let app = app();
    assert!(app.host.run_command("shell.new_project"));
    app.frames(20);
    app
}

/// A server that grants projects.manage is a device like any: its places,
/// its folders and the project made on it, each call naming it (the
/// engine sends them there: targetDeviceId, createSpace by its device).
#[test]
fn new_project_browses_and_adds_on_a_server_that_grants_projects_manage() {
    let mut app = new_project();
    pick(&mut app, "ai-server");
    let said = calls(&app);
    assert!(said.contains("drives:srv"), "its places weren't asked of it: {said}");
    assert!(!app.tree().contains("doesn't grant"), "said it doesn't grant it:\n{}", app.tree());
}

/// A server that doesn't grant it (files only, or one that didn't say) says
/// so on its page, names the command, and nothing is asked of it.
#[test]
fn new_project_says_a_server_doesnt_grant_projects_manage_and_sends_nothing() {
    for (name, id) in [("locked-server", "lock"), ("quiet-server", "quiet")] {
        let mut app = new_project();
        pick(&mut app, name);
        // (The title is set in capitals.)
        let tree = app.tree().to_lowercase();
        assert!(tree.contains(&format!("{name} doesn't grant projects.manage")), "{name}:\n{tree}");
        assert!(tree.contains("zend peer grant"), "{name}:\n{tree}");
        let said = calls(&app);
        assert!(!said.contains(&format!(":{id}")), "{name}: asked of it: {said}");
        assert!(!said.contains("create:"), "{name}: {said}");
    }
}

fn rename_in_editor(app: &mut TestApp, project: &str, name: &str) {
    assert!(app.wait_until(|app| app.find(&format!("#sidebar >> \"{project}\"")).is_some()), "no {project}:\n{}", app.tree());
    let head = app.bounds(&format!("#sidebar >> \"{project}\""));
    zgx_script::driver::right_click_at(&mut app.ui, (head.x + head.width / 2., head.y + head.height / 2.));
    let item = "role:menuitem \"Edit project…\"";
    assert!(app.wait_until(|app| app.find(item).is_some()), "the project's menu:\n{}", app.tree());
    app.frames(20);
    app.click(item);
    app.frames(40);
    assert!(app.find("role:dialog \"Edit project\"").is_some(), "no editor:\n{}", app.tree());
    app.fill("role:dialog \"Edit project\" >> role:textbox", name);
    app.frames(5);
    app.click("role:dialog \"Edit project\" >> role:button \"Save\"");
    app.frames(30);
}

/// A server's project: granted, Edit project… renames it there; not
/// granted, its menu says why on Edit project… and Remove… (disabled, no
/// error toast) and nothing is sent.
#[test]
fn a_server_projects_menu_renames_there_or_says_it_isnt_granted() {
    let mut granted = app();
    rename_in_editor(&mut granted, "Served", "Renamed");
    assert!(calls(&granted).contains("rename_project:srv-p:Renamed"), "{}", calls(&granted));
    assert!(!granted.tree().contains("doesn't grant"));

    let mut locked = app();
    assert!(locked.wait_until(|app| app.find("#sidebar >> \"Locked\"").is_some()), "no Locked:\n{}", locked.tree());
    let head = locked.bounds("#sidebar >> \"Locked\"");
    zgx_script::driver::right_click_at(&mut locked.ui, (head.x + head.width / 2., head.y + head.height / 2.));
    assert!(locked.wait_until(|app| app.find("role:menuitem \"Edit project…\"").is_some()), "no menu:\n{}", locked.tree());
    let tree = locked.tree();
    for item in ["Edit project…", "Remove…"] {
        let line = tree.lines().find(|l| l.contains(&format!("menuitem \"{item}\""))).unwrap_or_else(|| panic!("no {item}:\n{tree}"));
        assert!(line.ends_with("disabled"), "{item} should be disabled: {line}");
    }
    assert!(tree.contains("locked-server doesn't grant projects.manage"), "{tree}");
    assert!(!calls(&locked).contains("rename_project") && !calls(&locked).contains("delete_project"), "sent: {}", calls(&locked));
}

/// Settings → Projects lists a server's projects with the others, and its
/// group says plainly what the server doesn't grant and the command to run.
#[test]
fn settings_projects_lists_a_servers_projects_and_says_when_it_isnt_granted() {
    let mut app = app();
    assert!(app.host.run_command("settings_stub.sign_in"));
    assert!(app.host.run_command("shell.settings"));
    app.frames(20);
    app.click("#settings-nav >> role:tab \"Projects\"");
    app.frames(30);
    let page = app.texts().join("\n");
    assert!(page.contains("Served"), "{page}");
    assert!(page.contains("Locked"), "{page}");
    let page = page.to_lowercase();
    assert_eq!(page.matches("doesn't grant projects.manage").count(), 1, "only the locked server's group says it:\n{page}");
    assert!(page.contains("locked-server doesn't grant projects.manage"), "{page}");
    assert!(page.contains("zend peer grant"), "{page}");
}
