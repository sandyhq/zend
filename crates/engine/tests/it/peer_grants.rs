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

    let created = a.b.call(methods::MUTATE, create("peer-chat", "p")).await.expect("create in p");
    assert_eq!(created["sandbox"], "project", "{created}");
    assert_eq!(a.core.workspace.chat("peer-chat").unwrap().unwrap().space_id.as_deref(), Some("p"));
    let mark = a.core.sessions.chat_sandbox("peer-chat").expect("marked");
    assert_eq!((mark.sandbox, mark.root), (zend_proto::ChatSandbox::Project, Some(a.root.join("p"))));

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
    assert_eq!(a.core.sessions.chat_sandbox("p-child").and_then(|m| m.root), Some(a.root.join("p")));
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
}

/// One run of [`ShellAgent`]: its folder, its sandbox's root (None:
/// unconfined) and what the shell printed.
#[derive(Clone, Debug)]
struct ShellRun {
    cwd: String,
    root: Option<PathBuf>,
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
        self.runs.lock().unwrap().push(ShellRun { cwd: request.cwd.clone(), root, out });
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

/// End to end: B, granted sessions.run:p (and q unconfined), creates a chat
/// in p over the peer link and queues a run; its agent starts under
/// bubblewrap with p's folder as root and can read and write p only, not q,
/// Zend's data or the rest of the home. A child its agent starts is
/// confined to p too: not in q (though B may run q unconfined), its run in
/// bubblewrap at p, and no run of it outside p.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_peer_chat_runs_in_bubblewrap_in_its_project_and_so_do_its_children() {
    if !sandbox_tools_present() {
        return;
    }
    let runs = Arc::new(Mutex::new(Vec::new()));
    let registry = HarnessRegistry::new();
    registry.register(Arc::new(ShellAgent { runs: runs.clone() }));
    let a = paired_with(&["sessions.run:p", "sessions.run.unconfined:q"], registry).await;
    a.core.sessions.set_no_ipc_port();
    let p = a.root.join("p");
    let q = a.root.join("q");
    let script = format!(
        r#"for f in {q}/readme.md {data}/ipc-token {base}/outside.txt; do
            if cat "$f" >/dev/null 2>&1; then echo "READABLE $f"; fi
        done
        (echo x > {q}/written.txt) 2>/dev/null && echo "WROTE Q"
        cat ./readme.md
        echo made > ./made-in-p.txt && echo "wrote p"
        echo "in $(pwd)""#,
        q = q.display(),
        data = a.data.display(),
        base = a.root.display(),
    );
    let config = json!({ "harness": HarnessId::Mock, "sandbox": "danger-full-access", "access": "full-access" });
    let created = a
        .b
        .call(methods::MUTATE, json!({ "op": "createChat", "chatId": "peer-chat", "spaceId": "p", "config": config }))
        .await
        .expect("create in p");
    assert_eq!(created["sandbox"], "project");
    assert_eq!(a.core.sessions.chat_sandbox("peer-chat").and_then(|m| m.root), Some(p.clone()));

    let mut queued = run("peer-chat", "/");
    queued["command"]["request"]["prompt"] = script.clone().into();
    a.b.call(methods::QUEUE_COMMAND, queued).await.expect("queued");
    let first = shell_run(&runs, 1).await;
    println!("peer chat: {first:?}");
    assert_eq!((Path::new(&first.cwd), first.root.as_ref()), (p.as_path(), Some(&p)), "{first:?}");
    assert!(first.out.contains("project p") && first.out.contains("wrote p"), "{}", first.out);
    assert!(first.out.contains(&format!("in {}", p.display())), "{}", first.out);
    assert!(!first.out.contains("READABLE") && !first.out.contains("WROTE Q"), "{}", first.out);
    assert_eq!(std::fs::read_to_string(p.join("made-in-p.txt")).unwrap(), "made\n");
    assert!(!q.join("written.txt").exists());

    // Its agent's children.
    let rpc = a.core.rpc_service();
    let agent = Caller::Agent { chat: Some("peer-chat".into()) };
    let child = |id: &str, space: &str| {
        json!({
            "op": "createChat", "chatId": id, "spaceId": space, "deviceId": a.core.device_id,
            "parentChatId": "peer-chat", "childKind": "subagent", "config": config,
        })
    };
    let in_q = with_caller(agent.clone(), rpc.handle(methods::MUTATE, child("q-child", "q"))).await;
    assert!(in_q.as_ref().is_err_and(|e| e.to_string().contains("stays in its project")), "{:?}", in_q.map(|_| ()));
    assert!(a.core.workspace.chat("q-child").unwrap().is_none());
    with_caller(agent, rpc.handle(methods::MUTATE, child("p-child", "p"))).await.expect("a child in p");
    assert_eq!(a.core.sessions.chat_sandbox("p-child").and_then(|m| m.root), Some(p.clone()));
    let mut queued = run("p-child", "/");
    queued["command"]["request"]["prompt"] = script.into();
    a.b.call(methods::QUEUE_COMMAND, queued).await.expect("the child's run queued");
    let second = shell_run(&runs, 2).await;
    println!("child: {second:?}");
    assert_eq!(second.root.as_ref(), Some(&p), "{second:?}");
    assert!(second.out.contains("project p") && !second.out.contains("READABLE") && !second.out.contains("WROTE Q"), "{}", second.out);
    // A run of the child in q doesn't start.
    let mut request: RunRequest = serde_json::from_value(run("p-child", "/")["command"]["request"].clone()).unwrap();
    request.cwd = q.display().to_string();
    let outside = a.core.sessions.start_idle("p-child", request).await;
    assert!(outside.as_ref().is_err_and(|e| e.to_string().contains("outside its project")), "{outside:?}");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(runs.lock().unwrap().len(), 2, "{:?}", runs.lock().unwrap());
}
