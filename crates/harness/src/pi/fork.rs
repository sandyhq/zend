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

/// The session id a session file's header names.
pub(super) fn header_id(path: &Path) -> Option<String> {
    use std::io::BufRead;
    let mut line = String::new();
    std::io::BufReader::new(std::fs::File::open(path).ok()?)
        .take(64 * 1024)
        .read_line(&mut line)
        .ok()?;
    let header: Value = serde_json::from_str(&line).ok()?;
    (header["type"] == "session").then(|| header["id"].as_str().map(str::to_owned))?
}

/// Name `source` as the session `file` came from (its header's
/// `parentSession`): the copy was cut from an intermediate one. Written
/// whole and renamed over, before any Pi has it open.
pub(super) fn set_parent(file: &Path, source: &Path) -> std::io::Result<()> {
    let text = std::fs::read_to_string(file)?;
    let (head, rest) = text.split_once('\n').unwrap_or((&text, ""));
    let mut header: Value = serde_json::from_str(head)?;
    if header["type"] != "session" {
        return Err(std::io::Error::other("the session file has no header"));
    }
    header["parentSession"] = Value::from(source.display().to_string());
    let temp = file.with_extension("jsonl.tmp");
    std::fs::write(&temp, format!("{header}\n{rest}"))?;
    std::fs::rename(&temp, file).inspect_err(|_| {
        let _ = std::fs::remove_file(&temp);
    })
}

/// Removes the files it holds when dropped: the source's copy a sandboxed
/// fork reads, the intermediate fork.
#[derive(Default)]
pub(super) struct Scratch(pub Vec<PathBuf>);
impl Drop for Scratch {
    fn drop(&mut self) {
        for file in &self.0 {
            let _ = std::fs::remove_file(file);
        }
    }
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
    fn the_header_names_the_session() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("s.jsonl");
        std::fs::write(&file, "{\"type\":\"session\",\"id\":\"s1\"}\n{\"id\":\"e\"}\n").unwrap();
        assert_eq!(header_id(&file).as_deref(), Some("s1"));
        std::fs::write(&file, "-----BEGIN KEY-----\n").unwrap();
        assert_eq!(header_id(&file), None);
        std::fs::write(&file, "{\"type\":\"message\",\"id\":\"s1\"}\n").unwrap();
        assert_eq!(header_id(&file), None);
    }

    #[test]
    fn the_parent_is_set_in_the_header_and_the_rest_kept() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("b.jsonl");
        std::fs::write(&file, "{\"type\":\"session\",\"id\":\"b\",\"parentSession\":\"/a\"}\n{\"id\":\"e\"}\n").unwrap();
        set_parent(&file, Path::new("/src.jsonl")).unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        let mut lines = text.lines();
        let header: Value = serde_json::from_str(lines.next().unwrap()).unwrap();
        assert_eq!(header["parentSession"], "/src.jsonl");
        assert_eq!(header["id"], "b");
        assert_eq!(lines.next(), Some("{\"id\":\"e\"}"));
    }
}
