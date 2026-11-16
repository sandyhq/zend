//! Fork from a chosen message, and edit a prompt (`RewindChat`): what the
//! copy holds, what the rewind removes, that the removal holds across a
//! restart, and that the next turn's provider input lacks what went.
use async_trait::async_trait;
use futures::{StreamExt, stream::BoxStream};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use zend_doc::{MessagePart, MessageRole, MessageStatus, SessionMessageEntry};
use zend_engine::{EngineCore, HarnessRegistry};
use zend_harness::{Harness, HarnessError, RunControls};
use zend_proto::{
    AgentEvent, DoneStatus, HarnessId, Model, ReasoningLevel, RunRequest, SandboxLevel,
    SteeringMode,
};
use zend_rpc::methods;

struct Capture(Arc<Mutex<Vec<RunRequest>>>);
#[async_trait]
impl Harness for Capture {
    fn id(&self) -> HarnessId {
        HarnessId::Mock
    }
    fn display_name(&self) -> &str {
        "Capture"
    }
    fn supports_steering(&self) -> bool {
        false
    }
    fn steering_mode(&self) -> SteeringMode {
        SteeringMode::TurnBoundary
    }
    fn reasoning_levels(&self) -> &[ReasoningLevel] {
        &[]
    }
    async fn models(&self) -> Result<Vec<Model>, HarnessError> {
        Ok(vec![])
    }
    async fn run(
        &self,
        request: RunRequest,
        _: RunControls,
    ) -> Result<BoxStream<'static, Result<AgentEvent, HarnessError>>, HarnessError> {
        self.0.lock().unwrap().push(request);
        Ok(futures::stream::iter(vec![
            Ok(AgentEvent::TextDelta { text: "Fresh answer".into() }),
            Ok(AgentEvent::Done {
                status: DoneStatus::Completed,
                result: None,
                error: None,
                session_id: Some("fresh-provider-session".into()),
            }),
        ])
        .boxed())
    }
}

/// A harness whose start can be held: the first dispatch armed stops in
/// the engine's look at its CLI (after its prompt is written, before its run
/// is registered) till the test lets it go; its turn then never ends.
struct Held {
    armed: std::sync::atomic::AtomicBool,
    entered: Arc<tokio::sync::Notify>,
    release: Mutex<std::sync::mpsc::Receiver<()>>,
}
#[async_trait]
impl Harness for Held {
    fn id(&self) -> HarnessId {
        HarnessId::Mock
    }
    fn display_name(&self) -> &str {
        "Held"
    }
    fn supports_steering(&self) -> bool {
        false
    }
    fn steering_mode(&self) -> SteeringMode {
        SteeringMode::TurnBoundary
    }
    fn reasoning_levels(&self) -> &[ReasoningLevel] {
        &[]
    }
    fn executable_path(&self) -> Option<std::path::PathBuf> {
        if self.armed.swap(false, std::sync::atomic::Ordering::SeqCst) {
            self.entered.notify_one();
            let _ = self.release.lock().unwrap().recv();
        }
        None
    }
    async fn models(&self) -> Result<Vec<Model>, HarnessError> {
        Ok(vec![])
    }
    async fn run(
        &self,
        _: RunRequest,
        _: RunControls,
    ) -> Result<BoxStream<'static, Result<AgentEvent, HarnessError>>, HarnessError> {
        Ok(futures::stream::iter(vec![Ok(AgentEvent::TextDelta { text: "Working".into() })])
            .chain(futures::stream::pending())
            .boxed())
    }
}

fn message(id: &str, role: MessageRole, text: &str, status: MessageStatus) -> SessionMessageEntry {
    SessionMessageEntry {
        duration_ms: None,
        id: id.into(),
        role,
        parts: vec![MessagePart::Text { id: format!("{id}-text"), text: text.into() }],
        created_at: 1,
        device_id: "device".into(),
        status: Some(status),
        continuation_of: None,
    }
}

