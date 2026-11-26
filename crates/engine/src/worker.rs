//! The worker role: this engine runs in a container one Zend server made
//! for itself ([`crate::workers`], `zend worker create`), and answers that
//! server alone, over a unix socket, with sessions in `/work`.
//!
//! The container gets, read-only at `$ZEND_WORKER_DIR` (`/etc/zend-worker`):
//! - `node-key`: this worker's Ed25519 key (PKCS#8), made by its server,
//!   which pinned its public half;
//! - `server-key`: the public key of the one server it answers;
//! - `blocked`: the IPv4 networks it may not reach, one per line (the
//!   sandbox's [`zend_harness::sandbox::BLOCKED_NETWORKS`] and the host's
//!   own addresses and subnets); its server added an unreachable route for
//!   each before this engine started;
//! - `project` (optional): the id of its one project (`work-<its key's
//!   first 12 hex>`, written by its server on every start, so two workers'
//!   projects never share an id in the server's window); `work` without it;
//! - `work`, `socket` (optional): the project folder (`/work`) and the
//!   socket path (`/run/zend-worker/worker.sock`), for tests;
//! - `login/<provider>.json` (when its server's owner allowed it,
//!   [`crate::worker_logins`]): that provider's entry of the server's pi
//!   login, access token only (`refresh` empty), written by the server
//!   before each turn on it and taken away when the allow ends.
//!
//! Before each run its server queues, and within 2 s of each change, the
//! engine copies the lent logins for its pi ([`sync_logins`]): to a tmpfs
//! of its own at [`RUN_LOGIN_DIR`] (never the `/data` volume), read-only
//! (0444), one entry per lent provider and nothing else, pi's `auth.json`
//! a link to it; gone when none is lent, at start and on SIGTERM.
//!
//! On start it keeps the key in its data dir, writes `peers.json` with that
//! one server granted [`grants`] (`sessions.read`, and
//! `sessions.run.unconfined:<project>`; whatever was there), checks every
//! blocked network is a reject route (else it won't start), makes the
//! project (named `work`) at `/work`, and listens on the socket only: no
//! tailnet or phone listener, no edge, no release checks. What the server
//! may call is the peer link's ([`zend_rpc::peer_server_allowed`] for those
//! grants: engine info, harnesses, models, its chats and sessions, `Mutate`
//! createChat, `QueueCommand`, `WatchDocMessages`) and the peer gate's
//! (runs only in its project, chats and sessions only its own).

use std::path::{Path, PathBuf};
use std::time::Duration;

/// The environment variable naming the worker's configuration folder; set,
/// this engine is a worker.
pub const DIR_ENV: &str = "ZEND_WORKER_DIR";
/// The worker's one project: its name, and its id without a `project` file.
pub const PROJECT: &str = "work";
/// The file naming the project's id.
pub const PROJECT_FILE: &str = "project";

/// Its server's grants: read its chats and sessions (all of them are its
/// server's), run sessions in `project`, unsandboxed (the container is the
/// sandbox).
pub fn grants(project: &str) -> Vec<String> {
    vec!["sessions.read".to_string(), format!("sessions.run.unconfined:{project}")]
}
/// Where the project folder is (a named volume).
pub const DEFAULT_WORK: &str = "/work";
/// Where the socket is (a folder of the server's, bound in).
pub const DEFAULT_SOCKET: &str = "/run/zend-worker/worker.sock";
/// The files in the configuration folder.
pub const KEY_FILE: &str = "node-key";
pub const SERVER_KEY_FILE: &str = "server-key";
pub const BLOCKED_FILE: &str = "blocked";
/// The folder in the configuration folder holding the lent logins.
pub const LOGIN_DIR: &str = "login";
/// How long the engine waits for its blocked routes before it gives up.
const ROUTES_WAIT: Duration = Duration::from_secs(60);

