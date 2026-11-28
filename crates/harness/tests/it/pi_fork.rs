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

/// Turn `n`'s three entries (prompt, tool result, reply), ids u<n> t<n> a<n>
/// (with `suffix`), the first a child of `parent`.
fn turn(n: u32, suffix: &str, parent: Value) -> Vec<Value> {
    let mut parent = parent;
    let mut lines = vec![];
    for (kind, role) in [("u", "user"), ("t", "toolResult"), ("a", "assistant")] {
        let id = format!("{kind}{n}{suffix}");
        lines.push(json!({"type":"message","id":id,"parentId":parent,
            "message":{"role":role,"content":[{"type":"text","text":format!("{role} {n}{suffix}")}]}}));
        parent = Value::from(id);
    }
    lines
}

/// Where the host's Pi keeps the source's session: a folder of its own
/// under the agent folder's `sessions`, which the host's store searches.
fn sessions_dir(dir: &Path) -> std::path::PathBuf {
    dir.join("agent/sessions/--work--")
}

/// Session `source-session` with `entries`, at `file`, every line ended.
fn write_session(file: &Path, dir: &Path, entries: Vec<Value>) -> std::path::PathBuf {
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    let mut lines = vec![json!({"type":"session","version":3,"id":"source-session","cwd":dir})];
    lines.extend(entries);
    std::fs::write(file, lines.iter().map(|l| format!("{l}\n")).collect::<String>()).unwrap();
    file.to_path_buf()
}

/// The source: three turns, kept where the host's Pi keeps its sessions.
fn source(dir: &Path) -> std::path::PathBuf {
    let mut entries = turn(1, "", Value::Null);
    entries.extend(turn(2, "", "a1".into()));
    entries.extend(turn(3, "", "a2".into()));
    write_session(&sessions_dir(dir).join("source.jsonl"), dir, entries)
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

/// As [`run`], keeping a failed run's events up to its error.
async fn run_to_end(dir: &Path, prompt: &str, fork: Option<NativeFork>) -> Vec<Result<AgentEvent, String>> {
    match harness(dir).run(request(dir, prompt, fork), controls()).await {
        Ok(stream) => tokio::time::timeout(
            Duration::from_secs(20),
            stream.map(|e| e.map_err(|e| e.to_string())).collect(),
        )
        .await
        .unwrap(),
        Err(error) => vec![Err(error.to_string())],
    }
}

fn sorted<const N: usize>(files: [std::path::PathBuf; N]) -> Vec<std::path::PathBuf> {
    let mut files = files.to_vec();
    files.sort();
    files
}

/// Every session file under `dir`, at any depth.
fn all_sessions(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut found = vec![];
    let mut dirs = vec![dir.to_path_buf()];
    while let Some(at) = dirs.pop() {
        for entry in std::fs::read_dir(&at).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                dirs.push(path);
            } else if path.extension().is_some_and(|e| e == "jsonl") {
                found.push(path);
            }
        }
    }
    found.sort();
    found
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
    // Beside the source, and nothing else left anywhere.
    assert_eq!(file.parent(), src.parent());
    let mut kept = vec![src.clone(), file.clone()];
    kept.sort();
    assert_eq!(all_sessions(dir.path()), kept);
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
    assert_eq!(all_sessions(dir.path()).len(), 2);
}

#[tokio::test]
async fn every_failure_runs_fresh_with_the_history_and_leaves_nothing_behind() {
    // K5: an unknown leaf, a missing file, a fork Pi refuses, a session
    // the host doesn't have.
    for case in ["leaf", "file", "refused", "other"] {
        let dir = tempfile::tempdir().unwrap();
        let src = source(dir.path());
        let fork = match case {
            "leaf" => native(&src, "nowhere"),
            "file" => {
                std::fs::remove_file(&src).unwrap();
                write_session(&dir.path().join("elsewhere/source.jsonl"), dir.path(), vec![]);
                native(&src, "a2")
            }
            "other" => NativeFork { session_id: "another-session".into(), ..native(&src, "a2") },
            _ => {
                std::fs::write(dir.path().join("fail-fork"), "").unwrap();
                native(&src, "a2")
            }
        };
        let before = std::fs::read(&src).ok();
        let events = run(dir.path(), "hi", Some(fork)).await;
        let fresh = runs_fresh(&events, case);
        assert_eq!(std::fs::read(&src).ok(), before, "{case}");
        // The source (or what stands in for it) and the fresh session only:
        // no copy left over.
        let left = all_sessions(dir.path());
        assert_eq!(left.len(), 2, "{case}: {left:?}");
        assert!(left.contains(&fresh), "{case}: {left:?}");
    }
}

