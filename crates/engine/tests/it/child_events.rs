//! Child events: a child chat's settled turn tells its parent, once, without
//! the parent waiting (docs/mcp.md, "Child events"; `child_events`).
//!
//! Each test builds a parent and subagent children on a mock harness that
//! records every run it is asked for, keyed by the chat's cwd. The parent
//! ends its turn (nothing waits); what it is later told is the run the engine
//! starts in it.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use futures::{StreamExt, stream::BoxStream};
use zend_doc::SessionCommandPayload;
use zend_engine::child_events::{EVENT_PREFIX, WINDOW};
use zend_engine::{EngineCore, HarnessRegistry};
use zend_harness::{Harness, HarnessError, RunControls};
use zend_proto::{
    AgentEvent, DoneStatus, HarnessId, Model, ReasoningLevel, RunRequest, SandboxLevel,
    SteeringMode, UserInputQuestion,
};
use zend_rpc::methods;

const PARENT_CWD: &str = "/tmp/parent";

/// Answers "reply to <prompt>" and finishes; a prompt starting "ask" parks on
/// a question instead.
struct Scripted {
    runs: Arc<Mutex<Vec<RunRequest>>>,
    /// What a held ("hold…") run took in through its live mailbox.
    steered: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl Harness for Scripted {
    fn id(&self) -> HarnessId {
        HarnessId::Mock
    }
    fn display_name(&self) -> &str {
        "Scripted"
    }
    fn supports_steering(&self) -> bool {
        true
    }
    fn steering_mode(&self) -> SteeringMode {
        SteeringMode::StepBoundary
    }
    fn reasoning_levels(&self) -> &[ReasoningLevel] {
        &[]
    }
    /// Every mode: `a_plugin_s_child_runs_with_the_access_it_asks` asks for
    /// one, and a plugin's `new_chat` refuses a mode its harness doesn't offer.
    fn access_modes(&self) -> &[zend_proto::AccessMode] {
        &zend_proto::AccessMode::ALL
    }
    async fn models(&self) -> Result<Vec<Model>, HarnessError> {
        Ok(vec![])
    }
    async fn run(
        &self,
        request: RunRequest,
        controls: RunControls,
    ) -> Result<BoxStream<'static, Result<AgentEvent, HarnessError>>, HarnessError> {
        self.runs.lock().unwrap().push(request.clone());
        if request.prompt.starts_with("ask") {
            let answer = (controls.request_input)(vec![UserInputQuestion {
                id: "q1".into(),
                header: "Choose".into(),
                question: "Which database?".into(),
                options: vec!["pg".into(), "sqlite".into()],
                prefill: None,
                multiline: false,
                multi_select: false,
            }]);
            // The question owns its receiver until the run ends: hold it open.
            return Ok(futures::stream::once(async move {
                let _answer = answer;
                futures::future::pending::<Result<AgentEvent, HarnessError>>().await
            })
            .boxed());
        }
        if request.prompt.starts_with("hold") {
            // Working until its mailbox gets a message, then done.
            let steered = self.steered.clone();
            return Ok(futures::stream::unfold(
                (0u8, controls.steering),
                move |(step, mut steering)| {
                    let steered = steered.clone();
                    async move {
                        match step {
                            0 => {
                                let message = steering.recv().await?;
                                steered.lock().unwrap().push(message.prompt);
                                Some((
                                    Ok(AgentEvent::Steered {
                                        assistant_message_id: None,
                                        next_assistant_message_id: Some("after-steer".into()),
                                    }),
                                    (1, steering),
                                ))
                            }
                            1 => Some((
                                Ok(AgentEvent::Done {
                                    status: DoneStatus::Completed,
                                    result: None,
                                    error: None,
                                    session_id: Some("sess".into()),
                                }),
                                (2, steering),
                            )),
                            _ => None,
                        }
                    }
                },
            )
            .boxed());
        }
        Ok(futures::stream::iter(vec![
            Ok(AgentEvent::TextDelta {
                text: format!("reply to {}", request.prompt),
            }),
            Ok(AgentEvent::Done {
                status: DoneStatus::Completed,
                result: None,
                error: None,
                session_id: Some("sess".into()),
            }),
        ])
        .boxed())
    }
}

struct Rig {
    core: EngineCore,
    runs: Arc<Mutex<Vec<RunRequest>>>,
    steered: Arc<Mutex<Vec<String>>>,
    client: zend_rpc::RpcClient,
}

