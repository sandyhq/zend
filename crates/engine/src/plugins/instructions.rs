//! What plugins tell the agents (`session.instructions`): text a chat's
//! agent reads in full in its system prompt, a section per plugin
//! (`plugin-<id>`), never through MCP instructions (an agent under codemode
//! sees those only shortened).
//!
//! **Kept per plugin, by key and scope.** A plugin sets an entry with
//! `zend.set_instructions { key?, chat?, project?, text }` (no text takes it
//! away), or answers `run.preparing` with `{ instructions }`, kept for that
//! chat under the key `run.preparing`. Without a chat or project an entry is
//! for every chat on this device, child sessions and subagents included; a
//! chat's entry is for its child sessions and subagents too. For one chat,
//! each of a plugin's keys takes its most specific entry (the chat's own,
//! else the nearest parent's, else its project's, else the device's):
//! another key adds to what the plugin says, the same key for a chat
//! replaces it there.
//!
//! **Only while allowed.** Entries are kept on this device
//! (`{data_dir}/plugin-instructions.json`) and delivered only while the
//! plugin is on and allowed `session.instructions`: taking the permission
//! back stops them from the chat's next turn, giving it back brings them
//! back. Each entry, and each plugin's section for a chat, is at most
//! [`MAX_BYTES`].
use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub const SESSION_INSTRUCTIONS: &str = "session.instructions";
/// One entry's text, and one plugin's section for a chat, at most.
pub const MAX_BYTES: usize = 8 * 1024;
/// A key's length, at most.
pub const MAX_KEY: usize = 64;
/// The key a `run.preparing` answer is kept under (for that chat).
pub const PREPARING_KEY: &str = "run.preparing";
/// The key of an entry set with none.
pub const DEFAULT_KEY: &str = "default";
const FILE: &str = "plugin-instructions.json";

/// Which chats an entry is for.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    /// Every chat on this device.
    Device,
    /// The chats of a project (a space's id).
    Project(String),
    /// One chat.
    Chat(String),
}

