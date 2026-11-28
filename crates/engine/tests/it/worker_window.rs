//! Worker sessions from the server (step 5, follow-up): each of this
//! server's workers shows in its own window as a device with one project,
//! and a chat made there runs in the worker's project folder, over its
//! socket. In process: the worker is an engine of its own serving its
//! socket as a worker does (`zend_engine::peer_link::serve_worker`,
//! configured as `zend_engine::worker::Conf` installs it), its server's
//! `workers.json` written as `zend worker create` writes it.
//!
//! A worker is as trusted as the agent in it: what it says is display
//! only (`Liar`), and its rows show to this device's own window alone,
//! never to a paired server or the phone link.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::StreamExt as _;
use serde_json::{Value, json};
use zend_doc::commands::SessionCommandPayload;
use zend_engine::peer_link::{PeerLink, PeerLinkRpc, Seams};
use zend_engine::{EngineCore, HarnessRegistry};
use zend_proto::{HarnessId, RunRequest, SandboxLevel};
use zend_rpc::peer_link::NodeKey;
use zend_rpc::{RpcClient, RpcError, RpcReply, RpcService, methods};

type Lie = Arc<dyn Fn(&str, Value) -> Value + Send + Sync>;
type Heard = Arc<Mutex<Vec<(String, Value)>>>;

/// A service recording each call made of it, its replies through `lie`.
struct Liar {
    inner: Arc<dyn RpcService>,
    lie: Lie,
    heard: Heard,
}

#[async_trait::async_trait]
impl RpcService for Liar {
    async fn handle(&self, method: &str, params: Value) -> Result<RpcReply, RpcError> {
        self.heard.lock().unwrap().push((method.to_string(), params.clone()));
        match self.inner.handle(method, params).await? {
            RpcReply::Value(v) => Ok(RpcReply::Value((self.lie)(method, v))),
            RpcReply::Stream(items) => {
                let (lie, method) = (self.lie.clone(), method.to_string());
                Ok(RpcReply::Stream(items.map(move |v| lie(&method, v)).boxed()))
            }
        }
    }
}

