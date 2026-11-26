//! What another Zend may read of this device's files: a linked phone
//! ([`zend_rpc::Caller::Remote`], the phone link over the tailnet) and
//! another device through the device-room relay ([`zend_rpc::Caller::Peer`])
//! get one scope, resolved here, never from what they claim: only this
//! device's chats' and projects' own folders ([`Scope`]), never a host file
//! outside them, and never a folder that holds Zend's own data
//! (`ipc-token`, `phone-link-key`, `agent-accounts`, …) or the home
//! folder (a chat from before scratch folders runs in `~`).
//!
//! This device's own window (`Caller::Local`) keeps its wider reach: it
//! may open a host file a chat names by absolute path, read-only. An agent
//! here (`Caller::Agent`) runs on this device and isn't fenced either.

use std::path::{Path, PathBuf};

/// Whether the request being served came from another Zend (a linked phone
/// or another device) and is held to [`Scope`]. Read it in the request's
/// own task, before any `spawn` (a spawned task is `Local`).
///
/// A paired server granted `files` (or `full`) isn't: the owner gave it
/// the machine's files (plan step 6).
pub(crate) fn fenced() -> bool {
    match zend_rpc::caller() {
        zend_rpc::Caller::Remote | zend_rpc::Caller::Peer { .. } => true,
        zend_rpc::Caller::PeerServer { grants, .. } => !grants.covers(zend_rpc::Scope::Files),
        zend_rpc::Caller::Local | zend_rpc::Caller::Agent { .. } => false,
    }
}

/// The folders another Zend may name a path in: each of this device's
/// chats' working folders and projects' folders, canonical (symlinks
/// followed), less any that is [`too_wide`].
pub(crate) struct Scope {
    roots: Vec<PathBuf>,
    home: Option<PathBuf>,
    data: Vec<PathBuf>,
}

impl Scope {
    /// This device's chats' `cwd`s and projects' paths, as the workspace
    /// holds them. A scratch chat's folder is under `scratch`.
    pub(crate) fn of(
        chat_cwds: &[String],
        space_paths: &[String],
        scratch: &Path,
        home: Option<PathBuf>,
        data: Vec<PathBuf>,
    ) -> Self {
        // As `resolve_chat_cwd` names them, without making a scratch folder.
        let chat_folders = chat_cwds.iter().filter_map(|cwd| {
            if zend_proto::projectless::is_scratch_cwd(cwd) {
                let name = cwd.trim().rsplit('/').next().unwrap_or_default();
                (!name.is_empty()).then(|| scratch.join(name))
            } else {
                crate::repos::expand_home(cwd).ok().map(PathBuf::from)
            }
        });
        let space_folders = space_paths.iter().map(PathBuf::from);
        let mut roots: Vec<PathBuf> = chat_folders
            .chain(space_folders)
            .filter(|folder| folder.is_absolute())
            .filter_map(|folder| std::fs::canonicalize(folder).ok())
            .filter(|folder| too_wide(folder, home.as_deref(), &data).is_none())
            .collect();
        roots.sort();
        roots.dedup();
        Self { roots, home, data }
    }

    /// `path` canonical, when it lies in one of the folders; else why not.
    pub(crate) fn path(&self, path: &Path) -> Result<PathBuf, String> {
        let canonical = std::fs::canonicalize(path).map_err(|_| OUTSIDE_SCOPE.to_string())?;
        if self.roots.iter().any(|root| canonical.starts_with(root)) {
            Ok(canonical)
        } else {
            Err(OUTSIDE_SCOPE.into())
        }
    }

    /// Whether a checkout git found for a path in scope may be shown: a
    /// worktree's own root, or the repository a chat's subfolder is in, but
    /// never one that is the home folder, above it, or holds Zend's data
    /// (home kept as a dotfiles checkout).
    pub(crate) fn checkout(&self, root: &Path) -> Result<(), String> {
        let canonical = std::fs::canonicalize(root).map_err(|_| OUTSIDE_SCOPE.to_string())?;
        match too_wide(&canonical, self.home.as_deref(), &self.data) {
            Some(why) => Err(format!("{why}: {OUTSIDE_SCOPE}")),
            None => Ok(()),
        }
    }
}

/// The refusal for a path outside every chat's and project's folder.
pub(crate) const OUTSIDE_SCOPE: &str =
    "That folder is outside this device's chats and projects: another Zend reads only those";

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

