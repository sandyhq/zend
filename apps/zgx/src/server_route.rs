//! A paired server's grants, as a page of the window meets them (plan step
//! 6d): a call for a chat or project hosted on a paired server goes there
//! (`targetDeviceId`, src/files.rs `route_to`) when the server grants the
//! call's area; when it doesn't, nothing is sent and the page gets
//! `not_granted|<area>|<server>|<this device>`, which it shows as plain
//! words and the command to run on the server (never an error toast). The
//! engine holds the same line (crates/engine peer_view `held_back`); this
//! asks first, by the areas the device row says (`peerAreas`), and reads
//! the engine's own refusal in the same words when the row was out of date.

use serde_json::Value as Json;

/// What a refusal starts with, as scripts read it (`paths.not_granted`).
pub(crate) const NOT_GRANTED: &str = "not_granted";

/// The words for a page: the area, the server, and this device as the
/// server's `zend peer grant` takes it ([`grant_arg`]).
pub(crate) fn refusal(area: &str, server: &str, device: &str) -> String {
    let clean = |s: &str| s.replace('|', "/");
    format!("{NOT_GRANTED}|{}|{}|{}", clean(area), clean(server), clean(device))
}

/// The area a call is in, as a grant names it (`files`, `terminal`, …);
/// none for a call no area opens.
pub(crate) fn area_of(method: &str) -> Option<&'static str> {
    match zend_rpc::peer_scope(method, None)? {
        zend_rpc::Scope::Never => None,
        scope => Some(scope.name()),
    }
}

/// Whether a call in `area` to `device` may be sent: `Err` the refusal when
/// `device` is a paired server (in `devices`, the engine's device rows) that
/// doesn't grant it. `here` is this device's id.
pub(crate) fn check(devices: &Json, here: Option<&str>, device: &str, area: &str) -> Result<(), String> {
    let Some(server) = devices.as_array().into_iter().flatten().find(|d| d["id"].as_str() == Some(device)) else {
        return Ok(());
    };
    if crate::sessions::server_grants(server, area) {
        return Ok(());
    }
    Err(refusal(area, server["name"].as_str().unwrap_or("The server"), &grant_arg(Some(server), devices, here)))
}

/// This device as the server's `zend peer grant` takes it (one argument):
/// the key it's paired as there (`pairedAs`, its first 16 digits, as `zend
/// peer list` shows it), else its name, quoted for the shell where it needs
/// it; `<this device>` when neither is known.
fn grant_arg(server: Option<&Json>, devices: &Json, here: Option<&str>) -> String {
    let key = server.and_then(|s| s["pairedAs"].as_str()).filter(|k| k.len() >= 16 && k.bytes().all(|b| b.is_ascii_hexdigit()));
    if let Some(key) = key {
        return key[..16].to_string();
    }
    match here_name(devices, here) {
        Some(name) => shell_word(&name),
        None => "<this device>".to_string(),
    }
}

/// `word` as one shell argument: as it is when it has nothing a shell
/// reads, else in single quotes.
fn shell_word(word: &str) -> String {
    let plain = !word.is_empty() && word.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-+:@/=,".contains(&b));
    if plain { word.to_string() } else { format!("'{}'", word.replace('\'', "'\\''")) }
}

/// The engine's own refusal (`peer_refused: <server>: <server> doesn't grant
/// <area>, …`) as [`refusal`], and the server's gate refusing a chat in a
/// project (`… may not run sessions in project <id>`) as the refusal of
/// that project's run (`sessions.run:<id>`); `None` for any other error.
pub(crate) fn from_engine(error: &str, devices: &Json, here: Option<&str>) -> Option<String> {
    let said = error.split_once("peer_refused:")?.1.trim();
    let (server, rest) = said.split_once(": ")?;
    let row = devices
        .as_array()
        .into_iter()
        .flatten()
        .find(|d| d["name"].as_str() == Some(server) && crate::sessions::is_paired_server(d));
    let arg = || grant_arg(row, devices, here);
    if let Some((_, project)) = rest.split_once("may not run sessions in project ") {
        let project = project.trim();
        return (!project.is_empty() && !project.contains(' ')).then(|| refusal(&format!("sessions.run:{project}"), server, &arg()));
    }
    let area = rest.strip_prefix(server)?.strip_prefix(" doesn't grant ")?;
    let area = area.split([',', '(']).next()?.trim();
    (!area.is_empty() && !area.contains(' ')).then(|| refusal(area, server, &arg()))
}

