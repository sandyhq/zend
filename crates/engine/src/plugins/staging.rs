//! A plugin's preview (`zend plugin preview <folder>`, MCP `preview_plugin`):
//! its folder run in a Zend of its own, beside yours, for an agent to try
//! its changes on.
//!
//! The preview is an engine and a window of its own, under
//! `<data>/plugin-preview/<id>/`: `engine/` its data dir (the plugin a copy
//! in `engine/plugins/<id>`, its store copied from yours as it was, signed
//! with the preview's own key), `home/` the engine's home and the window's
//! own state (`window_home`: what the plugin runs, it runs as you). The window
//! runs offscreen (`ZGX_OFFSCREEN`): never shown, never focused, no
//! popups of its own, no application ID; it lays out and takes input from
//! the agent (`PluginInput`) and is pictured alone (`PluginStatus` with
//! `preview`). Nothing of yours changes: your plugin, its grants and what
//! it keeps stay as they are, and what fails in the preview fails there.
//!
//! What the plugin was allowed here goes with it (your decision, copied);
//! one never allowed here is allowed nothing it must ask for, so a preview
//! never grants what you haven't. Asked again, the folder is copied again
//! (its window reloads it, as your own plugins reload on save); `stop`
//! ends both processes.

use std::path::{Path, PathBuf};
use std::time::Duration;

use super::{scan, signing, status};

/// The previews' folder in an engine's data dir.
pub const DIR: &str = "plugin-preview";
/// Written into a preview's data dir: it is one (and whose).
pub const MARKER: &str = "plugin-preview.json";
/// The running preview's processes and port, beside its data dir.
const RUNNING: &str = "preview.json";
/// How long its engine has to answer, and its window to load the plugin.
const ENGINE_STARTS: Duration = Duration::from_secs(30);
const WINDOW_STARTS: Duration = Duration::from_secs(45);
/// Copying a folder stops at this much (a plugin is small; this keeps a
/// stray `node_modules` from filling the disk twice).
const COPY_MAX: u64 = 64 * 1024 * 1024;

/// Whether the engine with data dir `data_dir` is a preview's.
pub fn is_preview(data_dir: &Path) -> bool {
    data_dir.join(MARKER).is_file()
}

/// A preview's folder: `<data>/plugin-preview/<id>`.
pub fn dir(data_dir: &Path, id: &str) -> PathBuf {
    data_dir.join(DIR).join(id)
}

/// What a running preview is: kept in its folder's `preview.json`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Running {
    pub id: String,
    pub source: PathBuf,
    pub port: u16,
    pub engine_pid: u32,
    #[serde(default)]
    pub app_pid: Option<u32>,
}

impl Running {
    pub fn data_dir(&self, data_dir: &Path) -> PathBuf {
        dir(data_dir, &self.id).join("engine")
    }
}

/// The preview of `id`, if one runs (its engine alive).
pub fn running(data_dir: &Path, id: &str) -> Option<Running> {
    let bytes = std::fs::read(dir(data_dir, id).join(RUNNING)).ok()?;
    let running: Running = serde_json::from_slice(&bytes).ok()?;
    alive(running.engine_pid).then_some(running)
}

/// Every running preview.
pub fn all_running(data_dir: &Path) -> Vec<Running> {
    let Ok(entries) = std::fs::read_dir(data_dir.join(DIR)) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|e| running(data_dir, &e.file_name().to_string_lossy()))
        .collect()
}

/// A client of the preview's engine.
pub async fn connect(data_dir: &Path, preview: &Running) -> Result<zend_rpc::RpcClient, String> {
    zend_rpc::connect_ipc(preview.port, Some(&preview.data_dir(data_dir)))
        .await
        .map_err(|e| format!("The preview of {} doesn't answer ({e}): start it again with `zend plugin preview`", preview.id))
}

/// The preview of `id` running, or why there's none.
pub fn need(data_dir: &Path, id: &str) -> Result<Running, String> {
    running(data_dir, id).ok_or_else(|| format!("No preview of {id} runs: start one with `zend plugin preview <folder>` (MCP `preview_plugin`)"))
}

/// The engine and window a preview runs: Zend's own beside this engine.
pub struct Binaries {
    pub engine: PathBuf,
    pub app: PathBuf,
}

