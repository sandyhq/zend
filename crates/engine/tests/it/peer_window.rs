//! Plan step 4e: a paired server shows in this engine's own window as a
//! device, with the chats and projects its grants let this side see, and a
//! chat made from the window in one of its projects is made and run there,
//! over the peer link (no edge). Two engines, each its own data folder and
//! peer link, paired over loopback.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use zend_doc::commands::SessionCommandPayload;
use zend_engine::peer_link::{PeerLink, PeerLinkRpc, Seams};
use zend_engine::{EngineCore, HarnessRegistry};
use zend_proto::{HarnessId, RunRequest, SandboxLevel};
use zend_rpc::{RpcClient, methods};

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

/// A server with projects p and q and a chat in p, a laptop that joined it,
/// the grants the server gives the laptop, and the laptop's window.
struct Pair {
    _temp: tempfile::TempDir,
    server: EngineCore,
    server_link: Arc<PeerLink>,
    laptop: EngineCore,
    laptop_link: Arc<PeerLink>,
    window: RpcClient,
}

async fn pair(grants: &[&str]) -> Pair {
    let home = std::fs::canonicalize(std::env::var_os("HOME").expect("HOME")).unwrap();
    let temp = tempfile::Builder::new().prefix("zend-peer-window-").tempdir_in(home).unwrap();
    let root = std::fs::canonicalize(temp.path()).unwrap();
    let server = assemble(&root.join("server-data"), "device-server");
    for project in ["p", "q"] {
        let dir = root.join(project);
        std::fs::create_dir_all(&dir).unwrap();
        server.workspace.create_space(project, &server.device_id, &dir.to_string_lossy(), None, false).unwrap();
    }
    server.workspace.create_chat("srv-chat", Some("p"), Some(&server.device_id), None, None).unwrap();
    let laptop = assemble(&root.join("laptop-data"), "device-laptop");
    laptop.workspace.create_chat("own-chat", None, Some(&laptop.device_id), None, None).unwrap();

    let server_link = PeerLink::new(&root.join("server-data"), 0, server.rpc_service(), loopback());
    let laptop_link = PeerLink::new(&root.join("laptop-data"), 0, laptop.rpc_service(), loopback());
    let code = server_link.pair().await.expect("pair");
    laptop_link.join(code["code"].as_str().unwrap()).await.expect("join");
    let laptop_key = laptop_link.node_key().unwrap();
    server_link
        .set_grants(laptop_key.public(), &grants.iter().map(|g| g.to_string()).collect::<Vec<_>>())
        .expect("grants");
    let window = zend_rpc::memory_client(Arc::new(PeerLinkRpc { inner: laptop.rpc_service(), link: laptop_link.clone() }));
    Pair { _temp: temp, server, server_link, laptop, laptop_link, window }
}

/// A list watch's rows, frames applied (whole lists, or change frames).
struct Watched {
    sub: zend_rpc::RpcSubscription,
    rows: Vec<Value>,
    key: &'static str,
}

impl Watched {
    async fn open(window: &RpcClient, method: &str, params: Value, key: &'static str) -> Self {
        let sub = window.subscribe_checked(method, params).await.unwrap_or_else(|e| panic!("{method}: {e}"));
        Self { sub, rows: Vec::new(), key }
    }

