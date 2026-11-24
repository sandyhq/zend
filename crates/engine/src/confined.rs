//! Folders whose agent runs confined: a sandboxed chat's project root
//! ([`crate::sessions::SessionsEngine::sandbox_chat`]). Its agent may
//! write anything in there, `.git/config`, `.gitattributes` and hooks too,
//! so a `git` this engine ran there would run what it planted (a filter
//! driver, a textconv, `core.fsmonitor`, a hook) outside the sandbox, as the
//! owner. So this engine runs no git in them: turn snapshots, diffs, the
//! source stamp and the window's git actions skip or refuse such a folder.
//!
//! Neutralizing git's configuration instead (`-c` overrides) can't cover a
//! repository's own config: filter and diff drivers have names of the
//! agent's choosing, and the list of settings that run a program
//! (`core.fsmonitor`, `core.sshCommand`, `diff.external`, credential
//! helpers…) keeps growing.
//!
//! The list is per process: every engine in it skips these folders. It
//! holds each root the run journal has a mark for ([`load`]) and each one
//! marked since.
use std::path::{Path, PathBuf};
use std::sync::{PoisonError, RwLock};

use crate::EngineError;

static ROOTS: RwLock<Vec<PathBuf>> = RwLock::new(Vec::new());

/// Confine `root` (canonical) from now on.
pub(crate) fn add(root: &Path) {
    let mut roots = ROOTS.write().unwrap_or_else(PoisonError::into_inner);
    if !roots.iter().any(|known| known == root) {
        roots.push(root.to_path_buf());
    }
}

/// Confine every root the run journal has a mark for.
pub(crate) fn load(journal: &crate::run_journal::RunJournal) {
    for root in journal.sandbox_roots() {
        add(&root);
    }
}

/// Whether `path` is, or is inside, a confined folder: symlinks resolved
/// where it exists, else as given (made absolute).
pub fn holds(path: &Path) -> bool {
    let roots = ROOTS.read().unwrap_or_else(PoisonError::into_inner);
    if roots.is_empty() {
        return false;
    }
    let path = std::fs::canonicalize(path)
        .or_else(|_| std::path::absolute(path))
        .unwrap_or_else(|_| path.to_path_buf());
    roots.iter().any(|root| path.starts_with(root))
}

/// An error when `path` is in a confined folder ([`holds`]): git must not
/// run there.
pub(crate) fn refuse_git(path: &Path) -> Result<(), EngineError> {
    if holds(path) {
        return Err(EngineError::Other(format!(
            "git doesn't run in {}: a sandboxed chat's agent works there and can change its git configuration",
            path.display()
        )));
    }
    Ok(())
}
