//! Server to server: this engine paired with the owner's other Zend
//! servers, over the tailnet, with no edge between ([`zend_rpc::peer_link`]
//! for the wire, [`crate::rpc`]'s peer gate for what each call reaches).
//!
//! Kept in the data dir:
//! - `node-key`: this engine's Ed25519 key (PKCS#8, mode 600), made the
//!   first time it's needed and kept: a paired server knows this engine by
//!   its public half.
//! - `peers.json` (mode 600): `{peers: [{key, name?, host?, port?, grants,
//!   pairedAt}]}`, the servers paired with this one and what this side lets
//!   each do here (empty at first: nothing).
//!
//! Pairing codes live in memory only: one-time, 10 minutes.
//!
//! The peer listener binds only this device's tailnet addresses
//! ([`zend_rpc::bind_remote`]) on its own port (the IPC port + 1), apart from
//! the phone link (its own key and switch), and admits only nodes Tailscale
//! knows ([`crate::phone_link::tailnet_peer`]). It runs while any server is
//! paired, or a code is out.
//!
//! ```text
//! PeerPair       {}                     → {code, expiresAt, key, host, port}
//! PeerJoin       {code}                 → {peer}
//! ListPeers      {}                     → {key, port, listening, error?, peers}
//! SetPeerGrants  {peer, grants: [..]}   → {peer}
//! UnpairPeer     {peer}                 → {removed}
//! ```
//!
//! All five are this device's own (`zend_rpc::local_only`): never over a
//! peer link, the relay, a phone or from an agent's token. `peer` names a
//! paired server by its key, a key prefix of 8 or more, or its name.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::watch;
use zend_rpc::peer_link::{Grant, Grants, JoinInfo, NodeKey, PeerBook};
use zend_rpc::{RpcError, RpcReply, RpcService, methods};

use crate::phone_link::Tailnet;

/// The node key's file in the data dir.
pub const KEY_FILE: &str = "node-key";
/// The paired servers' file in the data dir.
pub const PEERS_FILE: &str = "peers.json";
/// How long a pairing code is good for.
pub const CODE_TTL: Duration = Duration::from_secs(10 * 60);
/// What a code starts with.
const CODE_PREFIX: &str = "zend-peer:1:";

/// One paired server, as this side keeps it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Peer {
    pub key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(default)]
    pub grants: Grants,
    #[serde(default)]
    pub paired_at: i64,
    /// The tailnet node it was paired from (Tailscale's stable node id): it is
    /// let in, and dialled, only there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Saved {
    #[serde(default)]
    peers: Vec<Peer>,
}

/// A pairing code: what the other side needs to dial and join this one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Code {
    pub key: String,
    pub host: String,
    pub port: u16,
    pub secret: String,
}

impl std::fmt::Display for Code {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{CODE_PREFIX}{}:{}:{}:{}", self.key, self.secret, self.port, self.host)
    }
}

impl std::str::FromStr for Code {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, String> {
        let bad = || "not a pairing code (zend-peer:1:…, from zend peer pair)".to_string();
        let rest = text.trim().strip_prefix(CODE_PREFIX).ok_or_else(bad)?;
        let mut parts = rest.splitn(4, ':');
        let (Some(key), Some(secret), Some(port), Some(host)) = (parts.next(), parts.next(), parts.next(), parts.next()) else {
            return Err(bad());
        };
        if !zend_rpc::peer_link::valid_key(key) || secret.len() < 32 || host.is_empty() {
            return Err(bad());
        }
        let port = port.parse().map_err(|_| bad())?;
        Ok(Code { key: key.into(), host: host.into(), port, secret: secret.into() })
    }
}

impl Code {
    /// Where to dial it.
    pub fn url(&self) -> String {
        match self.host.parse::<std::net::IpAddr>() {
            Ok(std::net::IpAddr::V6(ip)) => format!("ws://[{ip}]:{}", self.port),
            _ => format!("ws://{}:{}", self.host, self.port),
        }
    }
}

type Found = futures::future::BoxFuture<'static, Option<Tailnet>>;
type Bound = futures::future::BoxFuture<'static, std::io::Result<tokio::net::TcpListener>>;

