//! Workers: Podman containers this server makes for itself, each running
//! this engine in the worker role ([`crate::worker`]) with a project of
//! its own (`/work`, a named volume). A worker answers this server alone
//! and opens no connection to Zend or to this server: the server dials it.
//!
//! ```text
//! {data}/workers.json                 [{name, key, project, image, container, providers, createdAt, limits}]
//! {data}/workers/<name>/             0700, the server's user's
//! {data}/workers/<name>/conf/         ro at /etc/zend-worker: node-key, server-key, blocked
//! {data}/workers/<name>/run/          0711, the worker's: at /run/zend-worker: worker.sock
//! ```
//!
//! - Trust. A worker is as trusted as its agent: everything from it is
//!   hostile input. Its server dials its socket only as itself (no
//!   symlink followed), takes no message over 16 MiB from it, and uses
//!   nothing it answers but that it answered (ENGINE_INFO, to know it runs).
//! - As whom. `--userns=auto`: the container has a user namespace of its
//!   own, from the server's user's subordinate ids (its root isn't the
//!   server's user). Its engine runs as uid 1000 (the image's `node`). It
//!   reads `conf` (0755/0644, the server's) and makes its socket in `run`
//!   (0711, chowned to it by Podman, `U`); the 0700 folder around both
//!   keeps every other user of the host out.
//! - Limits. Memory (no swap), CPUs and processes are capped per worker
//!   ([`Limits`], 4 GiB, 2 CPUs, 512 by default). /work's size isn't.
//!
//! - Who. The server makes the worker's Ed25519 key, gives it its own
//!   public key (`server-key`) and pins the worker's (`key` in
//!   workers.json). Both prove their keys on every dial, as on the peer
//!   link ([`zend_rpc::peer_link::connect_peer_unix`]); the worker's book
//!   holds only this server, granted `sessions.read` and
//!   `sessions.run.unconfined:<its project>` ([`project_of`]).
//! - Where. A unix socket in a folder of this server's data dir, bound into
//!   the container: no port, on loopback or the tailnet, that another
//!   local user or a tailnet node could reach.
//! - The network. pasta, IPv4 only, no gateway mapping to the host
//!   (`--no-map-gw`, no forwarded ports), the sandbox's addresses
//!   (169.254.2.2/30, DNS through 169.254.2.1, which pasta sends to a
//!   public resolver, [`DNS_HOST`], not the host's: no tailnet names);
//!   then, on every start, before the container's first process runs
//!   (`podman init`), an unreachable route in its namespace for every
//!   network of [`blocked_networks`] as they are then (private, the
//!   tailnet's 100.64/10, link-local and cloud metadata, the host's own
//!   addresses and subnets, docker's bridges among them), also written to
//!   `conf/blocked`. No capabilities (`--cap-drop all`,
//!   `no-new-privileges`): nothing inside changes a route. The worker's
//!   engine checks the routes are there before it serves. A running worker
//!   keeps the routes it started with: an address the host gains later is
//!   blocked from its next start on (stop and start it).
//! - Its sessions. Its project's id is its own (`work-<its key's first 12
//!   hex>`, `conf/project`, written again at every start: a worker made
//!   before had `work` and gets its own from its next start), so its
//!   server's window ([`crate::peer_view`]) shows each running worker as a
//!   device with one project, `work`, and sends a chat made there to it.
//!   Its device id there is its server's word too (`worker-<its key's
//!   first 12 hex>`, [`device_of`]), never what it says.
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
pub const PASTA: &str = "pasta:-4,--no-map-gw,-a,169.254.2.2,-n,30,-g,169.254.2.1,--dns-forward,169.254.2.1,--dns-host,1.1.1.1,-t,none,-u,none,-T,none,-U,none";

/// Where pasta sends a worker's DNS queries: a public resolver, never the
/// host's (on a tailnet host that is MagicDNS, which names tailnet nodes).
pub const DNS_HOST: &str = "1.1.1.1";

/// What a worker may use of its host.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Limits {
    /// Memory, as Podman takes it (`4g`, `512m`); no swap beyond it.
    pub memory: String,
    /// CPUs (`2`, `0.5`).
    pub cpus: String,
    /// Processes and threads.
    pub pids: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Self { memory: "4g".into(), cpus: "2".into(), pids: 512 }
    }
}