/// The run fell back: fresh, with the history as its prompt. Its session file.
fn runs_fresh(events: &[AgentEvent], case: &str) -> std::path::PathBuf {
    assert!(
        events.iter().any(|e| matches!(e, AgentEvent::NativeForkFallback { .. })),
        "{case}: {events:?}"
    );
    assert_eq!(reply(events), "reply:HISTORY\n\nhi", "{case}");
    let file = turn_file(events);
    let (header, entries) = read(&file);
    assert!(header.get("parentSession").is_none(), "{case}: a fresh session");
    assert_eq!(texts(&entries)[0], "HISTORY\n\nhi", "{case}");
    file
}

/// Review finding 1: the doc names the source, but only a session the
/// host's own store has may be copied; one anywhere else, even with the
/// right header, is never read.
#[tokio::test]
async fn a_source_the_hosts_store_does_not_have_is_never_copied() {
    let dir = tempfile::tempdir().unwrap();
    let mut entries = turn(1, "", Value::Null);
    entries.extend(turn(2, "", "a1".into()));
    let planted = write_session(&dir.path().join("elsewhere/planted.jsonl"), dir.path(), entries);
    let events = run(dir.path(), "hi", Some(native(&planted, "a1"))).await;
    let fresh = runs_fresh(&events, "planted");
    assert!(!std::fs::read_to_string(&fresh).unwrap().contains("assistant 1"));
    assert_eq!(all_sessions(dir.path()), sorted([planted, fresh]));
}

/// Review finding 2: the turn's leaf has no children, but another branch
/// ends later in the file. `pi --fork` copies both and reopens at that
/// branch's end, so the copy isn't the fork: fresh with the history.
#[tokio::test]
async fn a_source_with_another_branch_after_the_leaf_runs_fresh() {
    let dir = tempfile::tempdir().unwrap();
    let mut entries = turn(1, "", Value::Null);
    entries.extend(turn(2, "", "a1".into()));
    entries.extend(turn(2, "b", "a1".into()));
    let src = write_session(&sessions_dir(dir.path()).join("source.jsonl"), dir.path(), entries);
    let events = run(dir.path(), "hi", Some(native(&src, "a2"))).await;
    let fresh = runs_fresh(&events, "branched");
    assert_eq!(all_sessions(dir.path()), sorted([fresh, src]));
}

/// Review finding 3: Pi may repair a session file it only reads (a last
/// line without its newline, session-manager.js:367). It never gets the
/// source: the source stays byte for byte as it was.
#[tokio::test]
async fn pi_never_gets_the_source_itself() {
    let dir = tempfile::tempdir().unwrap();
    let src = source(dir.path());
    let text = std::fs::read_to_string(&src).unwrap();
    std::fs::write(&src, text.trim_end()).unwrap();
    let before = std::fs::read(&src).unwrap();
    let events = run(dir.path(), "hi", Some(native(&src, "a2"))).await;
    assert!(!events.iter().any(|e| matches!(e, AgentEvent::NativeForkFallback { .. })), "{events:?}");
    assert_eq!(reply(&events), "reply:hi");
    assert_eq!(std::fs::read(&src).unwrap(), before);
}

/// Review finding 6: the helper Pi exits after writing its copy; the Pi
/// that is to run the fork fails to open it, or opens another session.
/// Each runs fresh with the history, and no copy is left anywhere.
#[tokio::test]
async fn a_pi_that_fails_with_the_copy_runs_fresh_and_leaves_nothing() {
    for marker in ["die-after-fork", "fail-open", "open-other"] {
        let dir = tempfile::tempdir().unwrap();
        let src = source(dir.path());
        std::fs::write(dir.path().join(marker), "").unwrap();
        let results = run_to_end(dir.path(), "hi", Some(native(&src, "a2"))).await;
        let errors: Vec<_> = results.iter().filter_map(|r| r.as_ref().err()).collect();
        assert!(errors.is_empty(), "{marker}: {errors:?}");
        let events: Vec<_> = results.into_iter().map(Result::unwrap).collect();
        let fresh = runs_fresh(&events, marker);
        let mut kept = vec![src.clone(), fresh];
        kept.sort();
        assert_eq!(all_sessions(dir.path()), kept, "{marker}");
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
    assert_eq!(entries.last().unwrap()["id"], leaf.unwrap().as_str());
    // Just before the turn's Done.
    let at = events.iter().position(|e| matches!(e, AgentEvent::NativeTurn { .. })).unwrap();
    assert!(matches!(events[at + 1], AgentEvent::Done { .. }), "{events:?}");
}
