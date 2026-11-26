use super::*;
use crate::RpcReply;
use futures::{SinkExt as _, StreamExt as _};
use std::collections::HashMap;
use std::sync::Mutex;
use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest, http::HeaderValue};

/// Every method name in lib.rs's methods module, and one added later.
fn every_method() -> Vec<&'static str> {
    let src = include_str!("lib.rs");
    // The methods module only (not capability_errors' codes).
    let start = src.find("pub mod methods {").unwrap();
    let src = &src[start..start + src[start..].find("\n}\n").unwrap()];
    let mut all: Vec<&str> = src
        .lines()
        .filter_map(|l| l.trim().strip_prefix("pub const ")?.split_once(": &str = \""))
        .map(|(_, rest)| rest.trim_end_matches("\";"))
        .collect();
    all.push("SomethingAddedLater");
    assert!(all.len() > 100, "{}", all.len());
    all
}

/// Whether `g` opens `method` (of `Mutate`, any op).
fn opens(method: &str, g: &Grants) -> bool {
    if method == methods::MUTATE {
        crate::peer_scope::MUTATE_OPS.iter().any(|op| peer_server_allowed(method, Some(op), g).is_ok())
    } else {
        peer_server_allowed(method, None, g).is_ok()
    }
}

fn grants(list: &[&str]) -> Grants {
    Grants::new(list.iter().map(|g| g.parse::<Grant>().unwrap()))
}

#[test]
fn grants_read_and_print_as_the_plan_names_them() {
    for text in [
        "sessions.read",
        "sessions.run:p1",
        "sessions.run.unconfined:p-2",
        "projects.share:abc_D.e",
        "workers.manage",
        "projects.manage",
        "files",
        "settings",
        "logins",
        "updates",
        "terminal",
        "full",
    ] {
        assert_eq!(text.parse::<Grant>().unwrap().to_string(), text);
    }
    for bad in ["", "sessions", "sessions.run", "sessions.run:", "sessions.run:a/b", "sessions.run:../x", "projects.share:/etc", "files.read", "sessions.read:p", "full:p", "files:/", "Full"] {
        assert!(bad.parse::<Grant>().is_err(), "{bad}");
    }
    let g = grants(&["sessions.run:p", "sessions.run.unconfined:q", "projects.share:s"]);
    assert_eq!((g.runs_in("p"), g.runs_in("q"), g.runs_in("s")), (Some(false), Some(true), None));
    assert!(g.shares("s") && !g.shares("p") && !g.reads());
    let json = serde_json::to_value(&g).unwrap();
    assert_eq!(serde_json::from_value::<Grants>(json).unwrap(), g);
}

/// No grants: every method there is, refused, each with a reason.
#[test]
fn with_no_grants_every_method_is_refused() {
    let none = Grants::default();
    for method in every_method().into_iter().chain(PEER_METHODS.iter().copied()) {
        let refused = peer_server_allowed(method, None, &none);
        assert!(refused.as_ref().is_err_and(|why| why.contains(method)), "{method}: {refused:?}");
    }
}

/// Each grant opens its own methods and no other; together, exactly
/// PEER_METHODS; never a local-only one (pairing, grants, credentials).
#[test]
fn each_grant_opens_only_its_methods() {
    let open = |g: &Grants| -> BTreeSet<&'static str> {
        every_method().into_iter().filter(|m| opens(m, g)).collect()
    };
    let set = |list: &[&'static str]| list.iter().copied().collect::<BTreeSet<_>>();
    assert_eq!(
        open(&grants(&["sessions.read"])),
        set(&[methods::ENGINE_INFO, methods::WATCH_SESSIONS, methods::WATCH_CHATS, methods::WATCH_DOC_MESSAGES, methods::LIST_HARNESSES, methods::LIST_MODELS])
    );
    let run = set(&[methods::ENGINE_INFO, methods::LIST_HARNESSES, methods::LIST_MODELS, methods::MUTATE, methods::QUEUE_COMMAND, methods::WATCH_DOC_MESSAGES]);
    assert_eq!(open(&grants(&["sessions.run:p"])), run);
    assert_eq!(open(&grants(&["sessions.run.unconfined:p"])), run);
    assert_eq!(open(&grants(&["projects.share:p"])), set(&[methods::PEER_LIST_PROJECT_FILES, methods::PEER_READ_PROJECT_FILE]));
    assert_eq!(open(&grants(&["workers.manage"])), BTreeSet::new());
    let all = grants(&["sessions.read", "sessions.run:p", "sessions.run.unconfined:p", "projects.share:p", "workers.manage"]);
    assert_eq!(open(&all), set(PEER_METHODS));
    for m in every_method().into_iter().filter(|m| crate::local_only(m)) {
        assert!(!opens(m, &all), "{m}");
    }
    for m in [
        methods::SET_PEER_GRANTS,
        methods::PEER_PAIR,
        methods::PEER_JOIN,
        methods::UNPAIR_PEER,
        methods::LIST_PEERS,
        methods::LIST_PEER_CHANGES,
        methods::SHOW_PEER_CHANGES,
        methods::APPLY_PEER_CHANGES,
        methods::DISCARD_PEER_CHANGES,
        methods::SET_PEER_KEY,
        methods::REMOVE_PEER_KEY,
        methods::LIST_PEER_KEYS,
        methods::ALLOW_PEER_LOGIN,
        methods::REMOVE_PEER_LOGIN,
        methods::LIST_PEER_LOGINS,
        methods::CREATE_WORKER,
        methods::LIST_WORKERS,
        methods::START_WORKER,
        methods::REMOVE_WORKER,
        methods::STOP_WORKER,
        methods::ALLOW_WORKER_LOGIN,
        methods::REMOVE_WORKER_LOGIN,
        methods::LIST_WORKER_LOGINS,
    ] {
        assert!(crate::local_only(m) && !crate::agent_allowed(m) && !crate::remote_allowed(m, &serde_json::Value::Null), "{m}");
    }
    // The window's path-taking methods are never a peer's.
    for m in [methods::READ_WORKSPACE_FILE, methods::LIST_WORKSPACE_DIRECTORY] {
        assert!(!PEER_METHODS.contains(&m), "{m}");
    }
}