impl Rig {
    async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap().keep();
        let runs = Arc::new(Mutex::new(Vec::new()));
        let steered = Arc::new(Mutex::new(Vec::new()));
        let registry = HarnessRegistry::new();
        registry.register(Arc::new(Scripted { runs: runs.clone(), steered: steered.clone() }));
        let core = EngineCore::assemble(&dir.join("data"), Arc::new(registry), HarnessId::Mock, None)
            .expect("engine core assembles");
        core.sessions.set_no_ipc_port();
        let client = zend_rpc::memory_client(core.rpc_service());
        let rig = Self { core, runs, steered, client };
        rig.chat("parent", PARENT_CWD, serde_json::json!({})).await;
        rig
    }

    async fn chat(&self, id: &str, cwd: &str, extra: serde_json::Value) {
        let mut op = serde_json::json!({
            "op": "createChat", "chatId": id, "deviceId": self.core.device_id, "cwd": cwd,
        });
        op.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        self.client.call(methods::MUTATE, op).await.expect("createChat");
        // Pre-titled: the auto-titler never dispatches a run of its own.
        self.core.workspace.rename_chat(id, &format!("Chat {id}")).unwrap();
    }

    /// A subagent of the parent; `notify` is the creator's notify_parent, if said.
    async fn child(&self, id: &str, notify: Option<bool>) {
        let mut extra = serde_json::json!({ "parentChatId": "parent", "childKind": "subagent" });
        if let Some(notify) = notify {
            extra["notifyParent"] = notify.into();
        }
        self.chat(id, &format!("/tmp/{id}"), extra).await;
    }

    /// As Zend MCP send_message mode auto into an idle chat.
    fn send(&self, chat: &str, text: &str) {
        self.core.doc_host.queue_message(chat, text, Vec::new()).expect("queue message");
    }

    /// As Zend MCP send_message with wait: true: the sender awaits this turn.
    async fn send_awaited(&self, chat: &str, text: &str) {
        let request = RunRequest {
            mcp: None,
            prompt: text.into(),
            harness: Some(HarnessId::Mock),
            model: None,
            reasoning: None,
            model_options: Default::default(),
            cwd: format!("/tmp/{chat}"),
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
        };
        let command = SessionCommandPayload::Run { request, message_id: format!("m-{chat}") };
        self.client
            .call(
                methods::QUEUE_COMMAND,
                serde_json::json!({
                    "chatId": chat,
                    "command": command,
                    "awaitedUntilMs": chrono::Utc::now().timestamp_millis() + 60_000,
                }),
            )
            .await
            .expect("queue command");
    }

    fn runs_in(&self, cwd: &str) -> Vec<String> {
        self.runs
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.cwd == cwd)
            .map(|r| r.prompt.clone())
            .collect()
    }

    /// What the parent has been told, one entry per run an event started.
    fn events(&self) -> Vec<String> {
        self.runs_in(PARENT_CWD).into_iter().filter(|p| p.starts_with(EVENT_PREFIX)).collect()
    }

    async fn until(&self, what: &str, mut ok: impl FnMut(&Self) -> bool) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
        while !ok(self) {
            assert!(tokio::time::Instant::now() < deadline, "timed out waiting for {what}");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    /// Long enough for any window to have closed and every event to have run.
    async fn quiet(&self) {
        tokio::time::sleep(WINDOW + Duration::from_millis(2500)).await;
    }
}

/// The parent ends its turn after launching a child; the child completes; the
/// parent hears of it exactly once, in a turn of its own.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_completed_child_tells_its_idle_parent_once() {
    let rig = Rig::new().await;
    rig.send("parent", "launch the worker, then I end my turn");
    rig.until("the parent's turn", |r| r.runs_in(PARENT_CWD).len() == 1).await;
    rig.child("kid", None).await;
    rig.send("kid", "build it");
    rig.until("the event", |r| r.events().len() == 1).await;
    rig.quiet().await;
    let events = rig.events();
    assert_eq!(events.len(), 1, "exactly one event: {events:?}");
    let line = &events[0];
    assert!(!line.starts_with("[Message from Zend chat"), "{line}");
    assert!(line.contains("chat Chat kid (kid) completed."), "{line}");
    assert!(line.contains("Last reply: reply to build it"), "{line}");
    assert!(line.ends_with("Read more: read_chat kid.]"), "{line}");
    println!("EVENT: {line}");
    // The event's own turn (the parent's) tells nobody: no loop.
    assert_eq!(rig.runs_in(PARENT_CWD).len(), 2);
    rig.core.shutdown().await;
}