/// A worker's configuration, as its server wrote it.
#[derive(Clone, Debug)]
pub struct Conf {
    pub dir: PathBuf,
    pub server_key: String,
    pub blocked: Vec<String>,
    /// Its project's id.
    pub project: String,
    pub work: PathBuf,
    pub socket: PathBuf,
}

/// The configuration folder, when this engine is a worker.
pub fn role() -> Option<PathBuf> {
    std::env::var_os(DIR_ENV).filter(|v| !v.is_empty()).map(PathBuf::from)
}

/// This engine's worker configuration, read once.
pub fn conf() -> Option<&'static Result<Conf, String>> {
    static CONF: std::sync::OnceLock<Option<Result<Conf, String>>> = std::sync::OnceLock::new();
    CONF.get_or_init(|| role().map(|dir| Conf::load(&dir))).as_ref()
}

impl Conf {
    pub fn load(dir: &Path) -> Result<Self, String> {
        let read = |name: &str| {
            std::fs::read_to_string(dir.join(name)).map_err(|e| format!("worker: {}: {e}", dir.join(name).display()))
        };
        let optional = |name: &str, default: &str| {
            std::fs::read_to_string(dir.join(name))
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| default.to_string())
        };
        let server_key = read(SERVER_KEY_FILE)?.trim().to_string();
        if !zend_rpc::peer_link::valid_key(&server_key) {
            return Err(format!("worker: {SERVER_KEY_FILE} isn't a server's key"));
        }
        let blocked: Vec<String> =
            read(BLOCKED_FILE)?.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect();
        if blocked.is_empty() {
            return Err(format!("worker: {BLOCKED_FILE} names no networks: not started"));
        }
        for net in &blocked {
            parse_net(net).ok_or_else(|| format!("worker: {BLOCKED_FILE}: not an IPv4 network: {net}"))?;
        }
        let project = optional(PROJECT_FILE, PROJECT);
        if !crate::peer_chats::valid_chat_id(&project) || project.len() > 64 {
            return Err(format!("worker: {PROJECT_FILE}: not a project id: {project}"));
        }
        Ok(Self {
            dir: dir.to_path_buf(),
            server_key,
            blocked,
            project,
            work: PathBuf::from(optional("work", DEFAULT_WORK)),
            socket: PathBuf::from(optional("socket", DEFAULT_SOCKET)),
        })
    }

    /// Keep the worker's key and its one server in `data_dir`, replacing what
    /// was there: no other server is ever paired here.
    pub fn install(&self, data_dir: &Path) -> Result<(), String> {
        std::fs::create_dir_all(data_dir).map_err(|e| e.to_string())?;
        let der = std::fs::read(self.dir.join(KEY_FILE)).map_err(|e| format!("worker: {KEY_FILE}: {e}"))?;
        zend_rpc::peer_link::NodeKey::from_pkcs8(&der).map_err(|e| format!("worker: {KEY_FILE}: {e}"))?;
        crate::runner::write_private_atomic(&data_dir.join(crate::peer_link::KEY_FILE), &der).map_err(|e| e.to_string())?;
        let peers = serde_json::json!({ "peers": [{
            "key": self.server_key,
            "name": "server",
            "grants": grants(&self.project),
            "pairedAt": crate::now_ms(),
        }]});
        let bytes = serde_json::to_vec_pretty(&peers).map_err(|e| e.to_string())?;
        crate::runner::write_private_atomic(&data_dir.join(crate::peer_link::PEERS_FILE), &bytes).map_err(|e| e.to_string())?;
        std::fs::create_dir_all(&self.work).map_err(|e| format!("worker: {}: {e}", self.work.display()))?;
        Ok(())
    }

    /// Wait until each blocked network is a reject route here (its server
    /// adds them as the container starts); an error once that took too long.
    pub async fn wait_for_routes(&self) -> Result<(), String> {
        let deadline = tokio::time::Instant::now() + ROUTES_WAIT;
        loop {
            let table = std::fs::read_to_string("/proc/net/route").unwrap_or_default();
            let missing = missing_routes(&self.blocked, &table);
            if missing.is_empty() {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(format!(
                    "worker: no unreachable route for {}: not started (the host could be reached)",
                    missing.join(", ")
                ));
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }
}

/// `a.b.c.d/n` as (network, mask), host order.
fn parse_net(net: &str) -> Option<(u32, u32)> {
    let (addr, bits) = net.split_once('/')?;
    let addr: std::net::Ipv4Addr = addr.parse().ok()?;
    let bits: u32 = bits.parse().ok().filter(|b| *b <= 32)?;
    let mask = if bits == 0 { 0 } else { u32::MAX << (32 - bits) };
    Some((u32::from(addr) & mask, mask))
}

/// The networks of `blocked` with no reject route (`RTF_REJECT`) of that
/// exact network in `table` (`/proc/net/route`).
pub fn missing_routes(blocked: &[String], table: &str) -> Vec<String> {
    const RTF_REJECT: u32 = 0x0200;
    let hex = |s: &str| u32::from_str_radix(s, 16).ok().map(|v| u32::from_be_bytes(v.to_le_bytes()));
    let rejects: Vec<(u32, u32)> = table
        .lines()
        .skip(1)
        .filter_map(|line| {
            let f: Vec<&str> = line.split_whitespace().collect();
            let (dest, flags, mask) = (hex(f.get(1)?)?, u32::from_str_radix(f.get(3)?, 16).ok()?, hex(f.get(7)?)?);
            (flags & RTF_REJECT != 0).then_some((dest, mask))
        })
        .collect();
    blocked
        .iter()
        .filter(|net| parse_net(net).is_none_or(|n| !rejects.contains(&n)))
        .cloned()
        .collect()
}

/// Where the worker keeps its pi's copy of the lent logins: a tmpfs of its
/// own (0700, its uid's), made by its server at create, so no token is on
/// a volume and none outlives the container's run. pi's `auth.json` in its
/// folder (on the `/data` volume) is a link to it.
pub const RUN_LOGIN_DIR: &str = "/run/zend-login";

/// The tmpfs for the logins' copy (`ZEND_WORKER_LOGIN_DIR`, for tests; else
/// [`RUN_LOGIN_DIR`]).
pub fn run_login_dir() -> PathBuf {
    std::env::var_os("ZEND_WORKER_LOGIN_DIR").filter(|d| !d.is_empty()).map(PathBuf::from).unwrap_or_else(|| PathBuf::from(RUN_LOGIN_DIR))
}

/// Whether `dir` is a folder of this user's, not a link, on a tmpfs, closed
/// to others: where a copy of a login may be.
#[cfg(target_os = "linux")]
fn private_tmpfs(dir: &Path) -> Result<(), String> {
    use std::os::unix::ffi::OsStrExt as _;
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
    let meta = std::fs::symlink_metadata(dir).map_err(|e| format!("{}: {e} (a worker made before its logins' tmpfs: make it again)", dir.display()))?;
    // SAFETY: getuid can't fail.
    if !meta.file_type().is_dir() || meta.uid() != unsafe { libc::getuid() } || meta.permissions().mode() & 0o077 != 0 {
        return Err(format!("{} is not a private folder of this user's", dir.display()));
    }
    let path = std::ffi::CString::new(dir.as_os_str().as_bytes()).map_err(|e| e.to_string())?;
    // SAFETY: a zeroed statfs is valid, and `path` is a C string.
    let mut fs: libc::statfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statfs(path.as_ptr(), &mut fs) } != 0 {
        return Err(format!("{}: {}", dir.display(), std::io::Error::last_os_error()));
    }
    if fs.f_type as i64 != libc::TMPFS_MAGIC as i64 {
        return Err(format!("{} is not a tmpfs: a login's copy stays off disk", dir.display()));
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn private_tmpfs(dir: &Path) -> Result<(), String> {
    Err(format!("{}: workers run on Linux only", dir.display()))
}

/// Whether pi's `auth.json` at `path` is Zend's: a link into `run_dir`, or
/// a file of only lent entries (an empty `refresh`), or of none.
fn ours(path: &Path, run_dir: &Path) -> bool {
    use serde_json::Value;
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => std::fs::read_link(path).is_ok_and(|to| to.starts_with(run_dir)),
        Ok(meta) if meta.file_type().is_file() => std::fs::read(path)
            .ok()
            .and_then(|raw| serde_json::from_slice::<Value>(&raw).ok())
            .and_then(|file| file.as_object().cloned())
            .is_some_and(|file| file.values().all(|e| e.get("refresh").and_then(Value::as_str) == Some(""))),
        _ => false,
    }
}

/// Copy the logins its server lent (`conf/login/<provider>.json`) for pi:
/// one entry per lent provider, only its type, access token, expiry and
/// account id, `refresh` empty, written to `run_dir` (a private tmpfs,
/// [`RUN_LOGIN_DIR`]) beside and renamed in, read-only (0444: pi reads it
/// and has nothing to refresh), and pi's `auth.json` in `agent_dir` a link
/// to it. None lent: the copy goes. The providers copied.
pub fn sync_logins(conf_dir: &Path, agent_dir: &Path, run_dir: &Path) -> Result<Vec<String>, String> {
    use serde_json::{Map, Value};
    let mut merged = Map::new();
    if let Ok(entries) = std::fs::read_dir(conf_dir.join(LOGIN_DIR)) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(provider) = name.to_str().and_then(|n| n.strip_suffix(".json")) else { continue };
            if !crate::peer_logins::lendable(provider) {
                continue;
            }
            let Some(lent) = std::fs::read(entry.path())
                .ok()
                .and_then(|raw| serde_json::from_slice::<Value>(&raw).ok())
                .and_then(|file| file.get(provider).and_then(Value::as_object).cloned())
            else {
                continue;
            };
            if lent.get("type").and_then(Value::as_str) != Some("oauth") || !lent.get("access").is_some_and(Value::is_string) {
                continue;
            }
            let mut kept: Map<String, Value> =
                lent.into_iter().filter(|(k, _)| matches!(k.as_str(), "type" | "access" | "expires" | "accountId")).collect();
            kept.insert("refresh".into(), Value::String(String::new()));
            merged.insert(provider.to_string(), Value::Object(kept));
        }
    }
    let copy = run_dir.join(crate::peer_logins::AUTH_FILE);
    let link = agent_dir.join(crate::peer_logins::AUTH_FILE);
    if merged.is_empty() {
        remove(&copy)?;
        // A copy of round one's (on the volume) goes too; the link stays.
        if ours(&link, run_dir) && !std::fs::symlink_metadata(&link).is_ok_and(|m| m.file_type().is_symlink()) {
            remove(&link)?;
        }
        return Ok(Vec::new());
    }
    if std::fs::symlink_metadata(&link).is_ok() && !ours(&link, run_dir) {
        return Err(format!("worker: pi's {} is not Zend's: the lent logins aren't copied over it", link.display()));
    }
    private_tmpfs(run_dir).map_err(|why| format!("worker: the lent logins aren't copied: {why}"))?;
    let providers: Vec<String> = merged.keys().cloned().collect();
    let bytes = serde_json::to_vec_pretty(&Value::Object(merged)).map_err(|e| e.to_string())?;
    write_read_only(run_dir, &copy, &bytes).map_err(|e| format!("worker: its pi's auth.json: {e}"))?;
    link_to(agent_dir, &link, &copy).map_err(|e| format!("worker: its pi's auth.json: {e}"))?;
    Ok(providers)
}