// ---- the scope table (plan step 6a) ----------------------------------------

/// Every method constant and every `Mutate` op has its row in
/// [`crate::peer_scope`]; a name it doesn't know has none. (The engine's
/// test holds [`crate::peer_scope::MUTATE_OPS`] to `MutateParams`.)
#[test]
fn the_table_classes_every_method_and_every_mutate_op() {
    let unclassed: Vec<&str> = every_method()
        .into_iter()
        .filter(|m| *m != "SomethingAddedLater" && *m != methods::MUTATE)
        .filter(|m| crate::peer_scope(m, None).is_none())
        .collect();
    assert!(unclassed.is_empty(), "methods with no row in peer_scope: {unclassed:?}");
    let ops: Vec<&str> =
        crate::peer_scope::MUTATE_OPS.iter().copied().filter(|op| crate::peer_scope(methods::MUTATE, Some(op)).is_none()).collect();
    assert!(ops.is_empty(), "Mutate ops with no row: {ops:?}");
    assert_eq!(crate::peer_scope("SomethingAddedLater", None), None);
    assert_eq!(crate::peer_scope(methods::MUTATE, None), None);
    assert_eq!(crate::peer_scope(methods::MUTATE, Some("somethingAddedLater")), None);
    // Unclassed is refused, even with everything.
    let all = grants(&["sessions.read", "sessions.run:p", "projects.share:p", "workers.manage", "full"]);
    assert!(peer_server_allowed("SomethingAddedLater", None, &all).is_err());
    assert!(peer_server_allowed(methods::MUTATE, Some("somethingAddedLater"), &all).is_err());
    assert!(peer_server_allowed(methods::MUTATE, None, &all).is_err());
}

/// How many methods (and `Mutate` ops) each scope holds, printed for the
/// record (`--nocapture`); every scope holds at least one.
#[test]
fn every_scope_holds_some_and_the_counts_are_printed() {
    use crate::Scope;
    let mut counts: std::collections::BTreeMap<Scope, (usize, usize)> = Default::default();
    for m in every_method().into_iter().filter(|m| *m != "SomethingAddedLater" && *m != methods::MUTATE) {
        counts.entry(crate::peer_scope(m, None).unwrap()).or_default().0 += 1;
    }
    for op in crate::peer_scope::MUTATE_OPS {
        counts.entry(crate::peer_scope(methods::MUTATE, Some(op)).unwrap()).or_default().1 += 1;
    }
    for (scope, (methods, ops)) in &counts {
        println!("scope {:<32} methods {methods:>3}  mutate ops {ops:>2}", scope.name());
    }
    assert_eq!(counts.len(), 14, "{counts:?}");
}

