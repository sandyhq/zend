//! Server to server: another Zend engine (a server the owner paired with
//! this one) calling this engine over the tailnet, with no edge between.
//!
//! Three gates, all required, as on the phone link ([`crate::remote`]):
//!
//! 1. Where. Its listener binds only a tailnet address
//!    ([`crate::bind_remote`]) and admits only peers the engine's check lets
//!    in (Tailscale's `whois`), before the handshake is read.
//! 2. Who. Each engine has an Ed25519 node key of its own; pairing pins each
//!    side's public key in the other's book ([`PeerBook`]). A dial proves it
//!    both ways, the callee first: the caller's handshake names its key and
//!    a fresh nonce; the callee answers only a key in its book (or a joiner
//!    holding a good pairing code), with its own key, a fresh nonce and its
//!    signature over both keys and both nonces ([`SIG_HEADER`]). The caller
//!    checks that signature against the key it pinned before it sends
//!    anything, then signs the same transcript in its first frame; until
//!    that matches, the callee serves nothing. No secret crosses the wire.
//! 3. What. A connection reaches only what this side granted that peer
//!    ([`Grants`], kept per peer by this side alone, empty at first):
//!    [`peer_server_allowed`], served as [`crate::Caller::PeerServer`] so the
//!    engine checks each call's project. Never [`crate::local_only`]:
//!    pairing, grants and credentials are this device's own.
//!
//! Pairing: one side shows a code (its key, address, port and a one-time
//! secret, good for 10 minutes; the engine's). The other dials with
//! [`join_peer`]: its handshake carries [`join_proof`] (an HMAC of its own
//! key and nonce under the secret, so the secret never goes out and the
//! proof works for no other key), the callee takes the code (once, even if
//! the dial then fails) and proves its key as on any dial. Once the joiner's
//! frame proves its key, both keep each other.
//!
//! A change to a peer's entry (its grants, or unpairing) ends that peer's
//! connections: it dials again and gets the new list, or nothing.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use hmac::{Hmac, Mac};
use ring::signature::{ED25519, Ed25519KeyPair, KeyPair, UnparsedPublicKey};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpListener;
use tokio::sync::{mpsc, watch};
use tokio_tungstenite::tungstenite::handshake::server::{
    ErrorResponse, Request as HandshakeRequest, Response as HandshakeResponse,
};
use tokio_tungstenite::tungstenite::http::StatusCode;

use crate::{RpcClient, RpcError, RpcReply, RpcService, methods};

/// The handshake header that names a side's public key (the caller's
/// request, the callee's response).
pub const KEY_HEADER: &str = "x-zend-peer-key";
/// The handshake header that carries a side's fresh nonce.
pub const NONCE_HEADER: &str = "x-zend-peer-nonce";
/// The response header that carries the callee's signature.
pub const SIG_HEADER: &str = "x-zend-peer-sig";
/// The request header of a joiner: [`join_proof`].
pub const JOIN_HEADER: &str = "x-zend-peer-join";
/// The field of the caller's first frame that carries its signature.
pub const SIG_FIELD: &str = "peerSig";

/// How long a caller has to prove its key once the handshake is done, and
/// a joiner to hear it was kept.
const PROOF_TIMEOUT: Duration = Duration::from_secs(10);
const DIAL_TIMEOUT: Duration = Duration::from_secs(5);

// ---- identity ---------------------------------------------------------------

/// This engine's node key: an Ed25519 key pair (kept by the engine, PKCS#8).
pub struct NodeKey {
    pair: Ed25519KeyPair,
    public: String,
}

impl std::fmt::Debug for NodeKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NodeKey").field("public", &self.public).finish_non_exhaustive()
    }
}

impl NodeKey {
    /// A new key pair, PKCS#8 DER, from the OS CSPRNG.
    pub fn generate_pkcs8() -> Result<Vec<u8>, String> {
        let rng = ring::rand::SystemRandom::new();
        Ed25519KeyPair::generate_pkcs8(&rng)
            .map(|der| der.as_ref().to_vec())
            .map_err(|_| "no randomness for a node key".to_string())
    }

