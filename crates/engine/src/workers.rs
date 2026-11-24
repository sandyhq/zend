//! Workers: Podman containers this server makes for itself, each running
//! this engine in the worker role ([`crate::worker`]) with a project of
//! its own (`/work`, a named volume). A worker answers this server alone
//! and opens no connection to Zend or to this server: the server dials it.
//!
//! ```text
//! {data}/workers.json                 [{name, key, image, container, providers, createdAt}]
//! {data}/workers/<name>/             0700, the server's user's
//! {data}/workers/<name>/conf/         ro at /etc/zend-worker: node-key, server-key, blocked
//! {data}/workers/<name>/run/          at /run/zend-worker: worker.sock
//! ```
//!
//! - As whom. The container's engine runs as uid 1000 (the image's `node`),
//!   which rootless Podman maps to a subordinate uid of the server's user:
//!   not the server's user, and nobody else's. It reads `conf` (0755/0644)
//!   and makes its socket in `run` (0777); the 0700 folder around both
//!   keeps every other user of the host out.
//!
//! - Who. The server makes the worker's Ed25519 key, gives it its own
//!   public key (`server-key`) and pins the worker's (`key` in
//!   workers.json). Both prove their keys on every dial, as on the peer
//!   link ([`zend_rpc::peer_link::connect_peer_unix`]); the worker's book
//!   holds only this server, granted `sessions.run.unconfined:work`.
//! - Where. A unix socket in a folder of this server's data dir, bound into
//!   the container: no port, on loopback or the tailnet, that another
//!   local user or a tailnet node could reach.
//! - The network. pasta, IPv4 only, no gateway mapping to the host
//!   (`--no-map-gw`, no forwarded ports), the sandbox's addresses
//!   (169.254.2.2/30, DNS through 169.254.2.1); then, before the container's
//!   first process runs (`podman init`), an unreachable route in its
//!   namespace for every network of [`blocked_networks`] (private, the
//!   tailnet's 100.64/10, link-local and cloud metadata, the host's own
//!   addresses and subnets, docker's bridges among them). No capabilities
//!   (`--cap-drop all`, `no-new-privileges`): nothing inside changes a
//!   route. The worker's engine checks the routes are there before it serves.
//! - Keys. API keys for its agents are Podman secrets (`zend-worker-<name>-<provider>`),
//!   given as the provider's variable (`ANTHROPIC_API_KEY`, …); no login
//!   file of the owner's goes in.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::process::Command;
use zend_rpc::peer_link::NodeKey;
use zend_rpc::{RpcClient, RpcError};

/// The workers' file in the data dir.
pub const FILE: &str = "workers.json";
/// Their folders in the data dir.
pub const DIR: &str = "workers";
/// The image `infra/worker/build.sh` builds.
pub const DEFAULT_IMAGE: &str = "localhost/zend-worker:latest";
/// How long a started worker has to answer on its socket.
const START_TIMEOUT: Duration = Duration::from_secs(90);

/// pasta's options: IPv4 only, the sandbox's addresses, no address mapped
/// to the host, no ports forwarded either way.
pub const PASTA: &str = "pasta:-4,--no-map-gw,-a,169.254.2.2,-n,30,-g,169.254.2.1,--dns-forward,169.254.2.1,-t,none,-u,none,-T,none,-U,none";

/// One worker, as its server keeps it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Worker {
    pub name: String,
    /// Its public key, pinned.
    pub key: String,
    pub image: String,
    pub container: String,
    /// Providers it has an API key for (the keys are Podman secrets).
    #[serde(default)]
    pub providers: Vec<String>,
    #[serde(default)]
    pub created_at: i64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Saved {
    #[serde(default)]
    workers: Vec<Worker>,
}

/// What `create` takes.
#[derive(Clone, Debug, Default)]
pub struct NewWorker {
    pub name: String,
    pub image: Option<String>,
    /// (provider, API key).
    pub keys: Vec<(String, String)>,
}

/// Every network a worker may not reach: the sandbox's list and the host's
/// own addresses and subnets, each once.
pub fn blocked_networks() -> Vec<String> {
    let mut blocked: Vec<String> = zend_harness::sandbox::BLOCKED_NETWORKS.iter().map(|s| s.to_string()).collect();
    blocked.extend(zend_harness::sandbox::host_networks());
    let mut seen = std::collections::BTreeSet::new();
    blocked.retain(|net| seen.insert(net.clone()));
    blocked
}

fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    name.len() <= 32
        && chars.next().is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn container_of(name: &str) -> String {
    format!("zend-worker-{name}")
}