fn honest() -> Lie {
    Arc::new(|_, v| v)
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

fn assemble(data_dir: &Path, device_id: &str) -> EngineCore {
    std::fs::create_dir_all(data_dir).expect("data dir");
    std::fs::write(data_dir.join("device-id"), device_id).expect("device id");
    EngineCore::assemble(data_dir, Arc::new(HarnessRegistry::new()), HarnessId::Mock, None).expect("engine assembles")
}

/// One in-process worker.
struct Worker {
    core: EngineCore,
    /// The device id its server shows it as (`worker-…`), whatever it says.
    device: String,
    heard: Heard,
    work: PathBuf,
    project: String,
    _served: tokio::task::JoinHandle<()>,
}

/// A server, its window, and its workers.
struct Server {
    _temp: tempfile::TempDir,
    root: PathBuf,
    core: EngineCore,
    link: Arc<PeerLink>,
    window: RpcClient,
    workers: Vec<Worker>,
}

impl Server {
    async fn new() -> Self {
        let home = std::fs::canonicalize(std::env::var_os("HOME").expect("HOME")).unwrap();
        // (In HOME, short: a socket's path is at most 107 bytes.)
        let temp = tempfile::Builder::new().prefix("zww-").tempdir_in(home).unwrap();
        let root = std::fs::canonicalize(temp.path()).unwrap();
        let core = assemble(&root.join("s"), "device-server");
        let own = root.join("s-projects").join("sp");
        std::fs::create_dir_all(&own).unwrap();
        core.workspace.create_space("sp", &core.device_id, &own.to_string_lossy(), None, false).unwrap();
        core.workspace.create_chat("own-chat", Some("sp"), Some(&core.device_id), None, None).unwrap();
        let link = PeerLink::new(&root.join("s"), 0, core.rpc_service(), loopback());
        link.node_key().unwrap();
        let window = zend_rpc::memory_client(Arc::new(PeerLinkRpc { inner: core.rpc_service(), link: link.clone() }));
        Self { _temp: temp, root, core, link, window, workers: Vec::new() }
    }

    /// Worker `name`, its engine calling itself `device`, saying `lie`;
    /// kept in `workers.json`.
    fn add_worker(&mut self, name: &str, device: &str, lie: Lie) {
        let der = NodeKey::generate_pkcs8().unwrap();
        let key = NodeKey::from_pkcs8(&der).unwrap();
        let project = zend_engine::workers::project_of(key.public());
        let conf = self.root.join(format!("{name}-conf"));
        let work = self.root.join(format!("{name}-work"));
        std::fs::create_dir_all(&conf).unwrap();
        std::fs::write(conf.join(zend_engine::worker::KEY_FILE), &der).unwrap();
        std::fs::write(conf.join(zend_engine::worker::SERVER_KEY_FILE), self.link.node_key().unwrap().public()).unwrap();
        std::fs::write(conf.join(zend_engine::worker::BLOCKED_FILE), "10.0.0.0/8\n").unwrap();
        std::fs::write(conf.join(zend_engine::worker::PROJECT_FILE), &project).unwrap();
        std::fs::write(conf.join("work"), work.to_string_lossy().as_bytes()).unwrap();
        let loaded = zend_engine::worker::Conf::load(&conf).expect("its conf");
        let data = self.root.join(format!("{name}-data"));
        loaded.install(&data).expect("installed");
        // As a worker runs: its harnesses, the mock among them.
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(data.join("device-id"), device).unwrap();
        let core = EngineCore::assemble(&data, zend_engine::registry::default_registry().into(), HarnessId::Mock, None).expect("the worker assembles");
        core.workspace.create_space(&loaded.project, &core.device_id, &work.to_string_lossy(), Some("work".into()), false).unwrap();
        // A project of its own besides (none of its server's to see).
        let other = self.root.join(format!("{name}-other"));
        std::fs::create_dir_all(&other).unwrap();
        core.workspace.create_space("elsewhere", &core.device_id, &other.to_string_lossy(), None, false).unwrap();
        let heard: Heard = Arc::default();
        let served = Arc::new(Liar { inner: core.rpc_service(), lie, heard: heard.clone() });
        let socket = zend_engine::workers::Workers::new(&self.root.join("s")).socket(name);
        let served = zend_engine::peer_link::serve_worker(served, &data, &socket).expect("it listens");
        let saved = self.root.join("s").join(zend_engine::workers::FILE);
        let mut all: Value = std::fs::read(&saved).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or(json!({ "workers": [] }));
        all["workers"].as_array_mut().unwrap().push(json!({
            "name": name, "key": key.public(), "image": "localhost/zend-worker:latest", "container": format!("zend-worker-{name}"),
            "providers": [], "createdAt": 1_700_000_000_000i64 + self.workers.len() as i64,
            "limits": { "memory": "4g", "cpus": "2", "pids": 512 }, "project": project,
        }));
        std::fs::write(&saved, serde_json::to_vec(&all).unwrap()).unwrap();
        self.link.view().refresh();
        self.workers.push(Worker { core, device: zend_engine::workers::device_of(key.public()), heard, work, project, _served: served });
    }
}

/// A list watch's rows, frames applied (whole lists, or change frames).
struct Watched {
    sub: zend_rpc::RpcSubscription,
    rows: Vec<Value>,
    key: &'static str,
}

impl Watched {
    async fn open(window: &RpcClient, method: &str, key: &'static str) -> Self {
        let sub = window.subscribe_checked(method, json!({ "changes": true })).await.unwrap_or_else(|e| panic!("{method}: {e}"));
        Self { sub, rows: Vec::new(), key }
    }

    async fn until(&mut self, what: &str, pred: impl Fn(&[Value]) -> bool) -> Vec<Value> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        while !pred(&self.rows) {
            let frame = tokio::time::timeout_at(deadline, self.sub.recv())
                .await
                .unwrap_or_else(|_| panic!("{what}: not in 30 s; rows {:?}", self.rows))
                .expect("watch alive");
            self.apply(frame);
        }
        self.rows.clone()
    }

    async fn settle(&mut self, secs: u64) -> Vec<Value> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(secs);
        while let Ok(Some(frame)) = tokio::time::timeout_at(deadline, self.sub.recv()).await {
            self.apply(frame);
        }
        self.rows.clone()
    }

    fn apply(&mut self, frame: Value) {
        match frame {
            Value::Array(rows) => self.rows = rows,
            frame => {
                let key = self.key;
                for row in frame["changed"].as_array().into_iter().flatten() {
                    match self.rows.iter_mut().find(|r| r[key] == row[key]) {
                        Some(r) => *r = row.clone(),
                        None => self.rows.push(row.clone()),
                    }
                }
                let removed: Vec<&Value> = frame["removed"].as_array().into_iter().flatten().collect();
                self.rows.retain(|r| !removed.contains(&&r[key]));
            }
        }
    }
}

fn has(rows: &[Value], id: &str) -> bool {
    rows.iter().any(|r| r["id"].as_str() == Some(id))
}

fn count(rows: &[Value], id: &str) -> usize {
    rows.iter().filter(|r| r["id"].as_str() == Some(id)).count()
}

fn row<'a>(rows: &'a [Value], id: &str) -> &'a Value {
    rows.iter().find(|r| r["id"].as_str() == Some(id)).unwrap_or_else(|| panic!("no {id} in {rows:?}"))
}

fn run(chat: &str, prompt: &str) -> Value {
    let request = RunRequest {
        mcp: None,
        prompt: prompt.into(),
        harness: Some(HarnessId::Mock),
        model: None,
        reasoning: None,
        model_options: Default::default(),
        cwd: "/home/server/private".into(),
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
        fork_from: None,
    };
    json!({ "chatId": chat, "command": SessionCommandPayload::Run { request, message_id: format!("m-{chat}") } })
}