/// Each new grant: a call of its area goes through with it (or with
/// `full`), and is refused with every other grant but it and `full`.
#[test]
fn each_area_grant_opens_its_area_and_only_with_it() {
    let others = |but: &str| {
        let mut list = vec!["sessions.read", "sessions.run.unconfined:p", "projects.share:p", "workers.manage"];
        for g in ["projects.manage", "files", "settings", "logins", "updates", "terminal"] {
            if g != but {
                list.push(g);
            }
        }
        grants(&list)
    };
    let full = grants(&["full"]);
    for (grant, method, op) in [
        ("projects.manage", methods::LIST_FOLDERS, None),
        ("projects.manage", methods::MUTATE, Some("createSpace")),
        ("projects.manage", methods::CREATE_WORKTREE, None),
        ("files", methods::READ_WORKSPACE_FILE, None),
        ("files", methods::WRITE_WORKSPACE_FILE, None),
        ("files", methods::DISCARD_WORKING_TREE, None),
        ("settings", methods::USE_THEME, None),
        ("settings", methods::INSTALL_PLUGIN, None),
        ("settings", methods::MUTATE, Some("renameDevice")),
        ("logins", methods::LIST_AGENT_ACCOUNTS, None),
        ("logins", methods::START_AGENT_LOGIN, None),
        ("updates", methods::UPDATE_STATUS, None),
        ("updates", methods::APPLY_HARNESS_UPDATE, None),
        ("terminal", methods::OPEN_TERMINAL, None),
        ("terminal", methods::WRITE_TERMINAL, None),
        ("terminal", methods::RUN_PROJECT_ACTION, None),
    ] {
        assert!(peer_server_allowed(method, op, &grants(&[grant])).is_ok(), "{grant}: {method} {op:?}");
        assert!(peer_server_allowed(method, op, &full).is_ok(), "full: {method} {op:?}");
        let refused = peer_server_allowed(method, op, &others(grant));
        assert!(refused.as_ref().is_err_and(|why| why.contains(grant)), "without {grant}: {method} {op:?}: {refused:?}");
    }
    // What only `full` opens.
    for (method, op) in [
        (methods::QUEUE_MESSAGE, None),
        (methods::FORK_SIDE_CHAT, None),
        (methods::DECIDE_PLUGIN, None),
        (methods::CREATE_WORKER, None),
        (methods::MUTATE, Some("setChatCwd")),
    ] {
        assert!(peer_server_allowed(method, op, &full).is_ok(), "{method} {op:?}");
        assert!(peer_server_allowed(method, op, &others("")).is_err(), "{method} {op:?}");
    }
    // `full` reads sessions but runs and shares no project of its own.
    assert!(peer_server_allowed(methods::WATCH_CHATS, None, &full).is_ok());
    assert!(full.reads() && !full.runs_anywhere() && !full.shares("p"));
    assert!(peer_server_allowed(methods::QUEUE_COMMAND, None, &full).is_err());
    assert!(peer_server_allowed(methods::PEER_READ_PROJECT_FILE, None, &full).is_err());
    // An area grant alone still says who this device is.
    assert!(peer_server_allowed(methods::ENGINE_INFO, None, &grants(&["updates"])).is_ok());
    // An agent in a server's chat holds only the grants before step 6.
    assert_eq!(grants(&["full", "files", "sessions.run:p"]).sessions_only(), grants(&["sessions.run:p"]));
    assert!(!grants(&["full"]).sessions_only().reads());
}

/// The binding `never` list (plan step 6), and every other `Never` row:
/// refused with every grant there is.
#[test]
fn never_is_refused_even_with_full() {
    let all = grants(&[
        "sessions.read",
        "sessions.run.unconfined:p",
        "projects.share:p",
        "workers.manage",
        "projects.manage",
        "files",
        "settings",
        "logins",
        "updates",
        "terminal",
        "full",
    ]);
    let binding = [
        methods::PEER_PAIR,
        methods::PEER_JOIN,
        methods::LIST_PEERS,
        methods::SET_PEER_GRANTS,
        methods::UNPAIR_PEER,
        methods::SET_PEER_KEY,
        methods::REMOVE_PEER_KEY,
        methods::LIST_PEER_KEYS,
        methods::ALLOW_PEER_LOGIN,
        methods::REMOVE_PEER_LOGIN,
        methods::LIST_PEER_LOGINS,
        methods::SIGN_IN,
        methods::SIGN_OUT,
        methods::ACCESS_TOKEN,
        methods::STEP_UP,
        methods::DELETE_ACCOUNT,
        methods::LIST_ORGS,
        methods::CREATE_ORG,
        methods::SELECT_ORG,
        methods::BILLING_STATUS,
        methods::BILLING_CHECKOUT,
        methods::BILLING_PORTAL,
        methods::GITHUB_CONNECT_START,
        methods::GITHUB_CONNECT_POLL,
        methods::PHONE_LINK_STATUS,
        methods::SET_PHONE_LINK,
        methods::NEW_PHONE_LINK_KEY,
        methods::STOP_ENGINE,
    ];
    let prefixed: Vec<&str> =
        every_method().into_iter().filter(|m| m.starts_with("Cloud") || m.starts_with("Billing") || m.starts_with("Vault")).collect();
    assert!(prefixed.len() >= 15, "{prefixed:?}");
    for m in binding.into_iter().chain(prefixed) {
        assert_eq!(crate::peer_scope(m, None), Some(crate::Scope::Never), "{m}");
    }
    let mut never = 0;
    for m in every_method() {
        if crate::peer_scope(m, None) == Some(crate::Scope::Never) {
            never += 1;
            let refused = peer_server_allowed(m, None, &all);
            assert!(refused.as_ref().is_err_and(|why| why.contains("never")), "{m}: {refused:?}");
        }
    }
    assert!(never >= 50, "{never}");
    assert!(peer_server_allowed(methods::MUTATE, Some("removeDevice"), &all).is_err());
}