impl Limits {
    /// Why Podman shouldn't be given these, if it shouldn't.
    pub fn check(&self) -> Result<(), String> {
        let memory = self.memory.to_ascii_lowercase();
        let (digits, unit) = memory.split_at(memory.find(|c: char| !c.is_ascii_digit()).unwrap_or(memory.len()));
        let factor: u64 = match unit {
            "" | "b" => 1,
            "k" => 1 << 10,
            "m" => 1 << 20,
            "g" => 1 << 30,
            _ => 0,
        };
        let bytes = digits.parse::<u64>().ok().filter(|_| factor > 0).and_then(|n| n.checked_mul(factor));
        if !bytes.is_some_and(|b| b >= 64 << 20) {
            return Err(format!("memory {:?}: a size like 4g or 512m, at least 64m", self.memory));
        }
        let cpus_ok = !self.cpus.is_empty()
            && self.cpus.chars().all(|c| c.is_ascii_digit() || c == '.')
            && self.cpus.parse::<f64>().is_ok_and(|n| n > 0.0 && n <= 1024.0);
        if !cpus_ok {
            return Err(format!("cpus {:?}: a number like 2 or 0.5", self.cpus));
        }
        if !(32..=1_000_000).contains(&self.pids) {
            return Err(format!("pids {}: 32 to 1000000", self.pids));
        }
        Ok(())
    }
}

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
    #[serde(default)]
    pub limits: Limits,
    /// Its project's id on the worker (empty: made before ids were its
    /// own); see [`Worker::project`].
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub project: String,
}

impl Worker {
    /// Its one project's id on the worker: [`project_of`] its key for one
    /// made before ids were its own (`work` then), from its next start on.
    pub fn project(&self) -> String {
        if self.project.is_empty() { project_of(&self.key) } else { self.project.clone() }
    }

    /// The device id its server shows it as ([`device_of`]).
    pub fn device_id(&self) -> String {
        device_of(&self.key)
    }
}

/// The device id a worker with public key `key` shows as on its server:
/// the server's, whatever the worker says (a worker is as trusted as its
/// agent).
pub fn device_of(key: &str) -> String {
    format!("{DEVICE_PREFIX}{}", key.get(..12).unwrap_or(key))
}

/// What a worker's device id starts with; no paired server shows as one.
pub const DEVICE_PREFIX: &str = "worker-";

