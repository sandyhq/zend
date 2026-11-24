//! Working copies of chats another server starts here under
//! `sessions.run:<project>` (plan step 4d-2): such a chat never works in the
//! owner's project folder. Its agent, sandboxed, works in a copy of the
//! project in Zend's data; its children share that copy; what it changed
//! reaches the owner's folder only as a patch the owner applies here
//! (`zend peer changes`), or is thrown away.
//!
//! Kept in the data dir:
//! - `peer-copies/<chat>`: the copy, the sandbox's root and the chat's cwd.
//!   A git worktree of the project on a new branch `zend-peer/<chat>`, at
//!   its HEAD commit, when the project is a git repository with a commit;
//!   else a copy of the folder (links copied as links, never followed).
//! - `peer-copies.meta/<chat>/`: what this engine knows of it (`copy.json`)
//!   and a git repository of its own (`git/`) that holds the copy's
//!   snapshots. Never in the sandbox.
//!
//! Git in the copy: the agent can write anything in it but its `.git`
//! pointer (a protected name, read-only in the sandbox), so no git this
//! engine runs reads the copy's own git setup. The worktree's git folder is
//! in the owner's repository (`.git/worktrees/<chat>`), which the sandbox
//! never sees. The general rule holds for the copy ([`crate::confined`]: no
//! host git there); the one exception is this module's snapshots, which run
//! git against its own repository (`GIT_DIR`), with no system or global
//! config, no hooks, no fsmonitor and attributes read from the empty tree:
//! nothing the agent wrote names a program for it to run.
//!
//! Applying writes the files that changed since the last snapshot taken
//! into the owner's folder, all or none: refused for a path in a protected
//! name ([`zend_harness::sandbox::PROTECTED`]: `.git`, `.pi`, `.claude`,
//! `.envrc`, …, at any depth) or a git settings file (`.gitattributes`,
//! `.gitmodules`, `.lfsconfig`: they make the owner's git run what its
//! config names), a symbolic link or nested repository, a path
//! through a link or a file in the owner's folder, and a file the owner
//! changed since the copy was made. Then that state becomes the base:
//! what the chat changes next is a new patch.
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

/// The copies' folder in the data dir.
pub const COPIES_DIR: &str = "peer-copies";
const META_DIR: &str = "peer-copies.meta";
const BRANCH_PREFIX: &str = "zend-peer/";
/// Git's empty tree: attributes are read from it, so none of the copy's.
const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

/// Files in a work tree that make git run the drivers and commands your
/// git config names (filters, textconv, submodule updates, LFS): never
/// applied, at any depth.
const GIT_SETTINGS: &[&str] = &[".gitattributes", ".gitmodules", ".lfsconfig"];

/// One snapshot or apply at a time.
static LOCK: Mutex<()> = Mutex::new(());

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    /// A git worktree of the project.
    Worktree,
    /// A copy of a folder that isn't a git repository (or has no commit).
    Copy,
}

/// What is kept of one copy.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Meta {
    /// The chat it was made for: its name.
    pub chat: String,
    /// The paired server whose chat it is (its key).
    pub peer: String,
    /// The project (space id).
    pub project: String,
    /// The owner's folder, canonical: where an apply writes.
    pub owner: PathBuf,
    pub kind: Kind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    /// The tree (in its own repository) the owner's folder has of it: as
    /// made, or as last applied.
    pub base: String,
    pub created_at: i64,
}

/// One changed path, from the base to the copy as it is.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Change {
    pub path: String,
    /// A (added), M (modified), D (deleted), T (its kind changed).
    pub status: String,
    #[serde(skip)]
    old_mode: String,
    #[serde(skip)]
    new_mode: String,
    #[serde(skip)]
    old: String,
    #[serde(skip)]
    new: String,
}

/// The working copies of one engine.
#[derive(Clone, Debug)]
pub struct Copies {
    dir: PathBuf,
    meta: PathBuf,
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 128 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn bad(why: impl std::fmt::Display) -> String {
    why.to_string()
}

/// `git -C dir`, for the owner's own repository: its own config, nothing
/// of the environment's git variables.
fn owner_git(dir: &Path) -> Command {
    let mut cmd = Command::new("git");
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("GIT_") {
            cmd.env_remove(key);
        }
    }
    cmd.arg("-C").arg(dir).env("GIT_TERMINAL_PROMPT", "0");
    cmd
}

