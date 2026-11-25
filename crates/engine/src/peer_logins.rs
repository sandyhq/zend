//! A paired server's chats on this device's own subscription logins (plan
//! step 4d-4): the owner allows, per server and provider, that its
//! sandboxed chats use the host's pi login (`zend peer login allow <server>
//! <provider>`). Kept in the data dir, `peer-logins.json` (names only,
//! owner-only 0600), never synced, never answered over a connection.
//!
//! The host keeps its login (pi's `auth.json`) and refreshes it itself:
//! each run of the chat gets an `auth.json` of its own with only that
//! provider's access token and expiry, `refresh` empty, no other provider.
//! It is kept where no sandbox reaches, [`TOKENS_DIR`] in Zend's data (0700,
//! the file 0600, one per run), and shown to the run read-only as
//! `auth.json` in its agent folder ([`AGENT_DIR`] in its state folder, its
//! `PI_CODING_AGENT_DIR`; `zend_harness::sandbox::Sandbox::with_agent_file`).
//! The host never acts by a path inside the state folder: that folder is
//! the sandbox's to write, links included. The file goes when its run is
//! over (its [`LentFile`] dropped), when the login is taken back or the
//! server unpaired ([`take_back`]), when the chat is deleted, and at boot
//! ([`sweep`]).
//!
//! A host token with less than [`MIN_VALIDITY`] left is first refreshed by
//! the host's own pi, outside the sandbox (`pi auth print-bearer-token
//! --provider <p> --min-expiry 10m`: pi's own refresh, under its own lock on
//! its file; what it prints is dropped); a refresh that failed isn't tried
//! again for [`REFRESH_BACKOFF`]. A key kept for the server and provider
//! ([`crate::peer_keys`]) wins over the login.
//!
//! No token is ever in an error, a log line or a `Debug` of these types.
//! What a server is told when its chat has neither is only that there is
//! no credential for the provider here; the why is logged on this device.
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use serde_json::{Map, Value, json};

pub const FILE: &str = "peer-logins.json";

/// The providers whose pi login can be lent: those Pi signs in to with a
/// browser login and that run on an access token alone (checked with Pi
/// 1.5.2).
pub const PROVIDERS: [&str; 2] = ["anthropic", "openai-codex"];

/// The agent folder of a chat's sandbox, inside its state folder.
pub const AGENT_DIR: &str = "pi-agent";

/// The least a lent token has left when its turn starts.
pub const MIN_VALIDITY: Duration = Duration::from_secs(10 * 60);

/// How long the host's pi may take to refresh its login.
const REFRESH_TIMEOUT: Duration = Duration::from_secs(90);

/// How long a failed refresh is not tried again (a run in that time is
/// refused at once).
pub const REFRESH_BACKOFF: Duration = Duration::from_secs(60);

/// The folder in Zend's data the lent logins are kept in while their runs
/// last: no sandbox sees it.
pub const TOKENS_DIR: &str = "peer-login-tokens";

/// When a refresh last failed, by pi folder, pi and provider.
type Tried = (PathBuf, Option<PathBuf>, String);
static FAILED_REFRESH: Mutex<BTreeMap<Tried, std::time::Instant>> = Mutex::new(BTreeMap::new());

static LOCK: Mutex<()> = Mutex::new(());

/// server key → providers it may use this device's login for.
type Book = BTreeMap<String, BTreeSet<String>>;

#[derive(Clone, Debug)]
pub struct PeerLogins {
    path: PathBuf,
}

/// Whether a provider's login can be lent.
pub fn lendable(provider: &str) -> bool {
    PROVIDERS.contains(&provider)
}

impl PeerLogins {
    pub fn new(data_dir: &Path) -> Self {
        Self { path: data_dir.join(FILE) }
    }

    fn load(&self) -> Book {
        std::fs::read(&self.path)
            .ok()
            .and_then(|raw| serde_json::from_slice(&raw).ok())
            .unwrap_or_default()
    }

