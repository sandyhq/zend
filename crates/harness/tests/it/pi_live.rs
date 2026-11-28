//! Real native Pi, isolated settings, and a local provider (no network/API spend).
#![cfg(unix)]
use futures::StreamExt;
use std::{os::unix::fs::PermissionsExt, time::Duration};
use tokio::sync::{mpsc, oneshot};
use zend_harness::{CancellationToken, Harness, PiHarness, RunControls, SteerMessage};
use zend_proto::{AgentEvent, DoneStatus, RunRequest, SandboxLevel, UserInputAnswer};

fn isolated_pi() -> (tempfile::TempDir, PiHarness) {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path();
    let agent = cwd.join("agent");
    std::fs::create_dir_all(&agent).unwrap();
    std::fs::write(
        agent.join("settings.json"),
        r#"{"retry":{"enabled":false}}"#,
    )
    .unwrap();
    let extensions = agent.join("extensions");
    std::fs::create_dir_all(&extensions).unwrap();
    std::fs::write(
        extensions.join("probe.ts"),
        include_str!("../fixtures/pi-rpc-probe.ts"),
    )
    .unwrap();
    let exe = PiHarness::new()
        .resolve_executable()
        .expect("Pi CLI installed");
    let quote =
        |p: &std::path::Path| format!("'{}'", p.display().to_string().replace('\'', "'\\''"));
    let wrapper = cwd.join("pi-probe");
    std::fs::write(
        &wrapper,
        format!(
            "#!/bin/sh\nexport PI_CODING_AGENT_DIR={}\nexec {} \"$@\"\n",
            quote(&agent),
            quote(&exe)
        ),
    )
    .unwrap();
    std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o700)).unwrap();
    let harness = PiHarness::new()
        .with_executable(wrapper)
        .with_agent_dir(&agent)
        .with_session_store(cwd.join("index"));
    (dir, harness)
}

