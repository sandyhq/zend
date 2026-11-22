//! What a linked phone ([`zend_rpc::Caller::Remote`], the phone link over
//! the tailnet) may read of this device's files: only its chats' own
//! folders, never a host file outside them, and never a folder that holds
//! Zend's own data (`ipc-token`, `phone-link-key`, `agent-accounts`, …)
//! or the home folder (a chat from before scratch folders runs in `~`).
//!
//! This device's own window (`Caller::Local`) keeps its wider reach: it
//! may open a host file a chat names by absolute path, read-only.

use std::path::{Path, PathBuf};

/// Whether the request being served came over the phone link. Read it in
/// the request's own task, before any `spawn` (a spawned task is `Local`).
pub(crate) fn is_phone() -> bool {
    matches!(zend_rpc::caller(), zend_rpc::Caller::Remote)
}

/// Why `folder` (canonical) is too wide to serve beyond this device's own
/// window, or `None` when it's a folder of its own: the home folder or
/// above it, a filesystem root, or any folder holding one of `data`
/// (Zend's own dirs) is refused. Fails closed: an unknown home refuses.
pub(crate) fn too_wide(folder: &Path, home: Option<&Path>, data: &[PathBuf]) -> Option<&'static str> {
    let Some(home) = home.and_then(|home| std::fs::canonicalize(home).ok()) else {
        return Some("this host's home folder is unknown");
    };
    let holds_data = data
        .iter()
        .filter_map(|dir| std::fs::canonicalize(dir).ok())
        .any(|dir| dir.starts_with(folder));
    (home.starts_with(folder) || folder.parent().is_none() || holds_data)
        .then_some("the folder is the home folder, above it, or holds Zend's data")
}

/// The refusal a phone gets for a file outside its chat's folder.
pub(crate) const OUTSIDE: &str = "That file is outside the chat's folder: a linked phone reads only its chats' folders";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_is_too_wide_when_it_is_home_above_it_or_holds_zend_s_data() {
        let temp = tempfile::tempdir().unwrap();
        let home = std::fs::canonicalize(temp.path()).unwrap().join("home");
        let data = home.join(".zend");
        let project = home.join("code/app");
        let scratch = data.join("scratch/abc");
        for dir in [&data, &project, &scratch] {
            std::fs::create_dir_all(dir).unwrap();
        }
        let data = [data.clone()];
        assert_eq!(too_wide(&project, Some(&home), &data), None);
        // A chat folder inside Zend's data (scratch, worktrees) is its own.
        assert_eq!(too_wide(&scratch, Some(&home), &data), None);
        for wide in [home.as_path(), home.parent().unwrap(), Path::new("/"), &data[0]] {
            assert!(too_wide(wide, Some(&home), &data).is_some(), "{wide:?}");
        }
        // A folder outside home that holds the data dir (another ZEND_HOME).
        let elsewhere = std::fs::canonicalize(temp.path()).unwrap().join("srv");
        std::fs::create_dir_all(elsewhere.join("zend")).unwrap();
        assert!(too_wide(&elsewhere, Some(&home), &[elsewhere.join("zend")]).is_some());
        assert!(too_wide(&project, None, &data).is_some());
    }
}
