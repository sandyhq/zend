//! What a Stop's version says its sender saw: the rows the transcript had
//! at that version, told from the doc's ops (crates/doc/src/row_history.rs).
use loro::{ExportMode, LoroDoc, LoroList, LoroMap, VersionVector};
use zend_doc::{MessageRole, SessionDoc, SessionMessageEntry, decode_version, encode_version};

fn entry(id: &str, continues: Option<&str>) -> SessionMessageEntry {
    SessionMessageEntry {
        id: id.into(),
        role: MessageRole::User,
        created_at: 1,
        device_id: "device".into(),
        parts: vec![],
        status: None,
        continuation_of: continues.map(str::to_owned),
        duration_ms: None,
    }
}

fn shown(doc: &SessionDoc) -> Vec<String> {
    let mut ids: Vec<String> = doc
        .read_entries()
        .unwrap()
        .into_iter()
        .map(|e| e.id)
        .collect();
    ids.sort();
    ids
}

fn held(doc: &SessionDoc, version: &VersionVector) -> Vec<String> {
    let mut ids: Vec<String> = doc.messages_in_version(version).into_iter().collect();
    ids.sort();
    ids
}

fn reopened(doc: &SessionDoc) -> SessionDoc {
    let copy = LoroDoc::new();
    copy.import(&doc.export_snapshot().unwrap()).unwrap();
    SessionDoc::from_doc(copy)
}

/// Round 8, findings 2 and 3: a row's map can be in the list before the
/// row has an id (its ops arrive in two imports). A version taken in
/// between doesn't hold the row, and asking early must not be remembered.
#[test]
fn a_row_given_its_id_after_a_version_is_not_in_it() {
    let doc = SessionDoc::init("late-id").unwrap();
    let row = doc
        .doc()
        .get_list("messages")
        .push_container(LoroMap::new())
        .unwrap();
    doc.commit();
    let before = decode_version(&doc.version_for_command()).unwrap();
    assert!(!doc.version_holds_message(&before, "late"));

    row.insert("id", "late").unwrap();
    row.insert("role", "user").unwrap();
    row.insert("createdAt", 1i64).unwrap();
    row.insert("deviceId", "other").unwrap();
    row.insert_container("parts", LoroList::new()).unwrap();
    doc.commit();
    let after = decode_version(&doc.version_for_command()).unwrap();

    assert_eq!(shown(&doc), ["late"]);
    assert!(
        doc.version_holds_message(&after, "late"),
        "asked before it had an id, never found again"
    );
    assert!(
        !doc.version_holds_message(&before, "late"),
        "the row had no id at that version"
    );
    assert_eq!(held(&doc, &before), Vec::<String>::new());
    assert_eq!(held(&doc, &after), ["late"]);
}

/// Round 8, finding 1: a row removed since (a rewind) is still one the
/// version that showed it holds, after a reopen too, and isn't one a
/// version after its removal holds.
#[test]
fn a_row_removed_since_is_still_in_the_version_that_showed_it() {
    let doc = SessionDoc::init("removed").unwrap();
    doc.push_message(&entry("kept", None)).unwrap();
    let early = decode_version(&doc.version_for_command()).unwrap();
    doc.push_message(&entry("seen", None)).unwrap();
    doc.push_message(&entry("seen-too", Some("seen"))).unwrap();
    let saw = decode_version(&doc.version_for_command()).unwrap();
    assert_eq!(doc.truncate_from("seen", &[]).unwrap(), 2);
    let gone = decode_version(&doc.version_for_command()).unwrap();
    assert_eq!(shown(&doc), ["kept"]);

    for doc in [&doc, &reopened(&doc)] {
        assert_eq!(held(doc, &early), ["kept"]);
        assert_eq!(held(doc, &saw), ["kept", "seen", "seen-too"]);
        assert_eq!(held(doc, &gone), ["kept"]);
        assert!(doc.version_holds_message(&saw, "seen"));
        assert!(doc.version_holds_message(&saw, "seen-too"));
        assert!(!doc.version_holds_message(&gone, "seen"));
        assert!(!doc.version_holds_message(&early, "seen"));
        assert_eq!(
            doc.message_continues("seen-too"),
            ["seen"],
            "a removed row's chain is still known"
        );
    }
}

