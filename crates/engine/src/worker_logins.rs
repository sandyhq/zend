//! A worker's agents on this server's own subscription logins (plan step
//! 4d-4, for workers): the owner allows, per worker and provider, that its
//! Pi turns use this device's pi login (`zend worker login allow <worker>
//! <provider>`). Kept in the data dir, [`FILE`] (names only, 0600), never
//! synced, never answered over a connection; the methods are this device's
//! own.
//!
//! As a run of a worker's chat leaves for it ([`prepare_turn`], from the
//! server's `peer_view`) the server writes that one provider's `auth.json`
//! entry (access token and expiry, Codex's account id, `refresh` empty, no
//! other provider; refreshed by the host's pi first when it has less than
//! 10 minutes left: [`crate::peer_logins::HostLogin`]) to
//! `workers/<name>/conf/login/<provider>.json`:
//!
//! ```text
//! {data}/workers/<name>/                  0700, the server's: no other host user
//! {data}/workers/<name>/conf/             ro in the container at /etc/zend-worker
//! {data}/workers/<name>/conf/login/       0755 (the container's uid is "other")
//! {data}/workers/<name>/conf/login/anthropic.json   0644, replaced atomically
//! ```
//!
//! `conf` is the server's alone: the container sees it read-only, so
//! nothing in it can be a link the worker put there (4d-4's symlink
//! redirect). It has been bound into every worker since step 5, so a worker
//! made before gets its login without being made again, even while it
//! runs. The token never crosses the socket: no frame to or from the worker
//! carries it. The worker's engine copies what is there for its pi
//! ([`crate::worker::sync_logins`]).
//!
//! A login is lent for a turn, not for the allow: the server keeps each
//! turn it lent for ([`Lent`]) and takes the file back once that turn's
//! chat has settled ([`settle`], from the worker's session rows), or its
//! connection to the worker ends. One provider's login at a time: a run on
//! another provider while a turn on one is live is refused (correctness
//! first: taking the first one's back would break it), and a turn on a key
//! or on nothing takes back every login no live turn holds.
//!
//! Each worker has a lock ([`locked`]), held across a turn's check of the
//! allow, its refresh and its lend, and by every change of the allow, the
//! worker's stop and its removal: an `rm` during a refresh waits for it and
//! then removes what it lent. The file goes at once when the allow is
//! taken back (`rm`, which also ends the worker's live turns on it), when
//! the worker stops, starts or is removed ([`release`]), and at boot
//! ([`sweep`]).
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::peer_logins::{HostLogin, PeerLogins, SandboxAuth, lendable};

/// The book of allowed logins in the data dir: worker name → providers.
pub const FILE: &str = "worker-logins.json";

/// How long a lent turn's chat must show settled (not working) before its
/// login goes: a run queued behind it in the same chat starts within it.
pub const SETTLE_GRACE: Duration = Duration::from_secs(5);
/// A run the worker never shows working (refused there, lost) gives its
/// login back after this.
pub const START_WAIT: Duration = Duration::from_secs(60);

/// The allowed logins, by worker name.
pub fn book(data_dir: &Path) -> PeerLogins {
    PeerLogins::at(data_dir.join(FILE))
}

fn worker_dir(data_dir: &Path, name: &str) -> PathBuf {
    data_dir.join(crate::workers::DIR).join(name)
}

/// The folder a worker's lent logins are kept in, inside its `conf`.
pub fn login_dir(data_dir: &Path, name: &str) -> PathBuf {
    worker_dir(data_dir, name).join("conf").join(crate::worker::LOGIN_DIR)
}

/// What a turn of a worker's chat runs on.
#[derive(Debug, PartialEq, Eq)]
pub enum Credential {
    /// Nothing to lend: a scripted agent, or a model with no known provider
    /// (the worker runs it on what it has).
    Nothing,
    /// An API key the worker was made with (a Podman secret): it wins.
    Key(String),
    /// This device's login for the provider, its file written for the turn.
    Login(String),
}

/// A turn refused: what the worker's chat is told, and why (for this
/// device's log only). Neither holds a token.
#[derive(Debug)]
pub struct Refused {
    pub provider: Option<String>,
    pub why: String,
}

impl Refused {
    /// What the caller is told.
    pub fn told(&self) -> String {
        match &self.provider {
            Some(provider) => format!("no credential for {provider} on this device"),
            None => "no credential for its model on this device".into(),
        }
    }
}

/// A turn of a worker's chat running on a lent login, as its server
/// follows it.
struct Turn {
    provider: String,
    /// Its chat's session row as the run left: its last finished turn and
    /// that turn's outcome.
    before: (Value, Value),
    sent: Instant,
    started: bool,
    settled_since: Option<Instant>,
}

/// A worker's turns on lent logins now, by chat.
#[derive(Default)]
pub struct Lent {
    turns: HashMap<String, Turn>,
}

impl Lent {
    /// The providers a live turn runs on.
    fn held(&self) -> BTreeSet<String> {
        self.turns.values().map(|t| t.provider.clone()).collect()
    }
}

static LENT: LazyLock<Mutex<HashMap<(PathBuf, String), Arc<Mutex<Lent>>>>> = LazyLock::new(Default::default);

