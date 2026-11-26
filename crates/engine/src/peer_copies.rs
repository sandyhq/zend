//! Working copies of chats another server starts here under
//! `sessions.run:<project>` (plan step 4d-2): such a chat never works in the
//! owner's project folder. Its agent, sandboxed, works in a copy of the
//! project in Zend's data; its children share that copy; what it changed
//! reaches the owner's folder only as a patch the owner applies here
//! (`zend peer changes`), or is thrown away.
//!
//! Kept in the data dir:
//! - `peer-copies/<chat>`: the copy, the sandbox's root and the chat's cwd.
//!   A detached git worktree of the project at its HEAD commit (no branch
//!   in the owner's repository), when the project is a git repository with
//!   a commit; else a copy of the folder (links copied as links, never
//!   followed; no `.git`, no file that holds secrets by its name
//!   ([`crate::peer_files::denied`]) and not Zend's data).
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
//! A snapshot is taken only while nothing of the copy's sandboxes runs: no
//! live run of a chat in it (the sessions engine says, [`on_held`]) and no
//! process on this host with the copy mounted (a sandbox's mount namespace:
//! [`sandbox_process`]); runs of its chats don't start meanwhile
//! ([`reviewing`]). Before it, the copy is measured without following
//! links: at most [`MAX_ENTRIES`] files and folders, [`MAX_FILE`] per file,
//! [`MAX_TOTAL`] in all (apparent sizes, sparse files at their length).
//!
//! The owner sees the changes as a tree (the snapshot's id) and applies
//! exactly that tree: when the copy moved since, nothing is applied.
//! Applying writes the files that changed since the base into the owner's
//! folder, all or none: refused for a path in a protected name
//! ([`zend_harness::sandbox::PROTECTED`]: `.git`, `.pi`, `.claude`,
//! `.envrc`, …, at any depth), a file the owner's tools run on their own
//! ([`RUNS_FOR_YOU`]: git settings, hook managers, Nix, editor tasks, MCP
//! servers, CI), a symbolic link or nested repository, a path through a
//! link or a file in the owner's folder, and a file the owner changed since
//! the copy was made. A file that instructs the owner's agents
//! ([`INSTRUCTS`]: `AGENTS.md`, `CLAUDE.md`, …) is applied only when the
//! owner allows that path. Mode changes (executable or not) are applied.
//! Then that tree becomes the base: what the chat changes next is a new
//! patch. Applying is a review step, not a filter: code and build scripts
//! are applied as the owner read them.
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

/// The copies' folder in the data dir.
pub const COPIES_DIR: &str = "peer-copies";
const META_DIR: &str = "peer-copies.meta";
/// Git's empty tree: attributes are read from it, so none of the copy's.
const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

/// Files and folders (by name, at any depth, any case) that make the
/// owner's own tools run something on their own: git settings (filters,
/// textconv, submodules, LFS name the drivers your config runs), hook
/// managers, Nix flakes and shells (direnv), MCP server lists, editor tasks
/// and dev containers. Never applied: read them in the patch and copy what
/// you want by hand. `.github/workflows` (CI runs it on push) is
/// [`runs_for_you`]'s too.
pub const RUNS_FOR_YOU: &[&str] = &[
    ".gitattributes", ".gitmodules", ".lfsconfig",
    ".pre-commit-config.yaml", "lefthook.yml", "lefthook.yaml", ".lefthook.yml", ".lefthook.yaml",
    "lefthook-local.yml", "lefthook-local.yaml", ".lefthook",
    "flake.nix", "flake.lock", "shell.nix",
    ".mcp.json", "opencode.json", ".zed", ".devcontainer",
];

/// Files (by name, at any depth, any case) that instruct or configure the
/// owner's agents: applied only when the owner allows each path.
pub const INSTRUCTS: &[&str] = &["AGENTS.md", "CLAUDE.md", "GEMINI.md", ".cursorrules", ".windsurfrules"];

/// What a flagged change says ([`INSTRUCTS`]).
pub const INSTRUCTS_NOTE: &str = "instructs or configures your tools: applied only when you allow this path";

/// The most files and folders a copy may hold, the largest file and all of
/// them together (apparent sizes): over these, its changes aren't read.
pub const MAX_ENTRIES: usize = 50_000;
pub const MAX_FILE: u64 = 20 << 20;
pub const MAX_TOTAL: u64 = 500 << 20;