    pub fn from_pkcs8(der: &[u8]) -> Result<Self, String> {
        let pair = Ed25519KeyPair::from_pkcs8(der).map_err(|err| format!("not a node key: {err}"))?;
        let public = crate::ipc_auth::hex(pair.public_key().as_ref());
        Ok(Self { pair, public })
    }

    /// A key of its own, kept nowhere (tests).
    pub fn generate() -> Result<Self, String> {
        Self::from_pkcs8(&Self::generate_pkcs8()?)
    }

    /// The public key, 64 lowercase hex digits.
    pub fn public(&self) -> &str {
        &self.public
    }

    fn sign(&self, message: &[u8]) -> String {
        crate::ipc_auth::hex(self.pair.sign(message).as_ref())
    }
}

/// Whether `key` is a public key as [`NodeKey::public`] writes one.
pub fn valid_key(key: &str) -> bool {
    key.len() == 64 && key.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    if text.len() % 2 != 0 {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok())
        .collect()
}

fn verify(public: &str, message: &[u8], signature: &str) -> bool {
    match (valid_key(public).then(|| unhex(public)).flatten(), unhex(signature.trim())) {
        (Some(public), Some(signature)) => {
            UnparsedPublicKey::new(&ED25519, public).verify(message, &signature).is_ok()
        }
        _ => false,
    }
}

/// What each side signs: its role, both keys, both nonces and whether the
/// dial joins (so no signature answers another dial, role or purpose).
fn transcript(side: &str, caller: &str, callee: &str, caller_nonce: &str, callee_nonce: &str, joining: bool) -> Vec<u8> {
    let purpose = if joining { "join" } else { "connect" };
    format!("zend peer link v1: {side}\n{caller}\n{callee}\n{caller_nonce}\n{callee_nonce}\n{purpose}\n").into_bytes()
}

/// A joiner's proof that it holds a pairing code's secret, for its own key
/// and this dial's nonce: HMAC-SHA256 under the secret.
pub fn join_proof(secret: &str, caller_nonce: &str, caller_key: &str) -> String {
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(secret.trim().as_bytes()).expect("HMAC takes a key of any length");
    mac.update(format!("zend peer join v1\n{caller_nonce}\n{caller_key}\n").as_bytes());
    crate::ipc_auth::hex(&mac.finalize().into_bytes())
}

/// Whether `presented` is `expected`, in constant time.
pub fn proof_matches(expected: &str, presented: &str) -> bool {
    crate::ipc_auth::constant_time_eq(expected.as_bytes(), presented.trim().as_bytes())
}

// ---- grants -----------------------------------------------------------------

/// One thing this side lets a paired server do here.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Grant {
    /// `sessions.read`: list this device's chats and sessions, read their
    /// transcripts.
    SessionsRead,
    /// `sessions.run:<project>`: start chats in that project and run them,
    /// confined to its folder (a sandbox), and read those chats.
    SessionsRun(String),
    /// `sessions.run.unconfined:<project>`: as `sessions.run`, unsandboxed,
    /// and any chat in that project.
    SessionsRunUnconfined(String),
    /// `projects.share:<project>`: read that project's files, by paths
    /// from its root.
    ProjectsShare(String),
    /// `workers.manage`: run this device's workers (none yet: step 5).
    WorkersManage,
}