/// The worker stops (SIGTERM) or starts: its copy of the lent logins goes,
/// and pi's `auth.json` with it when it is Zend's. How many files went.
pub fn take_back_logins(agent_dir: &Path, run_dir: &Path) -> usize {
    let copy = run_dir.join(crate::peer_logins::AUTH_FILE);
    let link = agent_dir.join(crate::peer_logins::AUTH_FILE);
    let mut gone = usize::from(std::fs::remove_file(&copy).is_ok());
    if ours(&link, run_dir) && std::fs::remove_file(&link).is_ok() {
        gone += 1;
    }
    gone
}

fn remove(path: &Path) -> Result<(), String> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("worker: {}: {e}", path.display())),
    }
}

/// pi's `auth.json` at `link` made a link to `to` (made beside, renamed in).
#[cfg(unix)]
fn link_to(dir: &Path, link: &Path, to: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt as _;
    if std::fs::read_link(link).is_ok_and(|now| now == to) {
        return Ok(());
    }
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    let temp = dir.join(format!(".auth.json.{}", uuid::Uuid::new_v4().simple()));
    std::os::unix::fs::symlink(to, &temp)?;
    std::fs::rename(&temp, link).inspect_err(|_| {
        let _ = std::fs::remove_file(&temp);
    })
}