/// The project id a worker with public key `key` gets.
pub fn project_of(key: &str) -> String {
    format!("{}-{}", crate::worker::PROJECT, key.get(..12).unwrap_or(key))
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
    pub limits: Limits,
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

pub(crate) fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    name.len() <= 32
        && chars.next().is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// An image name Podman can't take for an option: no leading `-`, no
/// spaces or control characters.
fn valid_image(image: &str) -> bool {
    !image.is_empty() && image.len() <= 255 && !image.starts_with('-') && image.chars().all(|c| c.is_ascii_graphic())
}

/// What a create made so far, taken away again if a later step fails.
#[derive(Default)]
struct Made {
    secrets: Vec<String>,
    volumes: Vec<String>,
    container: bool,
    saved: bool,
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

    /// Make a worker, start it and check it answers with the key it was
    /// given. When a step fails, what it made is taken away again (its key
    /// secrets, container, folders, the volumes it made, its entry): a create
    /// leaves a running worker or nothing.
    pub async fn create(&self, new: NewWorker) -> Result<Worker, String> {
        let name = new.name.trim().to_string();
        if !valid_name(&name) {
            return Err("a worker's name is lowercase letters, digits and -, at most 32".into());
        }
        if self.list().iter().any(|w| w.name == name) {
            return Err(format!("worker {name} exists"));
        }
        let image = new.image.clone().unwrap_or_else(|| DEFAULT_IMAGE.to_string());
        if !valid_image(&image) {
            return Err(format!("image {image:?}: a name, not an option"));
        }
        new.limits.check()?;
        let mut variables = Vec::new();
        for (provider, key) in &new.keys {
            let variable = zend_harness::sandbox::provider_key_var(provider)
                .ok_or_else(|| format!("{provider}: no API key variable known for it"))?;
            if key.trim().is_empty() {
                return Err(format!("no {provider} key given"));
            }
            variables.push((provider.clone(), variable, key.trim().to_string()));
        }
        let container = container_of(&name);
        if self.podman(&["container", "exists", &container], None).await.is_ok() {
            return Err(format!("a container {container} exists already (another Zend's?): not made"));
        }
        let mut made = Made::default();
        match self.make(&name, &image, &new.limits, &variables, &mut made).await {
            Ok(worker) => Ok(worker),
            Err(err) => {
                self.undo(&name, &made).await;
                Err(err)
            }
        }
    }

    async fn make(
        &self,
        name: &str,
        image: &str,
        limits: &Limits,
        variables: &[(String, &'static str, String)],
        made: &mut Made,
    ) -> Result<Worker, String> {
        let server = crate::peer_link::load_node_key(&self.data_dir)?;
        let der = NodeKey::generate_pkcs8()?;
        let key = NodeKey::from_pkcs8(&der)?;
        let dir = self.dir(name);
        let (conf, run) = (dir.join("conf"), dir.join("run"));
        {
            use std::os::unix::fs::DirBuilderExt as _;
            use std::os::unix::fs::PermissionsExt as _;
            for (folder, mode) in [(&dir, 0o700), (&conf, 0o755), (&run, 0o711)] {
                std::fs::DirBuilder::new()
                    .recursive(true)
                    .mode(mode)
                    .create(folder)
                    .and_then(|()| std::fs::set_permissions(folder, std::fs::Permissions::from_mode(mode)))
                    .map_err(|e| format!("{}: {e}", folder.display()))?;
            }
        }
        self.write_conf(name, crate::worker::KEY_FILE, &der)?;
        self.write_conf(name, crate::worker::SERVER_KEY_FILE, server.public().as_bytes())?;
        self.write_conf(name, crate::worker::BLOCKED_FILE, (blocked_networks().join("\n") + "\n").as_bytes())?;
        let project = project_of(key.public());
        self.write_conf(name, crate::worker::PROJECT_FILE, project.as_bytes())?;

        let mut secrets = Vec::new();
        for (provider, variable, value) in variables {
            let secret = secret_of(name, provider);
            made.secrets.push(secret.clone());
            self.podman(&["secret", "create", "--replace", &secret, "-"], Some(value)).await?;
            secrets.push(format!("{secret},type=env,target={variable}"));
        }
        for what in ["work", "data"] {
            let volume = volume_of(name, what);
            if self.podman(&["volume", "exists", &volume], None).await.is_err() {
                made.volumes.push(volume);
            }
        }
        let mounts = mounts(name, &conf, &run);
        let container = container_of(name);
        let label = format!("zend.worker={name}");
        let pids = limits.pids.to_string();
        let mut args: Vec<&str> = vec![
            "create", "--pull", "never", "--name", &container, "--label", &label, "--user", "1000:1000", "--userns", "auto",
            "--network", PASTA, "--dns", "169.254.2.1", "--dns-search", ".",
            "--cap-drop", "all", "--security-opt", "no-new-privileges",
            "--memory", &limits.memory, "--memory-swap", &limits.memory, "--cpus", &limits.cpus, "--pids-limit", &pids,
            "--env", "ZEND_WORKER_DIR=/etc/zend-worker",
        ];
        for mount in &mounts {
            args.extend(["--mount", mount.as_str()]);
        }
        for secret in &secrets {
            args.extend(["--secret", secret.as_str()]);
        }
        args.extend(["--", image]);
        self.podman(&args, None).await?;
        made.container = true;
        let worker = Worker {
            name: name.to_string(),
            key: key.public().to_string(),
            image: image.to_string(),
            container,
            providers: variables.iter().map(|(p, _, _)| p.clone()).collect(),
            created_at: crate::now_ms(),
            limits: limits.clone(),
            project,
        };
        let mut all = self.list();
        all.push(worker.clone());
        self.save(all)?;
        made.saved = true;
        self.start(name).await?;
        Ok(worker)
    }

    /// Take away what a failed create made.
    async fn undo(&self, name: &str, made: &Made) {
        if made.container {
            let _ = self.podman(&["rm", "--force", "--time", "0", "--ignore", &container_of(name)], None).await;
        }
        for secret in &made.secrets {
            let _ = self.podman(&["secret", "rm", secret], None).await;
        }
        for volume in &made.volumes {
            let _ = self.podman(&["volume", "rm", "--force", volume], None).await;
        }
        self.remove_dir(name).await;
        if made.saved {
            let _ = self.save(self.list().into_iter().filter(|w| w.name != name).collect());
        }
    }

    /// Write one of the worker's `conf` files, readable by the container's
    /// uid (see "As whom").
    fn write_conf(&self, name: &str, file: &str, bytes: &[u8]) -> Result<(), String> {
        use std::os::unix::fs::PermissionsExt as _;
        let path = self.dir(name).join("conf").join(file);
        crate::runner::write_private_atomic(&path, bytes)
            .map_err(|e| e.to_string())
            .and_then(|()| std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).map_err(|e| e.to_string()))
            .map_err(|e| format!("{file}: {e}"))
    }

    /// Remove a worker's folders; `run` (and its socket) is the container's
    /// uid's, so what this user can't remove goes through `podman unshare`.
    async fn remove_dir(&self, name: &str) {
        let dir = self.dir(name);
        if std::fs::remove_dir_all(&dir).is_err() && dir.exists() {
            let path = dir.display().to_string();
            let _ = self.podman(&["unshare", "rm", "-rf", "--", &path], None).await;
        }
    }

    /// The container's state (`running`, `exited`, …), or why it can't say.
    pub async fn state(&self, name: &str) -> String {
        self.podman(&["inspect", "--format", "{{.State.Status}}", &container_of(name)], None)
            .await
            .unwrap_or_else(|e| format!("unknown ({e})"))
    }

    /// Start a worker that isn't running: its network made, the blocked
    /// networks taken afresh ([`blocked_networks`], the host's addresses
    /// as they are now) and written to `conf/blocked`, their routes added
    /// before its first process runs, then started, and answering with its
    /// pinned key. A running worker is left as it is, its routes with it.
    pub async fn start(&self, name: &str) -> Result<(), String> {
        let worker = self.get(name)?;
        if self.state(name).await != "running" {
            // Nothing lent before this start stays (its next turns lend anew).
            self.release(name).await;
            self.write_conf(name, crate::worker::BLOCKED_FILE, (blocked_networks().join("\n") + "\n").as_bytes())
                .map_err(|e| format!("worker {name}: {e}"))?;
            // (A worker made with `work` gets its own id from here on.)
            self.write_conf(name, crate::worker::PROJECT_FILE, worker.project().as_bytes())
                .map_err(|e| format!("worker {name}: {e}"))?;
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
                self.release(name).await;
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

    /// Stop a running worker; the logins lent to it go first
    /// ([`crate::worker_logins`]), its allows stay for its next start.
    pub async fn stop(&self, name: &str) -> Result<(), String> {
        let worker = self.get(name)?;
        self.release(name).await;
        let stopped = self.podman(&["stop", "--time", "5", "--ignore", &worker.container], None).await;
        // (And again: a turn lent for while it stopped.)
        self.release(name).await;
        stopped.map(|_| ())
    }

    /// Every login lent to worker `name` goes, its turns no longer
    /// followed ([`crate::worker_logins::release`], under its lock).
    async fn release(&self, name: &str) {
        let (data_dir, name) = (self.data_dir.clone(), name.to_string());
        let _ = tokio::task::spawn_blocking(move || crate::worker_logins::release(&data_dir, &name)).await;
    }

    /// End worker `name`'s turns in `chats` (their login was taken back):
    /// an interrupt to each, over its socket. How many it took.
    pub async fn interrupt(&self, name: &str, chats: &[String]) -> usize {
        if chats.is_empty() {
            return 0;
        }
        let client = match self.connect(name).await {
            Ok(client) => client,
            Err(err) => {
                tracing::warn!(worker = %name, error = %err, "worker: not reached, its turns on a login taken back aren't interrupted (the login is gone)");
                return 0;
            }
        };
        let mut ended = 0;
        for chat in chats {
            let stop = serde_json::json!({ "chatId": chat, "command": zend_doc::commands::SessionCommandPayload::Interrupt { cancels: None } });
            match client.call(zend_rpc::methods::QUEUE_COMMAND, stop).await {
                Ok(_) => {
                    ended += 1;
                    tracing::info!(worker = %name, chat = %chat, "worker: a turn on a login taken back interrupted");
                }
                Err(err) => tracing::warn!(worker = %name, chat = %chat, error = %err, "worker: a turn on a login taken back couldn't be interrupted"),
            }
        }
        ended
    }

    /// Remove a worker: its container, its secrets, its folders and (unless
    /// `keep_volumes`) its volumes, /work among them. The logins lent to it
    /// go first, and its allows with it.
    pub async fn remove(&self, name: &str, keep_volumes: bool) -> Result<Worker, String> {
        let worker = self.get(name)?;
        let (data_dir, named) = (self.data_dir.clone(), name.to_string());
        tokio::task::spawn_blocking(move || crate::worker_logins::forget(&data_dir, &named)).await.map_err(|e| e.to_string())??;
        self.podman(&["rm", "--force", "--time", "5", "--ignore", &worker.container], None).await?;
        for provider in &worker.providers {
            let _ = self.podman(&["secret", "rm", &secret_of(name, provider)], None).await;
        }
        if !keep_volumes {
            for what in ["work", "data"] {
                let _ = self.podman(&["volume", "rm", "--force", &volume_of(name, what)], None).await;
            }
        }
        self.remove_dir(name).await;
        self.save(self.list().into_iter().filter(|w| w.name != name).collect())?;
        Ok(worker)
    }
}

/// A worker's mounts: its volumes (U: Podman chowns them to the container's
/// uid 1000, whichever subordinate ids its namespace got, --userns=auto),
/// its `conf` read-only (its lent logins in it, [`crate::worker_logins`]),
/// its `run` (the socket), and a tmpfs of its uid's for its copy of the
/// lent logins ([`crate::worker::RUN_LOGIN_DIR`]: no token on a volume).
pub fn mounts(name: &str, conf: &Path, run: &Path) -> [String; 5] {
    [
        format!("type=volume,src={},dst=/work,U=true", volume_of(name, "work")),
        format!("type=volume,src={},dst=/data,U=true", volume_of(name, "data")),
        format!("type=bind,src={},dst=/etc/zend-worker,ro", conf.display()),
        format!("type=bind,src={},dst=/run/zend-worker,U=true", run.display()),
        format!("type=tmpfs,dst={},tmpfs-mode=0700,tmpfs-size=1m,chown=true", crate::worker::RUN_LOGIN_DIR),
    ]
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
    #[serde(default)]
    memory: Option<String>,
    #[serde(default)]
    cpus: Option<String>,
    #[serde(default)]
    pids: Option<u32>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NameParams {
    name: String,
    #[serde(default)]
    keep_volumes: bool,
}

#[derive(Deserialize)]
struct LoginParams {
    name: String,
    provider: String,
}

#[async_trait::async_trait]
impl zend_rpc::RpcService for WorkersRpc {
    async fn handle(&self, method: &str, params: serde_json::Value) -> Result<zend_rpc::RpcReply, RpcError> {
        use serde_json::json;
        use zend_rpc::{RpcReply, methods};
        let given = || if params.is_null() { json!({}) } else { params.clone() };
        match method {
            methods::CREATE_WORKER
            | methods::LIST_WORKERS
            | methods::START_WORKER
            | methods::STOP_WORKER
            | methods::REMOVE_WORKER
            | methods::ALLOW_WORKER_LOGIN
            | methods::REMOVE_WORKER_LOGIN
            | methods::LIST_WORKER_LOGINS
                if !zend_rpc::caller().is_local() =>
            {
                Err(RpcError::Failed(format!("{method} is this device's own")))
            }
            methods::CREATE_WORKER => {
                let p: CreateParams = zend_rpc::parse_params(given())?;
                let worker = self
                    .workers
                    .create({
                        let default = Limits::default();
                        let limits = Limits {
                            memory: p.memory.unwrap_or(default.memory),
                            cpus: p.cpus.unwrap_or(default.cpus),
                            pids: p.pids.unwrap_or(default.pids),
                        };
                        NewWorker { name: p.name, image: p.image, keys: p.keys.into_iter().collect(), limits }
                    })
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
                    value["project"] = worker.project().into();
                    value["deviceId"] = worker.device_id().into();
                    listed.push(value);
                }
                Ok(RpcReply::Value(json!({ "workers": listed })))
            }
            methods::START_WORKER => {
                let p: NameParams = zend_rpc::parse_params(given())?;
                self.workers.start(&p.name).await.map_err(RpcError::Failed)?;
                Ok(RpcReply::Value(json!({ "name": p.name, "state": self.workers.state(&p.name).await })))
            }
            methods::STOP_WORKER => {
                let p: NameParams = zend_rpc::parse_params(given())?;
                self.workers.stop(&p.name).await.map_err(RpcError::Failed)?;
                Ok(RpcReply::Value(json!({ "name": p.name, "state": self.workers.state(&p.name).await })))
            }
            methods::ALLOW_WORKER_LOGIN => {
                let p: LoginParams = zend_rpc::parse_params(given())?;
                if !valid_name(&p.name) {
                    return Err(RpcError::BadParams(format!("{:?} is not a worker's name", p.name.chars().take(40).collect::<String>())));
                }
                let worker = self.workers.get(&p.name).map_err(RpcError::BadParams)?;
                let (data_dir, name, provider) = (self.workers.data_dir.clone(), worker.name.clone(), p.provider.clone());
                tokio::task::spawn_blocking(move || crate::worker_logins::allow(&data_dir, &name, &provider))
                    .await
                    .map_err(|e| RpcError::Failed(e.to_string()))?
                    .map_err(RpcError::BadParams)?;
                let key = worker.providers.contains(&p.provider);
                let host = crate::peer_logins::HostLogin::current().has(&p.provider);
                Ok(RpcReply::Value(json!({ "name": worker.name, "provider": p.provider, "keyWins": key, "hostSignedIn": host })))
            }
            methods::REMOVE_WORKER_LOGIN => {
                let p: LoginParams = zend_rpc::parse_params(given())?;
                if !valid_name(&p.name) {
                    return Err(RpcError::BadParams(format!("{:?} is not a worker's name", p.name.chars().take(40).collect::<String>())));
                }
                // Its file goes at once (after a refresh for it, held to the
                // worker's lock), then its live turns on it end.
                let (data_dir, name, provider) = (self.workers.data_dir.clone(), p.name.clone(), p.provider.clone());
                let (removed, chats) = tokio::task::spawn_blocking(move || crate::worker_logins::revoke(&data_dir, &name, &provider))
                    .await
                    .map_err(|e| RpcError::Failed(e.to_string()))?
                    .map_err(RpcError::Failed)?;
                let ended = self.workers.interrupt(&p.name, &chats).await;
                Ok(RpcReply::Value(json!({ "name": p.name, "provider": p.provider, "removed": removed, "turnsEnded": ended })))
            }
            methods::LIST_WORKER_LOGINS => {
                let logins: Vec<serde_json::Value> = crate::worker_logins::book(&self.workers.data_dir)
                    .list()
                    .into_iter()
                    .map(|(name, providers)| json!({ "name": name, "providers": providers }))
                    .collect();
                Ok(RpcReply::Value(json!({ "logins": logins })))
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
    use serde_json::json;

    /// A worker made before project ids were its own (none kept) gets its
    /// own, as a new one does: what its start writes to `conf/project`.
    /// Its device id is its server's, from its key.
    #[test]
    fn an_old_worker_gets_its_own_project_and_every_worker_its_device() {
        let key = "a4b3f3e0b0839d1c".repeat(4);
        let old: Worker = serde_json::from_value(json!({ "name": "w", "key": key, "image": "i", "container": "c" })).unwrap();
        assert_eq!(old.project(), "work-a4b3f3e0b083");
        assert_eq!(old.project(), project_of(&key));
        let new = Worker { project: project_of(&key), ..old.clone() };
        assert_eq!(new.project(), old.project());
        assert_eq!(old.device_id(), "worker-a4b3f3e0b083");
    }

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

    #[test]
    fn an_image_is_never_an_option() {
        for good in ["localhost/zend-worker:latest", "docker.io/library/node:22", "img@sha256:abc"] {
            assert!(valid_image(good), "{good}");
        }
        for bad in ["", "-v", "--privileged", "--network=host", "a b", "a\nb", "x\u{7f}"] {
            assert!(!valid_image(bad), "{bad:?}");
        }
    }

    #[test]
    fn limits_take_sizes_and_numbers_only() {
        assert!(Limits::default().check().is_ok());
        for (memory, cpus, pids) in [("512m", "0.5", 32), ("8G", "16", 4096), ("1073741824", "1", 1_000_000)] {
            let l = Limits { memory: memory.into(), cpus: cpus.into(), pids };
            assert!(l.check().is_ok(), "{l:?}");
        }
        for (memory, cpus, pids) in [("4x", "2", 512), ("-1", "2", 512), ("1m", "2", 512), ("", "2", 512), ("4g", "-2", 512), ("4g", "0", 512), ("4g", "2e3", 512), ("4g", "2", 10)] {
            let l = Limits { memory: memory.into(), cpus: cpus.into(), pids };
            assert!(l.check().is_err(), "{l:?}");
        }
    }
}
