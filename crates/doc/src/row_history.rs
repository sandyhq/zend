//! Which transcript rows a version of a session doc holds, read from the
//! doc's ops rather than from its state.
//!
//! A Stop says what its sender saw with a doc version
//! ([`crate::CommandBasedOn::frontier`]). A row was in the transcript at
//! that version iff the op that gave the row its message id is in it and no
//! op that removed the row is. Neither can be told from the state: a row
//! removed since has no state left, and a row's map says when it was
//! created, not when it got its id (2026-11-17). So the ops are read, each
//! once: what the oplog gained since the last read.

use std::collections::{HashMap, HashSet};

use loro::{
    ContainerID, ContainerTrait, ContainerType, ID, JsonListOp, JsonMapOp, JsonOpContent, LoroDoc,
    LoroValue, VersionVector,
};

/// One write of a key of a row's map.
struct Write {
    op: ID,
    lamport: loro::Lamport,
    /// `None`: the key was removed, or set to something that isn't text.
    value: Option<String>,
}

/// A map that is, or was, an item of the messages list.
#[derive(Default)]
struct Row {
    /// Every write of its `id`.
    ids: Vec<Write>,
    /// Every write of its `continuationOf`.
    continues: Vec<Write>,
    /// The ops that took it off the list (more than one when devices
    /// removed it concurrently).
    removed: Vec<ID>,
}

/// The value a key has among `writes`, those `known` counts: the last
/// writer wins, as in the doc's map (by lamport, then peer).
fn value_of(writes: &[Write], known: impl Fn(&Write) -> bool) -> Option<&str> {
    writes
        .iter()
        .filter(|write| known(write))
        .max_by_key(|write| (write.lamport, write.op.peer))
        .and_then(|write| write.value.as_deref())
}

impl Row {
    /// The message id the row has in the transcript at `version`; `None`
    /// when it isn't in it: not given an id yet, or removed.
    fn id_at(&self, version: &VersionVector) -> Option<&str> {
        if self.removed.iter().any(|op| version.includes_id(*op)) {
            return None;
        }
        value_of(&self.ids, |write| version.includes_id(write.op))
    }
}

/// The rows of a doc's messages list as its ops tell them.
#[derive(Default)]
pub(crate) struct RowHistory {
    /// The oplog version read so far; `None` before the first read.
    read: Option<VersionVector>,
    rows: HashMap<ContainerID, Row>,
    /// The rows that were ever given a message id, by that id.
    by_id: HashMap<String, Vec<ContainerID>>,
}

