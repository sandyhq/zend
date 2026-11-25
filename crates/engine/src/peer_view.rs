//! The paired servers this engine dials, shown to its own window as devices
//! (plan step 4e): their chats, sessions and projects merged into the
//! window's lists, and the calls that name one of them (its device, a chat
//! or project of its) sent to it over the peer link, under the grants *it*
//! gave this side. Nothing here is served to anyone: the lists merge only
//! for this device's own callers ([`zend_rpc::Caller::Local`]), and a paired
//! server calling here still sees only this device's own rows
//! (`rpc::peer_gate`).
//!
//! Per paired server (one with an address to dial and a tailnet node):
//! dial it ([`PeerLink::connect`]), ask `EngineInfo` (its device id, and
//! the projects its grants name; refused, it grants this side nothing and
//! shows nowhere), then follow `WatchChats` and `WatchSessions` (refused
//! without `sessions.read`: no chats shown). Lost, it is dialled again, waiting
//! longer each time; its rows stay, its device goes quiet (shown offline).
//!
//! What a server says is display, never authority over this device:
//! - This device's own ids come first. A call naming a chat, project or
//!   device this engine has (followed from its own lists) is never sent to
//!   a server, and a server's row whose chat, session or project id is this
//!   device's or another server's is dropped as it arrives (logged once).
//! - A server whose device id is this device's, one of its workspace's, a
//!   Cloud one's or another paired server's is refused (of two servers with
//!   one id, the first paired keeps it).
//! - Every row it sends is stamped with its device id, and loses what names
//!   a folder of its (an absolute or escaping `cwd`, the harness's resume
//!   folder, the checkout): the window never takes a server's row for a
//!   chat or file of this device's.
//! - Its messages are capped at 4 MiB (`zend_rpc::connect_peer`), each list
//!   at [`MAX_ROWS`] rows: a frame over that is dropped.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use futures::StreamExt as _;
use serde_json::{Value, json};
use tokio::sync::{Notify, watch};
use zend_rpc::{RpcClient, RpcError, RpcReply, RpcService, RpcSubscription, methods};

use crate::peer_link::PeerLink;

/// How often a connected server's device row is stamped seen (the window
/// shows a device online for 70 s after its last stamp).
const HEARTBEAT: Duration = Duration::from_secs(20);
/// How often the paired servers are looked at again (one paired or
/// unpaired since; [`PeerView::refresh`] at once).
const SCAN: Duration = Duration::from_secs(10);
const RETRY_MIN: Duration = Duration::from_secs(2);
const RETRY_MAX: Duration = Duration::from_secs(60);
/// How soon a server that granted this side nothing is asked again (it
/// may grant something any time; it doesn't say so).
const REFUSED_RETRY: Duration = SCAN;
/// The most rows a server's list may hold (chats, sessions, projects); a
/// frame over it is dropped, and what it showed before stays.
pub const MAX_ROWS: usize = 5000;
/// The one capability a server's device row carries: none of its own (the
/// window reads a device's capabilities to offer what isn't open over the
/// peer link, agent updates, or to take it for Cloud's).
pub const PAIRED_SERVER: &str = "paired-server";

/// What an error from a paired server starts with: its words follow, for
/// the window to show (`apps/zgx` sessions.create).
pub const REFUSED_PREFIX: &str = "peer_refused:";

/// One paired server, as last heard.
#[derive(Clone, Debug, Default)]
struct Seen {
    device_id: String,
    /// Its device row (`zend_proto::Device`).
    device: Value,
    /// The projects its grants name, as it said (`EngineInfo.peer.projects`).
    projects: Vec<Value>,
    chats: Vec<Value>,
    sessions: Vec<Value>,
    /// Project rows (`zend_proto::Space`): those it named, and those its chats are in.
    spaces: Vec<Value>,
    /// When this side first heard it (the project rows' `createdAt`).
    since: String,
    /// When it was paired (of two with one device id, the first keeps it).
    paired_at: i64,
    /// The config each chat made from here was made with, once its list
    /// has it (see [`PeerView::made`]).
    configs: HashMap<String, Value>,
}

