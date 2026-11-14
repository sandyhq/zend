//! Each plugin's log on this device: what its window part printed and the
//! errors it hit there (with their tracebacks, `PluginLog` from the app),
//! and its service's prints, errors and comings and goings. One place for
//! both, so an agent reads one truth (`PluginLogs`, `plugin_status`'s
//! `logs`, `zend plugin logs`).
//!
//! The last [`KEEP`] lines of each are held in memory and appended to
//! `<data>/logs/plugins/<id>.log`, which moves to `<id>.log.old` past
//! [`FILE_MAX`]. A plugin may print what it read from chats: the log stays
//! on this device, never synced, and each line is cut at [`LINE_MAX`].

use std::collections::{HashMap, VecDeque};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::Serialize;
use tokio::sync::broadcast;

/// Lines held per plugin.
pub const KEEP: usize = 500;
/// A log file's size before it moves to `.old`.
pub const FILE_MAX: u64 = 1024 * 1024;
/// A line's text at most (a traceback included), in bytes.
pub const LINE_MAX: usize = 16 * 1024;
/// Lines one `PluginLog` brings at most (the rest are dropped, and said).
pub const BATCH_MAX: usize = 200;
/// Where the files go, under the data dir.
pub const LOGS_DIR: &str = "logs/plugins";

/// One line of a plugin's log.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Line {
    /// Rises by one per line, across plugins: `since` reads on from it.
    pub seq: u64,
    /// When, RFC 3339 in this device's time zone (`2026-11-14T18:04:00.123+01:00`).
    pub at: String,
    /// `window` or `service`.
    pub source: String,
    /// `print`, `error` or `info` (Zend's own word about it).
    pub level: String,
    pub text: String,
}

pub struct Logs {
    dir: PathBuf,
    state: Mutex<State>,
    tx: broadcast::Sender<(String, Line)>,
}

#[derive(Default)]
struct State {
    seq: u64,
    by: HashMap<String, VecDeque<Line>>,
}

/// Whether `id` can name a log file: a plugin id's letters only.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
        && !id.starts_with('-')
}

/// `level` as one of the words a line takes (anything else is `print`).
pub fn level(word: &str) -> &'static str {
    match word {
        "error" => "error",
        "info" => "info",
        _ => "print",
    }
}

impl Logs {
    /// Logs whose files go in `data_dir/logs/plugins`.
    pub fn new(data_dir: &Path) -> Self {
        let (tx, _) = broadcast::channel(256);
        Self {
            dir: data_dir.join(LOGS_DIR),
            state: Mutex::new(State::default()),
            tx,
        }
    }

    /// Plugin `id`'s log file (there once it logged something).
    pub fn file(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.log"))
    }

    /// Add a line to `id`'s log. Nothing for an id that can't be a plugin's.
    pub fn push(&self, id: &str, source: &str, level: &str, text: &str) -> Option<Line> {
        if !valid_id(id) {
            return None;
        }
        let text = cut(text.trim_end());
        let line = {
            let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
            st.seq += 1;
            let line = Line {
                seq: st.seq,
                at: chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, false),
                source: source.to_owned(),
                level: self::level(level).to_owned(),
                text,
            };
            let ring = st.by.entry(id.to_owned()).or_default();
            if ring.len() >= KEEP {
                ring.pop_front();
            }
            ring.push_back(line.clone());
            line
        };
        if let Err(error) = self.append(id, &line) {
            tracing::debug!(plugin = %id, %error, "plugins: a log line didn't reach its file");
        }
        let _ = self.tx.send((id.to_owned(), line.clone()));
        Some(line)
    }

    /// `id`'s lines after `since` (all held without), the last `limit`,
    /// oldest first.
    pub fn lines(&self, id: &str, since: Option<u64>, limit: usize) -> Vec<Line> {
        let st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let Some(ring) = st.by.get(id) else { return Vec::new() };
        let newer: Vec<&Line> = ring.iter().filter(|l| since.is_none_or(|s| l.seq > s)).collect();
        let skip = newer.len().saturating_sub(limit);
        newer.into_iter().skip(skip).cloned().collect()
    }

    /// Every line as it's logged, with its plugin.
    pub fn subscribe(&self) -> broadcast::Receiver<(String, Line)> {
        self.tx.subscribe()
    }

    /// Forget `id`'s lines and files (the plugin was removed).
    pub fn forget(&self, id: &str) {
        if !valid_id(id) {
            return;
        }
        self.state.lock().unwrap_or_else(|e| e.into_inner()).by.remove(id);
        let file = self.file(id);
        let _ = std::fs::remove_file(old(&file));
        let _ = std::fs::remove_file(file);
    }

    fn append(&self, id: &str, line: &Line) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        let file = self.file(id);
        if std::fs::metadata(&file).is_ok_and(|m| m.len() > FILE_MAX) {
            std::fs::rename(&file, old(&file))?;
        }
        let mut out = std::fs::OpenOptions::new().create(true).append(true).open(&file)?;
        out.write_all(file_line(line).as_bytes())
    }
}