/// The parent has a visual open (it asks, nothing sent): the event names it,
/// with how many entries the person made, and says to update it rather than
/// decide or ask again. One sent, or another chat's, isn't named.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_open_visual_of_the_parent_is_named_in_the_event() {
    let rig = Rig::new().await;
    let uploads = &rig.core.uploads;
    let inputs = serde_json::json!([
        { "name": "goal", "kind": "field", "label": "Goal" },
        { "name": "when", "kind": "choice", "label": "When" },
    ]);
    let keep = |id: &str, chat: &str, title: &str, sent: bool| {
        let path = uploads.store_visual(id, "return function() return ui.text 'x' end").unwrap();
        let meta = serde_json::json!({
            "id": id, "chatId": chat, "title": title, "sent": sent, "inputs": inputs,
        });
        uploads.store_visual_meta(id, &meta.to_string()).unwrap();
        path
    };
    let open = keep("v:open", "parent", "Migration plan", false);
    uploads
        .store_visual_entries(&open, r#"{"values":{"goal":"ship it","when":"Tonight"}}"#)
        .unwrap();
    keep("v:sent", "parent", "Done already", true);
    keep("v:other", "kid", "Not the parent's", false);
    rig.child("kid", None).await;
    rig.send("kid", "build it");
    rig.until("the event", |r| r.events().len() == 1).await;
    let event = &rig.events()[0];
    let lines: Vec<&str> = event.lines().collect();
    assert_eq!(lines.len(), 2, "{event}");
    assert!(lines[0].contains("chat Chat kid (kid) completed."), "{event}");
    assert_eq!(
        lines[1],
        "Open visual v:open \"Migration plan\": 2 entries so far, not sent. Don't decide its questions or ask them again; update it with update_visual."
    );
    println!("EVENT:\n{event}");
    rig.core.shutdown().await;
}

/// Two children settling together: one message, a line each.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn children_settling_together_are_told_in_one_message() {
    let rig = Rig::new().await;
    rig.child("kid-a", None).await;
    rig.child("kid-b", None).await;
    rig.send("kid-a", "task a");
    rig.send("kid-b", "task b");
    rig.until("the event", |r| !r.events().is_empty()).await;
    rig.quiet().await;
    let events = rig.events();
    assert_eq!(events.len(), 1, "one batched message: {events:?}");
    let lines: Vec<&str> = events[0].lines().collect();
    assert_eq!(lines.len(), 2, "{}", events[0]);
    assert!(events[0].contains("chat Chat kid-a (kid-a) completed. Last reply: reply to task a"));
    assert!(events[0].contains("chat Chat kid-b (kid-b) completed. Last reply: reply to task b"));
    println!("EVENT:\n{}", events[0]);
    rig.core.shutdown().await;
}

/// A sender that awaits the turn (wait: true) gets the reply itself: no event
/// for that child's turn. A second, unawaited child is the control.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_awaited_turn_tells_no_one() {
    let rig = Rig::new().await;
    rig.child("waited", None).await;
    rig.child("free", None).await;
    rig.send_awaited("waited", "awaited work").await;
    rig.send("free", "free work");
    rig.until("the free child's event", |r| !r.events().is_empty()).await;
    rig.quiet().await;
    let events = rig.events();
    assert_eq!(events.len(), 1, "{events:?}");
    assert!(events[0].contains("(free)"), "{}", events[0]);
    assert!(!events[0].contains("(waited)"), "{}", events[0]);
    assert!(rig.runs_in("/tmp/waited").contains(&"awaited work".to_string()), "it did run");
    // The mark covers that turn only: its next turn, unawaited, is told.
    rig.send("waited", "second work");
    rig.until("the second turn's event", |r| r.events().iter().any(|e| e.contains("(waited)"))).await;
    rig.core.shutdown().await;
}

/// notify_parent: false on creation: the parent isn't told. Control: a sibling
/// with the default is.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn notify_parent_false_tells_no_one() {
    let rig = Rig::new().await;
    rig.child("quiet", Some(false)).await;
    rig.child("loud", None).await;
    rig.send("quiet", "quiet work");
    rig.send("loud", "loud work");
    rig.until("the loud child's event", |r| !r.events().is_empty()).await;
    rig.quiet().await;
    let events = rig.events();
    assert_eq!(events.len(), 1, "{events:?}");
    assert!(events[0].contains("(loud)") && !events[0].contains("(quiet)"), "{}", events[0]);
    assert!(rig.runs_in("/tmp/quiet").contains(&"quiet work".to_string()), "it did run");
    rig.core.shutdown().await;
}

