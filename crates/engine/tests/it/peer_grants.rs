//! A paired server (Caller::PeerServer, over the real peer listener) reaches
//! only what this side granted it: nothing with no grants; with
//! sessions.run:<p> its own chats in <p>, run under bubblewrap in p's folder;
//! files only in a projects.share project, by a path from its root, checked
//! after symlinks; never a key or token.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use zend_doc::commands::SessionCommandPayload;
use zend_engine::peer_link::{PeerLink, Seams};
use zend_engine::{EngineCore, HarnessRegistry};
use zend_proto::{HarnessId, RunRequest, SandboxLevel};
use zend_rpc::{Caller, RpcClient, RpcError, RpcService, methods, with_caller};

fn assemble(data_dir: &Path, device_id: &str, registry: HarnessRegistry) -> EngineCore {
    std::fs::create_dir_all(data_dir).expect("data dir");
    std::fs::write(data_dir.join("device-id"), device_id).expect("device id");
    EngineCore::assemble(data_dir, Arc::new(registry), HarnessId::Mock, None).expect("engine assembles")
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
        node_of: |_| Box::pin(std::future::ready(Some("node-test".to_string()))),
    }
}

/// This engine (A) with its peer link, a second server (B) paired with it,
/// and B dialled in with \`grants\` given on A's side.
struct Paired {
    /// First, so it goes first: every shell a test leaves open is killed,
    /// a failed one's too (a live terminal held the run up for minutes).
    _shells: ShellsDown,
    _temp: tempfile::TempDir,
    root: std::path::PathBuf,
    data: std::path::PathBuf,
    core: EngineCore,
    link: Arc<PeerLink>,
    b_key: String,
    b: RpcClient,
    b_node: Arc<zend_rpc::peer_link::NodeKey>,
    a_public: String,
    url: String,
}

impl Paired {
    /// B dialled in again (a grant change cut its connection).
    async fn redial(&mut self) {
        self.b = zend_rpc::connect_peer(&self.url, &self.b_node, &self.a_public).await.expect("B dials in again");
    }
}

async fn paired(grants: &[&str]) -> Paired {
    paired_with(grants, HarnessRegistry::new()).await
}

/// [`paired`], with A's harnesses. Its projects are in the home, as a
/// sandboxed root must be.
async fn paired_with(grants: &[&str], registry: HarnessRegistry) -> Paired {
    let home = std::fs::canonicalize(std::env::var_os("HOME").expect("HOME")).unwrap();
    let temp = tempfile::Builder::new().prefix("zend-peer-test-").tempdir_in(home).unwrap();
    let root = std::fs::canonicalize(temp.path()).unwrap();
    let data = root.join("a-data");
    let core = assemble(&data, "device-a", registry);
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
    let url = format!("ws://127.0.0.1:{port}");
    let b = tokio::time::timeout(
        Duration::from_secs(5),
        zend_rpc::connect_peer(&url, &b_key, a_key.public()),
    )
    .await
    .expect("dial timed out")
    .expect("B dials in");
    std::mem::forget(other);
    Paired {
        _shells: ShellsDown(core.terminals.clone()),
        _temp: temp,
        root,
        data,
        core,
        link,
        b_key: b_key.public().to_string(),
        b,
        b_node: b_key,
        a_public: a_key.public().to_string(),
        url,
    }
}

/// Kills the engine's live shells when dropped.
struct ShellsDown(zend_engine::Terminals);

impl Drop for ShellsDown {
    fn drop(&mut self) {
        self.0.shutdown();
    }
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

/// sessions.run:p: B creates a chat in p (sandboxed to p's folder) and
/// reads its messages and queues its runs; in q, and the owner's own chat in
/// p, are refused; a run of the marked chat outside p doesn't start.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sessions_run_reaches_its_own_chats_in_that_project_only() {
    let a = paired(&["sessions.run:p"]).await;
    a.core.workspace.create_chat("owner-chat", Some("p"), Some(&a.core.device_id), None, None).unwrap();

    a.link.keys().set(&a.b_key, "anthropic", B_KEY).unwrap();
    let mut made = create("peer-chat", "p");
    made["config"] = keyed_config();
    let created = a.b.call(methods::MUTATE, made).await.expect("create in p");
    assert_eq!(created["sandbox"], "project", "{created}");
    assert_eq!(a.core.workspace.chat("peer-chat").unwrap().unwrap().space_id.as_deref(), Some("p"));
    let mark = a.core.sessions.chat_sandbox("peer-chat").expect("marked");
    // Confined to its own working copy of p, in Zend's data.
    assert_eq!((mark.sandbox, mark.root), (zend_proto::ChatSandbox::Project, Some(a.data.join("peer-copies/peer-chat"))));

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
    ] {
        assert!(refused(&reply), "{what}: {reply:?}");
    }
    a.b.call(methods::QUEUE_COMMAND, run("peer-chat", "/")).await.expect("its own run queued");
    // Not outside p, even from this device's own side.
    let request: RunRequest = serde_json::from_value(run("peer-chat", "/")["command"]["request"].clone()).unwrap();
    let own = a.core.sessions.start_idle("peer-chat", request).await;
    assert!(own.as_ref().is_err_and(|e| e.to_string().contains("must run sandboxed")), "{own:?}");
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

/// A project at or around Zend's data folder is never shared: SetPeerGrants
/// refuses it, and a grant kept from before is refused at every read.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_project_holding_zend_data_is_never_shared() {
    let a = paired(&[]).await;
    std::fs::create_dir_all(a.data.join("inner")).unwrap();
    a.core.workspace.create_space("data", &a.core.device_id, &a.data.to_string_lossy(), None, false).unwrap();
    a.core.workspace.create_space("around", &a.core.device_id, &a.root.to_string_lossy(), None, false).unwrap();
    a.core.workspace.create_space("inner", &a.core.device_id, &a.data.join("inner").to_string_lossy(), None, false).unwrap();
    let local = zend_rpc::memory_client(Arc::new(zend_engine::peer_link::PeerLinkRpc { inner: a.core.rpc_service(), link: a.link.clone() }));
    for project in ["data", "around", "inner"] {
        let set = local.call(methods::SET_PEER_GRANTS, json!({ "peer": a.b_key, "grants": [format!("projects.share:{project}")] })).await;
        assert!(set.as_ref().is_err_and(|e| e.to_string().contains("never shared")), "{project}: {set:?}");
    }
    // A project this device doesn't (yet) list can't be checked: refused.
    let unknown = local.call(methods::SET_PEER_GRANTS, json!({ "peer": a.b_key, "grants": ["projects.share:nope"] })).await;
    assert!(unknown.as_ref().is_err_and(|e| e.to_string().contains("no such project")), "{unknown:?}");
    local.call(methods::SET_PEER_GRANTS, json!({ "peer": a.b_key, "grants": ["projects.share:p"] })).await.expect("p shares");
    // Kept from before the check (or the folder moved since): refused at read.
    let mut a = a;
    a.link.set_grants(&a.b_key.clone(), &["projects.share:data".into(), "projects.share:around".into()]).unwrap();
    a.redial().await;
    for (project, path) in [("data", "ipc-token"), ("around", "a-data/ipc-token"), ("around", "outside.txt")] {
        let reply = a.b.call(methods::PEER_READ_PROJECT_FILE, json!({ "project": project, "path": path })).await;
        assert!(reply.as_ref().is_err_and(|e| e.to_string().contains("data folder")), "{project}/{path}: {reply:?}");
        let listed = a.b.call(methods::PEER_LIST_PROJECT_FILES, json!({ "project": project })).await;
        assert!(listed.is_err(), "{project}: {listed:?}");
    }
}

/// An agent in a chat B started calls as B: reading another chat and making
/// one in a project B may not run in are refused, though the agent token
/// would reach them; its own child is B's and confined.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_agent_in_a_peer_chat_reaches_only_what_the_peer_may() {
    let a = paired(&["sessions.run:p"]).await;
    a.core.workspace.create_chat("owner-chat", Some("p"), Some(&a.core.device_id), None, None).unwrap();
    a.b.call(methods::MUTATE, create("peer-chat", "p")).await.expect("create in p");
    let rpc = a.core.rpc_service();
    let agent = |chat: &str| Caller::Agent { chat: Some(chat.to_string()) };
    let config = json!({ "harness": HarnessId::Mock, "sandbox": "danger-full-access", "access": "full-access" });
    let device = a.core.device_id.clone();

    let read = with_caller(agent("peer-chat"), rpc.handle(methods::WATCH_DOC_MESSAGES, json!({ "chatId": "owner-chat" }))).await;
    assert!(read.is_err(), "read the owner's chat");
    let elsewhere = with_caller(agent("peer-chat"), rpc.handle(methods::MUTATE, json!({
        "op": "createChat", "chatId": "q-child", "spaceId": "q", "deviceId": device,
        "parentChatId": "peer-chat", "childKind": "subagent", "config": config,
    }))).await;
    assert!(elsewhere.is_err(), "created in q: {:?}", elsewhere.map(|_| ()));
    let loose = with_caller(agent("peer-chat"), rpc.handle(methods::MUTATE, json!({
        "op": "createChat", "chatId": "loose-child", "deviceId": device, "config": config,
    }))).await;
    assert!(loose.is_err(), "created outside a project");
    let other_parent = with_caller(agent("peer-chat"), rpc.handle(methods::MUTATE, json!({
        "op": "createChat", "chatId": "adopted", "spaceId": "p", "deviceId": device,
        "parentChatId": "owner-chat", "childKind": "subagent", "config": config,
    }))).await;
    assert!(other_parent.is_err(), "made a child of the owner's chat");
    assert!(a.core.workspace.chat("q-child").unwrap().is_none());

    // Its own child, in p: B's, confined, this side's access.
    with_caller(agent("peer-chat"), rpc.handle(methods::MUTATE, json!({
        "op": "createChat", "chatId": "p-child", "spaceId": "p", "deviceId": device,
        "parentChatId": "peer-chat", "childKind": "subagent", "config": config,
    }))).await.expect("its own child in p");
    assert_eq!(a.core.sessions.peer_chats().peer("p-child").as_deref(), Some(a.b_key.as_str()));
    // In its parent's working copy.
    assert_eq!(a.core.sessions.chat_sandbox("p-child").and_then(|m| m.root), Some(a.data.join("peer-copies/peer-chat")));
    let child = a.core.workspace.chat("p-child").unwrap().unwrap();
    assert_eq!(child.parent_chat_id.as_deref(), Some("peer-chat"));
    assert_eq!(child.config.unwrap().access, zend_proto::AccessMode::Supervised);
    // Its child's agent is B too.
    let read = with_caller(agent("p-child"), rpc.handle(methods::WATCH_DOC_MESSAGES, json!({ "chatId": "owner-chat" }))).await;
    assert!(read.is_err(), "the child read the owner's chat");

    // The same calls from the owner's own chat's agent pass this gate.
    with_caller(agent("owner-chat"), rpc.handle(methods::MUTATE, json!({
        "op": "createChat", "chatId": "owner-child", "spaceId": "q", "deviceId": device,
        "parentChatId": "owner-chat", "childKind": "subagent", "config": config,
    }))).await.expect("the owner's agent creates in q");

    // Unpaired: its agent reaches nothing.
    a.link.unpair(&a.b_key).unwrap();
    let after = with_caller(agent("peer-chat"), rpc.handle(methods::ENGINE_INFO, json!({}))).await;
    assert!(after.is_err(), "unpaired, still reached the engine");
}

/// A chat B started unconfined doesn't run once B's grant for its project
/// is only sessions.run (it has no sandbox mark).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_confined_grant_stops_a_chat_started_unconfined() {
    let mut a = paired(&["sessions.run.unconfined:p"]).await;
    a.b.call(methods::MUTATE, create("free-chat", "p")).await.expect("create");
    assert_eq!(a.core.sessions.chat_sandbox("free-chat"), None);
    a.link.set_grants(&a.b_key.clone(), &["sessions.run:p".into()]).unwrap();
    a.redial().await;
    let reply = a.b.call(methods::QUEUE_COMMAND, run("free-chat", "/")).await;
    assert!(reply.as_ref().is_err_and(|e| e.to_string().contains("started unconfined")), "{reply:?}");
}

/// sessions.read: this device's chats and sessions only, and no chat's
/// folder as an absolute path.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reading_sessions_shows_this_devices_own_chats_without_paths() {
    let a = paired(&["sessions.read"]).await;
    let src = a.root.join("p").join("src");
    std::fs::create_dir_all(&src).unwrap();
    a.core.workspace.create_chat("mine", Some("p"), Some(&a.core.device_id), None, Some(src.to_string_lossy().into_owned())).unwrap();
    let elsewhere = a.root.join("r");
    std::fs::create_dir_all(&elsewhere).unwrap();
    a.core.workspace.create_space("r", "device-other", &elsewhere.to_string_lossy(), None, false).unwrap();
    a.core.workspace.create_chat("theirs", Some("r"), Some("device-other"), None, None).unwrap();
    assert_eq!(a.core.workspace.chat("theirs").unwrap().unwrap().device_id, "device-other");
    let mut chats = a.b.subscribe_scoped(methods::WATCH_CHATS, json!({})).await.expect("watch chats");
    let rows = tokio::time::timeout(Duration::from_secs(5), chats.recv()).await.unwrap().unwrap();
    let rows = rows.as_array().cloned().unwrap_or_default();
    let mine = rows.iter().find(|c| c["id"] == "mine").expect("its own chat listed");
    assert_eq!(mine["cwd"], "src", "{mine}");
    assert!(!rows.iter().any(|c| c["id"] == "theirs"), "another device's chat: {rows:?}");
    let shown = serde_json::to_string(&rows).unwrap();
    assert!(!shown.contains(&*a.root.to_string_lossy()), "an absolute path: {shown}");
    let mut sessions = a.b.subscribe_scoped(methods::WATCH_SESSIONS, json!({ "changes": true })).await.expect("watch sessions");
    let rows = tokio::time::timeout(Duration::from_secs(5), sessions.recv()).await.unwrap().unwrap();
    assert!(rows.as_array().is_some_and(|rows| rows.iter().all(|s| s["deviceId"] == a.core.device_id.as_str())), "{rows}");
}

