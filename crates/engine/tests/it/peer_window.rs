//! Plan step 4e: a paired server shows in this engine's own window as a
//! device, with the chats and projects its grants let this side see, and a
//! chat made from the window in one of its projects is made and run there,
//! over the peer link (no edge). Two engines, each its own data folder and
//! peer link, paired over loopback.
//!
//! What a server says is display only: its rows can't take this device's
//! ids, folders or device, nor another server's (a lying server: [`Liar`]).

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::StreamExt as _;
use serde_json::{Value, json};
use zend_doc::commands::SessionCommandPayload;
use zend_engine::peer_link::{PeerLink, PeerLinkRpc, Seams};
use zend_engine::{EngineCore, HarnessRegistry};
use zend_proto::{HarnessId, RunRequest, SandboxLevel};
use zend_rpc::{RpcClient, RpcError, RpcReply, RpcService, methods};

/// What a server's replies become before they leave it (a lying server).
type Lie = Arc<dyn Fn(&str, Value) -> Value + Send + Sync>;

/// A server's own service, recording each call a paired server makes of it
/// and passing each reply (and each item of a stream) through `lie`.
struct Liar {
    inner: Arc<dyn RpcService>,
    lie: Lie,
    heard: Arc<Mutex<Vec<(String, Value)>>>,
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

/// Each row of a list (whole lists, as the peer view asks) through `edit`.
fn rows(v: Value, edit: impl Fn(&mut serde_json::Map<String, Value>)) -> Value {
    match v {
        Value::Array(mut rows) => {
            rows.iter_mut().filter_map(Value::as_object_mut).for_each(edit);
            Value::Array(rows)
        }
        other => other,
    }
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

/// A server with projects p and q and a chat in p, a laptop that joined it,
/// the grants the server gives the laptop, and the laptop's window.
struct Pair {
    _temp: tempfile::TempDir,
    root: std::path::PathBuf,
    server: EngineCore,
    server_link: Arc<PeerLink>,
    /// What the laptop asked of the server.
    heard: Arc<Mutex<Vec<(String, Value)>>>,
    laptop: EngineCore,
    laptop_link: Arc<PeerLink>,
    window: RpcClient,
}

impl Pair {
    /// Whether any call the server heard names `id`.
    fn server_heard_of(&self, id: &str) -> Vec<(String, Value)> {
        let needle = format!("\"{id}\"");
        self.heard.lock().unwrap().iter().filter(|(_, p)| p.to_string().contains(&needle)).cloned().collect()
    }
}

/// A server engine in `root/name` as `device`: projects p and q, a chat
/// `chat` in p; its link serving its own service through `lie`.
fn server(root: &Path, name: &str, device: &str, chat: &str, lie: Lie) -> (EngineCore, Arc<PeerLink>, Arc<Mutex<Vec<(String, Value)>>>) {
    let core = assemble(&root.join(name), device);
    for project in ["p", "q"] {
        let dir = root.join(format!("{name}-projects")).join(project);
        std::fs::create_dir_all(&dir).unwrap();
        core.workspace.create_space(project, &core.device_id, &dir.to_string_lossy(), None, false).unwrap();
    }
    core.workspace.create_chat(chat, Some("p"), Some(&core.device_id), None, None).unwrap();
    let heard = Arc::new(Mutex::new(Vec::new()));
    let served = Arc::new(Liar { inner: core.rpc_service(), lie, heard: heard.clone() });
    let link = PeerLink::new(&root.join(name), 0, served, loopback());
    (core, link, heard)
}

/// The laptop joins `server`, which grants it `grants`.
async fn join(laptop: &Arc<PeerLink>, server: &Arc<PeerLink>, grants: &[&str]) {
    let code = server.pair().await.expect("pair");
    laptop.join(code["code"].as_str().unwrap()).await.expect("join");
    let laptop_key = laptop.node_key().unwrap();
    server.set_grants(laptop_key.public(), &grants.iter().map(|g| g.to_string()).collect::<Vec<_>>()).expect("grants");
}

async fn pair(grants: &[&str]) -> Pair {
    pair_with(grants, honest(), |_, _| {}).await
}

/// [`pair`] with a server that says `lie`, both engines set up further
/// by `before` (server, laptop) before they pair.
async fn pair_with(grants: &[&str], lie: Lie, before: impl FnOnce(&EngineCore, &EngineCore)) -> Pair {
    let home = std::fs::canonicalize(std::env::var_os("HOME").expect("HOME")).unwrap();
    let temp = tempfile::Builder::new().prefix("zend-peer-window-").tempdir_in(home).unwrap();
    let root = std::fs::canonicalize(temp.path()).unwrap();
    let (server, server_link, heard) = server(&root, "server-data", "device-server", "srv-chat", lie);
    let laptop = assemble(&root.join("laptop-data"), "device-laptop");
    laptop.workspace.create_chat("own-chat", None, Some(&laptop.device_id), None, None).unwrap();
    before(&server, &laptop);

    let laptop_link = PeerLink::new(&root.join("laptop-data"), 0, laptop.rpc_service(), loopback());
    join(&laptop_link, &server_link, grants).await;
    let window = zend_rpc::memory_client(Arc::new(PeerLinkRpc { inner: laptop.rpc_service(), link: laptop_link.clone() }));
    Pair { _temp: temp, root, server, server_link, heard, laptop, laptop_link, window }
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

fn count(rows: &[Value], key: &str, id: &str) -> usize {
    rows.iter().filter(|r| r[key].as_str() == Some(id)).count()
}

/// Frames for `secs` seconds (what shows once the view had time to hear).
async fn settle(watched: &mut Watched, secs: u64) -> Vec<Value> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(secs);
    while let Ok(Some(frame)) = tokio::time::timeout_at(deadline, watched.sub.recv()).await {
        watched.apply(frame);
    }
    watched.rows.clone()
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

/// A chat made on a server from here is held only until its list has it
/// (then only its config, for the window setting it again), and nothing
/// of it once the server is unpaired; one the server doesn't list (no
/// sessions.read) is held until then.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn what_this_side_keeps_of_chats_it_made_there_goes_with_the_server() {
    let t = pair(&["sessions.read", "sessions.run.unconfined:p"]).await;
    let mut chats = Watched::open(&t.window, methods::WATCH_CHATS, json!({}), "id").await;
    chats.until("the server's chat", |rows| has(rows, "id", "srv-chat")).await;
    let config = json!({ "harness": "mock", "model": null, "reasoning": null, "modelOptions": {}, "sandbox": "workspace-write" });
    let create = |chat: &str| json!({ "op": "createChat", "chatId": chat, "spaceId": "p", "deviceId": "device-server", "config": config });
    t.window.call(methods::MUTATE, create("made-1")).await.expect("made there");
    chats.until("listed", |rows| has(rows, "id", "made-1")).await;
    let view = t.laptop_link.view();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while view.made_counts() != (0, 1) && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(view.made_counts(), (0, 1), "listed: only its config is kept");
    t.window
        .call(methods::MUTATE, json!({ "op": "setChatConfig", "chatId": "made-1", "config": config }))
        .await
        .expect("its config, as made, still answered here");
    let server_key = t.server_link.node_key().unwrap().public().to_string();
    t.laptop_link.unpair(&server_key).expect("unpaired");
    view.refresh();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    while view.made_counts() != (0, 0) && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(view.made_counts(), (0, 0), "unpaired: nothing of it kept");

    // Not listed (it grants no sessions.read): held until it's unpaired.
    let t = pair(&["sessions.run.unconfined:q"]).await;
    let mut spaces = Watched::open(&t.window, methods::WATCH_SPACES, json!({}), "id").await;
    spaces.until("project q", |rows| has(rows, "id", "q")).await;
    let made = json!({ "op": "createChat", "chatId": "made-2", "spaceId": "q", "deviceId": "device-server", "config": config });
    t.window.call(methods::MUTATE, made).await.expect("made there");
    let view = t.laptop_link.view();
    assert_eq!(view.made_counts(), (1, 0));
    let server_key = t.server_link.node_key().unwrap().public().to_string();
    t.laptop_link.unpair(&server_key).expect("unpaired");
    view.refresh();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    while view.made_counts() != (0, 0) && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(view.made_counts(), (0, 0), "unpaired: nothing of it kept");
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

/// The server has a chat with the id of the laptop's own (own-chat), a
/// project with the id of one of the laptop's (lp) and a chat in it, and a
/// chat whose id the laptop then makes in lp: the laptop's ids stay the
/// laptop's. Its rows show once, as the laptop's; the server's rows with
/// those ids don't show; the window's calls naming them run here, and the
/// server never hears of them.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn this_devices_own_ids_stay_here_whatever_a_server_claims() {
    let t = pair_with(&["sessions.read", "sessions.run.unconfined:p", "sessions.run.unconfined:lp"], honest(), |server, laptop| {
        let dir = std::env::temp_dir();
        laptop.workspace.create_space("lp", &laptop.device_id, &dir.to_string_lossy(), None, false).unwrap();
        server.workspace.create_space("lp", &server.device_id, &dir.to_string_lossy(), None, false).unwrap();
        server.workspace.create_chat("own-chat", Some("lp"), Some(&server.device_id), None, None).unwrap();
        server.workspace.create_chat("srv-in-lp", Some("lp"), Some(&server.device_id), None, None).unwrap();
        server.workspace.create_chat("srv-claimed", Some("p"), Some(&server.device_id), None, None).unwrap();
    })
    .await;
    let mut chats = Watched::open(&t.window, methods::WATCH_CHATS, json!({}), "id").await;
    chats.until("the server's chat", |rows| has(rows, "id", "srv-chat")).await;
    let rows = settle(&mut chats, 1).await;
    assert_eq!(count(&rows, "id", "own-chat"), 1, "{rows:?}");
    let own = rows.iter().find(|c| c["id"] == "own-chat").unwrap();
    assert_eq!(own["deviceId"], "device-laptop", "{own}");
    assert!(!has(&rows, "id", "srv-in-lp"), "a server's chat in the laptop's project lp: {rows:?}");
    let mut spaces = Watched::open(&t.window, methods::WATCH_SPACES, json!({}), "id").await;
    let rows = spaces.until("the server's p", |rows| has(rows, "id", "p")).await;
    assert_eq!(count(&rows, "id", "lp"), 1, "{rows:?}");
    assert_eq!(rows.iter().find(|s| s["id"] == "lp").unwrap()["deviceId"], "device-laptop");

    let config = json!({ "harness": "mock", "model": null, "reasoning": null, "modelOptions": {}, "sandbox": "workspace-write" });
    t.window
        .call(methods::MUTATE, json!({ "op": "setChatConfig", "chatId": "own-chat", "config": config }))
        .await
        .expect("the laptop's own chat's config, set here");
    t.window.call(methods::QUEUE_COMMAND, run("own-chat")).await.expect("its run queued here");
    t.window
        .call(methods::MUTATE, json!({ "op": "createChat", "chatId": "srv-claimed", "spaceId": "lp", "deviceId": "device-laptop", "config": config }))
        .await
        .expect("made here, in lp");
    t.window
        .call(methods::MUTATE, json!({ "op": "createChat", "chatId": "new-in-lp", "spaceId": "lp", "deviceId": "device-laptop", "config": config }))
        .await
        .expect("made here, in lp");
    t.window
        .call(methods::MUTATE, json!({ "op": "setChatConfig", "chatId": "srv-claimed", "config": config }))
        .await
        .expect("the chat made here, set here");
    for chat in ["srv-claimed", "new-in-lp"] {
        let made = t.laptop.workspace.chat(chat).unwrap().unwrap_or_else(|| panic!("{chat} is the laptop's"));
        assert_eq!(made.space_id.as_deref(), Some("lp"));
    }
    assert!(t.server.workspace.chat("new-in-lp").unwrap().is_none());
    for id in ["own-chat", "srv-claimed", "new-in-lp", "lp"] {
        let heard = t.server_heard_of(id);
        assert!(heard.is_empty(), "the server heard of {id}: {heard:?}");
    }
}

/// A server saying it is the laptop, or a Cloud device, shows nowhere, and
/// the window's calls naming the laptop stay here.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_server_saying_it_is_this_device_or_cloud_is_refused() {
    for claimed in ["device-laptop", "cloud-device-1"] {
        let lie: Lie = Arc::new(move |method, mut v| {
            if method == methods::ENGINE_INFO {
                v["deviceId"] = json!(claimed);
            }
            v
        });
        let t = pair_with(&["sessions.read", "sessions.run.unconfined:p"], lie, |_, _| {}).await;
        let mut chats = Watched::open(&t.window, methods::WATCH_CHATS, json!({}), "id").await;
        chats.until("the laptop's own chat", |rows| has(rows, "id", "own-chat")).await;
        let rows = settle(&mut chats, 3).await;
        assert!(!has(&rows, "id", "srv-chat"), "{claimed}: {rows:?}");
        assert!(t.laptop_link.view().devices().is_empty(), "{claimed}: {:?}", t.laptop_link.view().devices());
        let mut devices = Watched::open(&t.window, methods::WATCH_DEVICES, json!({}), "id").await;
        let rows = settle(&mut devices, 1).await;
        assert_eq!(count(&rows, "id", claimed), usize::from(claimed == "device-laptop"), "{claimed}: {rows:?}");
        t.window
            .call(methods::MUTATE, json!({ "op": "createChat", "chatId": "here-1", "deviceId": "device-laptop" }))
            .await
            .expect("made here");
        assert!(t.laptop.workspace.chat("here-1").unwrap().is_some(), "{claimed}");
        assert!(t.server_heard_of("here-1").is_empty(), "{claimed}");
    }
}

/// Two servers saying one device id: the first paired shows, the second
/// doesn't (its chats neither).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn of_two_servers_with_one_device_id_the_first_paired_keeps_it() {
    let t = pair(&["sessions.read"]).await;
    let (_second, second_link, _) = server(&t.root, "second-data", "device-server", "second-chat", honest());
    tokio::time::sleep(Duration::from_millis(20)).await;
    join(&t.laptop_link, &second_link, &["sessions.read"]).await;
    t.laptop_link.view().refresh();
    let mut chats = Watched::open(&t.window, methods::WATCH_CHATS, json!({}), "id").await;
    chats.until("the first's chat", |rows| has(rows, "id", "srv-chat")).await;
    let rows = settle(&mut chats, 4).await;
    assert!(has(&rows, "id", "srv-chat"), "{rows:?}");
    assert!(!has(&rows, "id", "second-chat"), "{rows:?}");
    assert_eq!(t.laptop_link.view().devices(), ["device-server"]);
    let mut devices = Watched::open(&t.window, methods::WATCH_DEVICES, json!({}), "id").await;
    let rows = settle(&mut devices, 1).await;
    assert_eq!(count(&rows, "id", "device-server"), 1, "{rows:?}");
}