/// A child session tells its parent as a subagent does, unsaid; a side chat,
/// unsaid, does not.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_child_session_tells_its_parent_by_default() {
    let rig = Rig::new().await;
    for (id, kind) in [("worker", "session"), ("aside", "side")] {
        let extra = serde_json::json!({ "parentChatId": "parent", "childKind": kind });
        rig.chat(id, &format!("/tmp/{id}"), extra).await;
        rig.send(id, &format!("{id} work"));
    }
    rig.until("the child session's event", |r| !r.events().is_empty()).await;
    rig.until("the side chat's run", |r| !r.runs_in("/tmp/aside").is_empty()).await;
    rig.quiet().await;
    let events = rig.events();
    assert_eq!(events.len(), 1, "{events:?}");
    assert!(events[0].contains("(worker) completed."), "{}", events[0]);
    rig.core.shutdown().await;
}

/// A plugin allowed "messages.send" here, and no more.
struct Allowed;

impl zend_engine::plugins::agents::Sink for Allowed {
    fn grants(&self, _plugin: &str) -> Option<zend_engine::plugins::agents::Grant> {
        Some(zend_engine::plugins::agents::Grant { name: "Automation".into(), allowed: vec!["messages.send".into()] })
    }
    fn to_window(&self, _: &str, _: &str, _: &str, _: serde_json::Value) {}
    fn to_service(
        &self,
        _: &str,
        _: &str,
        _: &str,
        _: serde_json::Value,
    ) -> futures::future::BoxFuture<'static, Result<(), String>> {
        Box::pin(async { Ok(()) })
    }
}

/// A plugin's `zend.new_chat { parent, kind = "session", title }` makes a
/// child session as the Zend MCP's create_chat does: linked on its row,
/// titled, and its settled turn told to its parent. A parent that is itself a
/// child, or none, is refused.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_plugin_s_child_session_is_linked_and_tells_its_parent() {
    let rig = Rig::new().await;
    let agents = rig.core.plugins.agents();
    agents.set_sink(Arc::new(Allowed));
    let new_chat = |extra: serde_json::Value| {
        let mut args = serde_json::json!({
            "op": "new_chat", "prompt": "worker work", "device": rig.core.device_id, "cwd": "/tmp/worker",
        });
        args.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        agents.call("automation", args)
    };
    let made = new_chat(serde_json::json!({ "parent": "parent", "kind": "session", "title": "Worker" }))
        .await
        .expect("a child session");
    let id = made["chat"].as_str().unwrap().to_owned();
    let row = rig.core.workspace.read_chats().unwrap().into_iter().find(|c| c.id == id).expect("its row");
    assert_eq!(row.parent_chat_id.as_deref(), Some("parent"));
    assert_eq!(row.child_kind, Some(zend_proto::ChildKind::Session));
    assert_eq!(row.title.as_deref(), Some("Worker"));
    rig.until("the child session's event", |r| !r.events().is_empty()).await;
    let event = &rig.events()[0];
    assert!(event.contains(&format!("chat Worker ({}) completed.", &id[..8])), "{event}");
    assert!(event.ends_with(&format!("read_chat {id}.]")), "{event}");
    for (extra, why) in [
        (serde_json::json!({ "parent": id, "kind": "session" }), "itself a child chat"),
        (serde_json::json!({ "parent": "nobody", "kind": "session" }), "There's no chat nobody"),
        (serde_json::json!({ "parent": "parent", "kind": "orphan" }), "isn't a kind"),
    ] {
        let refused = new_chat(extra.clone()).await.unwrap_err();
        assert!(refused.message.contains(why), "{extra}: {refused:?}");
    }
    rig.core.shutdown().await;
}

/// The rig's engine, with the plugin "automation" allowed `messages.send`.
fn allowed_agents(rig: &Rig) -> &zend_engine::plugins::agents::Agents {
    let agents = rig.core.plugins.agents();
    agents.set_sink(Arc::new(Allowed));
    agents
}

