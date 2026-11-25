//! Which chats here a paired server created ([`crate::peer_link`]), and which
//! one: what it may run and read of them under a `sessions.run:<project>`
//! grant. Kept on this host only, beside the run journal (one
//! `{chat}.peer` file each, holding the chat's id and the server's key):
//! never in the synced chat row, which another device could rewrite.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Whether a paired server may name a new chat `id`: letters, digits, dashes
/// and underscores (a UUID), so no two ids share a record's file.
pub fn valid_chat_id(id: &str) -> bool {
    (1..=128).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Record {
    chat_id: String,
    peer: String,
}

/// The records, in `dir` (the run journal's folder).
#[derive(Clone, Debug)]
pub struct PeerChats {
    dir: PathBuf,
}

impl PeerChats {
    pub fn new(dir: &Path) -> Self {
        Self { dir: dir.to_path_buf() }
    }

    fn path(&self, chat_id: &str) -> Option<PathBuf> {
        valid_chat_id(chat_id).then(|| self.dir.join(format!("{chat_id}.peer")))
    }

    /// Keep that the server holding `peer` created `chat_id`.
    pub fn note(&self, chat_id: &str, peer: &str) -> std::io::Result<()> {
        let path = self.path(chat_id).ok_or_else(|| std::io::Error::other("not a chat id a server may name"))?;
        std::fs::create_dir_all(&self.dir)?;
        let record = Record { chat_id: chat_id.to_string(), peer: peer.to_string() };
        let tmp = path.with_extension("peer.tmp");
        std::fs::write(&tmp, serde_json::to_vec(&record).map_err(std::io::Error::other)?)?;
        std::fs::rename(&tmp, &path)
    }

    /// The key of the server that created `chat_id`, if one did.
    pub fn peer(&self, chat_id: &str) -> Option<String> {
        let raw = std::fs::read(self.path(chat_id)?).ok()?;
        let record: Record = serde_json::from_slice(&raw).ok()?;
        (record.chat_id == chat_id).then_some(record.peer)
    }

    /// Every chat a server created here, with that server's key.
    pub fn list(&self) -> Vec<(String, String)> {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        entries
            .flatten()
            .filter_map(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                let chat = name.strip_suffix(".peer")?.to_string();
                let peer = self.peer(&chat)?;
                Some((chat, peer))
            })
            .collect()
    }

    /// Drop the record (its chat was never made).
    pub fn forget(&self, chat_id: &str) {
        if let Some(path) = self.path(chat_id) {
            let _ = std::fs::remove_file(path);
        }
    }
}