/// A confined chat runs with this side's sandbox, access and plan mode, and
/// model options its model doesn't list are refused.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_confined_chat_takes_this_sides_access_not_the_peers() {
    let a = paired(&["sessions.run:p"]).await;
    let config = json!({ "harness": HarnessId::Mock, "sandbox": "danger-full-access", "access": "full-access", "plan": true });
    a.b.call(methods::MUTATE, json!({ "op": "createChat", "chatId": "c", "spaceId": "p", "config": config })).await.expect("create");
    let kept = a.core.workspace.chat("c").unwrap().unwrap().config.unwrap();
    assert_eq!((kept.sandbox, kept.access, kept.plan), (SandboxLevel::WorkspaceWrite, zend_proto::AccessMode::Supervised, false));
    let odd = json!({ "harness": HarnessId::Mock, "sandbox": "read-only", "modelOptions": { "bogus": "x" } });
    let reply = a.b.call(methods::MUTATE, json!({ "op": "createChat", "chatId": "odd", "spaceId": "p", "config": odd })).await;
    assert!(reply.as_ref().is_err_and(|e| e.to_string().contains("modelOptions")), "{reply:?}");
    assert!(a.core.workspace.chat("odd").unwrap().is_none());
    // No config at all: this device's default harness, under the same pins
    // (the row's fallback would run it with full access).
    a.b.call(methods::MUTATE, json!({ "op": "createChat", "chatId": "bare", "spaceId": "p" })).await.expect("create bare");
    let kept = a.core.workspace.chat("bare").unwrap().unwrap().config.expect("a config of this side's");
    assert_eq!((kept.harness, kept.sandbox, kept.access, kept.plan), (HarnessId::Mock, SandboxLevel::WorkspaceWrite, zend_proto::AccessMode::Supervised, false));
}

/// One run of [`ShellAgent`]: its folder, its sandbox's root (None:
/// unconfined) and what the shell printed.
#[derive(Clone, Debug)]
struct ShellRun {
    cwd: String,
    root: Option<PathBuf>,
    /// Its sandbox's state folder.
    state: Option<PathBuf>,
    out: String,
}

/// An agent that is /bin/sh, started as a sandboxed Pi is
/// (`zend_harness::sandbox::Launch`, bubblewrap and pasta, in the run's
/// sandbox), running the prompt as its script.
struct ShellAgent {
    runs: Arc<Mutex<Vec<ShellRun>>>,
}

#[async_trait::async_trait]
impl zend_harness::Harness for ShellAgent {
    fn id(&self) -> HarnessId {
        HarnessId::Mock
    }
    fn display_name(&self) -> &str {
        "Shell"
    }
    fn supports_steering(&self) -> bool {
        false
    }
    fn steering_mode(&self) -> zend_proto::SteeringMode {
        zend_proto::SteeringMode::TurnBoundary
    }
    fn reasoning_levels(&self) -> &[zend_proto::ReasoningLevel] {
        &[]
    }
    fn access_modes(&self) -> &[zend_proto::AccessMode] {
        &zend_proto::AccessMode::ALL
    }
    fn runs_sandboxed(&self) -> bool {
        true
    }
    async fn models(&self) -> Result<Vec<zend_proto::Model>, zend_harness::HarnessError> {
        Ok(vec![])
    }
    async fn run(
        &self,
        request: RunRequest,
        controls: zend_harness::RunControls,
    ) -> Result<futures::stream::BoxStream<'static, Result<zend_proto::AgentEvent, zend_harness::HarnessError>>, zend_harness::HarnessError>
    {
        use futures::StreamExt as _;
        let failed = |e: String| zend_harness::HarnessError::Sandbox(e);
        let root = controls.sandbox.as_ref().map(|s| s.root().to_path_buf());
        let state = controls.sandbox.as_ref().and_then(|s| s.state().map(Path::to_path_buf));
        let out = match &controls.sandbox {
            None => "UNCONFINED".to_string(),
            Some(sandbox) => {
                let cwd = Path::new(&request.cwd);
                let launch = zend_harness::sandbox::Launch::new(sandbox, HarnessId::Pi, Path::new("/bin/sh"), cwd, &[])?;
                let mut command = launch.command("/bin/sh");
                command
                    .arg("-c")
                    .arg(&request.prompt)
                    .env("PATH", "/usr/bin:/bin")
                    .current_dir(cwd)
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped());
                let child = launch.spawn(&mut command).map_err(|e| failed(e.to_string()))?;
                let output = child.wait_with_output().await.map_err(|e| failed(e.to_string()))?;
                format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr))
            }
        };
        self.runs.lock().unwrap().push(ShellRun { cwd: request.cwd.clone(), root, state, out });
        // A script starting "# hold": its turn stays live until stopped.
        if request.prompt.starts_with("# hold") {
            let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
            tokio::spawn(async move {
                let _ = tx.send(Ok(zend_proto::AgentEvent::SessionStarted {
                    harness: HarnessId::Mock,
                    model: "mock-1".into(),
                    tools: vec![],
                    cwd: request.cwd.clone(),
                    session_id: "shell-held".into(),
                    assistant_message_id: "a-held".into(),
                }));
                // Its sandbox (and what it lends) lives as long as the run,
                // as a real harness's does.
                let held = controls.sandbox;
                controls.interrupt.cancelled().await;
                drop(held);
            });
            return Ok(futures::stream::unfold(rx, |mut rx| async move { rx.recv().await.map(|ev| (ev, rx)) }).boxed());
        }
        Ok(futures::stream::iter(vec![Ok(zend_proto::AgentEvent::Done {
            status: zend_proto::DoneStatus::Completed,
            result: None,
            error: None,
            session_id: None,
        })])
        .boxed())
    }
}

fn sandbox_tools_present() -> bool {
    let present = ["/usr/bin/bwrap", "/usr/bin/pasta"].iter().all(|p| Path::new(p).is_file());
    if !present {
        eprintln!("bwrap or pasta not installed: skipped");
    }
    present
}

async fn shell_run(runs: &Arc<Mutex<Vec<ShellRun>>>, n: usize) -> ShellRun {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(run) = runs.lock().unwrap().get(n - 1).cloned() {
            return run;
        }
        assert!(tokio::time::Instant::now() < deadline, "run {n} didn't start");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// The key B's chats run on for anthropic models, set by the owner.
const B_KEY: &str = "sk-ant-B-ONLY";

/// The config of a chat B makes: a model of a provider B has a key for.
fn keyed_config() -> Value {
    json!({ "harness": HarnessId::Mock, "model": "anthropic/claude-test", "sandbox": "danger-full-access", "access": "full-access" })
}

/// End to end (4d-2, 4d-3): B, granted sessions.run:p (and q unconfined),
/// creates a chat in p over the peer link and queues a run; its agent
/// starts under bubblewrap in its own working copy of p in Zend's data,
/// never in p: the copy is its root and its folder, p itself (the owner's
/// folder), q, the rest of Zend's data and the home are out of its sight,
/// and what it writes lands in the copy only. Of the credentials in its
/// environment only B's key is there, as the provider's variable. A child
/// its agent starts shares the copy; not in q, and no run of it outside.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_peer_chat_runs_in_its_own_working_copy_on_its_servers_key_and_so_do_its_children() {
    if !sandbox_tools_present() {
        return;
    }
    let runs = Arc::new(Mutex::new(Vec::new()));
    let registry = HarnessRegistry::new();
    registry.register(Arc::new(ShellAgent { runs: runs.clone() }));
    let a = paired_with(&["sessions.run:p", "sessions.run.unconfined:q"], registry).await;
    a.core.sessions.set_no_ipc_port();
    a.link.keys().set(&a.b_key, "anthropic", B_KEY).unwrap();
    let p = a.root.join("p");
    let q = a.root.join("q");
    let copy = a.data.join(zend_engine::peer_copies::COPIES_DIR).join("peer-chat");
    let script = format!(
        r#"for f in {p}/readme.md {q}/readme.md {data}/ipc-token {data}/peers.json {data}/peer-keys.json {base}/outside.txt; do
            if cat "$f" >/dev/null 2>&1; then echo "READABLE $f"; fi
        done
        (echo x > {q}/written.txt) 2>/dev/null && echo "WROTE Q"
        (echo x > {p}/written.txt) 2>/dev/null && echo "WROTE P"
        cat ./readme.md
        echo made > ./made-in-copy.txt && echo "wrote copy"
        echo "in $(pwd)"
        env | grep -E 'KEY|TOKEN|SECRET|AUTH' || true"#,
        p = p.display(),
        q = q.display(),
        data = a.data.display(),
        base = a.root.display(),
    );
    let created = a
        .b
        .call(methods::MUTATE, json!({ "op": "createChat", "chatId": "peer-chat", "spaceId": "p", "config": keyed_config() }))
        .await
        .expect("create in p");
    assert_eq!(created["sandbox"], "project");
    assert_eq!(a.core.sessions.chat_sandbox("peer-chat").and_then(|m| m.root), Some(copy.clone()));
    assert_eq!(std::fs::read_to_string(copy.join("readme.md")).unwrap(), "project p\n");

    let mut queued = run("peer-chat", "/");
    queued["command"]["request"]["prompt"] = script.clone().into();
    a.b.call(methods::QUEUE_COMMAND, queued).await.expect("queued");
    let first = shell_run(&runs, 1).await;
    println!("peer chat: {first:?}");
    assert_eq!((Path::new(&first.cwd), first.root.as_ref()), (copy.as_path(), Some(&copy)), "{first:?}");
    assert!(first.out.contains("project p") && first.out.contains("wrote copy"), "{}", first.out);
    assert!(first.out.contains(&format!("in {}", copy.display())), "{}", first.out);
    assert!(!first.out.contains("READABLE") && !first.out.contains("WROTE"), "{}", first.out);
    let credentials: Vec<&str> = first.out.lines().filter(|l| l.contains('=')).collect();
    assert_eq!(credentials, [format!("ANTHROPIC_API_KEY={B_KEY}")], "{}", first.out);
    assert_eq!(std::fs::read_to_string(copy.join("made-in-copy.txt")).unwrap(), "made\n");
    assert!(!p.join("made-in-copy.txt").exists() && !p.join("written.txt").exists() && !q.join("written.txt").exists());

    // Its agent's children: in its copy.
    let rpc = a.core.rpc_service();
    let agent = Caller::Agent { chat: Some("peer-chat".into()) };
    let child = |id: &str, space: &str| {
        json!({
            "op": "createChat", "chatId": id, "spaceId": space, "deviceId": a.core.device_id,
            "parentChatId": "peer-chat", "childKind": "subagent", "config": keyed_config(),
        })
    };
    let in_q = with_caller(agent.clone(), rpc.handle(methods::MUTATE, child("q-child", "q"))).await;
    assert!(in_q.as_ref().is_err_and(|e| e.to_string().contains("stays in its project")), "{:?}", in_q.map(|_| ()));
    assert!(a.core.workspace.chat("q-child").unwrap().is_none());
    with_caller(agent, rpc.handle(methods::MUTATE, child("p-child", "p"))).await.expect("a child in p");
    assert_eq!(a.core.sessions.chat_sandbox("p-child").and_then(|m| m.root), Some(copy.clone()));
    let mut queued = run("p-child", "/");
    queued["command"]["request"]["prompt"] = script.into();
    a.b.call(methods::QUEUE_COMMAND, queued).await.expect("the child's run queued");
    let second = shell_run(&runs, 2).await;
    println!("child: {second:?}");
    assert_eq!((Path::new(&second.cwd), second.root.as_ref()), (copy.as_path(), Some(&copy)), "{second:?}");
    assert!(second.out.contains("project p") && !second.out.contains("READABLE") && !second.out.contains("WROTE"), "{}", second.out);
    // A run of the child in q, or in the owner's p beyond the copy's map, doesn't start.
    let mut request: RunRequest = serde_json::from_value(run("p-child", "/")["command"]["request"].clone()).unwrap();
    request.cwd = q.display().to_string();
    request.model = Some("anthropic/claude-test".into());
    let outside = a.core.sessions.start_idle("p-child", request).await;
    assert!(outside.as_ref().is_err_and(|e| e.to_string().contains("outside its project")), "{outside:?}");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(runs.lock().unwrap().len(), 2, "{:?}", runs.lock().unwrap());
}

