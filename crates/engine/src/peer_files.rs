//! A shared project's files as a paired server sees them (`projects.share:<p>`,
//! [`crate::rpc`]'s peer gate), safe against a project that changes under the
//! read:
//!
//! - Opened from the project's folder (a directory fd), never by a path
//!   checked first and opened after: on Linux with `openat2`
//!   (`RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS`), elsewhere (or a kernel
//!   without it) one component at a time with `O_NOFOLLOW`. No symlink is
//!   followed, inside the project or out of it; `..` never resolves.
//! - Size, kind and bytes come from that fd: a regular file only, with one
//!   name (a hard link could be a file from outside the project).
//! - A folder that is, holds or sits inside Zend's data folder, holds the
//!   home folder, or sits in a hidden folder of it (`~/.ssh`, `~/.config`,
//!   `~/.aws`, …) is never shared ([`unshareable`]).
//! - Files that hold secrets by name (`.env`, `.env.*`, `.netrc`,
//!   `.npmrc`, `.pypirc`, `*.pem`, `*.key`, `id_*`, `.git/config`) are
//!   left out of listings and refused ([`denied`]).

use std::path::{Component, Path, PathBuf};

/// The most of a file a read returns as text.
pub(crate) const MAX_READ_BYTES: u64 = 1 << 20;

/// The home folder, as this engine runs.
pub(crate) fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").filter(|h| !h.is_empty()).map(PathBuf::from)
}

fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Why the folder `root` can't be shared with another server, if it can't.
pub(crate) fn unshareable(root: &Path, data_dir: &Path, home: Option<&Path>) -> Option<String> {
    let root = canonical(root);
    let data = canonical(data_dir);
    if root.starts_with(&data) || data.starts_with(&root) {
        return Some(format!("{}: is, holds or sits inside Zend's data folder", root.display()));
    }
    let home = home.map(canonical)?;
    if home.starts_with(&root) {
        return Some(format!("{}: is or holds the home folder", root.display()));
    }
    let hidden = root
        .strip_prefix(&home)
        .ok()
        .and_then(|rest| rest.components().next())
        .is_some_and(|first| first.as_os_str().to_string_lossy().starts_with('.'));
    hidden.then(|| format!("{}: inside a hidden folder of the home folder (keys and settings)", root.display()))
}

/// Whether `relative` (from a project's root) names a file that holds
/// secrets by its name.
pub(crate) fn denied(relative: &Path) -> bool {
    let names: Vec<String> = relative
        .components()
        .filter_map(|c| match c {
            Component::Normal(name) => Some(name.to_string_lossy().to_ascii_lowercase()),
            _ => None,
        })
        .collect();
    let Some(last) = names.last() else { return false };
    let last = last.as_str();
    matches!(last, ".env" | ".netrc" | ".npmrc" | ".pypirc")
        || last.starts_with(".env.")
        || last.ends_with(".pem")
        || last.ends_with(".key")
        || last.starts_with("id_")
        || (last == "config" && names.len() >= 2 && names[names.len() - 2] == ".git")
}

/// `relative` as a path from a root: only names (and `.`), never `/` or `..`.
pub(crate) fn from_root(relative: &str) -> Result<&Path, String> {
    let rel = Path::new(relative);
    if rel.components().any(|c| !matches!(c, Component::Normal(_) | Component::CurDir)) {
        return Err(format!("{relative}: a path from the project's root (no leading /, no ..)"));
    }
    Ok(rel)
}

/// One entry of a listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Entry {
    pub name: String,
    pub kind: &'static str,
    pub size: Option<u64>,
}

/// A read: the file's size, and its text when it's UTF-8 and not over
/// [`MAX_READ_BYTES`].
#[derive(Debug)]
pub(crate) struct Read {
    pub size: u64,
    pub text: Option<String>,
}

#[cfg(unix)]
mod imp {
    use std::io::Read as _;
    use std::os::fd::OwnedFd;
    use std::path::{Component, Path};

    use rustix::fs::{AtFlags, Dir, FileType, Mode, OFlags};
    use rustix::io::Errno;

    fn why(relative: &Path, err: Errno) -> String {
        let shown = relative.display();
        match err {
            Errno::LOOP | Errno::XDEV => format!("{shown}: outside the project, or through a link (links aren't followed)"),
            Errno::NOENT => format!("{shown}: no such file"),
            Errno::NOTDIR => format!("{shown}: not a folder"),
            other => format!("{shown}: {}", std::io::Error::from(other)),
        }
    }

    fn open_root(root: &Path) -> Result<OwnedFd, String> {
        rustix::fs::open(root, OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC, Mode::empty())
            .map_err(|e| format!("the project's folder: {}", std::io::Error::from(e)))
    }

    /// `relative` under the directory `dir`, one component at a time,
    /// following no link.
    pub(super) fn walk(dir: &OwnedFd, relative: &Path, flags: OFlags) -> Result<OwnedFd, Errno> {
        let parts: Vec<_> = relative
            .components()
            .filter_map(|c| match c {
                Component::Normal(name) => Some(name),
                _ => None,
            })
            .collect();
        let Some((last, parents)) = parts.split_last() else {
            return rustix::fs::openat(dir, ".", flags | OFlags::CLOEXEC, Mode::empty());
        };
        let mut held: Option<OwnedFd> = None;
        for part in parents {
            let base = held.as_ref().unwrap_or(dir);
            let next = rustix::fs::openat(
                base,
                *part,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )?;
            held = Some(next);
        }
        rustix::fs::openat(held.as_ref().unwrap_or(dir), *last, flags | OFlags::NOFOLLOW | OFlags::CLOEXEC, Mode::empty())
    }