impl Seen {
    fn rebuild_spaces(&mut self) {
        let mut spaces: Vec<Value> = Vec::new();
        let mut ids = HashSet::new();
        let named = self.projects.iter().filter_map(|p| Some((p["id"].as_str()?.to_string(), p["name"].as_str().map(str::to_string))));
        let in_chats = self.chats.iter().filter_map(|c| Some((c["spaceId"].as_str()?.to_string(), None)));
        for (id, name) in named.chain(in_chats) {
            if id.is_empty() || !ids.insert(id.clone()) {
                continue;
            }
            let name = name.unwrap_or_else(|| format!("Project {}", id.chars().take(8).collect::<String>()));
            spaces.push(json!({
                "id": id,
                "deviceId": self.device_id,
                // Its folder is the server's own: never said.
                "path": "",
                "name": name,
                "gitDetected": false,
                "createdAt": self.since,
            }));
        }
        self.spaces = spaces;
    }

    fn rows(&self, list: List) -> &[Value] {
        match list {
            List::Devices => std::slice::from_ref(&self.device),
            List::Chats => &self.chats,
            List::Sessions => &self.sessions,
            List::Spaces => &self.spaces,
        }
    }
}

/// A list the window watches.
#[derive(Clone, Copy, Debug)]
enum List {
    Devices,
    Chats,
    Sessions,
    Spaces,
}

impl List {
    fn of(method: &str) -> Option<Self> {
        Some(match method {
            methods::WATCH_DEVICES => List::Devices,
            methods::WATCH_CHATS => List::Chats,
            methods::WATCH_SESSIONS => List::Sessions,
            methods::WATCH_SPACES => List::Spaces,
            _ => return None,
        })
    }

    fn key(self) -> &'static str {
        match self {
            List::Sessions => "chatId",
            _ => "id",
        }
    }

    fn bit(self) -> u8 {
        match self {
            List::Devices => 1,
            List::Chats => 2,
            List::Sessions => 4,
            List::Spaces => 8,
        }
    }

    fn method(self) -> &'static str {
        match self {
            List::Devices => methods::WATCH_DEVICES,
            List::Chats => methods::WATCH_CHATS,
            List::Sessions => methods::WATCH_SESSIONS,
            List::Spaces => methods::WATCH_SPACES,
        }
    }
}

/// A connected server: where to send its calls.
struct Live {
    client: Arc<RpcClient>,
}

/// The paired servers this engine dials, for its own window.
pub struct PeerView {
    link: Weak<PeerLink>,
    /// Heard of each server, by its key.
    seen: watch::Sender<BTreeMap<String, Seen>>,
    live: Mutex<HashMap<String, Live>>,
    tasks: Mutex<HashMap<String, tokio::task::JoinHandle<()>>>,
    /// Chats made on a server from here: its key and the config they were
    /// made with (known before its list has them).
    made: Mutex<HashMap<String, (String, Value)>>,
    /// This device's own ids, from its own lists.
    locals: watch::Sender<Locals>,
    /// What was dropped of a server's, logged once.
    logged: Mutex<HashSet<String>>,
    wake: Arc<Notify>,
}

/// This device's own ids (its engine's lists, followed), checked before any
/// server's.
#[derive(Debug, Default)]
struct Locals {
    /// Which of the four lists have been heard (one bit each).
    heard: u8,
    own: String,
    devices: HashSet<String>,
    chats: HashSet<String>,
    sessions: HashSet<String>,
    spaces: HashSet<String>,
    /// Chats this device's window made here, before its list has them.
    made: HashSet<String>,
}

impl Locals {
    fn ready(&self) -> bool {
        self.heard == 0b1111 && !self.own.is_empty()
    }

    fn device(&self, id: &str) -> bool {
        id == self.own || self.devices.contains(id)
    }

    fn chat(&self, id: &str) -> bool {
        self.chats.contains(id) || self.sessions.contains(id) || self.made.contains(id)
    }

    fn set(&mut self, list: List, rows: &Value) {
        let ids: HashSet<String> =
            rows.as_array().into_iter().flatten().filter_map(|r| r[list.key()].as_str().map(str::to_string)).collect();
        match list {
            List::Devices => self.devices = ids,
            List::Chats => {
                self.made.retain(|c| !ids.contains(c));
                self.chats = ids;
            }
            List::Sessions => self.sessions = ids,
            List::Spaces => self.spaces = ids,
        }
        self.heard |= list.bit();
    }
}

/// Ids a server's rows may not take: this device's and the other servers'.
#[derive(Debug, Default)]
struct Taken {
    chats: HashSet<String>,
    spaces: HashSet<String>,
}

impl PeerView {
    pub(crate) fn new(link: Weak<PeerLink>) -> Arc<Self> {
        Arc::new(Self {
            link,
            seen: watch::Sender::new(BTreeMap::new()),
            live: Mutex::default(),
            tasks: Mutex::default(),
            made: Mutex::default(),
            locals: watch::Sender::new(Locals::default()),
            logged: Mutex::default(),
            wake: Arc::new(Notify::new()),
        })
    }