impl Binaries {
    /// This engine's binary, and the window app beside it (or
    /// `ZEND_PLUGIN_PREVIEW_APP`: a development build's).
    pub fn here() -> Result<Self, String> {
        let engine = std::env::current_exe().map_err(|e| format!("This engine's binary isn't known: {e}"))?;
        let app = match std::env::var_os("ZEND_PLUGIN_PREVIEW_APP").filter(|v| !v.is_empty()) {
            Some(app) => PathBuf::from(app),
            None => {
                let name = if cfg!(windows) { "zend-zgx.exe" } else { "zend-zgx" };
                engine.parent().map(|d| d.join(name)).unwrap_or_default()
            }
        };
        if !app.is_file() {
            return Err(format!("There's no Zend window app beside this engine ({}) to preview in", app.display()));
        }
        Ok(Self { engine, app })
    }
}

/// Start the preview of the plugin folder `source`, or, running, copy the
/// folder into it again. Returns what runs.
pub async fn start(data_dir: &Path, source: &Path, binaries: Binaries) -> Result<(Running, bool), String> {
    let inspected = {
        let source = source.to_path_buf();
        tokio::task::spawn_blocking(move || scan::inspect(&source, false))
            .await
            .map_err(|_| "Couldn't read the folder".to_string())?
    };
    let Some(manifest) = inspected.manifest.as_ref() else {
        return Err(inspected.error.unwrap_or_else(|| "That folder has no zend-plugin.json".into()));
    };
    if let Some(problem) = manifest.problem() {
        return Err(problem.to_owned());
    }
    let id = manifest.id.clone();
    let root = dir(data_dir, &id);
    let engine_dir = root.join("engine");
    let plugin_dir = engine_dir.join("plugins").join(&id);
    if let Some(mut running) = running(data_dir, &id) {
        // Again: the folder copied over the preview's (its window reloads
        // it as it sees the change), from wherever it is now.
        sync(source, &plugin_dir, &engine_dir).await?;
        if running.source != source {
            running.source = source.to_path_buf();
            write_running(&root, &running)?;
        }
        return Ok((running, false));
    }
    let _ = stop(data_dir, &id);
    {
        let (data_dir, source, root, id) = (data_dir.to_path_buf(), source.to_path_buf(), root.clone(), id.clone());
        tokio::task::spawn_blocking(move || prepare(&data_dir, &source, &root, &id))
            .await
            .map_err(|_| "Couldn't set the preview up".to_string())??;
    }
    let port = free_port()?;
    let home = root.join("home");
    let engine_pid = spawn(&binaries.engine, &[], &home, &root.join("engine.log"), |command| {
        command
            .env("ZEND_DATA_DIR", &engine_dir)
            .env("ZEND_IPC_PORT", port.to_string());
    })?;
    let mut running = Running { id: id.clone(), source: source.to_path_buf(), port, engine_pid, app_pid: None };
    write_running(&root, &running)?;
    // Its engine answering, then its window.
    let client = wait_for_engine(&running, &engine_dir).await.inspect_err(|_| {
        let _ = stop(data_dir, &id);
    })?;
    let app_pid = spawn(&binaries.app, &["main"], &home, &root.join("app.log"), |command| {
        command
            // Its engine: the dev build's switches and the beta's.
            .env("ZEND_ZGX_ENGINE_PORT", port.to_string())
            .env("ZEND_ZGX_ENGINE_DATA", &engine_dir)
            .env("ZEND_ZGX_ENGINE_SPAWN", "0")
            .env("ZEND_IPC_PORT", port.to_string())
            .env("ZEND_DATA_DIR", &engine_dir)
            // Never on the screen, never focused, nothing of its own
            // outside its window, nothing for assistive clients.
            .env("ZGX_OFFSCREEN", "1")
            .env("ZGX_POPUP_WINDOWS", "0")
            .env("ZGX_ACCESSIBILITY", "0")
            .env("ZGX_CHECK", "0");
        window_home(command, &home);
    })
    .inspect_err(|_| {
        let _ = stop(data_dir, &id);
    })?;
    running.app_pid = Some(app_pid);
    write_running(&root, &running)?;
    wait_for_window(&client, &id).await;
    Ok((running, true))
}