/// How the link finds the tailnet, binds and admits: the real ones, or a
/// test's (loopback).
#[derive(Clone, Copy)]
pub struct Seams {
    pub discover: fn() -> Found,
    pub bind: fn(SocketAddr) -> Bound,
    pub admits: zend_rpc::remote::Admits,
    pub node_of: zend_rpc::peer_link::NodeOf,
}

impl Seams {
    pub fn real() -> Self {
        Self {
            discover: || Box::pin(crate::phone_link::discover()),
            bind: |addr| Box::pin(zend_rpc::bind_remote(addr)),
            admits: crate::phone_link::tailnet_peer,
            node_of: crate::phone_link::tailnet_node,
        }
    }
}

#[derive(Default)]
struct Running {
    listeners: Vec<tokio::task::JoinHandle<()>>,
    listening: Vec<SocketAddr>,
    error: Option<String>,
}

impl Running {
    fn stop(&mut self) {
        for listener in self.listeners.drain(..) {
            listener.abort();
        }
        self.listening.clear();
    }
}

/// The book the listener asks: the peers and open codes of one engine.
struct Book {
    data_dir: PathBuf,
    peers: Mutex<Vec<Peer>>,
    codes: Mutex<Vec<(String, Instant)>>,
    changes: watch::Sender<u64>,
    me: Mutex<JoinInfo>,
}

impl Book {
    fn load(data_dir: &Path) -> Self {
        let saved: Saved = std::fs::read(data_dir.join(PEERS_FILE))
            .ok()
            .and_then(|raw| serde_json::from_slice(&raw).ok())
            .unwrap_or_default();
        Self {
            data_dir: data_dir.to_path_buf(),
            peers: Mutex::new(saved.peers),
            codes: Mutex::default(),
            changes: watch::channel(0).0,
            me: Mutex::default(),
        }
    }

    fn save(&self, peers: &[Peer]) -> Result<(), String> {
        let bytes = serde_json::to_vec_pretty(&Saved { peers: peers.to_vec() }).map_err(|e| e.to_string())?;
        crate::runner::write_private_atomic(&self.data_dir.join(PEERS_FILE), &bytes)
            .map_err(|err| format!("couldn't keep the paired servers: {err}"))
    }

    /// Change the peers, keep them, and tell the listeners.
    fn change<R>(&self, f: impl FnOnce(&mut Vec<Peer>) -> Result<R, String>) -> Result<R, String> {
        let result = {
            let mut peers = self.peers.lock().unwrap();
            let mut next = peers.clone();
            let result = f(&mut next)?;
            self.save(&next)?;
            *peers = next;
            result
        };
        self.changes.send_modify(|tick| *tick += 1);
        Ok(result)
    }

    fn list(&self) -> Vec<Peer> {
        self.peers.lock().unwrap().clone()
    }
}

impl PeerBook for Book {
    fn grants(&self, key: &str) -> Option<Grants> {
        self.peers.lock().unwrap().iter().find(|p| p.key == key).map(|p| p.grants.clone())
    }

    fn take_code(&self, proof: &str, caller_nonce: &str, caller_key: &str) -> bool {
        let mut codes = self.codes.lock().unwrap();
        let matched = codes.iter().position(|(secret, _)| {
            zend_rpc::peer_link::proof_matches(&zend_rpc::peer_link::join_proof(secret, caller_nonce, caller_key), proof)
        });
        let Some(i) = matched else { return false };
        let (_, expires) = codes.remove(i);
        Instant::now() < expires
    }

    fn node(&self, key: &str) -> Option<String> {
        self.peers.lock().unwrap().iter().find(|p| p.key == key).and_then(|p| p.node.clone())
    }

    fn joined(&self, key: &str, info: JoinInfo, node: Option<String>) -> Result<JoinInfo, String> {
        let peer = Peer {
            node,
            key: key.to_string(),
            name: info.name.filter(|n| !n.trim().is_empty()).map(|n| n.chars().take(80).collect()),
            host: info.host.filter(|h| h.len() <= 253),
            port: info.port,
            grants: Grants::default(),
            paired_at: crate::now_ms(),
        };
        self.change(|peers| {
            peers.retain(|p| p.key != key);
            peers.push(peer);
            Ok(())
        })?;
        tracing::info!(peer = %&key[..12], "peer link: a server joined with a code");
        Ok(self.me.lock().unwrap().clone())
    }

