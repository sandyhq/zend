//! A paired server's chats on this device's own subscription logins (plan
//! step 4d-4): the owner allows, per server and provider, that its
//! sandboxed chats use the host's pi login (`zend peer login allow <server>
//! <provider>`). Kept in the data dir, `peer-logins.json` (names only,
//! owner-only 0600), never synced, never answered over a connection.
//!
//! The host keeps its login (pi's `auth.json`) and refreshes it itself:
//! before each turn the chat's sandbox gets an `auth.json` of its own in its
//! agent folder ([`AGENT_DIR`] in its state folder, its `PI_CODING_AGENT_DIR`)
//! with only that provider's access token and expiry, `refresh` empty, no other
//! provider. A host token with less than [`MIN_VALIDITY`] left is first
//! refreshed by the host's own pi, outside the sandbox
//! (`pi auth print-bearer-token --provider <p> --min-expiry 10m`: pi's own refresh,
//! under its own lock on its file; what it prints is dropped). A key kept
//! for the server and provider ([`crate::peer_keys`]) wins over the login.
//!
//! No token is ever in an error, a log line or a `Debug` of these types.
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
            blocking(|| self.refresh(provider))?;
            entry = self.entry(provider).ok_or_else(no_login)?;
            if !fresh(&entry) {
                return Err(format!(
                    "this device's {provider} login has less than {} minutes left and pi didn't refresh it: sign in again on this device (pi, /login)",
                    MIN_VALIDITY.as_secs() / 60
                ));
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

/// The sandbox's login file in its agent folder `dir`.
pub fn auth_file(dir: &Path) -> PathBuf {
    dir.join("auth.json")
}

/// Put `auth` in the agent folder `dir` (owner-only, replacing one there).
pub fn write(dir: &Path, auth: &SandboxAuth) -> Result<(), String> {
    crate::runner::write_private_atomic(&auth_file(dir), auth.bytes())
        .map_err(|e| format!("its login could not be put in its sandbox: {e}"))
}

/// Take the login out of the agent folder `dir` (and what pi made beside it).
/// Whether one was there.
pub fn remove(dir: &Path) -> bool {
    let file = auth_file(dir);
    let _ = std::fs::remove_dir_all(dir.join("auth.json.lock"));
    std::fs::remove_file(&file).is_ok()
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

    #[test]
    fn the_sandbox_file_is_written_owner_only_and_removed() {
        let dir = tempfile::tempdir().unwrap();
        let host = host(dir.path(), 300);
        let agent = dir.path().join("state").join(AGENT_DIR);
        write(&agent, &host.sandbox_auth("anthropic").unwrap()).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(std::fs::metadata(auth_file(&agent)).unwrap().permissions().mode() & 0o777, 0o600);
        }
        assert!(remove(&agent));
        assert!(!auth_file(&agent).exists());
        assert!(!remove(&agent));
    }
}