#[test]
fn a_signature_answers_only_its_own_keys_nonces_side_and_purpose() {
    let (a, b) = (NodeKey::generate().unwrap(), NodeKey::generate().unwrap());
    let (n1, n2) = (crate::link_proof::nonce().unwrap(), crate::link_proof::nonce().unwrap());
    let msg = transcript("callee", a.public(), b.public(), &n1, &n2, false);
    let sig = b.sign(&msg);
    assert!(verify(b.public(), &msg, &sig));
    assert!(!verify(a.public(), &msg, &sig));
    for other in [
        transcript("caller", a.public(), b.public(), &n1, &n2, false),
        transcript("callee", a.public(), b.public(), &n2, &n1, false),
        transcript("callee", a.public(), b.public(), &n1, &n2, true),
        transcript("callee", b.public(), a.public(), &n1, &n2, false),
    ] {
        assert!(!verify(b.public(), &other, &sig));
    }
    assert!(!verify(b.public(), &msg, "00"));
    let proof = join_proof("secret", &n1, a.public());
    assert!(proof_matches(&proof, &join_proof("secret", &n1, a.public())));
    assert!(!proof_matches(&proof, &join_proof("secret", &n1, b.public())));
    assert!(!proof_matches(&proof, &join_proof("secret", &n2, a.public())));
    assert!(!proof_matches(&proof, &join_proof("other", &n1, a.public())));
}

// ---- over the wire ------------------------------------------------------

/// Answers every call with the caller it was served as, and counts them.
#[derive(Default)]
struct Echo(std::sync::atomic::AtomicUsize);

/// Answers with `params.bytes` bytes of text.
struct Big;

#[async_trait::async_trait]
impl RpcService for Big {
    async fn handle(&self, _method: &str, params: serde_json::Value) -> Result<RpcReply, RpcError> {
        let bytes = params["bytes"].as_u64().unwrap_or(0) as usize;
        Ok(RpcReply::Value(serde_json::json!({ "blob": "x".repeat(bytes) })))
    }
}

#[async_trait::async_trait]
impl RpcService for Echo {
    async fn handle(&self, method: &str, _params: serde_json::Value) -> Result<RpcReply, RpcError> {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(RpcReply::Value(serde_json::json!({ "method": method, "caller": format!("{:?}", crate::caller()) })))
    }
}

/// A book in memory: codes are (secret, still good).
struct Book {
    peers: Mutex<HashMap<String, (Grants, JoinInfo)>>,
    codes: Mutex<Vec<(String, bool)>>,
    tick: watch::Sender<u64>,
    me: JoinInfo,
}

impl Book {
    fn new(name: &str) -> Arc<Self> {
        Arc::new(Self {
            peers: Mutex::default(),
            codes: Mutex::default(),
            tick: watch::channel(0).0,
            me: JoinInfo { name: Some(name.into()), ..JoinInfo::default() },
        })
    }

    fn pair(&self, key: &str, grants: Grants) {
        self.peers.lock().unwrap().insert(key.into(), (grants, JoinInfo::default()));
        self.tick.send_modify(|t| *t += 1);
    }

    fn unpair(&self, key: &str) {
        self.peers.lock().unwrap().remove(key);
        self.tick.send_modify(|t| *t += 1);
    }
}

impl PeerBook for Book {
    fn grants(&self, key: &str) -> Option<Grants> {
        self.peers.lock().unwrap().get(key).map(|(g, _)| g.clone())
    }
    fn take_code(&self, proof: &str, nonce: &str, key: &str) -> bool {
        let mut codes = self.codes.lock().unwrap();
        let Some(i) = codes.iter().position(|(s, _)| proof_matches(&join_proof(s, nonce, key), proof)) else {
            return false;
        };
        codes.remove(i).1
    }
    fn joined(&self, key: &str, info: JoinInfo, _node: Option<String>) -> Result<JoinInfo, String> {
        self.peers.lock().unwrap().insert(key.into(), (Grants::default(), info));
        self.tick.send_modify(|t| *t += 1);
        Ok(self.me.clone())
    }
    fn changes(&self) -> watch::Receiver<u64> {
        self.tick.subscribe()
    }
}