    fn changes(&self) -> watch::Receiver<u64> {
        self.changes.subscribe()
    }
}

/// The peer link of one engine.
pub struct PeerLink {
    data_dir: PathBuf,
    port: u16,
    service: Arc<dyn RpcService>,
    seams: Seams,
    book: Arc<Book>,
    key: Mutex<Option<Arc<NodeKey>>>,
    running: tokio::sync::Mutex<Running>,
}

impl Drop for PeerLink {
    fn drop(&mut self) {
        self.running.get_mut().stop();
    }
}

impl PeerLink {
    /// The link of the engine serving `service` from `data_dir`, listening on
    /// `port` once it's needed.
    pub fn new(data_dir: &Path, port: u16, service: Arc<dyn RpcService>, seams: Seams) -> Arc<Self> {
        Arc::new(Self {
            data_dir: data_dir.to_path_buf(),
            port,
            service,
            seams,
            book: Arc::new(Book::load(data_dir)),
            key: Mutex::default(),
            running: tokio::sync::Mutex::new(Running::default()),
        })
    }

    /// This engine's node key, made and kept (mode 600) the first time.
    pub fn node_key(&self) -> Result<Arc<NodeKey>, String> {
        let mut held = self.key.lock().unwrap();
        if let Some(key) = held.as_ref() {
            return Ok(key.clone());
        }
        let path = self.data_dir.join(KEY_FILE);
        let key = match std::fs::read(&path) {
            Ok(der) => NodeKey::from_pkcs8(&der).map_err(|e| format!("{}: {e}", path.display()))?,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                let der = NodeKey::generate_pkcs8()?;
                crate::runner::write_private_atomic(&path, &der)
                    .map_err(|err| format!("couldn't keep the node key: {err}"))?;
                NodeKey::from_pkcs8(&der)?
            }
            Err(err) => return Err(format!("{}: {err}", path.display())),
        };
        let key = Arc::new(key);
        *held = Some(key.clone());
        Ok(key)
    }

    /// The servers paired with this one.
    pub fn peers(&self) -> Vec<Peer> {
        self.book.list()
    }

    /// Listen as kept: while any server is paired (Tailscale may come up
    /// after the engine: tried again for a while).
    pub fn resume(self: &Arc<Self>) {
        if self.book.list().is_empty() {
            return;
        }
        let link = Arc::downgrade(self);
        tokio::spawn(async move {
            for attempt in 0..20u32 {
                let Some(link) = link.upgrade() else { return };
                if link.ensure_listening().await.is_ok() {
                    return;
                }
                drop(link);
                tokio::time::sleep(Duration::from_secs(if attempt < 5 { 5 } else { 30 })).await;
            }
        });
    }

    /// The listeners, started if they aren't: where this engine is reached.
    async fn ensure_listening(&self) -> Result<Vec<SocketAddr>, String> {
        let mut running = self.running.lock().await;
        if !running.listening.is_empty() {
            return Ok(running.listening.clone());
        }
        running.error = None;
        let key = self.node_key()?;
        let Some(tailnet) = (self.seams.discover)().await else {
            running.error = Some("no_tailnet".into());
            return Err("no_tailnet: this device isn't on a tailnet (tailscale status)".into());
        };
        let mut failed = Vec::new();
        for ip in &tailnet.ips {
            let addr = SocketAddr::new(*ip, self.port);
            match (self.seams.bind)(addr).await {
                Ok(listener) => {
                    let addr = listener.local_addr().unwrap_or(addr);
                    tracing::info!(%addr, "peer link: listening");
                    running.listening.push(addr);
                    running.listeners.push(tokio::spawn(zend_rpc::peer_link::serve_peer_listener(
                        listener,
                        self.service.clone(),
                        key.clone(),
                        self.book.clone(),
                        self.seams.admits,
                        self.seams.node_of,
                    )));
                }
                Err(err) => failed.push(format!("{addr}: {err}")),
            }
        }
        if running.listening.is_empty() {
            let error = format!("bind: {}", failed.join("; "));
            running.error = Some(error.clone());
            return Err(error);
        }
        let first = running.listening.iter().find(|a| a.is_ipv4()).or(running.listening.first()).copied();
        *self.book.me.lock().unwrap() = JoinInfo {
            name: Some(self.name()),
            host: first.map(|a| a.ip().to_string()),
            port: first.map(|a| a.port()),
        };
        Ok(running.listening.clone())
    }

    fn name(&self) -> String {
        let device = std::fs::read_to_string(self.data_dir.join("device-id")).unwrap_or_default();
        crate::local_device_name(device.trim())
    }

    /// A pairing code, good for `CODE_TTL`, once.
    pub async fn pair(&self) -> Result<Value, RpcError> {
        let listening = self.ensure_listening().await.map_err(RpcError::Failed)?;
        let key = self.node_key().map_err(RpcError::Failed)?;
        let addr = listening.iter().find(|a| a.is_ipv4()).or(listening.first()).copied().expect("listening");
        let secret = zend_rpc::ipc_auth::generate_token().map_err(|e| RpcError::Failed(e.to_string()))?;
        let expires = Instant::now() + CODE_TTL;
        {
            let mut codes = self.book.codes.lock().unwrap();
            let now = Instant::now();
            codes.retain(|(_, until)| *until > now);
            codes.push((secret.clone(), expires));
        }
        let code = Code { key: key.public().to_string(), host: addr.ip().to_string(), port: addr.port(), secret };
        Ok(json!({
            "code": code.to_string(),
            "expiresAt": crate::now_ms() + CODE_TTL.as_millis() as i64,
            "key": key.public(),
            "host": code.host,
            "port": code.port,
        }))
    }

    /// Join the server whose code this is: both then hold each other's key.
    pub async fn join(&self, code: &str) -> Result<Value, RpcError> {
        let code: Code = code.parse().map_err(RpcError::BadParams)?;
        let key = self.node_key().map_err(RpcError::Failed)?;
        if code.key == key.public() {
            return Err(RpcError::BadParams("that code is this server's own".into()));
        }
        // Where the other side can dial this one back, if it can listen.
        let _ = self.ensure_listening().await;
        let me = self.book.me.lock().unwrap().clone();
        let me = JoinInfo { name: Some(self.name()), ..me };
        let node = self.node_at(&code.host).await.ok_or_else(|| {
            RpcError::Failed(format!("can't tell which tailnet node {} is (tailscale whois): not paired", code.host))
        })?;
        let them = zend_rpc::peer_link::join_peer(&code.url(), &key, &code.key, &code.secret, me).await?;
        let peer = Peer {
            node: Some(node),
            key: code.key.clone(),
            name: them.name,
            host: Some(code.host.clone()),
            port: Some(code.port),
            grants: Grants::default(),
            paired_at: crate::now_ms(),
        };
        self.book
            .change(|peers| {
                peers.retain(|p| p.key != peer.key);
                peers.push(peer.clone());
                Ok(())
            })
            .map_err(RpcError::Failed)?;
        Ok(json!({ "peer": peer }))
    }

    /// The tailnet node at `host` (an address).
    async fn node_at(&self, host: &str) -> Option<String> {
        let ip = host.parse::<std::net::IpAddr>().ok()?;
        (self.seams.node_of)(ip).await
    }

    /// Dial the paired server `peer` names, where it said it listens, only
    /// when that address is still the tailnet node it was paired from.
    pub async fn connect(&self, peer: &str) -> Result<zend_rpc::RpcClient, RpcError> {
        let key = self.find(peer)?;
        let peer = self.book.list().into_iter().find(|p| p.key == key).ok_or_else(|| RpcError::Failed("unpaired".into()))?;
        let (Some(host), Some(port)) = (peer.host.clone(), peer.port) else {
            return Err(RpcError::Failed("that server said nowhere to dial it".into()));
        };
        let here = self.node_at(&host).await;
        if peer.node.is_none() || here != peer.node {
            return Err(RpcError::Failed(format!(
                "{host} is another tailnet node than the one {} was paired from: not dialled",
                peer.name.as_deref().unwrap_or("that server")
            )));
        }
        let code = Code { key: key.clone(), host, port, secret: String::new() };
        let own = self.node_key().map_err(RpcError::Failed)?;
        zend_rpc::connect_peer(&code.url(), &own, &key).await
    }

    /// The one paired server `name` names (key, key prefix of 8+, or name).
    fn find(&self, name: &str) -> Result<String, RpcError> {
        let name = name.trim();
        let peers = self.book.list();
        let matches: Vec<&Peer> = peers
            .iter()
            .filter(|p| p.key == name || (name.len() >= 8 && p.key.starts_with(name)) || p.name.as_deref() == Some(name))
            .collect();
        match matches.as_slice() {
            [peer] => Ok(peer.key.clone()),
            [] => Err(RpcError::BadParams(format!("no paired server {name}"))),
            _ => Err(RpcError::BadParams(format!("{name} names more than one paired server: give its key"))),
        }
    }

    /// What this side lets `peer` do here, replaced whole. Its open connections end.
    pub fn set_grants(&self, peer: &str, grants: &[String]) -> Result<Value, RpcError> {
        let key = self.find(peer)?;
        let grants = Grants::new(
            grants.iter().map(|g| g.parse::<Grant>()).collect::<Result<Vec<_>, _>>().map_err(RpcError::BadParams)?,
        );
        let peer = self
            .book
            .change(|peers| {
                let peer = peers.iter_mut().find(|p| p.key == key).ok_or("no such peer")?;
                peer.grants = grants;
                Ok(peer.clone())
            })
            .map_err(RpcError::Failed)?;
        grants_changed(&self.data_dir);
        Ok(json!({ "peer": peer }))
    }

    /// Forget `peer`: its connections end, its dials are refused.
    pub fn unpair(&self, peer: &str) -> Result<Value, RpcError> {
        let key = self.find(peer)?;
        self.book
            .change(|peers| {
                peers.retain(|p| p.key != key);
                Ok(())
            })
            .map_err(RpcError::Failed)?;
        grants_changed(&self.data_dir);
        Ok(json!({ "removed": key }))
    }

    pub async fn status(&self) -> Result<Value, RpcError> {
        let key = self.node_key().map_err(RpcError::Failed)?;
        let running = self.running.lock().await;
        Ok(json!({
            "key": key.public(),
            "port": running.listening.first().map_or(self.port, SocketAddr::port),
            "listening": running.listening.iter().map(ToString::to_string).collect::<Vec<_>>(),
            "error": running.error,
            "peers": self.book.list(),
        }))
    }
}