/// The refusal a paired server gets for a path in Zend's own folders.
pub(crate) const DATA_FENCED: &str =
    "That path is in Zend's own data or settings: a paired server never opens it, whatever it's granted";

/// Zend's own folders, which a paired server never opens by path,
/// whatever it's granted (`files`, `full`): its data (the peers and
/// their grants, keys, logins) and its settings. Its chats' working
/// folders in them (`worktrees`, `scratch`) aren't fenced.
pub(crate) struct DataFence {
    protected: Vec<PathBuf>,
    exempt: Vec<PathBuf>,
}

impl DataFence {
    /// The fence for the request being served: `None` unless it's a
    /// paired server's (read it in the request's own task).
    pub(crate) fn for_caller(data_dir: &Path) -> Option<DataFence> {
        if !matches!(zend_rpc::caller(), zend_rpc::Caller::PeerServer { .. }) {
            return None;
        }
        let home = crate::repos::session_home_dir().ok();
        let mut dirs = vec![data_dir.to_path_buf()];
        if let Some(home) = &home {
            dirs.push(home.join(".zend"));
            dirs.push(home.join(".config").join("zend"));
        }
        if let Some(config) = std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
            dirs.push(PathBuf::from(config).join("zend"));
        }
        let protected: Vec<PathBuf> = dirs.iter().map(|dir| canonical_lenient(dir)).collect();
        let exempt = protected.iter().flat_map(|dir| [dir.join("worktrees"), dir.join("scratch")]).map(|dir| canonical_lenient(&dir)).collect();
        Some(DataFence { protected, exempt })
    }

    /// Whether `path` (as named; links followed where it exists) is in
    /// one of Zend's own folders.
    pub(crate) fn refuses(&self, path: &Path) -> bool {
        let path = canonical_lenient(path);
        self.protected.iter().any(|dir| path.starts_with(dir)) && !self.exempt.iter().any(|dir| path.starts_with(dir))
    }

    /// [`DataFence::refuses`], or `root` holds one of them (a search or
    /// a watch of it would reach in).
    pub(crate) fn refuses_tree(&self, root: &Path) -> bool {
        let canonical = canonical_lenient(root);
        self.refuses(root)
            || (!self.exempt.iter().any(|dir| canonical.starts_with(dir)) && self.protected.iter().any(|dir| dir.starts_with(&canonical)))
    }
}

/// `path` canonical as far as it exists, the rest as named.
fn canonical_lenient(path: &Path) -> PathBuf {
    let mut rest = Vec::new();
    let mut at = path.to_path_buf();
    loop {
        if let Ok(found) = std::fs::canonicalize(&at) {
            return rest.into_iter().rev().fold(found, |path, part: std::ffi::OsString| path.join(part));
        }
        match (at.file_name().map(|n| n.to_os_string()), at.parent()) {
            (Some(name), Some(parent)) => {
                rest.push(name);
                at = parent.to_path_buf();
            }
            _ => return path.to_path_buf(),
        }
    }
}

/// The refusal another Zend gets for a file outside its chat's folder.
pub(crate) const OUTSIDE: &str =
    "That file is outside the chat's folder: another Zend reads only its chats' folders";

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

    #[test]
    fn a_scope_holds_this_device_s_chat_and_project_folders_symlinks_followed() {
        let temp = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(temp.path()).unwrap();
        let home = root.join("home");
        let data = home.join(".zend");
        let (project, chat, other, outside) =
            (root.join("project"), root.join("chat"), root.join("other"), root.join("dotfiles"));
        for dir in [&data, &project, &chat, &other, &outside, &data.join("scratch/s1")] {
            std::fs::create_dir_all(dir).unwrap();
        }
        std::os::unix::fs::symlink(&outside, project.join("escape")).unwrap();
        let text = |path: &Path| path.to_string_lossy().into_owned();
        let chats = [text(&chat), text(&home), "~".to_string()];
        let spaces = [text(&project)];
        let scope = Scope::of(&chats, &spaces, &data.join("scratch"), Some(home.clone()), vec![data.clone()]);
        assert_eq!(scope.path(&project.join(".")).unwrap(), project);
        assert!(scope.path(&chat).is_ok());
        // `other`: another device's chat folder, never passed in.
        for refused in [&other, &outside, &project.join("escape"), &home, &data, &root.join("missing")] {
            assert!(scope.path(refused).is_err(), "{refused:?}");
        }
        assert!(scope.checkout(&project).is_ok());
        assert!(scope.checkout(&home).is_err());
    }
}