fn mock_config() -> Value {
    json!({ "harness": "mock", "model": null, "reasoning": null, "modelOptions": {}, "sandbox": "danger-full-access", "access": "full-access" })
}

/// Make chat `chat` on worker device `device` from `window`, as the window does.
async fn create(window: &RpcClient, chat: &str, device: &str) -> Result<Value, RpcError> {
    window
        .call(
            methods::MUTATE,
            json!({ "op": "createChat", "chatId": chat, "deviceId": device, "cwd": "/home/server/private", "branch": "", "config": mock_config() }),
        )
        .await
}

/// Until `path` exists (30 s at most).
async fn file_appears(path: &Path) -> String {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        if let Ok(text) = std::fs::read_to_string(path) {
            return text;
        }
        assert!(tokio::time::Instant::now() < deadline, "{} not written in 30 s", path.display());
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// A transcript, read through `window`, until it says `words`.
async fn transcript_says(window: &RpcClient, chat: &str, words: &str) -> String {
    let mut sub = window.subscribe_checked(methods::WATCH_DOC_MESSAGES, json!({ "chatId": chat })).await.expect("its transcript");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    let mut all = String::new();
    while !all.contains(words) {
        let frame = tokio::time::timeout_at(deadline, sub.recv())
            .await
            .unwrap_or_else(|_| panic!("transcript of {chat}: no {words:?} in 30 s: {all}"))
            .expect("transcript alive");
        all.push_str(&frame.to_string());
    }
    all
}

/// A worker shows in its server's window as a device (a worker, named as
/// it), with one project, `work`, of no folder; a chat made there from the
/// window (by its device, with the server's folder) is made in that project
/// on the worker, runs in the worker's folder (the mock's `/pwd` writes
/// there), and its reply reads back through the window. Calls the worker
/// takes nothing of are refused here (control: its harnesses go).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_worker_shows_as_a_device_and_a_chat_runs_in_its_folder() {
    let mut s = Server::new().await;
    s.add_worker("w1", "device-w1", honest());
    let w1 = s.workers[0].device.clone();
    let (project, work) = (s.workers[0].project.clone(), s.workers[0].work.clone());
    assert!(project.starts_with("work-") && project.len() == 17, "{project}");

    let mut devices = Watched::open(&s.window, methods::WATCH_DEVICES, "id").await;
    let rows = devices.until("the worker's device", |rows| has(rows, w1.as_str())).await;
    // Its device id is its server's word, not what it says it is.
    assert!(w1.starts_with("worker-") && w1.len() == 19, "{w1}");
    assert!(!has(&rows, "device-w1"), "shown as what it says: {rows:?}");
    let device = row(&rows, w1.as_str());
    assert_eq!(device["name"], "w1", "{device}");
    assert_eq!(device["capabilities"], json!(["paired-server", "worker"]), "{device}");
    let mut spaces = Watched::open(&s.window, methods::WATCH_SPACES, "id").await;
    let rows = spaces.until("its project", |rows| has(rows, &project)).await;
    let p = row(&rows, &project);
    assert_eq!((p["deviceId"].as_str(), p["name"].as_str(), p["path"].as_str()), (Some(w1.as_str()), Some("work"), Some("")));
    assert!(!has(&rows, "elsewhere"), "a project it has besides shows: {rows:?}");

    let made = s
        .window
        .call(
            methods::MUTATE,
            json!({ "op": "createChat", "chatId": "w-chat", "deviceId": w1.as_str(), "cwd": "/home/server/private", "branch": "feature-x",
                    "title": "t", "parentChatId": "own-chat", "worktree": true, "sourceContext": { "path": "/home/server" }, "config": mock_config() }),
        )
        .await
        .expect("made on the worker");
    let sent: Vec<Value> = s.workers[0].heard.lock().unwrap().iter().filter(|(m, _)| m == methods::MUTATE).map(|(_, p)| p.clone()).collect();
    assert_eq!(sent, vec![json!({ "op": "createChat", "chatId": "w-chat", "spaceId": project, "config": mock_config() })], "a createChat sent more");
    assert_eq!(made["deviceId"], w1.as_str(), "{made}");
    let chat = s.workers[0].core.workspace.chat("w-chat").unwrap().expect("the chat is the worker's");
    assert_eq!(chat.space_id.as_deref(), Some(project.as_str()));
    assert!(s.core.workspace.chat("w-chat").unwrap().is_none(), "made here too");
    s.window.call(methods::QUEUE_COMMAND, run("w-chat", "/pwd")).await.expect("its run queued there");
    assert_eq!(file_appears(&work.join(".zend-mock-ran")).await, "the mock ran here\n");
    let said = transcript_says(&s.window, "w-chat", "wrote .zend-mock-ran").await;
    assert!(said.contains(&format!("cwd {}", work.display())), "{said}");
    let mut chats = Watched::open(&s.window, methods::WATCH_CHATS, "id").await;
    let rows = chats.until("its chat", |rows| has(rows, "w-chat")).await;
    let c = row(&rows, "w-chat");
    assert_eq!(c["deviceId"], w1.as_str());
    // Its folder as a field: none (what its agent says is its own words).
    assert!(c.get("cwd").is_none_or(|cwd| cwd == "."), "its folder shows: {c}");
    let queued: Vec<String> = s.workers[0].heard.lock().unwrap().iter().filter(|(m, _)| m == methods::QUEUE_COMMAND).map(|(_, p)| p.to_string()).collect();
    assert!(queued.len() == 1 && !queued[0].contains("/home/server"), "{queued:?}");

    // Not a worker's to take: said no to here, unheard there.
    for (method, params) in [
        (methods::PEER_LIST_PROJECT_FILES, json!({ "targetDeviceId": w1.as_str(), "projectId": project })),
        (methods::ENGINE_INFO, json!({ "targetDeviceId": w1.as_str() })),
        (methods::MUTATE, json!({ "op": "archiveChat", "chatId": "w-chat" })),
        (methods::SET_PEER_KEY, json!({ "targetDeviceId": w1.as_str(), "peer": "x", "provider": "anthropic", "key": "sk" })),
    ] {
        let before = s.workers[0].heard.lock().unwrap().len();
        let refused = s.window.call(method, params.clone()).await;
        // (A key is this device's own: refused before any routing.)
        let routed = method != methods::SET_PEER_KEY;
        assert!(refused.as_ref().is_err_and(|e| !routed || e.to_string().contains("isn't sent to a worker")), "{method} {params}: {refused:?}");
        let heard: Vec<String> = s.workers[0].heard.lock().unwrap()[before..].iter().map(|(m, _)| m.clone()).collect();
        assert!(!heard.iter().any(|m| m == method), "the worker heard {method}");
    }
    let harnesses = s.window.call(methods::LIST_HARNESSES, json!({ "targetDeviceId": w1.as_str() })).await;
    assert!(harnesses.is_ok(), "control: its harnesses: {harnesses:?}");
    assert!(s.workers[0].heard.lock().unwrap().iter().any(|(m, _)| m == methods::LIST_HARNESSES));
}