#[derive(Deserialize)]
struct JoinParams {
    code: String,
}

#[derive(Deserialize)]
struct PeerParams {
    peer: String,
}

#[derive(Deserialize)]
struct GrantsParams {
    peer: String,
    grants: Vec<String>,
}

/// `inner` with the peer link's methods answered by `link` (the rest passed on).
pub struct PeerLinkRpc {
    pub inner: Arc<dyn RpcService>,
    pub link: Arc<PeerLink>,
}

#[async_trait]
impl RpcService for PeerLinkRpc {
    async fn handle(&self, method: &str, params: Value) -> Result<RpcReply, RpcError> {
        let given = || if params.is_null() { json!({}) } else { params.clone() };
        match method {
            methods::PEER_PAIR | methods::PEER_JOIN | methods::LIST_PEERS | methods::SET_PEER_GRANTS | methods::UNPAIR_PEER
                if !zend_rpc::caller().is_local() =>
            {
                Err(RpcError::Failed(format!("{method} is this device's own")))
            }
            methods::PEER_PAIR => self.link.pair().await.map(RpcReply::Value),
            methods::PEER_JOIN => {
                let p: JoinParams = zend_rpc::parse_params(given())?;
                self.link.join(&p.code).await.map(RpcReply::Value)
            }
            methods::LIST_PEERS => self.link.status().await.map(RpcReply::Value),
            methods::SET_PEER_GRANTS => {
                let p: GrantsParams = zend_rpc::parse_params(given())?;
                self.check_shares(&p.grants).await?;
                self.link.set_grants(&p.peer, &p.grants).map(RpcReply::Value)
            }
            methods::UNPAIR_PEER => {
                let p: PeerParams = zend_rpc::parse_params(given())?;
                self.link.unpair(&p.peer).map(RpcReply::Value)
            }
            _ => self.inner.handle(method, params).await,
        }
    }
}