fn old(file: &Path) -> PathBuf {
    let mut name = file.as_os_str().to_owned();
    name.push(".old");
    PathBuf::from(name)
}

/// `text` cut at [`LINE_MAX`] bytes, on a character's edge.
fn cut(text: &str) -> String {
    if text.len() <= LINE_MAX {
        return text.to_owned();
    }
    let mut end = LINE_MAX;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{} … (cut)", &text[..end])
}

/// `2026-11-14T18:04:00.123+01:00 window error kaputt`, a traceback's
/// lines after it indented.
pub fn file_line(line: &Line) -> String {
    let mut out = format!("{} {} {}", line.at, line.source, line.level);
    for (i, part) in line.text.lines().enumerate() {
        out.push_str(if i == 0 { " " } else { "\n    " });
        out.push_str(part);
    }
    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_last_lines_of_each_and_writes_them_down() {
        let dir = tempfile::tempdir().unwrap();
        let logs = Logs::new(dir.path());
        for i in 0..KEEP + 3 {
            logs.push("focus", "window", "print", &format!("line {i}"));
        }
        logs.push("other", "service", "error", "boom\nstack traceback:\n  init.luau:3");
        let lines = logs.lines("focus", None, KEEP + 10);
        assert_eq!(lines.len(), KEEP);
        assert_eq!(lines[0].text, "line 3");
        assert_eq!(logs.lines("focus", None, 2).iter().map(|l| l.text.as_str()).collect::<Vec<_>>(), ["line 501", "line 502"]);
        let since = lines[KEEP - 2].seq;
        assert_eq!(logs.lines("focus", Some(since), 50).len(), 1);
        let file = std::fs::read_to_string(logs.file("other")).unwrap();
        assert!(file.contains(" service error boom\n    stack traceback:\n      init.luau:3\n"), "{file}");
        assert_eq!(std::fs::read_to_string(logs.file("focus")).unwrap().lines().count(), KEEP + 3);
    }

    #[test]
    fn refuses_ids_that_arent_a_plugins_and_cuts_long_lines() {
        let dir = tempfile::tempdir().unwrap();
        let logs = Logs::new(dir.path());
        assert!(logs.push("../evil", "window", "print", "x").is_none());
        assert!(logs.push("", "window", "print", "x").is_none());
        let long = "é".repeat(LINE_MAX);
        let line = logs.push("focus", "window", "weird", &long).unwrap();
        assert_eq!(line.level, "print");
        assert!(line.text.len() <= LINE_MAX + 12 && line.text.ends_with("(cut)"));
        logs.forget("focus");
        assert!(logs.lines("focus", None, 10).is_empty());
        assert!(!logs.file("focus").exists());
    }

    #[test]
    fn a_full_file_moves_aside() {
        let dir = tempfile::tempdir().unwrap();
        let logs = Logs::new(dir.path());
        std::fs::create_dir_all(dir.path().join(LOGS_DIR)).unwrap();
        std::fs::write(logs.file("focus"), vec![b'x'; FILE_MAX as usize + 1]).unwrap();
        logs.push("focus", "window", "print", "hallo");
        assert!(old(&logs.file("focus")).exists());
        assert!(std::fs::read_to_string(logs.file("focus")).unwrap().ends_with("window print hallo\n"));
    }
}