/// The owner's own methods (this device's app or CLI): the peer link's
/// RPC as a local caller sees it.
fn owner(a: &Paired) -> RpcClient {
    zend_rpc::memory_client(Arc::new(zend_engine::peer_link::PeerLinkRpc { inner: a.core.rpc_service(), link: a.link.clone() }))
}

/// Show the copy's changes, then apply exactly what was shown (its tree).
async fn apply_shown(owner: &RpcClient, chat: &str) -> Result<Value, String> {
    let shown = owner.call(methods::SHOW_PEER_CHANGES, json!({ "chat": chat })).await.map_err(|e| e.to_string())?;
    owner
        .call(methods::APPLY_PEER_CHANGES, json!({ "chat": chat, "tree": shown["tree"] }))
        .await
        .map_err(|e| e.to_string())
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git").arg("-C").arg(dir).args(args).output().expect("git");
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn git_project(dir: &Path) {
    git(dir, &["init", "-q", "-b", "main"]);
    git(dir, &["config", "user.email", "o@example.com"]);
    git(dir, &["config", "user.name", "Owner"]);
    std::fs::write(dir.join("gone.txt"), "to delete\n").unwrap();
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", "base"]);
}

/// 4d-2: a chat B makes in a git project works in a worktree of it on a
/// branch of its own, in Zend's data; what it changes comes back only as a
/// patch the owner lists, reads and applies here (all or nothing: refused
/// for .git and the protected dot-folders, a link, a path through a link in
/// the owner's folder, a file the owner changed since), or throws away.
/// These methods are the owner's: refused to an agent, a phone and B.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_servers_changes_reach_the_owners_folder_only_as_a_patch_the_owner_applies() {
    let a = paired(&["sessions.run:p"]).await;
    let p = a.root.join("p");
    git_project(&p);
    a.b.call(methods::MUTATE, json!({ "op": "createChat", "chatId": "theirs", "spaceId": "p", "config": keyed_config() })).await.expect("create");
    let copy = a.core.sessions.chat_sandbox("theirs").and_then(|m| m.root).expect("confined");
    assert_eq!(copy, a.data.join("peer-copies/theirs"));
    assert!(copy.join(".git").is_file(), "a worktree");
    // Detached: no branch of it in the owner's repository.
    assert_eq!(git(&p, &["for-each-ref", "--format=%(refname)"]).trim(), "refs/heads/main");
    // The worktree's git folder is the owner's, outside the copy.
    let pointer = std::fs::read_to_string(copy.join(".git")).unwrap();
    assert!(pointer.contains(&p.join(".git/worktrees").display().to_string()), "{pointer}");

    let owner = owner(&a);
    let listed = owner.call(methods::LIST_PEER_CHANGES, json!({})).await.unwrap();
    assert_eq!(listed["copies"][0]["chat"], "theirs");
    assert_eq!(listed["copies"][0]["kind"], "worktree");
    assert_eq!(listed["copies"][0]["changes"], json!([]));

    // What its agent did in the copy.
    std::fs::write(copy.join("readme.md"), "project p, changed\n").unwrap();
    std::fs::create_dir_all(copy.join("src")).unwrap();
    std::fs::write(copy.join("src/new.rs"), "fn main() {}\n").unwrap();
    std::fs::remove_file(copy.join("gone.txt")).unwrap();
    std::fs::create_dir_all(copy.join(".pi")).unwrap();
    std::fs::write(copy.join(".pi/settings.json"), "{}").unwrap();
    let changes = owner.call(methods::LIST_PEER_CHANGES, json!({})).await.unwrap()["copies"][0]["changes"].clone();
    let paths: Vec<(&str, &str)> = changes.as_array().unwrap().iter().map(|c| (c["status"].as_str().unwrap(), c["path"].as_str().unwrap())).collect();
    let mut paths: Vec<(&str, &str)> = paths;
    paths.sort_by_key(|(_, path)| *path);
    assert_eq!(paths, [("A", ".pi/settings.json"), ("D", "gone.txt"), ("M", "readme.md"), ("A", "src/new.rs")], "{changes}");
    let patch = owner.call(methods::SHOW_PEER_CHANGES, json!({ "chat": "theirs" })).await.unwrap()["patch"].as_str().unwrap().to_string();
    assert!(patch.contains("+project p, changed") && patch.contains("src/new.rs") && patch.contains("deleted file mode"), "{patch}");

    // Nobody but the owner.
    for caller in [Caller::Agent { chat: Some("theirs".into()) }, Caller::Remote] {
        for method in [methods::LIST_PEER_CHANGES, methods::SHOW_PEER_CHANGES, methods::APPLY_PEER_CHANGES, methods::DISCARD_PEER_CHANGES, methods::SET_PEER_KEY, methods::LIST_PEER_KEYS] {
            let service = zend_engine::peer_link::PeerLinkRpc { inner: a.core.rpc_service(), link: a.link.clone() };
            let reply = with_caller(caller.clone(), service.handle(method, json!({ "chat": "theirs", "peer": a.b_key, "provider": "anthropic", "key": "x" }))).await;
            assert!(reply.is_err(), "{method} for {caller:?}");
        }
    }
    for method in [methods::LIST_PEER_CHANGES, methods::SHOW_PEER_CHANGES, methods::APPLY_PEER_CHANGES, methods::DISCARD_PEER_CHANGES, methods::LIST_PEER_KEYS] {
        assert!(a.b.call(method, json!({ "chat": "theirs" })).await.is_err(), "{method} for B");
    }

    // Review item 1: what is applied is what was shown; written since, refused.
    let shown = owner.call(methods::SHOW_PEER_CHANGES, json!({ "chat": "theirs" })).await.unwrap();
    std::fs::write(copy.join("unread.txt"), "written after the owner read it
").unwrap();
    let moved = owner.call(methods::APPLY_PEER_CHANGES, json!({ "chat": "theirs", "tree": shown["tree"] })).await.unwrap_err().to_string();
    assert!(moved.contains("changed since you were shown"), "{moved}");
    assert!(!p.join("unread.txt").exists());
    std::fs::remove_file(copy.join("unread.txt")).unwrap();
    // Without the tree: not a request.
    assert!(owner.call(methods::APPLY_PEER_CHANGES, json!({ "chat": "theirs" })).await.is_err());


    // A protected folder: nothing applied.
    let refused = apply_shown(&owner, "theirs").await.unwrap_err();
    assert!(refused.contains("nothing applied") && refused.contains(".pi"), "{refused}");
    assert_eq!(std::fs::read_to_string(p.join("readme.md")).unwrap(), "project p\n");
    assert!(p.join("gone.txt").exists() && !p.join("src").exists() && !p.join(".pi").exists());
    std::fs::remove_dir_all(copy.join(".pi")).unwrap();
    // A link in the copy: nothing applied.
    std::os::unix::fs::symlink("/etc/passwd", copy.join("link")).unwrap();
    let refused = apply_shown(&owner, "theirs").await.unwrap_err();
    assert!(refused.contains("symbolic link"), "{refused}");
    std::fs::remove_file(copy.join("link")).unwrap();
    // A folder in the owner's p that is a link out: nothing applied there.
    let elsewhere = a.root.join("elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();
    std::os::unix::fs::symlink(&elsewhere, p.join("src")).unwrap();
    let refused = apply_shown(&owner, "theirs").await.unwrap_err();
    assert!(refused.contains("is a link"), "{refused}");
    assert!(std::fs::read_dir(&elsewhere).unwrap().next().is_none());
    std::fs::remove_file(p.join("src")).unwrap();

    let applied = apply_shown(&owner, "theirs").await.unwrap();
    assert_eq!(applied["applied"].as_array().unwrap().len(), 3, "{applied}");
    assert_eq!(std::fs::read_to_string(p.join("readme.md")).unwrap(), "project p, changed\n");
    assert_eq!(std::fs::read_to_string(p.join("src/new.rs")).unwrap(), "fn main() {}\n");
    assert!(!p.join("gone.txt").exists());
    // Applied: the base moves on.
    let listed = owner.call(methods::LIST_PEER_CHANGES, json!({})).await.unwrap();
    assert_eq!(listed["copies"][0]["changes"], json!([]), "{listed}");
    // The owner changed it since: refused.
    std::fs::write(p.join("readme.md"), "the owner's\n").unwrap();
    std::fs::write(copy.join("readme.md"), "theirs again\n").unwrap();
    let refused = apply_shown(&owner, "theirs").await.unwrap_err();
    assert!(refused.contains("changed in your folder"), "{refused}");
    assert_eq!(std::fs::read_to_string(p.join("readme.md")).unwrap(), "the owner's\n");
    // The owner's folder has its git back: it isn't confined.
    assert!(git(&p, &["status", "--porcelain"]).contains("readme.md"));

    // Thrown away: the copy goes, the owner's folder stays, the chat doesn't run.
    owner.call(methods::DISCARD_PEER_CHANGES, json!({ "chat": "theirs" })).await.unwrap();
    assert!(!copy.exists());
    assert!(!git(&p, &["branch", "--list"]).contains("zend-peer/theirs"));
    assert!(!git(&p, &["worktree", "list"]).contains("peer-copies"));
    assert_eq!(std::fs::read_to_string(p.join("readme.md")).unwrap(), "the owner's\n");
    a.link.keys().set(&a.b_key, "anthropic", B_KEY).unwrap();
    let mut request: RunRequest = serde_json::from_value(run("theirs", "/")["command"]["request"].clone()).unwrap();
    request.cwd = p.display().to_string();
    request.model = Some("anthropic/claude-test".into());
    let gone = a.core.sessions.start_idle("theirs", request).await;
    assert!(gone.as_ref().is_err_and(|e| e.to_string().contains("working copy is gone")), "{gone:?}");
    assert!(owner.call(methods::LIST_PEER_CHANGES, json!({})).await.unwrap()["copies"].as_array().unwrap().is_empty());
}

/// 4d-2: the copy's git can't make host git run what its agent planted.
/// The agent writes the copy, not its `.git` pointer (read-only) nor the
/// worktree's git folder (the owner's, never in its sandbox); the owner's
/// own filter driver, named by a `.gitattributes` it planted, and a
/// repository it made in the copy (its config naming an fsmonitor) run
/// nothing when the owner lists or applies its changes.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_copys_git_runs_nothing_its_agent_planted() {
    if !sandbox_tools_present() {
        return;
    }
    let runs = Arc::new(Mutex::new(Vec::new()));
    let registry = HarnessRegistry::new();
    registry.register(Arc::new(ShellAgent { runs: runs.clone() }));
    let a = paired_with(&["sessions.run:p"], registry).await;
    a.core.sessions.set_no_ipc_port();
    a.link.keys().set(&a.b_key, "anthropic", B_KEY).unwrap();
    let p = a.root.join("p");
    git_project(&p);
    let marker = a.root.join("PLANTED-RAN");
    // The owner's own driver (as git-lfs sets one up): a planted
    // attribute that names it would run it, as the owner.
    git(&p, &["config", "filter.evil.clean", &format!("touch {}; cat", marker.display())]);
    git(&p, &["config", "diff.evil.textconv", &format!("touch {}; cat", marker.display())]);
    a.b.call(methods::MUTATE, json!({ "op": "createChat", "chatId": "theirs", "spaceId": "p", "config": keyed_config() })).await.expect("create");
    let copy = a.core.sessions.chat_sandbox("theirs").and_then(|m| m.root).expect("confined");
    let worktree_git = p.join(".git/worktrees/theirs");
    assert!(worktree_git.is_dir());
    let script = format!(
        r#"(echo "gitdir: /tmp/evil" > .git) 2>/dev/null && echo "WROTE POINTER"
        (rm -f .git) 2>/dev/null && echo "REMOVED POINTER"
        (echo x >> {wt}/config) 2>/dev/null && echo "WROTE WORKTREE GIT"
        cat {wt}/HEAD >/dev/null 2>&1 && echo "READ WORKTREE GIT"
        printf '* filter=evil diff=evil
' > .gitattributes
        mkdir -p sub/.git && printf '[core]
	fsmonitor = touch {marker}
' > sub/.git/config && echo "ref: refs/heads/main" > sub/.git/HEAD
        echo changed > readme.md
        echo planted"#,
        wt = worktree_git.display(),
        marker = marker.display(),
    );
    let mut queued = run("theirs", "/");
    queued["command"]["request"]["prompt"] = script.into();
    a.b.call(methods::QUEUE_COMMAND, queued).await.expect("queued");
    let ran = shell_run(&runs, 1).await;
    println!("{ran:?}");
    assert!(ran.out.contains("planted"), "{}", ran.out);
    for not in ["WROTE POINTER", "REMOVED POINTER", "WROTE WORKTREE GIT", "READ WORKTREE GIT"] {
        assert!(!ran.out.contains(not), "{not}: {}", ran.out);
    }
    assert!(std::fs::read_to_string(copy.join(".git")).unwrap().contains("worktrees/theirs"));

    let owner = owner(&a);
    // A repository in the copy: not opened, the owner told.
    let listed = owner.call(methods::LIST_PEER_CHANGES, json!({})).await.unwrap();
    assert!(listed["copies"][0]["error"].as_str().unwrap_or_default().contains("git repository of its own"), "{listed}");
    assert!(apply_shown(&owner, "theirs").await.is_err());
    std::fs::remove_dir_all(copy.join("sub")).unwrap();
    let patch = owner.call(methods::SHOW_PEER_CHANGES, json!({ "chat": "theirs" })).await.unwrap()["patch"].as_str().unwrap().to_string();
    assert!(patch.contains("+changed"), "{patch}");
    // The planted attributes would make the owner's git run its drivers.
    let refused = apply_shown(&owner, "theirs").await.unwrap_err();
    assert!(refused.contains(".gitattributes") && refused.contains("nothing applied"), "{refused}");
    std::fs::remove_file(copy.join(".gitattributes")).unwrap();
    let applied = apply_shown(&owner, "theirs").await.unwrap();
    assert_eq!(applied["applied"], json!(["readme.md"]), "{applied}");
    assert_eq!(std::fs::read_to_string(p.join("readme.md")).unwrap(), "changed\n");
    assert!(!marker.exists(), "a planted driver or fsmonitor ran");
    // The owner's own git in its folder: its config, its attributes.
    git(&p, &["status"]);
    git(&p, &["diff"]);
    assert!(!marker.exists(), "a planted driver ran in the owner's folder");
}

/// Review item 1: while a run of a chat in the copy is live, the owner
/// can't list, read or apply it (what it shows could change under them);
/// once the run has ended (its sandbox gone), they can.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_copy_is_read_and_applied_only_while_nothing_runs_in_it() {
    if !sandbox_tools_present() {
        return;
    }
    let runs = Arc::new(Mutex::new(Vec::new()));
    let registry = HarnessRegistry::new();
    registry.register(Arc::new(ShellAgent { runs: runs.clone() }));
    let a = paired_with(&["sessions.run:p"], registry).await;
    a.core.sessions.set_no_ipc_port();
    a.link.keys().set(&a.b_key, "anthropic", B_KEY).unwrap();
    a.b.call(methods::MUTATE, json!({ "op": "createChat", "chatId": "theirs", "spaceId": "p", "config": keyed_config() })).await.expect("create");
    let copy = a.core.sessions.chat_sandbox("theirs").and_then(|m| m.root).expect("confined");
    let mut queued = run("theirs", "/");
    queued["command"]["request"]["prompt"] = "echo early > readme.md; touch started; sleep 3; echo late > readme.md; rm started; echo done".into();
    a.b.call(methods::QUEUE_COMMAND, queued).await.expect("queued");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    while !copy.join("started").exists() {
        assert!(tokio::time::Instant::now() < deadline, "the run didn't start");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let owner = owner(&a);
    let listed = owner.call(methods::LIST_PEER_CHANGES, json!({})).await.unwrap();
    let error = listed["copies"][0]["error"].as_str().unwrap_or_default().to_string();
    assert!(error.contains("chat theirs has a live run") && error.contains("stop it, or wait"), "{listed}");
    let shown = owner.call(methods::SHOW_PEER_CHANGES, json!({ "chat": "theirs" })).await.unwrap_err().to_string();
    assert!(shown.contains("live run"), "{shown}");
    let applied = owner.call(methods::APPLY_PEER_CHANGES, json!({ "chat": "theirs", "tree": "x" })).await.unwrap_err().to_string();
    assert!(applied.contains("live run"), "{applied}");
    assert_eq!(std::fs::read_to_string(a.root.join("p/readme.md")).unwrap(), "project p\n");
    let ran = shell_run(&runs, 1).await;
    assert!(ran.out.contains("done"), "{}", ran.out);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    let applied = loop {
        match apply_shown(&owner, "theirs").await {
            Ok(applied) => break applied,
            Err(e) if e.contains("live run") || e.contains("still runs") => {
                assert!(tokio::time::Instant::now() < deadline, "{e}");
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            Err(e) => panic!("{e}"),
        }
    };
    assert_eq!(applied["applied"], json!(["readme.md"]), "{applied}");
    assert_eq!(std::fs::read_to_string(a.root.join("p/readme.md")).unwrap(), "late\n");

    // Review item 6: a chat that isn't the server's, confined to its copy,
    // doesn't run there.
    owner.call(methods::MUTATE, json!({ "op": "createChat", "chatId": "not-theirs", "spaceId": "p", "config": keyed_config() })).await.expect("create");
    a.core.sessions.sandbox_chat("not-theirs", &copy).await.expect("marked");
    let mut request: RunRequest = serde_json::from_value(run("not-theirs", "/")["command"]["request"].clone()).unwrap();
    request.cwd = copy.display().to_string();
    request.model = Some("anthropic/claude-test".into());
    let other = a.core.sessions.dispatch("not-theirs", HarnessId::Mock, request, None).await;
    assert!(other.as_ref().is_err_and(|e| e.to_string().contains("isn't that server's chat")), "{other:?}");
    assert_eq!(runs.lock().unwrap().len(), 1, "it never ran");
}

/// 4d-3: a chat of B's runs only on a key the owner kept for B and its
/// model's provider: none, a key for another provider, the key removed,
/// or B unpaired (its keys go with it),
/// and the run is refused with a clear reason. Listing shows names only.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_servers_chat_runs_only_on_a_key_kept_for_that_server() {
    if !sandbox_tools_present() {
        return;
    }
    let runs = Arc::new(Mutex::new(Vec::new()));
    let registry = HarnessRegistry::new();
    registry.register(Arc::new(ShellAgent { runs: runs.clone() }));
    let a = paired_with(&["sessions.run:p"], registry).await;
    a.core.sessions.set_no_ipc_port();
    a.b.call(methods::MUTATE, json!({ "op": "createChat", "chatId": "theirs", "spaceId": "p", "config": keyed_config() })).await.expect("create");
    let copy = a.core.sessions.chat_sandbox("theirs").and_then(|m| m.root).expect("confined");
    let start = |model: &str| {
        let mut request: RunRequest = serde_json::from_value(run("theirs", "/")["command"]["request"].clone()).unwrap();
        request.cwd = copy.display().to_string();
        request.model = Some(model.into());
        request.prompt = "env | grep -E 'KEY|TOKEN' || true".into();
        a.core.sessions.start_idle("theirs", request)
    };
    let no_key = start("anthropic/claude-test").await;
    // What B is told (4d-4 review): only that there is none here.
    assert!(no_key.as_ref().is_err_and(|e| e.to_string().contains("no credential for anthropic on this device")), "{no_key:?}");

    let owner = owner(&a);
    owner.call(methods::SET_PEER_KEY, json!({ "peer": a.b_key, "provider": "openai", "key": "sk-openai-B" })).await.unwrap();
    let other = start("anthropic/claude-test").await;
    assert!(other.is_err(), "{other:?}");
    let set = owner.call(methods::SET_PEER_KEY, json!({ "peer": a.b_key, "provider": "anthropic", "key": B_KEY })).await.unwrap();
    assert_eq!(set["variable"], "ANTHROPIC_API_KEY");
    let listed = owner.call(methods::LIST_PEER_KEYS, json!({})).await.unwrap();
    assert_eq!(listed["keys"][0]["providers"], json!(["anthropic", "openai"]));
    assert!(!listed.to_string().contains("sk-"), "{listed}");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(a.data.join("peer-keys.json")).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
    let mut queued = run("theirs", "/");
    queued["command"]["request"]["prompt"] = "env | grep -E 'KEY|TOKEN' || true".into();
    a.b.call(methods::QUEUE_COMMAND, queued).await.expect("runs on B's key");
    let ran = shell_run(&runs, 1).await;
    assert_eq!(ran.out.trim(), format!("ANTHROPIC_API_KEY={B_KEY}"), "{}", ran.out);

    owner.call(methods::REMOVE_PEER_KEY, json!({ "peer": a.b_key, "provider": "anthropic" })).await.unwrap();
    let removed = start("anthropic/claude-test").await;
    assert!(removed.is_err(), "{removed:?}");
    owner.call(methods::SET_PEER_KEY, json!({ "peer": a.b_key, "provider": "anthropic", "key": B_KEY })).await.unwrap();
    owner.call(methods::UNPAIR_PEER, json!({ "peer": a.b_key })).await.unwrap();
    assert_eq!(owner.call(methods::LIST_PEER_KEYS, json!({})).await.unwrap()["keys"], json!([]));
    let unpaired = start("anthropic/claude-test").await;
    assert!(unpaired.is_err(), "{unpaired:?}");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(runs.lock().unwrap().len(), 1);
}

const HOST_ACCESS: &str = "sk-ant-oat01-HOST-ACCESS-LENT";
const HOST_REFRESH: &str = "sk-ant-ort01-HOST-REFRESH-KEPT";
const CODEX_ACCESS: &str = "codex-HOST-ACCESS-NOT-LENT";

/// A stand-in for this device's pi login: its folder with an auth.json of
/// two logins and an API key, `minutes` left on the anthropic one.
fn host_pi(root: &Path, minutes: i64) -> (PathBuf, zend_engine::peer_logins::HostLogin) {
    let dir = root.join("host-pi-agent");
    std::fs::create_dir_all(&dir).unwrap();
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64;
    let auth = json!({
        "anthropic": { "type": "oauth", "access": HOST_ACCESS, "refresh": HOST_REFRESH, "expires": now + minutes * 60_000 },
        "openai-codex": { "type": "oauth", "access": CODEX_ACCESS, "refresh": "codex-REFRESH", "expires": now + 6_000 * 60_000, "accountId": "acct" },
        "openrouter": { "type": "api_key", "key": "sk-or-HOST-KEY" },
    });
    let file = dir.join("auth.json");
    std::fs::write(&file, serde_json::to_vec_pretty(&auth).unwrap()).unwrap();
    (file, zend_engine::peer_logins::HostLogin { agent_dir: dir, pi: None })
}

/// Where A keeps the logins it lends for runs (outside every sandbox).
fn kept_dir(a: &Paired) -> PathBuf {
    kept_dir_of(&a.data)
}

/// [`kept_dir`] of an engine assembled on `data` (its device's folder).
fn kept_dir_of(data: &Path) -> PathBuf {
    zend_engine::EngineProfile::development(data, "dev-org", "dev-user").device_root().join("peer-login-tokens")
}

/// The logins A keeps for runs now.
fn kept_logins(a: &Paired) -> Vec<PathBuf> {
    std::fs::read_dir(kept_dir(a)).into_iter().flatten().flatten().map(|e| e.path()).collect()
}

/// Every file of the test's (Zend's data, the projects, the copies) that
/// holds the host's access token, but the host's own login file.
fn holding_token(a: &Paired, host_file: &Path) -> Vec<PathBuf> {
    files_holding(&a.root, HOST_ACCESS).into_iter().filter(|f| f != host_file).collect()
}

/// Every file below `dir` that holds `needle`.
fn files_holding(dir: &Path, needle: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path = entry.path();
            match entry.file_type() {
                Ok(t) if t.is_dir() => stack.push(path),
                Ok(t) if t.is_file() => {
                    if std::fs::read(&path).is_ok_and(|raw| raw.windows(needle.len()).any(|w| w == needle.as_bytes())) {
                        out.push(path);
                    }
                }
                _ => {}
            }
        }
    }
    out
}

