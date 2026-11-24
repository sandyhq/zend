//! A paired server (Caller::PeerServer, over the real peer listener) reaches
//! only what this side granted it: nothing with no grants; with
//! sessions.run:<p> its own chats in <p>, marked sandboxed and refused to
//! run until the sandbox lands; files only in a projects.share project, by
//! a path from its root, checked after symlinks; never a key or token.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use zend_doc::commands::SessionCommandPayload;
use zend_engine::peer_link::{PeerLink, Seams};
use zend_engine::{EngineCore, HarnessRegistry};
use zend_proto::{HarnessId, RunRequest, SandboxLevel};
use zend_rpc::{RpcClient, RpcError, methods};

fn assemble(data_dir: &Path, device_id: &str) -> EngineCore {
    std::fs::create_dir_all(data_dir).expect("data dir");
    std::fs::write(data_dir.join("device-id"), device_id).expect("device id");
    EngineCore::assemble(data_dir, Arc::new(HarnessRegistry::new()), HarnessId::Mock, None).expect("engine assembles")
}

fn loopback() -> Seams {
    Seams {
        discover: || {
            Box::pin(std::future::ready(Some(zend_engine::phone_link::Tailnet {
                name: None,
                ips: vec!["127.0.0.1".parse().unwrap()],
            })))
        },
        bind: |addr| Box::pin(tokio::net::TcpListener::bind(addr)),
        admits: |_| Box::pin(std::future::ready(true)),
    }
}

/// This engine (A) with its peer link, a second server (B) paired with it,
/// and B dialled in with \`grants\` given on A's side.
struct Paired {
    _temp: tempfile::TempDir,
    root: std::path::PathBuf,
    data: std::path::PathBuf,
    core: EngineCore,
    link: Arc<PeerLink>,
    b_key: String,
    b: RpcClient,
}

async fn paired(grants: &[&str]) -> Paired {
    let temp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(temp.path()).unwrap();
    let data = root.join("a-data");
    let core = assemble(&data, "device-a");
    for project in ["p", "q"] {
        let dir = root.join(project);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("readme.md"), format!("project {project}\n")).unwrap();
        core.workspace.create_space(project, &core.device_id, &dir.to_string_lossy(), None, false).unwrap();
    }
    std::fs::write(data.join("ipc-token"), "IPC-TOKEN-SECRET\n").unwrap();
    std::fs::write(root.join("outside.txt"), "OUTSIDE-SECRET\n").unwrap();
    let link = PeerLink::new(&data, 0, core.rpc_service(), loopback());
    let other = tempfile::tempdir().unwrap();
    let b_link = PeerLink::new(other.path(), 0, core.rpc_service(), loopback());
    let code = link.pair().await.expect("pair");
    b_link.join(code["code"].as_str().unwrap()).await.expect("join");
    let b_key = b_link.node_key().unwrap();
    let a_key = link.node_key().unwrap();
    link.set_grants(b_key.public(), &grants.iter().map(|g| g.to_string()).collect::<Vec<_>>()).expect("grants");
    let port = link.status().await.unwrap()["port"].as_u64().unwrap();
    let b = tokio::time::timeout(
        Duration::from_secs(5),
        zend_rpc::connect_peer(&format!("ws://127.0.0.1:{port}"), &b_key, a_key.public()),
    )
    .await
    .expect("dial timed out")
    .expect("B dials in");
    std::mem::forget(other);
    Paired { _temp: temp, root, data, core, link, b_key: b_key.public().to_string(), b }
}

fn create(chat: &str, project: &str) -> Value {
    json!({ "op": "createChat", "chatId": chat, "spaceId": project })
}