/// Run `f` holding worker `name`'s lock, with its live turns.
fn locked<T>(data_dir: &Path, name: &str, f: impl FnOnce(&mut Lent) -> T) -> T {
    let dir = std::fs::canonicalize(data_dir).unwrap_or_else(|_| data_dir.to_path_buf());
    let lent = LENT.lock().unwrap_or_else(|e| e.into_inner()).entry((dir, name.to_string())).or_default().clone();
    let mut lent = lent.lock().unwrap_or_else(|e| e.into_inner());
    f(&mut lent)
}

fn not_a_name(name: &str) -> String {
    format!("{:?} is not a worker's name", name.chars().take(40).collect::<String>())
}

/// Before a run of `worker`'s chat `chat` with `harness` on `model` leaves
/// for the worker (`session`: that chat's session row as this side has it
/// now): what it runs on, that provider's login file written and the turn
/// kept when it is a lent login; refused when there is neither a key nor
/// an allowed login, or a live turn of the worker's holds another
/// provider's login.
pub fn prepare_turn(
    data_dir: &Path,
    worker: &crate::workers::Worker,
    chat: &str,
    harness: zend_proto::HarnessId,
    model: Option<&str>,
    session: Option<&Value>,
    host: &HostLogin,
) -> Result<Credential, Refused> {
    let name = &worker.name;
    locked(data_dir, name, |lent| {
        let unheld = |lent: &Lent| take_back_unheld(data_dir, name, lent);
        if harness == zend_proto::HarnessId::Mock {
            unheld(lent);
            return Ok(Credential::Nothing);
        }
        let Some(provider) = crate::peer_keys::provider_of(harness, model) else {
            unheld(lent);
            return Ok(Credential::Nothing);
        };
        let refused = |why: String| Refused { provider: Some(provider.clone()), why };
        if worker.providers.iter().any(|p| p == &provider) {
            // Its key wins: no login of this device's for it.
            unheld(lent);
            return Ok(Credential::Key(provider));
        }
        let login = format!("allow it this device's {provider} login with `zend worker login allow {name} {provider}`");
        if harness != zend_proto::HarnessId::Pi || !lendable(&provider) {
            unheld(lent);
            return Err(refused(format!(
                "worker {name} was made with no {provider} API key (zend worker create --key {provider}){}",
                if lendable(&provider) { format!(", and only Pi runs on a lent login: {login}") } else { String::new() }
            )));
        }
        let book = book(data_dir);
        if !book.allowed(name, &provider) {
            unheld(lent);
            return Err(refused(format!("worker {name} has no {provider} API key and may not use this device's login for it: {login}")));
        }
        if let Some(other) = lent.held().into_iter().find(|p| p != &provider) {
            let chats: Vec<&str> = lent.turns.iter().filter(|(_, t)| t.provider == other).map(|(c, _)| c.as_str()).collect();
            return Err(refused(format!(
                "worker {name} has a turn running on this device's {other} login (chat {}): one provider's login at a time, this run can go once that turn ends",
                chats.join(", ")
            )));
        }
        let auth = host.sandbox_auth(&provider).map_err(|why| {
            unheld(lent);
            refused(why)
        })?;
        lend(data_dir, name, &provider, &auth).map_err(refused)?;
        // (Belt: the lock keeps the allow as it was checked.)
        if !book.allowed(name, &provider) {
            unheld(lent);
            return Err(refused(format!("worker {name}'s {provider} login was taken back as its turn started")));
        }
        let before = session.map_or((Value::Null, Value::Null), |row| (row["lastCompletedTurn"].clone(), row["lastTurnOutcome"].clone()));
        lent.turns.insert(
            chat.to_string(),
            Turn { provider: provider.clone(), before, sent: Instant::now(), started: false, settled_since: None },
        );
        unheld(lent);
        Ok(Credential::Login(provider))
    })
}

/// The worker's session rows `sessions` as of `now`: its lent turns whose
/// chat settled (not working for [`SETTLE_GRACE`] after it worked or
/// finished a turn, or never working within [`START_WAIT`]) end, and the
/// logins no live turn holds go. The turns ended, with their provider.
pub fn settle(data_dir: &Path, name: &str, sessions: &[Value], now: Instant) -> Vec<(String, String)> {
    locked(data_dir, name, |lent| {
        if lent.turns.is_empty() {
            return Vec::new();
        }
        let mut done = Vec::new();
        for (chat, turn) in lent.turns.iter_mut() {
            let row = sessions.iter().find(|s| s["chatId"].as_str() == Some(chat.as_str()));
            let working = row.is_some_and(|r| {
                matches!(r["status"].as_str(), Some("working" | "awaitingInput")) || r.get("waiting").is_some_and(|w| !w.is_null())
            });
            if working {
                turn.started = true;
                turn.settled_since = None;
                continue;
            }
            let moved = row.is_some_and(|r| (r["lastCompletedTurn"].clone(), r["lastTurnOutcome"].clone()) != turn.before);
            if turn.started || moved {
                let since = *turn.settled_since.get_or_insert(now);
                if now.saturating_duration_since(since) >= SETTLE_GRACE {
                    done.push(chat.clone());
                }
            } else if now.saturating_duration_since(turn.sent) >= START_WAIT {
                done.push(chat.clone());
            }
        }
        let ended: Vec<(String, String)> = done.into_iter().filter_map(|c| lent.turns.remove(&c).map(|t| (c, t.provider))).collect();
        if !ended.is_empty() {
            take_back_unheld(data_dir, name, lent);
        }
        ended
    })
}

