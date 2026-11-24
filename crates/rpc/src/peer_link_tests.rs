use super::*;
use crate::RpcReply;
use futures::{SinkExt as _, StreamExt as _};
use std::collections::HashMap;
use std::sync::Mutex;
use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest, http::HeaderValue};

/// Every method name in lib.rs's methods module, and one added later.
fn every_method() -> Vec<&'static str> {
    let src = include_str!("lib.rs");
    let mut all: Vec<&str> = src
        .lines()
        .filter_map(|l| l.trim().strip_prefix("pub const ")?.split_once(": &str = \""))
        .map(|(_, rest)| rest.trim_end_matches("\";"))
        .collect();
    all.push("SomethingAddedLater");
    assert!(all.len() > 100, "{}", all.len());
    all
}

fn grants(list: &[&str]) -> Grants {
    Grants::new(list.iter().map(|g| g.parse::<Grant>().unwrap()))
}

#[test]
fn grants_read_and_print_as_the_plan_names_them() {
    for text in ["sessions.read", "sessions.run:p1", "sessions.run.unconfined:p-2", "projects.share:abc_D.e", "workers.manage"] {
        assert_eq!(text.parse::<Grant>().unwrap().to_string(), text);
    }
    for bad in ["", "sessions", "sessions.run", "sessions.run:", "sessions.run:a/b", "sessions.run:../x", "projects.share:/etc", "files.read", "sessions.read:p"] {
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
        let refused = peer_server_allowed(method, &none);
        assert!(refused.as_ref().is_err_and(|why| why.contains(method)), "{method}: {refused:?}");
    }
}

/// Each grant opens its own methods and no other; together, exactly
/// PEER_METHODS; never a local-only one (pairing, grants, credentials).
#[test]
fn each_grant_opens_only_its_methods() {
    let open = |g: &Grants| -> BTreeSet<&'static str> {
        every_method().into_iter().filter(|m| peer_server_allowed(m, g).is_ok()).collect()
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
        assert!(peer_server_allowed(m, &all).is_err(), "{m}");
    }
    for m in [methods::SET_PEER_GRANTS, methods::PEER_PAIR, methods::PEER_JOIN, methods::UNPAIR_PEER, methods::LIST_PEERS] {
        assert!(crate::local_only(m) && !crate::agent_allowed(m) && !crate::remote_allowed(m, &serde_json::Value::Null), "{m}");
    }
    // The window's path-taking methods are never a peer's.
    for m in [methods::READ_WORKSPACE_FILE, methods::LIST_WORKSPACE_DIRECTORY] {
        assert!(!PEER_METHODS.contains(&m), "{m}");
    }
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
    fn joined(&self, key: &str, info: JoinInfo) -> Result<JoinInfo, String> {
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
    let task = tokio::spawn(serve_peer_listener(listener, echo.clone(), key.clone(), book.clone(), anyone));
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
    let refused = client.call(methods::MUTATE, serde_json::json!({})).await.unwrap_err();
    assert!(refused.to_string().contains("isn't open to your server"), "{refused}");
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