/// The window's home. Its plugin runs as you, as in your window (its
/// `commands` see your home: a plugin reading your files reads them), and
/// only the window's own state is the preview's: on Linux and the BSDs it
/// keeps that under XDG_DATA_HOME and XDG_STATE_HOME, so those are the
/// preview's. On macOS and Windows its state is under the home itself, so
/// the window keeps the preview's home there, never sharing yours.
/// (`ZEND_PLUGIN_PREVIEW_HOME`: another home than this engine's, for a
/// test standing in for yours.)
fn window_home(command: &mut std::process::Command, home: &Path) {
    if cfg!(any(target_os = "macos", windows)) {
        return;
    }
    let Some(yours) = std::env::var_os("ZEND_PLUGIN_PREVIEW_HOME").or_else(|| std::env::var_os("HOME")) else {
        return;
    };
    command
        .env("HOME", yours)
        .env_remove("USERPROFILE")
        .env_remove("CLAUDE_CONFIG_DIR")
        .env_remove("CODEX_HOME")
        .env("XDG_DATA_HOME", home.join(".local").join("share"))
        .env("XDG_STATE_HOME", home.join(".local").join("state"));
}

/// End the preview of `id`: its window and engine. Whether one ran.
pub fn stop(data_dir: &Path, id: &str) -> bool {
    let root = dir(data_dir, id);
    let Ok(bytes) = std::fs::read(root.join(RUNNING)) else {
        return false;
    };
    let _ = std::fs::remove_file(root.join(RUNNING));
    let Ok(running) = serde_json::from_slice::<Running>(&bytes) else {
        return false;
    };
    let was = alive(running.engine_pid);
    for pid in running.app_pid.into_iter().chain([running.engine_pid]) {
        end(pid);
    }
    was
}

/// The preview's folder made anew: its data dir marked, the plugin and its
/// store copied, the copy signed with the preview's own key and allowed
/// what you allowed it.
fn prepare(data_dir: &Path, source: &Path, root: &Path, id: &str) -> Result<(), String> {
    let engine_dir = root.join("engine");
    let plugins = engine_dir.join("plugins");
    // What a preview before this one left (its log stays).
    let _ = std::fs::remove_dir_all(&plugins);
    std::fs::create_dir_all(&plugins).map_err(|e| format!("Couldn't make the preview's folder: {e}"))?;
    // Its window opens on the main window at once: set up and welcomed
    // already (the window app's markers, development's and the beta's).
    for app in ["zend-zgx", "zend"] {
        let app_dir = root.join("home").join(".local").join("share").join(app);
        std::fs::create_dir_all(&app_dir).map_err(|e| format!("Couldn't make the preview's home: {e}"))?;
        for marker in ["onboarded", "welcomed"] {
            let _ = std::fs::write(app_dir.join(marker), b"");
        }
    }
    std::fs::write(
        engine_dir.join(MARKER),
        serde_json::to_vec_pretty(&serde_json::json!({ "id": id, "source": source })).unwrap_or_default(),
    )
    .map_err(|e| format!("Couldn't mark the preview's data: {e}"))?;
    let mut budget = COPY_MAX;
    copy_dir(source, &plugins.join(id), &mut budget)?;
    // What it keeps, as yours has it now.
    let storage = plugins.join(".storage");
    std::fs::create_dir_all(&storage).map_err(|e| e.to_string())?;
    if let Ok(entries) = std::fs::read_dir(data_dir.join("plugins").join(".storage")) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.strip_prefix(id).is_some_and(|rest| rest.starts_with('.')) && entry.path().is_file() {
                std::fs::copy(entry.path(), storage.join(&name)).map_err(|e| format!("Couldn't copy what it keeps ({name}): {e}"))?;
            }
        }
    }
    // Yours here (signed with the preview's key), allowed what you allowed.
    let key = signing::load_or_create_key(&engine_dir)?;
    super::sign_folder(&plugins.join(id), &key)?;
    let yours = status::State::load(&data_dir.join("plugins"));
    let mut state = status::State::default();
    state.own.insert(id.to_string());
    if let Some(mut decision) = yours.decisions.get(id).cloned().filter(|d| d.allowed) {
        decision.off = false;
        state.decisions.insert(id.to_string(), decision);
    }
    state.save(&plugins)
}

/// The folder copied over the preview's copy again, signed again.
async fn sync(source: &Path, plugin_dir: &Path, engine_dir: &Path) -> Result<(), String> {
    let (source, plugin_dir, engine_dir) = (source.to_path_buf(), plugin_dir.to_path_buf(), engine_dir.to_path_buf());
    tokio::task::spawn_blocking(move || {
        let fresh = plugin_dir.with_file_name(format!(".{}.sync-{}", plugin_dir.file_name().unwrap_or_default().to_string_lossy(), crate::new_id()));
        let mut budget = COPY_MAX;
        copy_dir(&source, &fresh, &mut budget).inspect_err(|_| {
            let _ = std::fs::remove_dir_all(&fresh);
        })?;
        let key = signing::load_or_create_key(&engine_dir)?;
        super::sign_folder(&fresh, &key).inspect_err(|_| {
            let _ = std::fs::remove_dir_all(&fresh);
        })?;
        let _ = std::fs::remove_dir_all(&plugin_dir);
        std::fs::rename(&fresh, &plugin_dir).map_err(|e| format!("Couldn't put the copy in place: {e}"))
    })
    .await
    .map_err(|_| "Couldn't copy the folder".to_string())?
}