/// One snapshot or apply at a time.
static LOCK: Mutex<()> = Mutex::new(());

/// Copies being read or applied now: their chats' runs don't start.
static REVIEWING: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

/// Who knows of live runs, by data folder: each says which chat's run
/// holds a copy (by its root), if one does.
type Holder = (PathBuf, Box<dyn Fn(&Path) -> Option<String> + Send + Sync>);
static HOLDERS: Mutex<Vec<Holder>> = Mutex::new(Vec::new());

/// Tell the copies in `data_dir` which live run holds a copy:
/// `holder(root)` names the chat whose run is live in the copy at `root`.
pub fn on_held(data_dir: &Path, holder: impl Fn(&Path) -> Option<String> + Send + Sync + 'static) {
    HOLDERS.lock().unwrap_or_else(|e| e.into_inner()).push((data_dir.to_path_buf(), Box::new(holder)));
}

/// Whether the copy at `root` is being read or applied now: a run of its
/// chats doesn't start.
pub fn reviewing(root: &Path) -> bool {
    REVIEWING.lock().unwrap_or_else(|e| e.into_inner()).iter().any(|r| r == root)
}

/// A copy marked as being read or applied; unmarked when dropped.
struct Review(PathBuf);

impl Drop for Review {
    fn drop(&mut self) {
        let mut reviewing = REVIEWING.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(at) = reviewing.iter().position(|r| *r == self.0) {
            reviewing.remove(at);
        }
    }
}