    /// Follow the paired servers (looked at again every [`SCAN`]).
    pub(crate) fn start(self: &Arc<Self>) {
        if let Some(link) = self.link.upgrade() {
            tokio::spawn(follow_locals(Arc::downgrade(self), link.service()));
        }
        let view = Arc::downgrade(self);
        tokio::spawn(async move {
            loop {
                let Some(me) = view.upgrade() else { return };
                if !me.scan() {
                    return;
                }
                let wake = me.wake.clone();
                drop(me);
                tokio::select! {
                    () = tokio::time::sleep(SCAN) => {}
                    () = wake.notified() => {}
                }
            }
        });
    }

    /// Look at the paired servers now, and dial the quiet ones again.
    pub fn refresh(&self) {
        self.wake.notify_waiters();
    }

    /// The paired servers' device ids, as heard (for the tests).
    pub fn devices(&self) -> Vec<String> {
        self.seen.borrow().values().map(|s| s.device_id.clone()).collect()
    }

    /// How many chats made on a server from here wait for its list to have
    /// them, and how many configs are kept for those it has (for the tests).
    pub fn made_counts(&self) -> (usize, usize) {
        (self.made.lock().unwrap().len(), self.seen.borrow().values().map(|s| s.configs.len()).sum())
    }

    /// Start following new servers, stop following unpaired ones. False
    /// once the link is gone.
    fn scan(self: &Arc<Self>) -> bool {
        let Some(link) = self.link.upgrade() else {
            return false;
        };
        let dialable: HashSet<String> = link
            .peers()
            .into_iter()
            .filter(|p| p.host.is_some() && p.port.is_some() && p.node.is_some())
            .map(|p| p.key)
            .collect();
        let mut tasks = self.tasks.lock().unwrap();
        tasks.retain(|key, task| {
            let keep = dialable.contains(key) && !task.is_finished();
            if !keep {
                task.abort();
            }
            keep
        });
        let gone: Vec<String> = self.seen.borrow().keys().filter(|k| !dialable.contains(*k)).cloned().collect();
        for key in &gone {
            self.forget(key);
        }
        for key in dialable {
            if !tasks.contains_key(&key) {
                let me = self.clone();
                let task_key = key.clone();
                tasks.insert(key, tokio::spawn(async move { me.follow(task_key).await }));
            }
        }
        true
    }

    fn forget(&self, key: &str) {
        self.live.lock().unwrap().remove(key);
        self.made.lock().unwrap().retain(|_, (k, _)| k != key);
        self.seen.send_if_modified(|seen| seen.remove(key).is_some());
    }

    /// Log `what` of server `key` once.
    fn note(&self, key: &str, what: &str) {
        if self.logged.lock().unwrap().insert(format!("{key} {what}")) {
            tracing::warn!(peer = %key, %what, "peer view: dropped what a paired server said");
        }
    }

    /// The ids server `key`'s rows may not take.
    fn taken(&self, key: &str) -> Taken {
        let locals = self.locals.borrow();
        let mut taken = Taken {
            chats: locals.chats.iter().chain(&locals.sessions).chain(&locals.made).cloned().collect(),
            spaces: locals.spaces.clone(),
        };
        drop(locals);
        for s in self.seen.borrow().iter().filter(|(k, _)| k.as_str() != key).map(|(_, s)| s) {
            taken.chats.extend(s.chats.iter().chain(&s.sessions).filter_map(|r| r["id"].as_str().or(r["chatId"].as_str()).map(str::to_string)));
            taken.spaces.extend(s.spaces.iter().filter_map(|p| p["id"].as_str().map(str::to_string)));
        }
        taken.chats.extend(self.made.lock().unwrap().iter().filter(|(_, (k, _))| k != key).map(|(c, _)| c.clone()));
        taken
    }

    /// Server `key`'s `rows` of `what`, as this side shows them (see
    /// [`clean`]); `None` over [`MAX_ROWS`].
    fn admit(&self, key: &str, device_id: &str, what: Kind, rows: &Value) -> Option<Vec<Value>> {
        let rows = rows.as_array().map(Vec::as_slice).unwrap_or_default();
        if rows.len() > MAX_ROWS {
            tracing::warn!(peer = %key, ?what, rows = rows.len(), "peer view: a paired server's list is over the cap; frame dropped");
            return None;
        }
        let mut dropped = Vec::new();
        let kept = clean(what, rows, device_id, &self.taken(key), &mut dropped);
        for d in dropped {
            self.note(key, &d);
        }
        Some(kept)
    }

