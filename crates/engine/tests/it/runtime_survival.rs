//! A live runtime is the agent's whole process — its background subagents and
//! background shells included. Sending the next message, steering, "send
//! now", switching models, or leaving a session parked must never tear it
//! down behind the user's back (2026-09-29: "sometimes sending a message
//! closes the existing claude session and re-opens it … it kills subagents
//! and sub processes"). The user's Stop is the one deliberate teardown.
//!
//! The harness here models one persistent agent process per `run()`: every
//! `run()` is a spawn, the `interrupt` token is its death, a turn stop ends
//! only the turn, and the test decides when the "background task" finishes.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use futures::stream::BoxStream;
use tokio::sync::mpsc;

use zend_doc::{
    CommandBasedOn, MessageStatus, SessionCommandEntry, SessionCommandPayload,
    SessionCommandStatus,
};
use zend_engine::{EngineCore, HarnessRegistry};
use zend_harness::{Harness, HarnessError, RunControls, TurnControl};
use zend_proto::{
    AgentEvent, DoneStatus, HarnessId, Model, ReasoningLevel, RunRequest, SandboxLevel,
    SessionStatus, SteeringMode,
};

const CHAT: &str = "chat-runtime-survival";

#[derive(Default)]
struct Ledger {
    spawns: AtomicUsize,
    kills: AtomicUsize,
    /// Every prompt the live process received, with its image count.
    prompts: Mutex<Vec<(String, usize)>>,
    /// The latest runtime's turn control (the test flips its background work).
    turn: Mutex<Option<TurnControl>>,
    /// The model each in-place reconfiguration switched the process to.
    reconfigured: Mutex<Vec<Option<String>>>,
    /// The model each process was spawned with.
    spawned_models: Mutex<Vec<Option<String>>>,
    /// Ends a turn whose prompt contains `wait`.
    release: tokio::sync::Notify,
    /// Stop requests that reached a turn whose prompt contains
    /// `delay-stop`: it acknowledges them only on `stop_release`.
    stop_reached: AtomicUsize,
    stop_release: tokio::sync::Notify,
    /// While set, a dispatch looking up the harness's binary (after it
    /// wrote its row, before its runtime is registered) stalls there until
    /// the test releases `binary_release`.
    binary_gate: AtomicBool,
    binary_reached: AtomicUsize,
    binary_release: (Mutex<bool>, std::sync::Condvar),
}

/// One persistent agent process per `run()`. A prompt containing `hold`
/// stays in flight until stopped; any other prompt completes at once. A
/// steer containing `vanish` kills the process before it confirms the steer.
/// A prompt containing `wait` runs until the test releases it. One containing
/// `delay-stop` acknowledges a stop request only when the test releases it.
struct ProcessHarness {
    ledger: Arc<Ledger>,
    /// Model changes apply to the live process (`set_model`-style).
    adopts_config: bool,
    /// Takes input only between turns (Hermes, Antigravity): the engine
    /// holds sends made mid-turn for the turn's end.
    turn_boundary: bool,
}

fn done(status: DoneStatus) -> AgentEvent {
    AgentEvent::Done {
        status,
        result: None,
        error: None,
        session_id: Some("provider-session".into()),
    }
}

#[async_trait]
impl Harness for ProcessHarness {
    fn id(&self) -> HarnessId {
        HarnessId::Mock
    }
    fn display_name(&self) -> &str {
        "Process"
    }
    fn executable_path(&self) -> Option<std::path::PathBuf> {
        if self.ledger.binary_gate.load(Ordering::SeqCst) {
            self.ledger.binary_reached.fetch_add(1, Ordering::SeqCst);
            let (released, wake) = &self.ledger.binary_release;
            let released = released.lock().unwrap();
            let _released = wake
                .wait_timeout_while(released, Duration::from_secs(10), |r| !*r)
                .unwrap();
        }
        None
    }
    fn supports_steering(&self) -> bool {
        true
    }
    fn steering_mode(&self) -> SteeringMode {
        if self.turn_boundary {
            SteeringMode::TurnBoundary
        } else {
            SteeringMode::StepBoundary
        }
    }
    fn reasoning_levels(&self) -> &[ReasoningLevel] {
        &[ReasoningLevel::Medium, ReasoningLevel::High]
    }
    fn deterministic_turn_end(&self) -> bool {
        true
    }
    fn stops_turn_in_place(&self) -> bool {
        true
    }
    fn reconfigures_in_place(&self, live: &RunRequest, next: &RunRequest) -> bool {
        self.adopts_config && live.cwd == next.cwd
    }
    async fn models(&self) -> Result<Vec<Model>, HarnessError> {
        Ok(vec![])
    }
    async fn run(
        &self,
        request: RunRequest,
        mut controls: RunControls,
    ) -> Result<BoxStream<'static, Result<AgentEvent, HarnessError>>, HarnessError> {
        let ledger = self.ledger.clone();
        ledger.spawns.fetch_add(1, Ordering::SeqCst);
        ledger
            .spawned_models
            .lock()
            .unwrap()
            .push(request.model.clone());
        *ledger.turn.lock().unwrap() = Some(controls.turn.clone());
        let (tx, rx) = mpsc::channel::<Result<AgentEvent, HarnessError>>(64);
        tokio::spawn(async move {
            let turn = controls.turn.clone();
            let send = |event: AgentEvent| {
                let tx = tx.clone();
                async move { tx.send(Ok(event)).await.is_ok() }
            };
            let prompt = |text: &str, images: usize| {
                ledger
                    .prompts
                    .lock()
                    .unwrap()
                    .push((text.to_owned(), images));
                text.contains("hold")
            };
            let _ = send(AgentEvent::SessionStarted {
                harness: HarnessId::Mock,
                model: "process".into(),
                tools: vec![],
                cwd: request.cwd.clone(),
                session_id: "provider-session".into(),
                assistant_message_id: "a0".into(),
            })
            .await;
            let holding = prompt(&request.prompt, request.attachments.len());
            let _ = send(AgentEvent::TextDelta {
                text: format!("on it: {}", request.prompt),
            })
            .await;
            let mut waiting = request.prompt.contains("wait");
            if !holding && !waiting && !send(done(DoneStatus::Completed)).await {
                return;
            }
            loop {
                tokio::select! {
                    biased;
                    _ = ledger.release.notified(), if waiting => {
                        waiting = false;
                        if !send(done(DoneStatus::Completed)).await {
                            return;
                        }
                    }
                    _ = controls.interrupt.cancelled() => {
                        // The process dies, and its background work with it.
                        ledger.kills.fetch_add(1, Ordering::SeqCst);
                        let _ = send(done(DoneStatus::Interrupted)).await;
                        return;
                    }
                    _ = turn.stop_requested() => {
                        if request.prompt.contains("delay-stop") {
                            ledger.stop_reached.fetch_add(1, Ordering::SeqCst);
                            ledger.stop_release.notified().await;
                        }
                        if !send(done(DoneStatus::Interrupted)).await {
                            return;
                        }
                    }
                    steer = controls.steering.recv() => {
                        let Some(steer) = steer else { return };
                        if steer.prompt.contains("vanish") {
                            return;
                        }
                        if let Some(config) = &steer.config {
                            ledger.reconfigured.lock().unwrap().push(config.model.clone());
                        }
                        let holding = prompt(&steer.prompt, steer.attachments.len());
                        let boundary = AgentEvent::Steered {
                            assistant_message_id: None,
                            next_assistant_message_id: None,
                        };
                        if !send(boundary).await
                            || !send(AgentEvent::TextDelta { text: format!("on it: {}", steer.prompt) }).await
                        {
                            return;
                        }
                        waiting = steer.prompt.contains("wait");
                        if !holding && !waiting && !send(done(DoneStatus::Completed)).await {
                            return;
                        }
                    }
                }
            }
        });
        Ok(
            futures::stream::unfold(rx, |mut rx| async move { rx.recv().await.map(|e| (e, rx)) })
                .boxed(),
        )
    }
}