    /// `relative` beneath `dir`: `openat2` where the kernel has it.
    pub(super) fn beneath(dir: &OwnedFd, relative: &Path, flags: OFlags) -> Result<OwnedFd, Errno> {
        #[cfg(any(target_os = "linux", target_os = "android"))]
        {
            use rustix::fs::ResolveFlags;
            let path = if relative.as_os_str().is_empty() { Path::new(".") } else { relative };
            match rustix::fs::openat2(
                dir,
                path,
                flags | OFlags::CLOEXEC,
                Mode::empty(),
                ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
            ) {
                Err(Errno::NOSYS) => {}
                done => return done,
            }
        }
        walk(dir, relative, flags)
    }

    /// Where `fd` is, when the system says (Linux).
    fn located(fd: &OwnedFd) -> Option<std::path::PathBuf> {
        #[cfg(any(target_os = "linux", target_os = "android"))]
        {
            use std::os::fd::AsRawFd as _;
            std::fs::read_link(format!("/proc/self/fd/{}", fd.as_raw_fd())).ok()
        }
        #[cfg(not(any(target_os = "linux", target_os = "android")))]
        {
            let _ = fd;
            None
        }
    }

    fn not_in_data(fd: &OwnedFd, relative: &Path, data_dir: &Path) -> Result<(), String> {
        let data = super::canonical(data_dir);
        match located(fd) {
            Some(at) if at.starts_with(&data) => Err(format!("{}: inside Zend's data folder", relative.display())),
            _ => Ok(()),
        }
    }

    pub(super) fn read(root: &Path, relative: &Path, data_dir: &Path) -> Result<super::Read, String> {
        let dir = open_root(root)?;
        let fd = beneath(&dir, relative, OFlags::RDONLY | OFlags::NONBLOCK | OFlags::NOCTTY).map_err(|e| why(relative, e))?;
        let stat = rustix::fs::fstat(&fd).map_err(|e| why(relative, e))?;
        if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile {
            return Err(format!("{}: not a file", relative.display()));
        }
        if stat.st_nlink > 1 {
            return Err(format!("{}: has other names (a hard link): not read", relative.display()));
        }
        not_in_data(&fd, relative, data_dir)?;
        let size = u64::try_from(stat.st_size).unwrap_or(0);
        let text = if size <= super::MAX_READ_BYTES {
            let mut bytes = Vec::new();
            std::fs::File::from(fd)
                .take(super::MAX_READ_BYTES)
                .read_to_end(&mut bytes)
                .map_err(|e| format!("{}: {e}", relative.display()))?;
            String::from_utf8(bytes).ok()
        } else {
            None
        };
        Ok(super::Read { size, text })
    }

    pub(super) fn list(root: &Path, relative: &Path, data_dir: &Path) -> Result<Vec<super::Entry>, String> {
        let dir = open_root(root)?;
        let fd = beneath(&dir, relative, OFlags::RDONLY | OFlags::DIRECTORY).map_err(|e| why(relative, e))?;
        not_in_data(&fd, relative, data_dir)?;
        let mut entries = Vec::new();
        for entry in Dir::read_from(&fd).map_err(|e| why(relative, e))? {
            let Ok(entry) = entry else { continue };
            let name = entry.file_name();
            let bytes = name.to_bytes();
            if bytes == b"." || bytes == b".." {
                continue;
            }
            let shown = name.to_string_lossy().into_owned();
            if super::denied(&relative.join(&shown)) {
                continue;
            }
            let Ok(stat) = rustix::fs::statat(&fd, name, AtFlags::SYMLINK_NOFOLLOW) else { continue };
            let (kind, size) = match FileType::from_raw_mode(stat.st_mode) {
                FileType::Symlink => ("link", None),
                FileType::Directory => ("dir", None),
                FileType::RegularFile => ("file", u64::try_from(stat.st_size).ok()),
                _ => ("other", None),
            };
            entries.push(super::Entry { name: shown, kind, size });
        }
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(entries)
    }
}

/// The file `relative` of the project at `root` (canonical), read safely.
pub(crate) fn read(root: &Path, relative: &str, data_dir: &Path) -> Result<Read, String> {
    let rel = from_root(relative)?;
    if denied(rel) {
        return Err(format!("{relative}: holds secrets by its name: never shared"));
    }
    #[cfg(unix)]
    {
        imp::read(root, rel, data_dir)
    }
    #[cfg(not(unix))]
    {
        let _ = (root, data_dir);
        Err("Sharing a project's files isn't on this system yet".into())
    }
}

/// The folder `relative` of the project at `root` (canonical), listed safely.
pub(crate) fn list(root: &Path, relative: &str, data_dir: &Path) -> Result<Vec<Entry>, String> {
    let rel = from_root(relative)?;
    #[cfg(unix)]
    {
        imp::list(root, rel, data_dir)
    }
    #[cfg(not(unix))]
    {
        let _ = (root, rel, data_dir);
        Err("Sharing a project's files isn't on this system yet".into())
    }
}

#[cfg(test)]
#[path = "peer_files_tests.rs"]
mod tests;