impl PeerLinkRpc {
    /// Refuse sharing a project whose folder is, holds or sits in Zend's
    /// data folder, holds the home folder or sits in a hidden folder of it
    /// ([`crate::peer_files::unshareable`]; checked again at every read).
    async fn check_shares(&self, grants: &[String]) -> Result<(), RpcError> {
        use futures::StreamExt as _;
        let shared: Vec<String> = grants
            .iter()
            .filter_map(|g| match g.parse::<Grant>() {
                Ok(Grant::ProjectsShare(project)) => Some(project),
                _ => None,
            })
            .collect();
        if shared.is_empty() {
            return Ok(());
        }
        // The published project list lags a project made a moment ago: wait
        // (5 s at most) until each named one shows, and refuse one that
        // doesn't (unchecked is never shared).
        let parse = |rows: Value| serde_json::from_value::<Vec<zend_proto::Space>>(rows).unwrap_or_default();
        let has_all = |spaces: &[zend_proto::Space]| shared.iter().all(|p| spaces.iter().any(|s| &s.id == p));
        let spaces = match self.inner.handle(methods::WATCH_SPACES, json!({})).await {
            Ok(RpcReply::Stream(mut rows)) => {
                let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
                let mut last = Vec::new();
                while !has_all(&last) {
                    match tokio::time::timeout_at(deadline, rows.next()).await {
                        Ok(Some(next)) => last = parse(next),
                        _ => break,
                    }
                }
                last
            }
            Ok(RpcReply::Value(rows)) => parse(rows),
            Err(_) => Vec::new(),
        };
        let home = crate::peer_files::home();
        for project in shared {
            let Some(space) = spaces.iter().find(|s| s.id == project) else {
                return Err(RpcError::BadParams(format!("projects.share:{project}: never shared: no such project on this device")));
            };
            if let Some(why) = crate::peer_files::unshareable(Path::new(&space.path), &self.link.data_dir, home.as_deref()) {
                return Err(RpcError::BadParams(format!("projects.share:{project}: never shared: {why}")));
            }
        }
        Ok(())
    }
}