fn volume_of(name: &str, what: &str) -> String {
    format!("zend-worker-{name}-{what}")
}

fn secret_of(name: &str, provider: &str) -> String {
    format!("zend-worker-{name}-{provider}")
}

/// The workers of the engine whose data dir this is.
pub struct Workers {
    data_dir: PathBuf,
    podman: String,
}

impl Workers {
    pub fn new(data_dir: &Path) -> Self {
        Self { data_dir: data_dir.to_path_buf(), podman: std::env::var("ZEND_PODMAN").unwrap_or_else(|_| "podman".into()) }
    }

    fn dir(&self, name: &str) -> PathBuf {
        self.data_dir.join(DIR).join(name)
    }

    /// The worker's socket, on this side.
    pub fn socket(&self, name: &str) -> PathBuf {
        self.dir(name).join("run").join("worker.sock")
    }

    pub fn list(&self) -> Vec<Worker> {
        std::fs::read(self.data_dir.join(FILE))
            .ok()
            .and_then(|raw| serde_json::from_slice::<Saved>(&raw).ok())
            .unwrap_or_default()
            .workers
    }

    fn save(&self, workers: Vec<Worker>) -> Result<(), String> {
        let bytes = serde_json::to_vec_pretty(&Saved { workers }).map_err(|e| e.to_string())?;
        crate::runner::write_private_atomic(&self.data_dir.join(FILE), &bytes).map_err(|e| format!("couldn't keep the workers: {e}"))
    }

    pub fn get(&self, name: &str) -> Result<Worker, String> {
        self.list().into_iter().find(|w| w.name == name).ok_or_else(|| format!("no worker {name}"))
    }