/// A paired server (here a laptop that dials this server) and the phone
/// link see none of a worker's: no device, chat or project, and their calls
/// naming them reach no worker. Control: this server's own window shows it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_paired_server_and_the_phone_see_no_worker() {
    let mut s = Server::new().await;
    s.add_worker("w1", "device-w1", honest());
    let w1 = s.workers[0].device.clone();
    let project = s.workers[0].project.clone();
    let mut chats = Watched::open(&s.window, methods::WATCH_CHATS, "id").await;
    let mut devices = Watched::open(&s.window, methods::WATCH_DEVICES, "id").await;
    devices.until("control: the window shows the worker", |rows| has(rows, w1.as_str())).await;
    create(&s.window, "w-chat", w1.as_str()).await.expect("made on the worker");
    chats.until("control: the window shows the worker's chat", |rows| has(rows, "w-chat")).await;

    // A laptop paired to this server, granted all it reads and runs.
    let laptop = assemble(&s.root.join("l"), "device-laptop");
    let laptop_link = PeerLink::new(&s.root.join("l"), 0, laptop.rpc_service(), loopback());
    let code = s.link.pair().await.expect("pair");
    laptop_link.join(code["code"].as_str().unwrap()).await.expect("join");
    let grants = vec!["sessions.read".to_string(), "sessions.run.unconfined:sp".to_string(), format!("sessions.run.unconfined:{project}")];
    s.link.set_grants(laptop_link.node_key().unwrap().public(), &grants).expect("grants");
    let laptop_window = zend_rpc::memory_client(Arc::new(PeerLinkRpc { inner: laptop.rpc_service(), link: laptop_link.clone() }));
    let mut l_chats = Watched::open(&laptop_window, methods::WATCH_CHATS, "id").await;
    l_chats.until("the laptop: this server's chat", |rows| has(rows, "own-chat")).await;
    let rows = l_chats.settle(2).await;
    assert!(!has(&rows, "w-chat"), "the laptop sees the worker's chat: {rows:?}");
    let mut l_devices = Watched::open(&laptop_window, methods::WATCH_DEVICES, "id").await;
    let rows = l_devices.until("the laptop: this server", |rows| has(rows, "device-server")).await;
    assert!(!has(&rows, w1.as_str()), "the laptop sees the worker: {rows:?}");
    let mut l_spaces = Watched::open(&laptop_window, methods::WATCH_SPACES, "id").await;
    let rows = l_spaces.settle(2).await;
    assert!(has(&rows, "sp") && !has(&rows, &project), "{rows:?}");
    // Calls straight to this server, as the paired server it is.
    let as_peer = laptop_link.connect(s.link.node_key().unwrap().public()).await.expect("the laptop dials this server");
    let before = s.workers[0].heard.lock().unwrap().len();
    let _ = as_peer.call(methods::QUEUE_COMMAND, run("w-chat", "/pwd")).await;
    let _ = as_peer.call(methods::MUTATE, json!({ "op": "createChat", "chatId": "l-made", "spaceId": project, "config": mock_config() })).await;
    let _ = as_peer.call(methods::LIST_HARNESSES, json!({ "targetDeviceId": w1.as_str() })).await;
    let _ = as_peer.subscribe_checked(methods::WATCH_DOC_MESSAGES, json!({ "chatId": "w-chat" })).await;

    // The phone link, served this server's own (wrapped) service.
    let wrapped: Arc<dyn RpcService> = Arc::new(PeerLinkRpc { inner: s.core.rpc_service(), link: s.link.clone() });
    let key = zend_rpc::ipc_auth::generate_token().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let _phone_served = tokio::spawn(zend_rpc::remote::serve_remote_listener_admitting(listener, wrapped, key.clone(), |_| {
        Box::pin(std::future::ready(true))
    }));
    let phone = tokio::time::timeout(Duration::from_secs(5), zend_rpc::connect_phone_link(&format!("ws://{addr}"), &key))
        .await
        .expect("in time")
        .expect("the phone dials in");
    let mut p_chats = Watched::open(&phone, methods::WATCH_CHATS, "id").await;
    let rows = p_chats.until("the phone: this server's chat", |rows| has(rows, "own-chat")).await;
    assert!(!has(&rows, "w-chat"), "the phone sees the worker's chat: {rows:?}");
    if let Ok(sub) = phone.subscribe_checked(methods::WATCH_DEVICES, json!({})).await {
        let mut devices = Watched { sub, rows: Vec::new(), key: "id" };
        let rows = devices.settle(1).await;
        assert!(!has(&rows, w1.as_str()), "the phone sees the worker: {rows:?}");
    }
    let _ = phone.call(methods::LIST_HARNESSES, json!({ "targetDeviceId": w1.as_str() })).await;
    let _ = phone.call(methods::QUEUE_COMMAND, run("w-chat", "/pwd")).await;
    let _ = phone.call(methods::MUTATE, json!({ "op": "createChat", "chatId": "phone-made", "deviceId": w1.as_str(), "spaceId": project })).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let heard: Vec<String> = s.workers[0].heard.lock().unwrap()[before..]
        .iter()
        .filter(|(m, _)| m != methods::ENGINE_INFO && m != methods::WATCH_CHATS && m != methods::WATCH_SESSIONS)
        .map(|(m, p)| format!("{m} {p}"))
        .collect();
    assert!(heard.is_empty(), "the worker heard a paired server's or the phone's calls: {heard:?}");
    for made in ["l-made", "phone-made"] {
        assert!(s.workers[0].core.workspace.chat(made).unwrap().is_none(), "{made} made on the worker");
    }
    assert!(!s.workers[0].work.join(".zend-mock-ran").exists(), "a run reached the worker");
}