/// A server whose rows name the laptop as their device, a folder of any
/// device's, the harness's folder, a checkout, a parent of the laptop's,
/// and capabilities the window acts on: shown as its own, none of that.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_servers_rows_are_its_own_and_name_no_folder() {
    let lie: Lie = Arc::new(|method, mut v| match method {
        methods::ENGINE_INFO => {
            v["capabilities"] = json!(["cloud-session", "cloud-account", "harness-updates-v1"]);
            v
        }
        methods::WATCH_CHATS => rows(v, |c| {
            c.insert("deviceId".into(), json!("device-laptop"));
            c.insert("cwd".into(), json!("/home/feysen/.ssh"));
            c.insert("harnessSessionCwd".into(), json!("/home/feysen"));
            c.insert("checkoutId".into(), json!("co-1"));
            c.insert("sourceContext".into(), json!({ "repoRoot": "/home/feysen/src" }));
            c.insert("parentChatId".into(), json!("own-chat"));
        }),
        methods::WATCH_SESSIONS => match v {
            Value::Array(mut sessions) => {
                sessions.push(json!({ "chatId": "srv-chat", "deviceId": "device-laptop", "status": "running" }));
                Value::Array(sessions)
            }
            other => other,
        },
        _ => v,
    });
    let t = pair_with(&["sessions.read", "sessions.run.unconfined:p"], lie, |_, _| {}).await;
    let mut chats = Watched::open(&t.window, methods::WATCH_CHATS, json!({}), "id").await;
    let rows = chats.until("the server's chat", |rows| has(rows, "id", "srv-chat")).await;
    for chat in rows.iter().filter(|c| c["id"] != "own-chat") {
        assert_eq!(chat["deviceId"], "device-server", "{chat}");
        for key in ["cwd", "harnessSessionCwd", "checkoutId", "sourceContext", "parentChatId"] {
            assert!(chat.get(key).is_none(), "{key}: {chat}");
        }
    }
    let mut sessions = Watched::open(&t.window, methods::WATCH_SESSIONS, json!({}), "chatId").await;
    let rows = sessions.until("the server's session", |rows| has(rows, "chatId", "srv-chat")).await;
    let session = rows.iter().find(|s| s["chatId"] == "srv-chat").unwrap();
    assert_eq!(session["deviceId"], "device-server", "{session}");
    let mut devices = Watched::open(&t.window, methods::WATCH_DEVICES, json!({}), "id").await;
    let rows = devices.until("the server", |rows| has(rows, "id", "device-server")).await;
    let device = rows.iter().find(|d| d["id"] == "device-server").unwrap();
    assert_eq!(device["capabilities"], json!(["paired-server"]), "{device}");
}