/// `from` copied to `to`: files and folders, not `.git`, nor what links
/// out (a link is followed only to a file).
fn copy_dir(from: &Path, to: &Path, budget: &mut u64) -> Result<(), String> {
    std::fs::create_dir_all(to).map_err(|e| format!("Couldn't make {}: {e}", to.display()))?;
    let entries = std::fs::read_dir(from).map_err(|e| format!("Couldn't read {}: {e}", from.display()))?;
    for entry in entries.flatten() {
        let name = entry.file_name();
        if name == ".git" {
            continue;
        }
        let path = entry.path();
        let meta = std::fs::metadata(&path).map_err(|e| format!("Couldn't read {}: {e}", path.display()))?;
        if meta.is_dir() {
            if entry.file_type().is_ok_and(|t| t.is_symlink()) {
                continue;
            }
            copy_dir(&path, &to.join(&name), budget)?;
        } else if meta.is_file() {
            *budget = budget
                .checked_sub(meta.len())
                .ok_or_else(|| format!("The folder is over {} MB: is something besides the plugin in it?", COPY_MAX / 1024 / 1024))?;
            std::fs::copy(&path, to.join(&name)).map_err(|e| format!("Couldn't copy {}: {e}", path.display()))?;
        }
    }
    Ok(())
}

fn write_running(root: &Path, running: &Running) -> Result<(), String> {
    std::fs::write(root.join(RUNNING), serde_json::to_vec_pretty(running).unwrap_or_default())
        .map_err(|e| format!("Couldn't keep what the preview runs: {e}"))
}

fn free_port() -> Result<u16, String> {
    std::net::TcpListener::bind(("127.0.0.1", 0))
        .and_then(|l| l.local_addr())
        .map(|a| a.port())
        .map_err(|e| format!("No free port for the preview: {e}"))
}

/// `bin` started detached (its own process group, nothing inherited but
/// what a desktop session needs), with a home of its own: what it runs
/// reads none of yours. Its output goes to `log`.
fn spawn(
    bin: &Path,
    args: &[&str],
    home: &Path,
    log: &Path,
    env: impl FnOnce(&mut std::process::Command),
) -> Result<u32, String> {
    let out = std::fs::File::create(log).map_err(|e| format!("Couldn't make {}: {e}", log.display()))?;
    let err = out.try_clone().map_err(|e| e.to_string())?;
    let mut command = std::process::Command::new(bin);
    command.args(args).env_clear();
    // What a program on this desktop needs: to find programs (a plugin's
    // `commands`), the display, the session bus, the locale.
    for key in [
        "PATH", "USER", "LOGNAME", "SHELL", "LANG", "LC_ALL", "TZ", "TMPDIR",
        "XDG_RUNTIME_DIR", "WAYLAND_DISPLAY", "DISPLAY", "XAUTHORITY", "DBUS_SESSION_BUS_ADDRESS",
        "SYSTEMROOT", "WINDIR", "LOCALAPPDATA", "APPDATA", "TEMP", "TMP",
    ] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    command
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("CLAUDE_CONFIG_DIR", home.join(".claude"))
        .env("CODEX_HOME", home.join(".codex"))
        .stdin(std::process::Stdio::null())
        .stdout(out)
        .stderr(err);
    env(&mut command);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command.spawn().map_err(|e| format!("Couldn't start {}: {e}", bin.display()))?;
    let pid = child.id();
    // Reaped whenever it ends.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(pid)
}