/// A worker's device id is its server's (`worker-<12 hex of its key>`),
/// whatever it says it is: this device, a paired server (shown or not, as
/// here before it grants anything) or no plain id. A paired server saying
/// it is a worker doesn't show. Its rows can't take this device's chats (an
/// archived one too) or a device's id, show a chat outside its project or a
/// session of a chat not shown, carry anything but a chat row's few
/// fields, or say they're another device's; a made chat's reply says the
/// worker's device. Control: an honest worker beside them shows.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn what_a_worker_lies_about_is_dropped() {
    let mut s = Server::new().await;
    s.core.workspace.create_chat("archived-chat", Some("sp"), Some(&s.core.device_id), None, None).unwrap();
    s.window.call(methods::MUTATE, json!({ "op": "setChatArchived", "chatId": "archived-chat", "archived": true })).await.expect("archived");
    // A paired server this server dials, as device-p, granting nothing yet:
    // not shown, its id free to take.
    let p = assemble(&s.root.join("p"), "device-p");
    let p_own = s.root.join("p-projects").join("pp");
    std::fs::create_dir_all(&p_own).unwrap();
    p.workspace.create_space("pp", &p.device_id, &p_own.to_string_lossy(), None, false).unwrap();
    p.workspace.create_chat("p-chat", Some("pp"), Some(&p.device_id), None, None).unwrap();
    let p_link = PeerLink::new(&s.root.join("p"), 0, p.rpc_service(), loopback());
    let code = p_link.pair().await.expect("pair");
    s.link.join(code["code"].as_str().unwrap()).await.expect("join");

    let says_device = |id: &'static str| -> Lie {
        Arc::new(move |method, mut v| {
            if method == methods::ENGINE_INFO {
                v["deviceId"] = json!(id);
            }
            v
        })
    };
    s.add_worker("w-paired", "device-wp", says_device("device-p"));
    s.add_worker("w-here", "device-wh", says_device("device-server"));
    s.add_worker("w-odd", "device-wo", says_device("device\nodd"));
    // Rows: this device's chats (an archived one), a device's id, a chat in
    // another project, a session of a chat not shown, fields no chat row
    // has, another device's id and a folder; a made chat's reply naming
    // this device.
    let rows_lie: Lie = Arc::new(|method, v| match v {
        Value::Array(mut rows) if method == methods::WATCH_CHATS => {
            for r in rows.iter_mut() {
                r["deviceId"] = json!("device-server");
                r["cwd"] = json!("src");
                r["roomGen"] = json!(99);
                r["invented"] = json!("x");
                r["harnessSessionCwd"] = json!("/etc");
                r["config"]["hostPath"] = json!("/home/server");
            }
            let project = rows.first().and_then(|r| r["spaceId"].as_str()).unwrap_or_default().to_string();
            for (id, title) in [("own-chat", "stolen"), ("archived-chat", "stolen"), ("device-server", "a device"), ("p-chat", "stolen")] {
                rows.push(json!({ "id": id, "spaceId": project, "title": title, "deviceId": "device-server" }));
            }
            rows.push(json!({ "id": "outside-chat", "spaceId": "elsewhere", "title": "outside" }));
            Value::Array(rows)
        }
        Value::Array(mut rows) if method == methods::WATCH_SESSIONS => {
            rows.push(json!({ "chatId": "outside-chat", "status": "working" }));
            rows.push(json!({ "chatId": "never-a-chat", "status": "working" }));
            Value::Array(rows)
        }
        Value::Object(mut map) if method == methods::MUTATE => {
            map.insert("deviceId".into(), json!("device-server"));
            Value::Object(map)
        }
        v => v,
    });
    s.add_worker("w-rows", "device-wr", rows_lie);
    s.add_worker("w-honest", "device-wk", honest());
    let devices_of: Vec<String> = s.workers.iter().map(|w| w.device.clone()).collect();
    let projects: Vec<String> = s.workers.iter().map(|w| w.project.clone()).collect();

    let mut devices = Watched::open(&s.window, methods::WATCH_DEVICES, "id").await;
    devices.until("control: every worker, as its server's id", |rows| devices_of.iter().all(|d| has(rows, d))).await;
    let rows = devices.settle(3).await;
    assert!(!has(&rows, "device-p"), "a worker shows as the paired server not shown: {rows:?}");
    assert_eq!(count(&rows, "device-server"), 1, "{rows:?}");
    assert!(!row(&rows, "device-server").to_string().contains("worker"), "{rows:?}");
    for (i, d) in devices_of.iter().enumerate() {
        assert_eq!(row(&rows, d)["capabilities"], json!(["paired-server", "worker"]), "worker {i}");
    }
    for said in ["device-wp", "device-wh", "device-wo", "device-wr", "device-wk"] {
        assert!(!has(&rows, said), "shown as what it says, {said}: {rows:?}");
    }
    assert!(!rows.iter().any(|r| r["id"].as_str().is_some_and(|id| id.contains('\n'))), "{rows:?}");
    // What it said of its device takes nothing: a call naming it stays here.
    let before = s.workers[0].heard.lock().unwrap().len();
    let _ = s.window.call(methods::LIST_HARNESSES, json!({ "targetDeviceId": "device-p" })).await;
    let _ = create(&s.window, "squat-made", "device-p").await;
    let heard: Vec<String> = s.workers[0].heard.lock().unwrap()[before..].iter().map(|(m, p)| format!("{m} {p}")).collect();
    assert!(!heard.iter().any(|h| h.starts_with(methods::LIST_HARNESSES) || h.contains("squat-made")), "the worker took the paired server's calls: {heard:?}");
    let mut spaces = Watched::open(&s.window, methods::WATCH_SPACES, "id").await;
    let rows = spaces.until("control: every worker's project", |rows| projects.iter().all(|p| has(rows, p))).await;
    for (i, p) in projects.iter().enumerate() {
        assert_eq!(row(&rows, p)["deviceId"].as_str(), Some(devices_of[i].as_str()), "{rows:?}");
    }

    let made = create(&s.window, "wr-chat", &devices_of[3]).await.expect("made on the lying worker");
    assert_eq!(made["deviceId"], devices_of[3].as_str(), "the reply's device is the worker's word: {made}");
    s.window.call(methods::QUEUE_COMMAND, run("wr-chat", "/pwd")).await.expect("run there");
    let mut chats = Watched::open(&s.window, methods::WATCH_CHATS, "id").await;
    chats.until("its chat", |rows| has(rows, "wr-chat")).await;
    let mut sessions = Watched::open(&s.window, methods::WATCH_SESSIONS, "chatId").await;
    let shown = sessions.until("control: its chat's session", |rows| rows.iter().any(|r| r["chatId"] == "wr-chat")).await;
    let shown = {
        let mut rows = shown;
        rows.extend(sessions.settle(2).await);
        rows
    };
    for not in ["outside-chat", "never-a-chat"] {
        assert!(!shown.iter().any(|r| r["chatId"] == not), "a session of a chat not shown: {not}: {shown:?}");
    }
    let rows = chats.settle(2).await;
    for own in ["own-chat", "archived-chat"] {
        assert_eq!(count(&rows, own), 1, "{rows:?}");
        let r = row(&rows, own);
        assert!(r["title"] != "stolen" && r["deviceId"] == "device-server", "this device's chat taken: {r}");
    }
    for not in ["outside-chat", "device-server"] {
        assert!(!has(&rows, not), "{not} shows: {rows:?}");
    }
    // The paired server shown at last (it grants this side its chats): its
    // chat is its, the worker's row of that id gone.
    p_link.set_grants(s.link.node_key().unwrap().public(), &["sessions.read".to_string()]).expect("grants");
    s.link.view().refresh();
    let rows = chats.until("the paired server's chat, its", |rows| rows.iter().any(|r| r["id"] == "p-chat" && r["deviceId"] == "device-p")).await;
    let _ = rows;
    let rows = chats.settle(2).await;
    assert_eq!(count(&rows, "p-chat"), 1, "{rows:?}");
    assert!(row(&rows, "p-chat")["title"] != "stolen", "{rows:?}");
    let mut devices = Watched::open(&s.window, methods::WATCH_DEVICES, "id").await;
    let shown = devices.until("the paired server, as itself", |rows| has(rows, "device-p")).await;
    assert_eq!(row(&shown, "device-p")["capabilities"], json!(["paired-server"]), "{shown:?}");
    let wr = row(&rows, "wr-chat");
    assert_eq!(wr["deviceId"], devices_of[3].as_str(), "{wr}");
    let allowed = ["id", "title", "archived", "spaceId", "config", "lastMessageAt", "lastMessagePreview", "createdAt", "lastSeenAt", "parentChatId", "childKind", "deviceId"];
    let extra: Vec<&String> = wr.as_object().unwrap().keys().filter(|k| !allowed.contains(&k.as_str())).collect();
    assert!(extra.is_empty(), "fields no worker chat row may carry: {extra:?} in {wr}");
    assert!(wr["config"].get("hostPath").is_none() && wr["config"]["harness"] == "mock", "{wr}");
    // This device's chat stays here: a run of it never goes to the worker.
    let before = s.workers[3].heard.lock().unwrap().len();
    let _ = s.window.call(methods::QUEUE_COMMAND, run("own-chat", "hello")).await;
    let heard: Vec<String> = s.workers[3].heard.lock().unwrap()[before..].iter().map(|(m, _)| m.clone()).collect();
    assert!(!heard.iter().any(|m| m == methods::QUEUE_COMMAND), "this device's chat went to the worker");
}