fn anyone(_: std::net::IpAddr) -> std::pin::Pin<Box<dyn std::future::Future<Output = bool> + Send>> {
    Box::pin(std::future::ready(true))
}

fn some_node(_: std::net::IpAddr) -> std::pin::Pin<Box<dyn std::future::Future<Output = Option<String>> + Send>> {
    Box::pin(std::future::ready(Some("node-test".to_string())))
}

struct Server {
    url: String,
    key: Arc<NodeKey>,
    book: Arc<Book>,
    echo: Arc<Echo>,
    task: tokio::task::JoinHandle<()>,
}

impl Server {
    fn served(&self) -> usize {
        self.echo.0.load(std::sync::atomic::Ordering::SeqCst)
    }
}

async fn server() -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let key = Arc::new(NodeKey::generate().unwrap());
    let book = Book::new("callee");
    let echo = Arc::new(Echo::default());
    let task = tokio::spawn(serve_peer_listener(listener, echo.clone(), key.clone(), book.clone(), anyone, some_node));
    Server { url, key, book, echo, task }
}

/// Whether the connection ends (the server cut it) within 5 s.
async fn cut(client: &RpcClient) -> bool {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while tokio::time::Instant::now() < deadline {
        if matches!(client.call(methods::ENGINE_INFO, serde_json::json!({})).await, Err(RpcError::Closed | RpcError::Transport(_))) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    false
}

/// Both prove their keys: the call is served as the caller's key, with
/// the grants the callee gave it.
#[tokio::test]
async fn a_paired_server_is_served_once_both_prove_their_keys() {
    let s = server().await;
    let me = NodeKey::generate().unwrap();
    s.book.pair(me.public(), grants(&["sessions.read"]));
    let client = connect_peer(&s.url, &me, s.key.public()).await.unwrap();
    let reply = client.call(methods::ENGINE_INFO, serde_json::json!({})).await.unwrap();
    let caller = reply["caller"].as_str().unwrap();
    assert!(caller.starts_with("PeerServer") && caller.contains(me.public()) && caller.contains("SessionsRead"), "{caller}");
    // Outside its grants: refused before the engine hears it, with why.
    let refused = client.call(methods::MUTATE, serde_json::json!({ "op": "createChat" })).await.unwrap_err();
    assert!(refused.to_string().contains("isn't open to your server"), "{refused}");
    // A Mutate that names no op is no server's.
    let refused = client.call(methods::MUTATE, serde_json::json!({})).await.unwrap_err();
    assert!(refused.to_string().contains("isn't open to another server"), "{refused}");
    let refused = client.call(methods::SET_PEER_GRANTS, serde_json::json!({})).await.unwrap_err();
    assert!(refused.to_string().contains("this device's own"), "{refused}");
    assert_eq!(s.served(), 1);
}

/// Paired with no grants: every method of the list refused, none served.
#[tokio::test]
async fn a_server_with_no_grants_is_refused_everything() {
    let s = server().await;
    let me = NodeKey::generate().unwrap();
    s.book.pair(me.public(), Grants::default());
    let client = connect_peer(&s.url, &me, s.key.public()).await.unwrap();
    for method in PEER_METHODS.iter().chain([methods::SET_PEER_GRANTS].iter()) {
        let refused = client.call(method, serde_json::json!({})).await;
        assert!(refused.as_ref().is_err_and(|e| e.to_string().contains(method)), "{method}: {refused:?}");
    }
    assert_eq!(s.served(), 0);
}

#[tokio::test]
async fn an_unknown_key_is_refused() {
    let s = server().await;
    let stranger = NodeKey::generate().unwrap();
    let refused = connect_peer(&s.url, &stranger, s.key.public()).await;
    assert!(refused.as_ref().is_err_and(|e| e.to_string().contains("401")), "{:?}", refused.err());
    assert_eq!(s.served(), 0);
}

/// The callee isn't the server the caller pinned (another key answers at
/// that address): the caller sends nothing, not even its signature.
#[tokio::test]
async fn a_callee_with_another_key_hears_nothing() {
    let s = server().await;
    let me = NodeKey::generate().unwrap();
    s.book.pair(me.public(), grants(&["sessions.read"]));
    let pinned = NodeKey::generate().unwrap();
    let refused = connect_peer(&s.url, &me, pinned.public()).await;
    assert!(refused.as_ref().is_err_and(|e| e.to_string().contains("didn't prove")), "{:?}", refused.err());
    assert_eq!(s.served(), 0);
}

/// A dial that names a paired key but can't sign for it (someone who knows
/// the public key only) is refused after the handshake, served nothing.
#[tokio::test]
async fn a_caller_naming_a_key_it_doesnt_hold_is_refused() {
    let s = server().await;
    let paired = NodeKey::generate().unwrap();
    let impostor = NodeKey::generate().unwrap();
    s.book.pair(paired.public(), grants(&["sessions.read"]));
    let nonce = crate::link_proof::nonce().unwrap();
    let mut request = s.url.as_str().into_client_request().unwrap();
    request.headers_mut().insert(KEY_HEADER, HeaderValue::from_str(paired.public()).unwrap());
    request.headers_mut().insert(NONCE_HEADER, HeaderValue::from_str(&nonce).unwrap());
    let (mut ws, response) = tokio_tungstenite::connect_async(request).await.unwrap();
    let callee_nonce = response.headers()[NONCE_HEADER].to_str().unwrap().to_string();
    let forged = impostor.sign(&transcript("caller", paired.public(), s.key.public(), &nonce, &callee_nonce, false));
    ws.send(Message::text(serde_json::json!({ SIG_FIELD: forged }).to_string())).await.unwrap();
    ws.send(Message::text(serde_json::json!({ "id": 1, "method": methods::ENGINE_INFO }).to_string())).await.unwrap();
    let answer = tokio::time::timeout(Duration::from_secs(5), ws.next()).await.unwrap();
    assert!(!matches!(answer, Some(Ok(Message::Text(_)))), "{answer:?}");
    assert_eq!(s.served(), 0);
}

/// Joining with a good code: both keep each other's key and what the other
/// said of itself. The code works once; an expired or wrong one, never.
#[tokio::test]
async fn a_code_pairs_once_and_an_expired_one_never() {
    let s = server().await;
    s.book.codes.lock().unwrap().push(("good-secret".into(), true));
    s.book.codes.lock().unwrap().push(("old-secret".into(), false));
    let me = NodeKey::generate().unwrap();
    let info = JoinInfo { name: Some("joiner".into()), host: Some("100.64.0.7".into()), port: Some(4711) };
    let them = join_peer(&s.url, &me, s.key.public(), "good-secret", info.clone()).await.unwrap();
    assert_eq!(them.name.as_deref(), Some("callee"));
    assert_eq!(s.book.peers.lock().unwrap().get(me.public()).cloned(), Some((Grants::default(), info.clone())));

    let again = NodeKey::generate().unwrap();
    let reused = join_peer(&s.url, &again, s.key.public(), "good-secret", info.clone()).await;
    assert!(reused.as_ref().is_err_and(|e| e.to_string().contains("401")), "{:?}", reused.err());
    let expired = join_peer(&s.url, &again, s.key.public(), "old-secret", info.clone()).await;
    assert!(expired.is_err());
    let wrong = join_peer(&s.url, &again, s.key.public(), "guessed", info).await;
    assert!(wrong.is_err());
    assert!(s.book.grants(again.public()).is_none());
    // The joined key now dials as a peer.
    assert!(connect_peer(&s.url, &me, s.key.public()).await.is_ok());
}

/// A joiner whose code is good but who dials the wrong server (another key
/// at the code's address) sends nothing and pairs nothing.
#[tokio::test]
async fn a_joiner_checks_the_codes_key_first() {
    let s = server().await;
    s.book.codes.lock().unwrap().push(("secret".into(), true));
    let me = NodeKey::generate().unwrap();
    let other = NodeKey::generate().unwrap();
    let refused = join_peer(&s.url, &me, other.public(), "secret", JoinInfo::default()).await;
    assert!(refused.as_ref().is_err_and(|e| e.to_string().contains("didn't prove")), "{:?}", refused.err());
    assert!(s.book.grants(me.public()).is_none());
}

/// Unpairing ends a paired server's connections; changing its grants does
/// too (it dials again for the new list). A new dial after unpairing: refused.
#[tokio::test]
async fn unpairing_or_new_grants_cut_its_connections() {
    let s = server().await;
    let me = NodeKey::generate().unwrap();
    s.book.pair(me.public(), grants(&["sessions.read"]));
    let client = connect_peer(&s.url, &me, s.key.public()).await.unwrap();
    assert!(client.call(methods::ENGINE_INFO, serde_json::json!({})).await.is_ok());
    s.book.pair(me.public(), grants(&["sessions.read", "projects.share:p"]));
    assert!(cut(&client).await, "new grants didn't cut");
    let client = connect_peer(&s.url, &me, s.key.public()).await.unwrap();
    assert!(client.call(methods::PEER_READ_PROJECT_FILE, serde_json::json!({})).await.is_ok());
    s.book.unpair(me.public());
    assert!(cut(&client).await, "unpairing didn't cut");
    assert!(connect_peer(&s.url, &me, s.key.public()).await.is_err());
    s.task.abort();
}

// ---- a worker's socket ----------------------------------------------------

/// A worker: its socket serves the one server it was made for, with the
/// grant it was made with (`sessions.run.unconfined:work`): the run
/// methods, nothing else; another key is refused at the handshake, a
/// worker that isn't the pinned one hears nothing, and no code pairs here.
#[cfg(unix)]
#[tokio::test]
async fn a_workers_socket_serves_only_its_server_and_only_the_run_methods() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("worker.sock");
    let listener = tokio::net::UnixListener::bind(&path).unwrap();
    let worker = Arc::new(NodeKey::generate().unwrap());
    let server = NodeKey::generate().unwrap();
    let book = Book::new("worker");
    book.pair(server.public(), grants(&["sessions.run.unconfined:work"]));
    book.codes.lock().unwrap().push(("a-code-that-would-pair".into(), true));
    let echo = Arc::new(Echo::default());
    let _task = tokio::spawn(serve_peer_unix(listener, echo.clone(), worker.clone(), book.clone()));

    let client = connect_peer_unix(&path, &server, worker.public()).await.unwrap();
    let reply = client.call(methods::ENGINE_INFO, serde_json::json!({})).await.unwrap();
    let caller = reply["caller"].as_str().unwrap();
    assert!(caller.starts_with("PeerServer") && caller.contains(server.public()), "{caller}");
    for open in [methods::LIST_HARNESSES, methods::LIST_MODELS, methods::MUTATE, methods::QUEUE_COMMAND, methods::WATCH_DOC_MESSAGES] {
        client.call(open, serde_json::json!({ "op": "createChat" })).await.unwrap_or_else(|e| panic!("{open}: {e}"));
    }
    for closed in [methods::WATCH_CHATS, methods::WATCH_SESSIONS, methods::PEER_READ_PROJECT_FILE, methods::SET_PEER_GRANTS, methods::PEER_PAIR, methods::ACCESS_TOKEN, "SomethingAddedLater"] {
        let refused = client.call(closed, serde_json::json!({})).await;
        assert!(refused.is_err(), "{closed}: {refused:?}");
    }
    assert_eq!(echo.0.load(std::sync::atomic::Ordering::SeqCst), 6);

    // Another key: refused before anything is served.
    let stranger = NodeKey::generate().unwrap();
    let refused = connect_peer_unix(&path, &stranger, worker.public()).await;
    assert!(refused.as_ref().is_err_and(|e| e.to_string().contains("401")), "{:?}", refused.err());
    // A socket answering with another key than the worker's pinned one.
    let other = NodeKey::generate().unwrap();
    let refused = connect_peer_unix(&path, &server, other.public()).await;
    assert!(refused.as_ref().is_err_and(|e| e.to_string().contains("didn't prove")), "{:?}", refused.err());
    // A code doesn't pair over a worker's socket (no tailnet node to pin).
    let (request, _) = caller_request(WORKER_URL, &stranger, Some("a-code-that-would-pair")).unwrap();
    let stream = tokio::net::UnixStream::connect(&path).await.unwrap();
    let joined = tokio_tungstenite::client_async(request, stream).await;
    assert!(joined.as_ref().is_err_and(|e| e.to_string().contains("401")), "{:?}", joined.err());
    assert!(book.grants(stranger.public()).is_none());
    assert_eq!(echo.0.load(std::sync::atomic::Ordering::SeqCst), 6);
}

