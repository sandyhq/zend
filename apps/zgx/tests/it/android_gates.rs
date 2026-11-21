//! Android's gates (ui/ds/platform.luau): what the device lacks has no
//! command, tab or picker anywhere the window makes one, Zend v2 too
//! (android-zgx-app G5, fix round 2). Each mounted twice, the host's
//! `platform` "android" and the desktop's, so the desktop keeps them.

use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    rc::Rc,
};

use zgx_script::{ScriptHost, Value, testing::TestApp};

/// The interface under test (`ZEND_ZGX_TEST_UI`: another one, e.g. an
/// older commit's, to see these fail there).
fn ui() -> PathBuf {
    std::env::var_os("ZEND_ZGX_TEST_UI")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("ui"))
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("zgx-android-gates-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// `fixture` mounted headless, on Android or not.
fn mount_at(fixture: &Path, android: bool, name: &str) -> TestApp {
    let (fixture, storage) = (fixture.to_path_buf(), scratch(&format!("{name}-{android}")));
    let app = TestApp::new(move |runtime| {
        let host = ScriptHost::new(runtime, fixture).storage(storage);
        if android {
            host.shared("platform", "android");
        }
        host
    });
    app.ui.scene.borrow_mut().resize(1400., 900.);
    app.frames(30);
    assert!(app.error().is_none(), "{:?}", app.error());
    app
}

fn mount(fixture: &str, android: bool) -> TestApp {
    mount_at(&ui().join(fixture), android, fixture)
}

/// What Android lacks, by the command that needs it.
const NEEDS: &[&str] = &[
    "shell.toggle_terminal",
    "shell.check_updates",
    "shell.dictate",
    "panel.new_browser_tab",
    "panel.focus_address",
    "panel.reload_page",
];

fn command_ids(app: &TestApp) -> Vec<String> {
    app.host.commands().get().into_iter().map(|c| c.id).collect()
}

fn state(app: &TestApp, name: &str) -> String {
    app.host.states().into_iter().find(|(n, _)| n == name || n.ends_with(&format!("::{name}"))).map(|(_, v)| format!("{v:?}")).unwrap_or_default()
}

fn has(app: &mut TestApp, target: &str) -> bool {
    app.ui.prepare_frame();
    app.find(target).is_some()
}

/// The whole interface, a node a line (`tree` stops at 200).
fn full_tree(app: &mut TestApp) -> String {
    app.ui.prepare_frame();
    let opts = zgx_script::driver::TreeOptions { depth: None, budget: usize::MAX, all: false };
    let (lines, hidden) = zgx_script::driver::tree_lines(&app.ui, &app.handle, None, &opts);
    zgx_script::driver::tree_lines_text(&lines, hidden)
}

/// Settings' page `tab` (its name in the nav), signed in.
fn settings(app: &mut TestApp, tab: &str) {
    app.host.run_command("settings_stub.sign_in");
    app.host.run_command("shell.settings");
    app.frames(20);
    app.click(&format!("#settings-nav >> role:tab \"{tab}\""));
    app.frames(40);
}

// MUST 1: one gate (shell/keys.luau's `needs`) under every command.
#[test]
fn android_has_no_terminal_updater_dictation_or_browser_commands() {
    let desktop = command_ids(&mount("shell_fixture.luau", false));
    for id in ["shell.toggle_terminal", "shell.check_updates", "shell.dictate"] {
        assert!(desktop.iter().any(|c| c == id), "the desktop lost {id}: {desktop:?}");
    }
    let android = command_ids(&mount("shell_fixture.luau", true));
    for id in NEEDS {
        assert!(!android.iter().any(|c| c == id), "{id} is a command on Android");
    }
    assert!(android.iter().any(|c| c == "shell.toggle_sidebar"), "the rest stay: {android:?}");
}

// MUST 2: a forwarded port's Open goes to the device's browser.
#[test]
fn a_port_s_open_opens_it_outside_on_android() {
    let mut app = mount("panel_home_fixture.luau", true);
    app.host
        .set_state(
            "previews",
            Value::from_json(r#"{"chat":"chat-1","ready":true,"services":[{"id":"svc-1","name":"web","port":3000,"url":"http://web.proj.localhost:7400","project":"comet-native","device":"work-laptop","cwd":"/proj","remote":false,"started":1}]}"#).unwrap(),
        )
        .unwrap();
    app.frames(20);
    assert!(app.wait_until(|app| app.find("role:button \"Open web preview\"").is_some()), "no port row");
    app.click("role:button \"Open web preview\"");
    app.frames(20);
    let calls = state(&app, "calls");
    assert!(calls.contains("external:http://web.proj.localhost:7400"), "not opened outside: {calls}");
    let tabs = state(&app, "panel_sessions");
    assert!(!tabs.contains("browser"), "a browser tab on Android: {tabs}");
}

// MUST 3: no picker control where there's no picker.
#[test]
fn android_shows_no_wallpaper_photo_or_theme_picker() {
    for android in [false, true] {
        let mut app = mount("shell_fixture.luau", android);
        app.host.set_state("shell_wallpaper", Value::from("image")).unwrap();
        settings(&mut app, "Appearance");
        assert_eq!(has(&mut app, "role:button \"Choose Image\""), !android, "wallpaper's Choose, android={android}:\n{}", full_tree(&mut app));
        settings(&mut app, "Account");
        assert_eq!(has(&mut app, "#settings-page >> role:button \"Change\""), !android, "profile's photo, android={android}:\n{}", full_tree(&mut app));
    }
    for android in [false, true] {
        let mut app = mount("themes_fixture.luau", android);
        app.host.run_command("themes");
        app.frames(30);
        assert_eq!(has(&mut app, "role:button \"Import theme\""), !android, "themes' Import, android={android}:\n{}", full_tree(&mut app));
    }
}

/// `from` copied into `to`, over what's there.
fn copy_over(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap().flatten() {
        let (path, dest) = (entry.path(), to.join(entry.file_name()));
        if path.is_dir() {
            copy_over(&path, &dest);
        } else {
            std::fs::copy(&path, &dest).unwrap();
        }
    }
}

// MUST 4: Zend v2 (a template shipped as an official plugin, ui/templates/
// zend-v2) over the same gates: its own shell and panel, the shared
// composer, panel state and keys.
#[test]
fn zend_v2_on_android_has_no_terminal_browser_tab_or_attach() {
    let root = scratch("v2");
    let ui_v2 = root.join("ui");
    copy_over(&ui(), &ui_v2);
    copy_over(&ui().join("templates/zend-v2"), &ui_v2);
    for android in [false, true] {
        let opened: Rc<RefCell<Vec<String>>> = Rc::default();
        let (fixture, storage, seen) = (ui_v2.join("shell_fixture.luau"), scratch(&format!("v2-{android}")), opened.clone());
        let mut app = TestApp::new(move |runtime| {
            let host = ScriptHost::new(runtime, fixture).storage(storage);
            if android {
                host.shared("platform", "android");
            }
            host.action("open_url", move |url| seen.borrow_mut().push(format!("{url:?}")));
            host
        });
        app.ui.scene.borrow_mut().resize(1400., 900.);
        app.frames(30);
        assert!(app.error().is_none(), "{:?}", app.error());
        // (It is v2's: its shell's page fills the window, no card.)
        assert!(app.bounds("shell-page").y < 1., "not v2's shell");
        let ids = command_ids(&app);
        for id in NEEDS {
            if android {
                assert!(!ids.iter().any(|c| c == id), "v2: {id} is a command on Android");
            }
        }
        assert!(ids.iter().any(|c| c == "shell.toggle_terminal") != android, "v2's terminal command, android={android}");
        app.click("#shell-sidebar >> role:button \"Fix the build\"");
        app.frames(30);
        assert_eq!(has(&mut app, "role:button \"Attach\""), !android, "v2's +, android={android}:\n{}", full_tree(&mut app));
        assert!(app.host.run_command("fixture.open_terminal") && app.host.run_command("fixture.open_url"));
        app.frames(30);
        let tabs = state(&app, "panel_sessions");
        assert_eq!(tabs.contains("terminal"), !android, "v2's terminal tab, android={android}: {tabs}");
        assert_eq!(tabs.contains("browser"), !android, "v2's browser tab, android={android}: {tabs}");
        if android {
            let opened = opened.borrow();
            assert!(opened.iter().any(|u| u.contains("https://example.com/")), "v2's link not opened outside: {opened:?}");
        }
    }
}