    fn change<T>(&self, f: impl FnOnce(&mut Book) -> T) -> Result<T, String> {
        let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut book = self.load();
        let out = f(&mut book);
        book.retain(|_, providers| !providers.is_empty());
        let bytes = serde_json::to_vec_pretty(&book).map_err(|e| e.to_string())?;
        crate::runner::write_private_atomic(&self.path, &bytes).map_err(|e| e.to_string())?;
        Ok(out)
    }

    /// Let `peer`'s sandboxed chats on `provider` models use this device's
    /// login for it.
    pub fn allow(&self, peer: &str, provider: &str) -> Result<(), String> {
        if !lendable(provider) {
            return Err(format!(
                "{provider}: only these providers' logins can be lent: {}",
                PROVIDERS.join(", ")
            ));
        }
        self.change(|book| {
            book.entry(peer.to_string()).or_default().insert(provider.to_string());
        })
    }

    /// Take the login for `provider` back from `peer`: whether it had it.
    pub fn remove(&self, peer: &str, provider: &str) -> Result<bool, String> {
        self.change(|book| book.get_mut(peer).is_some_and(|providers| providers.remove(provider)))
    }

    /// Take every login back from `peer` (it was unpaired).
    pub fn forget(&self, peer: &str) -> Result<(), String> {
        if !self.load().contains_key(peer) {
            return Ok(());
        }
        self.change(|book| {
            book.remove(peer);
        })
    }

    pub fn allowed(&self, peer: &str, provider: &str) -> bool {
        self.load().get(peer).is_some_and(|providers| providers.contains(provider))
    }

    /// Every server's providers.
    pub fn list(&self) -> BTreeMap<String, Vec<String>> {
        self.load().into_iter().map(|(peer, providers)| (peer, providers.into_iter().collect())).collect()
    }
}

/// Where this device keeps its pi login, and the pi that refreshes it.
#[derive(Clone, Debug)]
pub struct HostLogin {
    /// Pi's folder (its `PI_CODING_AGENT_DIR`), holding `auth.json`.
    pub agent_dir: PathBuf,
    /// The host's pi; `None`: none found, a token near its end can't be
    /// refreshed.
    pub pi: Option<PathBuf>,
}

/// The one login a sandbox gets: its `auth.json`'s bytes. Never shown.
pub struct SandboxAuth(Vec<u8>);

impl std::fmt::Debug for SandboxAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SandboxAuth(..)")
    }
}