/// Its server dials a worker's socket only as itself: a symlink to another
/// socket, or a file, put in its place is refused, not followed (the worker
/// writes where its socket is). Control: the socket itself answers.
#[cfg(unix)]
#[tokio::test]
async fn a_workers_socket_is_dialled_only_as_itself() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("worker.sock");
    let listener = tokio::net::UnixListener::bind(&path).unwrap();
    let worker = Arc::new(NodeKey::generate().unwrap());
    let server = NodeKey::generate().unwrap();
    let book = Book::new("worker");
    book.pair(server.public(), grants(&["sessions.run.unconfined:work"]));
    let _task = tokio::spawn(serve_peer_unix(listener, Arc::new(Echo::default()), worker.clone(), book));

    let client = connect_peer_unix(&path, &server, worker.public()).await.expect("the socket itself");
    client.call(methods::ENGINE_INFO, serde_json::json!({})).await.unwrap();
    let link = dir.path().join("link.sock");
    std::os::unix::fs::symlink(&path, &link).unwrap();
    let refused = connect_peer_unix(&link, &server, worker.public()).await;
    assert!(refused.as_ref().is_err_and(|e| e.to_string().contains("not a socket")), "a symlink was followed: {:?}", refused.err());
    let file = dir.path().join("file.sock");
    std::fs::write(&file, b"").unwrap();
    let refused = connect_peer_unix(&file, &server, worker.public()).await;
    assert!(refused.as_ref().is_err_and(|e| e.to_string().contains("not a socket")), "{:?}", refused.err());
}