/// What the engine logs, kept to search (where this process's subscriber
/// is ours: nextest runs each test alone).
#[derive(Clone, Default)]
struct Logged(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Logged {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// 4d-4: a chat of B's runs on this device's pi login only when the owner
/// allowed B that provider's login. (1) Not allowed: refused, and no token
/// is in its sandbox (also when it runs on a key, which wins). (2) Allowed:
/// its sandbox's auth.json, its PI_CODING_AGENT_DIR, holds that one
/// provider's access token with refresh empty; the host's file is unchanged
/// and unseen. (3) Taking the login back or unpairing ends the live run and
/// takes the file out. (4) The token is in no log line, no journal or
/// other file of Zend's, no reply to B or the owner.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_servers_chat_runs_on_this_devices_login_only_while_allowed() {
    if !sandbox_tools_present() {
        return;
    }
    let logged = Logged::default();
    let capturing = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .with_writer({
            let logged = logged.clone();
            move || logged.clone()
        })
        .try_init()
        .is_ok();
    let runs = Arc::new(Mutex::new(Vec::new()));
    let registry = HarnessRegistry::new();
    registry.register(Arc::new(ShellAgent { runs: runs.clone() }));
    let a = paired_with(&["sessions.run:p"], registry).await;
    a.core.sessions.set_no_ipc_port();
    let (host_file, login) = host_pi(&a.root, 300);
    a.core.sessions.set_host_login(login);
    let host_before = std::fs::read(&host_file).unwrap();
    let mut replies: Vec<String> = Vec::new();
    a.b.call(methods::MUTATE, json!({ "op": "createChat", "chatId": "theirs", "spaceId": "p", "config": keyed_config() })).await.expect("create");
    let copy = a.core.sessions.chat_sandbox("theirs").and_then(|m| m.root).expect("confined");
    let queue = |prompt: String, n: usize| {
        let mut queued = run("theirs", "/");
        queued["command"]["request"]["prompt"] = prompt.into();
        queued["command"]["messageId"] = format!("m-login-{n}").into();
        a.b.call(methods::QUEUE_COMMAND, queued)
    };
    let owner = owner(&a);

    // (1) Not allowed, no key: refused. B hears only that there is no
    // credential for the provider here; how to allow it is in this device's
    // log only.
    let refused = queue("true".into(), 0).await;
    let words = format!("zend peer login allow {} anthropic", a.b_key);
    let told = refused.as_ref().map(|_| String::new()).unwrap_or_else(|e| e.to_string());
    assert!(told.contains("no credential for anthropic on this device"), "{refused:?}");
    for detail in ["zend peer", "login", "API key", "sign in", "expire", "refresh"] {
        assert!(!told.contains(detail), "B is told {detail:?}: {told}");
    }
    replies.push(format!("{refused:?}"));

    // (2) Allowed: one provider's access token, no refresh, in its own
    // agent folder; the host's login file neither changed nor visible.
    let allowed = owner.call(methods::ALLOW_PEER_LOGIN, json!({ "peer": a.b_key, "provider": "anthropic" })).await.unwrap();
    assert_eq!((allowed["provider"].as_str(), allowed["keyWins"].as_bool()), (Some("anthropic"), Some(false)), "{allowed}");
    replies.push(allowed.to_string());
    let codex = owner.call(methods::ALLOW_PEER_LOGIN, json!({ "peer": a.b_key, "provider": "openrouter" })).await;
    assert!(codex.is_err(), "{codex:?}");
    let listed = owner.call(methods::LIST_PEER_LOGINS, json!({})).await.unwrap();
    assert_eq!(listed["logins"][0]["providers"], json!(["anthropic"]), "{listed}");
    replies.push(listed.to_string());
    let script = format!(
        r#"cat "$PI_CODING_AGENT_DIR/auth.json"; echo; echo "DIR=$PI_CODING_AGENT_DIR"; env | grep -E 'KEY|TOKEN|ACCESS' || true; cat '{}' 2>/dev/null && echo HOST-VISIBLE; ls '{}' 2>/dev/null && echo KEPT-VISIBLE; echo "MODE=$(stat -c %a "$PI_CODING_AGENT_DIR/auth.json")"; (echo x > "$PI_CODING_AGENT_DIR/auth.json") 2>/dev/null && echo WRITABLE; rm -f "$PI_CODING_AGENT_DIR/auth.json" 2>/dev/null && echo REMOVABLE; true"#,
        host_file.display(),
        kept_dir(&a).display()
    );
    replies.push(queue(script.clone(), 1).await.expect("runs on the lent login").to_string());
    let ran = shell_run(&runs, 1).await;
    let (json_part, rest) = ran.out.split_once("\nDIR=").expect("its auth.json and folder");
    let lent: Value = serde_json::from_str(json_part.trim()).unwrap_or_else(|e| panic!("{e}: {}", ran.out));
    assert_eq!(lent.as_object().unwrap().keys().collect::<Vec<_>>(), ["anthropic"], "{lent}");
    assert_eq!(lent["anthropic"], json!({ "type": "oauth", "refresh": "", "access": HOST_ACCESS, "expires": lent["anthropic"]["expires"] }));
    assert!(!ran.out.contains(HOST_REFRESH) && !ran.out.contains(CODEX_ACCESS) && !ran.out.contains("sk-or-HOST-KEY"), "{}", ran.out);
    assert!(!ran.out.contains("HOST-VISIBLE") && !ran.out.contains("API_KEY="), "{}", ran.out);
    // Read-only to the sandbox, and where it is kept out of its sight.
    assert!(ran.out.contains("\nMODE=600\n"), "{}", ran.out);
    for seen in ["WRITABLE", "REMOVABLE", "KEPT-VISIBLE"] {
        assert!(!ran.out.contains(seen), "{seen}: {}", ran.out);
    }
    let state = ran.state.clone().expect("a state folder");
    let agent_dir = state.join(zend_engine::peer_logins::AGENT_DIR);
    assert_eq!(rest.lines().next().unwrap(), agent_dir.display().to_string(), "{}", ran.out);
    assert_eq!(std::fs::read(&host_file).unwrap(), host_before, "the host's auth.json is unchanged");
    // (MEDIUM) Its run over, the token is on no disk but the host's file.
    until(|| holding_token(&a, &host_file).is_empty(), "the lent login to go with its run").await;
    let lent = || !kept_logins(&a).is_empty();

    // (1) A key kept for B and anthropic wins: the token is taken out
    // before that turn, which sees only the key.
    a.link.keys().set(&a.b_key, "anthropic", B_KEY).unwrap();
    let keyed = format!("ls '{}' 2>&1; cat '{}' 2>&1; env | grep -E 'KEY|TOKEN|PI_CODING' || true", agent_dir.display(), agent_dir.join("auth.json").display());
    replies.push(queue(keyed, 2).await.expect("runs on B's key").to_string());
    let ran = shell_run(&runs, 2).await;
    assert!(!ran.out.contains(HOST_ACCESS) && ran.out.contains(&format!("ANTHROPIC_API_KEY={B_KEY}")), "{}", ran.out);
    assert!(!lent());
    a.link.keys().remove(&a.b_key, "anthropic").unwrap();

    // (3) Taken back mid-turn: the run ends, the file leaves its sandbox,
    // the next is refused.
    queue("# hold\ntrue".into(), 3).await.expect("held on the login");
    shell_run(&runs, 3).await;
    until(|| a.core.sessions.has_run("theirs") && lent(), "a held turn on the login").await;
    let removed = owner.call(methods::REMOVE_PEER_LOGIN, json!({ "peer": a.b_key, "provider": "anthropic" })).await.unwrap();
    assert_eq!(removed["removed"], true);
    assert!(!lent(), "the login is taken back as the answer comes, before the run ends");
    replies.push(removed.to_string());
    until(|| !a.core.sessions.has_run("theirs"), "taking the login back to end the run").await;
    assert!(holding_token(&a, &host_file).is_empty());
    let again = queue("true".into(), 4).await;
    assert!(again.is_err(), "{again:?}");
    replies.push(format!("{again:?}"));

    // (3) Unpaired mid-turn: the same.
    owner.call(methods::ALLOW_PEER_LOGIN, json!({ "peer": a.b_key, "provider": "anthropic" })).await.unwrap();
    queue("# hold\ntrue".into(), 5).await.expect("held on the login again");
    shell_run(&runs, 4).await;
    until(|| a.core.sessions.has_run("theirs") && lent(), "a second held turn").await;
    replies.push(owner.call(methods::UNPAIR_PEER, json!({ "peer": a.b_key })).await.unwrap().to_string());
    assert!(!lent(), "unpairing takes the login back as it answers");
    until(|| !a.core.sessions.has_run("theirs"), "unpairing to end the run").await;
    assert!(holding_token(&a, &host_file).is_empty());
    assert_eq!(owner.call(methods::LIST_PEER_LOGINS, json!({})).await.unwrap()["logins"], json!([]));
    assert_eq!(runs.lock().unwrap().len(), 4);

    // (4) The token nowhere but the host's own file: not in Zend's data
    // (journals, chats, logs), the project, the copy, a reply or a log line.
    assert_eq!(std::fs::read(&host_file).unwrap(), host_before, "the host's auth.json is unchanged");
    let holding: Vec<PathBuf> = files_holding(&a.root, HOST_ACCESS).into_iter().filter(|f| !f.starts_with(host_file.parent().unwrap())).collect();
    assert!(holding.is_empty(), "the token is kept in {holding:?}");
    assert!(files_holding(&copy, HOST_ACCESS).is_empty());
    for reply in &replies {
        assert!(!reply.contains(HOST_ACCESS) && !reply.contains(HOST_REFRESH), "{reply}");
    }
    if capturing {
        let logs = String::from_utf8_lossy(&logged.0.lock().unwrap()).into_owned();
        assert!(logs.contains("runs on this device's login"), "the log captured nothing of it");
        assert!(logs.contains(&words), "this device's log says how to allow it");
        assert!(!logs.contains(HOST_ACCESS) && !logs.contains(HOST_REFRESH), "the token is in a log line");
    }
}

/// A's chat for B, its model on anthropic, B allowed A's anthropic login
/// (`minutes` left on it; `pi` the host's pi that refreshes it): A, its
/// runs, and the host's login file.
async fn on_login(minutes: i64, pi: Option<PathBuf>) -> (Paired, Arc<Mutex<Vec<ShellRun>>>, PathBuf) {
    let runs = Arc::new(Mutex::new(Vec::new()));
    let registry = HarnessRegistry::new();
    registry.register(Arc::new(ShellAgent { runs: runs.clone() }));
    let a = paired_with(&["sessions.run:p"], registry).await;
    a.core.sessions.set_no_ipc_port();
    let (host_file, login) = host_pi(&a.root, minutes);
    a.core.sessions.set_host_login(zend_engine::peer_logins::HostLogin { pi, ..login });
    a.b.call(methods::MUTATE, json!({ "op": "createChat", "chatId": "theirs", "spaceId": "p", "config": keyed_config() })).await.expect("create");
    owner(&a).call(methods::ALLOW_PEER_LOGIN, json!({ "peer": a.b_key, "provider": "anthropic" })).await.unwrap();
    (a, runs, host_file)
}

/// B queues `prompt` (a shell script) in its chat.
async fn queue_theirs(a: &Paired, prompt: &str, n: usize) -> Result<Value, RpcError> {
    let mut queued = run("theirs", "/");
    queued["command"]["request"]["prompt"] = prompt.into();
    queued["command"]["messageId"] = format!("m-theirs-{n}").into();
    a.b.call(methods::QUEUE_COMMAND, queued).await
}

/// 4d-4 review, HIGH (delete): a server's sandbox owns its state folder
/// and puts a link there, `pi-agent` to this device's pi folder (from
/// inside, in a turn on a key: `rm -rf pi-agent; ln -s <host> pi-agent`).
/// The next turn on a key and taking the login back used to remove
/// `pi-agent/auth.json` by that path: the owner's own login file. Now the
/// host acts by no path in the state folder: its file stays.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_sandbox_linking_its_agent_folder_to_the_hosts_cant_have_the_hosts_login_deleted() {
    if !sandbox_tools_present() {
        return;
    }
    let (a, runs, host_file) = on_login(300, None).await;
    let host_before = std::fs::read(&host_file).unwrap();
    queue_theirs(&a, "true", 1).await.expect("runs on the lent login");
    let state = shell_run(&runs, 1).await.state.expect("a state folder");
    let unchanged = |when: &str| assert_eq!(std::fs::read(&host_file).ok().as_ref(), Some(&host_before), "{when}: the host's login file");

    a.link.keys().set(&a.b_key, "anthropic", B_KEY).unwrap();
    let attack = format!("cd '{}' && rm -rf pi-agent && ln -s '{}' pi-agent && echo LINKED", state.display(), host_file.parent().unwrap().display());
    queue_theirs(&a, &attack, 2).await.expect("runs on B's key");
    let ran = shell_run(&runs, 2).await;
    assert!(ran.out.contains("LINKED"), "{}", ran.out);
    assert!(std::fs::symlink_metadata(state.join("pi-agent")).unwrap().file_type().is_symlink());
    unchanged("the attack itself");

    queue_theirs(&a, "true", 3).await.expect("runs on B's key again");
    shell_run(&runs, 3).await;
    unchanged("a turn on a key after it");

    a.link.keys().remove(&a.b_key, "anthropic").unwrap();
    let back = owner(&a).call(methods::REMOVE_PEER_LOGIN, json!({ "peer": a.b_key, "provider": "anthropic" })).await.unwrap();
    assert_eq!(back["removed"], true);
    tokio::time::sleep(Duration::from_millis(300)).await;
    unchanged("taking the login back");
    owner(&a).call(methods::ALLOW_PEER_LOGIN, json!({ "peer": a.b_key, "provider": "anthropic" })).await.unwrap();
    owner(&a).call(methods::UNPAIR_PEER, json!({ "peer": a.b_key })).await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    unchanged("unpairing");
    assert!(holding_token(&a, &host_file).is_empty());
}