/// With neither device nor project, a plugin's child runs on this device in
/// a scratch folder of its own, as the Zend MCP's create_chat makes it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_plugin_s_child_without_device_or_project_runs_here() {
    let rig = Rig::new().await;
    let agents = allowed_agents(&rig);
    let result = agents.call("automation", serde_json::json!({
        "op": "new_chat", "prompt": "work", "parent": "parent", "kind": "session", "title": "Worker"
    })).await;
    println!("minimal child result = {result:?}");
    let made = result.expect("documented optional device/project must allow a local child");
    let row = rig.core.workspace.chat(made["chat"].as_str().unwrap()).unwrap().unwrap();
    rig.core.shutdown().await;
    assert_eq!(row.device_id, rig.core.device_id);
    assert_eq!(row.space_id, None);
    assert!(zend_proto::projectless::is_scratch_cwd(row.cwd.as_deref().unwrap_or_default()), "{:?}", row.cwd);
}

/// `access` reaches the child's config and the run its harness starts.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_plugin_s_child_runs_with_the_access_it_asks() {
    let rig = Rig::new().await;
    let agents = allowed_agents(&rig);
    let result = agents.call("automation", serde_json::json!({
        "op": "new_chat", "prompt": "work", "device": rig.core.device_id, "cwd": "/tmp/worker",
        "parent": "parent", "kind": "session", "title": "Worker", "access": "supervised"
    })).await.unwrap();
    let id = result["chat"].as_str().unwrap();
    let config = rig.core.workspace.chat(id).unwrap().unwrap().config.unwrap();
    println!("requested supervised, persisted config = {config:?}");
    rig.until("child run", |r| r.runs.lock().unwrap().iter().any(|run| run.prompt == "work")).await;
    let access = rig.runs.lock().unwrap().iter().find(|r| r.prompt == "work").unwrap().access;
    println!("actual harness run access = {access:?}");
    rig.core.shutdown().await;
    assert_eq!(config.access, zend_proto::AccessMode::Supervised);
    assert_eq!(access, zend_proto::AccessMode::Supervised);
}

/// A link field or access that is there but says nothing is refused, as the
/// Zend MCP's create_chat refuses it, and makes no chat.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_plugin_s_malformed_link_fields_are_refused_like_mcp() {
    let rig = Rig::new().await;
    let agents = allowed_agents(&rig);
    let tools = zend_mcp::Tools::new(Arc::new(zend_mcp::Zend::with_client(
        zend_rpc::memory_client(rig.core.rpc_service()), zend_mcp::Origin::default(),
    )));
    let before = rig.core.workspace.read_chats().unwrap().len();
    let mut accepted = Vec::new();
    for extra in [
        serde_json::json!({"parent": "parent", "kind": 42}),
        serde_json::json!({"parent": "parent", "kind": false}),
        serde_json::json!({"parent": "parent", "kind": {}}),
        serde_json::json!({"parent": "parent", "kind": ""}),
        serde_json::json!({"parent": "parent", "kind": "  "}),
        serde_json::json!({"parent": 42}),
        serde_json::json!({"parent": "parent", "kind": "session", "access": "everything"}),
        serde_json::json!({"parent": "parent", "kind": "session", "access": 1}),
    ] {
        let mut args = serde_json::json!({"op": "new_chat", "prompt": "work", "device": rig.core.device_id, "title": "Worker"});
        args.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        let mcp = tools.call("create_chat", args.clone()).await;
        assert!(mcp.is_err(), "MCP refuses malformed fields: {extra}");
        let plugin = agents.call("automation", args).await;
        println!("fields={extra}: MCP={mcp:?}, plugin={plugin:?}");
        match plugin {
            Ok(made) => {
                let row = rig.core.workspace.chat(made["chat"].as_str().unwrap()).unwrap().unwrap();
                accepted.push(format!("{extra} -> parent={:?}, kind={:?}", row.parent_chat_id, row.child_kind));
            }
            Err(refused) => assert_eq!(refused.reason, "bad_call", "{extra}: {refused:?}"),
        }
    }
    let after = rig.core.workspace.read_chats().unwrap().len();
    rig.core.shutdown().await;
    assert!(accepted.is_empty(), "malformed link fields created chats:\n{}", accepted.join("\n"));
    assert_eq!(after, before, "a refused call makes nothing");
}