/// Its server takes no message over 16 MiB from a worker (a worker is as
/// trusted as its agent): the connection ends. Control: 15 MiB comes in.
#[cfg(unix)]
#[tokio::test]
async fn a_server_takes_no_message_over_16_mib_from_a_worker() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("worker.sock");
    let listener = tokio::net::UnixListener::bind(&path).unwrap();
    let worker = Arc::new(NodeKey::generate().unwrap());
    let server = NodeKey::generate().unwrap();
    let book = Book::new("worker");
    book.pair(server.public(), grants(&["sessions.run.unconfined:work"]));
    let _task = tokio::spawn(serve_peer_unix(listener, Arc::new(Big), worker.clone(), book));

    let client = connect_peer_unix(&path, &server, worker.public()).await.unwrap();
    let small = client.call(methods::ENGINE_INFO, serde_json::json!({ "bytes": 15 << 20 })).await.expect("15 MiB");
    assert_eq!(small["blob"].as_str().map(str::len), Some(15 << 20));
    let big = client.call(methods::ENGINE_INFO, serde_json::json!({ "bytes": 17 << 20 })).await;
    assert!(big.is_err(), "a 17 MiB reply was taken in");
}

/// The tailnet listener takes no message over 4 MiB from a paired server
/// (the connection ends, nothing served); 3 MiB is served.
#[tokio::test]
async fn the_tailnet_listener_takes_no_message_over_4_mib() {
    let s = server().await;
    let me = NodeKey::generate().unwrap();
    s.book.pair(me.public(), grants(&["sessions.read"]));
    let client = connect_peer(&s.url, &me, s.key.public()).await.unwrap();
    client.call(methods::ENGINE_INFO, serde_json::json!({ "pad": "x".repeat(3 << 20) })).await.expect("3 MiB");
    assert_eq!(s.served(), 1);
    let big = client.call(methods::ENGINE_INFO, serde_json::json!({ "pad": "x".repeat(5 << 20) })).await;
    assert!(matches!(big, Err(RpcError::Closed | RpcError::Transport(_))), "a 5 MiB call: {big:?}");
    assert_eq!(s.served(), 1, "the 5 MiB call was served");
}