/// A list over 5000 rows is dropped as a frame (the server's device still
/// shows); a message over 4 MiB costs the connection, never shows.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_server_can_t_fill_this_sides_memory() {
    let many: Lie = Arc::new(|method, v| match (method, v) {
        (methods::WATCH_CHATS, Value::Array(mut rows)) => {
            rows.extend((0..5001).map(|i| json!({ "id": format!("fake-{i}"), "spaceId": "p" })));
            Value::Array(rows)
        }
        (_, v) => v,
    });
    let big: Lie = Arc::new(|method, v| match (method, v) {
        (methods::WATCH_CHATS, Value::Array(mut rows)) => {
            rows.push(json!({ "id": "big", "spaceId": "p", "title": "x".repeat(5 << 20) }));
            Value::Array(rows)
        }
        (_, v) => v,
    });
    for (lie, shown) in [(many, "fake-0"), (big, "big")] {
        let t = pair_with(&["sessions.read"], lie, |_, _| {}).await;
        let mut devices = Watched::open(&t.window, methods::WATCH_DEVICES, json!({}), "id").await;
        devices.until("the server", |rows| has(rows, "id", "device-server")).await;
        let mut chats = Watched::open(&t.window, methods::WATCH_CHATS, json!({}), "id").await;
        let rows = settle(&mut chats, 3).await;
        assert!(!has(&rows, "id", shown) && !has(&rows, "id", "srv-chat"), "{shown}: {} rows", rows.len());
        assert!(has(&rows, "id", "own-chat"));
    }
}