/// A paired server saying it is a worker (its id `worker-…`) doesn't show
/// and takes nothing of the worker's; the worker shows. Control: a paired
/// server with an id of its own shows beside it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_paired_server_saying_it_is_a_worker_doesnt_show() {
    let mut s = Server::new().await;
    s.add_worker("w1", "device-w1", honest());
    let w1 = s.workers[0].device.clone();
    let mut links = Vec::new();
    for (dir, device) in [("p", w1.clone()), ("q", "device-q".to_string())] {
        let core = assemble(&s.root.join(dir), &device);
        let own = s.root.join(format!("{dir}-projects")).join(format!("{dir}p"));
        std::fs::create_dir_all(&own).unwrap();
        core.workspace.create_space(&format!("{dir}p"), &core.device_id, &own.to_string_lossy(), None, false).unwrap();
        core.workspace.create_chat(&format!("{dir}-chat"), Some(&format!("{dir}p")), Some(&core.device_id), None, None).unwrap();
        let link = PeerLink::new(&s.root.join(dir), 0, core.rpc_service(), loopback());
        let code = link.pair().await.expect("pair");
        s.link.join(code["code"].as_str().unwrap()).await.expect("join");
        link.set_grants(s.link.node_key().unwrap().public(), &["sessions.read".to_string()]).expect("grants");
        links.push((core, link));
    }
    let mut devices = Watched::open(&s.window, methods::WATCH_DEVICES, "id").await;
    devices.until("control: the other paired server and the worker", |rows| has(rows, "device-q") && has(rows, &w1)).await;
    let rows = devices.settle(3).await;
    assert_eq!(count(&rows, &w1), 1, "{rows:?}");
    assert_eq!(row(&rows, &w1)["capabilities"], json!(["paired-server", "worker"]), "a paired server took the worker's id: {rows:?}");
    let mut chats = Watched::open(&s.window, methods::WATCH_CHATS, "id").await;
    let rows = chats.until("control: the other server's chat", |rows| has(rows, "q-chat")).await;
    let rows = {
        let mut rows = rows;
        rows.extend(chats.settle(2).await);
        rows
    };
    assert!(!has(&rows, "p-chat"), "the server saying it is a worker shows: {rows:?}");
}