struct Rig {
    core: EngineCore,
    ledger: Arc<Ledger>,
    _dir: tempfile::TempDir,
}

fn rig() -> Rig {
    rig_with(false)
}

fn rig_with(adopts_config: bool) -> Rig {
    rig_full(adopts_config, false)
}

fn rig_full(adopts_config: bool, turn_boundary: bool) -> Rig {
    let ledger = Arc::new(Ledger::default());
    let registry = HarnessRegistry::new();
    registry.register(Arc::new(ProcessHarness {
        ledger: ledger.clone(),
        adopts_config,
        turn_boundary,
    }));
    let dir = tempfile::tempdir().unwrap();
    let core = EngineCore::assemble(dir.path(), Arc::new(registry), HarnessId::Mock, None)
        .expect("engine core assembles");
    Rig {
        core,
        ledger,
        _dir: dir,
    }
}

fn request(prompt: &str) -> RunRequest {
    RunRequest {
        mcp: None,
        env: Default::default(),
        prompt: prompt.into(),
        harness: Some(HarnessId::Mock),
        model: Some("process-1".into()),
        reasoning: Some(ReasoningLevel::High),
        model_options: serde_json::Map::from_iter([("fastMode".into(), serde_json::json!(true))]),
        cwd: "/tmp".into(),
        sandbox: SandboxLevel::WorkspaceWrite,
        auto_approve: false,
        access: Default::default(),
        plan: false,
        attachments: Vec::new(),
        worktree: None,
        resume: None,
        extra_mcp: Vec::new(),
        instructions: Vec::new(),
    }
}

fn status(core: &EngineCore) -> Option<SessionStatus> {
    core.sessions.session_status(CHAT).map(|s| s.status)
}

async fn wait_for(mut predicate: impl FnMut() -> bool, what: &str) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while !predicate() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for {what}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn settle(rig: &Rig, prompts: usize) {
    wait_for(
        || {
            status(&rig.core) == Some(SessionStatus::Idle)
                && rig.ledger.prompts.lock().unwrap().len() >= prompts
        },
        "the turn to park",
    )
    .await;
}

#[tokio::test]
async fn sends_with_images_or_respelled_options_join_the_live_runtime() {
    let rig = rig();
    rig.core
        .sessions
        .dispatch(CHAT, HarnessId::Mock, request("launch the scouts"), None)
        .await
        .unwrap();
    settle(&rig, 1).await;

    // The phone spells the same option as a string, and the message carries
    // an image: neither is a different runtime.
    let mut follow_up = request("what is in this screenshot?");
    follow_up.model_options =
        serde_json::Map::from_iter([("fastMode".into(), serde_json::json!("true"))]);
    follow_up.sandbox = SandboxLevel::WorkspaceWrite;
    follow_up.attachments = vec!["/tmp/shot.png".into()];
    rig.core
        .sessions
        .dispatch(CHAT, HarnessId::Mock, follow_up, None)
        .await
        .unwrap();
    settle(&rig, 2).await;

    // Through the durable command plane too (the desktop/mobile send path).
    rig.core
        .doc_host
        .queue_command(
            CHAT,
            SessionCommandPayload::Run {
                request: request("and one more"),
                message_id: "m-3".into(),
            },
        )
        .unwrap();
    settle(&rig, 3).await;

    assert_eq!(rig.ledger.spawns.load(Ordering::SeqCst), 1, "one process");
    assert_eq!(rig.ledger.kills.load(Ordering::SeqCst), 0);
    assert_eq!(
        rig.ledger.prompts.lock().unwrap()[1],
        ("what is in this screenshot?".to_string(), 1),
        "the image rode the mailbox"
    );

    // A real configuration change still gets a fresh runtime.
    let mut other_model = request("switch models");
    other_model.model = Some("process-2".into());
    rig.core
        .sessions
        .dispatch(CHAT, HarnessId::Mock, other_model, None)
        .await
        .unwrap();
    settle(&rig, 4).await;
    assert_eq!(rig.ledger.spawns.load(Ordering::SeqCst), 2);
    rig.core.sessions.shutdown().await;
}

/// Switching models mid-chat on a driver that adopts configuration in place
/// (Claude's `set_model`/`apply_flag_settings`, Codex's per-turn params)
/// keeps the process — and every background subagent and shell it holds.
#[tokio::test]
async fn a_model_switch_is_adopted_by_the_live_runtime() {
    let rig = rig_with(true);
    rig.core
        .sessions
        .dispatch(CHAT, HarnessId::Mock, request("launch the scouts"), None)
        .await
        .unwrap();
    settle(&rig, 1).await;

    let mut other_model = request("switch models");
    other_model.model = Some("process-2".into());
    other_model.reasoning = Some(ReasoningLevel::Medium);
    rig.core
        .sessions
        .dispatch(CHAT, HarnessId::Mock, other_model, None)
        .await
        .unwrap();
    settle(&rig, 2).await;

    // The same model again is the (now reconfigured) same runtime: no
    // second reconfiguration rides along.
    let mut again = request("still on the new model");
    again.model = Some("process-2".into());
    again.reasoning = Some(ReasoningLevel::Medium);
    rig.core
        .sessions
        .dispatch(CHAT, HarnessId::Mock, again, None)
        .await
        .unwrap();
    settle(&rig, 3).await;

    assert_eq!(rig.ledger.spawns.load(Ordering::SeqCst), 1, "one process");
    assert_eq!(rig.ledger.kills.load(Ordering::SeqCst), 0);
    assert_eq!(
        *rig.ledger.reconfigured.lock().unwrap(),
        vec![Some("process-2".to_string())]
    );

    // A different directory still needs a different process.
    let mut elsewhere = request("over there");
    elsewhere.cwd = "/var".into();
    rig.core
        .sessions
        .dispatch(CHAT, HarnessId::Mock, elsewhere, None)
        .await
        .unwrap();
    settle(&rig, 4).await;
    assert_eq!(rig.ledger.spawns.load(Ordering::SeqCst), 2);
    rig.core.sessions.shutdown().await;
}