impl RowHistory {
    /// Read the ops the oplog gained since the last read. The caller holds
    /// the doc's change lock: reading the ops commits what is pending.
    pub(crate) fn catch_up(&mut self, doc: &LoroDoc) {
        let now = doc.oplog_vv();
        if self.read.as_ref() == Some(&now) {
            return;
        }
        let first = self.read.is_none();
        let from = self.read.clone().unwrap_or_default();
        let Ok(json) = doc.export_json_updates_without_peer_compression(&from, &now) else {
            // Unread: the next call reads from the same version again.
            return;
        };
        let messages = ContainerID::new_root("messages", ContainerType::List);
        let ops = || {
            json.changes
                .iter()
                .flat_map(|change| change.ops.iter().map(move |op| (change, op)))
        };
        // The rows first, then what removed them, then their keys: a row's
        // ops come after the one that made it, in whatever order the
        // changes are listed.
        for (_, op) in ops() {
            if op.container != messages {
                continue;
            }
            let JsonOpContent::List(JsonListOp::Insert { value, .. }) = &op.content else {
                continue;
            };
            for item in value {
                if let LoroValue::Container(row) = item
                    && row.container_type() == ContainerType::Map
                {
                    self.rows.entry(row.clone()).or_default();
                }
            }
        }
        // A doc trimmed of its old history holds rows whose ops are gone:
        // such a row has had its id since the op that made it, as far as
        // this doc can tell.
        if first && doc.is_shallow() {
            self.read_trimmed_rows(doc);
        }
        for (change, op) in ops() {
            if op.container != messages {
                continue;
            }
            let JsonOpContent::List(JsonListOp::Delete { len, start_id, .. }) = &op.content else {
                continue;
            };
            // One op per removed item. `start_id` is the item with the
            // smallest counter; a span removed backwards (`len` < 0) takes
            // its items from the last one down.
            let count = len.unsigned_abs() as i32;
            for nth in 0..count {
                let item = if *len < 0 { count - 1 - nth } else { nth };
                let row = ContainerID::new_normal(
                    ID::new(start_id.peer, start_id.counter + item),
                    ContainerType::Map,
                );
                if let Some(row) = self.rows.get_mut(&row) {
                    row.removed.push(ID::new(change.id.peer, op.counter + nth));
                }
            }
        }
        for (change, op) in ops() {
            let JsonOpContent::Map(write) = &op.content else {
                continue;
            };
            let (key, value) = match write {
                JsonMapOp::Insert { key, value } => (key, value.as_string().map(|s| s.to_string())),
                JsonMapOp::Delete { key } => (key, None),
            };
            if key != "id" && key != "continuationOf" {
                continue;
            }
            let Some(row) = self.rows.get_mut(&op.container) else {
                continue;
            };
            let write = Write {
                op: ID::new(change.id.peer, op.counter),
                lamport: change.lamport + (op.counter - change.id.counter).max(0) as loro::Lamport,
                value,
            };
            if key == "id" {
                if let Some(id) = &write.value {
                    let rows = self.by_id.entry(id.clone()).or_default();
                    if !rows.contains(&op.container) {
                        rows.push(op.container.clone());
                    }
                }
                row.ids.push(write);
            } else {
                row.continues.push(write);
            }
        }
        self.read = Some(now);
    }

    /// The rows of the list that no op read made (the history that made
    /// them was trimmed away): each as it is now, written by the op that
    /// made its map.
    fn read_trimmed_rows(&mut self, doc: &LoroDoc) {
        let list = doc.get_list("messages");
        for at in 0..list.len() {
            let Some(loro::ValueOrContainer::Container(loro::Container::Map(map))) = list.get(at)
            else {
                continue;
            };
            let container = map.id();
            let ContainerID::Normal { peer, counter, .. } = container else {
                continue;
            };
            if self.rows.contains_key(&container) {
                continue;
            }
            let text = |key: &str| match map.get(key) {
                Some(loro::ValueOrContainer::Value(LoroValue::String(text))) => {
                    Some(text.to_string())
                }
                _ => None,
            };
            let written = |value: Option<String>| {
                value
                    .map(|value| Write {
                        op: ID::new(peer, counter),
                        lamport: 0,
                        value: Some(value),
                    })
                    .into_iter()
                    .collect::<Vec<_>>()
            };
            let id = text("id");
            if let Some(id) = &id {
                self.by_id
                    .entry(id.clone())
                    .or_default()
                    .push(container.clone());
            }
            self.rows.insert(
                container,
                Row {
                    ids: written(id),
                    continues: written(text("continuationOf")),
                    removed: Vec::new(),
                },
            );
        }
    }

    /// Whether the transcript at `version` had a row with the id
    /// `message_id`.
    pub(crate) fn holds(&self, version: &VersionVector, message_id: &str) -> bool {
        self.by_id.get(message_id).is_some_and(|rows| {
            rows.iter().any(|row| {
                self.rows
                    .get(row)
                    .is_some_and(|row| row.id_at(version) == Some(message_id))
            })
        })
    }

    /// The ids of the rows the transcript at `version` had.
    pub(crate) fn held(&self, version: &VersionVector) -> HashSet<String> {
        self.rows
            .values()
            .filter_map(|row| row.id_at(version).map(str::to_owned))
            .collect()
    }

    /// What the rows that were ever given the id `message_id` continue.
    pub(crate) fn continues(&self, message_id: &str) -> Vec<String> {
        self.by_id
            .get(message_id)
            .into_iter()
            .flatten()
            .filter_map(|row| value_of(&self.rows.get(row)?.continues, |_| true))
            .map(str::to_owned)
            .collect()
    }
}