/// 4d-4 review, HIGH (overwrite): the same link, put there while the host's
/// pi refreshes the login (a race with the chat's sandbox, here made sure:
/// the stand-in pi puts the link as it refreshes). Writing the lent file by
/// that path used to replace the owner's login file with the lent one (no
/// refresh token). Now the lent file is written only in Zend's data and
/// mounted by bubblewrap, which won't follow the link out: the owner's file
/// is what pi made of it.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_sandbox_linking_its_agent_folder_to_the_hosts_cant_have_the_hosts_login_overwritten() {
    use std::os::unix::fs::PermissionsExt as _;
    if !sandbox_tools_present() {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let target = temp.path().join("state-to-link");
    let refreshed = temp.path().join("refreshed.json");
    let far = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64 + 600 * 60_000;
    let refreshed_login = json!({ "anthropic": { "type": "oauth", "access": HOST_ACCESS, "refresh": HOST_REFRESH, "expires": far } });
    std::fs::write(&refreshed, serde_json::to_vec(&refreshed_login).unwrap()).unwrap();
    let pi = temp.path().join("pi");
    std::fs::write(
        &pi,
        format!(
            r#"#!/bin/sh
if [ -f '{t}' ]; then st=$(cat '{t}'); rm -rf "$st/pi-agent"; ln -s "$PI_CODING_AGENT_DIR" "$st/pi-agent"; fi
cp '{r}' "$PI_CODING_AGENT_DIR/auth.json"
"#,
            t = target.display(),
            r = refreshed.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&pi, std::fs::Permissions::from_mode(0o755)).unwrap();
    let (a, runs, host_file) = on_login(5, Some(pi)).await;
    queue_theirs(&a, "true", 1).await.expect("runs on the refreshed login");
    let state = shell_run(&runs, 1).await.state.expect("a state folder");

    // Near its end again: the next turn has pi refresh it, and the link is
    // put as it does.
    host_pi(&a.root, 5);
    std::fs::write(&target, state.display().to_string()).unwrap();
    let raced = queue_theirs(&a, "true", 2).await;
    until(|| std::fs::symlink_metadata(state.join("pi-agent")).is_ok_and(|m| m.file_type().is_symlink()), "the stand-in pi to put the link").await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let now: Value = serde_json::from_slice(&std::fs::read(&host_file).unwrap()).unwrap();
    assert_eq!(now, refreshed_login, "the host's login file was replaced (turn: {raced:?})");
    until(|| kept_logins(&a).is_empty(), "the lent login to go").await;
    assert!(holding_token(&a, &host_file).is_empty(), "the token is kept outside the host's file");
}

/// 4d-4 review, MEDIUM: a deleted chat's lent login goes with it (its run
/// ends), and none is left on disk.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_deleted_chats_lent_login_goes_with_it() {
    if !sandbox_tools_present() {
        return;
    }
    let (a, runs, host_file) = on_login(300, None).await;
    queue_theirs(&a, "# hold\ntrue", 1).await.expect("held on the login");
    shell_run(&runs, 1).await;
    until(|| a.core.sessions.has_run("theirs") && !holding_token(&a, &host_file).is_empty(), "a held turn on the login").await;
    owner(&a).call(methods::MUTATE, json!({ "op": "deleteChat", "chatId": "theirs" })).await.expect("deleted");
    until(|| !a.core.sessions.has_run("theirs"), "deleting the chat to end its run").await;
    until(|| holding_token(&a, &host_file).is_empty(), "the lent login to go with the chat").await;
}