/// A linked phone (served by the same wrapped service, as `serve_ipc`
/// does) sees none of a paired server's rows, and its calls naming the
/// server's device, chat or project don't reach the server.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_phone_link_gets_no_paired_server() {
    let t = pair(&["sessions.read", "sessions.run.unconfined:p"]).await;
    let mut chats = Watched::open(&t.window, methods::WATCH_CHATS, json!({}), "id").await;
    chats.until("the window: the server's chat", |rows| has(rows, "id", "srv-chat")).await;

    let wrapped: Arc<dyn RpcService> = Arc::new(PeerLinkRpc { inner: t.laptop.rpc_service(), link: t.laptop_link.clone() });
    let key = zend_rpc::ipc_auth::generate_token().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let _served = tokio::spawn(zend_rpc::remote::serve_remote_listener_admitting(listener, wrapped, key.clone(), |_| {
        Box::pin(std::future::ready(true))
    }));
    let phone = tokio::time::timeout(Duration::from_secs(5), zend_rpc::connect_phone_link(&format!("ws://{addr}"), &key))
        .await
        .expect("in time")
        .expect("the phone dials in");
    let mut phone_chats = Watched::open(&phone, methods::WATCH_CHATS, json!({}), "id").await;
    let rows = phone_chats.until("the phone: the laptop's chat", |rows| has(rows, "id", "own-chat")).await;
    assert!(!has(&rows, "id", "srv-chat"), "{rows:?}");
    if let Ok(sub) = phone.subscribe_checked(methods::WATCH_DEVICES, json!({})).await {
        let mut devices = Watched { sub, rows: Vec::new(), key: "id" };
        let rows = settle(&mut devices, 1).await;
        assert!(!has(&rows, "id", "device-server"), "{rows:?}");
    }
    let before = t.heard.lock().unwrap().len();
    let _ = phone.call(methods::LIST_HARNESSES, json!({ "targetDeviceId": "device-server" })).await;
    let _ = phone.call(methods::QUEUE_COMMAND, run("srv-chat")).await;
    let _ = phone.call(methods::MUTATE, json!({ "op": "createChat", "chatId": "phone-made", "spaceId": "p", "deviceId": "device-server" })).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let heard: Vec<String> = t.heard.lock().unwrap()[before..].iter().map(|(m, _)| m.clone()).collect();
    assert!(heard.is_empty(), "the server heard the phone's calls: {heard:?}");
    assert!(t.server.workspace.chat("phone-made").unwrap().is_none());
}