fn output(mut cmd: Command, what: &str) -> Result<Vec<u8>, String> {
    let out = cmd
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("{what}: git: {e}"))?;
    if !out.status.success() {
        return Err(format!("{what}: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(out.stdout)
}

fn text(cmd: Command, what: &str) -> Result<String, String> {
    output(cmd, what).map(|out| String::from_utf8_lossy(&out).trim().to_string())
}

impl Copies {
    pub fn new(data_dir: &Path) -> Self {
        Self { dir: data_dir.join(COPIES_DIR), meta: data_dir.join(META_DIR) }
    }

    /// The copies' folder (each copy is a folder directly in it).
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The copy of `chat` (whether or not it exists).
    pub fn root(&self, chat: &str) -> PathBuf {
        self.dir.join(chat)
    }

    /// Whether `path` is a copy's root or inside one.
    pub fn holds(&self, path: &Path) -> bool {
        let dir = std::fs::canonicalize(&self.dir).unwrap_or_else(|_| self.dir.clone());
        path.starts_with(&dir) || path.starts_with(&self.dir)
    }

    fn meta_dir(&self, chat: &str) -> PathBuf {
        self.meta.join(chat)
    }

    /// The copy `chat`'s record.
    pub fn meta(&self, chat: &str) -> Result<Meta, String> {
        if !valid_id(chat) {
            return Err(format!("{chat}: not a chat id"));
        }
        let raw = std::fs::read(self.meta_dir(chat).join("copy.json"))
            .map_err(|_| format!("no working copy of a server's chat {chat} here"))?;
        serde_json::from_slice(&raw).map_err(|e| format!("{chat}: its record: {e}"))
    }

    fn save(&self, meta: &Meta) -> Result<(), String> {
        let bytes = serde_json::to_vec_pretty(meta).map_err(bad)?;
        crate::runner::write_private_atomic(&self.meta_dir(&meta.chat).join("copy.json"), &bytes).map_err(bad)
    }

    /// Git against the copy's own repository, for its snapshots: nothing of
    /// the system's, the user's or the copy's git setup (see the module).
    fn git(&self, chat: &str, worktree: bool) -> Command {
        let meta = self.meta_dir(chat);
        let mut cmd = Command::new("git");
        cmd.env_clear();
        if let Some(path) = std::env::var_os("PATH") {
            cmd.env("PATH", path);
        }
        cmd.env("GIT_DIR", meta.join("git"))
            .env("GIT_INDEX_FILE", meta.join("index"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_ATTR_SOURCE", EMPTY_TREE)
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("HOME", &meta)
            .args([
                "-c", "core.fsmonitor=false",
                "-c", "core.hooksPath=/dev/null",
                "-c", "core.untrackedCache=false",
                "-c", "core.autocrlf=false",
                "-c", "core.symlinks=true",
                "-c", "core.quotePath=false",
                "-c", "core.excludesFile=/dev/null",
                "-c", "diff.external=",
            ]);
        if worktree {
            let root = self.root(chat);
            cmd.env("GIT_WORK_TREE", &root).current_dir(&root);
        } else {
            cmd.current_dir(&meta);
        }
        cmd
    }

    /// Make `chat`'s copy of the owner's folder `owner` (project `project`,
    /// server `peer`) and its first snapshot. Returns its root, canonical.
    pub fn create(&self, chat: &str, peer: &str, project: &str, owner: &Path) -> Result<PathBuf, String> {
        if !valid_id(chat) {
            return Err(format!("{chat}: not a chat id"));
        }
        let owner = std::fs::canonicalize(owner).map_err(|e| format!("the project's folder {}: {e}", owner.display()))?;
        let root = self.root(chat);
        if std::fs::symlink_metadata(&root).is_ok() || self.meta_dir(chat).exists() {
            return Err(format!("chat {chat} has a working copy already"));
        }
        for dir in [&self.dir, &self.meta] {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
            restrict(dir);
        }
        std::fs::create_dir(self.meta_dir(chat)).map_err(|e| format!("its record: {e}"))?;
        let made = self.make(chat, peer, project, &owner, &root);
        if made.is_err() {
            let _ = std::fs::remove_dir_all(&root);
            let _ = std::fs::remove_dir_all(self.meta_dir(chat));
            if owner.join(".git").exists() {
                let mut prune = owner_git(&owner);
                prune.args(["worktree", "prune"]);
                let _ = output(prune, "prune");
            }
        }
        made?;
        std::fs::canonicalize(&root).map_err(bad)
    }

    fn make(&self, chat: &str, peer: &str, project: &str, owner: &Path, root: &Path) -> Result<(), String> {
        crate::confined::refuse_git(owner).map_err(bad)?;
        let commit = owner
            .join(".git")
            .exists()
            .then(|| {
                let mut head = owner_git(owner);
                head.args(["rev-parse", "--verify", "--quiet", "HEAD^{commit}"]);
                text(head, "its HEAD").ok().filter(|c| !c.is_empty())
            })
            .flatten();
        let (kind, branch) = match &commit {
            Some(commit) => {
                let branch = format!("{BRANCH_PREFIX}{chat}");
                // A server's request runs none of your hooks.
                let mut add = owner_git(owner);
                add.args(["-c", "core.hooksPath=/dev/null", "worktree", "add", "--quiet", "-b", &branch])
                    .arg(root)
                    .arg(commit);
                output(add, "git worktree add")?;
                (Kind::Worktree, Some(branch))
            }
            None => {
                std::fs::create_dir(root).map_err(|e| format!("{}: {e}", root.display()))?;
                copy_tree(owner, root).map_err(|e| format!("copying {}: {e}", owner.display()))?;
                (Kind::Copy, None)
            }
        };
        let mut init = Command::new("git");
        init.env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .args(["init", "--quiet", "--bare", "--template="])
            .arg(self.meta_dir(chat).join("git"));
        output(init, "its snapshots' repository")?;
        let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let base = self.snapshot(chat)?;
        self.save(&Meta {
            chat: chat.into(),
            peer: peer.into(),
            project: project.into(),
            owner: owner.to_path_buf(),
            kind,
            branch,
            commit,
            base,
            created_at: crate::now_ms(),
        })
    }

    /// The copy as it is now, as a tree in its own repository. Holds LOCK.
    fn snapshot(&self, chat: &str) -> Result<String, String> {
        let root = self.root(chat);
        if !root.is_dir() {
            return Err(format!("chat {chat}'s working copy is gone"));
        }
        // A repository the agent made in it: git would open it (a gitlink),
        // its config with it. Not read: it goes first.
        if let Some(nested) = nested_git(&root) {
            return Err(format!(
                "{} in the working copy is a git repository of its own: remove it to see or apply the changes",
                nested.display()
            ));
        }
        let mut add = self.git(chat, true);
        add.args(["add", "--all", "--", "."]);
        output(add, "snapshot")?;
        let mut write = self.git(chat, true);
        write.arg("write-tree");
        text(write, "snapshot")
    }

    fn diff(&self, chat: &str, from: &str, to: &str) -> Result<Vec<Change>, String> {
        let mut raw = self.git(chat, false);
        raw.args(["diff-tree", "-r", "-z", "--raw", "--no-renames", "--full-index", from, to]);
        let out = output(raw, "its changes")?;
        let mut fields = out.split(|b| *b == 0).filter(|f| !f.is_empty());
        let mut changes = Vec::new();
        while let (Some(head), Some(path)) = (fields.next(), fields.next()) {
            let head = String::from_utf8_lossy(head);
            let parts: Vec<&str> = head.trim_start_matches(':').split(' ').collect();
            let [old_mode, new_mode, old, new, status] = parts[..] else {
                return Err(format!("its changes: can't read {head}"));
            };
            changes.push(Change {
                path: String::from_utf8_lossy(path).into_owned(),
                status: status.chars().next().map(String::from).unwrap_or_default(),
                old_mode: old_mode.into(),
                new_mode: new_mode.into(),
                old: old.into(),
                new: new.into(),
            });
        }
        Ok(changes)
    }

    fn blob(&self, chat: &str, id: &str) -> Result<Vec<u8>, String> {
        let mut cat = self.git(chat, false);
        cat.args(["cat-file", "blob", id]);
        output(cat, "a file of it")
    }

    /// Every copy: its record and what changed since its base (`None` when it
    /// can't be read: gone, or git failed, with why).
    pub fn list(&self) -> Vec<(Meta, Result<Vec<Change>, String>)> {
        let Ok(entries) = std::fs::read_dir(&self.meta) else { return Vec::new() };
        let mut ids: Vec<String> = entries
            .flatten()
            .filter_map(|e| e.file_name().to_str().map(String::from))
            .filter(|id| valid_id(id))
            .collect();
        ids.sort();
        ids.into_iter()
            .filter_map(|id| {
                let meta = self.meta(&id).ok()?;
                Some((meta, self.changes(&id).map(|(_, _, changes)| changes)))
            })
            .collect()
    }

    /// `chat`'s record, its tree now and what changed since its base.
    pub fn changes(&self, chat: &str) -> Result<(Meta, String, Vec<Change>), String> {
        let meta = self.meta(chat)?;
        let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let now = self.snapshot(chat)?;
        let changes = self.diff(chat, &meta.base, &now)?;
        Ok((meta, now, changes))
    }

    /// The patch from `chat`'s base to its copy now (git's format, binary
    /// files included).
    pub fn patch(&self, chat: &str) -> Result<String, String> {
        let (meta, now, _) = self.changes(chat)?;
        let mut diff = self.git(chat, false);
        diff.args([
            "diff-tree", "-r", "-p", "--binary", "--no-renames", "--no-ext-diff", "--no-textconv", "--full-index",
            &meta.base, &now,
        ]);
        output(diff, "its patch").map(|out| String::from_utf8_lossy(&out).into_owned())
    }

    /// Write what `chat`'s copy changed into the owner's folder: all of it
    /// or nothing (see the module). Returns the paths written or removed.
    pub fn apply(&self, chat: &str) -> Result<Vec<String>, String> {
        let meta = self.meta(chat)?;
        let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let now = self.snapshot(chat)?;
        let changes = self.diff(chat, &meta.base, &now)?;
        let owner = std::fs::canonicalize(&meta.owner)
            .map_err(|e| format!("your folder {}: {e}", meta.owner.display()))?;
        if owner != meta.owner {
            return Err(format!("your folder {} now leads elsewhere ({}): nothing applied", meta.owner.display(), owner.display()));
        }
        let mut refused = Vec::new();
        let mut todo = Vec::new();
        for change in &changes {
            match self.check(chat, &owner, change) {
                Ok(true) => todo.push(change),
                Ok(false) => {}
                Err(why) => refused.push(format!("{}: {why}", change.path)),
            }
        }
        if !refused.is_empty() {
            return Err(format!("nothing applied:\n  {}", refused.join("\n  ")));
        }
        let mut done = Vec::new();
        for change in todo {
            self.write(chat, &owner, change).map_err(|e| {
                format!("{}: {e} (applied before it: {})", change.path, if done.is_empty() { "none".into() } else { done.join(", ") })
            })?;
            done.push(change.path.clone());
        }
        self.save(&Meta { base: now, ..meta })?;
        Ok(done)
    }

    /// Whether `change` may be applied to `owner`: `Ok(false)` when it is
    /// there already, an error why not.
    fn check(&self, chat: &str, owner: &Path, change: &Change) -> Result<bool, String> {
        let parts: Vec<&str> = change.path.split('/').collect();
        if parts.iter().any(|p| p.is_empty() || *p == "." || *p == "..") {
            return Err("not a plain path".into());
        }
        if let Some(name) = parts
            .iter()
            .find(|p| zend_harness::sandbox::PROTECTED.iter().any(|name| name.eq_ignore_ascii_case(p)))
        {
            return Err(format!("in {name}, which a server's chat may not change"));
        }
        if let Some(name) = parts.iter().find(|p| GIT_SETTINGS.iter().any(|name| name.eq_ignore_ascii_case(p))) {
            return Err(format!("{name} tells your git what to run on your files: not applied (read it in the patch, copy it by hand)"));
        }
        for mode in [&change.old_mode, &change.new_mode] {
            match mode.as_str() {
                "120000" => return Err("a symbolic link: not applied".into()),
                "160000" => return Err("a nested repository: not applied".into()),
                _ => {}
            }
        }
        // Every folder on its way: a folder, never a link, or not there yet.
        let mut dir = owner.to_path_buf();
        for part in &parts[..parts.len() - 1] {
            dir.push(part);
            match std::fs::symlink_metadata(&dir) {
                Ok(m) if m.file_type().is_symlink() => return Err(format!("{} in your folder is a link", dir.display())),
                Ok(m) if !m.is_dir() => return Err(format!("{} in your folder is not a folder", dir.display())),
                Ok(_) => {}
                Err(_) => break,
            }
        }
        let target = owner.join(&change.path);
        let present = match std::fs::symlink_metadata(&target) {
            Ok(m) if m.file_type().is_symlink() => return Err("a link in your folder".into()),
            Ok(m) if !m.is_file() => return Err("not a file in your folder".into()),
            Ok(_) => Some(read_nofollow(&target).map_err(|e| format!("reading yours: {e}"))?),
            Err(_) => None,
        };
        let new = (change.status != "D").then(|| self.blob(chat, &change.new)).transpose()?;
        let old = (change.status != "A").then(|| self.blob(chat, &change.old)).transpose()?;
        if present == new {
            return Ok(false);
        }
        if present != old {
            return Err(match (&present, &old) {
                (Some(_), None) => "your folder has a file of its own there".into(),
                (None, Some(_)) => "deleted in your folder since the copy was made".into(),
                _ => "changed in your folder since the copy was made".into(),
            });
        }
        Ok(true)
    }

    fn write(&self, chat: &str, owner: &Path, change: &Change) -> Result<(), String> {
        let target = owner.join(&change.path);
        if change.status == "D" {
            return std::fs::remove_file(&target).map_err(bad);
        }
        let parts: Vec<&str> = change.path.split('/').collect();
        let mut dir = owner.to_path_buf();
        for part in &parts[..parts.len() - 1] {
            dir.push(part);
            match std::fs::symlink_metadata(&dir) {
                Ok(m) if m.is_dir() && !m.file_type().is_symlink() => {}
                Ok(_) => return Err(format!("{} is no longer a folder", dir.display())),
                Err(_) => std::fs::create_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))?,
            }
        }
        let bytes = self.blob(chat, &change.new)?;
        let temp = dir.join(format!(".zend-apply-{}", crate::new_id()));
        let written = (|| -> std::io::Result<()> {
            use std::io::Write as _;
            let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&temp)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt as _;
                let mode = std::fs::symlink_metadata(&target)
                    .map(|m| m.permissions().mode() & 0o777)
                    .unwrap_or(0o644);
                let mode = if change.new_mode == "100755" { mode | 0o111 } else { mode & !0o111 };
                std::fs::set_permissions(&temp, std::fs::Permissions::from_mode(mode))?;
            }
            std::fs::rename(&temp, &target)
        })();
        if written.is_err() {
            let _ = std::fs::remove_file(&temp);
        }
        written.map_err(bad)
    }

    /// Throw `chat`'s copy away (its worktree and branch too), and its
    /// record: the owner's folder stays as it is, and the chat's next run
    /// is refused.
    pub fn discard(&self, chat: &str) -> Result<Meta, String> {
        let meta = self.meta(chat)?;
        let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let root = self.root(chat);
        if std::fs::symlink_metadata(&root).is_ok() {
            std::fs::remove_dir_all(&root).map_err(|e| format!("removing {}: {e}", root.display()))?;
        }
        if meta.kind == Kind::Worktree && meta.owner.is_dir() {
            // Nothing that reads the copy: its folder is gone first.
            let mut prune = owner_git(&meta.owner);
            prune.args(["worktree", "prune"]);
            if let Err(error) = output(prune, "git worktree prune") {
                tracing::warn!(chat, %error, "peer copies: pruning the worktree failed");
            }
            if let Some(branch) = &meta.branch {
                let mut delete = owner_git(&meta.owner);
                delete.args(["branch", "-D", "--quiet", branch]);
                if let Err(error) = output(delete, "git branch -D") {
                    tracing::warn!(chat, %error, "peer copies: deleting its branch failed");
                }
            }
        }
        std::fs::remove_dir_all(self.meta_dir(chat)).map_err(bad)?;
        Ok(meta)
    }
}