/// 4d-4 review, MEDIUM: a login kept for a run when the engine stopped (a
/// crash) is gone once it starts again.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_login_kept_for_a_run_is_swept_at_start() {
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().join("data");
    drop(assemble(&data, "device-a", HarnessRegistry::new()));
    let kept = kept_dir_of(&data);
    std::fs::create_dir_all(&kept).unwrap();
    let left = kept.join("theirs.0123abcd.json");
    std::fs::write(&left, format!("{{\"anthropic\":{{\"access\":\"{HOST_ACCESS}\"}}}}")).unwrap();
    let core = assemble(&data, "device-a", HarnessRegistry::new());
    assert!(!left.exists(), "a login left from before the start is still kept");
    drop(core);
}

/// An agent that records each prompt it gets and completes the turn, except
/// one that starts with "block": that streams and then holds until it's
/// stopped (an event's prompt may quote one, and completes).
/// Steerable, its mailbox kept open while it holds.
struct Holder {
    prompts: Arc<Mutex<Vec<String>>>,
}

#[async_trait::async_trait]
impl zend_harness::Harness for Holder {
    fn id(&self) -> HarnessId {
        HarnessId::Mock
    }
    fn display_name(&self) -> &str {
        "Holder"
    }
    fn supports_steering(&self) -> bool {
        true
    }
    fn steering_mode(&self) -> zend_proto::SteeringMode {
        zend_proto::SteeringMode::TurnBoundary
    }
    fn reasoning_levels(&self) -> &[zend_proto::ReasoningLevel] {
        &[]
    }
    async fn models(&self) -> Result<Vec<zend_proto::Model>, zend_harness::HarnessError> {
        Ok(vec![])
    }
    async fn run(
        &self,
        request: RunRequest,
        controls: zend_harness::RunControls,
    ) -> Result<futures::stream::BoxStream<'static, Result<zend_proto::AgentEvent, zend_harness::HarnessError>>, zend_harness::HarnessError>
    {
        use futures::StreamExt as _;
        use zend_proto::AgentEvent;
        let n = {
            let mut prompts = self.prompts.lock().unwrap();
            prompts.push(request.prompt.clone());
            prompts.len()
        };
        let hold = request.prompt.starts_with("block");
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        tokio::spawn(async move {
            let _ = tx.send(Ok(AgentEvent::SessionStarted {
                harness: HarnessId::Mock,
                model: "mock-1".into(),
                tools: vec![],
                cwd: request.cwd.clone(),
                session_id: format!("hs-{n}"),
                assistant_message_id: format!("a-{n}"),
            }));
            let _ = tx.send(Ok(AgentEvent::TextDelta { text: "working…".into() }));
            if hold {
                let _steering = controls.steering;
                controls.interrupt.cancelled().await;
                return;
            }
            let _ = tx.send(Ok(AgentEvent::Done {
                status: zend_proto::DoneStatus::Completed,
                result: None,
                error: None,
                session_id: Some(format!("hs-{n}")),
            }));
        });
        Ok(futures::stream::unfold(rx, |mut rx| async move { rx.recv().await.map(|ev| (ev, rx)) }).boxed())
    }
}

async fn until(mut done: impl FnMut() -> bool, what: &str) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    while !done() {
        assert!(tokio::time::Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn working(core: &EngineCore, chat: &str) -> bool {
    core.sessions.has_run(chat)
        && core.sessions.session_status(chat).is_some_and(|s| s.status == zend_proto::SessionStatus::Working)
}

fn holding() -> (HarnessRegistry, Arc<Mutex<Vec<String>>>) {
    let prompts = Arc::new(Mutex::new(Vec::new()));
    let registry = HarnessRegistry::new();
    registry.register(Arc::new(Holder { prompts: prompts.clone() }));
    (registry, prompts)
}

fn prompted(prompts: &Arc<Mutex<Vec<String>>>, needle: &str) -> usize {
    prompts.lock().unwrap().iter().filter(|p| p.contains(needle)).count()
}

/// B's chat (unconfined, then confined) and its own run's prompt.
async fn queue(a: &Paired, chat: &str, prompt: &str) {
    let mut queued = run(chat, &a.root.join("p").to_string_lossy());
    queued["command"]["request"]["prompt"] = prompt.into();
    queued["command"]["messageId"] = format!("m-{chat}-{prompt}").replace(' ', "-").into();
    a.b.call(methods::QUEUE_COMMAND, queued).await.expect("queued");
}

/// Unpairing a server ends the live runs of its chats here, and what would
/// start a turn in one afterwards doesn't: a child's event to its parent
/// (checked against the same child stopped by a Stop while still paired,
/// which does start one).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unpairing_ends_a_servers_runs_and_no_child_event_starts_one() {
    let (registry, prompts) = holding();
    let a = paired_with(&["sessions.run.unconfined:p"], registry).await;
    a.core.sessions.set_no_ipc_port();
    let config = json!({ "harness": HarnessId::Mock, "sandbox": "workspace-write" });
    a.b.call(methods::MUTATE, json!({ "op": "createChat", "chatId": "parent", "spaceId": "p", "config": config })).await.expect("parent");
    queue(&a, "parent", "hello parent").await;
    until(|| prompted(&prompts, "hello parent") == 1 && !working(&a.core, "parent"), "the parent's turn").await;
    let rpc = a.core.rpc_service();
    with_caller(Caller::Agent { chat: Some("parent".into()) }, rpc.handle(methods::MUTATE, json!({
        "op": "createChat", "chatId": "kid", "spaceId": "p", "deviceId": a.core.device_id,
        "parentChatId": "parent", "childKind": "subagent", "config": config,
    }))).await.expect("the parent's agent makes a child");

    // Control: paired, the child stopped: its parent hears in a turn.
    queue(&a, "kid", "block one").await;
    until(|| working(&a.core, "kid"), "the child's held turn").await;
    a.core.sessions.interrupt("kid").await.unwrap();
    until(|| prompted(&prompts, "[Zend event: ") == 1, "the parent's event turn (control)").await;
    until(|| !working(&a.core, "parent"), "the event turn to end").await;

    // Unpaired mid-turn: the run ends, and its parent hears in no turn.
    queue(&a, "kid", "block two").await;
    until(|| working(&a.core, "kid"), "the child's second held turn").await;
    a.link.unpair(&a.b_key).unwrap();
    until(|| !a.core.sessions.has_run("kid"), "unpairing to end the child's run").await;
    tokio::time::sleep(Duration::from_secs(5)).await;
    assert_eq!(prompted(&prompts, "[Zend event: "), 1, "{:#?}", prompts.lock().unwrap());
    assert!(!a.core.sessions.has_run("parent"));
    let started = a.core.sessions.dispatch("parent", HarnessId::Mock, serde_json::from_value(run("parent", &a.root.join("p").to_string_lossy())["command"]["request"].clone()).unwrap(), None).await;
    assert!(started.as_ref().is_err_and(|e| e.to_string().contains("may no longer run")), "{started:?}");
}

/// A grant taken down to sessions.run ends the live run of a chat started
/// unconfined; a steer into the live run of a server's chat is refused the
/// moment its grant is gone, before that run is ended.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_lost_grant_ends_live_runs_and_refuses_steers() {
    let (registry, _prompts) = holding();
    let a = paired_with(&["sessions.run.unconfined:p"], registry).await;
    a.core.sessions.set_no_ipc_port();
    let config = json!({ "harness": HarnessId::Mock, "sandbox": "workspace-write" });
    for chat in ["free", "steered"] {
        a.b.call(methods::MUTATE, json!({ "op": "createChat", "chatId": chat, "spaceId": "p", "config": config })).await.expect("create");
        queue(&a, chat, "block").await;
        until(|| working(&a.core, chat), "a held turn").await;
    }
    // Steered while granted (control), then with peers.json gone and no
    // one told yet: refused.
    let steer = a.core.sessions.steer("steered", "still there?", None).await;
    assert!(matches!(steer, Ok(zend_engine::SteerOutcome::Accepted)), "{steer:?}");
    let peers = std::fs::read(a.data.join("peers.json")).unwrap();
    std::fs::remove_file(a.data.join("peers.json")).unwrap();
    let steer = a.core.sessions.steer("steered", "and now?", None).await;
    assert!(steer.as_ref().is_err_and(|e| e.to_string().contains("doesn't run")), "{steer:?}");
    std::fs::write(a.data.join("peers.json"), peers).unwrap();
    // Taken down to sessions.run: the unmarked chats' runs end.
    a.link.set_grants(&a.b_key.clone(), &["sessions.run:p".into()]).unwrap();
    until(|| !a.core.sessions.has_run("free") && !a.core.sessions.has_run("steered"), "the downgrade to end both runs").await;
    let request: RunRequest = serde_json::from_value(run("free", &a.root.join("p").to_string_lossy())["command"]["request"].clone()).unwrap();
    let again = a.core.sessions.dispatch("free", HarnessId::Mock, request, None).await;
    assert!(again.as_ref().is_err_and(|e| e.to_string().contains("started unconfined")), "{again:?}");
}

/// A server's chat cut off by a shutdown is picked back up on the next boot
/// while the server is still paired (control), and not once it's unpaired.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unpaired_servers_chat_is_not_picked_back_up_on_boot() {
    async fn revived(unpair: bool) -> usize {
        let (registry, _) = holding();
        let a = paired_with(&["sessions.run.unconfined:p"], registry).await;
        a.core.sessions.set_no_ipc_port();
        a.b.call(methods::MUTATE, json!({ "op": "createChat", "chatId": "cut", "spaceId": "p", "config": { "harness": HarnessId::Mock, "sandbox": "workspace-write" } })).await.expect("create");
        queue(&a, "cut", "block").await;
        until(|| working(&a.core, "cut"), "the held turn").await;
        a.core.shutdown().await;
        if unpair {
            a.link.unpair(&a.b_key).unwrap();
        }
        // The first engine gone (it holds the data folder) before the next.
        let Paired { _temp, data, core, link, b, .. } = a;
        drop((b, link, core));
        let (registry, prompts) = holding();
        let core = assemble(&data, "device-a", registry);
        core.sessions.set_no_ipc_port();
        if !unpair {
            until(|| prompted(&prompts, "Continue where you left off") == 1, "the cut-off turn picked back up").await;
        } else {
            tokio::time::sleep(Duration::from_secs(4)).await;
        }
        let n = prompted(&prompts, "Continue where you left off");
        core.shutdown().await;
        drop(_temp);
        n
    }
    assert_eq!(revived(false).await, 1, "control: paired, picked back up");
    assert_eq!(revived(true).await, 0, "unpaired: not picked back up");
}