    /// Why server `key` (paired at `paired_at`) may not show as
    /// `device_id`, else the servers it takes that id from.
    fn claim(&self, seen: &BTreeMap<String, Seen>, key: &str, device_id: &str, paired_at: i64) -> Result<Vec<String>, String> {
        if zend_proto::is_cloud_device(device_id, "") {
            return Err(format!("it says it is {device_id}, a Cloud device's id"));
        }
        if self.locals.borrow().device(device_id) {
            return Err(format!("it says it is {device_id}, this device or one of its workspace's"));
        }
        let mut losers = Vec::new();
        for (other, s) in seen.iter().filter(|(k, s)| k.as_str() != key && s.device_id == device_id) {
            if (s.paired_at, other.as_str()) <= (paired_at, key) {
                return Err(format!("it says it is {device_id}, as a server paired before it does"));
            }
            losers.push(other.clone());
        }
        Ok(losers)
    }

    /// Dial `key` and follow it, again after each loss.
    async fn follow(self: Arc<Self>, key: String) {
        let mut wait = RETRY_MIN;
        loop {
            match self.follow_once(&key).await {
                Ok(()) => wait = RETRY_MIN,
                Err(Quiet::Gone) => return,
                Err(Quiet::Refused(why)) => {
                    tracing::debug!(peer = %key, %why, "peer view: the server shows nothing to this side");
                    self.forget(&key);
                    wait = REFUSED_RETRY;
                }
                Err(Quiet::Unreachable(why)) => {
                    tracing::debug!(peer = %key, %why, "peer view: the server isn't reached");
                    wait = (wait * 2).min(RETRY_MAX);
                }
            }
            self.live.lock().unwrap().remove(&key);
            let wake = self.wake.clone();
            tokio::select! {
                () = tokio::time::sleep(wait) => {}
                () = wake.notified() => {}
            }
        }
    }