/// What runs when the grants kept in a data folder change (or a server is
/// unpaired), by that folder: each returns false once its owner is gone
/// (and is dropped then).
type GrantsWatcher = (PathBuf, Box<dyn Fn() -> bool + Send + Sync>);
static GRANTS_WATCHERS: Mutex<Vec<GrantsWatcher>> = Mutex::new(Vec::new());

fn same_dir(a: &Path, b: &Path) -> bool {
    a == b || std::fs::canonicalize(a).ok().is_some_and(|a| std::fs::canonicalize(b).ok() == Some(a))
}

/// Call `watcher` whenever the grants kept in `data_dir` change or a
/// server is unpaired there (the sessions engine ends the runs a server
/// lost: [`crate::sessions::SessionsEngine::end_ungranted_peer_runs`]).
pub fn on_grants_changed(data_dir: &Path, watcher: impl Fn() -> bool + Send + Sync + 'static) {
    GRANTS_WATCHERS.lock().unwrap_or_else(|e| e.into_inner()).push((data_dir.to_path_buf(), Box::new(watcher)));
}

fn grants_changed(data_dir: &Path) {
    GRANTS_WATCHERS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .retain(|(dir, watcher)| !same_dir(dir, data_dir) || watcher());
}

/// The grants kept in `data_dir` for the server holding `key`, as they are
/// now (read from peers.json), if it's paired.
pub fn kept_grants(data_dir: &Path, key: &str) -> Option<Grants> {
    Book::load(data_dir).grants(key)
}

/// The peer port of an engine on IPC port `ipc_port`.
pub fn peer_port(ipc_port: u16) -> u16 {
    ipc_port.checked_add(1).unwrap_or(ipc_port)
}

/// `service` with a peer link of its own on `port`, listening if a server is
/// paired. The link serves `service` itself (what a server may call of it is
/// its grants': [`zend_rpc::peer_server_allowed`]).
pub fn wrap(service: Arc<dyn RpcService>, data_dir: &Path, port: u16) -> Arc<dyn RpcService> {
    let link = PeerLink::new(data_dir, port, service.clone(), Seams::real());
    link.resume();
    Arc::new(PeerLinkRpc { inner: service, link })
}

#[cfg(test)]
#[path = "peer_link_tests.rs"]
mod tests;
