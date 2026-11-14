//! A plugin's preview in your own window (`zend plugin preview --in-window`,
//! MCP `preview_plugin { in_window: true }`): a changed version of one of
//! your plugins runs in the window you work in instead of the version you
//! keep, while the window shows a bar, "Previewing <name>", with Revert and
//! Keep, as a template's preview does.
//!
//! The preview lives under `<data>/plugin-preview/<id>/window/`:
//!
//! - `<id>/`: the folder previewed, copied and signed with this device's
//!   key (yours, as your plugins are);
//! - `.loaded/<id>/<hash>/`: its snapshot, what the window loads
//!   (`snapshot`), as your plugins' are;
//! - `storage/<id>.json`: what it keeps meanwhile (`zend.store`), copied
//!   from yours when the preview starts;
//! - `preview.json`: what it is, `{id, source, by, at}`.
//!
//! Only one of your own plugins that's on is previewed, and only while it
//! asks nothing more than you allowed (it runs with your decision): a
//! version that would need asking again is refused. The list carries it
//! with the plugin (`PluginItem.preview`); the window loads it in place of
//! the version you keep. One that fails there is withdrawn and the kept
//! version runs again, its log says so. Revert (`stop`) takes the preview
//! away: the kept version, its grants and what it keeps were never touched.
//! Keep (`KeepPluginPreview`) is the window's alone, never an agent's: the
//! copy goes over your plugin's folder (its `.git` stays) and what the
//! preview kept over what it kept.

use std::path::{Path, PathBuf};

use super::{scan, signing, snapshot, staging, status};

/// The in-window preview's folder, in a plugin's preview folder.
const DIR: &str = "window";
/// What the preview is.
const META: &str = "preview.json";
/// What it keeps meanwhile.
const STORAGE: &str = "storage";

/// What a staged preview is (`preview.json`).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Staged {
    pub id: String,
    pub source: PathBuf,
    /// Who asked for it: an agent's chat (`Caller::record`), or none for you.
    #[serde(default)]
    pub by: Option<serde_json::Value>,
    pub at: i64,
}

/// `<data>/plugin-preview/<id>/window`.
pub fn root(data_dir: &Path, id: &str) -> PathBuf {
    staging::dir(data_dir, id).join(DIR)
}

/// The preview's copy of what it keeps.
pub fn store(data_dir: &Path, id: &str) -> PathBuf {
    root(data_dir, id).join(STORAGE).join(format!("{id}.json"))
}