/// Why the owner's tools would treat `path` (from the project's root)
/// specially: `Err` never applied, `Ok(Some(note))` only when allowed.
pub fn path_rule(path: &str) -> Result<Option<&'static str>, String> {
    let parts: Vec<&str> = path.split('/').collect();
    if parts.iter().any(|p| p.is_empty() || *p == "." || *p == "..") {
        return Err("not a plain path".into());
    }
    let named = |names: &[&str], part: &str| names.iter().any(|name| name.eq_ignore_ascii_case(part));
    if let Some(name) = parts.iter().find(|p| named(zend_harness::sandbox::PROTECTED, p)) {
        return Err(format!("in {name}, which a server's chat may not change"));
    }
    if let Some(name) = parts.iter().find(|p| named(RUNS_FOR_YOU, p)) {
        return Err(format!("{name} makes your tools run something on their own: not applied (read it in the patch, copy it by hand)"));
    }
    if parts.windows(2).any(|w| w[0].eq_ignore_ascii_case(".github") && w[1].eq_ignore_ascii_case("workflows")) {
        return Err(".github/workflows runs on your next push: not applied (read it in the patch, copy it by hand)".into());
    }
    Ok(parts.last().is_some_and(|last| named(INSTRUCTS, last)).then_some(INSTRUCTS_NOTE))
}

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
    /// Applied only when the owner allows this path ([`INSTRUCTS`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flag: Option<String>,
    /// Never applied, and why ([`path_rule`], links, nested repositories).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refused: Option<String>,
    /// Only its mode changed (executable or not).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub mode_only: bool,
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
/// of the environment's git variables, none of its hooks and no fsmonitor
/// (what a server's request makes happen runs nothing of the owner's).
fn owner_git(dir: &Path) -> Command {
    let mut cmd = Command::new("git");
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("GIT_") {
            cmd.env_remove(key);
        }
    }
    cmd.args(["-c", "core.hooksPath=/dev/null", "-c", "core.fsmonitor=false"])
        .arg("-C")
        .arg(dir)
        .env("GIT_TERMINAL_PROMPT", "0");
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
        let kind = match &commit {
            Some(commit) => {
                // Detached: no branch of it in your repository, and none of
                // your hooks run (owner_git).
                let mut add = owner_git(owner);
                add.args(["worktree", "add", "--quiet", "--detach"]).arg(root).arg(commit);
                output(add, "git worktree add")?;
                Kind::Worktree
            }
            None => {
                std::fs::create_dir(root).map_err(|e| format!("{}: {e}", root.display()))?;
                let data = self.dir.parent().map(Path::to_path_buf).unwrap_or_default();
                let data = std::fs::canonicalize(&data).unwrap_or(data);
                let skip = |relative: &Path, path: &Path| {
                    crate::peer_files::denied(relative)
                        || std::fs::canonicalize(path).is_ok_and(|p| p.starts_with(&data))
                };
                copy_tree(owner, root, Path::new(""), &skip).map_err(|e| format!("copying {}: {e}", owner.display()))?;
                Kind::Copy
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
            branch: None,
            commit,
            base,
            created_at: crate::now_ms(),
        })
    }

    /// Mark `chat`'s copy as being read: refused while a run of a chat in
    /// it is live or a process of its sandbox is (see the module); until
    /// the mark is dropped no run of its chats starts. Checked after the
    /// mark is set: a run that starts meanwhile is seen live.
    fn review(&self, chat: &str) -> Result<Review, String> {
        let root = self.root(chat);
        let root = std::fs::canonicalize(&root).unwrap_or(root);
        REVIEWING.lock().unwrap_or_else(|e| e.into_inner()).push(root.clone());
        let review = Review(root.clone());
        let data = self.dir.parent().unwrap_or(&self.dir);
        let held = HOLDERS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .filter(|(dir, _)| same_dir(dir, data))
            .find_map(|(_, holder)| holder(&root));
        if let Some(holder) = held {
            return Err(format!(
                "chat {holder} has a live run in this working copy: stop it, or wait until it ends, then look again"
            ));
        }
        if let Some(pid) = sandbox_process(&root) {
            return Err(format!(
                "a process of this working copy's sandbox still runs (pid {pid}): stop its chat, or wait until it ends, then look again"
            ));
        }
        Ok(review)
    }

    /// The copy as it is now, as a tree in its own repository. Holds LOCK
    /// (and, but while it's made, a [`Review`]).
    fn snapshot(&self, chat: &str) -> Result<String, String> {
        let root = self.root(chat);
        if !root.is_dir() {
            return Err(format!("chat {chat}'s working copy is gone"));
        }
        let mut files = self.git(chat, true);
        files.args(["ls-files", "-z", "--cached", "--others", "--exclude-standard"]);
        measure(&root, &output(files, "its files")?)?;
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
            let path = String::from_utf8_lossy(path).into_owned();
            let (flag, mut refused) = match path_rule(&path) {
                Ok(flag) => (flag.map(String::from), None),
                Err(why) => (None, Some(why)),
            };
            for mode in [old_mode, new_mode] {
                match mode {
                    "120000" => refused = refused.or(Some("a symbolic link: not applied".into())),
                    "160000" => refused = refused.or(Some("a nested repository: not applied".into())),
                    _ => {}
                }
            }
            changes.push(Change {
                path,
                status: status.chars().next().map(String::from).unwrap_or_default(),
                flag,
                refused,
                mode_only: old == new && old_mode != new_mode,
                old_mode: old_mode.into(),
                new_mode: new_mode.into(),
                old: old.into(),
                new: new.into(),
            });
        }
        Ok(changes)
    }

    /// Every copy: its record and its tree now with what changed since its
    /// base (an error when it can't be read now: gone, too large, a live
    /// run in it, or git failed).
    pub fn list(&self) -> Vec<(Meta, Result<(String, Vec<Change>), String>)> {
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
                Some((meta, self.changes(&id).map(|(_, tree, changes)| (tree, changes))))
            })
            .collect()
    }

    /// `chat`'s record, its tree now and what changed since its base.
    pub fn changes(&self, chat: &str) -> Result<(Meta, String, Vec<Change>), String> {
        let meta = self.meta(chat)?;
        let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _review = self.review(chat)?;
        let now = self.snapshot(chat)?;
        let changes = self.diff(chat, &meta.base, &now)?;
        Ok((meta, now, changes))
    }

    /// The patch from `chat`'s base to its copy now (git's format, binary
    /// files included), with its tree (what [`Self::apply`] takes) and its
    /// changes.
    pub fn patch(&self, chat: &str) -> Result<(String, Vec<Change>, String), String> {
        let (meta, now, changes) = self.changes(chat)?;
        let mut diff = self.git(chat, false);
        diff.args([
            "diff-tree", "-r", "-p", "--binary", "--no-renames", "--no-ext-diff", "--no-textconv", "--full-index",
            &meta.base, &now,
        ]);
        let patch = output(diff, "its patch").map(|out| String::from_utf8_lossy(&out).into_owned())?;
        Ok((now, changes, patch))
    }

    /// Write what `chat`'s copy changed into the owner's folder, exactly the
    /// tree `tree` the owner was shown ([`Self::patch`], [`Self::list`]):
    /// all of it or nothing (see the module). `allow` names the flagged
    /// paths ([`INSTRUCTS`]) the owner lets through. Returns the paths
    /// written or removed.
    pub fn apply(&self, chat: &str, tree: &str, allow: &[String]) -> Result<Vec<String>, String> {
        let meta = self.meta(chat)?;
        let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _review = self.review(chat)?;
        let now = self.snapshot(chat)?;
        if now != tree.trim() {
            return Err(format!(
                "nothing applied: the working copy changed since you were shown {} (it is {} now): show it again",
                tree.trim(),
                now
            ));
        }
        let changes = self.diff(chat, &meta.base, &now)?;
        let owner = std::fs::canonicalize(&meta.owner)
            .map_err(|e| format!("your folder {}: {e}", meta.owner.display()))?;
        if owner != meta.owner {
            return Err(format!("your folder {} now leads elsewhere ({}): nothing applied", meta.owner.display(), owner.display()));
        }
        let mut refused = Vec::new();
        let mut todo = Vec::new();
        for change in &changes {
            if let Some(why) = &change.refused {
                refused.push(format!("{}: {why}", change.path));
                continue;
            }
            if change.flag.is_some() && !allow.iter().any(|path| *path == change.path) {
                refused.push(format!("{}: {INSTRUCTS_NOTE} (--allow {})", change.path, change.path));
                continue;
            }
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
    /// there already (content and mode), an error why not. Compares by the
    /// files' git ids, reading none of them whole.
    fn check(&self, chat: &str, owner: &Path, change: &Change) -> Result<bool, String> {
        let parts: Vec<&str> = change.path.split('/').collect();
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
            Ok(m) => Some((self.hash_file(chat, &target)?, executable(&m))),
            Err(_) => None,
        };
        let new = (change.status != "D").then(|| (change.new.clone(), change.new_mode == "100755"));
        let old = (change.status != "A").then(|| (change.old.clone(), change.old_mode == "100755"));
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

    /// The git blob id of the owner's file at `path` (not followed if a
    /// link), streamed through git.
    fn hash_file(&self, chat: &str, path: &Path) -> Result<String, String> {
        use std::process::Stdio;
        let mut file = open_nofollow(path).map_err(|e| format!("reading yours: {e}"))?;
        let mut hash = self.git(chat, false);
        hash.args(["hash-object", "--no-filters", "--stdin"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = hash.spawn().map_err(|e| format!("git: {e}"))?;
        let copied = child.stdin.take().map(|mut stdin| std::io::copy(&mut file, &mut stdin));
        let out = child.wait_with_output().map_err(|e| format!("git: {e}"))?;
        if let Some(Err(e)) = copied {
            return Err(format!("reading yours: {e}"));
        }
        if !out.status.success() {
            return Err("reading yours: git hash-object failed".into());
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }

    /// Stream the blob `id` of `chat`'s repository into `file`.
    fn blob_to(&self, chat: &str, id: &str, file: &mut std::fs::File) -> Result<(), String> {
        use std::process::Stdio;
        let mut cat = self.git(chat, false);
        cat.args(["cat-file", "blob", id]).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
        let mut child = cat.spawn().map_err(|e| format!("git: {e}"))?;
        let copied = child.stdout.take().map(|mut out| std::io::copy(&mut out, file));
        let status = child.wait().map_err(|e| format!("git: {e}"))?;
        match copied {
            Some(Ok(_)) if status.success() => Ok(()),
            Some(Err(e)) => Err(format!("writing it: {e}")),
            _ => Err(format!("a file of it ({id}): git cat-file failed")),
        }
    }

    fn write(&self, chat: &str, owner: &Path, change: &Change) -> Result<(), String> {
        let target = owner.join(&change.path);
        if change.status == "D" {
            return std::fs::remove_file(&target).map_err(bad);
        }
        let exec = change.new_mode == "100755";
        if change.mode_only {
            return set_executable(&target, exec).map_err(bad);
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
        let temp = dir.join(format!(".zend-apply-{}", crate::new_id()));
        let written = (|| -> Result<(), String> {
            let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&temp).map_err(bad)?;
            self.blob_to(chat, &change.new, &mut file)?;
            file.sync_all().map_err(bad)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt as _;
                let mode = std::fs::symlink_metadata(&target)
                    .map(|m| m.permissions().mode() & 0o777)
                    .unwrap_or(0o644);
                let mode = if exec { mode | 0o111 } else { mode & !0o111 };
                std::fs::set_permissions(&temp, std::fs::Permissions::from_mode(mode)).map_err(bad)?;
            }
            std::fs::rename(&temp, &target).map_err(bad)
        })();
        if written.is_err() {
            let _ = std::fs::remove_file(&temp);
        }
        written
    }

    /// Throw `chat`'s copy away (its worktree too), and its record: the
    /// owner's folder stays as it is, and the chat's next run is refused.
    /// No branch is deleted (a copy has none; one made before copies were
    /// detached keeps its branch, the owner's to delete).
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
        }
        std::fs::remove_dir_all(self.meta_dir(chat)).map_err(bad)?;
        Ok(meta)
    }
}

fn same_dir(a: &Path, b: &Path) -> bool {
    a == b || std::fs::canonicalize(a).ok().is_some_and(|a| std::fs::canonicalize(b).ok() == Some(a))
}

#[cfg(unix)]
fn executable(meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt as _;
    meta.permissions().mode() & 0o100 != 0
}

#[cfg(not(unix))]
fn executable(_meta: &std::fs::Metadata) -> bool {
    false
}

#[cfg(unix)]
fn set_executable(path: &Path, exec: bool) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    let meta = std::fs::symlink_metadata(path)?;
    if !meta.is_file() {
        return Err(std::io::Error::other("not a file"));
    }
    let mode = meta.permissions().mode() & 0o777;
    let mode = if exec { mode | 0o111 } else { mode & !0o111 };
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
}