#[tokio::test]
#[ignore = "requires Pi >= 0.85.1 installed; uses only a local mock provider"]
async fn real_pi_mock_lifecycle() {
    let (dir, harness) = isolated_pi();
    let cwd = dir.path();
    let mut session = None;
    for (prompt, expected) in [
        ("/probe-noop", DoneStatus::Completed),
        ("hello", DoneStatus::Completed),
        ("/probe-new", DoneStatus::Completed),
        ("resume", DoneStatus::Completed),
        ("/probe-noop", DoneStatus::Completed),
        ("/probe-input", DoneStatus::Completed),
        ("error", DoneStatus::Errored),
        ("slow", DoneStatus::Interrupted),
        ("steering slow", DoneStatus::Completed),
        ("/probe-new", DoneStatus::Completed),
        ("/probe-metadata", DoneStatus::Completed),
    ] {
        let (steer, steering) = mpsc::channel(8);
        let interrupt = CancellationToken::new();
        let controls = RunControls {
            realtime: None,
            execution_lease: None,
            steering,
            interrupt: interrupt.clone(),
            request_input: Box::new(|questions| {
                let (tx, rx) = oneshot::channel();
                tx.send(vec![UserInputAnswer {
                    question_id: questions[0].id.clone(),
                    labels: vec!["local answer".into()],
                }])
                .unwrap();
                rx
            }),
            turn: Default::default(),
            sandbox: None,
        };
        let request = RunRequest {
            prompt: prompt.into(),
            harness: None,
            model: Some("zend-probe/mock".into()),
            reasoning: None,
            model_options: Default::default(),
            cwd: cwd.display().to_string(),
            sandbox: SandboxLevel::WorkspaceWrite,
            auto_approve: true,
            access: Default::default(),
            plan: false,
            resume: session.clone(),
            attachments: vec![],
            worktree: None,
            mcp: None,
            env: Default::default(),
            extra_mcp: Vec::new(),
            instructions: Vec::new(),
            fork_from: None,
        };
        let previous_session = session.clone();
        let mut stream = harness.run(request, controls).await.unwrap();
        let mut sender = Some(steer);
        let mut done = 0;
        let mut confirmed = 0;
        let mut text = String::new();
        tokio::time::timeout(Duration::from_secs(20), async {
            while let Some(event) = stream.next().await {
                match event.unwrap() {
                    AgentEvent::SessionStarted { session_id, .. } => {
                        if let Some(old) = &session {
                            if prompt != "/probe-new" {
                                assert_eq!(old, &session_id);
                            }
                        }
                        session = Some(session_id);
                        if prompt == "steering slow" {
                            sender
                                .take()
                                .unwrap()
                                .send(SteerMessage {
                                    prompt: "redirect".into(),
                                    message_id: None,
                                    attachments: Vec::new(),
                                    config: None,
                                })
                                .await
                                .unwrap();
                        } else {
                            sender.take();
                        }
                        if prompt == "slow" {
                            let token = interrupt.clone();
                            tokio::spawn(async move {
                                tokio::time::sleep(Duration::from_millis(150)).await;
                                token.cancel();
                            });
                        }
                    }
                    AgentEvent::TextDelta { text: delta } => text.push_str(&delta),
                    AgentEvent::Steered { .. } => confirmed += 1,
                    AgentEvent::Done {
                        status,
                        error,
                        session_id,
                        ..
                    } => {
                        assert_eq!(
                            session_id, session,
                            "Done must publish the current native session identity"
                        );
                        assert_eq!(status, expected, "{prompt}: {error:?}");
                        done += 1;
                    }
                    _ => {}
                }
            }
        })
        .await
        .expect("native Pi run must settle");
        assert_eq!(done, 1, "{prompt}: {text}");
        if prompt == "/probe-new" {
            assert_ne!(session, previous_session);
        }
        if prompt == "/probe-input" {
            assert!(text.contains("answer:local answer"), "{text}");
        }
        if prompt == "steering slow" {
            assert_eq!(confirmed, 1);
            assert!(text.contains("MOCK:redirect"), "{text}");
        }
        if matches!(prompt, "hello" | "resume") {
            assert_eq!(text, format!("MOCK:{prompt}"));
        }
    }
    // Unsaved extension entries are not equivalent to an empty conversation, so
    // the UUID is not recreated. Pi has no public RPC to restore them; the chat
    // continues in a new session and says so instead of failing every message.
    let (_, steering) = mpsc::channel(1);
    let controls = RunControls {
        realtime: None,
        execution_lease: None,
        steering,
        interrupt: CancellationToken::new(),
        request_input: Box::new(|_| oneshot::channel().1),
        turn: Default::default(),
        sandbox: None,
    };
    let request = RunRequest {
        prompt: "after loss".into(),
        harness: None,
        model: Some("zend-probe/mock".into()),
        reasoning: None,
        model_options: Default::default(),
        cwd: cwd.display().to_string(),
        sandbox: SandboxLevel::WorkspaceWrite,
        auto_approve: true,
        access: Default::default(),
        plan: false,
        resume: session.clone(),
        attachments: vec![],
        worktree: None,
        mcp: None,
        env: Default::default(),
        extra_mcp: Vec::new(),
        instructions: Vec::new(),
        fork_from: None,
    };
    let events: Vec<_> = tokio::time::timeout(
        Duration::from_secs(20),
        harness
            .run(request, controls)
            .await
            .unwrap()
            .map(Result::unwrap)
            .collect(),
    )
    .await
    .expect("native Pi run must settle");
    assert!(
        events
            .iter()
            .any(|e| matches!(e, AgentEvent::Error { message }
            if message.contains("without the previous context"))),
        "{events:?}"
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, AgentEvent::Done { status, session_id, .. }
            if *status == DoneStatus::Completed && session_id.is_some() && *session_id != session)),
        "{events:?}"
    );
}

/// The plugin instructions of the ZEBRA check (session.instructions).
const ZEBRA: &str = "Antworte immer mit dem Wort ZEBRA am Ende";

fn zebra_request(cwd: &std::path::Path, prompt: &str, text: Option<&str>) -> RunRequest {
    RunRequest {
        prompt: prompt.into(),
        harness: None,
        model: Some("zend-probe/mock".into()),
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
        instructions: text
            .map(|text| {
                vec![zend_proto::SessionInstruction {
                    plugin: "zebra".into(),
                    name: "Zebra".into(),
                    text: text.into(),
                }]
            })
            .unwrap_or_default(),
        fork_from: None,
    }
}

fn open_controls() -> (mpsc::Sender<SteerMessage>, RunControls) {
    let (tx, steering) = mpsc::channel(8);
    let controls = RunControls {
        realtime: None,
        execution_lease: None,
        steering,
        interrupt: CancellationToken::new(),
        request_input: Box::new(|_| oneshot::channel().1),
        turn: Default::default(),
        sandbox: None,
    };
    (tx, controls)
}

/// The stream's events up to its next `Done`: that status.
async fn next_done(
    stream: &mut (impl futures::Stream<Item = Result<AgentEvent, zend_harness::HarnessError>> + Unpin),
) -> DoneStatus {
    tokio::time::timeout(Duration::from_secs(30), async {
        while let Some(event) = stream.next().await {
            match event.unwrap() {
                AgentEvent::Done { status, error, .. } => {
                    if let Some(error) = error {
                        eprintln!("turn settled {status:?}: {error}");
                    }
                    return status;
                }
                AgentEvent::Error { message } => eprintln!("error event: {message}"),
                _ => {}
            }
        }
        panic!("the run ended without a Done");
    })
    .await
    .expect("a Done in time")
}