/// A worker's socket takes up to 16 MiB from its one server (it runs what
/// that server sends: prompts, files): 5 MiB is served there.
#[cfg(unix)]
#[tokio::test]
async fn a_workers_socket_takes_5_mib_from_its_server() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("worker.sock");
    let listener = tokio::net::UnixListener::bind(&path).unwrap();
    let worker = Arc::new(NodeKey::generate().unwrap());
    let server = NodeKey::generate().unwrap();
    let book = Book::new("worker");
    book.pair(server.public(), grants(&["sessions.run.unconfined:work"]));
    let echo = Arc::new(Echo::default());
    let _task = tokio::spawn(serve_peer_unix(listener, echo.clone(), worker.clone(), book));
    let client = connect_peer_unix(&path, &server, worker.public()).await.unwrap();
    client.call(methods::ENGINE_INFO, serde_json::json!({ "pad": "x".repeat(5 << 20) })).await.expect("5 MiB");
    assert_eq!(echo.0.load(std::sync::atomic::Ordering::SeqCst), 1);
}

/// A server that connects granted nothing, again and again (it redials),
/// is said once; granted something, then nothing again, it's said again.
/// Each peer on its own.
#[test]
fn granted_nothing_is_said_once_per_change() {
    let (a, b) = ("granted-nothing-test-a", "granted-nothing-test-b");
    let said: Vec<bool> = [(a, true), (a, true), (b, true), (a, true), (a, false), (a, true), (a, true)]
        .iter()
        .map(|(peer, nothing)| granted_nothing(peer, *nothing))
        .collect();
    assert_eq!(said, [true, false, true, false, false, true, false]);
}