/// The window's calls that name a server but are no server's to take (a
/// key typed into Settings, a worker with its keys, an upload, a queued
/// message): said no to here, and the server hears nothing of them; what a
/// server takes still goes (control: its harnesses).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn calls_no_server_takes_never_leave_this_device() {
    let t = pair(&["sessions.read", "sessions.run.unconfined:p"]).await;
    let mut chats = Watched::open(&t.window, methods::WATCH_CHATS, json!({}), "id").await;
    chats.until("the server's chat", |rows| has(rows, "id", "srv-chat")).await;
    let calls = [
        (methods::SET_ACP_AGENT_KEY, json!({ "agentId": "gemini", "value": "secret-key-1", "targetDeviceId": "device-server" })),
        (methods::CREATE_WORKER, json!({ "name": "w", "keys": { "anthropic": "secret-key-2" }, "targetDeviceId": "device-server" })),
        (methods::UPLOAD_CHUNK, json!({ "uploadId": "u1", "seq": 0, "data": "c2VjcmV0", "targetDeviceId": "device-server" })),
        (methods::QUEUE_MESSAGE, json!({ "chatId": "srv-chat", "text": "secret words 3" })),
        (methods::MUTATE, json!({ "op": "setChatCwd", "chatId": "srv-chat", "cwd": "/home/laptop/secret" })),
    ];
    for (method, params) in calls {
        let refused = t.window.call(method, params).await.expect_err(method);
        let said = refused.to_string();
        assert!(said.contains("peer_refused") && said.contains("isn't sent to a paired server"), "{method}: {said}");
    }
    let heard: Vec<String> = t.heard.lock().unwrap().iter().map(|(m, p)| format!("{m} {p}")).collect();
    for word in ["secret", "SetAcpAgentKey", "CreateWorker", "UploadChunk", "QueueMessage", "setChatCwd"] {
        assert!(!heard.iter().any(|h| h.contains(word)), "the server heard {word}: {heard:?}");
    }
    t.window
        .call(methods::LIST_HARNESSES, json!({ "targetDeviceId": "device-server" }))
        .await
        .expect("its harnesses, asked there");
    assert!(t.heard.lock().unwrap().iter().any(|(m, _)| m == methods::LIST_HARNESSES));
}

