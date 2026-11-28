//! A fork's first turn on a copy of its source's Pi session (`fork_from`),
//! against the Pi-shaped fixture `fake-pi-rpc.py` (plans/pi-native-fork).
#![cfg(unix)]
use futures::StreamExt;
use serde_json::{Value, json};
use std::{path::Path, time::Duration};
use tokio::sync::{mpsc, oneshot};
use zend_harness::{CancellationToken, Harness, PiHarness, RunControls};
use zend_proto::{AgentEvent, NativeFork, RunRequest, SandboxLevel};

fn harness(dir: &Path) -> PiHarness {
    PiHarness::new()
        .with_executable(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake-pi-rpc.py"))
        .with_agent_dir(dir.join("agent"))
        .with_session_store(dir.join("index"))
        .with_graces(Duration::from_millis(50), Duration::from_millis(100))
}

fn controls() -> RunControls {
    let (_tx, rx) = mpsc::channel(8);
    RunControls {
        realtime: None,
        execution_lease: None,
        steering: rx,
        interrupt: CancellationToken::new(),
        request_input: Box::new(|_| {
            let (tx, rx) = oneshot::channel();
            let _ = tx.send(vec![]);
            rx
        }),
        turn: Default::default(),
        sandbox: None,
    }
}

fn request(cwd: &Path, prompt: &str, fork_from: Option<NativeFork>) -> RunRequest {
    RunRequest {
        prompt: prompt.into(),
        harness: None,
        model: None,
        reasoning: None,
        model_options: Default::default(),
        cwd: cwd.display().to_string(),
        sandbox: SandboxLevel::WorkspaceWrite,
        auto_approve: true,
        access: Default::default(),
        plan: false,
        resume: None,
        attachments: vec![],
        worktree: None,
        mcp: None,
        env: Default::default(),
        extra_mcp: Vec::new(),
        instructions: Vec::new(),
        fork_from,
    }
}

/// The source: three turns (prompt, tool result, reply), ids u1 t1 a1 …
fn source(dir: &Path) -> std::path::PathBuf {
    let file = dir.join("source.jsonl");
    let mut lines = vec![json!({"type":"session","version":3,"id":"source-session","cwd":dir})];
    let mut parent = Value::Null;
    for n in 1..=3 {
        for (kind, role) in [("u", "user"), ("t", "toolResult"), ("a", "assistant")] {
            let id = format!("{kind}{n}");
            lines.push(json!({"type":"message","id":id,"parentId":parent,
                "message":{"role":role,"content":[{"type":"text","text":format!("{role} {n}")}]}}));
            parent = Value::from(id);
        }
    }
    std::fs::write(&file, lines.iter().map(|l| format!("{l}\n")).collect::<String>()).unwrap();
    file
}

fn read(file: &Path) -> (Value, Vec<Value>) {
    let text = std::fs::read_to_string(file).unwrap();
    let mut lines = text.lines().map(|l| serde_json::from_str::<Value>(l).unwrap());
    (lines.next().unwrap(), lines.collect())
}

fn texts(entries: &[Value]) -> Vec<String> {
    entries
        .iter()
        .map(|e| e["message"]["content"][0]["text"].as_str().unwrap_or_default().to_owned())
        .collect()
}

async fn run(dir: &Path, prompt: &str, fork: Option<NativeFork>) -> Vec<AgentEvent> {
    let stream = harness(dir)
        .run(request(dir, prompt, fork), controls())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(20), stream.map(Result::unwrap).collect())
        .await
        .unwrap()
}

fn native(file: &Path, leaf: &str) -> NativeFork {
    NativeFork {
        session_id: "source-session".into(),
        session_file: file.display().to_string(),
        leaf: leaf.into(),
        fallback_prompt: "HISTORY\n\nhi".into(),
    }
}