/// A copy (fork) of a server's confined chat is still that server's chat,
/// confined to the same folder; a copy of the owner's own chat is neither.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_copy_of_a_servers_chat_stays_its_and_sandboxed_alike() {
    use zend_doc::{MessagePart, MessageRole, MessageStatus, SessionMessageEntry};
    let a = paired(&["sessions.run:p"]).await;
    a.b.call(methods::MUTATE, json!({ "op": "createChat", "chatId": "theirs", "spaceId": "p", "config": { "harness": HarnessId::Mock, "sandbox": "workspace-write" } })).await.expect("create");
    a.core.workspace.create_chat("own", Some("p"), Some(&a.core.device_id), None, None).unwrap();
    for chat in ["theirs", "own"] {
        a.core.doc_host.open(chat).unwrap().doc().push_message(&SessionMessageEntry {
            duration_ms: None,
            id: format!("{chat}-a1"),
            role: MessageRole::Assistant,
            parts: vec![MessagePart::Text { id: format!("{chat}-a1-text"), text: "done".into() }],
            created_at: 1,
            device_id: a.core.device_id.clone(),
            status: Some(MessageStatus::Complete),
            continuation_of: None,
        }).unwrap();
    }
    let owner = zend_rpc::memory_client(a.core.rpc_service());
    for (copy, source) in [("theirs-copy", "theirs"), ("own-copy", "own")] {
        owner.call(methods::FORK_SIDE_CHAT, json!({ "chatId": copy, "sourceChatId": source })).await.expect("fork");
    }
    let peers = a.core.sessions.peer_chats();
    assert_eq!(peers.peer("theirs-copy").as_deref(), Some(a.b_key.as_str()));
    let root = a.core.sessions.chat_sandbox("theirs").and_then(|m| m.root).expect("the source is confined");
    assert_eq!(root, a.data.join("peer-copies/theirs"), "its working copy, never the owner's folder");
    assert_eq!(a.core.sessions.chat_sandbox("theirs-copy").and_then(|m| m.root), Some(root));
    assert_eq!(peers.peer("own-copy"), None);
    assert!(a.core.sessions.chat_sandbox("own-copy").is_none());
}

/// A run a server sends that won't start here is refused to it at once, in
/// words: no key kept for it and its model's provider, an agent that can't
/// run sandboxed. One queued from this side and refused as it starts says
/// why in the chat, under the user's message.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_run_refused_here_says_why() {
    let prompts = Arc::new(Mutex::new(Vec::new()));
    let registry = HarnessRegistry::new();
    registry.register(Arc::new(Holder { prompts: prompts.clone() }));
    let a = paired_with(&["sessions.run:p"], registry).await;
    a.core.sessions.set_no_ipc_port();
    a.b.call(methods::MUTATE, json!({ "op": "createChat", "chatId": "theirs", "spaceId": "p", "config": keyed_config() })).await.expect("create");
    let copy = a.core.sessions.chat_sandbox("theirs").and_then(|m| m.root).expect("confined");

    let no_key = a.b.call(methods::QUEUE_COMMAND, run("theirs", "/")).await;
    let words = "no credential for anthropic on this device";
    assert!(no_key.as_ref().is_err_and(|e| e.to_string().contains(words) && !e.to_string().contains("zend peer")), "{no_key:?}");
    a.link.keys().set(&a.b_key, "anthropic", B_KEY).unwrap();
    let unsandboxed = a.b.call(methods::QUEUE_COMMAND, run("theirs", "/")).await;
    assert!(unsandboxed.as_ref().is_err_and(|e| e.to_string().contains("mock can't run sandboxed here; only pi does")), "{unsandboxed:?}");

    // From this side (queued unchecked): refused as it starts, in the chat.
    let owner = owner(&a);
    let mut queued = run("theirs", "/");
    queued["command"]["request"]["cwd"] = json!(copy.display().to_string());
    owner.call(methods::QUEUE_COMMAND, queued).await.expect("queued");
    let mut watch = owner
        .subscribe_checked(methods::WATCH_DOC_MESSAGES, json!({ "chatId": "theirs" }))
        .await
        .expect("its transcript");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let frame = tokio::time::timeout_at(deadline, watch.recv())
            .await
            .expect("the refusal in the chat in time")
            .expect("watch alive");
        let said = frame.to_string();
        if said.contains("mock can't run sandboxed here") {
            // A whole transcript, or the note placed after the user's message.
            assert!(said.contains("hello") || said.contains(r#""after":"m-theirs""#), "under the user's message: {frame}");
            break;
        }
    }
    assert!(prompts.lock().unwrap().is_empty(), "it never ran");
}


/// The chat ops a server takes of a chat it may run, under sessions.run:p:
/// its own chat's config (this side's sandbox, access and plan kept; model
/// options its model doesn't list refused), title, seen, archived, and
/// deleting it all go; of the owner's chat in p, none does, and nothing of
/// it changes. A chat of its own whose child it may not reach isn't deleted.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_server_changes_its_own_chats_only_and_keeps_this_sides_pins() {
    let a = paired(&["sessions.run:p"]).await;
    a.core.workspace.create_chat("owner-chat", Some("p"), Some(&a.core.device_id), None, None).unwrap();
    let mock = json!({ "harness": HarnessId::Mock, "sandbox": "workspace-write" });
    a.b.call(methods::MUTATE, json!({ "op": "createChat", "chatId": "mine", "spaceId": "p", "config": mock })).await.expect("create");
    let mutate = |op: &str, chat: &str| {
        let mut p = json!({ "op": op, "chatId": chat });
        match op {
            "setChatConfig" => p["config"] = json!({ "harness": HarnessId::Mock, "sandbox": "danger-full-access", "access": "full-access", "plan": true }),
            "setChatArchived" => p["archived"] = json!(true),
            "renameChat" => p["title"] = json!("Named by the server"),
            _ => {}
        }
        p
    };
    let ops = ["setChatConfig", "renameChat", "markChatSeen", "setChatArchived", "deleteChat"];

    // The owner's chat: none of them.
    let before = a.core.workspace.chat("owner-chat").unwrap().unwrap();
    for op in ops {
        let reply = a.b.call(methods::MUTATE, mutate(op, "owner-chat")).await;
        assert!(reply.as_ref().is_err_and(|e| e.to_string().contains("not one your server may reach")), "{op} of the owner's chat: {reply:?}");
    }
    let after = a.core.workspace.chat("owner-chat").unwrap().expect("the owner's chat stays");
    assert_eq!((after.title, after.archived, after.config, after.last_seen_at), (before.title, before.archived, before.config, before.last_seen_at));

    // Its own: each goes, the config under this side's pins.
    for op in &ops[..4] {
        a.b.call(methods::MUTATE, mutate(op, "mine")).await.unwrap_or_else(|e| panic!("{op} of its own chat: {e}"));
    }
    let mine = a.core.workspace.chat("mine").unwrap().unwrap();
    let config = mine.config.clone().unwrap();
    assert_eq!((config.harness, config.sandbox, config.access, config.plan), (HarnessId::Mock, SandboxLevel::WorkspaceWrite, zend_proto::AccessMode::Supervised, false));
    assert_eq!((mine.title.as_deref(), mine.archived, mine.last_seen_at.is_some()), (Some("Named by the server"), true, true));
    let odd = json!({ "op": "setChatConfig", "chatId": "mine", "config": { "harness": HarnessId::Mock, "sandbox": "workspace-write", "modelOptions": { "bogus": "x" } } });
    let reply = a.b.call(methods::MUTATE, odd).await;
    assert!(reply.as_ref().is_err_and(|e| e.to_string().contains("setChatConfig: modelOptions")), "{reply:?}");
    assert_eq!(a.core.workspace.chat("mine").unwrap().unwrap().config, Some(config));
    // Any other op, of its own chat too: refused.
    let cwd = a.b.call(methods::MUTATE, json!({ "op": "setChatCwd", "chatId": "mine", "cwd": "/" })).await;
    assert!(cwd.is_err(), "{cwd:?}");

    // A child of its chat that isn't its own (the owner's, in q): not
    // deleted with it, so its chat isn't either.
    a.core.workspace.create_chat_with_parent("owners-child", Some("q"), Some(&a.core.device_id), None, None, Some("mine".into())).unwrap();
    let reply = a.b.call(methods::MUTATE, mutate("deleteChat", "mine")).await;
    assert!(reply.as_ref().is_err_and(|e| e.to_string().contains("owners-child")), "{reply:?}");
    assert!(a.core.workspace.chat("mine").unwrap().is_some() && a.core.workspace.chat("owners-child").unwrap().is_some());
    a.core.workspace.delete_chat("owners-child").unwrap();
    a.b.call(methods::MUTATE, mutate("deleteChat", "mine")).await.expect("its own chat deleted");
    assert!(a.core.workspace.chat("mine").unwrap().is_none());
}

/// sessions.run.unconfined:p: any chat in p, the owner's too, takes the
/// chat ops, its config as sent (unconfined); a chat in q doesn't.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unconfined_grant_changes_any_chat_of_its_project() {
    let a = paired(&["sessions.run.unconfined:p"]).await;
    a.core.workspace.create_chat("owner-p", Some("p"), Some(&a.core.device_id), None, None).unwrap();
    a.core.workspace.create_chat("owner-q", Some("q"), Some(&a.core.device_id), None, None).unwrap();
    let config = json!({ "harness": HarnessId::Mock, "sandbox": "danger-full-access", "access": "full-access" });
    for chat in ["owner-p", "owner-q"] {
        let set = a.b.call(methods::MUTATE, json!({ "op": "setChatConfig", "chatId": chat, "config": config })).await;
        let archive = a.b.call(methods::MUTATE, json!({ "op": "setChatArchived", "chatId": chat, "archived": true })).await;
        assert_eq!((set.is_ok(), archive.is_ok()), (chat == "owner-p", chat == "owner-p"), "{chat}: {set:?} {archive:?}");
    }
    let p = a.core.workspace.chat("owner-p").unwrap().unwrap();
    assert_eq!((p.archived, p.config.map(|c| c.sandbox)), (true, Some(SandboxLevel::DangerFullAccess)));
    let q = a.core.workspace.chat("owner-q").unwrap().unwrap();
    assert_eq!((q.archived, q.config), (false, None));
    a.b.call(methods::MUTATE, json!({ "op": "deleteChat", "chatId": "owner-p" })).await.expect("deleted");
    assert!(a.core.workspace.chat("owner-p").unwrap().is_none());
}


// ---- plan step 6: the area grants -------------------------------------------

/// Refused by a grant check (the transport's or the engine's), not by
/// the call itself.
fn gated(reply: &Result<Value, RpcError>) -> bool {
    reply.as_ref().is_err_and(|e| {
        let e = e.to_string();
        e.contains("isn't open to your server") || e.contains("never open to another server") || e.contains("grants your server nothing")
    })
}

/// `params` of `method` as B sends them: a stream's first answer, a value.
async fn b_calls(a: &Paired, method: &str, params: Value) -> Result<Value, RpcError> {
    if method == methods::UPDATE_STATUS {
        return a.b.subscribe_checked(method, params).await.map(|_| Value::Null);
    }
    a.b.call(method, params).await
}

/// B's grants set to `grants`, and B dialled in again.
async fn regrant(a: &mut Paired, grants: &[&str]) {
    a.link.set_grants(&a.b_key, &grants.iter().map(|g| g.to_string()).collect::<Vec<_>>()).expect("grants");
    a.redial().await;
}

/// Every method constant (crates/rpc's `methods` module).
fn every_method() -> Vec<&'static str> {
    let src = include_str!("../../../rpc/src/lib.rs");
    let start = src.find("pub mod methods {").unwrap();
    let src = &src[start..start + src[start..].find("\n}\n").unwrap()];
    src.lines()
        .filter_map(|l| l.trim().strip_prefix("pub const ")?.split_once(": &str = \""))
        .map(|(_, rest)| rest.trim_end_matches("\";"))
        .collect()
}