impl SandboxAuth {
    pub fn bytes(&self) -> &[u8] {
        &self.0
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

impl HostLogin {
    /// The engine's own: `PI_CODING_AGENT_DIR` or `~/.pi/agent`, and the pi
    /// its pi driver would start.
    pub fn current() -> Self {
        let agent_dir = std::env::var_os("PI_CODING_AGENT_DIR")
            .filter(|dir| !dir.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| zend_harness::sandbox::Host::current().home.join(".pi/agent"));
        Self { agent_dir, pi: zend_harness::PiHarness::new().resolve_executable().ok() }
    }

    fn auth_path(&self) -> PathBuf {
        self.agent_dir.join("auth.json")
    }

    /// The host's `provider` entry, if it is a login (oauth) with an access
    /// token and an expiry. Read only; never changed here.
    fn entry(&self, provider: &str) -> Option<Map<String, Value>> {
        let raw = std::fs::read(self.auth_path()).ok()?;
        let file: Value = serde_json::from_slice(&raw).ok()?;
        let entry = file.get(provider)?.as_object()?;
        let ok = entry.get("type").and_then(Value::as_str) == Some("oauth")
            && entry.get("access").and_then(Value::as_str).is_some_and(|a| !a.is_empty())
            && entry.get("expires").and_then(Value::as_i64).is_some();
        ok.then(|| entry.clone())
    }

    /// Whether this device's pi is signed in to `provider` with a login.
    pub fn has(&self, provider: &str) -> bool {
        self.entry(provider).is_some()
    }

    /// The `auth.json` for a sandbox that may use the host's `provider`
    /// login: its access token, expiry (and Codex's account id), `refresh`
    /// empty, nothing else. A token with less than [`MIN_VALIDITY`] left is
    /// refreshed by the host's pi first; when that fails: why, never the token.
    pub fn sandbox_auth(&self, provider: &str) -> Result<SandboxAuth, String> {
        if !lendable(provider) {
            return Err(format!("{provider}: this device's login for it can't be lent"));
        }
        let no_login = || format!("this device's pi has no {provider} login to lend: sign in on this device (pi, /login)");
        let mut entry = self.entry(provider).ok_or_else(no_login)?;
        let fresh = |entry: &Map<String, Value>| {
            entry.get("expires").and_then(Value::as_i64).is_some_and(|expires| expires - now_ms() >= MIN_VALIDITY.as_millis() as i64)
        };
        if !fresh(&entry) {
            let tried: Tried = (self.agent_dir.clone(), self.pi.clone(), provider.to_string());
            let failed_lately = FAILED_REFRESH
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(&tried)
                .is_some_and(|at| at.elapsed() < REFRESH_BACKOFF);
            if failed_lately {
                return Err(format!(
                    "this device's {provider} login is about to expire and pi couldn't refresh it less than {}s ago: sign in again on this device (pi, /login)",
                    REFRESH_BACKOFF.as_secs()
                ));
            }
            let refreshed = blocking(|| self.refresh(provider)).and_then(|()| {
                let entry = self.entry(provider).ok_or_else(no_login)?;
                if !fresh(&entry) {
                    return Err(format!(
                        "this device's {provider} login has less than {} minutes left and pi didn't refresh it: sign in again on this device (pi, /login)",
                        MIN_VALIDITY.as_secs() / 60
                    ));
                }
                Ok(entry)
            });
            let mut failed = FAILED_REFRESH.lock().unwrap_or_else(|e| e.into_inner());
            match refreshed {
                Ok(fresh_entry) => {
                    failed.remove(&tried);
                    entry = fresh_entry;
                }
                Err(why) => {
                    failed.insert(tried, std::time::Instant::now());
                    return Err(why);
                }
            }
        }
        let mut lent = json!({
            "type": "oauth",
            "refresh": "",
            "access": entry["access"],
            "expires": entry["expires"],
        });
        if provider == "openai-codex" {
            let account = entry
                .get("accountId")
                .and_then(Value::as_str)
                .filter(|a| !a.is_empty())
                .ok_or_else(|| format!("this device's {provider} login has no account id: sign in again on this device (pi, /login)"))?;
            lent["accountId"] = account.into();
        }
        let bytes = serde_json::to_vec_pretty(&json!({ provider: lent })).map_err(|e| e.to_string())?;
        Ok(SandboxAuth(bytes))
    }

    /// Have the host's pi refresh its `provider` login, outside any sandbox:
    /// `pi auth print-bearer-token --provider <p> --min-expiry 10m` refreshes it
    /// under pi's own lock and keeps it in its file. What it prints (the
    /// token, its errors) is dropped; only its exit is told.
    fn refresh(&self, provider: &str) -> Result<(), String> {
        let pi = self.pi.as_ref().ok_or_else(|| {
            format!("this device's {provider} login is about to expire and there is no pi here to refresh it")
        })?;
        let mut child = std::process::Command::new(pi)
            .args(["auth", "print-bearer-token", "--provider", provider, "--min-expiry"])
            .arg(format!("{}m", MIN_VALIDITY.as_secs() / 60))
            .env("PI_CODING_AGENT_DIR", &self.agent_dir)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| format!("this device's pi could not start to refresh its {provider} login: {e}"))?;
        let deadline = std::time::Instant::now() + REFRESH_TIMEOUT;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if std::time::Instant::now() < deadline => std::thread::sleep(Duration::from_millis(100)),
                Ok(None) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!("this device's pi took too long to refresh its {provider} login"));
                }
                Err(e) => return Err(format!("this device's pi refreshing its {provider} login: {e}")),
            }
        };
        if !status.success() {
            return Err(format!(
                "this device's pi could not refresh its {provider} login ({status}): sign in again on this device (pi, /login)"
            ));
        }
        Ok(())
    }
}

/// Run `f`, which waits on a process, off the async workers when on a
/// multi-threaded runtime (a run starts from one).
fn blocking<T>(f: impl FnOnce() -> T) -> T {
    match tokio::runtime::Handle::try_current() {
        Ok(rt) if rt.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread => tokio::task::block_in_place(f),
        _ => f(),
    }
}

