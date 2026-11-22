//! What a client (a linked phone's window, `Caller::Remote`) gets of this
//! device's plugins: the list without this device's paths, and each
//! plugin's files by id and relative path (`ReadPluginFile`), so it runs the
//! window part itself. Never a path outside a plugin's checked snapshot,
//! never through a link, never a file other than what was checked.
//!
//! The engine has already checked each plugin's signature and your
//! decisions; the client checks each file against the hash listed with it
//! and trusts this engine for the rest (the link it reached it on).

use std::io::Read as _;
use std::path::{Component, Path};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;

use super::manifest::MANIFEST_FILE;
use super::signing::{FileHashes, sha256_hex};

/// Whether `caller` is a client of this engine's (a linked phone's window),
/// which runs plugins from what it's sent, never from this device's paths.
pub fn is_client(caller: &zend_rpc::Caller) -> bool {
    matches!(caller, zend_rpc::Caller::Remote)
}

/// A client's call for a plugin's service (`CallPluginService`) naming
/// `device`: refused unless that's this device (`here`) or none. A linked
/// phone reaches this device's services only, never another device's
/// through this one. Which plugins answer, and with what, is the service's
/// own check, as for this device's window: only one that's on and allowed
/// here, with what it was allowed here, whatever the call says.
pub fn service_elsewhere(device: Option<&str>, here: &str) -> Option<super::service::Refusal> {
    let device = device.map(str::trim).filter(|d| !d.is_empty() && *d != here)?;
    Some(super::service::Refusal::new(
        super::service::reason::BAD_CALL,
        format!("A linked phone calls this computer's plugin services only, not {device}'s"),
    ))
}

/// The most of one file a client reads in one answer.
pub const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;

/// Fields of a listed plugin that name this device's paths.
const PATHS: [&str; 3] = ["dir", "snapshot", "preview"];

/// `ListPlugins`' reply as a client sees it: no `dirs`, no plugin's `dir`,
/// `snapshot` or `preview` (this device's window's), no source path; and,
/// for each plugin that's on and loadable, `files`: its relative paths
/// (the manifest's too) → sha256, as `files_of(id, hash)` says.
pub fn for_client(mut list: serde_json::Value, files_of: impl Fn(&str, &str) -> Option<serde_json::Value>) -> serde_json::Value {
    if let Some(fields) = list.as_object_mut() {
        fields.remove("dirs");
    }
    for item in list["plugins"].as_array_mut().into_iter().flatten() {
        let loadable = item["snapshot"].is_string() && item["status"] == "on";
        let (id, hash) = (item["id"].as_str().unwrap_or("").to_owned(), item["hash"].as_str().unwrap_or("").to_owned());
        let Some(fields) = item.as_object_mut() else { continue };
        for key in PATHS {
            fields.remove(key);
        }
        if let Some(source) = fields.get_mut("source").and_then(|s| s.as_object_mut()) {
            source.remove("path");
        }
        if loadable && let Some(files) = files_of(&id, &hash) {
            fields.insert("files".into(), files);
        }
    }
    list
}

/// A plugin's files as `for_client` lists them: `files` and the manifest.
pub fn listed_files(files: &FileHashes, manifest_bytes: &[u8]) -> serde_json::Value {
    let mut out: serde_json::Map<String, serde_json::Value> =
        files.iter().map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone()))).collect();
    out.insert(MANIFEST_FILE.into(), serde_json::Value::String(sha256_hex(manifest_bytes)));
    serde_json::Value::Object(out)
}

/// Whether `path` is a plain relative path of normal parts (no `..`,
/// `.`, root, prefix, backslash, empty part or dot-named part).
fn plain(path: &str) -> bool {
    !path.is_empty()
        && !path.contains('\\')
        && !path.contains(':')
        && path.split('/').all(|part| !part.is_empty() && !part.starts_with('.'))
        && Path::new(path).components().all(|c| matches!(c, Component::Normal(_)))
}

/// `path` of the snapshot at `snapshot`, as base64 with its sha256, only
/// when it's one of `files` (or the manifest, `manifest_bytes`), reached
/// with no link on the way, a regular file, and still what was checked.
pub fn read_file(snapshot: &Path, files: &FileHashes, manifest_bytes: &[u8], path: &str) -> Result<serde_json::Value, String> {
    let refused = || format!("{path:?} isn't one of this plugin's files");
    if !plain(path) {
        return Err(refused());
    }
    let expected = if path == MANIFEST_FILE {
        sha256_hex(manifest_bytes)
    } else {
        files.get(path).cloned().ok_or_else(refused)?
    };
    // Each step a real folder, the last a real file: never through a link.
    let mut at = snapshot.to_path_buf();
    let meta = std::fs::symlink_metadata(&at).map_err(|_| "Its checked copy isn't there".to_string())?;
    if !meta.is_dir() {
        return Err("Its checked copy isn't there".into());
    }
    let parts: Vec<&str> = path.split('/').collect();
    for (i, part) in parts.iter().enumerate() {
        at.push(part);
        let meta = std::fs::symlink_metadata(&at).map_err(|_| refused())?;
        let last = i + 1 == parts.len();
        if meta.file_type().is_symlink() || (last && !meta.is_file()) || (!last && !meta.is_dir()) {
            return Err(refused());
        }
        if last && meta.len() > MAX_FILE_BYTES {
            return Err(format!("{path} is bigger than {} MB", MAX_FILE_BYTES / (1024 * 1024)));
        }
    }
    let mut bytes = Vec::new();
    std::fs::File::open(&at)
        .and_then(|f| f.take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes))
        .map_err(|_| format!("{path} can't be read"))?;
    let sha = sha256_hex(&bytes);
    if sha != expected {
        return Err(format!("{path} isn't what was checked"));
    }
    Ok(serde_json::json!({ "path": path, "sha256": sha, "data": B64.encode(&bytes) }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_plain_relative_paths_are_plain() {
        for ok in ["init.luau", "locales/en.json", "a/b/c.png"] {
            assert!(plain(ok), "{ok}");
        }
        for bad in ["", "/etc/passwd", "../x", "a/../b", "./a", "a//b", "a/", ".git/config", "a\\b", "C:x", "a/.hidden"] {
            assert!(!plain(bad), "{bad}");
        }
    }

    #[test]
    fn a_client_s_list_names_no_paths_and_lists_files_of_what_loads() {
        let list = serde_json::json!({
            "dirs": { "plugins": "/home/me/.zend/plugins", "templates": "/home/me/.zend/templates" },
            "plugins": [
                { "id": "a", "hash": "h", "status": "on", "dir": "/home/me/.zend/plugins/a",
                  "snapshot": "/home/me/.zend/plugins/.loaded/a/h", "source": { "kind": "file", "path": "/home/me/a.zend-plugin" },
                  "preview": { "snapshot": "/x" } },
                { "id": "b", "hash": "g", "status": "pending", "dir": "/home/me/.zend/plugins/b" },
            ],
            "key_notices": [],
        });
        let out = for_client(list, |id, hash| (id == "a" && hash == "h").then(|| serde_json::json!({ "init.luau": "00" })));
        assert!(!out.to_string().contains("/home/me"), "{out}");
        assert_eq!(out["plugins"][0]["files"], serde_json::json!({ "init.luau": "00" }));
        assert_eq!(out["plugins"][0]["source"], serde_json::json!({ "kind": "file" }));
        assert!(out["plugins"][1].get("files").is_none());
    }
}
