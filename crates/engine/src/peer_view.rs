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
//! longer each time. Not reached, its rows stay and its device shows
//! offline from the first failed redial, for [`QUIET_KEEP`] (then they go);
//! refused (unpaired there: its handshake's 401), nothing of it stays.
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
//!
//! This engine's workers ([`crate::workers`], step 5) show the same way,
//! each as a device of its own (capabilities [`PAIRED_SERVER`] and
//! [`WORKER`], named as the worker), followed under the key
//! `worker:<name>`: dialled over its socket
//! ([`crate::workers::Workers::connect`], 16 MiB cap), with one project,
//! the one this server gave it ([`crate::workers::Worker::project`],
//! named `work`; what it says of its projects is never read). A worker is
//! as trusted as the agent in it, so on top of the above:
//! - its device id is a plain id (letters, digits, `-_.:`, at most 128),
//!   and loses to a paired server's of that id;
//! - only its chats in that project show; any other is dropped;
//! - of the calls naming it only [`forwarded_to_worker`] go (harnesses,
//!   models, making a chat (always in its project) and setting its config,
//!   `QueueCommand`, `WatchDocMessages`); a made chat's reply says its
//!   device, not the worker's words.
//! Its rows merge only for this device's own callers, as a server's do: a
//! paired server and the phone link never see a worker.

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
/// How often a worker's turns on lent logins are looked at, settled or not
/// ([`crate::worker_logins::settle`]).
const SETTLE_EVERY: Duration = Duration::from_secs(1);
/// How often the paired servers are looked at again (one paired or
/// unpaired since; [`PeerView::refresh`] at once).
const SCAN: Duration = Duration::from_secs(10);
const RETRY_MIN: Duration = Duration::from_secs(2);
const RETRY_MAX: Duration = Duration::from_secs(60);
/// How soon a server that granted this side nothing is asked again (it
/// may grant something any time; it doesn't say so).
const REFUSED_RETRY: Duration = SCAN;
/// How long a server not reached keeps its rows here (shown offline from
/// its first failed redial); then they go, until it's reached again.
const QUIET_KEEP: Duration = Duration::from_secs(10 * 60);
/// The most rows a server's list may hold (chats, sessions, projects); a
/// frame over it is dropped, and what it showed before stays.
pub const MAX_ROWS: usize = 5000;
/// The one capability a server's device row carries: none of its own (the
/// window reads a device's capabilities to offer what isn't open over the
/// peer link, agent updates, or to take it for Cloud's).
pub const PAIRED_SERVER: &str = "paired-server";
/// What a worker's device row carries besides [`PAIRED_SERVER`].
pub const WORKER: &str = "worker";
/// How a worker's key in the view starts (`worker:<name>`).
const WORKER_PREFIX: &str = "worker:";

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
    /// A worker's: its one project's id.
    worker_project: Option<String>,
    /// What its grants open for this side, as it said
    /// (`EngineInfo.peer.areas`, plan step 6b); `None`: not said (a Zend
    /// from before step 6, or a worker).
    areas: Option<HashSet<zend_rpc::Scope>>,
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
    /// How long a server not reached keeps its rows ([`QUIET_KEEP`]).
    quiet_keep: Mutex<Duration>,
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
            quiet_keep: Mutex::new(QUIET_KEEP),
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

    /// Keep a server not reached `keep` instead of [`QUIET_KEEP`] (for
    /// the tests).
    #[doc(hidden)]
    pub fn keep_quiet_servers_for(&self, keep: Duration) {
        *self.quiet_keep.lock().unwrap() = keep;
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
        let mut dialable: HashSet<String> = link
            .peers()
            .into_iter()
            .filter(|p| p.host.is_some() && p.port.is_some() && p.node.is_some())
            .map(|p| p.key)
            .collect();
        // (A worker has none of its own.)
        if crate::worker::role().is_none() {
            dialable.extend(crate::workers::Workers::new(link.data_dir()).list().into_iter().map(|w| format!("{WORKER_PREFIX}{}", w.name)));
        }
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
        // (Nor a device's id: this one's, its workspace's, a server's.)
        taken.chats.extend(locals.devices.iter().chain([&locals.own]).cloned());
        drop(locals);
        taken.chats.extend(self.seen.borrow().values().map(|s| s.device_id.clone()));
        // A paired server's ids aren't a worker's to take, whichever came
        // first: a worker's rows count only against another worker's (and
        // what a paired server shows merges first, [`merge`]).
        let worker = key.starts_with(WORKER_PREFIX);
        for s in self.seen.borrow().iter().filter(|(k, _)| k.as_str() != key && (worker || !k.starts_with(WORKER_PREFIX))).map(|(_, s)| s) {
            taken.chats.extend(s.chats.iter().chain(&s.sessions).filter_map(|r| r["id"].as_str().or(r["chatId"].as_str()).map(str::to_string)));
            taken.spaces.extend(s.spaces.iter().filter_map(|p| p["id"].as_str().map(str::to_string)));
        }
        taken.chats.extend(self.made.lock().unwrap().iter().filter(|(_, (k, _))| k != key).map(|(c, _)| c.clone()));
        taken
    }

    /// Server `key`'s `rows` of `what`, as this side shows them (see
    /// [`clean`]); `None` over [`MAX_ROWS`].
    fn admit(&self, key: &str, device_id: &str, what: Kind, rows: &Value, only_in: Option<&str>) -> Option<Vec<Value>> {
        let rows = rows.as_array().map(Vec::as_slice).unwrap_or_default();
        if rows.len() > MAX_ROWS {
            tracing::warn!(peer = %key, ?what, rows = rows.len(), "peer view: a paired server's list is over the cap; frame dropped");
            return None;
        }
        let mut dropped = Vec::new();
        let kept = clean(what, rows, device_id, &self.taken(key), only_in, &mut dropped);
        for d in dropped {
            self.note(key, &d);
        }
        Some(kept)
    }

    /// Why server `key` (paired at `paired_at`) may not show as
    /// `device_id`, else the servers it takes that id from.
    fn claim(&self, seen: &BTreeMap<String, Seen>, key: &str, device_id: &str, paired_at: i64) -> Result<Vec<String>, String> {
        // Paired servers first, then workers, each by when it came.
        let rank = |key: &str| key.starts_with(WORKER_PREFIX);
        if rank(key) && !plain_id(device_id) {
            return Err(format!("it says it is {:?}, not a device id", device_id.chars().take(40).collect::<String>()));
        }
        if !rank(key) && device_id.starts_with(crate::workers::DEVICE_PREFIX) {
            return Err(format!("it says it is {device_id}, a worker's id"));
        }
        if zend_proto::is_cloud_device(device_id, "") {
            return Err(format!("it says it is {device_id}, a Cloud device's id"));
        }
        if self.locals.borrow().device(device_id) {
            return Err(format!("it says it is {device_id}, this device or one of its workspace's"));
        }
        let mut losers = Vec::new();
        for (other, s) in seen.iter().filter(|(k, s)| k.as_str() != key && s.device_id == device_id) {
            if (rank(other), s.paired_at, other.as_str()) <= (rank(key), paired_at, key) {
                return Err(format!("it says it is {device_id}, as a server paired (or a worker made) before it does"));
            }
            losers.push(other.clone());
        }
        Ok(losers)
    }

    /// Dial `key` and follow it, again after each loss.
    async fn follow(self: Arc<Self>, key: String) {
        let mut wait = RETRY_MIN;
        // Since when it isn't reached (its first failed redial).
        let mut quiet: Option<tokio::time::Instant> = None;
        // Why it last refused, said once: it's asked again every
        // [`REFUSED_RETRY`], and says the same until something changes.
        let mut refusal = Refusal::default();
        loop {
            match self.follow_once(&key).await {
                Ok(()) => {
                    wait = RETRY_MIN;
                    quiet = None;
                    refusal.clear();
                }
                Err(Quiet::Gone) => return,
                // Unpaired there (its handshake refused), or granted nothing:
                // nothing of it stays.
                Err(Quiet::Refused(why)) => {
                    if refusal.first(&why) {
                        tracing::info!(peer = %key, %why, "peer view: the server shows nothing to this side (said once, until that changes)");
                    } else {
                        tracing::debug!(peer = %key, %why, "peer view: the server shows nothing to this side, still");
                    }
                    self.forget(&key);
                    wait = REFUSED_RETRY;
                    quiet = None;
                }
                Err(Quiet::Unreachable(why)) => {
                    refusal.clear();
                    tracing::debug!(peer = %key, %why, "peer view: the server isn't reached");
                    let since = *quiet.get_or_insert_with(|| {
                        // Shown offline now, not once its last beat is old.
                        self.update(&key, |s| s.device["lastSeenAt"] = Value::Null);
                        tokio::time::Instant::now()
                    });
                    let keep = *self.quiet_keep.lock().unwrap();
                    if since.elapsed() >= keep && self.seen.borrow().contains_key(&key) {
                        tracing::info!(peer = %key, ?keep, "peer view: a server not reached for long: its rows go");
                        self.forget(&key);
                    }
                    wait = (wait * 2).min(RETRY_MAX).min(keep.max(RETRY_MIN));
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
        let data_dir = link.data_dir().to_path_buf();
        let quiet = |e: RpcError| match e {
            RpcError::Transport(_) | RpcError::Closed => Quiet::Unreachable(e.to_string()),
            other => Quiet::Refused(other.to_string()),
        };
        // Not reached, or refused (unpaired there: its handshake's 401; a
        // worker answering with another key).
        let (client, name, paired_at, worker_project, worker_device) = match key.strip_prefix(WORKER_PREFIX) {
            Some(worker) => {
                let workers = crate::workers::Workers::new(link.data_dir());
                let worker = workers.get(worker).map_err(|_| Quiet::Gone)?;
                drop(link);
                let client = workers.connect(&worker.name).await.map_err(quiet)?;
                (client, worker.name.clone(), worker.created_at, Some(worker.project()), Some(worker.device_id()))
            }
            None => {
                let peer = link.peers().into_iter().find(|p| p.key == key).ok_or(Quiet::Gone)?;
                let client = link.connect(key).await.map_err(quiet)?;
                drop(link);
                (client, peer.name.clone().unwrap_or_else(|| "Paired server".into()), peer.paired_at, None, None)
            }
        };
        let client = Arc::new(client);
        let info = client.call(methods::ENGINE_INFO, json!({})).await.map_err(|e| match e {
            RpcError::Transport(_) | RpcError::Closed => Quiet::Unreachable(e.to_string()),
            other => Quiet::Refused(other.to_string()),
        })?;
        // A worker's device id is its server's word ([`crate::workers::device_of`]):
        // what it says (`EngineInfo`) is only that it answers.
        let device_id = worker_device.or_else(|| info["deviceId"].as_str().filter(|d| !d.is_empty() && d.len() <= 200).map(str::to_string));
        let Some(device_id) = device_id else {
            return Err(Quiet::Refused("it said no device id".into()));
        };
        // A worker's one project is the one its server gave it; what it
        // says of its projects isn't read.
        let projects = match &worker_project {
            Some(project) => json!([{ "id": project, "name": crate::worker::PROJECT }]),
            None => info["peer"]["projects"].clone(),
        };
        let projects = self.admit(key, &device_id, Kind::Projects, &projects, None).unwrap_or_default();
        let areas = if worker_project.is_some() { None } else { areas_of(&info) };
        let since = chrono::Utc::now().to_rfc3339();
        let created_at = chrono::DateTime::from_timestamp_millis(paired_at).map(|t| t.to_rfc3339());
        let capabilities = if worker_project.is_some() { json!([PAIRED_SERVER, WORKER]) } else { json!([PAIRED_SERVER]) };
        let device = |seen_at: &str| {
            json!({
                "id": device_id,
                "name": name,
                // (A server: the window draws it as one.)
                "platform": "linux",
                "lastSeenAt": seen_at,
                "createdAt": created_at,
                "capabilities": capabilities,
            })
        };
        let only_in = worker_project.as_deref();
        let mut chats = client.subscribe_checked(methods::WATCH_CHATS, json!({})).await.ok();
        let mut sessions = client.subscribe_checked(methods::WATCH_SESSIONS, json!({})).await.ok();
        let mut claimed = Ok(Vec::new());
        self.seen.send_modify(|seen| {
            claimed = self.claim(seen, key, &device_id, paired_at);
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
                paired_at,
                worker_project: worker_project.clone(),
                areas,
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
        // A worker's sessions, as it said them: shown only for a chat of its
        // shown now (in its project), so again as its chats change.
        let mut said_sessions: Option<Vec<Value>> = None;
        let worker = worker_project.is_some();
        let mut beat = tokio::time::interval(HEARTBEAT);
        beat.tick().await;
        // A worker's turns on lent logins, settled: their logins go.
        let mut settle = tokio::time::interval(SETTLE_EVERY);
        let settled = |sessions: &[Value]| {
            for (chat, provider) in crate::worker_logins::settle(&data_dir, &name, sessions, std::time::Instant::now()) {
                tracing::info!(worker = %name, chat = %chat, %provider, "worker: a turn on this device's login settled, its login taken back");
            }
        };
        loop {
            tokio::select! {
                rows = next(&mut chats) => {
                    let Some(rows) = rows else { break };
                    let Some(rows) = self.admit(key, &device_id, Kind::Chats, &rows, only_in) else { continue };
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
                        if worker {
                            s.sessions = of_chats_shown(said_sessions.as_deref().unwrap_or_default(), &s.chats);
                        }
                    });
                    if !shown {
                        break;
                    }
                }
                rows = next(&mut sessions) => {
                    let Some(rows) = rows else { break };
                    let Some(rows) = self.admit(key, &device_id, Kind::Sessions, &rows, only_in) else { continue };
                    let shown = if worker {
                        let shown = self.update(key, |s| s.sessions = of_chats_shown(&rows, &s.chats));
                        settled(&rows);
                        said_sessions = Some(rows);
                        shown
                    } else {
                        self.update(key, |s| s.sessions = rows)
                    };
                    if !shown {
                        break;
                    }
                }
                _ = beat.tick() => {
                    let at = chrono::Utc::now().to_rfc3339();
                    if !self.update(key, |s| s.device["lastSeenAt"] = json!(at)) {
                        break;
                    }
                }
                _ = settle.tick(), if worker => settled(said_sessions.as_deref().unwrap_or_default()),
                () = client.closed() => break,
            }
        }
        if worker {
            // Its turns can't be followed any more: what was lent goes.
            let chats = crate::worker_logins::release(&data_dir, &name);
            if !chats.is_empty() {
                tracing::info!(worker = %name, turns = chats.len(), "worker: connection ended, the logins lent to its turns taken back");
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
            paired_first(&seen).find(|(_, s)| s.spaces.iter().any(|p| p["id"].as_str() == Some(id))).map(|(k, _)| k.clone())
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
            if let Some(key) = paired_first(&seen).find(|(_, s)| s.chats.iter().any(|c| c["id"].as_str() == Some(chat))).map(|(k, _)| k.clone()) {
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

    /// Which of this device's own ids the call names, at any depth and by
    /// any key (`chatId`, `sourceChatId`, `parentChatId`, `spaceId`,
    /// `requesterDeviceId`, …, [`id_fields`]), as words for a refusal:
    /// such a call is never a server's.
    fn names_own(&self, params: &Value) -> Option<String> {
        let locals = self.locals.borrow();
        id_fields(params).into_iter().find_map(|(kind, id)| match kind {
            IdKind::Chat if locals.chat(&id) => Some(format!("chat {id}")),
            IdKind::Space if locals.spaces.contains(&id) => Some(format!("project {id}")),
            IdKind::Device if locals.device(&id) => Some(format!("device {id}")),
            _ => None,
        })
    }

    /// This device's own call, answered here or by a paired server; `None`:
    /// the engine's (nothing of a paired server's in it).
    pub(crate) async fn route(&self, inner: &Arc<dyn RpcService>, method: &str, params: &Value) -> Option<Result<RpcReply, RpcError>> {
        if let Some(list) = List::of(method) {
            return Some(self.merged(inner, method, params, list).await);
        }
        let target = self.target(method, params);
        // Aimed at a server (`targetDeviceId`) while naming this device's
        // own chat, project or device: never sent, whichever it means.
        if let Some(key) = &target
            && let Some(own) = self.names_own(&{
                // As it's sent, and the device it was asked to be made on.
                let mut checked = self.as_sent(key, method, params.clone());
                if let (Some(map), Some(device)) = (checked.as_object_mut(), params.get("deviceId")) {
                    map.insert("deviceId".into(), device.clone());
                }
                checked
            })
        {
            tracing::warn!(method, server = %key.get(..12).unwrap_or(key), "peer view: refused a call aimed at a paired server that names this device's own {own}");
            return Some(Err(RpcError::Failed(format!(
                "{REFUSED_PREFIX} {}: {method} names this device's own {own}, so it isn't sent to a paired server: send it without targetDeviceId",
                self.name(key)
            ))));
        }
        if let Some(key) = &target {
            // Names a server, but isn't one it takes from here (never one
            // a server takes, or not granted by it): said no to here, and
            // nothing of it leaves this device.
            let name = self.name(key);
            let held = if key.starts_with(WORKER_PREFIX) {
                (!forwarded_to_worker(method, params)).then(|| format!("{method} isn't sent to a worker: do it in its chat"))
            } else {
                let areas = self.seen.borrow().get(key.as_str()).and_then(|s| s.areas.clone());
                held_back(method, params, areas.as_ref(), &name)
            };
            if let Some(why) = held {
                tracing::warn!(method, server = %key.get(..12).unwrap_or(key), "peer view: held back a call naming a paired server: {why}");
                return Some(Err(RpcError::Failed(format!("{REFUSED_PREFIX} {name}: {why}"))));
            }
        }
        let Some(key) = target else {
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

    /// `params` as they're sent to the server `key`: a `createChat` is
    /// made at its project's folder there, with none of this side's
    /// folder, device or (an empty) branch; a worker's is in its one
    /// project, whatever named it, and is sent its id and config alone.
    /// What's checked for this device's own ids is this, not what the
    /// window gave.
    fn as_sent(&self, key: &str, method: &str, mut params: Value) -> Value {
        if method != methods::MUTATE || params["op"].as_str() != Some("createChat") {
            return params;
        }
        let worker_project = self.seen.borrow().get(key).and_then(|s| s.worker_project.clone());
        let chat = params["chatId"].clone();
        if let Some(map) = params.as_object_mut() {
            map.remove("cwd");
            map.remove("deviceId");
            if map.get("branch").and_then(Value::as_str).is_none_or(str::is_empty) {
                map.remove("branch");
            }
            if let Some(project) = worker_project {
                let config = map.remove("config").unwrap_or(Value::Null);
                *map = serde_json::Map::from_iter([
                    ("op".to_string(), json!("createChat")),
                    ("chatId".to_string(), chat),
                    ("spaceId".to_string(), json!(project)),
                    ("config".to_string(), config),
                ]);
            }
        }
        params
    }

    async fn forward(&self, key: &str, method: &str, mut params: Value) -> Result<RpcReply, RpcError> {
        let name = self.name(key);
        let client = self.live.lock().unwrap().get(key).map(|l| l.client.clone());
        let Some(client) = client else {
            return Err(RpcError::Failed(format!("{name} is unreachable: not connected over the peer link")));
        };
        let op = (method == methods::MUTATE).then(|| params["op"].as_str().unwrap_or_default().to_string());
        let chat = params["chatId"].as_str().unwrap_or_default().to_string();
        let (device_id, worker_project) = {
            let seen = self.seen.borrow();
            let s = seen.get(key);
            (s.map(|s| s.device_id.clone()).unwrap_or_default(), s.and_then(|s| s.worker_project.clone()))
        };
        params = self.as_sent(key, method, params);
        match op.as_deref() {
            Some("createChat") => {
                self.made.lock().unwrap().insert(chat.clone(), (key.to_string(), params["config"].clone()));
            }
            // Every config is sent, the same one too (the window sets it
            // again as it sends the first message, and a revert is one):
            // the server's gate takes it of a chat it may run, or refuses.
            _ => {}
        }
        // A worker's device is its server's name for it, not its own; a
        // chat's op names its chat, which says where it goes there.
        let chat_op = method == methods::MUTATE && op.as_deref().is_some_and(|op| op != "createChat");
        if (worker_project.is_some() || chat_op)
            && let Some(map) = params.as_object_mut()
        {
            map.remove("targetDeviceId");
        }
        if method == methods::QUEUE_COMMAND {
            only_what_a_server_runs(&mut params);
            if worker_project.is_some() {
                self.worker_turn(key, &chat, &params).await?;
            }
        }
        let said = |e: RpcError| match e {
            RpcError::Transport(_) | RpcError::Closed => RpcError::Failed(format!("{name} is unreachable: {e}")),
            other => RpcError::Failed(format!("{REFUSED_PREFIX} {name}: {other}")),
        };
        let config = (op.as_deref() == Some("setChatConfig")).then(|| params["config"].clone());
        let reply = if crate::rpc::is_stream_method(method) {
            client.subscribe_checked(method, params).await.map(|sub| RpcReply::Stream(items(sub)))
        } else {
            client.call(method, params).await.map(RpcReply::Value)
        };
        if reply.is_err() && op.as_deref() == Some("createChat") {
            self.made.lock().unwrap().remove(&chat);
        }
        // Refused for its grants: they changed since it said them, so hear
        // them again (a grant change also ends its connection: redialled).
        if worker_project.is_none()
            && let Err(e) = &reply
            && !matches!(e, RpcError::Transport(_) | RpcError::Closed)
            && e.to_string().contains("isn't open to your server")
            && let Ok(info) = client.call(methods::ENGINE_INFO, json!({})).await
        {
            let areas = areas_of(&info);
            self.seen.send_modify(|seen| {
                if let Some(s) = seen.get_mut(key) {
                    s.areas = areas;
                }
            });
        }
        // What it runs with now (a worker's turn picks its login by it).
        if reply.is_ok()
            && let Some(config) = config
            && let Some((k, kept)) = self.made.lock().unwrap().get_mut(&chat)
            && k == key
        {
            *kept = config;
        }
        let mut reply = reply.map_err(said)?;
        // A worker's chat is on the device this side shows it as.
        if worker_project.is_some()
            && op.as_deref() == Some("createChat")
            && let RpcReply::Value(Value::Object(map)) = &mut reply
            && map.contains_key("deviceId")
        {
            map.insert("deviceId".into(), json!(device_id));
        }
        Ok(reply)
    }

    /// The config chat `chat` was made with from here, on server `key`.
    /// A run queued for worker `key`'s chat, as it leaves (plan step 4d-4):
    /// on the API key it was made with for its model's provider, which wins,
    /// else on this device's login when the owner allowed it that worker
    /// and provider, its access token written to the worker's read-only
    /// `conf` for this turn ([`crate::worker_logins`]); else refused, the
    /// chat told "no credential for <provider> on this device" and the
    /// reason logged here. The run itself is not changed: no token, key or
    /// path of this device's goes into it.
    async fn worker_turn(&self, key: &str, chat: &str, params: &Value) -> Result<(), RpcError> {
        let (Some(name), Some(link)) = (key.strip_prefix(WORKER_PREFIX), self.link.upgrade()) else {
            return Ok(());
        };
        let config = self.made_with(key, chat).unwrap_or(Value::Null);
        let Some((harness, model)) = crate::worker_logins::run_of(params, &config) else {
            return Ok(());
        };
        let data_dir = link.data_dir().to_path_buf();
        drop(link);
        let worker = crate::workers::Workers::new(&data_dir).get(name).map_err(RpcError::Failed)?;
        let host = crate::peer_logins::HostLogin::current();
        let session = self.seen.borrow().get(key).and_then(|s| s.sessions.iter().find(|r| r["chatId"].as_str() == Some(chat)).cloned());
        let turn_of = chat.to_string();
        // (Off the runtime: it may wait on the worker's lock and a refresh.)
        let prepared = tokio::task::spawn_blocking(move || {
            crate::worker_logins::prepare_turn(&data_dir, &worker, &turn_of, harness, model.as_deref(), session.as_ref(), &host)
        })
        .await
        .map_err(|e| RpcError::Failed(format!("{name}: {e}")))?;
        match prepared {
            Ok(crate::worker_logins::Credential::Login(provider)) => {
                tracing::info!(worker = %name, chat = %chat, %provider, "worker: a turn runs on this device's login (its access token only)");
                Ok(())
            }
            Ok(_) => Ok(()),
            Err(refused) => {
                tracing::warn!(worker = %name, chat = %chat, provider = refused.provider.as_deref().unwrap_or("?"), why = %refused.why, "worker: a turn has no credential to run on");
                Err(RpcError::Failed(format!("{REFUSED_PREFIX} {name}: {}", refused.told())))
            }
        }
    }

    fn made_with(&self, key: &str, chat: &str) -> Option<Value> {
        if let Some((k, config)) = self.made.lock().unwrap().get(chat) {
            return (k == key).then(|| config.clone());
        }
        self.seen.borrow().get(key).and_then(|s| s.configs.get(chat).cloned())
    }

    fn name(&self, key: &str) -> String {
        if let Some(worker) = key.strip_prefix(WORKER_PREFIX) {
            return format!("Worker {worker}");
        }
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

/// What a server last refused this side with, so a refusal is said once
/// per change, not at every redial.
#[derive(Debug, Default)]
struct Refusal(Option<String>);

impl Refusal {
    /// Whether `why` is new (not what it refused with last).
    fn first(&mut self, why: &str) -> bool {
        if self.0.as_deref() == Some(why) {
            return false;
        }
        self.0 = Some(why.to_string());
        true
    }

    /// It answered, or wasn't reached: a refusal after is news again.
    fn clear(&mut self) {
        self.0 = None;
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

/// What an id names: a chat, a project, a device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IdKind {
    Chat,
    Space,
    Device,
}

/// Every id `params` names, at any depth: each string (or list of
/// strings) under a key `chatId`/`…ChatId`, `spaceId`/`…SpaceId`,
/// `deviceId`/`…DeviceId` (and their plurals, `…Ids`), and `sessionId`,
/// `chats` (chats). Empty ones aren't ids. A `checkoutId` isn't one: it
/// is a checkout's content id, not a chat's.
pub(crate) fn id_fields(params: &Value) -> Vec<(IdKind, String)> {
    fn kind(key: &str) -> Option<IdKind> {
        // A sidebar pin's chat, a list of chats.
        if key == "sessionId" || key == "chats" {
            return Some(IdKind::Chat);
        }
        let key = key.strip_suffix('s').filter(|k| k.ends_with("Id")).unwrap_or(key);
        [("chatId", "ChatId", IdKind::Chat), ("spaceId", "SpaceId", IdKind::Space), ("deviceId", "DeviceId", IdKind::Device)]
            .into_iter()
            .find(|(bare, suffix, _)| key == *bare || key.ends_with(suffix))
            .map(|(_, _, kind)| kind)
    }
    fn walk(value: &Value, out: &mut Vec<(IdKind, String)>) {
        match value {
            Value::Object(map) => {
                for (key, value) in map {
                    if let Some(kind) = kind(key) {
                        let ids: Vec<&Value> = match value {
                            Value::Array(items) => items.iter().collect(),
                            other => vec![other],
                        };
                        out.extend(ids.into_iter().filter_map(Value::as_str).filter(|id| !id.is_empty()).map(|id| (kind, id.to_string())));
                    }
                    walk(value, out);
                }
            }
            Value::Array(items) => items.iter().for_each(|item| walk(item, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    walk(params, &mut out);
    out
}

/// Why a call of this device's own that names a paired server isn't
/// sent to it (plan step 6b), or `None`: it goes. Every call
/// [`zend_rpc::peer_scope`] knows (of `Mutate`, by its op), streams and
/// chunk calls with them, goes when the server said its grants open that
/// scope here (`EngineInfo.peer.areas`, [`Seen::areas`]); one that didn't
/// say (a Zend from before step 6) gets only what the grants before step 6
/// open ([`before_step6`]). Never sent, whatever it grants: pairing,
/// grants, the account, Cloud, billing, vault, GitHub, the phone link,
/// stopping Zend (`Scope::Never`), and a call naming this device's own
/// chat, project or device ([`PeerView::route`], first). Whether the
/// server takes it stays its grants' (`zend_rpc::peer_server_allowed`
/// there): its refusal comes back as [`REFUSED_PREFIX`].
fn held_back(method: &str, params: &Value, areas: Option<&HashSet<zend_rpc::Scope>>, name: &str) -> Option<String> {
    let scope = match zend_rpc::peer_scope(method, zend_rpc::mutate_op(method, params)) {
        None | Some(zend_rpc::Scope::Never) => return Some(format!("{method} isn't sent to a paired server: do it on that server")),
        Some(scope) => scope,
    };
    match areas {
        Some(areas) if areas.contains(&scope) => None,
        Some(_) => Some(format!("{name} doesn't grant {}, so {method} isn't sent", scope.name())),
        None if before_step6(method, params) => None,
        None => Some(format!("{name} doesn't grant {} (it doesn't say what it grants), so {method} isn't sent", scope.name())),
    }
}

/// The areas a server's `EngineInfo` says its grants open here.
fn areas_of(info: &Value) -> Option<HashSet<zend_rpc::Scope>> {
    let areas = info["peer"]["areas"].as_array()?;
    Some(areas.iter().filter_map(Value::as_str).filter_map(zend_rpc::Scope::from_name).collect())
}

/// What the grants before step 6 open, sent to a server that doesn't say
/// what it grants: `EngineInfo`, harnesses, models, `Mutate` `createChat`
/// and `setChatConfig`, `setChatArchived`, `markChatSeen`, `renameChat`,
/// `deleteChat`, `QueueCommand`, `WatchDocMessages`, project files.
fn before_step6(method: &str, params: &Value) -> bool {
    if zend_rpc::local_only(method) || !zend_rpc::peer_link::PEER_METHODS.contains(&method) {
        return false;
    }
    match method {
        methods::ENGINE_INFO
        | methods::LIST_HARNESSES
        | methods::LIST_MODELS
        | methods::QUEUE_COMMAND
        | methods::WATCH_DOC_MESSAGES
        | methods::PEER_LIST_PROJECT_FILES
        | methods::PEER_READ_PROJECT_FILE => true,
        methods::MUTATE => matches!(
            params["op"].as_str(),
            Some("createChat" | "setChatConfig" | "setChatArchived" | "markChatSeen" | "renameChat" | "deleteChat")
        ),
        _ => false,
    }
}

/// Whether a call of this device's own that names one of its workers is
/// sent to it: what its server's window needs to run a chat there (its
/// harnesses and models, making a chat and setting its config, a run, its
/// transcript), nothing else, files and engine info among it.
fn forwarded_to_worker(method: &str, params: &Value) -> bool {
    match method {
        methods::LIST_HARNESSES | methods::LIST_MODELS | methods::QUEUE_COMMAND | methods::WATCH_DOC_MESSAGES => true,
        methods::MUTATE => matches!(params["op"].as_str(), Some("createChat" | "setChatConfig")),
        _ => false,
    }
}


/// A plain device id: letters, digits and `-_.:`, 1 to 128 of them.
fn plain_id(id: &str) -> bool {
    (1..=128).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b':'))
}

/// A `QueueCommand` for a server's chat as it leaves: a run only with
/// what the server runs it with (the words, the agent and model picked),
/// none of this device's folder, files, worktree, MCP servers or a resume;
/// no files queued with it. (The server builds the run from its own row
/// and the words alone; this keeps the rest from crossing at all.)
fn only_what_a_server_runs(params: &mut Value) {
    let Some(map) = params.as_object_mut() else { return };
    map.remove("transfers");
    let Some(command) = map.get_mut("command").and_then(Value::as_object_mut) else { return };
    if command.get("kind").and_then(Value::as_str) != Some("run") {
        return;
    }
    let Some(request) = command.get_mut("request").and_then(Value::as_object_mut) else { return };
    let keep = ["prompt", "harness", "model", "reasoning", "modelOptions", "sandbox", "access", "plan"];
    request.retain(|key, _| keep.contains(&key.as_str()));
    for (key, value) in [("cwd", json!(".")), ("resume", Value::Null), ("model", Value::Null), ("reasoning", Value::Null)] {
        request.entry(key).or_insert(value);
    }
    request.entry("sandbox").or_insert(json!("workspace-write"));
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
/// only among these, a project only its id and name; with `only_in`, a chat
/// only in that project.
fn clean(what: Kind, rows: &[Value], device: &str, taken: &Taken, only_in: Option<&str>, dropped: &mut Vec<String>) -> Vec<Value> {
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
                if let Some(only) = only_in
                    && obj.get("spaceId").and_then(Value::as_str) != Some(only)
                {
                    dropped.push(format!("Chats {id}: not in its project {only}"));
                    ids.remove(id);
                    continue;
                }
                for key in ["harnessSessionCwd", "harnessSessionId", "sourceContext", "checkoutId"] {
                    obj.remove(key);
                }
                if !obj.get("cwd").and_then(Value::as_str).is_some_and(project_relative) {
                    obj.remove("cwd");
                }
                // A worker's: only what the window shows of a chat.
                if only_in.is_some() {
                    obj = worker_chat(obj);
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

/// What of a worker's chat row shows: a fixed few fields, each of its
/// kind (text at most [`MAX_TEXT`] characters), its config only the
/// agent's settings; no `roomGen`, folder or anything else it adds. (Its
/// parent, among its own chats, is checked after.)
fn worker_chat(row: serde_json::Map<String, Value>) -> serde_json::Map<String, Value> {
    const TEXT: [&str; 8] = ["id", "title", "spaceId", "lastMessageAt", "lastMessagePreview", "createdAt", "lastSeenAt", "parentChatId"];
    const CONFIG: [&str; 7] = ["harness", "model", "reasoning", "modelOptions", "sandbox", "access", "plan"];
    let mut kept = serde_json::Map::new();
    for (key, value) in row {
        let value = match (key.as_str(), value) {
            (k, Value::String(text)) if TEXT.contains(&k) || k == "childKind" => Value::String(text.chars().take(MAX_TEXT).collect()),
            (k, Value::Null) if TEXT.contains(&k) || k == "childKind" => Value::Null,
            ("archived", Value::Bool(b)) => Value::Bool(b),
            ("config", Value::Object(config)) => Value::Object(config.into_iter().filter(|(k, _)| CONFIG.contains(&k.as_str())).collect()),
            ("config", Value::Null) => Value::Null,
            _ => continue,
        };
        kept.insert(key, value);
    }
    kept
}

/// The longest text of a worker's chat row shown.
const MAX_TEXT: usize = 2000;

/// Of a worker's session rows, those of a chat of it shown (`chats`).
fn of_chats_shown(sessions: &[Value], chats: &[Value]) -> Vec<Value> {
    let shown: HashSet<&str> = chats.iter().filter_map(|c| c["id"].as_str()).collect();
    sessions.iter().filter(|s| s["chatId"].as_str().is_some_and(|c| shown.contains(c))).cloned().collect()
}

/// `seen`, the paired servers before the workers: of an id both have, the
/// paired server's shows and is called.
fn paired_first(seen: &BTreeMap<String, Seen>) -> impl Iterator<Item = (&String, &Seen)> {
    let worker = |k: &String| k.starts_with(WORKER_PREFIX);
    seen.iter().filter(move |(k, _)| !worker(k)).chain(seen.iter().filter(move |(k, _)| worker(k)))
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
    for (_, s) in paired_first(seen) {
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

    /// A server refusing this side the same way at every redial is said
    /// once; a new reason, or a refusal after it answered or wasn't
    /// reached, is said again.
    #[test]
    fn a_refusal_is_said_once_per_change() {
        let mut r = Refusal::default();
        let said: Vec<bool> = ["granted nothing", "granted nothing", "granted nothing", "unpaired", "unpaired"].iter().map(|why| r.first(why)).collect();
        assert_eq!(said, [true, false, false, true, false]);
        r.clear();
        assert!(r.first("unpaired"));
    }

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