    async fn podman(&self, args: &[&str], stdin: Option<&str>) -> Result<String, String> {
        use tokio::io::AsyncWriteExt as _;
        let mut command = Command::new(&self.podman);
        command.args(args).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
        command.stdin(if stdin.is_some() { std::process::Stdio::piped() } else { std::process::Stdio::null() });
        let mut child = command.spawn().map_err(|e| format!("{}: {e}", self.podman))?;
        if let (Some(text), Some(mut input)) = (stdin, child.stdin.take()) {
            input.write_all(text.as_bytes()).await.map_err(|e| e.to_string())?;
        }
        let out = child.wait_with_output().await.map_err(|e| e.to_string())?;
        if !out.status.success() {
            return Err(format!(
                "podman {}: {}",
                args.first().copied().unwrap_or_default(),
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }

    /// Make a worker, start it and check it answers with the key it was given.
    pub async fn create(&self, new: NewWorker) -> Result<Worker, String> {
        let name = new.name.trim().to_string();
        if !valid_name(&name) {
            return Err("a worker's name is lowercase letters, digits and -, at most 32".into());
        }
        if self.list().iter().any(|w| w.name == name) {
            return Err(format!("worker {name} exists"));
        }
        let mut variables = Vec::new();
        for (provider, key) in &new.keys {
            let variable = zend_harness::sandbox::provider_key_var(provider)
                .ok_or_else(|| format!("{provider}: no API key variable known for it"))?;
            if key.trim().is_empty() {
                return Err(format!("no {provider} key given"));
            }
            variables.push((provider.clone(), variable, key.trim().to_string()));
        }
        let server = crate::peer_link::load_node_key(&self.data_dir)?;
        let der = NodeKey::generate_pkcs8()?;
        let key = NodeKey::from_pkcs8(&der)?;
        let dir = self.dir(&name);
        let (conf, run) = (dir.join("conf"), dir.join("run"));
        {
            use std::os::unix::fs::DirBuilderExt as _;
            use std::os::unix::fs::PermissionsExt as _;
            for (folder, mode) in [(&dir, 0o700), (&conf, 0o755), (&run, 0o777)] {
                std::fs::DirBuilder::new()
                    .recursive(true)
                    .mode(mode)
                    .create(folder)
                    .and_then(|()| std::fs::set_permissions(folder, std::fs::Permissions::from_mode(mode)))
                    .map_err(|e| format!("{}: {e}", folder.display()))?;
            }
        }
        // Readable by the container's uid (see "As whom").
        let write = |file: &str, bytes: &[u8]| {
            use std::os::unix::fs::PermissionsExt as _;
            let path = conf.join(file);
            crate::runner::write_private_atomic(&path, bytes)
                .map_err(|e| e.to_string())
                .and_then(|()| std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).map_err(|e| e.to_string()))
                .map_err(|e| format!("{file}: {e}"))
        };
        write(crate::worker::KEY_FILE, &der)?;
        write(crate::worker::SERVER_KEY_FILE, server.public().as_bytes())?;
        write(crate::worker::BLOCKED_FILE, (blocked_networks().join("\n") + "\n").as_bytes())?;

        let image = new.image.clone().unwrap_or_else(|| DEFAULT_IMAGE.to_string());
        let container = container_of(&name);
        let mut secrets = Vec::new();
        for (provider, variable, value) in &variables {
            let secret = secret_of(&name, provider);
            self.podman(&["secret", "create", "--replace", &secret, "-"], Some(value)).await?;
            secrets.push(format!("{secret},type=env,target={variable}"));
        }
        let mounts = [
            format!("type=volume,src={},dst=/work", volume_of(&name, "work")),
            format!("type=volume,src={},dst=/data", volume_of(&name, "data")),
            format!("type=bind,src={},dst=/etc/zend-worker,ro", conf.display()),
            format!("type=bind,src={},dst=/run/zend-worker", run.display()),
        ];
        let label = format!("zend.worker={name}");
        let mut args: Vec<&str> = vec![
            "create", "--name", &container, "--label", &label, "--user", "1000:1000",
            "--network", PASTA, "--dns", "169.254.2.1", "--dns-search", ".",
            "--cap-drop", "all", "--security-opt", "no-new-privileges",
            "--env", "ZEND_WORKER_DIR=/etc/zend-worker",
        ];
        for mount in &mounts {
            args.extend(["--mount", mount.as_str()]);
        }
        for secret in &secrets {
            args.extend(["--secret", secret.as_str()]);
        }
        args.push(&image);
        if let Err(err) = self.podman(&args, None).await {
            let _ = std::fs::remove_dir_all(&dir);
            return Err(err);
        }
        let worker = Worker {
            name: name.clone(),
            key: key.public().to_string(),
            image,
            container,
            providers: variables.iter().map(|(p, _, _)| p.clone()).collect(),
            created_at: crate::now_ms(),
        };
        let mut all = self.list();
        all.push(worker.clone());
        self.save(all)?;
        self.start(&name).await?;
        Ok(worker)
    }

    /// The container's state (`running`, `exited`, …), or why it can't say.
    pub async fn state(&self, name: &str) -> String {
        self.podman(&["inspect", "--format", "{{.State.Status}}", &container_of(name)], None)
            .await
            .unwrap_or_else(|e| format!("unknown ({e})"))
    }

    /// Start a worker that isn't running: its network made, the blocked
    /// routes added before its first process runs, then started, and
    /// answering with its pinned key.
    pub async fn start(&self, name: &str) -> Result<(), String> {
        let worker = self.get(name)?;
        if self.state(name).await != "running" {
            self.podman(&["init", &worker.container], None).await?;
            let pid = self.podman(&["inspect", "--format", "{{.State.Pid}}", &worker.container], None).await?;
            if pid.parse::<u32>().ok().filter(|p| *p > 0).is_none() {
                return Err(format!("worker {name}: no process to reach its network ({pid})"));
            }
            let blocked = std::fs::read_to_string(self.dir(name).join("conf").join(crate::worker::BLOCKED_FILE))
                .map_err(|e| format!("worker {name}: {e}"))?;
            let batch: String = blocked
                .lines()
                .map(str::trim)
                .filter(|n| !n.is_empty())
                .map(|net| format!("route replace unreachable {net}\n"))
                .collect();
            if let Err(err) = self.podman(&["unshare", "nsenter", "-t", &pid, "-n", "ip", "-4", "-batch", "-"], Some(&batch)).await {
                let _ = self.podman(&["stop", "-t", "0", &worker.container], None).await;
                return Err(format!("worker {name}: blocking its networks failed, not started: {err}"));
            }
            self.podman(&["start", &worker.container], None).await?;
        }
        let server = crate::peer_link::load_node_key(&self.data_dir)?;
        let deadline = tokio::time::Instant::now() + START_TIMEOUT;
        loop {
            let reply = match self.connect_as(&worker, &server).await {
                Ok(client) => client.call(zend_rpc::methods::ENGINE_INFO, serde_json::json!({})).await,
                Err(err) => Err(err),
            };
            match reply {
                Ok(_) => return Ok(()),
                Err(err) if tokio::time::Instant::now() >= deadline => {
                    return Err(format!("worker {name} didn't answer: {err} ({})", self.state(name).await));
                }
                Err(_) => tokio::time::sleep(Duration::from_millis(300)).await,
            }
        }
    }

    /// Dial a worker as this server.
    pub async fn connect(&self, name: &str) -> Result<RpcClient, RpcError> {
        let worker = self.get(name).map_err(RpcError::BadParams)?;
        let server = crate::peer_link::load_node_key(&self.data_dir).map_err(RpcError::Failed)?;
        self.connect_as(&worker, &server).await
    }

    async fn connect_as(&self, worker: &Worker, key: &NodeKey) -> Result<RpcClient, RpcError> {
        zend_rpc::peer_link::connect_peer_unix(&self.socket(&worker.name), key, &worker.key).await
    }

    /// Remove a worker: its container, its secrets, its folders and (unless
    /// `keep_volumes`) its volumes, /work among them.
    pub async fn remove(&self, name: &str, keep_volumes: bool) -> Result<Worker, String> {
        let worker = self.get(name)?;
        self.podman(&["rm", "--force", "--time", "5", "--ignore", &worker.container], None).await?;
        for provider in &worker.providers {
            let _ = self.podman(&["secret", "rm", &secret_of(name, provider)], None).await;
        }
        if !keep_volumes {
            for what in ["work", "data"] {
                let _ = self.podman(&["volume", "rm", "--force", &volume_of(name, what)], None).await;
            }
        }
        let _ = std::fs::remove_dir_all(self.dir(name));
        self.save(self.list().into_iter().filter(|w| w.name != name).collect())?;
        Ok(worker)
    }
}

/// `inner` with the workers' methods answered (this device's own only).
pub struct WorkersRpc {
    pub inner: std::sync::Arc<dyn zend_rpc::RpcService>,
    pub workers: Workers,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateParams {
    name: String,
    #[serde(default)]
    image: Option<String>,
    #[serde(default)]
    keys: std::collections::BTreeMap<String, String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NameParams {
    name: String,
    #[serde(default)]
    keep_volumes: bool,
}

#[async_trait::async_trait]
impl zend_rpc::RpcService for WorkersRpc {
    async fn handle(&self, method: &str, params: serde_json::Value) -> Result<zend_rpc::RpcReply, RpcError> {
        use serde_json::json;
        use zend_rpc::{RpcReply, methods};
        let given = || if params.is_null() { json!({}) } else { params.clone() };
        match method {
            methods::CREATE_WORKER | methods::LIST_WORKERS | methods::START_WORKER | methods::REMOVE_WORKER
                if !zend_rpc::caller().is_local() =>
            {
                Err(RpcError::Failed(format!("{method} is this device's own")))
            }
            methods::CREATE_WORKER => {
                let p: CreateParams = zend_rpc::parse_params(given())?;
                let worker = self
                    .workers
                    .create(NewWorker { name: p.name, image: p.image, keys: p.keys.into_iter().collect() })
                    .await
                    .map_err(RpcError::Failed)?;
                Ok(RpcReply::Value(serde_json::to_value(worker).map_err(|e| RpcError::Failed(e.to_string()))?))
            }
            methods::LIST_WORKERS => {
                let mut listed = Vec::new();
                for worker in self.workers.list() {
                    let state = self.workers.state(&worker.name).await;
                    let mut value = serde_json::to_value(&worker).map_err(|e| RpcError::Failed(e.to_string()))?;
                    value["state"] = state.into();
                    listed.push(value);
                }
                Ok(RpcReply::Value(json!({ "workers": listed })))
            }
            methods::START_WORKER => {
                let p: NameParams = zend_rpc::parse_params(given())?;
                self.workers.start(&p.name).await.map_err(RpcError::Failed)?;
                Ok(RpcReply::Value(json!({ "name": p.name, "state": self.workers.state(&p.name).await })))
            }
            methods::REMOVE_WORKER => {
                let p: NameParams = zend_rpc::parse_params(given())?;
                self.workers.remove(&p.name, p.keep_volumes).await.map_err(RpcError::Failed)?;
                Ok(RpcReply::Value(json!({ "name": p.name, "removed": true })))
            }
            _ => self.inner.handle(method, params).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_blocked_networks() {
        for good in ["a", "w1", "build-box", "0x"] {
            assert!(valid_name(good), "{good}");
        }
        for bad in ["", "-a", "A", "a_b", "a/b", "a.b", &"x".repeat(33)] {
            assert!(!valid_name(bad), "{bad}");
        }
        let blocked = blocked_networks();
        for net in ["10.0.0.0/8", "100.64.0.0/10", "169.254.0.0/16", "172.16.0.0/12", "192.168.0.0/16"] {
            assert!(blocked.iter().any(|b| b == net), "{net}");
        }
        assert!(!blocked.iter().any(|b| b.starts_with("127.")), "{blocked:?}");
    }
}