/// Whether `id` may name a project in a grant.
fn valid_project(id: &str) -> bool {
    (1..=128).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

impl std::str::FromStr for Grant {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, String> {
        let text = text.trim();
        let project = |rest: &str| {
            valid_project(rest).then(|| rest.to_string()).ok_or_else(|| format!("{text}: not a project id"))
        };
        match text.split_once(':') {
            None if text == "sessions.read" => Ok(Grant::SessionsRead),
            None if text == "workers.manage" => Ok(Grant::WorkersManage),
            Some(("sessions.run", rest)) => project(rest).map(Grant::SessionsRun),
            Some(("sessions.run.unconfined", rest)) => project(rest).map(Grant::SessionsRunUnconfined),
            Some(("projects.share", rest)) => project(rest).map(Grant::ProjectsShare),
            _ => Err(format!(
                "{text}: not a grant (sessions.read, sessions.run:<project>, sessions.run.unconfined:<project>, projects.share:<project>, workers.manage)"
            )),
        }
    }
}

impl std::fmt::Display for Grant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Grant::SessionsRead => f.write_str("sessions.read"),
            Grant::SessionsRun(p) => write!(f, "sessions.run:{p}"),
            Grant::SessionsRunUnconfined(p) => write!(f, "sessions.run.unconfined:{p}"),
            Grant::ProjectsShare(p) => write!(f, "projects.share:{p}"),
            Grant::WorkersManage => f.write_str("workers.manage"),
        }
    }
}

impl Serialize for Grant {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Grant {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        String::deserialize(d)?.parse().map_err(serde::de::Error::custom)
    }
}

/// What this side lets one paired server do here (empty: nothing).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Grants(BTreeSet<Grant>);

impl Grants {
    pub fn new(grants: impl IntoIterator<Item = Grant>) -> Self {
        Self(grants.into_iter().collect())
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Grant> {
        self.0.iter()
    }

    pub fn contains(&self, grant: &Grant) -> bool {
        self.0.contains(grant)
    }

    /// `sessions.read`.
    pub fn reads(&self) -> bool {
        self.contains(&Grant::SessionsRead)
    }

    /// Whether it may run chats in `project`: `Some(true)` unconfined,
    /// `Some(false)` confined to its folder, `None` not at all.
    pub fn runs_in(&self, project: &str) -> Option<bool> {
        if self.contains(&Grant::SessionsRunUnconfined(project.to_string())) {
            Some(true)
        } else if self.contains(&Grant::SessionsRun(project.to_string())) {
            Some(false)
        } else {
            None
        }
    }

    /// Whether it may run chats in any project.
    pub fn runs_anywhere(&self) -> bool {
        self.0.iter().any(|g| matches!(g, Grant::SessionsRun(_) | Grant::SessionsRunUnconfined(_)))
    }

    /// `projects.share:<project>`.
    pub fn shares(&self, project: &str) -> bool {
        self.contains(&Grant::ProjectsShare(project.to_string()))
    }

    fn shares_any(&self) -> bool {
        self.0.iter().any(|g| matches!(g, Grant::ProjectsShare(_)))
    }
}

// ---- the method list --------------------------------------------------------

/// With `sessions.read`: the chats and sessions, and their transcripts.
fn reads(method: &str) -> bool {
    matches!(
        method,
        methods::ENGINE_INFO
            | methods::WATCH_SESSIONS
            | methods::WATCH_CHATS
            | methods::WATCH_DOC_MESSAGES
            | methods::LIST_HARNESSES
            | methods::LIST_MODELS
    )
}

/// With `sessions.run:<project>` (either kind): a chat created there
/// (`Mutate` `createChat` only, the engine checks), run, steered, stopped
/// or answered (`QueueCommand`), and its transcript read; the harnesses
/// and models to pick.
fn runs(method: &str) -> bool {
    matches!(
        method,
        methods::ENGINE_INFO
            | methods::LIST_HARNESSES
            | methods::LIST_MODELS
            | methods::MUTATE
            | methods::QUEUE_COMMAND
            | methods::WATCH_DOC_MESSAGES
    )
}

/// With `projects.share:<project>`: its files, by project id and a path
/// from its root (no method of the window's, which name absolute paths).
fn shares(method: &str) -> bool {
    matches!(method, methods::PEER_LIST_PROJECT_FILES | methods::PEER_READ_PROJECT_FILE)
}

/// Every method any grant opens to a paired server (for the record and
/// the tests): nothing else is ever served on a peer connection.
pub const PEER_METHODS: &[&str] = &[
    methods::ENGINE_INFO,
    methods::WATCH_SESSIONS,
    methods::WATCH_CHATS,
    methods::WATCH_DOC_MESSAGES,
    methods::LIST_HARNESSES,
    methods::LIST_MODELS,
    methods::MUTATE,
    methods::QUEUE_COMMAND,
    methods::PEER_LIST_PROJECT_FILES,
    methods::PEER_READ_PROJECT_FILE,
];

/// Whether a paired server holding `grants` may call `method` here, or why
/// not. An allow-list of its own, built from the grants: a method added
/// later, anywhere, stays this device's until it's listed here. Which
/// project a call touches is the engine's to check
/// ([`crate::Caller::PeerServer`]).
pub fn peer_server_allowed(method: &str, grants: &Grants) -> Result<(), String> {
    if crate::local_only(method) {
        return Err(format!("{method} is this device's own: never open to another server"));
    }
    if grants.is_empty() {
        return Err(format!("{method}: this device grants your server nothing yet"));
    }
    let open = (grants.reads() && reads(method))
        || (grants.runs_anywhere() && runs(method))
        || (grants.shares_any() && shares(method));
    // workers.manage opens nothing until there are workers (step 5).
    if open {
        Ok(())
    } else {
        Err(format!("{method} isn't open to your server: no grant here covers it"))
    }
}

// ---- the book ---------------------------------------------------------------

/// What a side says of itself when pairing: a name to show, and where the
/// other can dial it back (its tailnet address and peer port, if any).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct JoinInfo {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
}