#[cfg(not(unix))]
fn link_to(_: &Path, _: &Path, _: &Path) -> std::io::Result<()> {
    Err(std::io::Error::other("workers run on Linux only"))
}

#[cfg(unix)]
fn write_read_only(dir: &Path, path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
    let temp = dir.join(format!(".auth.json.{}", uuid::Uuid::new_v4().simple()));
    let written = (|| {
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).custom_flags(libc::O_NOFOLLOW).open(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        file.set_permissions(std::fs::Permissions::from_mode(0o444))?;
        std::fs::rename(&temp, path)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    written
}

#[cfg(not(unix))]
fn write_read_only(_: &Path, path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    std::fs::write(path, bytes)
}

/// How often the worker looks whether its server lent or took back a login.
pub const LOGIN_POLL: Duration = Duration::from_secs(2);

/// What the login folder holds now: each file's name, size and change time.
fn logins_seen(conf_dir: &Path) -> Vec<(std::ffi::OsString, u64, Option<std::time::SystemTime>)> {
    let mut seen: Vec<_> = std::fs::read_dir(conf_dir.join(LOGIN_DIR))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.metadata().ok().map(|m| (e.file_name(), m.len(), m.modified().ok())))
        .collect();
    seen.sort();
    seen
}

/// Keep pi's `auth.json` in step with what the server lent: copied now (a
/// copy left from before a stop goes when nothing is lent), and again
/// within [`LOGIN_POLL`] of each change, so a login taken back leaves pi's
/// folder too, not only at the next run.
pub fn follow_logins(conf_dir: PathBuf, agent_dir: PathBuf, run_dir: PathBuf) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        // What a run before left goes first (its server lends anew).
        let gone = take_back_logins(&agent_dir, &run_dir);
        if gone > 0 {
            tracing::info!(files = gone, "worker: logins lent before this start removed");
        }
        let mut last = None;
        loop {
            let now = logins_seen(&conf_dir);
            if last.as_ref() != Some(&now) {
                match sync_logins(&conf_dir, &agent_dir, &run_dir) {
                    Ok(providers) => tracing::info!(providers = %providers.join(","), "worker: lent logins in step with its server's"),
                    Err(why) => tracing::warn!(%why, "worker: the lent logins could not be copied for pi"),
                }
                last = Some(now);
            }
            tokio::time::sleep(LOGIN_POLL).await;
        }
    })
}