/// A message the runtime accepted but died before confirming is replayed as
/// it was sent — its images and configuration included, under its own id —
/// not as the chat's latest request with the images stripped.
#[tokio::test]
async fn a_message_orphaned_by_a_dying_runtime_replays_as_sent() {
    let rig = rig_with(true);
    rig.core
        .sessions
        .dispatch(CHAT, HarnessId::Mock, request("hold: working"), None)
        .await
        .unwrap();
    wait_for(
        || rig.ledger.prompts.lock().unwrap().len() == 1,
        "the agent to be running the turn",
    )
    .await;

    let mut sent = request("vanish: what is in this screenshot?");
    sent.model = Some("process-2".into());
    sent.attachments = vec!["/tmp/shot.png".into()];
    rig.core
        .sessions
        .dispatch(CHAT, HarnessId::Mock, sent, Some("m-orphan".into()))
        .await
        .unwrap();
    settle(&rig, 2).await;

    assert_eq!(
        rig.ledger.spawns.load(Ordering::SeqCst),
        2,
        "a fresh runtime"
    );
    assert_eq!(
        rig.ledger.prompts.lock().unwrap()[1],
        ("vanish: what is in this screenshot?".to_string(), 1),
        "the image survived the replay"
    );
    assert_eq!(
        rig.ledger.spawned_models.lock().unwrap()[1].as_deref(),
        Some("process-2"),
        "the configuration it was sent with"
    );
    let entries = rig
        .core
        .doc_host
        .open(CHAT)
        .unwrap()
        .doc()
        .read_entries()
        .unwrap();
    let orphan: Vec<_> = entries
        .iter()
        .filter(|e| e.role == zend_doc::MessageRole::User && e.id != entries[0].id)
        .map(|e| e.id.as_str())
        .collect();
    assert_eq!(orphan, ["m-orphan"], "one entry, under its own id");
    rig.core.sessions.shutdown().await;
}

/// Every distinct completion ping the chat publishes, recorded until the
/// returned handle is aborted.
fn record_pings(rig: &Rig) -> (Arc<Mutex<Vec<String>>>, tokio::task::JoinHandle<()>) {
    let pings = Arc::new(Mutex::new(Vec::<String>::new()));
    let sessions = rig.core.sessions.clone();
    let seen = pings.clone();
    let task = tokio::spawn(async move {
        loop {
            if let Some(ping) = sessions
                .session_status(CHAT)
                .and_then(|s| s.last_completed_turn)
            {
                let mut seen = seen.lock().unwrap();
                if seen.last() != Some(&ping) {
                    seen.push(ping);
                }
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    });
    (pings, task)
}

fn queued(rig: &Rig) -> Vec<String> {
    rig.core
        .doc_host
        .open(CHAT)
        .unwrap()
        .watch_queue()
        .borrow()
        .iter()
        .map(|row| row.text.clone())
        .collect()
}

/// A send to a turn-boundary agent mid-turn waits for the turn's end — and
/// then runs as it was sent (its image, its model), with no "done" ping for
/// the turn that hands over to it.
#[tokio::test]
async fn a_send_held_for_the_turn_end_runs_as_sent_without_a_ping() {
    let rig = rig_full(true, true);
    let (pings, recorder) = record_pings(&rig);
    rig.core
        .sessions
        .dispatch(CHAT, HarnessId::Mock, request("wait: first"), None)
        .await
        .unwrap();
    wait_for(
        || rig.ledger.prompts.lock().unwrap().len() == 1,
        "the first turn to run",
    )
    .await;
    let mut sent = request("second");
    sent.model = Some("process-2".into());
    sent.attachments = vec!["/tmp/shot.png".into()];
    rig.core
        .doc_host
        .queue_command(
            CHAT,
            SessionCommandPayload::Run {
                request: sent,
                message_id: "m-held".into(),
            },
        )
        .unwrap();
    wait_for(|| queued(&rig) == ["second"], "the send to be held").await;

    rig.ledger.release.notify_one();
    settle(&rig, 2).await;
    wait_for(
        || {
            rig.core
                .sessions
                .session_status(CHAT)
                .and_then(|s| s.last_completed_turn)
                .is_some()
        },
        "the held send's reply to complete",
    )
    .await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    recorder.abort();
    assert_eq!(
        rig.ledger.prompts.lock().unwrap()[1],
        ("second".to_string(), 1),
        "the held send kept its image"
    );
    assert_eq!(
        *rig.ledger.reconfigured.lock().unwrap(),
        vec![Some("process-2".to_string())],
        "the held send ran on the model it was sent with"
    );
    assert_eq!(rig.ledger.spawns.load(Ordering::SeqCst), 1);
    let last_reply = rig
        .core
        .doc_host
        .open(CHAT)
        .unwrap()
        .doc()
        .read_entries()
        .unwrap()
        .last()
        .map(|e| e.id.clone())
        .unwrap();
    assert_eq!(
        *pings.lock().unwrap(),
        vec![last_reply],
        "only the held send's reply pings: the handover to it is not a completion"
    );
    rig.core.sessions.shutdown().await;
}

/// A Stop cancels what was sent during the turn it stops: a send held for
/// that turn's end never runs — not even after the user's next message
/// thaws the queue.
#[tokio::test]
async fn stop_cancels_sends_held_for_the_stopped_turn() {
    let rig = rig_full(false, true);
    rig.core
        .sessions
        .dispatch(CHAT, HarnessId::Mock, request("wait: first"), None)
        .await
        .unwrap();
    wait_for(
        || rig.ledger.prompts.lock().unwrap().len() == 1,
        "the first turn to run",
    )
    .await;
    rig.core
        .doc_host
        .queue_command(
            CHAT,
            SessionCommandPayload::Run {
                request: request("held before the stop"),
                message_id: "m-held".into(),
            },
        )
        .unwrap();
    wait_for(
        || queued(&rig) == ["held before the stop"],
        "the send to be held",
    )
    .await;
    rig.core
        .doc_host
        .queue_command(CHAT, SessionCommandPayload::Interrupt { cancels: None })
        .unwrap();
    wait_for(
        || {
            rig.ledger.kills.load(Ordering::SeqCst) == 1
                && status(&rig.core) == Some(SessionStatus::Idle)
                && queued(&rig).is_empty()
        },
        "the stop to cancel the held send",
    )
    .await;
    rig.core
        .doc_host
        .queue_command(
            CHAT,
            SessionCommandPayload::Run {
                request: request("after the stop"),
                message_id: "m-after".into(),
            },
        )
        .unwrap();
    wait_for(
        || {
            rig.ledger
                .prompts
                .lock()
                .unwrap()
                .iter()
                .any(|(p, _)| p == "after the stop")
        },
        "the next message to run",
    )
    .await;
    settle(&rig, 2).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        !rig.ledger
            .prompts
            .lock()
            .unwrap()
            .iter()
            .any(|(p, _)| p.contains("held before the stop")),
        "a send held for the stopped turn ran after the Stop"
    );
    rig.core.sessions.shutdown().await;
}

/// A message sent before a Stop whose dispatch only lands once the Stop
/// has torn the runtime down (a loaded host, a slow agent) was cancelled by
/// it: it must not start the stopped work again in a fresh runtime.
#[tokio::test]
async fn a_message_the_stop_cancelled_never_reaches_a_fresh_runtime() {
    let rig = rig();
    rig.core
        .sessions
        .dispatch(CHAT, HarnessId::Mock, request("hold: working"), None)
        .await
        .unwrap();
    wait_for(
        || rig.ledger.prompts.lock().unwrap().len() == 1,
        "the agent to be running the turn",
    )
    .await;
    // The Stop acts with the message still on its way …
    rig.core
        .sessions
        .cancel_sent_messages(CHAT, vec!["m-late".into()]);
    rig.core.sessions.interrupt(CHAT).await.unwrap();
    // … and the message reaches dispatch only now.
    rig.core
        .sessions
        .dispatch(
            CHAT,
            HarnessId::Mock,
            request("hold: sent before the stop"),
            Some("m-late".into()),
        )
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        rig.ledger.spawns.load(Ordering::SeqCst),
        1,
        "no fresh runtime"
    );
    assert_eq!(status(&rig.core), Some(SessionStatus::Idle));
    // The user's next message still runs.
    rig.core
        .sessions
        .dispatch(
            CHAT,
            HarnessId::Mock,
            request("after the stop"),
            Some("m-next".into()),
        )
        .await
        .unwrap();
    settle(&rig, 2).await;
    assert_eq!(rig.ledger.prompts.lock().unwrap()[1].0, "after the stop");
    rig.core.sessions.shutdown().await;
}

