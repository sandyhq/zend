//! A fork of a Pi chat starts on a copy of the source's own Pi session, cut
//! where the fork copies through; every doubt takes today's history prompt
//! (plans/pi-native-fork, K1-K5). Against the Pi-shaped `fake-pi-rpc.py`,
//! which answers "reply:<what it was sent>": a reply shows Pi's whole input.
#![cfg(unix)]
use std::{path::PathBuf, sync::Arc, time::Duration};
use zend_doc::{MessagePart, MessageRole};
use zend_engine::{EngineCore, HarnessRegistry};
use zend_harness::PiHarness;
use zend_proto::{HarnessId, RunRequest, SandboxLevel};
use zend_rpc::methods;

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
        fork_from: None,
    }
}

struct World {
    core: EngineCore,
    cwd: String,
    _dir: tempfile::TempDir,
}

impl World {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../harness/tests/fixtures/fake-pi-rpc.py");
        let registry = HarnessRegistry::new();
        registry.register(Arc::new(
            PiHarness::new()
                .with_executable(fixture)
                .with_session_store(dir.path().join("index"))
                .with_agent_dir(dir.path().join("agent"))
                .with_graces(Duration::from_millis(50), Duration::from_millis(100)),
        ));
        let core = EngineCore::assemble(dir.path(), Arc::new(registry), HarnessId::Pi, None).unwrap();
        let cwd = dir.path().join("work");
        std::fs::create_dir_all(&cwd).unwrap();
        World { core, cwd: cwd.display().to_string(), _dir: dir }
    }

    fn chat(&self, chat: &str) {
        self.core
            .workspace
            .create_chat(chat, None, Some(&self.core.device_id), None, None)
            .unwrap();
    }

    /// Send `prompt` as user entry `id`; Pi's reply to it (its whole input).
    async fn turn(&self, chat: &str, id: &str, prompt: &str) -> String {
        self.core
            .sessions
            .dispatch(chat, HarnessId::Pi, request(prompt, &self.cwd), Some(id.into()))
            .await
            .unwrap();
        let handle = self.core.doc_host.open(chat).unwrap();
        let reply = tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                let entries = handle.doc().read_entries().unwrap();
                let after = entries.iter().position(|e| e.id == id).map_or(entries.len(), |at| at + 1);
                let found = entries[after..].iter().find_map(|e| {
                    e.parts.iter().find_map(|p| match p {
                        MessagePart::Text { text, .. } if text.starts_with("reply:") => Some(text.clone()),
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
        .unwrap_or_else(|_| panic!("{chat}: no reply to {id}"));
        tokio::time::timeout(Duration::from_secs(10), async {
            while self.core.sessions.turn_in_flight(chat) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        reply
    }

    async fn fork(&self, chat: &str, source: &str, at: &str, standalone: bool) -> serde_json::Value {
        let client = zend_rpc::memory_client(self.core.rpc_service());
        client
            .call_as(
                methods::FORK_SIDE_CHAT,
                serde_json::json!({ "chatId": chat, "sourceChatId": source, "messageId": at, "standalone": standalone }),
            )
            .await
            .unwrap()
    }

    /// The Pi session file `chat`'s last turn ended in.
    fn session_file(&self, chat: &str) -> PathBuf {
        let doc = self.core.doc_host.open(chat).unwrap();
        let marks = doc.doc().native_turns();
        PathBuf::from(&marks.last().unwrap_or_else(|| panic!("{chat}: no native turn")).session_file)
    }

    fn session_id(&self, chat: &str) -> Option<String> {
        self.core.workspace.chat(chat).unwrap().unwrap().harness_session_id
    }

    /// The entry `chat`'s turn `user` ended on (its last reply).
    fn end_of(&self, chat: &str, user: &str) -> String {
        let entries = self.core.doc_host.open(chat).unwrap().doc().read_entries().unwrap();
        let at = entries.iter().position(|e| e.id == user).unwrap();
        entries[at + 1..]
            .iter()
            .take_while(|e| e.role != MessageRole::User)
            .last()
            .unwrap()
            .id
            .clone()
    }

    /// "src": three turns, PINEAPPLE, BANANA, CHERRY.
    async fn source(&self) {
        self.chat("src");
        for (id, word) in [("u1", "PINEAPPLE"), ("u2", "BANANA"), ("u3", "CHERRY")] {
            let reply = self.turn("src", id, &format!("say {word}")).await;
            assert_eq!(reply, format!("reply:say {word}"), "the source runs on: {reply}");
        }
    }
}

fn texts(file: &std::path::Path) -> (serde_json::Value, String) {
    let text = std::fs::read_to_string(file).unwrap();
    let header = serde_json::from_str(text.lines().next().unwrap()).unwrap();
    (header, text)
}

#[tokio::test]
async fn a_fork_of_a_pi_chat_starts_on_its_cut_session_and_the_source_runs_on() {
    let w = World::new();
    w.source().await;
    let src_file = w.session_file("src");
    let src_session = w.session_id("src");
    assert_eq!(w.core.doc_host.open("src").unwrap().doc().native_turns().len(), 3);
    let before = std::fs::read(&src_file).unwrap();

    // K1, K3: at the reply of turn 2, a side chat; Pi gets the bare prompt.
    w.fork("k1", "src", &w.end_of("src", "u2"), false).await;
    let reply = w.turn("k1", "f1", "what now").await;
    assert_eq!(reply, "reply:what now", "no history block");
    let file = w.session_file("k1");
    assert_ne!(file, src_file);
    let (header, text) = texts(&file);
    assert_eq!(header["parentSession"], src_file.display().to_string());
    assert!(text.contains("say PINEAPPLE") && text.contains("say BANANA"), "{text}");
    assert!(!text.contains("CHERRY"), "turn 3 is past the fork: {text}");
    // The fork's next turn continues its own session.
    let session = w.session_id("k1");
    assert_eq!(w.turn("k1", "f2", "and then").await, "reply:and then");
    assert_eq!(w.session_id("k1"), session);
    assert_eq!(w.session_file("k1"), file);

    // K2: at the prompt of turn 2, a standalone session: turn 1 only, and
    // the prompt comes back as the draft.
    let reply = w.fork("k2", "src", "u2", true).await;
    assert_eq!(reply["draft"], "say BANANA");
    assert_eq!(w.turn("k2", "g1", "say BANANA again").await, "reply:say BANANA again");
    let (header, text) = texts(&w.session_file("k2"));
    assert_eq!(header["parentSession"], src_file.display().to_string());
    assert!(text.contains("say PINEAPPLE") && !text.contains("\"say BANANA\""), "{text}");

    // K4: the source's file untouched by both forks; it runs on in it.
    assert_eq!(std::fs::read(&src_file).unwrap(), before);
    assert_eq!(w.turn("src", "u4", "say DATE").await, "reply:say DATE");
    assert_eq!(w.session_id("src"), src_session);
    assert_eq!(w.session_file("src"), src_file);
    assert!(std::fs::read_to_string(&src_file).unwrap().contains("say DATE"));
    w.core.shutdown().await;
}

/// K5: each doubt gives the fork the copied history as its prompt, and a
/// working reply.
#[tokio::test]
async fn every_unsafe_fork_gets_the_history_prompt() {
    let w = World::new();
    w.source().await;
    let src_file = w.session_file("src");
    let at = w.end_of("src", "u2");
    let history = |reply: &str, case: &str| {
        assert!(reply.contains("PINEAPPLE") && reply.contains("BANANA") && !reply.contains("CHERRY"), "{case}: {reply}");
        assert!(reply.ends_with("what now"), "{case}: {reply}");
    };

    // Pi refuses the fork.
    let fail = PathBuf::from(&w.cwd).join("fail-fork");
    std::fs::write(&fail, "").unwrap();
    w.fork("refused", "src", &at, false).await;
    history(&w.turn("refused", "r1", "what now").await, "refused");
    std::fs::remove_file(&fail).unwrap();
    assert!(texts(&w.session_file("refused")).0.get("parentSession").is_none());

    // The source's session file is gone.
    let aside = src_file.with_extension("aside");
    std::fs::rename(&src_file, &aside).unwrap();
    w.fork("missing", "src", &at, false).await;
    history(&w.turn("missing", "m1", "what now").await, "missing");
    std::fs::rename(&aside, &src_file).unwrap();

    let doc = w.core.doc_host.open("src").unwrap();
    // The source switched provider, or was rewound.
    doc.doc().mark_provider_switched().unwrap();
    w.fork("switched", "src", &at, false).await;
    history(&w.turn("switched", "s1", "what now").await, "switched");
    w.core.shutdown().await;
}

#[tokio::test]
async fn a_rewound_source_and_an_old_chat_fork_with_the_history() {
    let w = World::new();
    w.source().await;
    w.core.sessions.rewind("src", "u3").await.unwrap();
    w.fork("rewound", "src", &w.end_of("src", "u2"), false).await;
    let reply = w.turn("rewound", "r1", "what now").await;
    assert!(reply.contains("PINEAPPLE") && reply.contains("BANANA") && reply.ends_with("what now"), "{reply}");
    assert!(texts(&w.session_file("rewound")).0.get("parentSession").is_none());

    // An old chat: its turns left no marks.
    w.chat("old");
    let doc = w.core.doc_host.open("old").unwrap();
    for (id, role, text) in [("o1", MessageRole::User, "say KIWI"), ("o2", MessageRole::Assistant, "KIWI")] {
        doc.doc()
            .push_message(&zend_doc::SessionMessageEntry {
                duration_ms: None,
                id: id.into(),
                role,
                parts: vec![MessagePart::Text { id: format!("{id}-t"), text: text.into() }],
                created_at: 1,
                device_id: "device".into(),
                status: Some(zend_doc::MessageStatus::Complete),
                continuation_of: None,
            })
            .unwrap();
    }
    w.fork("of-old", "old", "o2", false).await;
    let reply = w.turn("of-old", "x1", "what now").await;
    assert!(reply.contains("KIWI") && reply.ends_with("what now"), "{reply}");
    w.core.shutdown().await;
}