fn request(prompt: &str, cwd: &str) -> RunRequest {
    RunRequest {
        mcp: None,
        prompt: prompt.into(),
        harness: None,
        model: None,
        reasoning: None,
        model_options: Default::default(),
        cwd: cwd.into(),
        sandbox: SandboxLevel::WorkspaceWrite,
        auto_approve: true,
        access: Default::default(),
        plan: false,
        resume: None,
        attachments: vec![],
        worktree: None,
        env: Default::default(),
        extra_mcp: Vec::new(),
        instructions: Vec::new(),
    }
}

/// A chat "main" on this device: u1 a1 u2 a2 (a2 finished unless `streaming`).
fn seeded(dir: &std::path::Path, registry: HarnessRegistry, streaming: bool) -> EngineCore {
    let core = EngineCore::assemble(dir, Arc::new(registry), HarnessId::Mock, None).unwrap();
    core.workspace
        .create_chat("main", None, Some(&core.device_id), None, Some("/tmp".into()))
        .unwrap();
    let doc = core.doc_host.open("main").unwrap();
    let last = if streaming { MessageStatus::Streaming } else { MessageStatus::Complete };
    for entry in [
        message("u1", MessageRole::User, "Remember PINEAPPLE", MessageStatus::Complete),
        message("a1", MessageRole::Assistant, "I remember PINEAPPLE", MessageStatus::Complete),
        message("u2", MessageRole::User, "Now say BANANA", MessageStatus::Complete),
        message("a2", MessageRole::Assistant, "BANANA", last),
    ] {
        doc.doc().push_message(&entry).unwrap();
    }
    core
}

fn ids(core: &EngineCore, chat: &str) -> Vec<String> {
    let doc = core.doc_host.open(chat).unwrap();
    doc.doc().read_entries().unwrap().into_iter().map(|e| e.id).collect()
}

#[tokio::test]
async fn fork_at_a_reply_copies_through_it_and_at_a_prompt_stops_before_it() {
    let dir = tempfile::tempdir().unwrap();
    let core = seeded(dir.path(), HarnessRegistry::new(), false);
    let client = zend_rpc::memory_client(core.rpc_service());
    let fork = |chat: &str, at: &str| {
        serde_json::json!({ "chatId": chat, "sourceChatId": "main", "messageId": at })
    };

    // At a reply: everything up to and including it, then the seam.
    let reply: serde_json::Value =
        client.call_as(methods::FORK_SIDE_CHAT, fork("at-a1", "a1")).await.unwrap();
    assert_eq!(ids(&core, "at-a1"), ["u1", "a1", "fork:at-a1"]);
    assert!(reply.get("draft").is_none(), "a reply boundary leaves no draft");

    // At a prompt: what came before it; its text comes back for the box.
    let reply: serde_json::Value =
        client.call_as(methods::FORK_SIDE_CHAT, fork("at-u2", "u2")).await.unwrap();
    assert_eq!(ids(&core, "at-u2"), ["u1", "a1", "fork:at-u2"]);
    assert_eq!(reply["draft"], "Now say BANANA");
    assert_eq!(reply["parentChatId"], "main");

    // The first prompt: nothing before it to copy, only the seam.
    let reply: serde_json::Value =
        client.call_as(methods::FORK_SIDE_CHAT, fork("at-u1", "u1")).await.unwrap();
    assert_eq!(ids(&core, "at-u1"), ["fork:at-u1"]);
    assert_eq!(reply["draft"], "Remember PINEAPPLE");

    // The last reply, though a later one exists only through messageId.
    client.call(methods::FORK_SIDE_CHAT, fork("at-a2", "a2")).await.unwrap();
    assert_eq!(ids(&core, "at-a2"), ["u1", "a1", "u2", "a2", "fork:at-a2"]);

    // An id that isn't there is refused, and makes no chat.
    assert!(client.call(methods::FORK_SIDE_CHAT, fork("gone", "nope")).await.is_err());
    assert!(core.workspace.chat("gone").unwrap().is_none());
    // The source is untouched.
    assert_eq!(ids(&core, "main"), ["u1", "a1", "u2", "a2"]);
    core.shutdown().await;
}