#[cfg(unix)]
fn restrict(dir: &Path) {
    use std::os::unix::fs::PermissionsExt as _;
    let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
}

#[cfg(not(unix))]
fn restrict(_dir: &Path) {}

#[cfg(unix)]
fn read_nofollow(path: &Path) -> std::io::Result<Vec<u8>> {
    use std::io::Read as _;
    use std::os::unix::fs::OpenOptionsExt as _;
    let mut file = std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW).open(path)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(bytes)
}

#[cfg(not(unix))]
fn read_nofollow(path: &Path) -> std::io::Result<Vec<u8>> {
    std::fs::read(path)
}

/// A `.git` (any case, any kind) below `root`'s top: a repository in it.
/// Links aren't followed.
fn nested_git(root: &Path) -> Option<PathBuf> {
    let mut todo = vec![(root.to_path_buf(), true)];
    while let Some((dir, top)) = todo.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let is_git = name.to_string_lossy().eq_ignore_ascii_case(".git");
            if is_git && !top {
                return Some(entry.path());
            }
            if !is_git && entry.file_type().is_ok_and(|t| t.is_dir()) {
                todo.push((entry.path(), false));
            }
        }
    }
    None
}

/// Copy the folder `from` into `to` (made): files and folders, links as
/// links (never followed), nothing else (sockets, devices), and no `.git`
/// (a repository with no commit yet, a nested one): the copy has no git.
fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        if entry.file_name().to_string_lossy().eq_ignore_ascii_case(".git") {
            continue;
        }
        let source = entry.path();
        let target = to.join(entry.file_name());
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            #[cfg(unix)]
            std::os::unix::fs::symlink(std::fs::read_link(&source)?, &target)?;
        } else if kind.is_dir() {
            std::fs::create_dir(&target)?;
            copy_tree(&source, &target)?;
        } else if kind.is_file() {
            std::fs::copy(&source, &target)?;
        }
    }
    Ok(())
}