/// "Send now" replaces the in-flight turn but keeps the runtime and its
/// background work; the user's Stop is a hard boundary that tears all of it
/// down, and the next message starts a fresh process.
#[tokio::test]
async fn send_now_keeps_the_runtime_and_stop_tears_it_down() {
    let rig = rig();
    rig.core
        .sessions
        .dispatch(CHAT, HarnessId::Mock, request("hold: build it"), None)
        .await
        .unwrap();
    wait_for(
        || rig.ledger.turn.lock().unwrap().is_some(),
        "the runtime to start",
    )
    .await;
    let turn = rig.ledger.turn.lock().unwrap().clone().unwrap();
    turn.set_background(1);
    wait_for(
        || status(&rig.core) == Some(SessionStatus::Working),
        "the turn to run",
    )
    .await;

    // Send now: the turn ends in place.
    assert!(rig.core.sessions.stop_turn(CHAT).await.unwrap());
    assert_eq!(status(&rig.core), Some(SessionStatus::Idle));
    let doc = rig.core.doc_host.open(CHAT).unwrap();
    assert!(
        doc.doc()
            .read_entries()
            .unwrap()
            .iter()
            .any(|entry| entry.status == Some(MessageStatus::Aborted)),
        "the replaced turn reads as stopped"
    );
    assert_eq!(rig.ledger.kills.load(Ordering::SeqCst), 0, "no teardown");
    assert!(
        !rig.core.sessions.stop_turn(CHAT).await.unwrap(),
        "a parked runtime has no turn to stop"
    );
    rig.core
        .sessions
        .dispatch(CHAT, HarnessId::Mock, request("hold: again"), None)
        .await
        .unwrap();
    wait_for(
        || {
            status(&rig.core) == Some(SessionStatus::Working)
                && rig.ledger.prompts.lock().unwrap().len() == 2
        },
        "the second turn to run",
    )
    .await;
    assert_eq!(rig.ledger.spawns.load(Ordering::SeqCst), 1, "one process");
    assert!(turn.background_live());

    // Stop (the Cancel command): the runtime and its work are gone.
    rig.core
        .doc_host
        .queue_command(CHAT, SessionCommandPayload::Interrupt { cancels: None })
        .unwrap();
    wait_for(
        || status(&rig.core) == Some(SessionStatus::Idle),
        "stop to settle",
    )
    .await;
    wait_for(
        || rig.ledger.kills.load(Ordering::SeqCst) == 1,
        "the runtime to be torn down",
    )
    .await;
    assert!(!rig.core.sessions.live_run_steerable(CHAT));

    // The next message starts a fresh process.
    rig.core
        .sessions
        .dispatch(CHAT, HarnessId::Mock, request("status?"), None)
        .await
        .unwrap();
    settle(&rig, 3).await;
    assert_eq!(rig.ledger.spawns.load(Ordering::SeqCst), 2);
    rig.core.sessions.shutdown().await;
}

/// Stop pressed right after Send: controls bypass the prompt drain, and the
/// Stop used to run before the send had dispatched — it found no turn, did
/// nothing, and the turn then ran in full. The Stop names the send (its row
/// isn't written yet, ADR 0007), so the send gives up as the last step before
/// its runtime starts: it never reaches the agent, and there is nothing to
/// kill. (Before the Stop stopped waiting for sends, it waited for this one
/// to start and killed it: one prompt, one kill.)
#[tokio::test]
async fn a_stop_right_after_send_stops_that_turn() {
    let rig = rig();
    rig.core
        .doc_host
        .queue_command(
            CHAT,
            SessionCommandPayload::Run {
                request: request("hold: the essay"),
                message_id: "m-1".into(),
            },
        )
        .unwrap();
    rig.core
        .doc_host
        .queue_command(CHAT, SessionCommandPayload::Interrupt { cancels: None })
        .unwrap();
    wait_for(
        || has_row(&rig, "m-1"),
        "the send to land in the transcript",
    )
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    // No runtime started, so the chat has no live status at all.
    assert_eq!(status(&rig.core), None, "the chat was left working");
    assert!(
        rig.ledger.prompts.lock().unwrap().is_empty(),
        "the send the Stop named reached the agent"
    );
    assert_eq!(rig.ledger.kills.load(Ordering::SeqCst), 0);
    rig.core.sessions.shutdown().await;
}

/// Send pressed right after Stop: it used to land in the mailbox of the
/// runtime the Stop was tearing down, and an interrupted run re-dispatches
/// nothing — the message showed as sent and never ran.
#[tokio::test]
async fn a_send_right_after_stop_runs_in_a_fresh_runtime() {
    let rig = rig();
    rig.core
        .sessions
        .dispatch(CHAT, HarnessId::Mock, request("hold: working"), None)
        .await
        .unwrap();
    wait_for(
        || rig.ledger.prompts.lock().unwrap().len() == 1,
        "the agent to be running the turn",
    )
    .await;
    rig.core
        .doc_host
        .queue_command(CHAT, SessionCommandPayload::Interrupt { cancels: None })
        .unwrap();
    rig.core
        .doc_host
        .queue_command(
            CHAT,
            SessionCommandPayload::Run {
                request: request("after the stop"),
                message_id: "m-2".into(),
            },
        )
        .unwrap();
    wait_for(
        || {
            rig.ledger
                .prompts
                .lock()
                .unwrap()
                .iter()
                .any(|(p, _)| p == "after the stop")
        },
        "the message sent after Stop to run",
    )
    .await;
    settle(&rig, 2).await;
    assert_eq!(rig.ledger.kills.load(Ordering::SeqCst), 1);
    assert_eq!(
        rig.ledger.spawns.load(Ordering::SeqCst),
        2,
        "a fresh runtime"
    );
    rig.core.sessions.shutdown().await;
}

