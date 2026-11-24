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
    wake: Arc<Notify>,
}

impl PeerView {
    pub(crate) fn new(link: Weak<PeerLink>) -> Arc<Self> {
        Arc::new(Self {
            link,
            seen: watch::Sender::new(BTreeMap::new()),
            live: Mutex::default(),
            tasks: Mutex::default(),
            made: Mutex::default(),
            wake: Arc::new(Notify::new()),
        })
    }

    /// Follow the paired servers (looked at again every [`SCAN`]).
    pub(crate) fn start(self: &Arc<Self>) {
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
        self.seen.send_if_modified(|seen| seen.remove(key).is_some());
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
                    wait = RETRY_MAX;
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
        let link = self.link.upgrade().ok_or(Quiet::Gone)?;
        let peer = link.peers().into_iter().find(|p| p.key == key).ok_or(Quiet::Gone)?;
        let client = Arc::new(link.connect(key).await.map_err(|e| Quiet::Unreachable(e.to_string()))?);
        drop(link);
        let info = client.call(methods::ENGINE_INFO, json!({})).await.map_err(|e| match e {
            RpcError::Transport(_) | RpcError::Closed => Quiet::Unreachable(e.to_string()),
            other => Quiet::Refused(other.to_string()),
        })?;
        let device_id = info["deviceId"].as_str().filter(|d| !d.is_empty()).map(str::to_string);
        let Some(device_id) = device_id else {
            return Err(Quiet::Refused("it said no device id".into()));
        };
        let mut capabilities: Vec<Value> = info["capabilities"].as_array().cloned().unwrap_or_default();
        capabilities.push(json!("paired-server"));
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
                "capabilities": capabilities,
            })
        };
        let mut chats = client.subscribe_checked(methods::WATCH_CHATS, json!({})).await.ok();
        let mut sessions = client.subscribe_checked(methods::WATCH_SESSIONS, json!({})).await.ok();
        self.live.lock().unwrap().insert(key.to_string(), Live { client: client.clone() });
        self.seen.send_modify(|seen| {
            let was = seen.remove(key).filter(|s| s.device_id == device_id);
            let mut now = Seen {
                device_id: device_id.clone(),
                device: device(&since),
                projects: info["peer"]["projects"].as_array().cloned().unwrap_or_default(),
                since: was.as_ref().map_or_else(|| since.clone(), |w| w.since.clone()),
                // Unreadable now: none shown.
                chats: if chats.is_some() { was.as_ref().map(|w| w.chats.clone()).unwrap_or_default() } else { Vec::new() },
                sessions: if sessions.is_some() { was.map(|w| w.sessions).unwrap_or_default() } else { Vec::new() },
                spaces: Vec::new(),
            };
            now.rebuild_spaces();
            seen.insert(key.to_string(), now);
        });
        tracing::info!(peer = %key, device = %device_id, reads = chats.is_some(), "peer view: a paired server is shown");
        let mut beat = tokio::time::interval(HEARTBEAT);
        beat.tick().await;
        loop {
            tokio::select! {
                rows = next(&mut chats) => {
                    let Some(rows) = rows else { break };
                    self.update(key, |s| {
                        s.chats = rows.as_array().cloned().unwrap_or_default();
                        s.rebuild_spaces();
                    });
                }
                rows = next(&mut sessions) => {
                    let Some(rows) = rows else { break };
                    self.update(key, |s| s.sessions = rows.as_array().cloned().unwrap_or_default());
                }
                _ = beat.tick() => {
                    let at = chrono::Utc::now().to_rfc3339();
                    self.update(key, |s| s.device["lastSeenAt"] = json!(at));
                }
                () = client.closed() => break,
            }
        }
        Ok(())
    }

    fn update(&self, key: &str, change: impl FnOnce(&mut Seen)) {
        self.seen.send_modify(|seen| {
            if let Some(s) = seen.get_mut(key) {
                change(s);
            }
        });
    }

    // ---- routing ------------------------------------------------------------

    /// The paired server a call of this device's own goes to, if any: one
    /// whose device it names (`targetDeviceId`), or a chat or project of
    /// one (`chatId`, a `Mutate`'s `spaceId` or `deviceId`).
    fn target(&self, method: &str, params: &Value) -> Option<String> {
        let seen = self.seen.borrow();
        let by_device = |id: &str| seen.iter().find(|(_, s)| s.device_id == id).map(|(k, _)| k.clone());
        let text = |key: &str| params.get(key).and_then(Value::as_str).filter(|v| !v.is_empty());
        if let Some(device) = text("targetDeviceId") {
            return by_device(device);
        }
        if let Some(chat) = text("chatId") {
            if let Some((key, _)) = self.made.lock().unwrap().get(chat) {
                return Some(key.clone());
            }
            if let Some(key) = seen.iter().find(|(_, s)| s.chats.iter().any(|c| c["id"].as_str() == Some(chat))).map(|(k, _)| k.clone()) {
                return Some(key);
            }
        }
        if method == methods::MUTATE {
            if let Some(space) = text("spaceId")
                && let Some(key) = seen.iter().find(|(_, s)| s.spaces.iter().any(|p| p["id"].as_str() == Some(space))).map(|(k, _)| k.clone())
            {
                return Some(key);
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
        let key = self.target(method, params)?;
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
            Some("setChatConfig")
                if self.made.lock().unwrap().get(&chat).is_some_and(|(_, config)| *config == params["config"]) =>
            {
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