/// What the window does to a server's chat (its config before the first
/// message, renaming, seen, archiving, deleting) goes to that server, with
/// no `targetDeviceId` of this side's; the same on this device's own chat
/// stays here.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_servers_chat_is_changed_there() {
    let t = pair(&["sessions.read", "sessions.run.unconfined:p"]).await;
    let mut chats = Watched::open(&t.window, methods::WATCH_CHATS, json!({}), "id").await;
    chats.until("the server's chat", |rows| has(rows, "id", "srv-chat")).await;
    let config = json!({ "harness": "mock", "model": null, "reasoning": null, "modelOptions": {}, "sandbox": "read-only" });
    let ops = |chat: &str| {
        let mut set = json!({ "op": "setChatConfig", "chatId": chat, "config": config });
        if chat == "srv-chat" {
            set["targetDeviceId"] = json!("device-server");
        }
        [
            set,
            json!({ "op": "renameChat", "chatId": chat, "title": "Renamed from the laptop" }),
            json!({ "op": "markChatSeen", "chatId": chat }),
            json!({ "op": "setChatArchived", "chatId": chat, "archived": true }),
        ]
    };
    for params in ops("own-chat") {
        t.window.call(methods::MUTATE, params.clone()).await.unwrap_or_else(|e| panic!("{params}: {e}"));
    }
    assert!(t.server_heard_of("own-chat").is_empty(), "{:?}", t.server_heard_of("own-chat"));
    for params in ops("srv-chat") {
        t.window.call(methods::MUTATE, params.clone()).await.unwrap_or_else(|e| panic!("{params}: {e}"));
    }
    let heard = t.server_heard_of("srv-chat");
    let sent: Vec<&str> = heard.iter().filter_map(|(_, p)| p["op"].as_str()).collect();
    assert_eq!(sent, ["setChatConfig", "renameChat", "markChatSeen", "setChatArchived"], "{heard:?}");
    assert!(heard.iter().all(|(_, p)| p.get("targetDeviceId").is_none()), "{heard:?}");
    let there = t.server.workspace.chat("srv-chat").unwrap().unwrap();
    assert_eq!((there.title.as_deref(), there.archived), (Some("Renamed from the laptop"), true));
    assert_eq!(there.config.map(|c| c.sandbox), Some(SandboxLevel::ReadOnly));
    let here = t.laptop.workspace.chat("own-chat").unwrap().unwrap();
    assert_eq!((here.title.as_deref(), here.archived), (Some("Renamed from the laptop"), true));
    t.window.call(methods::MUTATE, json!({ "op": "deleteChat", "chatId": "srv-chat" })).await.expect("deleted there");
    assert!(t.server.workspace.chat("srv-chat").unwrap().is_none());
}

/// A chat made there from here, its config changed and then changed back
/// to the one it was made with: the server ends on that one (each is sent).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_config_set_back_to_the_one_made_with_reaches_the_server() {
    let t = pair(&["sessions.read", "sessions.run.unconfined:p"]).await;
    let mut spaces = Watched::open(&t.window, methods::WATCH_SPACES, json!({}), "id").await;
    spaces.until("project p", |rows| has(rows, "id", "p")).await;
    let config = |sandbox: &str| json!({ "harness": "mock", "model": null, "reasoning": null, "modelOptions": {}, "sandbox": sandbox });
    t.window
        .call(methods::MUTATE, json!({ "op": "createChat", "chatId": "made-a", "spaceId": "p", "deviceId": "device-server", "config": config("workspace-write") }))
        .await
        .expect("made there");
    let sandbox = || t.server.workspace.chat("made-a").unwrap().unwrap().config.map(|c| c.sandbox);
    for (sent, now) in [("read-only", SandboxLevel::ReadOnly), ("workspace-write", SandboxLevel::WorkspaceWrite)] {
        t.window
            .call(methods::MUTATE, json!({ "op": "setChatConfig", "chatId": "made-a", "config": config(sent) }))
            .await
            .unwrap_or_else(|e| panic!("{sent}: {e}"));
        assert_eq!(sandbox(), Some(now), "after {sent}");
    }
}