    async fn follow_once(&self, key: &str) -> Result<(), Quiet> {
        // This device's own ids first: nothing of a server's before them.
        let _ = self.locals.subscribe().wait_for(Locals::ready).await;
        let link = self.link.upgrade().ok_or(Quiet::Gone)?;
        let peer = link.peers().into_iter().find(|p| p.key == key).ok_or(Quiet::Gone)?;
        let client = Arc::new(link.connect(key).await.map_err(|e| Quiet::Unreachable(e.to_string()))?);
        drop(link);
        let info = client.call(methods::ENGINE_INFO, json!({})).await.map_err(|e| match e {
            RpcError::Transport(_) | RpcError::Closed => Quiet::Unreachable(e.to_string()),
            other => Quiet::Refused(other.to_string()),
        })?;
        let device_id = info["deviceId"].as_str().filter(|d| !d.is_empty() && d.len() <= 200).map(str::to_string);
        let Some(device_id) = device_id else {
            return Err(Quiet::Refused("it said no device id".into()));
        };
        let projects = self.admit(key, &device_id, Kind::Projects, &info["peer"]["projects"]).unwrap_or_default();
        let since = chrono::Utc::now().to_rfc3339();
        let paired_at = chrono::DateTime::from_timestamp_millis(peer.paired_at).map(|t| t.to_rfc3339());
        let device = |seen_at: &str| {
            json!({
                "id": device_id,
                "name": peer.name.clone().unwrap_or_else(|| "Paired server".into()),
                // (A server: the window draws it as one.)
                "platform": "linux",
                "lastSeenAt": seen_at,
                "createdAt": paired_at,
                "capabilities": [PAIRED_SERVER],
            })
        };
        let mut chats = client.subscribe_checked(methods::WATCH_CHATS, json!({})).await.ok();
        let mut sessions = client.subscribe_checked(methods::WATCH_SESSIONS, json!({})).await.ok();
        let mut claimed = Ok(Vec::new());
        self.seen.send_modify(|seen| {
            claimed = self.claim(seen, key, &device_id, peer.paired_at);
            let Ok(losers) = &claimed else { return };
            for loser in losers {
                seen.remove(loser);
            }
            let was = seen.remove(key).filter(|s| s.device_id == device_id);
            let mut now = Seen {
                device_id: device_id.clone(),
                device: device(&since),
                projects,
                since: was.as_ref().map_or_else(|| since.clone(), |w| w.since.clone()),
                paired_at: peer.paired_at,
                // Unreadable now: none shown.
                chats: if chats.is_some() { was.as_ref().map(|w| w.chats.clone()).unwrap_or_default() } else { Vec::new() },
                sessions: if sessions.is_some() { was.as_ref().map(|w| w.sessions.clone()).unwrap_or_default() } else { Vec::new() },
                configs: was.map(|w| w.configs).unwrap_or_default(),
                spaces: Vec::new(),
            };
            now.rebuild_spaces();
            seen.insert(key.to_string(), now);
        });
        match claimed {
            Err(why) => {
                self.note(key, &why);
                return Err(Quiet::Refused(why));
            }
            Ok(losers) => {
                for loser in losers {
                    self.note(&loser, &format!("its device id {device_id} is a server's paired before it"));
                    self.forget(&loser);
                }
            }
        }
        self.live.lock().unwrap().insert(key.to_string(), Live { client: client.clone() });
        tracing::info!(peer = %key, device = %device_id, reads = chats.is_some(), "peer view: a paired server is shown");
        let mut beat = tokio::time::interval(HEARTBEAT);
        beat.tick().await;
        loop {
            tokio::select! {
                rows = next(&mut chats) => {
                    let Some(rows) = rows else { break };
                    let Some(rows) = self.admit(key, &device_id, Kind::Chats, &rows) else { continue };
                    let ids: HashSet<&str> = rows.iter().filter_map(|c| c["id"].as_str()).collect();
                    // Made from here and listed now: only its config is kept.
                    let listed: Vec<(String, Value)> = {
                        let mut made = self.made.lock().unwrap();
                        let mine: Vec<String> = made.iter().filter(|(c, (k, _))| k == key && ids.contains(c.as_str())).map(|(c, _)| c.clone()).collect();
                        mine.into_iter().filter_map(|c| made.remove(&c).map(|(_, config)| (c, config))).collect()
                    };
                    let shown = self.update(key, |s| {
                        s.configs.extend(listed);
                        s.configs.retain(|c, _| ids.contains(c.as_str()));
                        s.chats = rows.clone();
                        s.rebuild_spaces();
                    });
                    if !shown {
                        break;
                    }
                }
                rows = next(&mut sessions) => {
                    let Some(rows) = rows else { break };
                    let Some(rows) = self.admit(key, &device_id, Kind::Sessions, &rows) else { continue };
                    if !self.update(key, |s| s.sessions = rows) {
                        break;
                    }
                }
                _ = beat.tick() => {
                    let at = chrono::Utc::now().to_rfc3339();
                    if !self.update(key, |s| s.device["lastSeenAt"] = json!(at)) {
                        break;
                    }
                }
                () = client.closed() => break,
            }
        }
        if !self.seen.borrow().contains_key(key) {
            // Another server, paired before, took its device id.
            return Err(Quiet::Refused("another server shows as its device".into()));
        }
        Ok(())
    }

    /// Change what is shown of `key`; false when nothing is (forgotten).
    fn update(&self, key: &str, change: impl FnOnce(&mut Seen)) -> bool {
        let mut shown = false;
        self.seen.send_if_modified(|seen| match seen.get_mut(key) {
            Some(s) => {
                change(s);
                shown = true;
                true
            }
            None => false,
        });
        shown
    }

    // ---- routing ------------------------------------------------------------

    /// The paired server a call of this device's own goes to, if any: one
    /// whose device it names (`targetDeviceId`), or a chat or project of
    /// one (`chatId`, a `Mutate`'s `spaceId` or `deviceId`). An id this
    /// device has is its own, whatever a server says; a chat being made goes
    /// by its project or device, never by the id it is to have.
    fn target(&self, method: &str, params: &Value) -> Option<String> {
        let locals = self.locals.borrow();
        let seen = self.seen.borrow();
        let by_device = |id: &str| {
            if locals.device(id) {
                return None;
            }
            seen.iter().find(|(_, s)| s.device_id == id).map(|(k, _)| k.clone())
        };
        let by_space = |id: &str| {
            if locals.spaces.contains(id) {
                return None;
            }
            seen.iter().find(|(_, s)| s.spaces.iter().any(|p| p["id"].as_str() == Some(id))).map(|(k, _)| k.clone())
        };
        let text = |key: &str| params.get(key).and_then(Value::as_str).filter(|v| !v.is_empty());
        if let Some(device) = text("targetDeviceId") {
            return by_device(device);
        }
        let mutate = method == methods::MUTATE;
        if mutate && params["op"].as_str() == Some("createChat") {
            return match (text("spaceId"), text("deviceId")) {
                (Some(space), _) => by_space(space),
                (None, Some(device)) => by_device(device),
                (None, None) => None,
            };
        }
        if let Some(chat) = text("chatId") {
            if locals.chat(chat) {
                return None;
            }
            if let Some((key, _)) = self.made.lock().unwrap().get(chat) {
                return Some(key.clone());
            }
            if let Some(key) = seen.iter().find(|(_, s)| s.chats.iter().any(|c| c["id"].as_str() == Some(chat))).map(|(k, _)| k.clone()) {
                return Some(key);
            }
        }
        if mutate {
            if let Some(space) = text("spaceId") {
                return by_space(space);
            }
            if let Some(device) = text("deviceId") {
                return by_device(device);
            }
        }
        None
    }

