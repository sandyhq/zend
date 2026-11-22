//! A client's plugins (the phone, a client of the desktop's engine): the
//! engine's list names no paths of its device, only each loaded plugin's
//! files (relative path → sha256). The window part is fetched file by file
//! (`ReadPluginFile {id, hash, path}`), each checked against its listed
//! hash, into `<app data>/plugins-remote/<id>/<hash16>/`, and loaded from
//! there as the desktop loads its snapshot. The desktop's engine checked the
//! signature and your decisions; this side checks every byte it got against
//! what that engine listed, over the link it pinned.

use std::cell::RefCell;
use std::collections::HashSet;
use std::future::Future;
use std::path::{Component, Path, PathBuf};

use base64::Engine as _;
use serde_json::{Map, Value as Json};
use sha2::{Digest, Sha256};

/// Written last into a complete copy: the sha256 of its files' list.
const DONE: &str = ".fetched";

zgx::tls::thread_local! {
    /// `<id>:<hash>` being fetched now: one fetch each.
    static FETCHING: RefCell<HashSet<String>> = RefCell::default();
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// A plain relative path of normal parts (no `..`, root, backslash or
/// dot-named part): the only kind a listed file may have.
pub fn plain(path: &str) -> bool {
    !path.is_empty()
        && !path.contains('\\')
        && !path.contains(':')
        && path.split('/').all(|part| !part.is_empty() && !part.starts_with('.'))
        && Path::new(path).components().all(|c| matches!(c, Component::Normal(_)))
}

/// Where plugin `id` at `hash` is kept under `root` (`<app data>/plugins-remote`).
fn folder(root: &Path, id: &str, hash: &str) -> Option<PathBuf> {
    let id_ok = !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    let hash_ok = hash.len() >= 16 && hash.chars().all(|c| c.is_ascii_hexdigit());
    (id_ok && hash_ok).then(|| root.join(id).join(&hash[..16]))
}

fn mark(files: &Map<String, Json>) -> String {
    sha256_hex(Json::Object(files.clone()).to_string().as_bytes())
}

/// The listed plugin's files, when it's one fetched from another engine.
pub fn listed(item: &Json) -> Option<&Map<String, Json>> {
    item.get("snapshot").is_none_or(Json::is_null).then(|| item["files"].as_object()).flatten()
}

/// Its complete copy under `root`, when there's one of exactly `files`.
pub fn ready(root: &Path, id: &str, hash: &str, files: &Map<String, Json>) -> Option<PathBuf> {
    let dir = folder(root, id, hash)?;
    (std::fs::read_to_string(dir.join(DONE)).ok()? == mark(files)).then_some(dir)
}

/// Fetch plugin `id`'s `files` at `hash` with `read` (one `ReadPluginFile`
/// each) into its folder under `root`: every file checked against its
/// listed sha256 before the copy counts as complete.
pub async fn fetch<F, Fut>(root: &Path, id: &str, hash: &str, files: &Map<String, Json>, read: F) -> Result<PathBuf, String>
where
    F: Fn(Json) -> Fut,
    Fut: Future<Output = Result<Json, String>>,
{
    let dir = folder(root, id, hash).ok_or_else(|| format!("{id} isn't a plugin id this side keeps"))?;
    let part = dir.with_extension("part");
    let _ = std::fs::remove_dir_all(&part);
    std::fs::create_dir_all(&part).map_err(|e| e.to_string())?;
    for (path, sha) in files {
        let sha = sha.as_str().unwrap_or_default();
        if !plain(path) {
            return Err(format!("{id} lists {path:?}, not a file of its own"));
        }
        let got = read(serde_json::json!({ "id": id, "hash": hash, "path": path })).await?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(got["data"].as_str().unwrap_or_default())
            .map_err(|_| format!("{id}: {path} came garbled"))?;
        if sha256_hex(&bytes) != sha {
            return Err(format!("{id}: {path} isn't what its engine listed"));
        }
        let to = part.join(path);
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::write(&to, &bytes).map_err(|e| e.to_string())?;
    }
    std::fs::write(part.join(DONE), mark(files)).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::rename(&part, &dir).map_err(|e| e.to_string())?;
    // Older copies of it go (one copy each plugin).
    if let Some(parent) = dir.parent()
        && let Ok(read) = std::fs::read_dir(parent)
    {
        for old in read.flatten().map(|e| e.path()).filter(|p| *p != dir) {
            let _ = std::fs::remove_dir_all(old);
        }
    }
    Ok(dir)
}

/// Whether `id` at `hash` may start fetching now (none running for it).
pub fn start(id: &str, hash: &str) -> bool {
    FETCHING.with(|f| f.borrow_mut().insert(format!("{id}:{hash}")))
}

/// Its fetch ended.
pub fn finished(id: &str, hash: &str) {
    FETCHING.with(|f| f.borrow_mut().remove(&format!("{id}:{hash}")));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block_on<T>(future: impl Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread().build().unwrap().block_on(future)
    }

    fn file(bytes: &[u8]) -> Json {
        serde_json::json!({ "data": base64::engine::general_purpose::STANDARD.encode(bytes) })
    }

    fn files(list: &[(&str, &[u8])]) -> Map<String, Json> {
        list.iter().map(|(p, b)| (p.to_string(), Json::from(sha256_hex(b)))).collect()
    }

    const HASH: &str = "0123456789abcdef0123456789abcdef";

    #[test]
    fn a_plugin_s_files_are_fetched_checked_and_kept() {
        let root = tempfile::tempdir().unwrap();
        let list = files(&[("init.luau", b"return 1"), ("locales/en.json", b"{}")]);
        assert!(ready(root.path(), "side", HASH, &list).is_none());
        let dir = block_on(fetch(root.path(), "side", HASH, &list, |asked| async move {
            Ok(match asked["path"].as_str().unwrap() {
                "init.luau" => file(b"return 1"),
                _ => file(b"{}"),
            })
        }))
        .unwrap();
        assert_eq!(std::fs::read(dir.join("init.luau")).unwrap(), b"return 1");
        assert_eq!(std::fs::read(dir.join("locales/en.json")).unwrap(), b"{}");
        assert_eq!(ready(root.path(), "side", HASH, &list), Some(dir));
    }

    #[test]
    fn a_file_not_as_listed_or_a_path_not_its_own_is_refused() {
        let root = tempfile::tempdir().unwrap();
        let list = files(&[("init.luau", b"return 1")]);
        let changed = block_on(fetch(root.path(), "side", HASH, &list, |_| async { Ok(file(b"evil")) }));
        assert!(changed.is_err());
        assert!(ready(root.path(), "side", HASH, &list).is_none());
        for bad in ["../escape.luau", "/etc/x", ".hidden", "a/../../b"] {
            let list = files(&[(bad, b"x")]);
            let refused = block_on(fetch(root.path(), "side", HASH, &list, |_| async { Ok(file(b"x")) }));
            assert!(refused.is_err(), "{bad}");
        }
        assert!(!root.path().join("escape.luau").exists());
        for (id, hash) in [("../x", HASH), ("side", "../../.."), ("side", "short")] {
            assert!(folder(root.path(), id, hash).is_none(), "{id} {hash}");
        }
    }

    #[test]
    fn only_an_item_with_files_and_no_snapshot_is_fetched() {
        assert!(listed(&serde_json::json!({ "files": { "init.luau": "00" } })).is_some());
        assert!(listed(&serde_json::json!({ "snapshot": "/s", "files": { "init.luau": "00" } })).is_none());
        assert!(listed(&serde_json::json!({ "snapshot": "/s" })).is_none());
    }
}