    /// Frames until `pred` holds of the rows (20 s at most).
    async fn until(&mut self, what: &str, pred: impl Fn(&[Value]) -> bool) -> Vec<Value> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
        while !pred(&self.rows) {
            let frame = tokio::time::timeout_at(deadline, self.sub.recv())
                .await
                .unwrap_or_else(|_| panic!("{what}: not in 20 s; rows {:?}", self.rows))
                .expect("watch alive");
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

fn has(rows: &[Value], key: &str, id: &str) -> bool {
    rows.iter().any(|r| r[key].as_str() == Some(id))
}

fn run(chat: &str) -> Value {
    let request = RunRequest {
        mcp: None,
        prompt: "hello from the laptop".into(),
        harness: Some(HarnessId::Mock),
        model: None,
        reasoning: None,
        model_options: Default::default(),
        cwd: "~".into(),
        sandbox: SandboxLevel::WorkspaceWrite,
        auto_approve: false,
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

/// The server shows as a device (change frames, as the window asks), its
/// chat and project with it; a chat made from the window in p, as the
/// window makes one (its folder, device, an empty branch, then its config
/// set again), is made in p on the server and its run is queued there; its
/// transcript reads through the laptop.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_paired_server_shows_as_a_device_and_a_chat_starts_there() {
    let t = pair(&["sessions.read", "sessions.run.unconfined:p"]).await;
    let mut devices = Watched::open(&t.window, methods::WATCH_DEVICES, json!({ "changes": true }), "id").await;
    let rows = devices.until("the server's device", |rows| has(rows, "id", "device-server")).await;
    let device = rows.iter().find(|d| d["id"] == "device-server").unwrap();
    assert_eq!(device["platform"], "linux", "{device}");
    assert!(device["lastSeenAt"].is_string(), "{device}");

    let mut chats = Watched::open(&t.window, methods::WATCH_CHATS, json!({ "changes": true }), "id").await;
    let rows = chats.until("the server's chat", |rows| has(rows, "id", "srv-chat")).await;
    assert!(has(&rows, "id", "own-chat"), "the laptop's own chat stays: {rows:?}");
    let srv = rows.iter().find(|c| c["id"] == "srv-chat").unwrap();
    assert_eq!(srv["deviceId"], "device-server");
    let mut spaces = Watched::open(&t.window, methods::WATCH_SPACES, json!({}), "id").await;
    let rows = spaces.until("project p", |rows| has(rows, "id", "p")).await;
    let p = rows.iter().find(|s| s["id"] == "p").unwrap();
    assert_eq!((p["deviceId"].as_str(), p["name"].as_str(), p["path"].as_str()), (Some("device-server"), Some("p"), Some("")));
    assert!(!has(&rows, "id", "q"), "q is neither granted nor holds a chat it reads: {rows:?}");

    let config = json!({ "harness": "mock", "model": null, "reasoning": null, "modelOptions": {}, "sandbox": "workspace-write" });
    t.window
        .call(methods::MUTATE, json!({
            "op": "createChat", "chatId": "laptop-made", "spaceId": "p", "deviceId": "device-server",
            "cwd": "/home/laptop/somewhere", "branch": "", "config": config,
        }))
        .await
        .expect("made on the server");
    let made = t.server.workspace.chat("laptop-made").unwrap().expect("the server has it");
    assert_eq!((made.space_id.as_deref(), made.device_id.as_str()), (Some("p"), "device-server"));
    assert!(t.laptop.workspace.chat("laptop-made").unwrap().is_none(), "not the laptop's");
    t.window
        .call(methods::MUTATE, json!({ "op": "setChatConfig", "chatId": "laptop-made", "config": config }))
        .await
        .expect("its config, as made");
    t.window.call(methods::QUEUE_COMMAND, run("laptop-made")).await.expect("its run queued there");
    let mut transcript = t
        .window
        .subscribe_checked(methods::WATCH_DOC_MESSAGES, json!({ "chatId": "laptop-made" }))
        .await
        .expect("its transcript through the laptop");
    tokio::time::timeout(Duration::from_secs(10), transcript.recv()).await.expect("in time").expect("alive");
    chats.until("the chat made there", |rows| has(rows, "id", "laptop-made")).await;
    drop(t.server_link);
}

/// Granted only after this side first dialled (and was refused), as
/// pairing goes (join, then grant): the server shows within a scan.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn grants_given_after_the_first_dial_show_soon() {
    let t = pair(&[]).await;
    let mut devices = Watched::open(&t.window, methods::WATCH_DEVICES, json!({}), "id").await;
    devices.until("the laptop's own list", |rows| !rows.is_empty()).await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(!has(&devices.rows, "id", "device-server"), "nothing granted, nothing shown: {:?}", devices.rows);
    let laptop_key = t.laptop_link.node_key().unwrap().public().to_string();
    t.server_link.set_grants(&laptop_key, &["sessions.read".into()]).unwrap();
    devices.until("the server, granted since", |rows| has(rows, "id", "device-server")).await;
    let mut chats = Watched::open(&t.window, methods::WATCH_CHATS, json!({}), "id").await;
    chats.until("its chat", |rows| has(rows, "id", "srv-chat")).await;
}

/// Without sessions.read the server's chats don't show (its device does,
/// and q, the project it may run in, with no chat of its).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn without_sessions_read_a_servers_chats_dont_show() {
    let t = pair(&["sessions.run:q"]).await;
    let mut spaces = Watched::open(&t.window, methods::WATCH_SPACES, json!({}), "id").await;
    let rows = spaces.until("project q", |rows| has(rows, "id", "q")).await;
    assert!(!has(&rows, "id", "p"), "{rows:?}");
    let mut devices = Watched::open(&t.window, methods::WATCH_DEVICES, json!({}), "id").await;
    devices.until("the server's device", |rows| has(rows, "id", "device-server")).await;
    let mut chats = Watched::open(&t.window, methods::WATCH_CHATS, json!({}), "id").await;
    let rows = chats.until("the laptop's own chat", |rows| has(rows, "id", "own-chat")).await;
    assert!(!has(&rows, "id", "srv-chat"), "{rows:?}");
    assert!(rows.iter().all(|c| c["deviceId"] != "device-server"), "{rows:?}");
    drop(t.server_link);
}

/// A chat made in p, which the server shows (its chat) but grants no run
/// in: refused, in the server's words, marked for the window; not made.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_create_where_the_server_grants_no_run_is_refused_in_its_words() {
    let t = pair(&["sessions.read", "sessions.run.unconfined:q"]).await;
    let mut spaces = Watched::open(&t.window, methods::WATCH_SPACES, json!({}), "id").await;
    spaces.until("project p", |rows| has(rows, "id", "p")).await;
    let refused = t
        .window
        .call(methods::MUTATE, json!({ "op": "createChat", "chatId": "not-here", "spaceId": "p", "deviceId": "device-server" }))
        .await
        .expect_err("refused");
    let words = refused.to_string();
    assert!(words.starts_with(zend_engine::peer_view::REFUSED_PREFIX), "{words}");
    assert!(words.contains("may not run sessions in project p"), "{words}");
    assert!(t.server.workspace.chat("not-here").unwrap().is_none());
    // Its config, which it then sets, is the server's to refuse too.
    let config = t.window.call(methods::MUTATE, json!({ "op": "setChatConfig", "chatId": "srv-chat", "config": { "harness": "mock" } })).await;
    assert!(config.is_err(), "{config:?}");
    drop(t.server_link);
}