async fn wait_for_engine(running: &Running, engine_dir: &Path) -> Result<zend_rpc::RpcClient, String> {
    let until = tokio::time::Instant::now() + ENGINE_STARTS;
    loop {
        if let Ok(client) = zend_rpc::connect_ipc(running.port, Some(engine_dir)).await
            && client.call(zend_rpc::methods::ENGINE_INFO, serde_json::json!({})).await.is_ok()
        {
            return Ok(client);
        }
        if !alive(running.engine_pid) {
            return Err(format!("The preview's engine stopped as it started: see {}", engine_dir.parent().unwrap_or(engine_dir).join("engine.log").display()));
        }
        if tokio::time::Instant::now() > until {
            return Err("The preview's engine didn't answer in time".into());
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// Until the preview's window answers for the plugin (or that takes too
/// long: its status says how it is then).
async fn wait_for_window(client: &zend_rpc::RpcClient, id: &str) {
    let until = tokio::time::Instant::now() + WINDOW_STARTS;
    while tokio::time::Instant::now() < until {
        if let Ok(status) = client.call(zend_rpc::methods::PLUGIN_STATUS, serde_json::json!({ "id": id })).await
            && status["open"] == true
            && status.get("note").is_none()
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(400)).await;
    }
}

fn alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        let Ok(pid) = i32::try_from(pid) else { return false };
        // SAFETY: signal 0 only checks the process exists.
        let exists = unsafe { libc::kill(pid, 0) } == 0;
        // (A zombie, reaped soon, counts as gone.)
        exists && !std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|s| s.rsplit(')').next().is_some_and(|r| r.trim_start().starts_with('Z')))
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        false
    }
}

/// `pid`'s process group ended (it leads one: `spawn`).
fn end(pid: u32) {
    #[cfg(unix)]
    if let Ok(pid) = i32::try_from(pid)
        && pid > 1
    {
        // SAFETY: a plain signal to the group this preview started.
        unsafe {
            libc::kill(-pid, libc::SIGTERM);
        }
    }
    #[cfg(not(unix))]
    let _ = pid;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn yours(data: &Path, allowed: bool) -> String {
        let plugins = data.join("plugins");
        write(&plugins.join("pv").join("zend-plugin.json"), r#"{"id":"pv","name":"P","version":"1.0.0","entry":"init.luau","permissions":["pages.add"]}"#);
        write(&plugins.join("pv").join("init.luau"), "print('mine')");
        write(&plugins.join(".storage").join("pv.json"), r#"{"tab":"usage"}"#);
        write(&plugins.join(".storage").join("pvq.json"), r#"{"other":true}"#);
        let state = format!(
            r#"{{"own":["pv","other"],"decisions":{{"pv":{{"allowed":{allowed},"author_key":"k","permissions":["pages.add"],"off":true}}}}}}"#
        );
        write(&plugins.join(".state.json"), &state);
        state
    }

    #[test]
    fn a_preview_is_a_copy_allowed_what_you_allowed_and_yours_stays_as_it_was() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("zend");
        let state = yours(&data, true);
        let source = data.join("plugins").join("pv");
        let root = dir(&data, "pv");
        prepare(&data, &source, &root, "pv").unwrap();

        let engine = root.join("engine");
        assert!(is_preview(&engine));
        assert!(!is_preview(&data));
        let copy = engine.join("plugins").join("pv");
        assert_eq!(std::fs::read_to_string(copy.join("init.luau")).unwrap(), "print('mine')");
        // What it keeps came along; another plugin's whose id starts the same didn't.
        assert_eq!(std::fs::read_to_string(engine.join("plugins/.storage/pv.json")).unwrap(), r#"{"tab":"usage"}"#);
        assert!(!engine.join("plugins/.storage/pvq.json").exists());
        // Its own here, allowed and on.
        let theirs = status::State::load(&engine.join("plugins"));
        assert_eq!(theirs.own.iter().collect::<Vec<_>>(), ["pv"]);
        let decision = &theirs.decisions["pv"];
        assert!(decision.allowed && !decision.off);
        // The copy is signed with the preview's key, not yours.
        let manifest: serde_json::Value = serde_json::from_slice(&std::fs::read(copy.join("zend-plugin.json")).unwrap()).unwrap();
        assert!(manifest["signature"]["key"].is_string());
        assert!(!std::fs::read_to_string(source.join("zend-plugin.json")).unwrap().contains("signature"));
        // Yours: untouched.
        assert_eq!(std::fs::read_to_string(data.join("plugins/.state.json")).unwrap(), state);
        assert!(!data.join("plugin-signing-key").exists());
    }

    #[test]
    fn a_plugin_you_never_allowed_is_allowed_nothing_in_its_preview() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("zend");
        yours(&data, false);
        let root = dir(&data, "pv");
        prepare(&data, &data.join("plugins").join("pv"), &root, "pv").unwrap();
        let theirs = status::State::load(&root.join("engine").join("plugins"));
        assert!(theirs.decisions.get("pv").is_none());
    }
}