    /// This device's own call, answered here or by a paired server; `None`:
    /// the engine's (nothing of a paired server's in it).
    pub(crate) async fn route(&self, inner: &Arc<dyn RpcService>, method: &str, params: &Value) -> Option<Result<RpcReply, RpcError>> {
        if let Some(list) = List::of(method) {
            return Some(self.merged(inner, method, params, list).await);
        }
        let Some(key) = self.target(method, params) else {
            if method == methods::MUTATE
                && params["op"].as_str() == Some("createChat")
                && let Some(chat) = params["chatId"].as_str().filter(|c| !c.is_empty())
            {
                // Made here: this device's, before its list says so.
                self.locals.send_modify(|l| {
                    l.made.insert(chat.to_string());
                });
            }
            return None;
        };
        Some(self.forward(&key, method, params.clone()).await)
    }

    async fn forward(&self, key: &str, method: &str, mut params: Value) -> Result<RpcReply, RpcError> {
        let name = self.name(key);
        let client = self.live.lock().unwrap().get(key).map(|l| l.client.clone());
        let Some(client) = client else {
            return Err(RpcError::Failed(format!("{name} is unreachable: not connected over the peer link")));
        };
        let op = (method == methods::MUTATE).then(|| params["op"].as_str().unwrap_or_default().to_string());
        let chat = params["chatId"].as_str().unwrap_or_default().to_string();
        match op.as_deref() {
            Some("createChat") => {
                // Made at its project's folder, there: none of this side's
                // folder, device or (an empty) branch.
                if let Some(map) = params.as_object_mut() {
                    map.remove("cwd");
                    map.remove("deviceId");
                    if map.get("branch").and_then(Value::as_str).is_none_or(str::is_empty) {
                        map.remove("branch");
                    }
                }
                self.made.lock().unwrap().insert(chat.clone(), (key.to_string(), params["config"].clone()));
            }
            // A chat made from here, sent the config it was made with (the
            // window sets it again as it sends the first message): nothing
            // to change there. Another is the server's to refuse.
            Some("setChatConfig") if self.made_with(key, &chat).is_some_and(|config| config == params["config"]) => {
                return Ok(RpcReply::Value(json!({})));
            }
            _ => {}
        }
        let said = |e: RpcError| match e {
            RpcError::Transport(_) | RpcError::Closed => RpcError::Failed(format!("{name} is unreachable: {e}")),
            other => RpcError::Failed(format!("{REFUSED_PREFIX} {name}: {other}")),
        };
        let reply = if crate::rpc::is_stream_method(method) {
            client.subscribe_checked(method, params).await.map(|sub| RpcReply::Stream(items(sub)))
        } else {
            client.call(method, params).await.map(RpcReply::Value)
        };
        if reply.is_err() && op.as_deref() == Some("createChat") {
            self.made.lock().unwrap().remove(&chat);
        }
        reply.map_err(said)
    }

    /// The config chat `chat` was made with from here, on server `key`.
    fn made_with(&self, key: &str, chat: &str) -> Option<Value> {
        if let Some((k, config)) = self.made.lock().unwrap().get(chat) {
            return (k == key).then(|| config.clone());
        }
        self.seen.borrow().get(key).and_then(|s| s.configs.get(chat).cloned())
    }

    fn name(&self, key: &str) -> String {
        self.link
            .upgrade()
            .and_then(|link| link.peers().into_iter().find(|p| p.key == key))
            .and_then(|p| p.name)
            .unwrap_or_else(|| "The paired server".into())
    }