/// The name the lent login has in its run's agent folder (pi's).
pub const AUTH_FILE: &str = "auth.json";

/// The folder the lent logins are kept in: made owner-only (0700) if
/// missing; refused if it is anything but a folder of this user's.
fn tokens_dir(data_dir: &Path) -> Result<PathBuf, String> {
    let dir = data_dir.join(TOKENS_DIR);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt as _, MetadataExt as _, PermissionsExt as _};
        match std::fs::DirBuilder::new().mode(0o700).create(&dir) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(format!("its login's folder could not be made: {e}")),
        }
        let meta = std::fs::symlink_metadata(&dir).map_err(|e| format!("its login's folder: {e}"))?;
        // SAFETY: getuid can't fail.
        let me = unsafe { libc::getuid() };
        if !meta.file_type().is_dir() || meta.uid() != me {
            return Err("its login's folder is not a folder of this user's".into());
        }
        if meta.permissions().mode() & 0o777 != 0o700 {
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))
                .map_err(|e| format!("its login's folder: {e}"))?;
        }
    }
    #[cfg(not(unix))]
    std::fs::create_dir_all(&dir).map_err(|e| format!("its login's folder could not be made: {e}"))?;
    Ok(dir)
}

/// The chat a kept login's file belongs to: `<chat>.<run>.json`.
fn chat_of(name: &str) -> Option<&str> {
    name.strip_suffix(".json")?.rsplit_once('.').map(|(chat, _)| chat)
}

/// Keep `auth` for one run of `chat_id` in [`TOKENS_DIR`] (owner-only), to
/// be shown read-only in its sandbox; removed when the returned file is
/// dropped (its run is over).
pub fn lend(data_dir: &Path, chat_id: &str, auth: &SandboxAuth) -> Result<zend_harness::sandbox::LentFile, String> {
    if !crate::peer_chats::valid_chat_id(chat_id) {
        return Err("its chat id can't name a file".into());
    }
    let path = tokens_dir(data_dir)?.join(format!("{chat_id}.{}.json", uuid::Uuid::new_v4().simple()));
    crate::runner::write_private_atomic(&path, auth.bytes()).map_err(|e| format!("its login could not be kept for it: {e}"))?;
    Ok(zend_harness::sandbox::LentFile::new(path))
}

/// Take the logins kept for `chat_id`'s runs away (taken back, its server
/// unpaired, the chat deleted). How many there were. A run still live keeps
/// what its sandbox mounted until it ends (taking back ends it).
pub fn take_back(data_dir: &Path, chat_id: &str) -> usize {
    kept(data_dir).into_iter().filter(|(chat, _)| chat == chat_id).filter(|(_, path)| std::fs::remove_file(path).is_ok()).count()
}

/// Every login kept for a run: its chat and its file.
fn kept(data_dir: &Path) -> Vec<(String, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(data_dir.join(TOKENS_DIR)) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_str()?.to_string();
            Some((chat_of(&name)?.to_string(), entry.path()))
        })
        .collect()
}

/// The chats a login is kept for now.
pub fn lent_chats(data_dir: &Path) -> BTreeSet<String> {
    kept(data_dir).into_iter().map(|(chat, _)| chat).collect()
}

