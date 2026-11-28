//! A fork's first turn on a copy of its source's Pi session
//! ([`zend_proto::NativeFork`]): `pi --fork <source>` copies the session, and
//! RPC `fork {entryId}` cuts that copy before the first prompt after the
//! source turn the fork copies through. The source file is only read.
use serde_json::Value;
use std::{
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

/// Where the copy ends, from the turn's last entry (`leaf`).
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Cut {
    /// Nothing came after the turn: the whole copy is the fork.
    End,
    /// The next turn's prompt: the fork holds what came before it.
    Before(String),
}

/// Entries Pi keeps beside the conversation, never sent to the model: they
/// may follow a turn's last entry without changing what the fork knows.
const BESIDE: &[&str] = &["model_change", "thinking_level_change", "custom", "label", "session_info"];

/// Where to cut `entries` (Pi's `get_entries`, in file order) so the fork
/// holds the session as turn `leaf` left it. Walks from `leaf` down its only
/// line of children: entries beside the conversation are passed, the first
/// user message is the cut. Anything else (a branch, more conversation) means
/// `leaf` wasn't where a turn ended, and no cut is safe.
pub(super) fn cut(entries: &[Value], leaf: &str) -> Result<Cut, String> {
    if !entries.iter().any(|e| e["id"] == leaf) {
        return Err(format!("the source session has no entry {leaf}"));
    }
    let mut at = leaf.to_owned();
    loop {
        let mut children = entries.iter().filter(|e| e["parentId"] == at.as_str());
        let Some(child) = children.next() else {
            return Ok(Cut::End);
        };
        if children.next().is_some() {
            return Err(format!("the source session branches after {at}"));
        }
        let id = child["id"].as_str().unwrap_or_default().to_owned();
        let kind = child["type"].as_str().unwrap_or_default();
        if kind == "message" && child["message"]["role"] == "user" {
            return Ok(Cut::Before(id));
        }
        if !BESIDE.contains(&kind) || id.is_empty() {
            return Err(format!("the source session goes on after {leaf} without a prompt ({kind})"));
        }
        at = id;
    }
}

/// The id of a session file's last entry: where Pi's session stands (Pi
/// appends every entry, and Zend never moves its leaf elsewhere). Read from
/// the end, so a long session costs only its last line.
pub(super) fn last_entry(path: &Path) -> Option<String> {
    let mut file = std::fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let mut tail = Vec::new();
    let mut step = 16 * 1024u64;
    loop {
        let start = len.saturating_sub(step);
        file.seek(SeekFrom::Start(start)).ok()?;
        tail.clear();
        file.read_to_end(&mut tail).ok()?;
        let text = String::from_utf8_lossy(&tail);
        let mut lines = text.lines().rev().filter(|l| !l.trim().is_empty());
        let last = lines.next()?;
        // A whole line: something before it in this window, or the file's start.
        if start == 0 || lines.next().is_some() {
            let entry: Value = serde_json::from_str(last).ok()?;
            return (entry["type"] != "session")
                .then(|| entry["id"].as_str().map(str::to_owned))
                .flatten();
        }
        step *= 4;
    }
}

/// A fork's own folder while it is made: new (never one that was there),
/// private, inside the folder the fork is kept in. Pi gets a snapshot of the
/// source in it and writes its copies there; dropping it removes everything
/// in it, whatever Pi left half done.
pub(super) struct Staging(PathBuf);
impl Staging {
    pub(super) fn new(parent: &Path) -> std::io::Result<Self> {
        let path = parent.join(format!(".zend-fork-{}", uuid::Uuid::new_v4()));
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
        builder.create(&path)?;
        Ok(Self(path))
    }
    pub(super) fn path(&self) -> &Path {
        &self.0
    }
    /// Whether `file` names a file right in this folder.
    pub(super) fn holds(&self, file: &Path) -> bool {
        file.parent() == Some(self.0.as_path())
            && file.file_name().is_some_and(|name| name != "." && name != "..")
    }
}
impl Drop for Staging {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A regular file's bytes, through one handle, never through a symlink
/// (nor blocking on a pipe).
pub(super) fn read_file(path: &Path) -> std::io::Result<Vec<u8>> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::custom_flags(&mut options, libc::O_NOFOLLOW | libc::O_NONBLOCK);
    let mut file = options.open(path)?;
    if !file.metadata()?.is_file() {
        return Err(std::io::Error::other("not a regular file"));
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(bytes)
}

/// A session file's header and entries, in file order.
pub(super) fn parse(bytes: &[u8]) -> Result<(Value, Vec<Value>), String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "the session file isn't text".to_string())?;
    let mut lines = text.lines().filter(|l| !l.trim().is_empty());
    let header: Value = lines
        .next()
        .and_then(|l| serde_json::from_str(l).ok())
        .filter(|h: &Value| h["type"] == "session")
        .ok_or("the file isn't a Pi session")?;
    let entries = lines
        .map(serde_json::from_str)
        .collect::<Result<Vec<Value>, _>>()
        .map_err(|e| format!("the session file has a broken line: {e}"))?;
    Ok((header, entries))
}