/// [`check`] against the devices the window knows now.
pub(crate) fn gate(device: Option<&str>, area: &str) -> Result<(), String> {
    let Some(device) = device.filter(|d| !d.is_empty()) else {
        return Ok(());
    };
    crate::sessions::with_devices(|devices, here| check(devices, here, device, area))
}

/// An engine error as a page reads it: a refusal for grants in the words of
/// [`refusal`], else as it is.
pub(crate) fn said(error: String) -> String {
    crate::sessions::with_devices(|devices, here| from_engine(&error, devices, here)).unwrap_or(error)
}

/// An engine error as an outcome word (`offline`, `failed`, …) for a call
/// whose page has no words of its own, but a refusal for grants kept whole
/// (`not_granted|…`) for the page to say.
pub(crate) fn failure(error: String) -> String {
    let error = said(error);
    if error.starts_with(NOT_GRANTED) { error } else { crate::engine::outcome(&error).to_string() }
}

fn here_name(devices: &Json, here: Option<&str>) -> Option<String> {
    here.and_then(|id| devices.as_array()?.iter().find(|d| d["id"].as_str() == Some(id)))
        .and_then(|d| d["name"].as_str())
        .filter(|name| !name.is_empty())
        .map(str::to_string)
}

/// The device a call goes to (`targetDeviceId`): none for this one, and an
/// `Err` (the refusal) when it's a paired server that doesn't grant `area`.
pub(crate) async fn route(device: Option<String>, area: &str) -> Result<Option<String>, String> {
    gate(device.as_deref(), area)?;
    Ok(crate::files::route_to(device).await)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn devices() -> Json {
        json!([
            { "id": "lap", "name": "Laptop" },
            { "id": "ai", "name": "AI", "capabilities": ["paired-server"], "peerAreas": ["sessions.read", "files"] },
            { "id": "full", "name": "Big", "capabilities": ["paired-server"], "peerAreas": ["full"] },
            { "id": "old", "name": "Old", "capabilities": ["paired-server"] },
            { "id": "other", "name": "Phone", "capabilities": ["agent-updates"] },
        ])
    }

    /// A call to a paired server goes when the server grants its area and
    /// is held, in words, when it doesn't; this device and any other
    /// device (not a server) are never held. A server that didn't say what
    /// it grants holds only the sessions areas.
    #[test]
    fn a_call_to_a_server_goes_only_where_it_grants_the_area() {
        let d = devices();
        assert_eq!(check(&d, Some("lap"), "ai", "files"), Ok(()));
        assert_eq!(check(&d, Some("lap"), "ai", "terminal"), Err("not_granted|terminal|AI|Laptop".to_string()));
        assert_eq!(check(&d, Some("lap"), "full", "terminal"), Ok(()));
        assert_eq!(check(&d, Some("lap"), "old", "files"), Err("not_granted|files|Old|Laptop".to_string()));
        assert_eq!(check(&d, Some("lap"), "lap", "terminal"), Ok(()));
        assert_eq!(check(&d, Some("lap"), "other", "terminal"), Ok(()));
        assert_eq!(check(&d, Some("lap"), "unknown", "terminal"), Ok(()));
    }

    /// Each call of the pages is in the area the engine gates it by.
    #[test]
    fn the_calls_of_the_pages_are_in_their_areas() {
        for method in ["ListWorkspaceDirectory", "ReadWorkspaceFile", "WriteWorkspaceFile", "DeleteWorkspaceEntry", "WatchWorkspaceFiles", "WatchWorkspaceGitStatus", "GetCheckoutDiff", "DiscardWorkingTree"] {
            assert_eq!(area_of(method), Some("files"), "{method}");
        }
        for method in ["OpenTerminal", "SubscribeTerminal", "WriteTerminal", "ResizeTerminal", "CloseTerminal", "TerminalStatus", "RunProjectAction", "UpsertProjectAction", "StopBackgroundTask", "WatchPreviews"] {
            assert_eq!(area_of(method), Some("terminal"), "{method}");
        }
        assert_eq!(area_of("ListProjectActions"), Some("projects.manage"));
        assert_eq!(area_of("NoSuchCall"), None);
    }

    /// The engine's own refusal, when the row was out of date, reads the
    /// same as ours; another error is left as it is.
    #[test]
    fn the_engines_refusal_reads_like_ours() {
        let said = "peer_refused: AI: AI doesn't grant terminal, so OpenTerminal isn't sent";
        let (d, lap) = (devices(), Some("lap"));
        assert_eq!(from_engine(said, &d, lap), Some("not_granted|terminal|AI|Laptop".to_string()));
        let silent = "peer_refused: Old: Old doesn't grant files (it doesn't say what it grants), so ReadWorkspaceFile isn't sent";
        assert_eq!(from_engine(silent, &d, lap), Some("not_granted|files|Old|Laptop".to_string()));
        assert_eq!(from_engine("peer_refused: AI: AI is unreachable: closed", &d, lap), None);
        assert_eq!(from_engine("offline", &d, lap), None);
    }

    /// A chat a server won't run in a project (its gate: "your server may
    /// not run sessions in project <id>") reads as the grant to give, that
    /// project's run (step 6g).
    #[test]
    fn a_refused_run_reads_as_the_projects_grant() {
        let said = "peer_refused: AI: createChat: your server may not run sessions in project 51faf7a2-aa";
        assert_eq!(from_engine(said, &devices(), Some("lap")), Some("not_granted|sessions.run:51faf7a2-aa|AI|Laptop".to_string()));
    }

    /// The command names this device as the server knows it: by the key
    /// it's paired as there (`pairedAs`, its first 16 digits, as `zend peer
    /// list` shows it), else by its name quoted for the shell (6g review).
    #[test]
    fn the_grant_names_this_device_by_its_key_or_its_name_quoted() {
        let key = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let d = json!([
            { "id": "lap", "name": "MacBook Pro" },
            { "id": "ai", "name": "AI", "capabilities": ["paired-server"], "peerAreas": ["files"], "pairedAs": key },
            { "id": "old", "name": "Old", "capabilities": ["paired-server"] },
        ]);
        assert_eq!(check(&d, Some("lap"), "ai", "terminal"), Err("not_granted|terminal|AI|0123456789abcdef".to_string()));
        assert_eq!(check(&d, Some("lap"), "old", "files"), Err("not_granted|files|Old|'MacBook Pro'".to_string()));
        // The engine's refusal, read the same way.
        let said = "peer_refused: AI: createChat: your server may not run sessions in project p1";
        assert_eq!(from_engine(said, &d, Some("lap")), Some("not_granted|sessions.run:p1|AI|0123456789abcdef".to_string()));
        let said = "peer_refused: Old: Old doesn't grant files, so ReadWorkspaceFile isn't sent";
        assert_eq!(from_engine(said, &d, Some("lap")), Some("not_granted|files|Old|'MacBook Pro'".to_string()));
        assert_eq!(check(&d, None, "old", "files"), Err("not_granted|files|Old|<this device>".to_string()));
        let d = json!([{ "id": "lap", "name": "Ana's Mac" }, { "id": "old", "name": "Old", "capabilities": ["paired-server"] }]);
        assert_eq!(check(&d, Some("lap"), "old", "files"), Err("not_granted|files|Old|'Ana'\\''s Mac'".to_string()));
    }

    /// A call that has only outcome words keeps a grant's refusal whole.
    #[test]
    fn a_failure_keeps_the_refusal_and_names_any_other_by_outcome() {
        let refused = "peer_refused: AI: AI doesn't grant terminal, so StopBackgroundTask isn't sent";
        assert_eq!(failure(refused.to_string()), "not_granted|terminal|AI|<this device>");
        assert_eq!(failure("offline".to_string()), "offline");
        assert_eq!(failure("something broke".to_string()), "failed");
    }

    /// This device's name, for the command to run on the server.
    #[test]
    fn this_devices_name_is_the_one_the_row_says() {
        assert_eq!(here_name(&devices(), Some("lap")).as_deref(), Some("Laptop"));
        assert_eq!(here_name(&devices(), Some("nope")), None);
        assert_eq!(here_name(&devices(), None), None);
    }

    /// A refusal's words survive a name with the separator in it.
    #[test]
    fn a_refusal_keeps_to_its_four_parts() {
        let r = refusal("files", "A|B", "C");
        assert_eq!(r.split('|').count(), 4, "{r}");
    }

    /// The files pane of a session on a paired server that doesn't grant
    /// files says so in words and the command to run on the server, and
    /// lists nothing (ui/files_fixture.luau, its host stubbed to refuse as
    /// the real one does).
    #[test]
    fn the_files_pane_says_a_server_doesnt_grant_files() {
        use zgx_script::{ScriptHost, testing::TestApp};
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("ui/files_fixture.luau");
        let mut app = TestApp::new(|runtime| ScriptHost::new(runtime, path));
        app.ui.scene.borrow_mut().resize(900., 600.);
        app.frames(30);
        assert!(app.wait_until(|app| app.find("role:treeitem \"src\"").is_some()), "the tree never listed src");
        let session = zgx_script::Value::from_json(r#"{"id":"chat-9","cwd":"/srv","device":"server-no-files"}"#).unwrap();
        app.host.set_state("test_files_session", session).unwrap();
        assert!(
            app.wait_until(|app| app.find("\"AI doesn't grant files\"").is_some()),
            "the pane never said the server doesn't grant files"
        );
        assert!(
            app.find("\"AI hasn't opened files to this device. On AI, run: zend peer grant Laptop files\"").is_some(),
            "the pane never gave the command"
        );
        assert!(app.find("role:treeitem \"src\"").is_none(), "the old tree stayed");
    }

    /// Stop on a background process of a chat on a server that doesn't
    /// grant terminal says why, in words with the command, in place of
    /// "Couldn't stop that process" (ui/chat_fixture.luau).
    #[test]
    fn a_refused_stop_says_the_grant_it_needs() {
        use zgx_script::{ScriptHost, testing::TestApp};
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("ui/chat_fixture.luau");
        let mut app = TestApp::new(|runtime| ScriptHost::new(runtime, path));
        app.ui.scene.borrow_mut().resize(900., 700.);
        app.frames(30);
        for command in ["go_s2", "task_running"] {
            assert!(app.host.run_command(command), "{command}");
            app.frames(30);
        }
        app.click("role:button \"Open Running\"");
        app.frames(30);
        assert!(app.host.run_command("stop_refused"));
        app.click("role:button \"Stop npm run dev\"");
        app.frames(10);
        app.click("role:tab \"Open Alerts\"");
        app.frames(30);
        assert!(
            app.find("~\"AI hasn't opened terminal to this device. On AI, run: zend peer grant Laptop terminal\"").is_some(),
            "the stop never said which grant it needs\n{}",
            app.tree()
        );
        assert!(app.find("~\"Couldn't stop that process\"").is_none());
    }

    /// Saving an action of a project on a server that doesn't grant
    /// terminal says which grant it needs, in the editor's problem line, not
    /// the refusal's four parts (ui/panel_home_fixture.luau).
    #[test]
    fn a_refused_action_save_says_the_grant_it_needs() {
        use zgx_script::{ScriptHost, testing::TestApp};
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("ui/panel_home_fixture.luau");
        let mut app = TestApp::new(|runtime| ScriptHost::new(runtime, path));
        app.ui.scene.borrow_mut().resize(1000., 900.);
        app.frames(30);
        assert!(app.wait_until(|app| app.find("role:button \"Add action\"").is_some()), "the home never showed Add action");
        assert!(app.host.run_command("refuse_edits"));
        app.click("role:button \"Add action\"");
        assert!(app.wait_until(|app| app.find("role:dialog >> \"Add action\"").is_some()), "the editor never opened");
        app.frames(30);
        app.type_text("role:textbox \"Name\"", "Lint");
        app.type_text("role:textbox \"Command\"", "cargo clippy");
        app.click("role:button \"Save action\"");
        assert!(
            app.wait_until(|app| app.find("#panel-home-editor-problem >> \"AI hasn't opened terminal to this device. On AI, run: zend peer grant Laptop terminal\"").is_some()),
            "the editor never said which grant it needs\n{}",
            app.tree()
        );
        assert!(app.find("~\"not_granted|\"").is_none(), "the refusal's parts showed raw");
    }

    /// The terminal of a session on a paired server that doesn't grant
    /// terminal says so in words and the command to run on the server, in
    /// place of a terminal (ui/panel_fixture.luau, its host stubbed to
    /// refuse as the real one does).
    #[test]
    fn the_terminal_says_a_server_doesnt_grant_terminal() {
        use zgx_script::{ScriptHost, testing::TestApp};
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("ui/panel_fixture.luau");
        let mut app = TestApp::new(|runtime| ScriptHost::new(runtime, path));
        app.ui.scene.borrow_mut().resize(1000., 700.);
        app.frames(30);
        let session = zgx_script::Value::from_json(r#"{"id":"chat-9","cwd":"/srv","device":"server-no-terminal"}"#).unwrap();
        app.host.set_state("test_panel_session", session).unwrap();
        app.frames(10);
        assert!(app.host.run_command("test.show_terminal"));
        assert!(
            app.wait_until(|app| app.find("\"AI doesn't grant terminal\"").is_some()),
            "the terminal never said the server doesn't grant terminal"
        );
        assert!(
            app.find("\"AI hasn't opened terminal to this device. On AI, run: zend peer grant Laptop terminal\"").is_some(),
            "the terminal never gave the command"
        );
    }
}
