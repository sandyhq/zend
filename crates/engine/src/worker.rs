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
//! - `work`, `socket` (optional): the project folder (`/work`) and the
//!   socket path (`/run/zend-worker/worker.sock`), for tests.
//!
//! On start it keeps the key in its data dir, writes `peers.json` with that
//! one server granted `sessions.run.unconfined:work` (whatever was there),
//! checks every blocked network is a reject route (else it won't start),
//! makes the project `work` at `/work`, and listens on the socket only: no
//! tailnet or phone listener, no edge, no release checks. What the server
//! may call is the peer link's ([`zend_rpc::peer_server_allowed`] for that
//! grant: engine info, harnesses, models, `Mutate` createChat, `QueueCommand`,
//! `WatchDocMessages`) and the peer gate's (only in project `work`, only
//! its own chats there).

use std::path::{Path, PathBuf};
use std::time::Duration;

/// The environment variable naming the worker's configuration folder; set,
/// this engine is a worker.
pub const DIR_ENV: &str = "ZEND_WORKER_DIR";
/// The worker's one project.
pub const PROJECT: &str = "work";
/// Its grant: run sessions in [`PROJECT`], unsandboxed (the container is the
/// sandbox).
pub const GRANT: &str = "sessions.run.unconfined:work";
/// Where the project folder is (a named volume).
pub const DEFAULT_WORK: &str = "/work";
/// Where the socket is (a folder of the server's, bound in).
pub const DEFAULT_SOCKET: &str = "/run/zend-worker/worker.sock";
/// The files in the configuration folder.
pub const KEY_FILE: &str = "node-key";
pub const SERVER_KEY_FILE: &str = "server-key";
pub const BLOCKED_FILE: &str = "blocked";
/// How long the engine waits for its blocked routes before it gives up.
const ROUTES_WAIT: Duration = Duration::from_secs(60);

/// A worker's configuration, as its server wrote it.
#[derive(Clone, Debug)]
pub struct Conf {
    pub dir: PathBuf,
    pub server_key: String,
    pub blocked: Vec<String>,
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
        Ok(Self {
            dir: dir.to_path_buf(),
            server_key,
            blocked,
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
            "grants": [GRANT],
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
        assert_eq!((conf.work.as_path(), conf.blocked.len()), (Path::new(DEFAULT_WORK), 2));
    }
}