/// At boot: no run is live, so every kept login (one a crash left, and
/// what a write left half-done) goes. How many.
pub fn sweep(data_dir: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(data_dir.join(TOKENS_DIR)) else {
        return 0;
    };
    entries.flatten().filter(|entry| std::fs::remove_file(entry.path()).is_ok()).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ACCESS: &str = "sk-ant-oat01-HOST-ACCESS";
    const REFRESH: &str = "sk-ant-ort01-HOST-REFRESH";

    fn host(dir: &Path, minutes: i64) -> HostLogin {
        let auth = json!({
            "anthropic": { "type": "oauth", "access": ACCESS, "refresh": REFRESH, "expires": now_ms() + minutes * 60_000 },
            "openai-codex": { "type": "oauth", "access": "codex-ACCESS", "refresh": "codex-REFRESH", "expires": now_ms() + 600 * 60_000, "accountId": "acct-1" },
            "openrouter": { "type": "api_key", "key": "sk-or-OTHER" },
        });
        std::fs::write(dir.join("auth.json"), serde_json::to_vec(&auth).unwrap()).unwrap();
        HostLogin { agent_dir: dir.to_path_buf(), pi: None }
    }

    #[test]
    fn logins_are_allowed_per_server_and_lendable_provider() {
        let dir = tempfile::tempdir().unwrap();
        let logins = PeerLogins::new(dir.path());
        logins.allow("srv", "anthropic").unwrap();
        assert!(logins.allow("srv", "openai").unwrap_err().contains("anthropic, openai-codex"));
        assert!(logins.allowed("srv", "anthropic") && !logins.allowed("srv", "openai-codex") && !logins.allowed("x", "anthropic"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(dir.path().join(FILE)).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        logins.allow("srv", "openai-codex").unwrap();
        assert_eq!(logins.list()["srv"], ["anthropic", "openai-codex"]);
        assert!(logins.remove("srv", "anthropic").unwrap());
        assert!(!logins.remove("srv", "anthropic").unwrap());
        logins.forget("srv").unwrap();
        assert!(logins.list().is_empty());
    }

    /// (2): one provider, refresh empty, the host's file untouched.
    #[test]
    fn a_sandbox_gets_one_providers_access_token_and_no_refresh() {
        let dir = tempfile::tempdir().unwrap();
        let host = host(dir.path(), 300);
        let before = std::fs::read(dir.path().join("auth.json")).unwrap();
        let auth = host.sandbox_auth("anthropic").unwrap();
        let lent: Value = serde_json::from_slice(auth.bytes()).unwrap();
        let only = lent.as_object().unwrap();
        assert_eq!(only.keys().collect::<Vec<_>>(), ["anthropic"]);
        assert_eq!(lent["anthropic"]["refresh"], "");
        assert_eq!(lent["anthropic"]["access"], ACCESS);
        assert_eq!(lent["anthropic"]["type"], "oauth");
        assert!(!String::from_utf8_lossy(auth.bytes()).contains(REFRESH));
        assert_eq!(format!("{auth:?}"), "SandboxAuth(..)");
        let codex: Value = serde_json::from_slice(host.sandbox_auth("openai-codex").unwrap().bytes()).unwrap();
        assert_eq!(codex["openai-codex"]["accountId"], "acct-1");
        assert_eq!(codex["openai-codex"]["refresh"], "");
        assert_eq!(codex.as_object().unwrap().len(), 1);
        assert!(host.sandbox_auth("openrouter").is_err());
        assert_eq!(std::fs::read(dir.path().join("auth.json")).unwrap(), before, "the host's file is unchanged");
    }

    /// A token near its end is refreshed by the host's pi first (its own
    /// command, here a stand-in); a pi that fails refuses, without the token.
    #[cfg(unix)]
    #[test]
    fn a_token_near_its_end_is_refreshed_by_the_hosts_pi_or_refused() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().unwrap();
        let mut host = host(dir.path(), 5);
        assert!(host.sandbox_auth("anthropic").unwrap_err().contains("no pi here"));
        let pi = dir.path().join("pi");
        let log = dir.path().join("pi.args");
        std::fs::write(
            &pi,
            format!(
                "#!/bin/sh\necho \"$@\" > '{}'\necho \"$PI_CODING_AGENT_DIR\" >> '{}'\nexp=$(( $(date +%s) * 1000 + 3600000 ))\nprintf '{{\"anthropic\":{{\"type\":\"oauth\",\"access\":\"NEW-ACCESS\",\"refresh\":\"NEW-REFRESH\",\"expires\":%s}}}}' $exp > \"$PI_CODING_AGENT_DIR/auth.json\"\necho NEW-ACCESS\n",
                log.display(),
                log.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&pi, std::fs::Permissions::from_mode(0o755)).unwrap();
        host.pi = Some(pi.clone());
        let lent: Value = serde_json::from_slice(host.sandbox_auth("anthropic").unwrap().bytes()).unwrap();
        assert_eq!(lent["anthropic"]["access"], "NEW-ACCESS");
        assert_eq!(lent["anthropic"]["refresh"], "");
        let args = std::fs::read_to_string(&log).unwrap();
        assert_eq!(args, format!("auth print-bearer-token --provider anthropic --min-expiry 10m\n{}\n", dir.path().display()));

        let host = self::host(dir.path(), 5);
        std::fs::write(&pi, format!("#!/bin/sh\necho '{ACCESS}' >&2\nexit 3\n")).unwrap();
        let host = HostLogin { pi: Some(pi), ..host };
        let refused = host.sandbox_auth("anthropic").unwrap_err();
        assert!(refused.contains("could not refresh its anthropic login"), "{refused}");
        assert!(!refused.contains(ACCESS) && !refused.contains(REFRESH), "{refused}");

        std::fs::remove_file(dir.path().join("auth.json")).unwrap();
        assert!(host.sandbox_auth("anthropic").unwrap_err().contains("no anthropic login"));
    }

    /// A run's login is kept owner-only in a folder of its own (0700) and
    /// goes with its run; taking back and the boot sweep take what is left.
    #[test]
    fn a_lent_login_is_kept_owner_only_and_goes_with_its_run() {
        let dir = tempfile::tempdir().unwrap();
        let host = host(dir.path(), 300);
        let data = dir.path().join("data");
        std::fs::create_dir_all(&data).unwrap();
        let auth = host.sandbox_auth("anthropic").unwrap();
        let file = lend(&data, "chat-1", &auth).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(file.path()), 0o600);
            assert_eq!(mode(&data.join(TOKENS_DIR)), 0o700);
        }
        assert_eq!(std::fs::read(file.path()).unwrap(), auth.bytes());
        let path = file.path().to_path_buf();
        drop(file);
        assert!(!path.exists(), "gone with its run");

        let a = lend(&data, "chat-1", &auth).unwrap();
        let b = lend(&data, "chat-2", &auth).unwrap();
        assert_ne!(a.path(), lend(&data, "chat-1", &auth).unwrap().path(), "one file per run");
        assert_eq!(lent_chats(&data), BTreeSet::from(["chat-1".to_string(), "chat-2".to_string()]));
        assert_eq!(take_back(&data, "chat-1"), 1);
        assert!(!a.path().exists() && b.path().exists());
        std::mem::forget(b);
        assert_eq!(sweep(&data), 1);
        assert!(lent_chats(&data).is_empty());
        assert!(lend(&data, "../x", &auth).is_err());
    }

    /// A refresh that failed isn't tried again for a minute: the next run is
    /// refused at once, without starting the host's pi.
    #[cfg(unix)]
    #[test]
    fn a_failed_refresh_is_not_tried_again_for_a_minute() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().unwrap();
        let pi = dir.path().join("pi");
        let count = dir.path().join("pi.count");
        std::fs::write(&pi, format!("#!/bin/sh
echo x >> '{}'
exit 3
", count.display())).unwrap();
        std::fs::set_permissions(&pi, std::fs::Permissions::from_mode(0o755)).unwrap();
        let host = HostLogin { pi: Some(pi), ..host(dir.path(), 5) };
        let first = host.sandbox_auth("anthropic").unwrap_err();
        assert!(first.contains("could not refresh"), "{first}");
        let second = host.sandbox_auth("anthropic").unwrap_err();
        assert!(!second.contains(ACCESS) && !second.contains(REFRESH), "{second}");
        let started = std::fs::read_to_string(&count).unwrap().lines().count();
        assert_eq!(started, 1, "pi was started again within the minute");
        // Another provider's, or another pi folder's, is its own.
        let other = HostLogin { agent_dir: dir.path().join("elsewhere"), ..host.clone() };
        std::fs::create_dir_all(&other.agent_dir).unwrap();
        std::fs::copy(dir.path().join("auth.json"), other.agent_dir.join("auth.json")).unwrap();
        assert!(other.sandbox_auth("anthropic").is_err());
        assert_eq!(std::fs::read_to_string(&count).unwrap().lines().count(), 2);
    }
}