/// The raw race under the command plane: a routed send accepted into a
/// runtime after its Stop began is re-dispatched, never dropped.
#[tokio::test]
async fn a_send_accepted_by_a_stopping_runtime_is_redispatched() {
    let rig = rig();
    rig.core
        .sessions
        .dispatch(CHAT, HarnessId::Mock, request("hold: working"), None)
        .await
        .unwrap();
    wait_for(
        || rig.ledger.prompts.lock().unwrap().len() == 1,
        "the agent to be running the turn",
    )
    .await;
    let (stop, send) = tokio::join!(
        rig.core.sessions.interrupt(CHAT),
        rig.core
            .sessions
            .dispatch(CHAT, HarnessId::Mock, request("racing the stop"), None)
    );
    stop.unwrap();
    send.unwrap();
    wait_for(
        || {
            rig.ledger
                .prompts
                .lock()
                .unwrap()
                .iter()
                .filter(|(p, _)| p == "racing the stop")
                .count()
                >= 1
                && status(&rig.core) == Some(SessionStatus::Idle)
        },
        "the racing message to run",
    )
    .await;
    assert_eq!(
        rig.ledger.kills.load(Ordering::SeqCst),
        1,
        "the Stop tore it down"
    );
    rig.core.sessions.shutdown().await;
}

/// A steer sent before Stop is cancelled with the turn: it must never run
/// later in the fresh runtime as if it were the user's next message.
#[tokio::test]
async fn a_steer_sent_before_stop_is_cancelled_with_it() {
    let rig = rig();
    rig.core
        .sessions
        .dispatch(CHAT, HarnessId::Mock, request("hold: working"), None)
        .await
        .unwrap();
    wait_for(
        || rig.ledger.prompts.lock().unwrap().len() == 1,
        "the agent to be running the turn",
    )
    .await;
    rig.core
        .doc_host
        .queue_command(
            CHAT,
            SessionCommandPayload::Run {
                request: request("hold: steer before stop"),
                message_id: "m-steer".into(),
            },
        )
        .unwrap();
    rig.core
        .doc_host
        .queue_command(CHAT, SessionCommandPayload::Interrupt { cancels: None })
        .unwrap();
    wait_for(
        || {
            rig.ledger.kills.load(Ordering::SeqCst) == 1
                && status(&rig.core) == Some(SessionStatus::Idle)
        },
        "the stop to settle",
    )
    .await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(
        rig.ledger.spawns.load(Ordering::SeqCst),
        1,
        "nothing re-ran"
    );
    assert_eq!(status(&rig.core), Some(SessionStatus::Idle));
    rig.core.sessions.shutdown().await;
}

/// A command forwarded over the peer relay: its doc row hasn't synced to
/// this host, so only what the entry itself carries is known here.
fn relayed(
    id: &str,
    payload: SessionCommandPayload,
    issued_at: i64,
    based_on: Option<String>,
) -> SessionCommandEntry {
    SessionCommandEntry {
        id: id.into(),
        payload,
        issued_by: "device-elsewhere".into(),
        issued_at,
        based_on: based_on.map(|turn_id| CommandBasedOn {
            turn_id: Some(turn_id),
            frontier: None,
        }),
        expires_at: None,
        status: SessionCommandStatus::Pending,
        resolution: None,
    }
}

/// A relayed Stop aimed at `aimed_at`, naming the sends it stops (`None`:
/// a Stop of an engine before 2026-11-17, which names none).
fn relayed_stop(
    id: &str,
    issued_at: i64,
    aimed_at: String,
    cancels: Option<&[&str]>,
) -> SessionCommandEntry {
    let cancels = cancels.map(|ids| ids.iter().map(|id| id.to_string()).collect());
    relayed(
        id,
        SessionCommandPayload::Interrupt { cancels },
        issued_at,
        Some(aimed_at),
    )
}

fn relayed_run(
    id: &str,
    prompt: &str,
    message_id: &str,
    issued_at: i64,
    aimed_at: String,
) -> SessionCommandEntry {
    relayed(
        id,
        SessionCommandPayload::Run {
            request: request(prompt),
            message_id: message_id.into(),
        },
        issued_at,
        Some(aimed_at),
    )
}

fn last_row(rig: &Rig) -> String {
    let doc = rig.core.doc_host.open(CHAT).unwrap();
    doc.doc().read_entries().unwrap().last().unwrap().id.clone()
}

fn has_row(rig: &Rig, id: &str) -> bool {
    let doc = rig.core.doc_host.open(CHAT).unwrap();
    doc.doc().read_entries().unwrap().iter().any(|m| m.id == id)
}

fn now() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

async fn ingest(rig: &Rig, entry: SessionCommandEntry) -> &'static str {
    rig.core
        .doc_host
        .ingest_relayed_command(CHAT, entry)
        .await
        .unwrap()
}

/// The relay road of [`a_steer_sent_before_stop_is_cancelled_with_it`]: a
/// steer and then a Stop, both from another device, arrive by relay before
/// either row syncs. The steer's row lands first; the Stop, which names it,
/// still stops the turn, steer and all.
#[tokio::test]
async fn a_relayed_stop_after_a_relayed_steer_still_stops_the_turn() {
    let rig = rig();
    rig.core
        .sessions
        .dispatch(CHAT, HarnessId::Mock, request("hold: working"), None)
        .await
        .unwrap();
    wait_for(
        || rig.ledger.prompts.lock().unwrap().len() == 1,
        "the agent to be running the turn",
    )
    .await;
    let aimed_at = last_row(&rig);
    let t = now();
    let steer = relayed_run(
        "c-steer",
        "hold: steer before stop",
        "m-relayed-steer",
        t,
        aimed_at.clone(),
    );
    let stop = relayed_stop("c-stop", t + 1, aimed_at, Some(&["m-relayed-steer"]));
    assert_eq!(ingest(&rig, steer).await, "executed");
    wait_for(
        || rig.ledger.prompts.lock().unwrap().len() == 2,
        "the steer to reach the agent",
    )
    .await;
    assert_eq!(
        ingest(&rig, stop).await,
        "executed",
        "the Stop covers the steer sent before it"
    );
    wait_for(
        || {
            rig.ledger.kills.load(Ordering::SeqCst) == 1
                && status(&rig.core) == Some(SessionStatus::Idle)
        },
        "the stop to settle",
    )
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        rig.ledger.spawns.load(Ordering::SeqCst),
        1,
        "nothing re-ran"
    );
    rig.core.sessions.shutdown().await;
}

/// A Stop from another device that arrives late, by relay, after the person
/// sent a new message here: it was aimed at the turn before, and leaves the
/// new turn alone.
#[tokio::test]
async fn a_late_relayed_stop_leaves_a_turn_sent_after_it_alone() {
    let rig = rig();
    rig.core
        .sessions
        .dispatch(CHAT, HarnessId::Mock, request("first"), None)
        .await
        .unwrap();
    settle(&rig, 1).await;
    let stop = relayed_stop("c-late-stop", now(), last_row(&rig), Some(&[]));
    tokio::time::sleep(Duration::from_millis(20)).await;
    rig.core
        .doc_host
        .queue_command(
            CHAT,
            SessionCommandPayload::Run {
                request: request("hold: the next turn"),
                message_id: "m-next".into(),
            },
        )
        .unwrap();
    wait_for(
        || {
            rig.ledger.prompts.lock().unwrap().len() == 2
                && status(&rig.core) == Some(SessionStatus::Working)
        },
        "the next turn to run",
    )
    .await;
    assert_eq!(
        ingest(&rig, stop).await,
        "superseded",
        "the late Stop was aimed at the finished turn"
    );
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        rig.ledger.kills.load(Ordering::SeqCst),
        0,
        "the new turn lives"
    );
    assert_eq!(status(&rig.core), Some(SessionStatus::Working));
    rig.core.sessions.shutdown().await;
}