impl Scope {
    /// How specific it is for a chat of `lineage` (itself, then its parents)
    /// in `project`, or `None` when it isn't for that chat: the chat's own
    /// beats a nearer parent's beats a farther one's beats its project's
    /// beats the device's.
    fn rank(&self, lineage: &[&str], project: Option<&str>) -> Option<usize> {
        match self {
            Scope::Device => Some(0),
            Scope::Project(p) => (project == Some(p.as_str())).then_some(1),
            Scope::Chat(c) => lineage.iter().position(|id| id == c).map(|depth| 2 + lineage.len() - depth),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub plugin: String,
    pub key: String,
    pub scope: Scope,
    pub text: String,
    /// Epoch millis it was set.
    pub since: i64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct File {
    #[serde(default)]
    entries: Vec<Entry>,
}

/// Every plugin's entries on this device.
#[derive(Debug, Default)]
pub struct Store {
    path: Option<PathBuf>,
    entries: Vec<Entry>,
}

impl Store {
    /// The entries kept in `data_dir` (none when there's no file yet, or it
    /// can't be read: logged, and the next write replaces it).
    pub fn open(data_dir: &std::path::Path) -> Self {
        let path = data_dir.join(FILE);
        let entries = match std::fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<File>(&bytes) {
                Ok(file) => file.entries,
                Err(error) => {
                    tracing::warn!(%error, path = %path.display(), "plugins: unreadable instructions file");
                    Vec::new()
                }
            },
            Err(_) => Vec::new(),
        };
        Self {
            path: Some(path),
            entries,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Whether any entry is for a project (finding a chat's costs a lookup).
    pub fn has_projects(&self) -> bool {
        self.entries.iter().any(|e| matches!(e.scope, Scope::Project(_)))
    }

    /// Set `plugin`'s entry `key` at `scope`; `None` takes it away. Whether
    /// anything changed.
    pub fn set(&mut self, plugin: &str, key: &str, scope: Scope, text: Option<String>, now: i64) -> bool {
        let at = self
            .entries
            .iter()
            .position(|e| e.plugin == plugin && e.key == key && e.scope == scope);
        let changed = match (at, text) {
            (None, None) => false,
            (Some(i), None) => {
                self.entries.remove(i);
                true
            }
            (Some(i), Some(text)) if self.entries[i].text == text => false,
            (Some(i), Some(text)) => {
                self.entries[i].text = text;
                self.entries[i].since = now;
                true
            }
            (None, Some(text)) => {
                self.entries.push(Entry {
                    plugin: plugin.to_owned(),
                    key: key.to_owned(),
                    scope,
                    text,
                    since: now,
                });
                true
            }
        };
        if changed {
            self.save();
        }
        changed
    }

    /// Take away every entry of `plugin` (it was removed). Whether any went.
    pub fn forget(&mut self, plugin: &str) -> bool {
        self.retain(|p| p != plugin)
    }

    /// Keep only the entries of plugins `keep` says yes to. Whether any went.
    pub fn retain(&mut self, keep: impl Fn(&str) -> bool) -> bool {
        let before = self.entries.len();
        self.entries.retain(|e| keep(&e.plugin));
        let gone = self.entries.len() != before;
        if gone {
            self.save();
        }
        gone
    }

    /// `plugin`'s own entries.
    pub fn of(&self, plugin: &str) -> Vec<Entry> {
        let mut own: Vec<Entry> = self.entries.iter().filter(|e| e.plugin == plugin).cloned().collect();
        own.sort_by(|a, b| (&a.key, &a.scope).cmp(&(&b.key, &b.scope)));
        own
    }

    /// For a chat (`lineage`: its id, then its parent's, and so on) in
    /// `project`: each plugin's texts, by key, each key's most specific
    /// entry. Plugins and keys in order.
    pub fn for_chat(&self, lineage: &[&str], project: Option<&str>) -> BTreeMap<String, Vec<(String, String)>> {
        let mut best: BTreeMap<(String, String), (usize, &Entry)> = BTreeMap::new();
        for entry in &self.entries {
            let Some(rank) = entry.scope.rank(lineage, project) else { continue };
            let slot = best.entry((entry.plugin.clone(), entry.key.clone())).or_insert((rank, entry));
            if rank > slot.0 {
                *slot = (rank, entry);
            }
        }
        let mut out: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
        for ((plugin, key), (_, entry)) in best {
            if !entry.text.is_empty() {
                out.entry(plugin).or_default().push((key, entry.text.clone()));
            }
        }
        out
    }

    fn save(&self) {
        let Some(path) = &self.path else { return };
        let file = File {
            entries: self.entries.clone(),
        };
        let bytes = serde_json::to_vec_pretty(&file).expect("serializable instructions");
        let tmp = path.with_extension("json.tmp");
        let written = std::fs::write(&tmp, bytes).and_then(|()| std::fs::rename(&tmp, path));
        if let Err(error) = written {
            tracing::warn!(%error, path = %path.display(), "plugins: couldn't keep the instructions");
        }
    }
}

/// One plugin's section: its texts in key order, joined, up to [`MAX_BYTES`]
/// (whole texts only). What was left out, by key.
pub fn section(texts: &[(String, String)]) -> (String, Vec<String>) {
    let mut out = String::new();
    let mut left_out = Vec::new();
    for (key, text) in texts {
        let sep = if out.is_empty() { 0 } else { 2 };
        if out.len() + sep + text.len() > MAX_BYTES {
            left_out.push(key.clone());
            continue;
        }
        if sep > 0 {
            out.push_str("\n\n");
        }
        out.push_str(text);
    }
    (out, left_out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chat_takes_each_keys_most_specific_entry() {
        let mut s = Store::default();
        s.set("orch", "default", Scope::Device, Some("everyone".into()), 1);
        s.set("orch", "extra", Scope::Device, Some("also".into()), 1);
        s.set("orch", "default", Scope::Project("p1".into()), Some("project one".into()), 1);
        s.set("orch", "default", Scope::Chat("c2".into()), Some("chat two".into()), 1);
        s.set("orch", "extra", Scope::Chat("lead".into()), Some("from the lead".into()), 1);
        s.set("orch", "extra", Scope::Chat("child".into()), Some("the child's own".into()), 1);
        s.set("other", "default", Scope::Chat("c9".into()), Some("not yours".into()), 1);
        fn texts(s: &Store, lineage: &[&str], project: Option<&str>) -> Vec<(String, String)> {
            s.for_chat(lineage, project).remove("orch").unwrap_or_default()
        }
        let kv = |pairs: &[(&str, &str)]| pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<Vec<_>>();
        assert_eq!(texts(&s, &["c1"], None), kv(&[("default", "everyone"), ("extra", "also")]));
        assert_eq!(texts(&s, &["c1"], Some("p1")), kv(&[("default", "project one"), ("extra", "also")]));
        assert_eq!(texts(&s, &["c2"], Some("p1")), kv(&[("default", "chat two"), ("extra", "also")]));
        // A subagent hears its lead's entries; its own beat them.
        assert_eq!(texts(&s, &["sub", "lead"], None), kv(&[("default", "everyone"), ("extra", "from the lead")]));
        assert_eq!(texts(&s, &["child", "lead"], None), kv(&[("default", "everyone"), ("extra", "the child's own")]));
        assert_eq!(texts(&s, &["grandchild", "child", "lead"], None)[1].1, "the child's own", "the nearer parent wins");
        assert!(!s.for_chat(&["c1"], None).contains_key("other"));
        assert!(s.set("orch", "extra", Scope::Device, None, 2));
        assert!(!s.set("orch", "extra", Scope::Device, None, 2), "nothing to take away");
        assert_eq!(texts(&s, &["c1"], None), kv(&[("default", "everyone")]));
    }

    #[test]
    fn entries_are_kept_across_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = Store::open(dir.path());
        assert!(s.is_empty());
        s.set("zebra", "default", Scope::Device, Some("Antworte immer mit dem Wort ZEBRA am Ende".into()), 7);
        let again = Store::open(dir.path());
        assert_eq!(again.of("zebra").len(), 1);
        assert_eq!(again.of("zebra")[0].text, "Antworte immer mit dem Wort ZEBRA am Ende");
        assert_eq!(again.of("zebra")[0].scope, Scope::Device);
    }

    #[test]
    fn a_removed_plugins_entries_are_forgotten_on_disk_too() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = Store::open(dir.path());
        s.set("gone", "default", Scope::Device, Some("old rules".into()), 1);
        s.set("gone", "extra", Scope::Chat("c1".into()), Some("for c1".into()), 1);
        s.set("kept", "default", Scope::Device, Some("still here".into()), 1);
        assert!(s.forget("gone"));
        assert!(!s.forget("gone"), "nothing left to forget");
        let again = Store::open(dir.path());
        assert!(again.of("gone").is_empty(), "gone from plugin-instructions.json");
        assert_eq!(again.of("kept").len(), 1);
        let on_disk = std::fs::read_to_string(dir.path().join("plugin-instructions.json")).unwrap();
        assert!(!on_disk.contains("old rules") && on_disk.contains("still here"), "{on_disk}");
    }

    #[test]
    fn a_section_holds_whole_texts_up_to_the_limit() {
        // "first", a blank line and this come to 4 bytes over.
        let big = "x".repeat(MAX_BYTES - 3);
        let (text, left_out) = section(&[("a".into(), "first".into()), ("b".into(), big.clone()), ("c".into(), "tail".into())]);
        assert_eq!(text, "first\n\ntail", "the one too long is left out whole, the rest kept");
        assert_eq!(left_out, vec!["b".to_string()]);
        let (text, left_out) = section(&[("a".into(), "one".into()), ("b".into(), "two".into())]);
        assert_eq!(text, "one\n\ntwo");
        assert!(left_out.is_empty());
    }
}