/// A pi folder of this server's signed in to anthropic (hours left): the
/// login its workers' turns are lent.
fn host_login(dir: &Path) -> zend_engine::peer_logins::HostLogin {
    std::fs::create_dir_all(dir).unwrap();
    let expires = chrono::Utc::now().timestamp_millis() + 5 * 3_600_000;
    let auth = json!({ "anthropic": { "type": "oauth", "access": "HOST-ACCESS", "refresh": "HOST-REFRESH", "expires": expires } });
    std::fs::write(dir.join("auth.json"), auth.to_string()).unwrap();
    zend_engine::peer_logins::HostLogin { agent_dir: dir.to_path_buf(), pi: None }
}

/// (Review 2, 3) A login lent for a worker's turn goes once that turn has
/// settled, by the server's word (its session row), not the worker's; and
/// `zend worker login rm` during a live turn on it ends that turn: an
/// interrupt to the worker over its socket, the file gone at once.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_lent_login_goes_when_its_turn_settles_and_rm_ends_a_live_turn_on_it() {
    use std::os::unix::fs::PermissionsExt as _;
    let mut s = Server::new().await;
    s.add_worker("w1", "device-w1", honest());
    let w1 = s.workers[0].device.clone();
    let data = s.root.join("s");
    let dir = data.join(zend_engine::workers::DIR).join("w1");
    for (folder, mode) in [(dir.clone(), 0o700), (dir.join("conf"), 0o755)] {
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(mode)).unwrap();
    }
    let host = host_login(&s.root.join("host-pi"));
    let workers = zend_engine::workers::Workers::new(&data);
    let worker = workers.get("w1").unwrap();
    let file = zend_engine::worker_logins::login_dir(&data, "w1").join("anthropic.json");
    let mut devices = Watched::open(&s.window, methods::WATCH_DEVICES, "id").await;
    devices.until("the worker's device", |rows| has(rows, w1.as_str())).await;
    let mut sessions = Watched::open(&s.window, methods::WATCH_SESSIONS, "chatId").await;
    let state = |rows: &[Value], chat: &str| rows.iter().find(|r| r["chatId"] == chat).map(|r| r["status"].as_str().unwrap_or("").to_string());
    zend_engine::worker_logins::allow(&data, "w1", "anthropic").unwrap();
    let lend = |chat: &str| {
        let got = zend_engine::worker_logins::prepare_turn(&data, &worker, chat, HarnessId::Pi, Some("anthropic/claude-haiku-4-5"), None, &host);
        assert_eq!(got.unwrap(), zend_engine::worker_logins::Credential::Login("anthropic".into()));
        assert!(file.is_file());
    };

    // A turn that finishes: its login goes after it settles.
    create(&s.window, "settles", &w1).await.expect("made there");
    lend("settles");
    s.window.call(methods::QUEUE_COMMAND, run("settles", "/pwd")).await.expect("runs there");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    while file.exists() {
        assert!(tokio::time::Instant::now() < deadline, "the login stayed 30 s after its turn; sessions {:?}", sessions.rows);
        let _ = sessions.settle(1).await;
    }

    // A live turn: rm ends it.
    create(&s.window, "live", &w1).await.expect("made there");
    lend("live");
    s.window.call(methods::QUEUE_COMMAND, run("live", "/stream 60")).await.expect("runs there");
    sessions.until("the live turn working", |rows| state(rows, "live").as_deref() == Some("working")).await;
    let local = zend_rpc::memory_client(Arc::new(zend_engine::workers::WorkersRpc { inner: s.core.rpc_service(), workers }));
    let reply = local.call(methods::REMOVE_WORKER_LOGIN, json!({ "name": "w1", "provider": "anthropic" })).await.expect("rm");
    assert_eq!((reply["removed"].as_bool(), reply["turnsEnded"].as_u64()), (Some(true), Some(1)), "{reply}");
    assert!(!file.exists(), "the file stayed after rm");
    let heard = s.workers[0].heard.lock().unwrap().clone();
    assert!(
        heard.iter().any(|(m, p)| m == methods::QUEUE_COMMAND && p["chatId"] == "live" && p["command"]["kind"] == "interrupt"),
        "no interrupt reached the worker"
    );
    // Ended well before its 60 s.
    sessions.until("the live turn ended", |rows| state(rows, "live").as_deref().is_some_and(|s| s != "working")).await;
    // Its next turn is refused.
    assert!(zend_engine::worker_logins::prepare_turn(&data, &worker, "live", HarnessId::Pi, Some("anthropic/claude-haiku-4-5"), None, &host).is_err());
}