/// pi's folder in this worker (its `PI_CODING_AGENT_DIR`, as its pi reads it).
pub fn agent_dir() -> PathBuf {
    std::env::var_os("PI_CODING_AGENT_DIR")
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| zend_harness::sandbox::Host::current().home.join(".pi/agent"))
}

/// What a worker answers its server with: the engine's methods, the lent
/// logins copied for pi ([`sync_logins`]) before each run is queued.
pub struct WorkerRpc {
    pub inner: std::sync::Arc<dyn zend_rpc::RpcService>,
    pub conf_dir: PathBuf,
    pub agent_dir: PathBuf,
    /// Where the logins' copy is ([`run_login_dir`]).
    pub run_dir: PathBuf,
}

#[async_trait::async_trait]
impl zend_rpc::RpcService for WorkerRpc {
    async fn handle(&self, method: &str, params: serde_json::Value) -> Result<zend_rpc::RpcReply, zend_rpc::RpcError> {
        if method == zend_rpc::methods::QUEUE_COMMAND && params["command"]["kind"].as_str() == Some("run") {
            match sync_logins(&self.conf_dir, &self.agent_dir, &self.run_dir) {
                Ok(providers) if !providers.is_empty() => {
                    tracing::info!(providers = %providers.join(","), "worker: its server's lent logins copied for pi (access tokens only)");
                }
                Ok(_) => {}
                Err(why) => tracing::warn!(%why, "worker: the lent logins could not be copied for pi"),
            }
        }
        self.inner.handle(method, params).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A container's table as pasta and its server leave it: only routes
    /// that reject count.
    #[test]
    fn a_blocked_network_counts_only_as_a_reject_route_of_that_network() {
        let table = "Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT\n\
            eth0\t00000000\t0102FEA9\t0003\t0\t0\t0\t00000000\t0\t0\t0\n\
            *\t0000000A\t00000000\t0201\t0\t0\t0\t000000FF\t0\t0\t0\n\
            *\t00004064\t00000000\t0201\t0\t0\t0\t0000C0FF\t0\t0\t0\n\
            *\t555E3598\t00000000\t0205\t0\t0\t0\tFFFFFFFF\t0\t0\t0\n\
            eth0\t0002FEA9\t00000000\t0001\t0\t0\t0\tFCFFFFFF\t0\t0\t0\n";
        let nets = |l: &[&str]| l.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(missing_routes(&nets(&["10.0.0.0/8", "100.64.0.0/10", "152.53.94.85/32"]), table).is_empty());
        // A route that forwards (169.254.2.0/30 on eth0) isn't a block.
        assert_eq!(missing_routes(&nets(&["169.254.2.0/30", "172.16.0.0/12", "10.0.0.0/16"]), table), nets(&["169.254.2.0/30", "172.16.0.0/12", "10.0.0.0/16"]));
        assert_eq!(missing_routes(&nets(&["10.0.0.0/8"]), ""), nets(&["10.0.0.0/8"]));
    }

    #[test]
    fn a_conf_without_its_server_or_blocked_networks_doesnt_load() {
        let dir = tempfile::tempdir().unwrap();
        assert!(Conf::load(dir.path()).is_err());
        let key = zend_rpc::peer_link::NodeKey::generate().unwrap();
        std::fs::write(dir.path().join(SERVER_KEY_FILE), key.public()).unwrap();
        std::fs::write(dir.path().join(BLOCKED_FILE), "").unwrap();
        assert!(Conf::load(dir.path()).unwrap_err().contains("no networks"));
        std::fs::write(dir.path().join(BLOCKED_FILE), "10.0.0.0/8\nbogus\n").unwrap();
        assert!(Conf::load(dir.path()).unwrap_err().contains("bogus"));
        std::fs::write(dir.path().join(BLOCKED_FILE), "10.0.0.0/8\n100.64.0.0/10\n").unwrap();
        let conf = Conf::load(dir.path()).unwrap();
        assert_eq!((conf.work.as_path(), conf.blocked.len(), conf.project.as_str()), (Path::new(DEFAULT_WORK), 2, PROJECT));
        std::fs::write(dir.path().join(PROJECT_FILE), "work-0123abcd\n").unwrap();
        assert_eq!(Conf::load(dir.path()).unwrap().project, "work-0123abcd");
        std::fs::write(dir.path().join(PROJECT_FILE), "../x").unwrap();
        assert!(Conf::load(dir.path()).unwrap_err().contains("not a project id"));
    }

    /// (2, worker side) What its server lent reaches pi's auth.json: each
    /// lent provider's access token alone, refresh empty, read-only; a stray
    /// refresh token, other fields or files not of a lendable provider are
    /// dropped; none lent, Zend's copy goes (a file of the worker's own
    /// stays).
    /// A private tmpfs for a login's copy, as the container's
    /// `/run/zend-login` is (`/dev/shm` here).
    #[cfg(target_os = "linux")]
    fn tmpfs() -> tempfile::TempDir {
        let dir = tempfile::tempdir_in("/dev/shm").expect("/dev/shm, a tmpfs");
        std::fs::set_permissions(dir.path(), std::os::unix::fs::PermissionsExt::from_mode(0o700)).unwrap();
        dir
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn the_lent_logins_reach_pis_auth_json_read_only() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().unwrap();
        let run = tmpfs();
        let (conf, agent) = (dir.path().join("conf"), dir.path().join("pi"));
        std::fs::create_dir_all(conf.join(LOGIN_DIR)).unwrap();
        let lent = serde_json::json!({ "anthropic": { "type": "oauth", "access": "A", "refresh": "LEAKED", "expires": 1, "other": "x" } });
        std::fs::write(conf.join(LOGIN_DIR).join("anthropic.json"), lent.to_string()).unwrap();
        std::fs::write(conf.join(LOGIN_DIR).join("openai.json"), r#"{"openai":{"type":"oauth","access":"B"}}"#).unwrap();
        std::fs::write(conf.join(LOGIN_DIR).join("openai-codex.json"), r#"{"anthropic":{"type":"oauth","access":"C"}}"#).unwrap();
        assert_eq!(sync_logins(&conf, &agent, run.path()).unwrap(), ["anthropic"]);
        let path = agent.join("auth.json");
        let auth: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(auth, serde_json::json!({ "anthropic": { "type": "oauth", "access": "A", "refresh": "", "expires": 1 } }));
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o444);
        // Again, replaced (it is read-only, its folder isn't).
        assert_eq!(sync_logins(&conf, &agent, run.path()).unwrap(), ["anthropic"]);
        assert_eq!(std::fs::read_dir(run.path()).unwrap().count(), 1, "a temporary file left behind");
        assert_eq!(std::fs::read_dir(&agent).unwrap().count(), 1, "a temporary link left behind");
        std::fs::remove_file(conf.join(LOGIN_DIR).join("anthropic.json")).unwrap();
        assert!(sync_logins(&conf, &agent, run.path()).unwrap().is_empty());
        assert!(!path.exists());
        assert_eq!(std::fs::read_dir(run.path()).unwrap().count(), 0, "the copy stayed");
        std::fs::remove_file(&path).unwrap();
        std::fs::write(&path, r#"{"anthropic":{"type":"oauth","access":"OWN","refresh":"OWN-R"}}"#).unwrap();
        assert!(sync_logins(&conf, &agent, run.path()).unwrap().is_empty());
        assert!(path.exists(), "the worker's own login removed");
        // Nor is a lent one written over it.
        std::fs::write(conf.join(LOGIN_DIR).join("anthropic.json"), lent.to_string()).unwrap();
        assert!(sync_logins(&conf, &agent, run.path()).is_err());
        assert!(std::fs::read_to_string(&path).unwrap().contains("OWN-R"));
    }

    /// (Review 4) The worker's copy of a lent login is on a tmpfs of its
    /// own, never on pi's folder (the `/data` volume): pi's `auth.json`
    /// there is only a link to it. A folder that isn't a private tmpfs
    /// gets no copy. At SIGTERM and at start the copy and the link go, and a
    /// copy an older worker left on the volume with them.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_workers_copy_is_on_its_tmpfs_never_the_volume_and_goes_at_stop_and_start() {
        let volume = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).unwrap();
        let run = tmpfs();
        let (conf, agent) = (volume.path().join("conf"), volume.path().join("pi"));
        std::fs::create_dir_all(conf.join(LOGIN_DIR)).unwrap();
        std::fs::write(conf.join(LOGIN_DIR).join("openai-codex.json"), r#"{"openai-codex":{"type":"oauth","access":"A","refresh":"","expires":1,"accountId":"x"}}"#)
            .unwrap();
        // Not a tmpfs: no copy anywhere.
        let disk = volume.path().join("run");
        std::fs::create_dir(&disk).unwrap();
        std::fs::set_permissions(&disk, std::os::unix::fs::PermissionsExt::from_mode(0o700)).unwrap();
        let refused = sync_logins(&conf, &agent, &disk).unwrap_err();
        assert!(refused.contains("not a tmpfs") || refused.contains("private"), "{refused}");
        assert!(!agent.join("auth.json").exists() && std::fs::read_dir(&disk).unwrap().count() == 0);
        // On the tmpfs: the volume holds a link only.
        assert_eq!(sync_logins(&conf, &agent, run.path()).unwrap(), ["openai-codex"]);
        let link = agent.join("auth.json");
        assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
        assert_eq!(std::fs::read_link(&link).unwrap(), run.path().join("auth.json"));
        let on_volume: Vec<_> = walk(volume.path()).into_iter().filter(|p| std::fs::symlink_metadata(p).unwrap().is_file() && std::fs::read_to_string(p).unwrap().contains("\"access\"")).collect();
        assert_eq!(on_volume, [conf.join(LOGIN_DIR).join("openai-codex.json")], "a token on the volume");
        // SIGTERM: both go.
        assert_eq!(take_back_logins(&agent, run.path()), 2);
        assert!(std::fs::symlink_metadata(&link).is_err() && std::fs::read_dir(run.path()).unwrap().count() == 0);
        // A copy an older worker left on its volume goes at start.
        std::fs::write(&link, r#"{"openai-codex":{"type":"oauth","access":"OLD","refresh":"","expires":1}}"#).unwrap();
        assert_eq!(take_back_logins(&agent, run.path()), 1);
        assert!(!link.exists());
    }

    #[cfg(target_os = "linux")]
    fn walk(dir: &Path) -> Vec<PathBuf> {
        let mut all = Vec::new();
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if entry.file_type().unwrap().is_dir() {
                all.extend(walk(&path));
            } else {
                all.push(path);
            }
        }
        all
    }

    /// (4, worker side) A login taken back leaves pi's folder within a poll,
    /// with no run needed; one lent shows up the same way.
    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn a_login_taken_back_leaves_pis_folder_without_a_run() {
        let dir = tempfile::tempdir().unwrap();
        let run = tmpfs();
        let (conf, agent) = (dir.path().join("conf"), dir.path().join("pi"));
        std::fs::create_dir_all(conf.join(LOGIN_DIR)).unwrap();
        let lent = conf.join(LOGIN_DIR).join("openai-codex.json");
        std::fs::write(&lent, r#"{"openai-codex":{"type":"oauth","access":"A","refresh":"","expires":1,"accountId":"x"}}"#).unwrap();
        let follow = follow_logins(conf.clone(), agent.clone(), run.path().to_path_buf());
        let auth = agent.join("auth.json");
        let until = |want: bool| {
            let auth = auth.clone();
            async move {
                for _ in 0..50 {
                    if auth.exists() == want {
                        return true;
                    }
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                false
            }
        };
        assert!(until(true).await, "the lent login didn't reach pi");
        std::fs::remove_file(&lent).unwrap();
        assert!(until(false).await, "the login taken back stayed in pi's folder");
        follow.abort();
    }
}