fn run(chat: &str, cwd: &str) -> Value {
    let request = RunRequest {
        mcp: None,
        prompt: "hello".into(),
        harness: Some(HarnessId::Mock),
        model: None,
        reasoning: None,
        model_options: Default::default(),
        cwd: cwd.into(),
        sandbox: SandboxLevel::DangerFullAccess,
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
    json!({ "chatId": chat, "command": SessionCommandPayload::Run { request, message_id: format!("m-{chat}") } })
}

fn refused(reply: &Result<Value, RpcError>) -> bool {
    reply.is_err()
}

/// With no grants every method a server could call is refused, through the
/// real engine, with a reason.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn with_no_grants_a_server_reaches_nothing() {
    let a = paired(&[]).await;
    a.core.workspace.create_chat("owner-chat", Some("p"), Some(&a.core.device_id), None, None).unwrap();
    for (method, params) in [
        (methods::ENGINE_INFO, json!({})),
        (methods::WATCH_CHATS, json!({})),
        (methods::MUTATE, create("c1", "p")),
        (methods::QUEUE_COMMAND, run("owner-chat", "/")),
        (methods::QUEUE_MESSAGE, json!({ "chatId": "owner-chat", "text": "hi" })),
        (methods::READ_WORKSPACE_FILE, json!({ "chatId": "owner-chat", "path": "readme.md" })),
        (methods::PEER_READ_PROJECT_FILE, json!({ "project": "p", "path": "readme.md" })),
        (methods::PEER_LIST_PROJECT_FILES, json!({ "project": "p" })),
        (methods::SET_PEER_GRANTS, json!({ "peer": a.b_key, "grants": ["sessions.read"] })),
        (methods::LIST_PEERS, json!({})),
    ] {
        let reply = a.b.call(method, params).await;
        let why = reply.as_ref().err().map(ToString::to_string).unwrap_or_default();
        assert!(refused(&reply) && !why.is_empty(), "{method}: {reply:?}");
    }
    assert!(a.core.workspace.chat("c1").unwrap().is_none());
    assert!(a.link.peers()[0].grants.is_empty());
}

/// sessions.run:p: B creates a chat in p (marked sandbox "project") and
/// reads its messages; in q, and the owner's own chat in p, are refused;
/// running the marked chat is refused, not run unconfined.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sessions_run_reaches_its_own_chats_in_that_project_only() {
    let a = paired(&["sessions.run:p"]).await;
    a.core.workspace.create_chat("owner-chat", Some("p"), Some(&a.core.device_id), None, None).unwrap();

    let created = a.b.call(methods::MUTATE, create("peer-chat", "p")).await.expect("create in p");
    assert_eq!(created["sandbox"], "project", "{created}");
    assert_eq!(a.core.workspace.chat("peer-chat").unwrap().unwrap().space_id.as_deref(), Some("p"));
    assert_eq!(a.core.sessions.chat_sandbox("peer-chat"), Some(zend_proto::ChatSandbox::Project));

    let mut watch = a.b
        .subscribe_scoped(methods::WATCH_DOC_MESSAGES, json!({ "chatId": "peer-chat" }))
        .await
        .expect("watch own chat");
    tokio::time::timeout(Duration::from_secs(5), watch.recv()).await.expect("messages in time").expect("stream alive");

    for (what, reply) in [
        ("create in q", a.b.call(methods::MUTATE, create("q-chat", "q")).await),
        ("create without project", a.b.call(methods::MUTATE, json!({ "op": "createChat", "chatId": "loose" })).await),
        ("create with a cwd", a.b.call(methods::MUTATE, json!({ "op": "createChat", "chatId": "cwd-chat", "spaceId": "p", "cwd": "/" })).await),
        ("another mutate", a.b.call(methods::MUTATE, json!({ "op": "deleteChat", "chatId": "owner-chat" })).await),
        ("run the owner's chat", a.b.call(methods::QUEUE_COMMAND, run("owner-chat", "/")).await),
        ("run its own, sandboxed", a.b.call(methods::QUEUE_COMMAND, run("peer-chat", "/")).await),
    ] {
        assert!(refused(&reply), "{what}: {reply:?}");
        if what == "run its own, sandboxed" {
            assert!(reply.unwrap_err().to_string().contains("sandboxed"));
        }
    }
    // Not from this device's own side either: the marked chat doesn't run.
    let request: RunRequest = serde_json::from_value(run("peer-chat", "/")["command"]["request"].clone()).unwrap();
    let own = a.core.sessions.start_idle("peer-chat", request).await;
    assert!(own.as_ref().is_err_and(|e| e.to_string().contains("sandboxed")), "{own:?}");
    let owner = a.b.subscribe_checked(methods::WATCH_DOC_MESSAGES, json!({ "chatId": "owner-chat" })).await;
    assert!(owner.is_err(), "the owner's chat watched");
    assert!(a.core.workspace.chat("q-chat").unwrap().is_none());
    assert!(a.core.workspace.chat("owner-chat").unwrap().is_some());
}