#[tokio::test]
async fn fork_at_a_reply_still_streaming_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let core = seeded(dir.path(), HarnessRegistry::new(), true);
    let client = zend_rpc::memory_client(core.rpc_service());
    let params = serde_json::json!({ "chatId": "side", "sourceChatId": "main", "messageId": "a2" });
    let err = client.call(methods::FORK_SIDE_CHAT, params).await.unwrap_err();
    assert!(err.to_string().contains("finish"), "{err}");
    assert!(core.workspace.chat("side").unwrap().is_none());
    core.shutdown().await;
}

#[tokio::test]
async fn rewind_removes_the_tail_for_good_and_the_next_turn_starts_fresh_without_it() {
    let dir = tempfile::tempdir().unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let registry = HarnessRegistry::new();
    registry.register(Arc::new(Capture(requests.clone())));
    let core = seeded(dir.path(), registry, false);
    // The provider session that holds every turn so far.
    core.workspace
        .set_chat_harness_session("main", "old-provider-session", "/tmp", Some(HarnessId::Mock));
    let client = zend_rpc::memory_client(core.rpc_service());

    let missing = serde_json::json!({ "chatId": "main", "messageId": "nope" });
    assert!(client.call(methods::REWIND_CHAT, missing).await.is_err());
    assert_eq!(ids(&core, "main"), ["u1", "a1", "u2", "a2"], "a refused rewind removes nothing");

    let params = serde_json::json!({ "chatId": "main", "messageId": "u2" });
    let reply: serde_json::Value = client.call_as(methods::REWIND_CHAT, params).await.unwrap();
    assert_eq!(reply["removed"], 2);
    assert_eq!(ids(&core, "main"), ["u1", "a1"]);

    // The removal and the dropped provider session hold across a restart.
    core.shutdown().await;
    drop(client);
    drop(core);
    let requests2 = requests.clone();
    let registry = HarnessRegistry::new();
    registry.register(Arc::new(Capture(requests2)));
    let core = EngineCore::assemble(dir.path(), Arc::new(registry), HarnessId::Mock, None).unwrap();
    assert_eq!(ids(&core, "main"), ["u1", "a1"]);
    assert_eq!(
        core.workspace.chat("main").unwrap().unwrap().harness_session_id.as_deref(),
        Some(""),
        "the chat row keeps the do-not-resume tombstone"
    );

    // The edited prompt runs in a fresh provider session, fed what's kept.
    core.sessions
        .dispatch("main", HarnessId::Mock, request("Now say CHERRY", "/tmp"), Some("u2b".into()))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while requests.lock().unwrap().is_empty() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    let sent = requests.lock().unwrap()[0].clone();
    assert_eq!(sent.resume, None, "the old provider session holds the removed turns");
    assert!(sent.prompt.contains("PINEAPPLE"), "kept history rides along: {}", sent.prompt);
    assert!(sent.prompt.contains("Now say CHERRY"));
    assert!(!sent.prompt.contains("BANANA"), "removed turns never reach the provider: {}", sent.prompt);
    core.shutdown().await;
}