/// A send from another device and then its Stop, the Stop arriving first by
/// relay: the send, named by the Stop, arrives after it and never runs. It
/// stays in the transcript, unrun.
#[tokio::test]
async fn a_send_arriving_after_the_stop_that_names_it_never_runs() {
    let rig = rig();
    rig.core
        .sessions
        .dispatch(CHAT, HarnessId::Mock, request("hold: first"), None)
        .await
        .unwrap();
    wait_for(
        || rig.ledger.prompts.lock().unwrap().len() == 1,
        "the agent to be running the turn",
    )
    .await;
    let aimed_at = last_row(&rig);
    let t = now();
    let send = relayed_run(
        "c-send",
        "hold: sent before Stop",
        "m-sent-before",
        t,
        aimed_at.clone(),
    );
    let stop = relayed_stop("c-stop", t + 1, aimed_at, Some(&["m-sent-before"]));
    assert_eq!(ingest(&rig, stop).await, "executed");
    wait_for(
        || status(&rig.core) == Some(SessionStatus::Idle),
        "the stop to settle",
    )
    .await;
    ingest(&rig, send).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        rig.ledger.prompts.lock().unwrap().len(),
        1,
        "the send the Stop named must not run after it"
    );
    assert_eq!(rig.ledger.spawns.load(Ordering::SeqCst), 1);
    assert!(
        has_row(&rig, "m-sent-before"),
        "the cancelled message stays in the transcript"
    );
    rig.core.sessions.shutdown().await;
}

/// A Stop built on another device (aimed at the finished turn, naming no
/// send) and, after it, a new Run whose clock reads `stop_clock_ahead_ms`
/// before the Stop's. The Run is delivered first and starts a turn; the Stop
/// arriving then is moot. Returns its disposition and how many runs it killed.
async fn a_stop_built_before_a_run(stop_clock_ahead_ms: i64) -> (&'static str, usize) {
    let rig = rig();
    rig.core
        .sessions
        .dispatch(CHAT, HarnessId::Mock, request("first"), None)
        .await
        .unwrap();
    settle(&rig, 1).await;
    let t = now();
    let aimed_at = last_row(&rig);
    let stop = relayed_stop(
        "c-stop",
        t + stop_clock_ahead_ms,
        aimed_at.clone(),
        Some(&[]),
    );
    let send = relayed_run("c-send", "hold: new turn", "m-new", t, aimed_at);
    assert_eq!(ingest(&rig, send).await, "executed");
    wait_for(
        || rig.ledger.prompts.lock().unwrap().len() == 2,
        "the new turn to run",
    )
    .await;
    let disposition = ingest(&rig, stop).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let kills = rig.ledger.kills.load(Ordering::SeqCst);
    rig.core.sessions.shutdown().await;
    (disposition, kills)
}

#[tokio::test]
async fn a_stop_built_before_a_run_in_the_same_millisecond_leaves_it_alone() {
    assert_eq!(a_stop_built_before_a_run(0).await, ("superseded", 0));
}

#[tokio::test]
async fn a_stop_from_a_clock_ahead_leaves_a_run_sent_after_it_alone() {
    assert_eq!(a_stop_built_before_a_run(1000).await, ("superseded", 0));
}

/// A Stop of an engine before 2026-11-17 names no sends and keeps main's
/// rule: any row after the turn it was aimed at makes it moot (here a steer
/// that landed first), and aimed at the chat's last row it stops the turn.
#[tokio::test]
async fn an_older_engines_stop_keeps_the_turn_rule() {
    let rig = rig();
    rig.core
        .sessions
        .dispatch(CHAT, HarnessId::Mock, request("hold: working"), None)
        .await
        .unwrap();
    wait_for(
        || rig.ledger.prompts.lock().unwrap().len() == 1,
        "the agent to be running the turn",
    )
    .await;
    let aimed_at = last_row(&rig);
    let t = now();
    let steer = relayed_run("c-steer", "hold: steer", "m-steer", t, aimed_at.clone());
    assert_eq!(ingest(&rig, steer).await, "executed");
    wait_for(|| has_row(&rig, "m-steer"), "the steer's row to land").await;
    assert_eq!(
        ingest(&rig, relayed_stop("c-old-stop", t + 1, aimed_at, None)).await,
        "superseded",
        "a row came after the turn it was aimed at"
    );
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(rig.ledger.kills.load(Ordering::SeqCst), 0);
    assert_eq!(
        ingest(
            &rig,
            relayed_stop("c-old-stop-2", now(), last_row(&rig), None)
        )
        .await,
        "executed"
    );
    wait_for(
        || {
            rig.ledger.kills.load(Ordering::SeqCst) == 1
                && status(&rig.core) == Some(SessionStatus::Idle)
        },
        "the stop to settle",
    )
    .await;
    rig.core.sessions.shutdown().await;
}

/// A turn held open by the agent, its prompt at the agent.
async fn start_hold(rig: &Rig) {
    rig.core
        .sessions
        .dispatch(CHAT, HarnessId::Mock, request("hold: original"), None)
        .await
        .unwrap();
    wait_for(
        || rig.ledger.prompts.lock().unwrap().len() == 1,
        "the turn to start",
    )
    .await;
}

/// A Stop is moot when the person wrote again after it, but what it names
/// is still cancelled: sent M1, Stop (naming M1), M2; delivered M2, the
/// Stop, then M1. M1 never runs. (GPT review 3, moot_stop_must_still_cancel_its_older_send.)
#[tokio::test]
async fn a_moot_stop_still_cancels_the_send_it_names() {
    let rig = rig();
    rig.core
        .sessions
        .dispatch(CHAT, HarnessId::Mock, request("first"), None)
        .await
        .unwrap();
    settle(&rig, 1).await;
    let aimed = last_row(&rig);
    let t = now();
    let old = relayed_run("old-send", "hold: cancelled", "m-old", t, aimed.clone());
    let stop = relayed_stop("stop", t + 1, aimed.clone(), Some(&["m-old"]));
    let new = relayed_run("new-send", "hold: after Stop", "m-new", t + 2, aimed);
    assert_eq!(ingest(&rig, new).await, "executed");
    wait_for(
        || rig.ledger.prompts.lock().unwrap().len() == 2,
        "the new turn",
    )
    .await;
    assert_eq!(ingest(&rig, stop).await, "superseded");
    ingest(&rig, old).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let got = rig.ledger.prompts.lock().unwrap().len();
    rig.core.sessions.shutdown().await;
    assert_eq!(got, 2, "a moot Stop must not let its named send run");
}

/// A Stop that gives way to a newer one (another device's, naming none)
/// still cancels what it names. (GPT review 3, coalesced_stop_loses_its_names.)
#[tokio::test]
async fn a_stop_giving_way_to_a_newer_stop_still_cancels_what_it_names() {
    let rig = rig();
    start_hold(&rig).await;
    let aimed = last_row(&rig);
    let second = relayed_stop("stop-second", now() + 1, aimed.clone(), Some(&[]));
    rig.core
        .doc_host
        .open(CHAT)
        .unwrap()
        .doc()
        .queue_command(&second)
        .unwrap();
    let first = relayed_stop("stop-first", now() - 1, aimed.clone(), Some(&["m-late"]));
    assert_eq!(ingest(&rig, first).await, "superseded");
    ingest(&rig, second).await;
    ingest(
        &rig,
        relayed_run("late-send", "hold: cancelled", "m-late", now(), aimed),
    )
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let got = rig.ledger.prompts.lock().unwrap().len();
    rig.core.sessions.shutdown().await;
    assert_eq!(got, 1, "the coalesced Stop's names were dropped");
}