/// sessions.run.unconfined:p: the chat is not marked, and its run starts.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unconfined_grant_runs_unmarked() {
    let a = paired(&["sessions.run.unconfined:p"]).await;
    let created = a.b.call(methods::MUTATE, create("free-chat", "p")).await.expect("create");
    assert!(created["sandbox"].is_null(), "{created}");
    assert_eq!(a.core.sessions.chat_sandbox("free-chat"), None);
    a.b.call(methods::QUEUE_COMMAND, run("free-chat", "/")).await.expect("runs");
}

/// projects.share:p: B reads and lists p by a path from its root; ../, an
/// absolute path and a link out of p are refused, q isn't shared, and no
/// reply carries a secret.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shared_files_stay_inside_their_project() {
    let a = paired(&["projects.share:p"]).await;
    let p = a.root.join("p");
    std::os::unix::fs::symlink(a.root.join("outside.txt"), p.join("out-link")).unwrap();
    std::os::unix::fs::symlink(&a.data, p.join("data-link")).unwrap();

    let read = a.b.call(methods::PEER_READ_PROJECT_FILE, json!({ "project": "p", "path": "readme.md" })).await.unwrap();
    assert_eq!(read["text"], "project p\n");
    let listed = a.b.call(methods::PEER_LIST_PROJECT_FILES, json!({ "project": "p" })).await.unwrap();
    assert!(listed["entries"].as_array().unwrap().iter().any(|e| e["name"] == "readme.md"), "{listed}");

    let absolute = a.root.join("outside.txt").to_string_lossy().into_owned();
    let token = a.data.join("ipc-token").to_string_lossy().into_owned();
    for path in ["../outside.txt", "../a-data/ipc-token", absolute.as_str(), token.as_str(), "out-link", "data-link/ipc-token", "data-link"] {
        for method in [methods::PEER_READ_PROJECT_FILE, methods::PEER_LIST_PROJECT_FILES] {
            let reply = a.b.call(method, json!({ "project": "p", "path": path })).await;
            assert!(refused(&reply), "{method} {path}: {reply:?}");
        }
    }
    let q = a.b.call(methods::PEER_READ_PROJECT_FILE, json!({ "project": "q", "path": "readme.md" })).await;
    assert!(refused(&q), "{q:?}");

    // No peer call answers with a key or a token: the node key's file, the
    // IPC token and the planted secrets stay out of every reply.
    let node_key = std::fs::read(a.data.join(zend_engine::peer_link::KEY_FILE)).unwrap();
    let node_hex: String = node_key.iter().map(|b| format!("{b:02x}")).collect();
    let mut replies = String::new();
    for (method, params) in [
        (methods::PEER_LIST_PROJECT_FILES, json!({ "project": "p" })),
        (methods::PEER_READ_PROJECT_FILE, json!({ "project": "p", "path": "readme.md" })),
        (methods::LIST_PEERS, json!({})),
        (methods::ENGINE_INFO, json!({})),
        (methods::PEER_PAIR, json!({})),
    ] {
        replies.push_str(&format!("{:?}", a.b.call(method, params).await));
    }
    assert!(!replies.contains("SECRET") && !replies.contains(&node_hex[..32]), "{replies}");
}