#[cfg(not(unix))]
fn set_executable(_path: &Path, _exec: bool) -> std::io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn restrict(dir: &Path) {
    use std::os::unix::fs::PermissionsExt as _;
    let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
}

#[cfg(not(unix))]
fn restrict(_dir: &Path) {}

#[cfg(unix)]
fn open_nofollow(path: &Path) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt as _;
    std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW).open(path)
}

#[cfg(not(unix))]
fn open_nofollow(path: &Path) -> std::io::Result<std::fs::File> {
    std::fs::File::open(path)
}

/// Refuse a copy too large to read: more than [`MAX_ENTRIES`] files, a
/// file over [`MAX_FILE`], or over [`MAX_TOTAL`] in all. `listed` are the
/// paths a snapshot would read (git's list of the copy's files, without
/// what its .gitignore leaves out: a build folder isn't counted); each is
/// measured with lstat (a link counts as itself, never followed), sizes
/// apparent (a sparse file counts at its length).
fn measure(root: &Path, listed: &[u8]) -> Result<(), String> {
    let mib = |bytes: u64| format!("{:.1} MiB", bytes as f64 / (1024.0 * 1024.0));
    let (mut entries, mut total) = (0usize, 0u64);
    let mut large: Vec<(u64, String)> = Vec::new();
    let mut over = Vec::new();
    for path in listed.split(|b| *b == 0).filter(|p| !p.is_empty()) {
        entries += 1;
        if entries > MAX_ENTRIES {
            return Err(format!(
                "the working copy holds more than {MAX_ENTRIES} files: its changes aren't read (remove what it made, or discard it)"
            ));
        }
        let relative = path_of(path);
        let Ok(meta) = std::fs::symlink_metadata(root.join(&relative)) else { continue };
        if meta.is_dir() {
            continue;
        }
        let len = meta.len();
        total = total.saturating_add(len);
        let name = relative.display().to_string();
        if len > MAX_FILE {
            over.push(format!("{name} ({})", mib(len)));
        }
        if len >= 1 << 20 {
            large.push((len, name));
        }
    }
    if !over.is_empty() {
        over.sort();
        return Err(format!(
            "files larger than {} in the working copy: its changes aren't read (remove them, or discard it): {}",
            mib(MAX_FILE),
            over.join(", ")
        ));
    }
    if total > MAX_TOTAL {
        large.sort_by(|a, b| b.0.cmp(&a.0));
        let names: Vec<String> = large.iter().take(10).map(|(len, p)| format!("{p} ({})", mib(*len))).collect();
        return Err(format!(
            "the working copy holds {} in all, more than {}: its changes aren't read; largest: {}",
            mib(total),
            mib(MAX_TOTAL),
            names.join(", ")
        ));
    }
    Ok(())
}