/// The engine's paired servers and its open pairing codes, as the listener
/// asks them.
pub trait PeerBook: Send + Sync {
    /// The grants this side gives the server holding `key`, if it's paired.
    fn grants(&self, key: &str) -> Option<Grants>;
    /// A joiner's [`join_proof`]: takes the code it proves (once: it's gone
    /// whatever happens next) when one is still good.
    fn take_code(&self, proof: &str, caller_nonce: &str, caller_key: &str) -> bool;
    /// The joiner, proved, kept as a peer (grants empty) and pinned to the
    /// tailnet node it came from (`node`); what this side says of itself back.
    fn joined(&self, key: &str, info: JoinInfo, node: Option<String>) -> Result<JoinInfo, String>;
    /// The tailnet node (Tailscale's stable node id) the server holding
    /// `key` was paired from: it is only ever let in from that node.
    fn node(&self, _key: &str) -> Option<String> {
        None
    }
    /// Ticks on every change to the book (grants, unpairing).
    fn changes(&self) -> watch::Receiver<u64>;
}

// ---- serving ----------------------------------------------------------------

/// A paired server's connection's view of the service.
struct PeerGuard {
    inner: Arc<dyn RpcService>,
    book: Arc<dyn PeerBook>,
    peer: String,
}

#[async_trait::async_trait]
impl RpcService for PeerGuard {
    async fn handle(&self, method: &str, params: serde_json::Value) -> Result<RpcReply, RpcError> {
        let Some(grants) = self.book.grants(&self.peer) else {
            return Err(RpcError::Failed("this device isn't paired with your server any more".into()));
        };
        if let Err(why) = peer_server_allowed(method, &grants) {
            tracing::warn!(method, peer = %short(&self.peer), "rpc: refused a paired server's call outside its grants");
            return Err(RpcError::Failed(why));
        }
        let caller = crate::Caller::PeerServer { peer: self.peer.clone(), grants };
        crate::with_caller(caller, self.inner.handle(method, params)).await
    }
}

fn short(key: &str) -> &str {
    key.get(..12).unwrap_or(key)
}

/// Serve paired servers on `listener` (from [`crate::bind_remote`]) as
/// `key`: only peers `admits` lets in, only keys in `book` (or a joiner
/// with a good code), only what their grants open. Every connection lives
/// in this future: dropping it ends them all.
/// Which tailnet node holds an address: Tailscale's stable node id
/// (`tailscale whois`), `None` when it can't say.
pub type NodeOf = fn(std::net::IpAddr) -> std::pin::Pin<Box<dyn std::future::Future<Output = Option<String>> + Send>>;