fn reply(events: &[AgentEvent]) -> String {
    events
        .iter()
        .filter_map(|e| match e {
            AgentEvent::TextDelta { text } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

/// The session file the run ended on, from its `NativeTurn`.
fn turn_file(events: &[AgentEvent]) -> std::path::PathBuf {
    events
        .iter()
        .find_map(|e| match e {
            AgentEvent::NativeTurn { session_file, .. } => Some(session_file.into()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no NativeTurn: {events:?}"))
}

fn sessions_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|n| n.ends_with(".jsonl"))
        .collect();
    names.sort();
    names
}

#[tokio::test]
async fn a_fork_at_a_middle_turn_starts_on_a_cut_copy_naming_the_source() {
    let dir = tempfile::tempdir().unwrap();
    let src = source(dir.path());
    let before = std::fs::read(&src).unwrap();
    let events = run(dir.path(), "hi", Some(native(&src, "a2"))).await;
    assert!(!events.iter().any(|e| matches!(e, AgentEvent::NativeForkFallback { .. })), "{events:?}");
    // K3: Pi got the person's prompt alone, no history block.
    assert_eq!(reply(&events), "reply:hi");
    let file = turn_file(&events);
    let (header, entries) = read(&file);
    // K1: the source named, turns 1-2 with their tool results, not turn 3.
    assert_eq!(header["parentSession"], src.display().to_string());
    assert_ne!(header["id"], "source-session");
    assert_eq!(
        texts(&entries),
        ["user 1", "toolResult 1", "assistant 1", "user 2", "toolResult 2", "assistant 2", "hi", "tool ran", "reply:hi"]
    );
    // K4: the source untouched; the intermediate copy gone.
    assert_eq!(std::fs::read(&src).unwrap(), before);
    let started: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            AgentEvent::SessionStarted { session_id, .. } => Some(session_id.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(started, [header["id"].as_str().unwrap()]);
    assert_eq!(
        sessions_in(dir.path()),
        {
            let mut v = vec!["source.jsonl".to_string(), file.file_name().unwrap().to_string_lossy().into_owned()];
            v.sort();
            v
        }
    );
}

#[tokio::test]
async fn a_fork_at_a_first_prompt_holds_only_what_came_before_it() {
    let dir = tempfile::tempdir().unwrap();
    let src = source(dir.path());
    // K2: fork at the prompt of turn 2 = through turn 1's end.
    let events = run(dir.path(), "hi", Some(native(&src, "a1"))).await;
    assert_eq!(reply(&events), "reply:hi");
    let (_, entries) = read(&turn_file(&events));
    assert_eq!(texts(&entries), ["user 1", "toolResult 1", "assistant 1", "hi", "tool ran", "reply:hi"]);
}

#[tokio::test]
async fn a_fork_at_the_last_turn_copies_the_whole_session() {
    let dir = tempfile::tempdir().unwrap();
    let src = source(dir.path());
    let events = run(dir.path(), "hi", Some(native(&src, "a3"))).await;
    let file = turn_file(&events);
    let (header, entries) = read(&file);
    assert_eq!(header["parentSession"], src.display().to_string());
    assert_eq!(entries.len(), 12, "{:?}", texts(&entries));
    assert_eq!(sessions_in(dir.path()).len(), 2);
}

#[tokio::test]
async fn every_failure_runs_fresh_with_the_history_and_leaves_nothing_behind() {
    // K5: an unknown leaf, a missing file, a fork Pi refuses, a file that
    // isn't the source's session (another session's, or no session at all).
    for case in ["leaf", "file", "refused", "other", "not-a-session"] {
        let dir = tempfile::tempdir().unwrap();
        let src = source(dir.path());
        let before = std::fs::read(&src).unwrap();
        let fork = match case {
            "leaf" => native(&src, "nowhere"),
            "file" => native(&dir.path().join("gone.jsonl"), "a2"),
            "other" => NativeFork { session_id: "another-session".into(), ..native(&src, "a2") },
            "not-a-session" => {
                let key = dir.path().join("id_ed25519");
                std::fs::write(&key, "-----BEGIN OPENSSH PRIVATE KEY-----\n").unwrap();
                native(&key, "a2")
            }
            _ => {
                std::fs::write(dir.path().join("fail-fork"), "").unwrap();
                native(&src, "a2")
            }
        };
        let events = run(dir.path(), "hi", Some(fork)).await;
        assert!(
            events.iter().any(|e| matches!(e, AgentEvent::NativeForkFallback { .. })),
            "{case}: {events:?}"
        );
        assert_eq!(reply(&events), "reply:HISTORY\n\nhi", "{case}");
        let (header, entries) = read(&turn_file(&events));
        assert!(header.get("parentSession").is_none(), "{case}: a fresh session");
        assert_eq!(texts(&entries)[0], "HISTORY\n\nhi");
        assert_eq!(std::fs::read(&src).unwrap(), before, "{case}");
        // The source and the fresh session only: no copy left over.
        assert_eq!(sessions_in(dir.path()).len(), 2, "{case}: {:?}", sessions_in(dir.path()));
    }
}

#[tokio::test]
async fn every_turn_reports_where_it_left_the_session() {
    let dir = tempfile::tempdir().unwrap();
    let events = run(dir.path(), "hello", None).await;
    let (session_id, leaf) = events
        .iter()
        .find_map(|e| match e {
            AgentEvent::NativeTurn { session_id, leaf, .. } => Some((session_id.clone(), leaf.clone())),
            _ => None,
        })
        .unwrap();
    let (header, entries) = read(&turn_file(&events));
    assert_eq!(header["id"], session_id.as_str());
    assert_eq!(entries.last().unwrap()["id"], leaf.as_str());
    // Just before the turn's Done.
    let at = events.iter().position(|e| matches!(e, AgentEvent::NativeTurn { .. })).unwrap();
    assert!(matches!(events[at + 1], AgentEvent::Done { .. }), "{events:?}");
}