/// A call aimed at a server (`targetDeviceId`) that names this device's
/// own chat, project or device is refused here, in words saying so, and
/// the server hears nothing of it; this device's chat stays.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_call_aimed_at_a_server_naming_this_devices_own_ids_stays_here() {
    let t = pair_with(&["sessions.read", "sessions.run.unconfined:p"], honest(), |_, laptop| {
        // (Kept: the project's folder outlives the test's few seconds.)
        let home = std::fs::canonicalize(std::env::var_os("HOME").expect("HOME")).unwrap();
        let dir = tempfile::Builder::new().prefix("zend-peer-window-lap-").tempdir_in(home).unwrap().keep();
        laptop.workspace.create_space("lap-p", &laptop.device_id, &dir.to_string_lossy(), None, false).unwrap();
    })
    .await;
    let mut chats = Watched::open(&t.window, methods::WATCH_CHATS, json!({}), "id").await;
    chats.until("the server's chat", |rows| has(rows, "id", "srv-chat")).await;
    let mut spaces = Watched::open(&t.window, methods::WATCH_SPACES, json!({}), "id").await;
    spaces.until("both projects", |rows| has(rows, "id", "lap-p") && has(rows, "id", "p")).await;
    let before = t.heard.lock().unwrap().len();
    let calls = [
        (methods::MUTATE, json!({ "op": "deleteChat", "chatId": "own-chat", "targetDeviceId": "device-server" }), "chat own-chat"),
        (methods::MUTATE, json!({ "op": "setChatArchived", "chatId": "own-chat", "archived": true, "targetDeviceId": "device-server" }), "chat own-chat"),
        (methods::QUEUE_COMMAND, { let mut r = run("own-chat"); r["targetDeviceId"] = json!("device-server"); r }, "chat own-chat"),
        (methods::MUTATE, json!({ "op": "createChat", "chatId": "new-1", "spaceId": "lap-p", "targetDeviceId": "device-server" }), "project lap-p"),
        (methods::MUTATE, json!({ "op": "createChat", "chatId": "new-2", "deviceId": "device-laptop", "targetDeviceId": "device-server" }), "device device-laptop"),
    ];
    for (method, params, own) in calls {
        let said = t.window.call(method, params.clone()).await.expect_err(method).to_string();
        assert!(said.starts_with(zend_engine::peer_view::REFUSED_PREFIX) && said.contains(&format!("names this device's own {own}")), "{params}: {said}");
    }
    let heard: Vec<(String, Value)> = t.heard.lock().unwrap()[before..].to_vec();
    assert!(heard.iter().all(|(m, _)| m != methods::MUTATE && m != methods::QUEUE_COMMAND), "the server heard: {heard:?}");
    let own = t.laptop.workspace.chat("own-chat").unwrap().expect("this device's chat stays");
    assert!(!own.archived);
}

/// This device's archived chat is still its own: a server claiming its id
/// gets none of its calls.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_archived_chat_here_stays_here() {
    let t = pair_with(&["sessions.read", "sessions.run.unconfined:p"], honest(), |server, laptop| {
        laptop.workspace.create_chat("old-chat", None, Some(&laptop.device_id), None, None).unwrap();
        assert!(laptop.workspace.set_chat_archived("old-chat", true).unwrap());
        server.workspace.create_chat("old-chat", Some("p"), Some(&server.device_id), None, None).unwrap();
    })
    .await;
    let mut chats = Watched::open(&t.window, methods::WATCH_CHATS, json!({}), "id").await;
    chats.until("the server's chat", |rows| has(rows, "id", "srv-chat")).await;
    let config = json!({ "harness": "mock", "model": null, "reasoning": null, "modelOptions": {}, "sandbox": "workspace-write" });
    t.window
        .call(methods::MUTATE, json!({ "op": "setChatConfig", "chatId": "old-chat", "config": config }))
        .await
        .expect("set here");
    let _ = t.window.call(methods::QUEUE_COMMAND, run("old-chat")).await;
    let _ = t.window.subscribe_checked(methods::WATCH_DOC_MESSAGES, json!({ "chatId": "old-chat" })).await;
    let heard = t.server_heard_of("old-chat");
    assert!(heard.is_empty(), "the server heard of the archived chat: {heard:?}");
}