#[cfg(unix)]
fn path_of(bytes: &[u8]) -> PathBuf {
    use std::os::unix::ffi::OsStrExt as _;
    PathBuf::from(std::ffi::OsStr::from_bytes(bytes))
}

#[cfg(not(unix))]
fn path_of(bytes: &[u8]) -> PathBuf {
    PathBuf::from(String::from_utf8_lossy(bytes).into_owned())
}

/// A process on this host with the copy at `root` mounted: one in a mount
/// namespace other than this engine's (a sandbox's, which bubblewrap made
/// with the copy bound at its own path) that has a mount there. The kernel
/// keeps a mount namespace while a process is in it, and an agent in it
/// can't take the mount away (it has no capabilities there), so none found
/// means no process of a sandbox of the copy is alive. Its pid.
#[cfg(target_os = "linux")]
pub fn sandbox_process(root: &Path) -> Option<u32> {
    let own = std::fs::read_link("/proc/self/ns/mnt").ok()?;
    let canonical = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let mut seen = std::collections::HashSet::new();
    for entry in std::fs::read_dir("/proc").ok()?.flatten() {
        let Some(pid) = entry.file_name().to_str().and_then(|s| s.parse::<u32>().ok()) else { continue };
        let Ok(ns) = std::fs::read_link(entry.path().join("ns/mnt")) else { continue };
        if ns == own || seen.contains(&ns) {
            continue;
        }
        let Ok(info) = std::fs::read_to_string(entry.path().join("mountinfo")) else { continue };
        seen.insert(ns);
        let mounted = info.lines().any(|line| {
            line.split(' ').nth(4).is_some_and(|point| {
                let point = PathBuf::from(unescape_mount(point));
                point == root || point == canonical
            })
        });
        if mounted {
            return Some(pid);
        }
    }
    None
}

