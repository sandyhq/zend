//! Whether a fork can start on a native copy of its source's provider
//! session (plans/pi-native-fork, E4), decided as it is forked: the copy must
//! hold exactly what the fork copies, so every doubt is the copied history
//! as a prompt instead (`fork_history_prompt`).
use zend_doc::{MessageRole, NativeForkMark, SessionDoc, SessionMessageEntry};
use zend_proto::{Chat, HarnessId};

/// The source session a fork of `source` copying `entries[..copied]` (its
/// transcript, `doc` its doc) starts on a copy of, or why it can't.
pub(crate) fn mark(
    source: &Chat,
    doc: &SessionDoc,
    entries: &[SessionMessageEntry],
    copied: usize,
) -> Result<NativeForkMark, &'static str> {
    let pi = HarnessId::Pi.as_str();
    let harness = source
        .harness_session_harness
        .clone()
        .or_else(|| source.config.as_ref().map(|c| c.harness.as_str().to_owned()));
    if harness.as_deref() != Some(pi) {
        return Err("the source's session isn't Pi's");
    }
    if doc.provider_switched() {
        return Err("the source changed provider");
    }
    if doc.history_rewound() || doc.history_moved() {
        return Err("the source was rewound or moved");
    }
    let session = source
        .harness_session_id
        .as_deref()
        .filter(|id| !id.is_empty())
        .ok_or("the source has no session")?;
    let copied = &entries[..copied.min(entries.len())];
    // Above the source's own seam: what it was forked with, already in its
    // session (natively, or as the history its first prompt carried).
    let seam = format!("fork:{}", source.id);
    let own = copied
        .iter()
        .position(|entry| entry.id == seam)
        .map_or(0, |at| at + 1);
    let users: Vec<&str> = copied[own..]
        .iter()
        .filter(|entry| entry.role == MessageRole::User)
        .map(|entry| entry.id.as_str())
        .collect();
    let last = *users.last().ok_or("the fork copies no turn of the source's own")?;
    let marks: Vec<_> = doc
        .native_turns()
        .into_iter()
        .filter(|mark| mark.harness == pi && mark.session_id == session)
        .collect();
    let covered = |id: &str| marks.iter().any(|mark| mark.user_ids.iter().any(|u| u == id));
    if !users.iter().all(|id| covered(id)) {
        return Err("a copied turn ran outside the source's current session");
    }
    let turn = marks
        .iter()
        .rev()
        .find(|mark| mark.user_ids.iter().any(|u| u == last))
        .ok_or("the copied turn left no mark")?;
    // A steer the turn took after the fork point would be in the copy.
    if !turn.user_ids.iter().all(|u| copied.iter().any(|entry| &entry.id == u)) {
        return Err("the fork cuts inside a turn");
    }
    Ok(NativeForkMark {
        session_id: turn.session_id.clone(),
        session_file: turn.session_file.clone(),
        leaf: turn.leaf.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use zend_doc::{MessagePart, MessageStatus, NativeTurnMark};

    fn entry(id: &str, role: MessageRole) -> SessionMessageEntry {
        SessionMessageEntry {
            id: id.into(),
            role,
            parts: vec![MessagePart::Text { id: format!("{id}-t"), text: id.into() }],
            created_at: 0,
            device_id: "d".into(),
            status: Some(MessageStatus::Complete),
            continuation_of: None,
            duration_ms: None,
        }
    }

    fn turn(leaf: &str, users: &[&str]) -> NativeTurnMark {
        NativeTurnMark {
            harness: "pi".into(),
            session_id: "s1".into(),
            session_file: "/s1.jsonl".into(),
            leaf: leaf.into(),
            user_ids: users.iter().map(|u| u.to_string()).collect(),
        }
    }

    fn source() -> Chat {
        serde_json::from_value(serde_json::json!({
            "id": "src", "deviceId": "d", "title": null, "archived": false, "cwd": null,
            "branch": null, "checkoutId": null, "config": null, "lastMessagePreview": null,
            "lastMessageAt": null, "createdAt": "2026-11-28T00:00:00Z",
            "harnessSessionId": "s1", "harnessSessionHarness": "pi"
        }))
        .unwrap()
    }

    /// Three turns, each marked: u1 a1 | u2 a2 a2b (steer u2s) | u3 a3.
    fn chat() -> (SessionDoc, Vec<SessionMessageEntry>) {
        let doc = SessionDoc::init("src").unwrap();
        let entries = vec![
            entry("u1", MessageRole::User),
            entry("a1", MessageRole::Assistant),
            entry("u2", MessageRole::User),
            entry("a2", MessageRole::Assistant),
            entry("u2s", MessageRole::User),
            entry("a2b", MessageRole::Assistant),
            entry("u3", MessageRole::User),
            entry("a3", MessageRole::Assistant),
        ];
        for entry in &entries {
            doc.push_message(entry).unwrap();
        }
        doc.push_native_turn(&turn("l1", &["u1"])).unwrap();
        doc.push_native_turn(&turn("l2", &["u2", "u2s"])).unwrap();
        doc.push_native_turn(&turn("l3", &["u3"])).unwrap();
        (doc, entries)
    }

    #[test]
    fn a_fork_through_a_whole_marked_turn_starts_on_its_leaf() {
        let (doc, entries) = chat();
        // At the reply of turn 2 (through its steer's reply), or at the
        // prompt of turn 3 (copies up to it): turn 2's leaf.
        let at = mark(&source(), &doc, &entries, 6).unwrap();
        assert_eq!((at.leaf.as_str(), at.session_file.as_str()), ("l2", "/s1.jsonl"));
        // At the prompt of turn 2: turn 1's leaf.
        assert_eq!(mark(&source(), &doc, &entries, 2).unwrap().leaf, "l1");
        assert_eq!(mark(&source(), &doc, &entries, 8).unwrap().leaf, "l3");
    }

    #[test]
    fn every_doubt_falls_back() {
        let (doc, entries) = chat();
        // Cut between turn 2's prompt and its steer.
        assert!(mark(&source(), &doc, &entries, 4).is_err());
        // Nothing of the source's own copied.
        assert!(mark(&source(), &doc, &entries, 0).is_err());
        // Another provider's session, or none.
        let mut other = source();
        other.harness_session_harness = Some("claude-code".into());
        assert!(mark(&other, &doc, &entries, 8).is_err());
        let mut none = source();
        none.harness_session_id = Some(String::new());
        assert!(mark(&none, &doc, &entries, 8).is_err());
        // The source now runs another session than the marks name.
        let mut moved_on = source();
        moved_on.harness_session_id = Some("s2".into());
        assert!(mark(&moved_on, &doc, &entries, 8).is_err());
        // An old chat: no marks at all.
        let old = SessionDoc::init("src").unwrap();
        assert_eq!(mark(&source(), &old, &entries, 8), Err("a copied turn ran outside the source's current session"));
        // A turn without a mark (it ran before marks were kept).
        let partial = SessionDoc::init("src").unwrap();
        partial.push_native_turn(&turn("l3", &["u3"])).unwrap();
        assert!(mark(&source(), &partial, &entries, 8).is_err());
        // Switched, rewound, moved.
        let (switched, _) = chat();
        switched.mark_provider_switched().unwrap();
        assert!(mark(&source(), &switched, &entries, 8).is_err());
        let (moved, _) = chat();
        moved.mark_history_moved().unwrap();
        assert!(mark(&source(), &moved, &entries, 8).is_err());
        let (rewound, _) = chat();
        rewound.truncate_from("u3", &[]).unwrap();
        assert!(mark(&source(), &rewound, &entries, 6).is_err());
    }

    #[test]
    fn what_came_above_the_sources_own_seam_needs_no_mark() {
        let doc = SessionDoc::init("src").unwrap();
        let mut seam = entry("fork:src", MessageRole::System);
        seam.parts.clear();
        let entries = vec![
            entry("x1", MessageRole::User),
            entry("x2", MessageRole::Assistant),
            seam,
            entry("u1", MessageRole::User),
            entry("a1", MessageRole::Assistant),
        ];
        doc.push_native_turn(&turn("l1", &["u1"])).unwrap();
        assert_eq!(mark(&source(), &doc, &entries, 5).unwrap().leaf, "l1");
        assert!(mark(&source(), &doc, &entries, 3).is_err(), "only the source's copied history");
    }
}