/// Whether `entries` are what a fork through `leaf` holds: one line from
/// the first entry to `leaf`, each the child of the one before, then only
/// entries beside the conversation. Pi resumes a session at its last entry,
/// and builds its context from that entry's line: another branch anywhere
/// (`pi --fork` copies all of them) is not the fork.
pub(super) fn one_line_to(entries: &[Value], leaf: &str) -> Result<(), String> {
    let mut parent = Value::Null;
    let mut past_leaf = false;
    for entry in entries {
        if entry["parentId"] != parent {
            return Err("the copy holds another branch of the session".into());
        }
        if past_leaf && !BESIDE.contains(&entry["type"].as_str().unwrap_or_default()) {
            return Err(format!("the copy goes on after {leaf}"));
        }
        past_leaf |= entry["id"] == leaf;
        parent = entry["id"].clone();
    }
    if !past_leaf {
        return Err(format!("the copy has no entry {leaf}"));
    }
    Ok(())
}

/// The session file `bytes` as kept: its header names `source` as the
/// session it came from (`parentSession`), every line ended.
pub(super) fn with_parent(header: &Value, bytes: &[u8], source: &Path) -> Result<Vec<u8>, String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "the copy isn't text".to_string())?;
    let rest = text.split_once('\n').map_or("", |(_, rest)| rest);
    let mut header = header.clone();
    header["parentSession"] = Value::from(source.display().to_string());
    let mut out = format!("{header}\n{rest}");
    if !out.ends_with('\n') {
        out.push('\n');
    }
    Ok(out.into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn msg(id: &str, parent: Option<&str>, role: &str) -> Value {
        json!({"type":"message","id":id,"parentId":parent,"message":{"role":role}})
    }

    #[test]
    fn the_cut_is_the_next_prompt_past_entries_beside_the_conversation() {
        let entries = vec![
            msg("u1", None, "user"),
            msg("a1", Some("u1"), "assistant"),
            json!({"type":"model_change","id":"m","parentId":"a1"}),
            json!({"type":"custom","id":"c","parentId":"m"}),
            msg("u2", Some("c"), "user"),
            msg("a2", Some("u2"), "assistant"),
        ];
        assert_eq!(cut(&entries, "a1"), Ok(Cut::Before("u2".into())));
        assert_eq!(cut(&entries, "a2"), Ok(Cut::End));
        assert!(cut(&entries, "gone").is_err());
        // More conversation after the leaf: it wasn't a turn's end.
        assert!(cut(&entries, "u1").is_err());
        let mut branched = entries.clone();
        branched.push(msg("u2b", Some("c"), "user"));
        assert!(cut(&branched, "a1").is_err());
    }

    #[test]
    fn the_last_entry_is_read_from_the_end_even_past_a_long_line() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("s.jsonl");
        std::fs::write(&file, "{\"type\":\"session\",\"id\":\"s\"}\n").unwrap();
        assert_eq!(last_entry(&file), None, "a header alone is no entry");
        let long = "x".repeat(100_000);
        std::fs::write(
            &file,
            format!("{{\"type\":\"session\",\"id\":\"s\"}}\n{{\"id\":\"a\",\"t\":\"{long}\"}}\n{{\"id\":\"b\",\"t\":\"{long}\"}}\n"),
        )
        .unwrap();
        assert_eq!(last_entry(&file).as_deref(), Some("b"));
        assert_eq!(last_entry(&dir.path().join("none")), None);
    }

    #[test]
    fn only_one_line_to_the_leaf_is_the_fork() {
        let line = vec![
            msg("u1", None, "user"),
            msg("a1", Some("u1"), "assistant"),
            json!({"type":"model_change","id":"m","parentId":"a1"}),
        ];
        assert_eq!(one_line_to(&line, "a1"), Ok(()));
        assert!(one_line_to(&line, "u1").is_err(), "goes on after the leaf");
        assert!(one_line_to(&line, "gone").is_err());
        let mut branched = line.clone();
        branched.push(msg("u2b", Some("a1"), "user"));
        assert!(one_line_to(&branched, "a1").is_err(), "a later branch");
        assert!(one_line_to(&branched, "u2b").is_err(), "an earlier branch");
    }

    #[test]
    fn the_kept_copy_names_its_source_and_keeps_the_rest() {
        let bytes = b"{\"type\":\"session\",\"id\":\"b\",\"parentSession\":\"/a\"}\n{\"id\":\"e\"}";
        let (header, entries) = parse(bytes).unwrap();
        assert_eq!(entries.len(), 1);
        let text = String::from_utf8(with_parent(&header, bytes, Path::new("/src.jsonl")).unwrap()).unwrap();
        let mut lines = text.lines();
        let header: Value = serde_json::from_str(lines.next().unwrap()).unwrap();
        assert_eq!((header["parentSession"].as_str(), header["id"].as_str()), (Some("/src.jsonl"), Some("b")));
        assert_eq!(lines.next(), Some("{\"id\":\"e\"}"));
        assert!(text.ends_with('\n'));
        assert!(parse(b"-----BEGIN KEY-----\n").is_err());
        assert!(parse(b"{\"type\":\"message\",\"id\":\"s1\"}\n").is_err());
    }

    #[test]
    fn the_staging_folder_goes_with_all_it_holds() {
        let dir = tempfile::tempdir().unwrap();
        let staging = Staging::new(dir.path()).unwrap();
        let path = staging.path().to_path_buf();
        std::fs::write(path.join("half.jsonl"), "x").unwrap();
        assert!(staging.holds(&path.join("half.jsonl")));
        assert!(!staging.holds(&dir.path().join("half.jsonl")));
        assert!(!staging.holds(&path.join("sub/half.jsonl")));
        drop(staging);
        assert!(!path.exists());
    }
}