/// A Cloud chat's children start from Cloud: a plugin naming one as parent
/// is refused, and nothing is made (Cloud child sessions are out of scope).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_plugin_cannot_start_a_cloud_chat_s_child() {
    let rig = Rig::new().await;
    let agents = allowed_agents(&rig);
    rig.core.workspace.create_chat_hosted(
        "cloud-parent", None, Some("cloud-parent-machine"), Some("cloud-parent-machine"),
        None, Some("/home/user/app".into()), None, None,
    ).unwrap();
    let before = rig.core.workspace.read_chats().unwrap().len();
    let mut refusals = Vec::new();
    for kind in ["session", "subagent", "side"] {
        refusals.push(agents.call("automation", serde_json::json!({
            "op": "new_chat", "prompt": "work", "device": rig.core.device_id,
            "parent": "cloud-parent", "kind": kind, "title": "Worker",
        })).await);
    }
    let after = rig.core.workspace.read_chats().unwrap().len();
    rig.core.shutdown().await;
    for refused in refusals {
        let refused = refused.expect_err("a Cloud parent is refused");
        assert_eq!(refused.reason, "bad_call");
        assert!(refused.message.contains("start from Cloud"), "{refused:?}");
    }
    assert_eq!(after, before, "a refused call makes nothing");
}

/// An archived parent still takes children; a blank title leaves the child
/// untitled; a deleted parent and the voice orchestrator are no parents.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_plugin_s_child_takes_an_archived_parent_and_ignores_a_blank_title() {
    let rig = Rig::new().await;
    let agents = allowed_agents(&rig);
    rig.core.workspace.set_chat_archived("parent", true).unwrap();
    for title in ["", "   "] {
        let result = agents.call("automation", serde_json::json!({
            "op": "new_chat", "prompt": "work", "device": rig.core.device_id,
            "parent": "parent", "kind": "session", "title": title,
        })).await.unwrap();
        let row = rig.core.workspace.chat(result["chat"].as_str().unwrap()).unwrap().unwrap();
        assert!(row.title.is_none());
        assert_eq!(row.parent_chat_id.as_deref(), Some("parent"));
    }
    rig.core.workspace.delete_chat("parent").unwrap();
    let result = agents.call("automation", serde_json::json!({
        "op": "new_chat", "prompt": "work", "device": rig.core.device_id, "parent": "parent", "kind": "session"
    })).await;
    assert_eq!(result.unwrap_err().reason, "not_found");
    rig.chat("voice-orchestrator-review", "/tmp/voice", serde_json::json!({})).await;
    let result = agents.call("automation", serde_json::json!({
        "op": "new_chat", "prompt": "work", "device": rig.core.device_id, "parent": "voice-orchestrator-review", "kind": "session"
    })).await;
    assert_eq!(result.unwrap_err().reason, "not_found");
    rig.core.shutdown().await;
}

/// A child that asks tells its parent the question and the request to answer.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_child_awaiting_input_tells_its_parent_the_question() {
    let rig = Rig::new().await;
    rig.child("asker", None).await;
    rig.send("asker", "ask which database");
    rig.until("the event", |r| !r.events().is_empty()).await;
    rig.quiet().await;
    let events = rig.events();
    assert_eq!(events.len(), 1, "{events:?}");
    let line = &events[0];
    assert!(line.contains("chat Chat asker (asker) is waiting for input."), "{line}");
    assert!(line.contains("It asks: Which database? [pg / sqlite] (request "), "{line}");
    assert!(line.contains("answer with respond_to_input"), "{line}");
    println!("EVENT: {line}");
    rig.core.shutdown().await;
}

/// A parent still working takes the event through its live mailbox, not as a
/// run of its own: one steer, no new turn.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_working_parent_is_steered() {
    let rig = Rig::new().await;
    rig.send("parent", "hold on, children are working");
    rig.until("the parent's run", |r| r.runs_in(PARENT_CWD).len() == 1).await;
    rig.child("kid", None).await;
    rig.send("kid", "build it");
    rig.until("the steer", |r| !r.steered.lock().unwrap().is_empty()).await;
    rig.quiet().await;
    let steered = rig.steered.lock().unwrap().clone();
    assert_eq!(steered.len(), 1, "{steered:?}");
    assert!(steered[0].starts_with(EVENT_PREFIX) && steered[0].contains("(kid) completed."), "{}", steered[0]);
    assert_eq!(rig.runs_in(PARENT_CWD).len(), 1, "no run of its own");
    println!("STEERED: {}", steered[0]);
    rig.core.shutdown().await;
}