/// The system context of the model call for `prompt` (the last one), as
/// text: Pi's system prompt and any system patches it sent.
fn system_context(cwd: &std::path::Path, prompt: &str) -> String {
    let calls: Vec<serde_json::Value> = std::fs::read_to_string(cwd.join("probe-contexts.jsonl"))
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();
    let call = calls
        .iter()
        .rev()
        .find(|c| c["text"] == prompt)
        .unwrap_or_else(|| panic!("no model call for {prompt:?}"));
    let mut out = call["systemPrompt"].as_str().unwrap_or_default().to_owned();
    for message in call["messages"].as_array().into_iter().flatten() {
        if message["role"] != "user" && message["role"] != "assistant" && message["role"] != "toolResult" {
            out.push('\n');
            out.push_str(&message.to_string());
        }
    }
    out
}

#[tokio::test]
#[ignore = "requires Pi >= 0.85.1 installed; uses only a local mock provider"]
async fn real_pi_plugin_instructions_reach_the_system_prompt() {
    let (dir, harness) = isolated_pi();
    let lead = dir.path().join("lead");
    let child = dir.path().join("child");
    let plain = dir.path().join("plain");
    for d in [&lead, &child, &plain] {
        std::fs::create_dir_all(d).unwrap();
    }
    let section = format!("<plugin-zebra>\n{ZEBRA}\n</plugin-zebra>");
    // Compaction keeps no recent turn whole, so this small session compacts.
    std::fs::write(
        dir.path().join("agent").join("settings.json"),
        r#"{"retry":{"enabled":false},"compaction":{"keepRecentTokens":1}}"#,
    )
    .unwrap();
    let has = |ctx: &str, text: &str| ctx.contains(text);

    // A new session: the plugin's text, whole, in its own section.
    let request = zebra_request(&lead, "hello", Some(ZEBRA));
    let (tx, controls) = open_controls();
    let mut stream = harness.run(request.clone(), controls).await.unwrap();
    assert_eq!(next_done(&mut stream).await, DoneStatus::Completed);
    let first = system_context(&lead, "hello");
    assert!(has(&first, ZEBRA), "the new session's system prompt holds the instructions: {first}");
    assert!(
        has(&first, "<plugin-zebra>"),
        "in a section of its own (plugin-zebra): {first}"
    );
    eprintln!("new session, plugin section as the model got it: {:?}", first.find("<plugin-zebra>").map(|at| &first[at..(at + section.len() + 8).min(first.len())]));

    // Compacted: the next model call still has it.
    tx.send(SteerMessage::text("second turn")).await.unwrap();
    assert_eq!(next_done(&mut stream).await, DoneStatus::Completed);
    tx.send(SteerMessage::text("/compact")).await.unwrap();
    assert_eq!(next_done(&mut stream).await, DoneStatus::Completed);
    tx.send(SteerMessage::text("after compaction")).await.unwrap();
    assert_eq!(next_done(&mut stream).await, DoneStatus::Completed);
    let compacted = system_context(&lead, "after compaction");
    assert!(has(&compacted, ZEBRA), "still there after a compaction: {compacted}");
    let calls = std::fs::read_to_string(lead.join("probe-contexts.jsonl")).unwrap();
    assert!(calls.lines().count() >= 3, "the compaction asked the model too");
    let last: serde_json::Value = serde_json::from_str(calls.lines().last().unwrap()).unwrap();
    let said = |m: &serde_json::Value| match &m["content"] {
        serde_json::Value::String(s) => s.clone(),
        content => content.as_array().into_iter().flatten().filter_map(|c| c["text"].as_str()).collect(),
    };
    let users: Vec<String> = last["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m["role"] == "user")
        .map(said)
        .collect();
    eprintln!("after compaction the model got these user messages: {users:?}");
    assert!(!users.iter().any(|u| u == "hello"), "the first turn was compacted away: {users:?}");

    // Changed while the process lives: the next turn has the new text only.
    let changed = "Antworte immer mit dem Wort OKAPI am Ende";
    tx.send(SteerMessage {
        prompt: "changed".into(),
        message_id: None,
        attachments: Vec::new(),
        config: Some(Box::new(zebra_request(&lead, "changed", Some(changed)))),
    })
    .await
    .unwrap();
    assert_eq!(next_done(&mut stream).await, DoneStatus::Completed);
    let now = system_context(&lead, "changed");
    assert!(has(&now, changed), "a change reaches the live process: {now}");
    drop(tx);
    while stream.next().await.is_some() {}

    // A child session (its own run, a session of its own): the same text.
    let (tx, controls) = open_controls();
    let mut stream = harness
        .run(zebra_request(&child, "child hello", Some(ZEBRA)), controls)
        .await
        .unwrap();
    assert_eq!(next_done(&mut stream).await, DoneStatus::Completed);
    drop(tx);
    while stream.next().await.is_some() {}
    let childs = system_context(&child, "child hello");
    assert!(has(&childs, ZEBRA), "the child session's system prompt holds them: {childs}");
    assert!(has(&childs, "<plugin-zebra>"));

    // And a run told nothing has no such section.
    let (tx, controls) = open_controls();
    let mut stream = harness.run(zebra_request(&plain, "plain hello", None), controls).await.unwrap();
    assert_eq!(next_done(&mut stream).await, DoneStatus::Completed);
    drop(tx);
    while stream.next().await.is_some() {}
    let none = system_context(&plain, "plain hello");
    assert!(!has(&none, "ZEBRA") && !has(&none, "<plugin-"), "{none}");
}

async fn wait_probe_lines(path: &std::path::Path, count: usize) -> Vec<serde_json::Value> {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let lines: Vec<_> = std::fs::read_to_string(path)
                .unwrap_or_default()
                .lines()
                .filter_map(|line| serde_json::from_str(line).ok())
                .collect();
            if lines.len() >= count {
                return lines;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("Pi probe must reach the expected barrier")
}

#[tokio::test]
#[ignore = "requires Pi >= 0.85.1 installed; uses only a local mock provider"]
async fn real_pi_steering_bursts_share_the_next_model_call() {
    let (dir, harness) = isolated_pi();
    let cwd = dir.path();
    let (tx, steering) = mpsc::channel(8);
    let controls = RunControls {
        realtime: None,
        execution_lease: None,
        steering,
        interrupt: CancellationToken::new(),
        request_input: Box::new(|_| oneshot::channel().1),
        turn: Default::default(),
        sandbox: None,
    };
    let request = RunRequest {
        prompt: "burst hold".into(),
        harness: None,
        model: Some("zend-probe/mock".into()),
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
        fork_from: None,
    };
    let mut stream = harness.run(request, controls).await.unwrap();
    let calls = cwd.join("probe-model-calls.jsonl");
    let inputs = cwd.join("probe-inputs.jsonl");
    assert_eq!(
        wait_probe_lines(&calls, 1).await[0],
        serde_json::json!(["burst hold"])
    );
    let burst: Vec<_> = (0..40).map(|i| format!("burst-{}", i / 2)).collect();
    for (i, prompt) in burst.iter().enumerate() {
        tx.send(SteerMessage {
            prompt: prompt.clone(),
            message_id: Some(format!("burst-user-{i}")),
            attachments: Vec::new(),
            config: None,
        })
        .await
        .unwrap();
    }
    assert_eq!(
        wait_probe_lines(&inputs, burst.len()).await,
        burst
            .iter()
            .map(|s| serde_json::json!(s))
            .collect::<Vec<_>>()
    );
    assert_eq!(wait_probe_lines(&calls, 1).await.len(), 1);
    std::fs::write(cwd.join("probe-release-initial"), "").unwrap();
    let snapshot = wait_probe_lines(&calls, 2).await;
    assert_eq!(snapshot.len(), 2);
    assert_eq!(snapshot[1], serde_json::json!(burst));
    // Anything arriving after the next call began belongs to the following
    // step, rather than being claimed as part of the already-running call.
    let late = vec!["late-1", "late-2", "late-3"];
    for prompt in &late {
        tx.send(SteerMessage::text(*prompt)).await.unwrap();
    }
    drop(tx);
    wait_probe_lines(&inputs, burst.len() + late.len()).await;
    assert_eq!(wait_probe_lines(&calls, 2).await.len(), 2);
    std::fs::write(cwd.join("probe-release-burst"), "").unwrap();
    let snapshot = wait_probe_lines(&calls, 3).await;
    assert_eq!(snapshot.len(), 3);
    assert_eq!(snapshot[2], serde_json::json!(late));
    std::fs::write(cwd.join("probe-release-late"), "").unwrap();
    let mut confirmed = 0;
    let mut done = vec![];
    let mut text = String::new();
    tokio::time::timeout(Duration::from_secs(15), async {
        while let Some(event) = stream.next().await {
            match event.unwrap() {
                AgentEvent::Steered { .. } => confirmed += 1,
                AgentEvent::Done { status, .. } => done.push(status),
                AgentEvent::TextDelta { text: delta } => text.push_str(&delta),
                _ => {}
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(confirmed, burst.len() + late.len());
    assert_eq!(done, vec![DoneStatus::Completed]);
    assert_eq!(
        text,
        format!(
            "MOCK:burst holdMOCK:{}MOCK:{}",
            burst.join("|"),
            late.join("|")
        )
    );
    assert_eq!(wait_probe_lines(&calls, 3).await.len(), 3);
}