/// The laptop exposes nothing new: the server, dialling it back, is refused
/// with no grants; granted sessions.read, it sees the laptop's own chats,
/// never the server's rows the laptop shows its window.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_laptop_shows_a_server_nothing_of_another() {
    let t = pair(&["sessions.read", "sessions.run.unconfined:p"]).await;
    let mut chats = Watched::open(&t.window, methods::WATCH_CHATS, json!({}), "id").await;
    chats.until("the server's chat", |rows| has(rows, "id", "srv-chat")).await;
    let laptop_key = t.laptop_link.node_key().unwrap().public().to_string();
    let back = t.server_link.connect(&laptop_key).await.expect("the server dials the laptop");
    assert!(back.call(methods::ENGINE_INFO, json!({})).await.is_err(), "no grants: nothing");
    let server_key = t.server_link.node_key().unwrap().public().to_string();
    t.laptop_link.set_grants(&server_key, &["sessions.read".into()]).unwrap();
    let back = t.server_link.connect(&laptop_key).await.expect("dials again");
    let mut sub = back.subscribe_checked(methods::WATCH_CHATS, json!({})).await.expect("reads");
    let rows = tokio::time::timeout(Duration::from_secs(5), sub.recv()).await.unwrap().unwrap();
    let ids: Vec<&str> = rows.as_array().unwrap().iter().filter_map(|c| c["id"].as_str()).collect();
    assert_eq!(ids, ["own-chat"], "{rows}");
    assert!(back.subscribe_checked(methods::WATCH_DEVICES, json!({})).await.is_err(), "devices aren't a server's to read");
}
