//! Where Ideas & Intents keeps what it holds: one folder on this device
//! (`<data>/ideas`), one `<id>.json` an idea, and a memo's audio beside it
//! as `<id>.wav`. Each file is written whole to a temporary name, synced,
//! then renamed into place, so a crash leaves the old file or the new one,
//! never half of one. A memo's audio is in place before its record names it.
use serde::{Deserialize, Serialize};
use std::{
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use super::wav;

/// Where a memo's transcript stands.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Transcript {
    /// Saved, not transcribed yet (or the app quit while it was).
    Pending,
    /// Transcribed (`text` may be empty: nothing was heard).
    Done,
    /// The speech model isn't on this device.
    NoModel,
    /// Transcription failed (`error` says why); the audio is kept.
    Failed,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Idea {
    pub id: String,
    /// When it was saved, in milliseconds since the epoch.
    pub created: i64,
    /// A typed idea's words, or a memo's transcript.
    #[serde(default)]
    pub text: String,
    /// A memo's audio file, beside its record.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seconds: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcript: Option<Transcript>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

pub struct Store {
    dir: PathBuf,
}

/// An id is ours (made by `new_id`) and names nothing outside the folder.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 40 && id.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-')
}

fn new_id(created: i64) -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let salt = (std::process::id() as u64) << 16 ^ n;
    format!("{created:013x}-{:08x}", salt & 0xffff_ffff)
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

/// `bytes` at `path`, whole or not at all.
fn write_whole(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("idea");
    let next = path.with_file_name(format!(".{name}.{}.part", std::process::id()));
    let result = (|| {
        let mut file = std::fs::File::create(&next)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&next, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&next);
    }
    result
}

impl Store {
    pub fn open(dir: PathBuf) -> Result<Self, String> {
        std::fs::create_dir_all(&dir).map_err(|e| format!("Couldn't open {}: {e}", dir.display()))?;
        Ok(Self { dir })
    }

    fn record(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.json"))
    }

    /// Every idea, newest first. A record that can't be read is skipped,
    /// never deleted.
    pub fn list(&self) -> Vec<Idea> {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        let mut ideas: Vec<Idea> = entries
            .flatten()
            .filter(|e| {
                let name = e.file_name();
                let name = name.to_string_lossy();
                !name.starts_with('.') && name.ends_with(".json")
            })
            .filter_map(|e| serde_json::from_slice::<Idea>(&std::fs::read(e.path()).ok()?).ok())
            .filter(|i| valid_id(&i.id))
            .collect();
        ideas.sort_by(|a, b| b.created.cmp(&a.created).then_with(|| b.id.cmp(&a.id)));
        ideas
    }

    pub fn get(&self, id: &str) -> Option<Idea> {
        if !valid_id(id) {
            return None;
        }
        serde_json::from_slice(&std::fs::read(self.record(id)).ok()?).ok()
    }

    pub fn put(&self, idea: &Idea) -> Result<(), String> {
        if !valid_id(&idea.id) {
            return Err("Not an idea".into());
        }
        let bytes = serde_json::to_vec_pretty(idea).map_err(|e| e.to_string())?;
        write_whole(&self.record(&idea.id), &bytes).map_err(|e| format!("Couldn't save the idea: {e}"))
    }

    pub fn add_text(&self, text: &str) -> Result<Idea, String> {
        let text = text.trim();
        if text.is_empty() {
            return Err("Write something first".into());
        }
        let created = now_ms();
        let idea = Idea { id: new_id(created), created, text: text.to_owned(), audio: None, seconds: None, transcript: None, error: None };
        self.put(&idea)?;
        Ok(idea)
    }

    /// A memo: its audio on disk first, then its record (`Pending`). Only
    /// once both are in place is there a memo to transcribe.
    pub fn add_memo(&self, samples: &[f32], rate: u32) -> Result<Idea, String> {
        let created = now_ms();
        let id = new_id(created);
        let audio = format!("{id}.wav");
        write_whole(&self.dir.join(&audio), &wav::encode(samples, rate))
            .map_err(|e| format!("Couldn't save the recording: {e}"))?;
        let idea = Idea {
            id,
            created,
            text: String::new(),
            audio: Some(audio),
            seconds: Some((samples.len() as f64 / rate as f64 * 10.0).round() / 10.0),
            transcript: Some(Transcript::Pending),
            error: None,
        };
        self.put(&idea)?;
        Ok(idea)
    }

    /// A memo's audio, as recorded.
    pub fn audio(&self, id: &str) -> Result<(Vec<f32>, u32), String> {
        let idea = self.get(id).ok_or("This idea is gone")?;
        let file = idea.audio.ok_or("This idea has no recording")?;
        let bytes = std::fs::read(self.dir.join(file)).map_err(|e| format!("Couldn't read the recording: {e}"))?;
        wav::decode(&bytes)
    }

    /// The idea and its audio. The record goes first: a crash between the
    /// two leaves a stray file, never a memo without its audio.
    pub fn delete(&self, id: &str) -> Result<(), String> {
        let idea = self.get(id).ok_or("This idea is gone")?;
        std::fs::remove_file(self.record(id)).map_err(|e| format!("Couldn't delete the idea: {e}"))?;
        if let Some(file) = idea.audio.filter(|f| f.strip_suffix(".wav").is_some_and(valid_id)) {
            let _ = std::fs::remove_file(self.dir.join(file));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_ideas_come_back_newest_first_from_a_new_store() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(tmp.path().join("ideas")).unwrap();
        let first = store.add_text("  first thought ").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(3));
        let second = store.add_text("second").unwrap();
        assert!(store.add_text("   ").is_err());
        let again = Store::open(tmp.path().join("ideas")).unwrap();
        let listed = again.list();
        assert_eq!(listed.iter().map(|i| i.text.as_str()).collect::<Vec<_>>(), ["second", "first thought"]);
        assert_eq!(listed[0].id, second.id);
        assert_eq!(listed[1].id, first.id);
        assert!(listed.iter().all(|i| i.audio.is_none()));
    }

    #[test]
    fn a_memo_is_audio_first_then_a_pending_record() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(tmp.path().to_path_buf()).unwrap();
        let samples = vec![0.25f32; 8_000];
        let memo = store.add_memo(&samples, 16_000).unwrap();
        assert_eq!(memo.transcript, Some(Transcript::Pending));
        assert_eq!(memo.seconds, Some(0.5));
        assert!(tmp.path().join(format!("{}.wav", memo.id)).is_file());
        let (back, rate) = store.audio(&memo.id).unwrap();
        assert_eq!((back.len(), rate), (8_000, 16_000));
        // No temporary file left behind.
        let names: Vec<String> = std::fs::read_dir(tmp.path()).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        assert!(names.iter().all(|n| !n.ends_with(".part")), "{names:?}");
        store.delete(&memo.id).unwrap();
        assert!(store.list().is_empty());
        assert!(!tmp.path().join(format!("{}.wav", memo.id)).exists());
    }

    #[test]
    fn ids_never_leave_the_folder_and_broken_records_are_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(tmp.path().to_path_buf()).unwrap();
        assert!(!valid_id("../x"));
        assert!(!valid_id(""));
        assert!(store.get("../../etc/passwd").is_none());
        assert!(store.delete("../x").is_err());
        std::fs::write(tmp.path().join("broken.json"), b"{not json").unwrap();
        store.add_text("kept").unwrap();
        assert_eq!(store.list().len(), 1);
        assert!(tmp.path().join("broken.json").exists());
    }
}