/// Allow worker `name` this device's login for `provider` (held to its
/// lock; a login no live turn holds goes, as on every change).
pub fn allow(data_dir: &Path, name: &str, provider: &str) -> Result<(), String> {
    if !crate::workers::valid_name(name) {
        return Err(not_a_name(name));
    }
    locked(data_dir, name, |lent| {
        book(data_dir).allow(name, provider)?;
        take_back_unheld(data_dir, name, lent);
        Ok(())
    })
}

/// Take worker `name`'s allow for `provider` back: whether it had it, and
/// the chats whose live turns ran on it (theirs to end now). Its file goes
/// at once; a turn that was refreshing for it waits on the lock, then goes.
pub fn revoke(data_dir: &Path, name: &str, provider: &str) -> Result<(bool, Vec<String>), String> {
    if !crate::workers::valid_name(name) {
        return Err(not_a_name(name));
    }
    locked(data_dir, name, |lent| {
        let removed = book(data_dir).remove(name, provider)?;
        let mut ended: Vec<String> = lent.turns.iter().filter(|(_, t)| t.provider == provider).map(|(c, _)| c.clone()).collect();
        ended.sort();
        lent.turns.retain(|_, t| t.provider != provider);
        take_back(data_dir, name, Some(provider));
        Ok((removed, ended))
    })
}

/// Worker `name` stops, starts or can't be reached: none of its turns is
/// followed any more, every login lent to it goes. The chats that had one.
pub fn release(data_dir: &Path, name: &str) -> Vec<String> {
    if !crate::workers::valid_name(name) {
        return Vec::new();
    }
    locked(data_dir, name, |lent| {
        let mut chats: Vec<String> = lent.turns.drain().map(|(c, _)| c).collect();
        chats.sort();
        take_back(data_dir, name, None);
        chats
    })
}

/// Worker `name` is removed: [`release`], and its allows forgotten.
pub fn forget(data_dir: &Path, name: &str) -> Result<(), String> {
    if !crate::workers::valid_name(name) {
        return Err(not_a_name(name));
    }
    locked(data_dir, name, |lent| {
        lent.turns.clear();
        take_back(data_dir, name, None);
        book(data_dir).forget(name)
    })
}

#[cfg(unix)]
fn mine(path: &Path, what: &str) -> Result<std::fs::Metadata, String> {
    use std::os::unix::fs::MetadataExt as _;
    let meta = std::fs::symlink_metadata(path).map_err(|e| format!("its {what}: {e}"))?;
    // SAFETY: getuid can't fail.
    let me = unsafe { libc::getuid() };
    if !meta.file_type().is_dir() || meta.uid() != me {
        return Err(format!("its {what} is not a folder of this user's"));
    }
    Ok(meta)
}

/// Keep `auth` (one provider's) for worker `name`'s turn in its login
/// folder: every folder on the way checked to be this user's, none a
/// link, the worker's own 0700; the file written beside and renamed in.
#[cfg(unix)]
pub fn lend(data_dir: &Path, name: &str, provider: &str, auth: &SandboxAuth) -> Result<(), String> {
    use std::io::Write as _;
    use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _, PermissionsExt as _};
    if !crate::workers::valid_name(name) {
        return Err(not_a_name(name));
    }
    if !lendable(provider) {
        return Err(format!("{provider}: no login of this device's is lent for it"));
    }
    let dir = worker_dir(data_dir, name);
    let meta = mine(&dir, "worker's folder")?;
    if meta.permissions().mode() & 0o077 != 0 {
        return Err("its worker's folder is open to other users (not 0700)".into());
    }
    mine(&dir.join("conf"), "worker's conf folder")?;
    let login = login_dir(data_dir, name);
    match std::fs::DirBuilder::new().mode(0o755).create(&login) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(format!("its login folder could not be made: {e}")),
    }
    let meta = mine(&login, "login folder")?;
    if meta.permissions().mode() & 0o777 != 0o755 {
        std::fs::set_permissions(&login, std::fs::Permissions::from_mode(0o755)).map_err(|e| format!("its login folder: {e}"))?;
    }
    let target = login.join(format!("{provider}.json"));
    let temp = login.join(format!(".{provider}.json.{}", uuid::Uuid::new_v4().simple()));
    let written = (|| -> std::io::Result<()> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o644)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&temp)?;
        // Readable by the container's uid whatever this process's umask.
        file.set_permissions(std::fs::Permissions::from_mode(0o644))?;
        file.write_all(auth.bytes())?;
        file.sync_all()?;
        std::fs::rename(&temp, &target)
    })();
    if let Err(e) = written {
        let _ = std::fs::remove_file(&temp);
        return Err(format!("its login could not be kept for it: {e}"));
    }
    Ok(())
}

#[cfg(not(unix))]
pub fn lend(_: &Path, _: &str, _: &str, _: &SandboxAuth) -> Result<(), String> {
    Err("workers run on Linux only".into())
}

/// Take worker `name`'s lent login file for `provider` away (every
/// provider's with `None`), whatever turn holds it: how many files went.
pub fn take_back(data_dir: &Path, name: &str, provider: Option<&str>) -> usize {
    take_back_where(data_dir, name, |p| provider.is_none_or(|want| want == p))
}

/// The login files of worker `name` no live turn holds go.
fn take_back_unheld(data_dir: &Path, name: &str, lent: &Lent) -> usize {
    let held = lent.held();
    take_back_where(data_dir, name, |p| !held.contains(p))
}