/// A relayed Stop's names hold across a restart of the host before its row
/// syncs: the named send, arriving after, never runs. (GPT review 3,
/// relay_stop_lost_after_restart.)
#[tokio::test]
async fn a_relayed_stop_cancels_its_send_after_a_restart() {
    let rig = rig();
    start_hold(&rig).await;
    let aimed = last_row(&rig);
    assert_eq!(
        ingest(
            &rig,
            relayed_stop("stop", now(), aimed.clone(), Some(&["m-late"]))
        )
        .await,
        "executed"
    );
    rig.core.doc_host.flush_all();
    rig.core.sessions.shutdown().await;
    rig.core.doc_host.shutdown_workers().await;
    let Rig { core, ledger, _dir } = rig;
    drop(core);
    let registry = HarnessRegistry::new();
    registry.register(Arc::new(ProcessHarness {
        ledger: ledger.clone(),
        adopts_config: false,
        turn_boundary: false,
    }));
    let core = EngineCore::assemble(_dir.path(), Arc::new(registry), HarnessId::Mock, None)
        .expect("engine core reassembles");
    let rig = Rig { core, ledger, _dir };
    ingest(
        &rig,
        relayed_run("late-send", "hold: cancelled", "m-late", now(), aimed),
    )
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let got = rig.ledger.prompts.lock().unwrap().len();
    rig.core.sessions.shutdown().await;
    assert_eq!(
        got, 1,
        "the cancelled send ran after the Stop and a restart"
    );
}

/// A Stop naming a send still on its way (waiting on its image) doesn't wait
/// for it: it stops its turn at once, the send it names never runs, and the
/// person's next message, which it doesn't name, starts a turn it leaves
/// alone. (GPT review 3, new_turn_during_relay_stop_wait_is_killed: there
/// the new send landed while the Stop waited; since attempt 4 the Stop waits
/// for no send, so there is no such wait to land in.)
#[tokio::test]
async fn a_stop_does_not_wait_for_the_send_it_names() {
    let rig = rig();
    start_hold(&rig).await;
    let aimed = last_row(&rig);
    let mut held = relayed_run(
        "pending",
        "hold: before Stop",
        "m-pending",
        now(),
        aimed.clone(),
    );
    if let SessionCommandPayload::Run { request, .. } = &mut held.payload {
        request.attachments = vec!["pending://up-held/photo.png".into()];
    }
    rig.core
        .doc_host
        .open(CHAT)
        .unwrap()
        .doc()
        .queue_command(&held)
        .unwrap();
    let stop = relayed_stop("stop", now(), aimed.clone(), Some(&["m-pending"]));
    let host = rig.core.doc_host.clone();
    let result = tokio::time::timeout(
        Duration::from_secs(2),
        host.ingest_relayed_command(CHAT, stop),
    )
    .await
    .expect("the Stop waits for no send")
    .unwrap();
    assert_eq!(
        ingest(
            &rig,
            relayed_run("new-send", "hold: after Stop", "m-new", now(), aimed)
        )
        .await,
        "executed"
    );
    wait_for(
        || rig.ledger.prompts.lock().unwrap().len() == 2,
        "the new send",
    )
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let kills = rig.ledger.kills.load(Ordering::SeqCst);
    let prompts = rig.ledger.prompts.lock().unwrap().clone();
    rig.core.sessions.shutdown().await;
    // The one kill is the turn the Stop was aimed at.
    assert_eq!(
        (result, kills),
        ("executed", 1),
        "the Stop must not kill a newer send's turn"
    );
    assert_eq!(prompts.len(), 2, "the named send never ran");
    assert_eq!(prompts[1].0, "hold: after Stop", "the named send never ran");
}

/// A send-now that holds the queue while it ends the turn a Stop was aimed
/// at, then starts the person's next turn: the Stop doesn't wait behind it,
/// and the next turn survives the Stop. (GPT review 4,
/// stop_recheck_before_drain_lock_still_kills_new_turn.)
#[tokio::test]
async fn a_stop_waiting_behind_a_send_now_leaves_its_turn_alone() {
    let rig = rig();
    rig.core
        .sessions
        .dispatch(
            CHAT,
            HarnessId::Mock,
            request("hold: delay-stop original"),
            None,
        )
        .await
        .unwrap();
    wait_for(
        || rig.ledger.prompts.lock().unwrap().len() == 1,
        "the turn to start",
    )
    .await;
    let aimed = last_row(&rig);
    let id = rig
        .core
        .doc_host
        .queue_message(CHAT, "hold: newer queued turn", vec![])
        .unwrap();
    let host = rig.core.doc_host.clone();
    let send = tokio::spawn(async move { host.send_queued_now(CHAT, &id).await.unwrap() });
    wait_for(
        || rig.ledger.stop_reached.load(Ordering::SeqCst) == 1,
        "the send-now to end the turn, holding the queue",
    )
    .await;
    let host = rig.core.doc_host.clone();
    let stop = relayed_stop("late-stop", now(), aimed, Some(&[]));
    let stop_task =
        tokio::spawn(async move { host.ingest_relayed_command(CHAT, stop).await.unwrap() });
    // Past the Stop's bounded try for the queue's lock: it has judged, and
    // is ending the original runtime alongside the send-now.
    tokio::time::sleep(Duration::from_millis(500)).await;
    rig.ledger.stop_release.notify_one();
    assert!(send.await.unwrap());
    wait_for(
        || rig.ledger.prompts.lock().unwrap().len() == 2,
        "the queued turn",
    )
    .await;
    let result = stop_task.await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    let kills = rig.ledger.kills.load(Ordering::SeqCst);
    let prompts = rig.ledger.prompts.lock().unwrap().clone();
    rig.core.sessions.shutdown().await;
    // What matters: the turn started after the Stop is never killed (the one
    // kill is the send-now retiring the original runtime). The Stop itself
    // is "executed", not "superseded" as before attempt 4: it no longer
    // waits for the send-now, so it judges before the newer turn's row
    // exists, finds the original runtime (handed nothing its sender didn't
    // see) and stops that; the newer turn starts after, in a fresh runtime.
    assert_eq!(
        (result, kills),
        ("executed", 1),
        "the Stop killed the turn started after it"
    );
    assert_eq!(prompts[1].0, "hold: newer queued turn");
}

/// Lets the dispatches stalled in the binary lookup go on.
fn release_binary(rig: &Rig) {
    *rig.ledger.binary_release.0.lock().unwrap() = true;
    rig.ledger.binary_release.1.notify_all();
}