#[cfg(unix)]
#[tokio::test]
async fn pi_edit_end_to_end_next_turn_lacks_the_removed_messages() {
    use zend_harness::PiHarness;
    let dir = tempfile::tempdir().unwrap();
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../harness/tests/fixtures/fake-pi-rpc.py");
    let registry = HarnessRegistry::new();
    registry.register(Arc::new(
        PiHarness::new()
            .with_executable(fixture)
            .with_session_store(dir.path().join("index"))
            .with_graces(Duration::from_millis(50), Duration::from_millis(100)),
    ));
    let core = EngineCore::assemble(dir.path(), Arc::new(registry), HarnessId::Pi, None).unwrap();
    let chat = "pi-edit";
    core.workspace
        .create_chat(chat, None, Some(&core.device_id), None, None)
        .unwrap();
    let cwd = dir.path().display().to_string();
    let handle = core.doc_host.open(chat).unwrap();
    // The fixture answers "reply:<what it was sent>": the reply shows the
    // provider's whole input.
    let reply_to = |needle: &'static str| {
        let handle = handle.clone();
        async move {
            tokio::time::timeout(Duration::from_secs(10), async {
                loop {
                    let found = handle.doc().read_entries().unwrap().into_iter().find_map(|e| {
                        e.parts.into_iter().find_map(|p| match p {
                            MessagePart::Text { text, .. }
                                if text.starts_with("reply:") && text.contains(needle) =>
                            {
                                Some(text)
                            }
                            _ => None,
                        })
                    });
                    if let Some(text) = found {
                        return text;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap_or_else(|_| panic!("no reply containing {needle}"))
        }
    };
    let idle = || async {
        tokio::time::timeout(Duration::from_secs(10), async {
            while core.sessions.turn_in_flight(chat) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
    };
    for (id, prompt) in [("u1", "Remember PINEAPPLE"), ("u2", "Now say BANANA")] {
        core.sessions
            .dispatch(chat, HarnessId::Pi, request(prompt, &cwd), Some(id.into()))
            .await
            .unwrap();
        reply_to(if id == "u1" { "PINEAPPLE" } else { "BANANA" }).await;
        idle().await;
    }
    let first_session = core.workspace.chat(chat).unwrap().unwrap().harness_session_id;
    assert!(first_session.as_deref().is_some_and(|s| !s.is_empty()));

    let removed = core.sessions.rewind(chat, "u2").await.unwrap();
    assert_eq!(removed, 2);
    assert!(
        !handle.doc().read_entries().unwrap().iter().any(|e| e.parts.iter().any(
            |p| matches!(p, MessagePart::Text { text, .. } if text.contains("BANANA"))
        )),
        "the edited prompt and its reply are gone"
    );

    core.sessions
        .dispatch(chat, HarnessId::Pi, request("Now say CHERRY", &cwd), Some("u2b".into()))
        .await
        .unwrap();
    let input = reply_to("CHERRY").await;
    assert!(input.contains("PINEAPPLE"), "the fresh session gets the kept history: {input}");
    assert!(!input.contains("BANANA"), "the removed turn never reaches pi: {input}");
    idle().await;
    let second_session = core.workspace.chat(chat).unwrap().unwrap().harness_session_id;
    assert!(second_session.as_deref().is_some_and(|s| !s.is_empty()));
    assert_ne!(first_session, second_session, "pi ran a new session, not the old one");
    core.shutdown().await;
}

#[tokio::test]
async fn a_rewind_waits_for_a_turn_starting_and_never_removes_its_prompt() {
    let dir = tempfile::tempdir().unwrap();
    let entered = Arc::new(tokio::sync::Notify::new());
    let (release, released) = std::sync::mpsc::channel();
    let registry = HarnessRegistry::new();
    registry.register(Arc::new(Held {
        armed: true.into(),
        entered: entered.clone(),
        release: Mutex::new(released),
    }));
    let core = seeded(dir.path(), registry, false);

    // A prompt on its way: written to the transcript, its run not yet
    // registered (held in the engine's look at the CLI) when the edit of an
    // earlier prompt arrives.
    let starting = core.sessions.dispatch(
        "main",
        HarnessId::Mock,
        request("Now say CHERRY", "/tmp"),
        Some("u3".into()),
    );
    let go_edit = tokio::sync::Notify::new();
    let edit = async {
        go_edit.notified().await;
        core.sessions.rewind("main", "u2").await
    };
    let let_go = async {
        entered.notified().await;
        go_edit.notify_one();
        tokio::time::sleep(Duration::from_millis(300)).await;
        release.send(()).unwrap();
    };
    let (started, edited, ()) =
        tokio::time::timeout(Duration::from_secs(30), async { tokio::join!(starting, edit, let_go) })
            .await
            .expect("the dispatch and the edit both settle");
    started.unwrap();
    let err = edited.expect_err("the turn that started first holds the chat");
    assert!(err.to_string().contains("Stop the running turn"), "{err}");
    assert_eq!(ids(&core, "main")[..5], ["u1", "a1", "u2", "a2", "u3"], "the starting turn's prompt stays");
    core.sessions.terminate("main").await.unwrap();
    core.shutdown().await;
}

#[tokio::test]
async fn a_rewind_holds_when_the_registry_tombstone_was_lost_in_a_crash() {
    let dir = tempfile::tempdir().unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let registry = HarnessRegistry::new();
    registry.register(Arc::new(Capture(requests.clone())));
    let core = seeded(dir.path(), registry, false);
    core.workspace
        .set_chat_harness_session("main", "old-provider-session", "/tmp", Some(HarnessId::Mock));
    assert_eq!(core.sessions.rewind("main", "u2").await.unwrap(), 2);
    core.shutdown().await;
    drop(core);

    // The crash came inside the registry's debounce: the doc lost the tail,
    // the chat row still names the session that holds it.
    let registry = HarnessRegistry::new();
    registry.register(Arc::new(Capture(requests.clone())));
    let core = EngineCore::assemble(dir.path(), Arc::new(registry), HarnessId::Mock, None).unwrap();
    core.workspace
        .set_chat_harness_session("main", "old-provider-session", "/tmp", Some(HarnessId::Mock));
    core.sessions
        .dispatch("main", HarnessId::Mock, request("Now say CHERRY", "/tmp"), Some("u2b".into()))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while requests.lock().unwrap().is_empty() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    let sent = requests.lock().unwrap()[0].clone();
    assert_eq!(sent.resume, None, "the doc alone says the old session holds removed turns");
    assert!(sent.prompt.contains("PINEAPPLE") && !sent.prompt.contains("BANANA"), "{}", sent.prompt);
    core.shutdown().await;
}

#[tokio::test]
async fn a_fork_at_a_steer_ends_the_reply_still_streaming_above_it() {
    let dir = tempfile::tempdir().unwrap();
    let core = seeded(dir.path(), HarnessRegistry::new(), true);
    // A steer sent while a2 was still being written.
    core.doc_host
        .open("main")
        .unwrap()
        .doc()
        .push_message(&message("u3", MessageRole::User, "Also CHERRY", MessageStatus::Complete))
        .unwrap();
    let client = zend_rpc::memory_client(core.rpc_service());
    let params = serde_json::json!({ "chatId": "side", "sourceChatId": "main", "messageId": "u3" });
    let reply: serde_json::Value = client.call_as(methods::FORK_SIDE_CHAT, params).await.unwrap();
    assert_eq!(reply["draft"], "Also CHERRY");
    let copied = core.doc_host.open("side").unwrap().doc().read_entries().unwrap();
    let a2 = copied.iter().find(|e| e.id == "a2").unwrap();
    assert_eq!(a2.status, Some(MessageStatus::Aborted), "nothing in the fork would ever finish it");
    let source = core.doc_host.open("main").unwrap().doc().read_entries().unwrap();
    assert_eq!(source[3].status, Some(MessageStatus::Streaming), "the source's reply goes on");
    core.shutdown().await;
}

#[tokio::test]
async fn a_retried_fork_at_a_prompt_gives_its_words_back_again() {
    let dir = tempfile::tempdir().unwrap();
    let core = seeded(dir.path(), HarnessRegistry::new(), false);
    let client = zend_rpc::memory_client(core.rpc_service());
    let params = serde_json::json!({ "chatId": "side", "sourceChatId": "main", "messageId": "u2" });
    let first: serde_json::Value =
        client.call_as(methods::FORK_SIDE_CHAT, params.clone()).await.unwrap();
    // Its reply lost on the way: the window asks again with the same id.
    let again: serde_json::Value = client.call_as(methods::FORK_SIDE_CHAT, params).await.unwrap();
    assert_eq!(first["draft"], "Now say BANANA");
    assert_eq!(again["draft"], "Now say BANANA");
    assert_eq!(again["id"], first["id"]);
    assert_eq!(ids(&core, "side"), ["u1", "a1", "fork:side"]);
    core.shutdown().await;
}