fn take_back_where(data_dir: &Path, name: &str, gone: impl Fn(&str) -> bool) -> usize {
    if !crate::workers::valid_name(name) {
        return 0;
    }
    let Ok(entries) = std::fs::read_dir(login_dir(data_dir, name)) else {
        return 0;
    };
    entries
        .flatten()
        .filter(|entry| {
            let file = entry.file_name();
            let Some(file) = file.to_str() else { return false };
            // Its own files, and a temporary one left by a lend cut short.
            let provider = file.strip_suffix(".json").or_else(|| file.strip_prefix('.').and_then(|f| f.split(".json.").next()));
            provider.is_some_and(&gone)
        })
        .filter(|entry| std::fs::remove_file(entry.path()).is_ok())
        .count()
}

/// At boot: every worker's lent logins go (no turn of theirs is followed
/// yet). How many.
pub fn sweep(data_dir: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(data_dir.join(crate::workers::DIR)) else {
        return 0;
    };
    entries
        .flatten()
        .filter_map(|entry| entry.file_name().to_str().map(str::to_string))
        .map(|name| take_back(data_dir, &name, None))
        .sum()
}

/// The agent and model a run sent to a worker runs with: the run's own, else
/// what its chat was made with (`config`); Pi when neither names one (a
/// worker's default).
pub fn run_of(params: &serde_json::Value, config: &serde_json::Value) -> Option<(zend_proto::HarnessId, Option<String>)> {
    let command = &params["command"];
    if command["kind"].as_str() != Some("run") {
        return None;
    }
    let request = &command["request"];
    let pick = |key: &str| Some(&request[key]).filter(|v| !v.is_null()).or_else(|| Some(&config[key]).filter(|v| !v.is_null()));
    let harness = pick("harness").and_then(|h| serde_json::from_value(h.clone()).ok()).unwrap_or(zend_proto::HarnessId::Pi);
    let model = pick("model").and_then(|m| m.as_str()).filter(|m| !m.is_empty()).map(str::to_string);
    Some((harness, model))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    use std::os::unix::fs::PermissionsExt as _;

    const ACCESS: &str = "sk-ant-oat01-HOST-ACCESS";
    const REFRESH: &str = "sk-ant-ort01-HOST-REFRESH";

    fn now_ms() -> i64 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64
    }

    /// A host pi folder signed in to anthropic (`minutes` left) and codex,
    /// with an API key entry besides.
    fn host(dir: &Path, minutes: i64) -> HostLogin {
        std::fs::create_dir_all(dir).unwrap();
        let auth = json!({
            "anthropic": { "type": "oauth", "access": ACCESS, "refresh": REFRESH, "expires": now_ms() + minutes * 60_000 },
            "openai-codex": { "type": "oauth", "access": "codex-ACCESS", "refresh": "codex-REFRESH", "expires": now_ms() + 600 * 60_000, "accountId": "acct-1" },
            "openrouter": { "type": "api_key", "key": "sk-or-OTHER" },
        });
        std::fs::write(dir.join("auth.json"), serde_json::to_vec(&auth).unwrap()).unwrap();
        HostLogin { agent_dir: dir.to_path_buf(), pi: None }
    }

    /// A server's data dir with worker `name` as `Workers::create` leaves
    /// its folders (0700, conf 0755, run 0711).
    fn server(root: &Path, name: &str, providers: &[&str]) -> (PathBuf, crate::workers::Worker) {
        let data = root.join("data");
        let dir = data.join(crate::workers::DIR).join(name);
        for (folder, mode) in [(dir.clone(), 0o700), (dir.join("conf"), 0o755), (dir.join("run"), 0o711)] {
            std::fs::create_dir_all(&folder).unwrap();
            std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(mode)).unwrap();
        }
        let worker: crate::workers::Worker = serde_json::from_value(json!({
            "name": name, "key": "a4b3f3e0b0839d1c".repeat(4), "image": "i", "container": format!("zend-worker-{name}"),
            "providers": providers,
        }))
        .unwrap();
        (data, worker)
    }

    fn mode(path: &Path) -> u32 {
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    fn sha(path: &Path) -> Vec<u8> {
        use sha2::Digest as _;
        sha2::Sha256::digest(std::fs::read(path).unwrap()).to_vec()
    }

    /// (1) Without an allow and without a key a Pi turn is refused, told
    /// only "no credential for <provider> on this device"; allowed, it runs
    /// on the login. A scripted agent and a model with no provider are left
    /// as they were.
    #[test]
    fn a_workers_turn_runs_on_the_login_only_when_allowed() {
        let temp = tempfile::tempdir().unwrap();
        let host = host(&temp.path().join("pi"), 300);
        let (data, worker) = server(temp.path(), "w", &[]);
        let pi = zend_proto::HarnessId::Pi;
        let refused = prepare_turn(&data, &worker, "chat", pi, Some("anthropic/claude-haiku-4-5"), None, &host).unwrap_err();
        assert_eq!(refused.told(), "no credential for anthropic on this device");
        assert!(refused.why.contains("zend worker login allow w anthropic"), "{}", refused.why);
        assert!(!login_dir(&data, "w").join("anthropic.json").exists());
        // Another worker's allow, or another provider's, isn't this one's.
        book(&data).allow("other", "anthropic").unwrap();
        book(&data).allow("w", "openai-codex").unwrap();
        assert!(prepare_turn(&data, &worker, "chat", pi, Some("anthropic/claude-haiku-4-5"), None, &host).is_err());
        book(&data).allow("w", "anthropic").unwrap();
        assert_eq!(prepare_turn(&data, &worker, "chat", pi, Some("anthropic/claude-haiku-4-5"), None, &host).unwrap(), Credential::Login("anthropic".into()));
        assert!(login_dir(&data, "w").join("anthropic.json").is_file());
        // Only lendable providers can be allowed; one with no key is refused.
        assert!(book(&data).allow("w", "openai").is_err());
        assert_eq!(prepare_turn(&data, &worker, "chat", pi, Some("openai/gpt-5"), None, &host).unwrap_err().told(), "no credential for openai on this device");
        assert_eq!(prepare_turn(&data, &worker, "chat", zend_proto::HarnessId::Mock, Some("anthropic/x"), None, &host).unwrap(), Credential::Nothing);
        assert_eq!(prepare_turn(&data, &worker, "chat", pi, None, None, &host).unwrap(), Credential::Nothing);
    }

    /// (2) The file holds that one provider's access token and expiry (and
    /// Codex's account id), `refresh` empty; the host's file is unchanged.
    /// A token near its end is refreshed by the host's pi first.
    #[test]
    fn the_lent_file_holds_one_providers_access_token_and_no_refresh() {
        let temp = tempfile::tempdir().unwrap();
        let pi_dir = temp.path().join("pi");
        let host = host(&pi_dir, 300);
        let before = sha(&pi_dir.join("auth.json"));
        let (data, worker) = server(temp.path(), "w", &[]);
        book(&data).allow("w", "anthropic").unwrap();
        book(&data).allow("w", "openai-codex").unwrap();
        prepare_turn(&data, &worker, "chat", zend_proto::HarnessId::Pi, Some("anthropic/claude-haiku-4-5"), None, &host).unwrap();
        let anthropic = std::fs::read(login_dir(&data, "w").join("anthropic.json")).unwrap();
        // (One provider's at a time: that turn ends first.)
        release(&data, "w");
        prepare_turn(&data, &worker, "chat", zend_proto::HarnessId::Pi, Some("openai-codex/gpt-5.1-codex-mini"), None, &host).unwrap();
        std::fs::write(login_dir(&data, "w").join("anthropic.json"), anthropic).unwrap();
        let read = |p: &str| -> Value { serde_json::from_slice(&std::fs::read(login_dir(&data, "w").join(format!("{p}.json"))).unwrap()).unwrap() };
        let anthropic = read("anthropic");
        assert_eq!(anthropic.as_object().unwrap().keys().collect::<Vec<_>>(), ["anthropic"]);
        assert_eq!(anthropic["anthropic"]["refresh"], "");
        assert_eq!(anthropic["anthropic"]["access"], ACCESS);
        assert!(!anthropic.to_string().contains(REFRESH));
        let codex = read("openai-codex");
        assert_eq!(codex.as_object().unwrap().len(), 1);
        assert_eq!(codex["openai-codex"]["accountId"], "acct-1");
        assert_eq!(codex["openai-codex"]["refresh"], "");
        assert_eq!(sha(&pi_dir.join("auth.json")), before, "the host's auth.json changed");

        // Near its end: the host's pi refreshes it (a stand-in here).
        let pi = temp.path().join("pi-bin");
        std::fs::write(
            &pi,
            "#!/bin/sh\nexp=$(( $(date +%s) * 1000 + 3600000 ))\nprintf '{\"anthropic\":{\"type\":\"oauth\",\"access\":\"NEW-ACCESS\",\"refresh\":\"NEW-REFRESH\",\"expires\":%s}}' $exp > \"$PI_CODING_AGENT_DIR/auth.json\"\n",
        )
        .unwrap();
        std::fs::set_permissions(&pi, std::fs::Permissions::from_mode(0o755)).unwrap();
        release(&data, "w");
        let near = HostLogin { pi: Some(pi), ..self::host(&temp.path().join("pi2"), 5) };
        prepare_turn(&data, &worker, "chat", zend_proto::HarnessId::Pi, Some("anthropic/claude-haiku-4-5"), None, &near).unwrap();
        assert_eq!(read("anthropic")["anthropic"]["access"], "NEW-ACCESS");
        assert_eq!(read("anthropic")["anthropic"]["refresh"], "");
    }

    /// (3) Kept in the server's own folder, which the container sees only
    /// read-only: 0644 in a 0755 folder inside the worker's 0700 one, under
    /// `conf` (bound read-only). A folder on the way that is a link, or a
    /// worker folder open to others, is refused; nothing goes into `run`
    /// (the worker's).
    #[test]
    fn the_login_is_kept_in_the_servers_own_folder_and_no_link_is_followed() {
        let temp = tempfile::tempdir().unwrap();
        let host = host(&temp.path().join("pi"), 300);
        let (data, worker) = server(temp.path(), "w", &[]);
        book(&data).allow("w", "anthropic").unwrap();
        let turn = || prepare_turn(&data, &worker, "chat", zend_proto::HarnessId::Pi, Some("anthropic/claude-haiku-4-5"), None, &host);
        turn().unwrap();
        let dir = data.join(crate::workers::DIR).join("w");
        let file = dir.join("conf/login/anthropic.json");
        assert_eq!((mode(&dir), mode(&dir.join("conf/login")), mode(&file)), (0o700, 0o755, 0o644));
        assert_eq!(std::fs::read_dir(dir.join("run")).unwrap().count(), 0, "something written in the worker's run folder");
        let only: Vec<_> = std::fs::read_dir(dir.join("conf/login")).unwrap().flatten().map(|e| e.file_name()).collect();
        assert_eq!(only, ["anthropic.json"], "a temporary file left behind");

        // A link where its login folder is: refused, nothing written through it.
        let elsewhere = temp.path().join("elsewhere");
        std::fs::create_dir_all(&elsewhere).unwrap();
        std::fs::remove_dir_all(dir.join("conf/login")).unwrap();
        std::os::unix::fs::symlink(&elsewhere, dir.join("conf/login")).unwrap();
        let refused = turn().unwrap_err();
        assert_eq!(refused.told(), "no credential for anthropic on this device");
        assert_eq!(std::fs::read_dir(&elsewhere).unwrap().count(), 0, "written through a link");
        std::fs::remove_file(dir.join("conf/login")).unwrap();
        // A link at the file's name is replaced, not followed.
        std::fs::create_dir_all(dir.join("conf/login")).unwrap();
        let victim = temp.path().join("victim");
        std::fs::write(&victim, "keep").unwrap();
        std::os::unix::fs::symlink(&victim, dir.join("conf/login/anthropic.json")).unwrap();
        turn().unwrap();
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "keep");
        assert!(std::fs::symlink_metadata(&file).unwrap().file_type().is_file());
        // A worker folder others can enter: refused.
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(turn().unwrap_err().why.contains("0700"));
    }

    /// (4) Any change of the allow, taking all back (stop, rm) and the boot
    /// sweep remove the file; a key kept for the worker and provider wins
    /// (and removes a file left from before it).
    #[test]
    fn a_change_of_the_allow_stop_rm_and_boot_remove_the_file_and_a_key_wins() {
        let temp = tempfile::tempdir().unwrap();
        let host = host(&temp.path().join("pi"), 300);
        let (data, worker) = server(temp.path(), "w", &[]);
        let file = login_dir(&data, "w").join("anthropic.json");
        let lent = || {
            book(&data).allow("w", "anthropic").unwrap();
            prepare_turn(&data, &worker, "chat", zend_proto::HarnessId::Pi, Some("anthropic/claude-haiku-4-5"), None, &host).unwrap();
            assert!(file.exists());
        };
        lent();
        assert_eq!(take_back(&data, "w", Some("openai-codex")), 0);
        assert_eq!(revoke(&data, "w", "anthropic").unwrap(), (true, vec!["chat".to_string()]));
        assert!(!file.exists());
        lent();
        assert_eq!(release(&data, "w"), ["chat"], "stop and rm take every provider's");
        assert!(!file.exists());
        lent();
        let (_, other) = server(temp.path(), "v", &[]);
        book(&data).allow("v", "anthropic").unwrap();
        prepare_turn(&data, &other, "chat", zend_proto::HarnessId::Pi, Some("anthropic/claude-haiku-4-5"), None, &host).unwrap();
        assert_eq!(sweep(&data), 2);
        assert!(!file.exists());
        // A key wins: the turn runs on it and no login file stays.
        lent();
        release(&data, "w");
        crate::worker_logins::lend(&data, "w", "anthropic", &host.sandbox_auth("anthropic").unwrap()).unwrap();
        let keyed = crate::workers::Worker { providers: vec!["anthropic".into()], ..worker.clone() };
        assert_eq!(
            prepare_turn(&data, &keyed, "chat", zend_proto::HarnessId::Pi, Some("anthropic/claude-haiku-4-5"), None, &host).unwrap(),
            Credential::Key("anthropic".into())
        );
        assert!(!file.exists());
        // A refused turn leaves none either (taken back since, or a failed refresh).
        lent();
        revoke(&data, "w", "anthropic").unwrap();
        assert!(prepare_turn(&data, &worker, "chat", zend_proto::HarnessId::Pi, Some("anthropic/claude-haiku-4-5"), None, &host).is_err());
        assert!(!file.exists());
    }

    /// (5) No token in what a refusal says or logs, even when the host's pi
    /// prints one as it fails; `run_of` reads the run and changes nothing.
    #[test]
    fn no_token_in_a_refusal_and_the_run_is_read_not_changed() {
        let temp = tempfile::tempdir().unwrap();
        let pi = temp.path().join("pi-bin");
        std::fs::write(&pi, format!("#!/bin/sh\necho '{ACCESS} {REFRESH}'\necho '{ACCESS}' >&2\nexit 3\n")).unwrap();
        std::fs::set_permissions(&pi, std::fs::Permissions::from_mode(0o755)).unwrap();
        let host = HostLogin { pi: Some(pi), ..host(&temp.path().join("pi"), 5) };
        let (data, worker) = server(temp.path(), "w", &[]);
        book(&data).allow("w", "anthropic").unwrap();
        let refused = prepare_turn(&data, &worker, "chat", zend_proto::HarnessId::Pi, Some("anthropic/claude-haiku-4-5"), None, &host).unwrap_err();
        let all = format!("{} {} {refused:?}", refused.told(), refused.why);
        assert!(!all.contains(ACCESS) && !all.contains(REFRESH), "{all}");
        assert!(refused.why.contains("could not refresh"), "{all}");

        let params = json!({ "chatId": "c", "command": { "kind": "run", "request": { "prompt": "hi", "harness": "pi", "model": null } } });
        let config = json!({ "harness": "pi", "model": "anthropic/claude-haiku-4-5" });
        let copy = params.clone();
        assert_eq!(run_of(&params, &config), Some((zend_proto::HarnessId::Pi, Some("anthropic/claude-haiku-4-5".into()))));
        assert_eq!(params, copy);
        let own = json!({ "command": { "kind": "run", "request": { "harness": "mock", "model": "openai-codex/x" } } });
        assert_eq!(run_of(&own, &config), Some((zend_proto::HarnessId::Mock, Some("openai-codex/x".into()))));
        assert_eq!(run_of(&json!({ "command": { "kind": "interrupt" } }), &config), None);
        assert_eq!(run_of(&json!({ "command": { "kind": "run", "request": {} } }), &Value::Null), Some((zend_proto::HarnessId::Pi, None)));
    }

    /// (6) A worker made before (its entry with no login fields, its conf
    /// without a login folder) gets its login in the folder already bound
    /// into it read-only: no new mount, so no need to make it again.
    #[test]
    fn a_worker_made_before_gets_its_login_in_the_folder_it_already_has() {
        let temp = tempfile::tempdir().unwrap();
        let host = host(&temp.path().join("pi"), 300);
        let (data, _) = server(temp.path(), "old", &[]);
        let old: crate::workers::Worker =
            serde_json::from_value(json!({ "name": "old", "key": "b".repeat(64), "image": "i", "container": "c" })).unwrap();
        assert!(!login_dir(&data, "old").exists());
        book(&data).allow("old", "anthropic").unwrap();
        prepare_turn(&data, &old, "chat", zend_proto::HarnessId::Pi, Some("anthropic/claude-haiku-4-5"), None, &host).unwrap();
        let conf = data.join(crate::workers::DIR).join("old/conf");
        assert!(login_dir(&data, "old").starts_with(&conf));
        let mounts = crate::workers::mounts("old", &conf, &conf.with_file_name("run"));
        let bound = format!("type=bind,src={},dst=/etc/zend-worker,ro", conf.display());
        assert!(mounts.contains(&bound), "{mounts:?}");
        // (Its copy's tmpfs, though, comes only with a worker made again.)
        assert!(mounts.iter().any(|m| m.starts_with("type=tmpfs,dst=/run/zend-login,tmpfs-mode=0700")), "{mounts:?}");
        assert!(login_dir(&data, "old").join("anthropic.json").is_file());
    }

    fn files(data: &Path, name: &str) -> Vec<String> {
        let mut all: Vec<String> = std::fs::read_dir(login_dir(data, name))
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| e.file_name().to_str().map(str::to_string))
            .collect();
        all.sort();
        all
    }

    /// (Review 1) An `rm` while a turn's refresh runs waits for it (the
    /// worker's lock is held across check, refresh and lend) and then
    /// removes what it lent: no file stays, and the turn is one `rm` ends.
    #[test]
    fn an_rm_during_a_refresh_leaves_no_file_behind() {
        let temp = tempfile::tempdir().unwrap();
        let pi = temp.path().join("pi-bin");
        // A refresh that takes a second.
        std::fs::write(
            &pi,
            "#!/bin/sh\nsleep 1\nexp=$(( $(date +%s) * 1000 + 3600000 ))\nprintf '{\"anthropic\":{\"type\":\"oauth\",\"access\":\"NEW\",\"refresh\":\"R\",\"expires\":%s}}' $exp > \"$PI_CODING_AGENT_DIR/auth.json\"\n",
        )
        .unwrap();
        std::fs::set_permissions(&pi, std::fs::Permissions::from_mode(0o755)).unwrap();
        let host = HostLogin { pi: Some(pi), ..host(&temp.path().join("pi"), 5) };
        let (data, worker) = server(temp.path(), "w", &[]);
        allow(&data, "w", "anthropic").unwrap();
        let turn = {
            let (data, worker, host) = (data.clone(), worker.clone(), host.clone());
            std::thread::spawn(move || prepare_turn(&data, &worker, "c1", zend_proto::HarnessId::Pi, Some("anthropic/claude-haiku-4-5"), None, &host))
        };
        std::thread::sleep(Duration::from_millis(300));
        let (removed, ended) = revoke(&data, "w", "anthropic").unwrap();
        assert!(removed);
        assert_eq!(turn.join().unwrap().unwrap(), Credential::Login("anthropic".into()), "the turn's refresh didn't finish first");
        assert_eq!(ended, ["c1"], "rm didn't wait for the turn it raced");
        assert!(files(&data, "w").is_empty(), "a file lent during rm stayed: {:?}", files(&data, "w"));
        // And its next turn is refused.
        assert!(prepare_turn(&data, &worker, "c1", zend_proto::HarnessId::Pi, Some("anthropic/claude-haiku-4-5"), None, &host).is_err());
    }

    /// (Review 2) A turn gets its own provider's login and no other: a
    /// turn on one provider while another's is live is refused (never
    /// both lent at once); one on a key or on nothing takes back every
    /// login no live turn holds; the file goes once its turn settles.
    #[test]
    fn a_turn_gets_only_its_providers_login_and_it_goes_when_the_turn_settles() {
        let temp = tempfile::tempdir().unwrap();
        let host = host(&temp.path().join("pi"), 300);
        let (data, worker) = server(temp.path(), "w", &[]);
        allow(&data, "w", "anthropic").unwrap();
        allow(&data, "w", "openai-codex").unwrap();
        let pi = zend_proto::HarnessId::Pi;
        let t0 = Instant::now();
        prepare_turn(&data, &worker, "a", pi, Some("anthropic/claude-haiku-4-5"), None, &host).unwrap();
        assert_eq!(files(&data, "w"), ["anthropic.json"]);
        let refused = prepare_turn(&data, &worker, "b", pi, Some("openai-codex/gpt-5.6-luna"), None, &host).unwrap_err();
        assert_eq!(refused.told(), "no credential for openai-codex on this device");
        assert!(refused.why.contains("one provider's login at a time") && refused.why.contains("chat a"), "{}", refused.why);
        assert_eq!(files(&data, "w"), ["anthropic.json"], "both lent");
        // A leftover of another provider's goes with the next turn.
        lend(&data, "w", "openai-codex", &host.sandbox_auth("openai-codex").unwrap()).unwrap();
        prepare_turn(&data, &worker, "a", pi, Some("anthropic/claude-haiku-4-5"), None, &host).unwrap();
        assert_eq!(files(&data, "w"), ["anthropic.json"]);
        // A turn on nothing takes back only what no live turn holds.
        lend(&data, "w", "openai-codex", &host.sandbox_auth("openai-codex").unwrap()).unwrap();
        assert_eq!(prepare_turn(&data, &worker, "m", zend_proto::HarnessId::Mock, None, None, &host).unwrap(), Credential::Nothing);
        assert_eq!(files(&data, "w"), ["anthropic.json"]);

        // Settling: shown working, then idle for the grace, it goes.
        let row = |status: &str, done: Option<&str>| json!({ "chatId": "a", "status": status, "lastCompletedTurn": done });
        assert!(settle(&data, "w", &[], t0).is_empty());
        assert!(settle(&data, "w", &[row("working", None)], t0 + Duration::from_secs(1)).is_empty());
        assert!(settle(&data, "w", &[row("idle", Some("t1"))], t0 + Duration::from_secs(2)).is_empty());
        assert!(settle(&data, "w", &[row("idle", Some("t1"))], t0 + Duration::from_secs(3)).is_empty(), "gone before its grace");
        assert_eq!(files(&data, "w"), ["anthropic.json"]);
        // (A run queued behind it starts within the grace: it stays.)
        assert!(settle(&data, "w", &[row("working", Some("t1"))], t0 + Duration::from_secs(4)).is_empty());
        assert!(settle(&data, "w", &[row("idle", Some("t2"))], t0 + Duration::from_secs(5)).is_empty());
        assert_eq!(settle(&data, "w", &[row("idle", Some("t2"))], t0 + Duration::from_secs(11)), [("a".to_string(), "anthropic".to_string())]);
        assert!(files(&data, "w").is_empty(), "the file stayed after its turn");
        // Now the other provider's turn may run.
        prepare_turn(&data, &worker, "b", pi, Some("openai-codex/gpt-5.6-luna"), None, &host).unwrap();
        assert_eq!(files(&data, "w"), ["openai-codex.json"]);
        // Never shown working, never finished: it goes after START_WAIT.
        let late = Instant::now() + START_WAIT + Duration::from_secs(1);
        assert_eq!(settle(&data, "w", &[], late), [("b".to_string(), "openai-codex".to_string())]);
        assert!(files(&data, "w").is_empty());
        // A key wins and takes back what is left.
        lend(&data, "w", "anthropic", &host.sandbox_auth("anthropic").unwrap()).unwrap();
        let keyed = crate::workers::Worker { providers: vec!["openai-codex".into()], ..worker.clone() };
        assert_eq!(prepare_turn(&data, &keyed, "k", pi, Some("openai-codex/x"), None, &host).unwrap(), Credential::Key("openai-codex".into()));
        assert!(files(&data, "w").is_empty());
    }

    /// (Review 5) Every login function taking a worker's name refuses one
    /// that isn't (no path out of the workers' folder).
    #[test]
    fn a_name_that_isnt_a_workers_is_refused_by_every_login_function() {
        let temp = tempfile::tempdir().unwrap();
        let host = host(&temp.path().join("pi"), 300);
        let (data, _) = server(temp.path(), "w", &[]);
        let outside = data.join(crate::workers::DIR).join("w").join("conf").join(crate::worker::LOGIN_DIR);
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("anthropic.json"), "{}").unwrap();
        for bad in ["../workers/w", "w/../w", "W", "", "-w", &"a".repeat(40)] {
            assert!(allow(&data, bad, "anthropic").is_err(), "{bad}");
            assert!(revoke(&data, bad, "anthropic").is_err(), "{bad}");
            assert!(forget(&data, bad).is_err(), "{bad}");
            assert!(release(&data, bad).is_empty(), "{bad}");
            assert!(lend(&data, bad, "anthropic", &host.sandbox_auth("anthropic").unwrap()).is_err(), "{bad}");
            assert_eq!(take_back(&data, bad, None), 0, "{bad}");
        }
        assert!(book(&data).list().is_empty());
        assert!(outside.join("anthropic.json").exists());
    }
}