    /// The engine's list `method` with the paired servers' rows after its
    /// own (its own win a key both have), whole or as change frames as
    /// asked (`changes`), on every change of either.
    async fn merged(&self, inner: &Arc<dyn RpcService>, method: &str, params: &Value, list: List) -> Result<RpcReply, RpcError> {
        let changes = crate::rpc::wants_changes(params);
        let mut whole = params.clone();
        if let Some(map) = whole.as_object_mut() {
            map.remove("changes");
        }
        let local = match inner.handle(method, whole).await? {
            RpcReply::Stream(local) => local,
            value @ RpcReply::Value(_) => return Ok(value),
        };
        struct State {
            local: futures::stream::BoxStream<'static, Value>,
            seen: watch::Receiver<BTreeMap<String, Seen>>,
            seen_open: bool,
            rows: Option<Value>,
            fresh: bool,
            sent: Option<Value>,
            last: Option<crate::rpc::SentRows>,
        }
        let state = State { local, seen: self.seen.subscribe(), seen_open: true, rows: None, fresh: false, sent: None, last: None };
        let stream = futures::stream::unfold(state, move |mut st| async move {
            loop {
                if !st.fresh {
                    tokio::select! {
                        next = st.local.next() => match next {
                            Some(rows) => st.rows = Some(rows),
                            None => return None,
                        },
                        changed = st.seen.changed(), if st.seen_open && st.rows.is_some() => {
                            if changed.is_err() {
                                st.seen_open = false;
                                continue;
                            }
                        }
                    }
                }
                st.fresh = false;
                let Some(local) = st.rows.clone() else { continue };
                let rows = merge(list, local, &st.seen.borrow_and_update());
                if changes {
                    let (frame, sent) = crate::rpc::keyed_frame(rows, list.key(), st.last.take());
                    st.last = Some(sent);
                    if let Some(frame) = frame {
                        return Some((frame, st));
                    }
                } else {
                    let rows = Value::Array(rows);
                    if st.sent.as_ref() != Some(&rows) {
                        st.sent = Some(rows.clone());
                        return Some((rows, st));
                    }
                }
            }
        });
        Ok(RpcReply::Stream(stream.boxed()))
    }
}

enum Quiet {
    /// Unpaired, or the engine is going.
    Gone,
    /// It answered, and grants this side nothing it shows.
    Refused(String),
    Unreachable(String),
}

/// What of a server's [`PeerView::admit`] takes.
#[derive(Clone, Copy, Debug)]
enum Kind {
    Chats,
    Sessions,
    /// The projects its grants name (`EngineInfo.peer.projects`).
    Projects,
}

/// Follow this device's own lists into `view`'s [`Locals`], again after
/// each end, while the view lives.
async fn follow_locals(view: Weak<PeerView>, inner: Arc<dyn RpcService>) {
    loop {
        let own = match inner.handle(methods::ENGINE_INFO, json!({})).await {
            Ok(RpcReply::Value(info)) => info["deviceId"].as_str().unwrap_or_default().to_string(),
            _ => String::new(),
        };
        let mut lists = Vec::new();
        for list in [List::Devices, List::Chats, List::Sessions, List::Spaces] {
            if let Ok(RpcReply::Stream(rows)) = inner.handle(list.method(), json!({})).await {
                lists.push(rows.map(move |rows| (list, rows)).boxed());
            }
        }
        let mut all = futures::stream::select_all(lists);
        {
            let Some(view) = view.upgrade() else { return };
            view.locals.send_modify(|l| l.own = own);
        }
        while let Some((list, rows)) = all.next().await {
            let Some(view) = view.upgrade() else { return };
            view.locals.send_modify(|l| l.set(list, &rows));
        }
        tokio::time::sleep(RETRY_MIN).await;
    }
}