/// Each area grant, through the real listener and engine: a call of its
/// area goes through with it alone, and is refused with every other grant
/// (`full` aside). With `files` a file outside every project is read; a
/// chat of another device is no server's, whatever it holds.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn each_area_grant_opens_its_area_and_nothing_without_it() {
    let logged = Logged::default();
    let capturing = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_ansi(false)
        .with_writer({
            let logged = logged.clone();
            move || logged.clone()
        })
        .try_init()
        .is_ok();
    let mut a = paired(&[]).await;
    a.core.workspace.create_chat("owner-chat", Some("p"), Some(&a.core.device_id), None, None).unwrap();
    a.core.workspace.create_chat("far-chat", None, Some("device-elsewhere"), None, None).unwrap();
    let outside = a.root.join("outside.txt").to_string_lossy().into_owned();
    let areas = ["projects.manage", "files", "settings", "logins", "updates", "terminal"];
    let calls: Vec<(&str, &str, Value)> = vec![
        ("projects.manage", methods::LIST_DRIVES, json!({})),
        ("projects.manage", methods::MUTATE, json!({ "op": "renameSpace", "spaceId": "p", "name": "Renamed by B" })),
        ("files", methods::READ_WORKSPACE_FILE, json!({ "chatId": "owner-chat", "path": outside })),
        ("settings", methods::GET_TITLE_SETTINGS, json!({})),
        ("logins", methods::LIST_AGENT_ACCOUNTS, json!({})),
        ("updates", methods::UPDATE_STATUS, json!({})),
        ("terminal", methods::TERMINAL_STATUS, json!({ "terminalId": "no-such-terminal" })),
        ("full", methods::LOCAL_DEVICE, json!({})),
    ];
    for (grant, method, params) in &calls {
        regrant(&mut a, &[grant]).await;
        let with = b_calls(&a, method, params.clone()).await;
        assert!(!gated(&with), "{grant}: {method} {params}: {with:?}");
        let mut others: Vec<&str> = areas.iter().copied().filter(|g| g != grant).collect();
        others.extend(["sessions.read", "sessions.run.unconfined:p", "projects.share:p", "workers.manage"]);
        regrant(&mut a, &others).await;
        let without = b_calls(&a, method, params.clone()).await;
        assert!(gated(&without), "without {grant}: {method} {params}: {without:?}");
    }
    // What the calls did with their grant.
    regrant(&mut a, &["files"]).await;
    let read = a.b.call(methods::READ_WORKSPACE_FILE, json!({ "chatId": "owner-chat", "path": outside })).await.expect("read with files");
    assert!(read.to_string().contains("OUTSIDE-SECRET"), "{read}");
    // On the record at INFO: the call and the server, never what it read.
    // (Only where this test's subscriber is the process's: run alone.)
    if capturing {
        let log = String::from_utf8_lossy(&logged.0.lock().unwrap()).into_owned();
        let line = log.lines().find(|l| l.contains("a paired server's call") && l.contains("ReadWorkspaceFile")).unwrap_or_else(|| panic!("no INFO line: {log}"));
        assert!(line.contains(" INFO ") && line.contains("scope=\"files\"") && line.contains(&a.b_key[..12]), "{line}");
        assert!(!log.contains("OUTSIDE-SECRET"), "the log holds what was read");
    } else {
        eprintln!("each_area_grant: another test's log subscriber is the process's; the INFO line isn't checked here");
    }
    assert_eq!(a.core.workspace.space("p").unwrap().unwrap().name.as_deref(), Some("Renamed by B"));
    // Another device's chat: not this server's to reach, with `files` or `full`.
    for grants in [&["files"][..], &["full"][..]] {
        regrant(&mut a, grants).await;
        let far = a.b.call(methods::READ_WORKSPACE_FILE, json!({ "chatId": "far-chat", "path": "x" })).await;
        assert!(far.as_ref().is_err_and(|e| e.to_string().contains("isn't one of this device's")), "{grants:?}: {far:?}");
    }
    // `full` holds every area.
    for (grant, method, params) in &calls {
        let reply = b_calls(&a, method, params.clone()).await;
        assert!(!gated(&reply), "full ({grant}): {method}: {reply:?}");
    }
    assert_eq!(a.b.call(methods::LOCAL_DEVICE, json!({})).await.unwrap()["deviceId"], json!("device-a"));
}

/// Every method the table keeps `never` (pairing, grants, the account,
/// Cloud, billing, vault, GitHub, the phone link, StopEngine, …) is
/// refused with every grant there is, `full` among them; the engine runs on.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn never_is_refused_with_every_grant() {
    let a = paired(&[
        "sessions.read", "sessions.run.unconfined:p", "projects.share:p", "workers.manage",
        "projects.manage", "files", "settings", "logins", "updates", "terminal", "full",
    ])
    .await;
    let never: Vec<&str> = every_method().into_iter().filter(|m| zend_rpc::peer_scope(m, None) == Some(zend_rpc::Scope::Never)).collect();
    assert!(never.contains(&methods::STOP_ENGINE) && never.contains(&methods::SET_PEER_GRANTS) && never.len() >= 50, "{never:?}");
    for method in &never {
        let reply = a.b.call(method, json!({ "peer": a.b_key, "grants": ["full"] })).await;
        assert!(reply.as_ref().is_err_and(|e| e.to_string().contains("never open")), "{method}: {reply:?}");
    }
    let remove = a.b.call(methods::MUTATE, json!({ "op": "removeDevice", "deviceId": "device-a" })).await;
    assert!(remove.as_ref().is_err_and(|e| e.to_string().contains("never open")), "{remove:?}");
    a.b.call(methods::ENGINE_INFO, json!({})).await.expect("still running, still paired");
    assert_eq!(a.link.peers().iter().find(|p| p.key == a.b_key).map(|p| p.grants.iter().count()), Some(11));
}

/// An agent in a chat B started gets nothing of B's `full`, `files`,
/// `terminal` or `settings`: the same calls B itself makes are refused
/// for it, and it reads no other chat (`full` holds sessions.read). What
/// it could do before still works (its own child in p).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_agent_in_a_peer_chat_gets_nothing_of_full() {
    let a = paired(&["sessions.run:p", "full", "files", "terminal", "settings"]).await;
    a.core.workspace.create_chat("owner-chat", Some("p"), Some(&a.core.device_id), None, None).unwrap();
    a.b.call(methods::MUTATE, create("peer-chat", "p")).await.expect("create in p");
    let rpc = a.core.rpc_service();
    let agent = || Caller::Agent { chat: Some("peer-chat".to_string()) };
    let outside = a.root.join("outside.txt").to_string_lossy().into_owned();
    for (method, params) in [
        (methods::LOCAL_DEVICE, json!({})),
        (methods::GET_TITLE_SETTINGS, json!({})),
        (methods::TERMINAL_STATUS, json!({ "terminalId": "no-such-terminal" })),
        (methods::READ_WORKSPACE_FILE, json!({ "chatId": "owner-chat", "path": outside })),
        (methods::WATCH_DOC_MESSAGES, json!({ "chatId": "owner-chat" })),
        (methods::MUTATE, json!({ "op": "renameSpace", "spaceId": "p", "name": "x" })),
    ] {
        // B itself may (the control).
        if method != methods::WATCH_DOC_MESSAGES {
            let b = b_calls(&a, method, params.clone()).await;
            assert!(!gated(&b), "B: {method}: {b:?}");
        }
        let reply = with_caller(agent(), rpc.handle(method, params.clone())).await;
        assert!(reply.is_err(), "the agent reached {method} {params}");
    }
    let config = json!({ "harness": HarnessId::Mock, "sandbox": "danger-full-access", "access": "full-access" });
    with_caller(agent(), rpc.handle(methods::MUTATE, json!({
        "op": "createChat", "chatId": "p-child", "spaceId": "p", "deviceId": a.core.device_id,
        "parentChatId": "peer-chat", "childKind": "subagent", "config": config,
    }))).await.expect("its own child in p, as before");
}


/// A project's setup action is a command: a server with `projects.manage`
/// alone neither sets one (`terminal`'s) nor has the owner's run in the
/// worktree it makes: the worktree is made, its setup not run, and it's
/// told so. With `terminal` the setup runs (its terminal closed after).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_servers_worktree_runs_its_setup_only_with_terminal() {
    let mut a = paired(&["projects.manage"]).await;
    let repo = a.root.join("p");
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git").args(args).current_dir(&repo).output().expect("git");
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    };
    git(&["init", "-q", "-b", "main"]);
    git(&["add", "."]);
    git(&["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "-m", "first"]);
    let marker = a.root.join("setup-ran");
    a.core
        .project_actions
        .upsert(
            "p",
            &repo,
            None,
            zend_proto::ProjectActionDraft {
                name: "Setup".into(),
                command: format!("touch '{}'", marker.display()),
                icon: zend_proto::ProjectActionIcon::Configure,
                run_on_worktree_create: true,
            },
        )
        .expect("the owner's setup action");
    let set = a.b.call(methods::UPSERT_PROJECT_ACTION, json!({ "spaceId": "p", "action": { "name": "x", "command": "id", "icon": "test", "runOnWorktreeCreate": true } })).await;
    assert!(gated(&set), "an action set with projects.manage: {set:?}");
    let repo_path = repo.to_string_lossy().into_owned();
    let made = a.b.call(methods::CREATE_WORKTREE, json!({ "repoPath": repo_path, "branch": "main", "spaceId": "p" })).await.expect("its worktree");
    assert!(made["setupAction"].is_null(), "{made}");
    assert!(made["setupError"].as_str().is_some_and(|e| e.contains("isn't granted terminal")), "{made}");
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert!(!marker.exists(), "the setup ran for a server without terminal");

    regrant(&mut a, &["projects.manage", "terminal"]).await;
    let made = a.b.call(methods::CREATE_WORKTREE, json!({ "repoPath": repo_path, "branch": "main", "spaceId": "p" })).await.expect("its worktree");
    let terminal = made["setupAction"]["terminal"]["id"].as_str().unwrap_or_else(|| panic!("no setup terminal: {made}")).to_string();
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    while !marker.exists() {
        assert!(std::time::Instant::now() < deadline, "the setup never ran with terminal");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let _ = a.core.terminals.close(&terminal);
    assert!(!a.core.terminals.any_open(), "a shell left open");
}

/// Zend's own data (the peers and their grants, keys) is never a
/// server's by path, with `files` or `full`: not read, not written, not
/// listed, not searched, even through a project that holds it. Other
/// files of that project are.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn zends_own_folders_are_never_a_servers_by_path() {
    let a = paired(&["full"]).await;
    a.core.workspace.create_chat("owner-chat", Some("p"), Some(&a.core.device_id), None, None).unwrap();
    let root = a.root.to_string_lossy().into_owned();
    a.b.call(methods::MUTATE, json!({ "op": "createSpace", "spaceId": "wide", "deviceId": a.core.device_id, "path": root }))
        .await
        .expect("a project holding Zend's data");
    let peers = a.data.join("peers.json");
    let before = std::fs::read_to_string(&peers).unwrap();
    let owner = zend_rpc::memory_client(a.core.rpc_service());
    let read = owner.call(methods::READ_WORKSPACE_FILE, json!({ "spaceId": "wide", "path": "a-data/peers.json" })).await.expect("the owner reads it");
    let fenced = |reply: &Result<Value, RpcError>| reply.as_ref().is_err_and(|e| e.to_string().contains("Zend's own data"));
    let write = a.b
        .call(
            methods::WRITE_WORKSPACE_FILE,
            json!({
                "spaceId": "wide", "path": "a-data/peers.json", "text": "{\"peers\":[]}",
                "expectedCheckoutId": read["checkoutId"], "expectedContentHash": read["contentHash"],
                "encoding": "utf8", "lineEnding": "lf",
            }),
        )
        .await;
    assert!(fenced(&write), "written: {write:?}");
    assert_eq!(std::fs::read_to_string(&peers).unwrap(), before);
    let calls = [
        (methods::READ_WORKSPACE_FILE, json!({ "spaceId": "wide", "path": "a-data/peers.json" })),
        (methods::READ_WORKSPACE_FILE, json!({ "chatId": "owner-chat", "path": peers.to_string_lossy() })),
        (methods::LIST_WORKSPACE_DIRECTORY, json!({ "spaceId": "wide", "directory": "a-data" })),
        (methods::SEARCH_WORKSPACE_CONTENT, json!({ "spaceId": "wide", "query": "grants" })),
    ];
    for (method, params) in calls {
        let reply = a.b.call(method, params.clone()).await;
        assert!(fenced(&reply), "{method} {params}: {reply:?}");
    }
    let other = a.b.call(methods::READ_WORKSPACE_FILE, json!({ "spaceId": "wide", "path": "outside.txt" })).await.expect("another file of it");
    assert!(other.to_string().contains("OUTSIDE-SECRET"), "{other}");
}

/// Every id a server's call names, by any key and at any depth
/// (`parentChatId`, `requesterDeviceId`, …), is of this device; the
/// sidebar's pins are of chats hosted here; a chat isn't moved to
/// another device (`setChatHost`: never).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_servers_ids_are_this_devices_by_any_key() {
    let a = paired(&["full"]).await;
    a.core.workspace.create_chat("owner-chat", Some("p"), Some(&a.core.device_id), None, None).unwrap();
    a.core.workspace.create_chat("far-chat", None, Some("device-elsewhere"), None, None).unwrap();
    let refusals = [
        (methods::GET_TITLE_SETTINGS, json!({ "parentChatId": "far-chat" }), "chat far-chat isn't one of this device's"),
        (methods::GET_TITLE_SETTINGS, json!({ "nested": { "sourceChatId": "far-chat" } }), "chat far-chat isn't one of this device's"),
        (methods::START_AGENT_LOGIN, json!({ "harness": "codex", "requesterDeviceId": "device-elsewhere" }), "device device-elsewhere isn't this device"),
        (methods::MUTATE, json!({ "op": "changeSidebarPin", "change": { "action": "pin", "sessionId": "far-chat", "after": null, "before": null } }), "chat far-chat isn't this device's"),
        (methods::MUTATE, json!({ "op": "changeSidebarPin", "change": { "action": "pin", "sessionId": "owner-chat", "after": "far-chat", "before": null } }), "chat far-chat isn't this device's"),
        (methods::MUTATE, json!({ "op": "setChatHost", "chatId": "owner-chat", "deviceId": "device-elsewhere" }), "never open to another server"),
    ];
    for (method, params, said) in refusals {
        let reply = a.b.call(method, params.clone()).await;
        assert!(reply.as_ref().is_err_and(|e| e.to_string().contains(said)), "{method} {params}: {reply:?}");
    }
    a.b.call(methods::GET_TITLE_SETTINGS, json!({ "parentChatId": "owner-chat" })).await.expect("its own chat named");
    a.b.call(methods::MUTATE, json!({ "op": "changeSidebarPin", "change": { "action": "pin", "sessionId": "owner-chat", "after": null, "before": null } }))
        .await
        .expect("a chat hosted here pinned");
    assert_eq!(a.core.workspace.chat("owner-chat").unwrap().unwrap().device_id, a.core.device_id);
}