/// Round 8 (should): a doc checked out at an older version shows that
/// version's rows, so the version its Stop carries is that one, not the
/// newest its oplog holds.
#[test]
fn a_checked_out_doc_says_the_version_it_shows() {
    let doc = SessionDoc::init("checkout").unwrap();
    doc.push_message(&entry("shown", None)).unwrap();
    let before = doc.doc().state_frontiers();
    let shown_version = doc.doc().state_vv();
    doc.push_message(&entry("hidden", None)).unwrap();
    doc.doc().checkout(&before).unwrap();
    assert_eq!(shown(&doc), ["shown"]);

    assert_eq!(
        doc.version_for_command(),
        encode_version(&shown_version),
        "the Stop's version is newer than the transcript the doc shows"
    );
    assert_ne!(
        doc.version_for_command(),
        encode_version(&doc.doc().oplog_vv())
    );
}

/// The same, asked of the rows: the checked-out doc's version doesn't hold
/// the row it doesn't show, with no change of the oplog in between.
#[test]
fn a_checked_out_docs_version_holds_only_the_rows_it_shows() {
    let doc = SessionDoc::init("checkout").unwrap();
    let before = doc.doc().state_frontiers();
    doc.push_message(&entry("hidden", None)).unwrap();
    let newest = decode_version(&doc.version_for_command()).unwrap();
    assert!(doc.version_holds_message(&newest, "hidden"));
    doc.doc().checkout(&before).unwrap();
    let version = decode_version(&doc.version_for_command()).unwrap();
    assert!(shown(&doc).is_empty());
    assert!(!doc.version_holds_message(&version, "hidden"));
    assert!(held(&doc, &version).is_empty());
}

/// The rows a version holds are the rows a doc at that version shows:
/// three devices add, rewind and exchange in a fixed pseudo-random order,
/// and every version on the way is checked against a doc forked at it.
#[test]
fn the_rows_of_a_version_are_the_ones_a_doc_at_it_shows() {
    let first = SessionDoc::init("walk").unwrap();
    let docs = [reopened(&first), reopened(&first), first];
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    let mut next = |below: usize| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % below as u64) as usize
    };
    let mut versions: Vec<(loro::Frontiers, VersionVector, Vec<String>)> = Vec::new();
    let mut removals = 0;
    for step in 0..400 {
        let at = next(3);
        let doc = &docs[at];
        match next(10) {
            0..=5 => doc.push_message(&entry(&format!("m{step}"), None)).unwrap(),
            6 => {
                let ids = shown(doc);
                if !ids.is_empty() {
                    // (Sorted by id, not by place: any row and all after it.)
                    let from = &ids[next(ids.len())];
                    removals += doc.truncate_from(from, &[]).unwrap();
                }
            }
            _ => {
                let from = &docs[next(3)];
                doc.import(&from.export(ExportMode::all_updates()).unwrap())
                    .unwrap();
            }
        }
        versions.push((
            doc.doc().state_frontiers(),
            doc.doc().state_vv(),
            shown(doc),
        ));
    }
    assert!(removals > 20, "the walk removed {removals} rows");
    let all = &docs[0];
    for doc in &docs[1..] {
        all.import(&doc.export(ExportMode::all_updates()).unwrap())
            .unwrap();
    }
    for copy in [all, &reopened(all)] {
        for (frontiers, version, rows) in &versions {
            assert_eq!(&held(copy, version), rows, "at {frontiers:?}");
            let at = SessionDoc::from_doc(copy.doc().fork_at(frontiers).unwrap());
            assert_eq!(
                &shown(&at),
                rows,
                "the check's own reading at {frontiers:?}"
            );
        }
    }
}

/// A doc trimmed of its old history (a shallow snapshot) still says its
/// old rows are in every version it can name, and reads the ops since.
#[test]
fn a_trimmed_doc_still_knows_its_old_rows() {
    let doc = SessionDoc::init("trimmed").unwrap();
    doc.push_message(&entry("old", None)).unwrap();
    doc.push_message(&entry("old-too", Some("old"))).unwrap();
    let trimmed_at = doc.doc().state_frontiers();
    let old = decode_version(&doc.version_for_command()).unwrap();
    doc.push_message(&entry("new", None)).unwrap();
    let saw = decode_version(&doc.version_for_command()).unwrap();
    doc.truncate_from("new", &[]).unwrap();

    let copy = LoroDoc::new();
    copy.import(
        &doc.export(ExportMode::shallow_snapshot(&trimmed_at))
            .unwrap(),
    )
    .unwrap();
    assert!(copy.is_shallow());
    let copy = SessionDoc::from_doc(copy);
    assert_eq!(held(&copy, &old), ["old", "old-too"]);
    assert_eq!(held(&copy, &saw), ["new", "old", "old-too"]);
    assert_eq!(held(&copy, &copy.doc().oplog_vv()), ["old", "old-too"]);
    assert_eq!(copy.message_continues("old-too"), ["old"]);
}