#[cfg(not(target_os = "linux"))]
pub fn sandbox_process(_root: &Path) -> Option<u32> {
    None
}

/// A mountinfo path: spaces, tabs, newlines and backslashes as `\ooo`.
#[cfg(target_os = "linux")]
fn unescape_mount(field: &str) -> std::ffi::OsString {
    use std::os::unix::ffi::OsStringExt as _;
    let bytes = field.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 4 <= bytes.len() && bytes[i + 1..i + 4].iter().all(|b| (b'0'..=b'7').contains(b)) {
            let value = (bytes[i + 1] - b'0') * 64 + (bytes[i + 2] - b'0') * 8 + (bytes[i + 3] - b'0');
            out.push(value);
            i += 4;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    std::ffi::OsString::from_vec(out)
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
/// links (never followed), nothing else (sockets, devices), no `.git` (a
/// repository with no commit yet, a nested one: the copy has no git), and
/// nothing `skip` names (by its path from the top and its own).
fn copy_tree(from: &Path, to: &Path, relative: &Path, skip: &dyn Fn(&Path, &Path) -> bool) -> std::io::Result<()> {
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        if entry.file_name().to_string_lossy().eq_ignore_ascii_case(".git") {
            continue;
        }
        let source = entry.path();
        let inside = relative.join(entry.file_name());
        let kind = entry.file_type()?;
        if crate::peer_files::denied(&inside) || (!kind.is_symlink() && skip(&inside, &source)) {
            continue;
        }
        let target = to.join(entry.file_name());
        if kind.is_symlink() {
            #[cfg(unix)]
            std::os::unix::fs::symlink(std::fs::read_link(&source)?, &target)?;
        } else if kind.is_dir() {
            std::fs::create_dir(&target)?;
            copy_tree(&source, &target, &inside, skip)?;
        } else if kind.is_file() {
            std::fs::copy(&source, &target)?;
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "peer_copies_tests.rs"]
mod tests;