/// A send a Stop names never starts after the Stop, even when its dispatch
/// had written its row and stalled before its runtime was registered (GPT
/// review 5, named_relayed_run_stalled_in_dispatch_runs_after_stop).
#[tokio::test]
async fn a_named_send_stalled_before_its_runtime_never_starts() {
    let rig = rig();
    start_hold(&rig).await;
    let aimed = last_row(&rig);
    rig.core.sessions.interrupt(CHAT).await.unwrap();
    rig.ledger.binary_gate.store(true, Ordering::SeqCst);
    let run = relayed_run(
        "stalled-send",
        "hold: named stalled send",
        "m-stalled",
        now(),
        aimed.clone(),
    );
    let host = rig.core.doc_host.clone();
    let sent = tokio::spawn(async move { host.ingest_relayed_command(CHAT, run).await.unwrap() });
    wait_for(
        || rig.ledger.binary_reached.load(Ordering::SeqCst) > 0,
        "the dispatch to stall in its binary lookup",
    )
    .await;
    assert!(has_row(&rig, "m-stalled"), "the send wrote its row first");
    assert!(!rig.core.sessions.has_run(CHAT));
    let stop = relayed_stop("stop", now(), aimed, Some(&["m-stalled"]));
    let result = ingest(&rig, stop).await;
    let before = rig.ledger.prompts.lock().unwrap().len();
    release_binary(&rig);
    sent.await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    let after = rig.ledger.prompts.lock().unwrap().len();
    rig.core.sessions.shutdown().await;
    assert_eq!(result, "executed");
    assert_eq!(after, before, "the send the Stop named ran after it");
}

/// A queue send stuck in its dispatch doesn't hold up a Stop that names it,
/// and never starts after it (GPT review 5,
/// stalled_queue_dispatch_blocks_stop_past_two_seconds).
#[tokio::test]
async fn a_stuck_queue_send_does_not_hold_up_the_stop_naming_it() {
    let rig = rig();
    start_hold(&rig).await;
    let aimed = last_row(&rig);
    rig.core.sessions.interrupt(CHAT).await.unwrap();
    rig.ledger.binary_gate.store(true, Ordering::SeqCst);
    let id = rig
        .core
        .doc_host
        .queue_message(CHAT, "hold: stalled queued send", vec![])
        .unwrap();
    let queued = id.clone();
    let host = rig.core.doc_host.clone();
    let sent = tokio::spawn(async move { host.send_queued_now(CHAT, &id).await.unwrap() });
    wait_for(
        || rig.ledger.binary_reached.load(Ordering::SeqCst) > 0,
        "the queue send to stall in its binary lookup",
    )
    .await;
    let stop = relayed_stop("stop", now(), aimed, Some(&[queued.as_str()]));
    let host = rig.core.doc_host.clone();
    let stopped =
        tokio::spawn(async move { host.ingest_relayed_command(CHAT, stop).await.unwrap() });
    let result = tokio::time::timeout(Duration::from_secs(5), stopped).await;
    let before = rig.ledger.prompts.lock().unwrap().len();
    release_binary(&rig);
    sent.await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    let after = rig.ledger.prompts.lock().unwrap().len();
    rig.core.sessions.shutdown().await;
    assert!(
        result.is_ok(),
        "the Stop waited for the stuck queue send's dispatch"
    );
    assert_eq!(after, before, "the queue send the Stop named ran after it");
}

/// A late Stop aimed at a row a rewind removed is moot: it leaves the turn
/// sent after the rewind alone. (GPT review 3, old_stop_after_rewind_kills_new_turn.)
#[tokio::test]
async fn a_stop_aimed_at_a_rewound_turn_leaves_the_new_turn_alone() {
    let rig = rig();
    rig.core
        .sessions
        .dispatch(CHAT, HarnessId::Mock, request("first"), None)
        .await
        .unwrap();
    settle(&rig, 1).await;
    let aimed = last_row(&rig);
    let stop = relayed_stop("late-stop", now(), aimed.clone(), Some(&[]));
    rig.core.sessions.rewind(CHAT, &aimed).await.unwrap();
    ingest(
        &rig,
        relayed_run(
            "new-send",
            "hold: after rewind",
            "m-new",
            now(),
            last_row(&rig),
        ),
    )
    .await;
    wait_for(
        || rig.ledger.prompts.lock().unwrap().len() == 2,
        "the new turn",
    )
    .await;
    let before = rig.ledger.kills.load(Ordering::SeqCst);
    let result = ingest(&rig, stop).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    let kills = rig.ledger.kills.load(Ordering::SeqCst);
    rig.core.sessions.shutdown().await;
    assert_eq!(
        (result, kills),
        ("superseded", before),
        "the late Stop killed the turn after its anchor was removed"
    );
}

/// The idle reaper spares a parked runtime holding background work, so the
/// user's Stop must reach it and end that work.
#[tokio::test]
async fn stop_ends_a_parked_runtime_holding_background_work() {
    let rig = rig();
    rig.core
        .sessions
        .dispatch(
            CHAT,
            HarnessId::Mock,
            request("launch a background scout"),
            None,
        )
        .await
        .unwrap();
    settle(&rig, 1).await;
    // A runtime parked with nothing running is left alone.
    rig.core
        .doc_host
        .queue_command(CHAT, SessionCommandPayload::Interrupt { cancels: None })
        .unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(rig.ledger.kills.load(Ordering::SeqCst), 0);

    let turn = rig.ledger.turn.lock().unwrap().clone().unwrap();
    turn.set_background(1);
    rig.core
        .doc_host
        .queue_command(CHAT, SessionCommandPayload::Interrupt { cancels: None })
        .unwrap();
    wait_for(
        || rig.ledger.kills.load(Ordering::SeqCst) == 1,
        "the Stop to end the background work",
    )
    .await;
    assert!(!rig.core.sessions.live_run_steerable(CHAT));
    rig.core.sessions.shutdown().await;
}

#[tokio::test(start_paused = true)]
async fn the_idle_reaper_spares_a_runtime_holding_background_work() {
    let rig = rig();
    rig.core
        .sessions
        .dispatch(
            CHAT,
            HarnessId::Mock,
            request("launch a background scout"),
            None,
        )
        .await
        .unwrap();
    settle(&rig, 1).await;
    let turn = rig.ledger.turn.lock().unwrap().clone().unwrap();
    turn.set_background(1);

    // Far past the 30-minute idle window: the scout is still running. (The
    // odd 20 minutes put its end mid-way through a deferred window.)
    tokio::time::sleep(Duration::from_secs(3 * 60 * 60 + 20 * 60)).await;
    assert!(
        rig.core.sessions.live_run_steerable(CHAT),
        "the reaper killed a runtime holding live background work"
    );
    assert_eq!(rig.ledger.kills.load(Ordering::SeqCst), 0);

    // Once the background work is done, an idle runtime is released again —
    // but not before a full idle window has passed since: the completion's
    // wake turn starts a moment after the agent reports the work drained.
    turn.set_background(0);
    tokio::time::sleep(Duration::from_secs(29 * 60)).await;
    assert!(
        rig.core.sessions.live_run_steerable(CHAT),
        "the reaper killed the runtime as its background work ended"
    );
    tokio::time::sleep(Duration::from_secs(2 * 60 * 60)).await;
    wait_for(
        || rig.ledger.kills.load(Ordering::SeqCst) == 1,
        "the idle runtime to be reaped",
    )
    .await;
    rig.core.sessions.shutdown().await;
}