/// `rows` of a server's `what`, as this side shows them: only those with an
/// id none of `taken` has (each dropped one said in `dropped`), each
/// stamped with `device`, a chat with none of the server's folders (its
/// `cwd` only relative to its project, as the server says it) and a parent
/// only among these, a project only its id and name.
fn clean(what: Kind, rows: &[Value], device: &str, taken: &Taken, dropped: &mut Vec<String>) -> Vec<Value> {
    let mut ids = HashSet::new();
    let mut kept = Vec::new();
    for row in rows {
        let Some(obj) = row.as_object() else { continue };
        let key = if matches!(what, Kind::Sessions) { "chatId" } else { "id" };
        let Some(id) = obj.get(key).and_then(Value::as_str).filter(|id| !id.is_empty()) else { continue };
        let theirs = match what {
            Kind::Chats | Kind::Sessions => &taken.chats,
            Kind::Projects => &taken.spaces,
        };
        if theirs.contains(id) {
            dropped.push(format!("{what:?} {id}: this device's or another server's"));
            continue;
        }
        if !ids.insert(id.to_string()) {
            continue;
        }
        let mut obj = obj.clone();
        match what {
            Kind::Projects => {
                let name = obj.get("name").and_then(Value::as_str).map(|n| n.chars().take(200).collect::<String>());
                kept.push(json!({ "id": id, "name": name }));
                continue;
            }
            Kind::Chats => {
                if let Some(space) = obj.get("spaceId").and_then(Value::as_str)
                    && taken.spaces.contains(space)
                {
                    dropped.push(format!("Chats {id}: in project {space}, this device's or another server's"));
                    ids.remove(id);
                    continue;
                }
                for key in ["harnessSessionCwd", "harnessSessionId", "sourceContext", "checkoutId"] {
                    obj.remove(key);
                }
                if !obj.get("cwd").and_then(Value::as_str).is_some_and(project_relative) {
                    obj.remove("cwd");
                }
            }
            Kind::Sessions => {}
        }
        obj.insert("deviceId".into(), json!(device));
        kept.push(Value::Object(obj));
    }
    if matches!(what, Kind::Chats) {
        for row in &mut kept {
            let parent = row["parentChatId"].as_str().map(str::to_string);
            if parent.is_some_and(|p| !ids.contains(&p))
                && let Some(obj) = row.as_object_mut()
            {
                obj.remove("parentChatId");
            }
        }
    }
    kept
}

/// Whether `cwd` is a folder inside a project, named from its root (`.`,
/// `src/app`): not a whole path on any system, nothing climbing out.
fn project_relative(cwd: &str) -> bool {
    let bytes = cwd.as_bytes();
    let whole = cwd.starts_with(['/', '\\', '~']) || (bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':');
    !cwd.is_empty() && !whole && cwd.split(['/', '\\']).all(|part| part != "..")
}

/// The next item of a stream this side may not have (never, then).
async fn next(sub: &mut Option<RpcSubscription>) -> Option<Value> {
    match sub {
        Some(sub) => sub.recv().await,
        None => std::future::pending().await,
    }
}

fn items(sub: RpcSubscription) -> futures::stream::BoxStream<'static, Value> {
    futures::stream::unfold(sub, |mut sub| async move { sub.recv().await.map(|item| (item, sub)) }).boxed()
}

/// `local` (a list of rows) with each server's rows of `list` whose key it
/// doesn't have.
fn merge(list: List, local: Value, seen: &BTreeMap<String, Seen>) -> Vec<Value> {
    let mut rows = match local {
        Value::Array(rows) => rows,
        _ => Vec::new(),
    };
    let key = list.key();
    let mut keys: HashSet<String> = rows.iter().filter_map(|r| r[key].as_str().map(str::to_string)).collect();
    for s in seen.values() {
        for row in s.rows(list) {
            let Some(k) = row[key].as_str() else { continue };
            if keys.insert(k.to_string()) {
                rows.push(row.clone());
            }
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_engines_own_rows_win_and_the_servers_follow() {
        let mut seen = BTreeMap::new();
        let mut s = Seen { device_id: "srv".into(), device: json!({ "id": "srv" }), ..Default::default() };
        s.chats = vec![json!({ "id": "c1", "spaceId": "p" }), json!({ "id": "mine" })];
        s.projects = vec![json!({ "id": "q", "name": "q-name" })];
        s.rebuild_spaces();
        seen.insert("k".to_string(), s);
        let chats = merge(List::Chats, json!([{ "id": "mine", "local": true }]), &seen);
        assert_eq!(chats, vec![json!({ "id": "mine", "local": true }), json!({ "id": "c1", "spaceId": "p" })]);
        let spaces = merge(List::Spaces, json!([]), &seen);
        let ids: Vec<&str> = spaces.iter().map(|s| s["id"].as_str().unwrap()).collect();
        assert_eq!(ids, ["q", "p"]);
        assert_eq!(spaces[0]["name"], "q-name");
        assert_eq!(spaces[0]["deviceId"], "srv");
        assert_eq!(spaces[0]["path"], "", "a server's folder is never said");
        assert_eq!(merge(List::Devices, json!([{ "id": "here" }]), &seen), vec![json!({ "id": "here" }), json!({ "id": "srv" })]);
    }
}