/// A run sent to a server's chat carries the words and the agent picked,
/// none of this device's folder, attachment paths, worktree, MCP servers,
/// resume or queued files.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_run_sent_to_a_server_carries_no_path_of_this_device() {
    let t = pair(&["sessions.read", "sessions.run.unconfined:p"]).await;
    let mut chats = Watched::open(&t.window, methods::WATCH_CHATS, json!({}), "id").await;
    chats.until("the server's chat", |rows| has(rows, "id", "srv-chat")).await;
    let mut params = run("srv-chat");
    let request = &mut params["command"]["request"];
    request["cwd"] = json!("/home/laptop/private");
    request["attachments"] = json!(["/home/laptop/private/shot.png"]);
    request["worktree"] = json!({ "repoPath": "/home/laptop/private", "base": "HEAD" });
    request["resume"] = json!("laptop-session-1");
    request["mcp"] = json!({ "name": "zend", "command": "/home/laptop/bin/zend", "args": [], "env": {} });
    params["transfers"] = json!([{ "path": "/home/laptop/private/shot.png" }]);
    t.window.call(methods::QUEUE_COMMAND, params).await.expect("its run queued there");
    let heard: Vec<Value> =
        t.heard.lock().unwrap().iter().filter(|(m, _)| m == methods::QUEUE_COMMAND).map(|(_, p)| p.clone()).collect();
    assert_eq!(heard.len(), 1, "{heard:?}");
    let sent = heard[0].to_string();
    for word in ["/home/laptop", "laptop-session-1", "transfers", "worktree", "mcp"] {
        assert!(!sent.contains(word), "{word} crossed: {sent}");
    }
    assert!(sent.contains("hello from the laptop"), "{sent}");
}

/// Unpaired there (the server forgot this side's key): its handshake is
/// refused (401), and nothing of it stays here, not kept as if it were
/// only out of reach.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_server_that_unpaired_this_side_leaves_nothing_here() {
    let t = pair(&["sessions.read", "sessions.run.unconfined:p"]).await;
    let mut devices = Watched::open(&t.window, methods::WATCH_DEVICES, json!({}), "id").await;
    devices.until("the server", |rows| has(rows, "id", "device-server")).await;
    let mut chats = Watched::open(&t.window, methods::WATCH_CHATS, json!({}), "id").await;
    chats.until("its chat", |rows| has(rows, "id", "srv-chat")).await;
    let mut spaces = Watched::open(&t.window, methods::WATCH_SPACES, json!({}), "id").await;
    spaces.until("its project", |rows| has(rows, "id", "p")).await;
    let laptop_key = t.laptop_link.node_key().unwrap().public().to_string();
    t.server_link.unpair(&laptop_key).expect("unpaired there");
    devices.until("the server gone", |rows| !has(rows, "id", "device-server")).await;
    chats.until("its chat gone", |rows| !has(rows, "id", "srv-chat")).await;
    spaces.until("its project gone", |rows| !has(rows, "id", "p")).await;
    assert!(has(&chats.rows, "id", "own-chat"), "{:?}", chats.rows);
}

/// Not reached (it stopped): its device shows offline from the first
/// failed redial, its rows stay a while, then go.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_server_not_reached_shows_offline_then_goes() {
    let t = pair(&["sessions.read"]).await;
    t.laptop_link.view().keep_quiet_servers_for(Duration::from_secs(6));
    let mut devices = Watched::open(&t.window, methods::WATCH_DEVICES, json!({}), "id").await;
    let online = |rows: &[Value]| rows.iter().any(|d| d["id"] == "device-server" && d["lastSeenAt"].is_string());
    devices.until("the server, online", online).await;
    let mut chats = Watched::open(&t.window, methods::WATCH_CHATS, json!({}), "id").await;
    chats.until("its chat", |rows| has(rows, "id", "srv-chat")).await;
    let Pair { _temp, server_link, window, laptop_link, server, laptop, .. } = t;
    drop(server_link);
    let offline = |rows: &[Value]| rows.iter().any(|d| d["id"] == "device-server" && d["lastSeenAt"].is_null());
    devices.until("the server, offline", offline).await;
    let rows = settle(&mut chats, 1).await;
    assert!(has(&rows, "id", "srv-chat"), "kept while it may come back: {rows:?}");
    devices.until("the server gone", |rows| !has(rows, "id", "device-server")).await;
    chats.until("its chat gone", |rows| !has(rows, "id", "srv-chat")).await;
    drop((window, laptop_link, server, laptop));
}