/// Plugin `id`'s preview in your window, if one is staged.
pub fn staged(data_dir: &Path, id: &str) -> Option<Staged> {
    let bytes = std::fs::read(root(data_dir, id).join(META)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Every staged preview's id.
pub fn all(data_dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(data_dir.join(staging::DIR)) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|id| staged(data_dir, id).is_some())
        .collect()
}

/// The staged copy, inspected (as your plugins' folders are).
pub fn inspect(data_dir: &Path, id: &str) -> scan::Inspected {
    scan::inspect(&root(data_dir, id).join(id), true)
}

/// `source` staged as plugin `id`'s preview in your window: copied (over a
/// copy staged before), signed with this device's key, what it keeps
/// copied from yours the first time. `kept` is your plugin's entry, which
/// it must stay within: refused (and nothing staged) if it would ask more.
pub fn stage(
    data_dir: &Path,
    plugins_dir: &Path,
    source: &Path,
    kept: &scan::Entry,
    state: &status::State,
    by: Option<serde_json::Value>,
) -> Result<scan::Entry, String> {
    let id = kept.item.id.as_str();
    let root = root(data_dir, id);
    std::fs::create_dir_all(&root).map_err(|e| format!("Couldn't make the preview's folder: {e}"))?;
    let fresh = root.join(format!(".{id}.stage-{}", crate::new_id()));
    let result = (|| {
        staging::copy_tree(source, &fresh)?;
        let key = signing::load_or_create_key(data_dir)?;
        super::sign_folder(&fresh, &key)?;
        let place = root.join(id);
        let _ = std::fs::remove_dir_all(&place);
        std::fs::rename(&fresh, &place).map_err(|e| format!("Couldn't put the copy in place: {e}"))?;
        Ok::<_, String>(())
    })();
    if let Err(why) = result {
        let _ = std::fs::remove_dir_all(&fresh);
        end_if_unused(data_dir, id);
        return Err(why);
    }
    let entry = scan::entry(inspect(data_dir, id), status::Author::You, state);
    if entry.item.status != status::Status::On {
        let why = entry.item.error.clone().unwrap_or_else(|| {
            if entry.item.ask.is_empty() {
                "it would need allowing again".to_string()
            } else {
                format!("it asks for what you haven't allowed it ({})", entry.item.ask.join(", "))
            }
        });
        end(data_dir, id);
        return Err(format!("{} can't preview in your window: {why}. Preview it on its own (without in_window), or install it to be asked.", kept.item.name));
    }
    // What it keeps: yours as it is now, the first time (asked again, the
    // preview goes on with what it kept).
    let storage = root.join(STORAGE);
    std::fs::create_dir_all(&storage).map_err(|e| e.to_string())?;
    let copy = storage.join(format!("{id}.json"));
    if staged(data_dir, id).is_none() {
        let _ = std::fs::remove_file(&copy);
        let yours = plugins_dir.join(".storage").join(format!("{id}.json"));
        if yours.is_file() {
            std::fs::copy(&yours, &copy).map_err(|e| format!("Couldn't copy what it keeps: {e}"))?;
        }
    }
    let staged = Staged { id: id.to_string(), source: source.to_path_buf(), by, at: chrono::Utc::now().timestamp() };
    std::fs::write(root.join(META), serde_json::to_vec_pretty(&staged).unwrap_or_default())
        .map_err(|e| format!("Couldn't keep what the preview is: {e}"))?;
    Ok(entry)
}

/// What the list carries of plugin `id`'s preview: its snapshot (made
/// here), with what the window needs to load it, or none (none staged,
/// or it isn't on as it is now: then it's taken away, said in the log).
pub fn item(data_dir: &Path, id: &str, state: &status::State) -> Result<Option<serde_json::Value>, String> {
    let Some(staged) = staged(data_dir, id) else { return Ok(None) };
    let root = root(data_dir, id);
    let entry = scan::entry(inspect(data_dir, id), status::Author::You, state);
    if entry.item.status != status::Status::On {
        end(data_dir, id);
        return Err(entry.item.error.unwrap_or_else(|| "it would need allowing again".into()));
    }
    let i = &entry.inspected;
    let snapshot = match snapshot::ensure(&root, id, &i.dir, &i.hash, &i.manifest_bytes, &i.files) {
        Ok(path) => path,
        // It changed while it was copied: the next scan has it.
        Err(error) => {
            tracing::debug!(plugin = %id, %error, "plugins: no snapshot of the preview yet");
            return Ok(None);
        }
    };
    let item = &entry.item;
    Ok(Some(serde_json::json!({
        "snapshot": snapshot.display().to_string(),
        "hash": item.hash,
        "name": item.name,
        "version": item.version,
        "entry": item.entry,
        "ui": item.ui,
        "permissions": item.permissions,
        "allowed": item.allowed,
        "network_hosts": item.network_hosts,
        "tools": item.tools,
        "store": store(data_dir, id).display().to_string(),
        "source": staged.source,
        "by": staged.by,
        "at": staged.at,
    })))
}

/// The preview taken away (Revert): whether one was staged.
pub fn end(data_dir: &Path, id: &str) -> bool {
    let root = root(data_dir, id);
    let was = root.join(META).is_file();
    let _ = std::fs::remove_dir_all(&root);
    was
}

/// A failed first stage leaves nothing behind.
fn end_if_unused(data_dir: &Path, id: &str) {
    if staged(data_dir, id).is_none() {
        end(data_dir, id);
    }
}

/// The preview kept: its copy over your plugin's folder `kept_dir` (whose
/// `.git` stays), what it kept over what yours kept, the preview gone.
/// Only as of `hash`: what the person saw on the bar.
pub fn keep(data_dir: &Path, plugins_dir: &Path, id: &str, kept_dir: &Path, hash: &str) -> Result<(), String> {
    if staged(data_dir, id).is_none() {
        return Err(format!("There's no preview of {id} in your window"));
    }
    let inspected = inspect(data_dir, id);
    if inspected.hash != hash {
        return Err("The preview changed since you looked: look again, then keep it".into());
    }
    let fresh = plugins_dir.join(format!(".{id}.keep-{}", crate::new_id()));
    let old = plugins_dir.join(format!(".{id}.kept-{}", crate::new_id()));
    staging::copy_tree(&inspected.dir, &fresh).inspect_err(|_| {
        let _ = std::fs::remove_dir_all(&fresh);
    })?;
    let git = kept_dir.join(".git");
    if git.exists() {
        std::fs::rename(&git, fresh.join(".git")).map_err(|e| format!("Couldn't move its .git: {e}"))?;
    }
    let undo = |why: String| {
        if fresh.join(".git").exists() {
            let _ = std::fs::rename(fresh.join(".git"), &git);
        }
        let _ = std::fs::remove_dir_all(&fresh);
        why
    };
    std::fs::rename(kept_dir, &old).map_err(|e| undo(format!("Couldn't set the kept version aside: {e}")))?;
    if let Err(e) = std::fs::rename(&fresh, kept_dir) {
        let _ = std::fs::rename(&old, kept_dir);
        return Err(undo(format!("Couldn't put the preview in place: {e}")));
    }
    let _ = std::fs::remove_dir_all(&old);
    let yours = plugins_dir.join(".storage").join(format!("{id}.json"));
    let theirs = store(data_dir, id);
    if theirs.is_file() {
        let _ = std::fs::create_dir_all(plugins_dir.join(".storage"));
        std::fs::copy(&theirs, &yours).map_err(|e| format!("Couldn't keep what it kept: {e}"))?;
    } else {
        let _ = std::fs::remove_file(&yours);
    }
    end(data_dir, id);
    Ok(())
}