pub async fn serve_peer_listener(
    listener: TcpListener,
    service: Arc<dyn RpcService>,
    key: Arc<NodeKey>,
    book: Arc<dyn PeerBook>,
    admits: crate::remote::Admits,
    node_of: NodeOf,
) {
    let mut connections = tokio::task::JoinSet::new();
    loop {
        while connections.try_join_next().is_some() {}
        match listener.accept().await {
            Ok((stream, peer)) => {
                let (service, key, book) = (service.clone(), key.clone(), book.clone());
                connections.spawn(async move {
                    if !admits(peer.ip()).await {
                        tracing::warn!(%peer, "rpc: peer link: refused a peer outside the tailnet");
                        return;
                    }
                    let node = node_of(peer.ip()).await;
                    let _ = stream.set_nodelay(true);
                    serve_peer_socket(stream, service, key, book, node).await;
                });
            }
            Err(err) => {
                tracing::warn!(error = %err, "rpc: peer link: accept failed");
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }
}

/// What the handshake settled, for the caller's first frame.
struct Expected {
    caller: String,
    caller_nonce: String,
    callee_nonce: String,
    joining: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CallerFrame {
    peer_sig: String,
    #[serde(default)]
    join: Option<JoinInfo>,
}

async fn serve_peer_socket<S>(
    stream: S,
    service: Arc<dyn RpcService>,
    key: Arc<NodeKey>,
    book: Arc<dyn PeerBook>,
    node: Option<String>,
) where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    use futures::{SinkExt as _, StreamExt as _};
    use tokio_tungstenite::tungstenite::{Message, http::HeaderValue};

    let expected = std::sync::Mutex::new(None::<Expected>);
    #[allow(clippy::result_large_err)]
    let authorize = |req: &HandshakeRequest, mut resp: HandshakeResponse| {
        let refuse = |status: StatusCode, why: &str| {
            tracing::warn!(why, "rpc: peer link: refused a handshake");
            let mut err = ErrorResponse::new(Some(why.to_string()));
            *err.status_mut() = status;
            err
        };
        if req.headers().get("origin").is_some() {
            return Err(refuse(StatusCode::FORBIDDEN, "origin not allowed"));
        }
        let header = |name: &str| req.headers().get(name).and_then(|v| v.to_str().ok()).map(str::trim);
        let Some(caller) = header(KEY_HEADER).filter(|k| valid_key(k)) else {
            return Err(refuse(StatusCode::UNAUTHORIZED, "a server names its key"));
        };
        let Some(caller_nonce) = header(NONCE_HEADER).filter(|n| crate::link_proof::valid_nonce(n)) else {
            return Err(refuse(StatusCode::UNAUTHORIZED, "a server sends its challenge"));
        };
        let joining = match header(JOIN_HEADER) {
            Some(_) if node.is_none() => {
                return Err(refuse(StatusCode::UNAUTHORIZED, "can't tell which tailnet node this is: no pairing"));
            }
            Some(proof) => {
                if !book.take_code(proof, caller_nonce, caller) {
                    return Err(refuse(StatusCode::UNAUTHORIZED, "no pairing code matches: wrong, expired or used"));
                }
                true
            }
            None if book.grants(caller).is_some() => {
                if let Some(pinned) = book.node(caller)
                    && node.as_deref() != Some(pinned.as_str())
                {
                    return Err(refuse(StatusCode::UNAUTHORIZED, "this key comes from another tailnet node than the one it was paired from"));
                }
                false
            }
            None => return Err(refuse(StatusCode::UNAUTHORIZED, "unknown key: pair the servers first")),
        };
        let Ok(callee_nonce) = crate::link_proof::nonce() else {
            return Err(refuse(StatusCode::INTERNAL_SERVER_ERROR, "no randomness"));
        };
        let sig = key.sign(&transcript("callee", caller, key.public(), caller_nonce, &callee_nonce, joining));
        for (name, value) in [(KEY_HEADER, key.public()), (NONCE_HEADER, callee_nonce.as_str()), (SIG_HEADER, sig.as_str())] {
            let value = HeaderValue::from_str(value).map_err(|_| refuse(StatusCode::INTERNAL_SERVER_ERROR, "header"))?;
            resp.headers_mut().insert(name, value);
        }
        *expected.lock().unwrap() = Some(Expected {
            caller: caller.to_string(),
            caller_nonce: caller_nonce.to_string(),
            callee_nonce,
            joining,
        });
        Ok(resp)
    };
    let mut ws = match tokio_tungstenite::accept_hdr_async_with_config(stream, authorize, Some(crate::ws::config())).await {
        Ok(ws) => ws,
        Err(err) => {
            tracing::debug!(error = %err, "rpc: peer link: handshake failed");
            return;
        }
    };
    let Some(expected) = expected.into_inner().ok().flatten() else {
        return;
    };
    let first = tokio::time::timeout(PROOF_TIMEOUT, ws.next()).await;
    let frame = match &first {
        Ok(Some(Ok(Message::Text(frame)))) => serde_json::from_str::<CallerFrame>(frame).ok(),
        _ => None,
    };
    let signed = transcript(
        "caller",
        &expected.caller,
        key.public(),
        &expected.caller_nonce,
        &expected.callee_nonce,
        expected.joining,
    );
    let Some(frame) = frame.filter(|f| verify(&expected.caller, &signed, &f.peer_sig)) else {
        tracing::warn!(peer = %short(&expected.caller), "rpc: peer link: refused a server that didn't prove its key");
        let _ = ws.close(None).await;
        return;
    };
    if expected.joining {
        let reply = match book.joined(&expected.caller, frame.join.unwrap_or_default(), node.clone()) {
            Ok(me) => serde_json::json!({ "paired": me }),
            Err(error) => serde_json::json!({ "error": error }),
        };
        let _ = ws.send(Message::text(reply.to_string())).await;
        let _ = ws.close(None).await;
        return;
    }
    let Some(granted) = book.grants(&expected.caller) else {
        let _ = ws.close(None).await;
        return;
    };
    tracing::info!(peer = %short(&expected.caller), "rpc: peer link: connection");
    let mut changes = book.changes();
    let peer = expected.caller.clone();
    let guard: Arc<dyn RpcService> = Arc::new(PeerGuard { inner: service, book: book.clone(), peer: peer.clone() });
    let (out_tx, out_rx) = mpsc::channel::<String>(256);
    let (in_tx, in_rx) = mpsc::channel::<String>(256);
    let mut pump = tokio::spawn(async move {
        if let Some(reason) = crate::ws::pump(ws, out_rx, in_tx).await {
            tracing::info!(%reason, "rpc: peer link: connection lost");
        }
    });
    struct AbortOnDrop(tokio::task::AbortHandle);
    impl Drop for AbortOnDrop {
        fn drop(&mut self) {
            self.0.abort();
        }
    }
    let _pump_guard = AbortOnDrop(pump.abort_handle());
    // Its entry changed (grants, unpaired): it ends, and dials again for
    // what holds now.
    let changed = async {
        loop {
            if changes.changed().await.is_err() {
                std::future::pending::<()>().await;
            }
            if book.grants(&peer).as_ref() != Some(&granted) {
                tracing::info!(peer = %short(&peer), "rpc: peer link: its entry changed: cut");
                return;
            }
        }
    };
    tokio::select! {
        () = crate::server::serve_connection(guard, out_tx, in_rx) => {}
        _ = &mut pump => {}
        () = changed => {}
    }
}

// ---- dialing ----------------------------------------------------------------

type ClientWs = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// Dial `url` as `key`, expecting the server holding `peer_key`; the frame
/// to send once it proved that, signed.
async fn dial(url: &str, key: &NodeKey, peer_key: &str, secret: Option<&str>) -> Result<(ClientWs, String), RpcError> {
    let (request, caller_nonce) = caller_request(url, key, secret)?;
    let (ws, response) = tokio::time::timeout(
        DIAL_TIMEOUT,
        tokio_tungstenite::connect_async_with_config(request, Some(crate::ws::config()), true),
    )
    .await
    .map_err(|_| RpcError::Transport(format!("timed out dialing {url}")))?
    .map_err(|e| RpcError::Transport(e.to_string()))?;
    callee_proved(ws, &response, url, key, peer_key, &caller_nonce, secret.is_some()).await
}

/// The caller's handshake request to `url`: its key, a fresh nonce (also
/// returned) and, joining, its proof of the code's secret.
fn caller_request(
    url: &str,
    key: &NodeKey,
    secret: Option<&str>,
) -> Result<(tokio_tungstenite::tungstenite::handshake::client::Request, String), RpcError> {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    use tokio_tungstenite::tungstenite::http::HeaderValue;

    let transport = |e: &dyn std::fmt::Display| RpcError::Transport(e.to_string());
    let caller_nonce = crate::link_proof::nonce().map_err(|e| transport(&e))?;
    let mut request = url.into_client_request().map_err(|e| transport(&e))?;
    let mut headers = vec![(KEY_HEADER, key.public().to_string()), (NONCE_HEADER, caller_nonce.clone())];
    if let Some(secret) = secret {
        headers.push((JOIN_HEADER, join_proof(secret, &caller_nonce, key.public())));
    }
    for (name, value) in headers {
        request.headers_mut().insert(name, HeaderValue::from_str(&value).map_err(|e| transport(&e))?);
    }
    Ok((request, caller_nonce))
}

/// `ws` once the callee's handshake `response` proved `peer_key`, and the
/// caller's signature for its first frame; closed (nothing sent) if not.
async fn callee_proved<S>(
    mut ws: tokio_tungstenite::WebSocketStream<S>,
    response: &tokio_tungstenite::tungstenite::handshake::client::Response,
    url: &str,
    key: &NodeKey,
    peer_key: &str,
    caller_nonce: &str,
    joining: bool,
) -> Result<(tokio_tungstenite::WebSocketStream<S>, String), RpcError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let caller_nonce = caller_nonce.to_string();
    let header = |name: &str| response.headers().get(name).and_then(|v| v.to_str().ok()).map(str::to_owned);
    let proved = match (header(KEY_HEADER), header(NONCE_HEADER), header(SIG_HEADER)) {
        (Some(callee), Some(callee_nonce), Some(sig))
            if callee == peer_key && crate::link_proof::valid_nonce(&callee_nonce) =>
        {
            verify(peer_key, &transcript("callee", key.public(), peer_key, &caller_nonce, &callee_nonce, joining), &sig)
                .then_some(callee_nonce)
        }
        _ => None,
    };
    let Some(callee_nonce) = proved else {
        let _ = ws.close(None).await;
        return Err(RpcError::Transport(format!(
            "{url} didn't prove it holds the paired server's key (a wrong key, or not that server): nothing was sent"
        )));
    };
    let sig = key.sign(&transcript("caller", key.public(), peer_key, &caller_nonce, &callee_nonce, joining));
    Ok((ws, sig))
}

/// Dial a paired server at `url` (`ws://host:port`) as `key`, expecting
/// it to prove `peer_key` first. A server that can't (another key, not
/// that server) is an error, and nothing was sent to it.
pub async fn connect_peer(url: &str, key: &NodeKey, peer_key: &str) -> Result<RpcClient, RpcError> {
    use futures::SinkExt as _;
    use tokio_tungstenite::tungstenite::Message;

    let (mut ws, sig) = dial(url, key, peer_key, None).await?;
    ws.send(Message::text(serde_json::json!({ SIG_FIELD: sig }).to_string()))
        .await
        .map_err(|e| RpcError::Transport(e.to_string()))?;
    Ok(crate::client::client_on(ws, url))
}

/// Join the server whose code holds `peer_key` and `secret`, at `url`, as
/// `key`, saying `me` of this side: what it said of itself once it kept
/// this side's key.
pub async fn join_peer(url: &str, key: &NodeKey, peer_key: &str, secret: &str, me: JoinInfo) -> Result<JoinInfo, RpcError> {
    use futures::{SinkExt as _, StreamExt as _};
    use tokio_tungstenite::tungstenite::Message;

    let (mut ws, sig) = dial(url, key, peer_key, Some(secret)).await?;
    ws.send(Message::text(serde_json::json!({ SIG_FIELD: sig, "join": me }).to_string()))
        .await
        .map_err(|e| RpcError::Transport(e.to_string()))?;
    let reply = tokio::time::timeout(PROOF_TIMEOUT, ws.next()).await;
    let _ = ws.close(None).await;
    let value = match reply {
        Ok(Some(Ok(Message::Text(text)))) => serde_json::from_str::<serde_json::Value>(&text).ok(),
        _ => None,
    };
    match value {
        Some(value) if value.get("paired").is_some() => serde_json::from_value(value["paired"].clone())
            .map_err(|e| RpcError::Transport(format!("pairing reply: {e}"))),
        Some(value) => Err(RpcError::Failed(format!(
            "the server refused to pair: {}",
            value["error"].as_str().unwrap_or("no reason given")
        ))),
        None => Err(RpcError::Transport("the server ended the pairing without keeping this side".into())),
    }
}

// ---- a worker's socket -------------------------------------------------------

/// The URL a worker's socket is dialled as (the socket is the address; the
/// name only fills the handshake's Host).
pub const WORKER_URL: &str = "ws://zend-worker/";

/// Serve a worker's one server on `listener`, a unix socket only that
/// server's user reaches (a folder of its data dir, bound into the
/// worker's container): the same handshake as over the tailnet, only keys
/// in `book`, only what their grants open. No pairing here (no tailnet
/// node to pin a joiner to: refused), the worker's book is written by its
/// server when it makes the worker. Every connection lives in this future.
#[cfg(unix)]
pub async fn serve_peer_unix(
    listener: tokio::net::UnixListener,
    service: Arc<dyn RpcService>,
    key: Arc<NodeKey>,
    book: Arc<dyn PeerBook>,
) {
    let mut connections = tokio::task::JoinSet::new();
    loop {
        while connections.try_join_next().is_some() {}
        match listener.accept().await {
            Ok((stream, _)) => {
                connections.spawn(serve_peer_socket(stream, service.clone(), key.clone(), book.clone(), None));
            }
            Err(err) => {
                tracing::warn!(error = %err, "rpc: worker socket: accept failed");
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }
}

/// Dial the worker whose socket is `path` as `key`, expecting it to prove
/// `worker_key` (pinned when the worker was made) first: one that can't is
/// an error, and nothing was sent to it.
#[cfg(unix)]
pub async fn connect_peer_unix(path: &std::path::Path, key: &NodeKey, worker_key: &str) -> Result<RpcClient, RpcError> {
    use futures::SinkExt as _;
    use tokio_tungstenite::tungstenite::Message;

    let (request, caller_nonce) = caller_request(WORKER_URL, key, None)?;
    let dialled = tokio::time::timeout(DIAL_TIMEOUT, async {
        let stream = tokio::net::UnixStream::connect(path)
            .await
            .map_err(|e| RpcError::Transport(format!("{}: {e}", path.display())))?;
        tokio_tungstenite::client_async_with_config(request, stream, Some(crate::ws::config()))
            .await
            .map_err(|e| RpcError::Transport(e.to_string()))
    })
    .await
    .map_err(|_| RpcError::Transport(format!("timed out dialing {}", path.display())))??;
    let (ws, response) = dialled;
    let url = format!("unix:{}", path.display());
    let (mut ws, sig) = callee_proved(ws, &response, &url, key, worker_key, &caller_nonce, false).await?;
    ws.send(Message::text(serde_json::json!({ SIG_FIELD: sig }).to_string()))
        .await
        .map_err(|e| RpcError::Transport(e.to_string()))?;
    Ok(crate::client::client_on(ws, &url))
}

#[cfg(test)]
#[path = "peer_link_tests.rs"]
mod tests;
