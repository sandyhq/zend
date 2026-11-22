//! The phone link: this engine served to the owner's phone over the
//! tailnet, with no edge between (the phone's window drives it as this
//! device's window does over loopback).
//!
//! Off until the person turns it on (Settings → Devices: "Show code", or
//! the switch there). On, it finds this device's tailnet address
//! ([discover]: tailscale status --json, nothing else), binds each tailnet address on the IPC port
//! (never another: [zend_rpc::bind_remote]) and serves
//! the phone link there with the phone-link key, admitting only peers
//! Tailscale knows as nodes of the tailnet ([tailnet_peer]).
//!
//! Kept in the data dir: phone-link-key (32 random bytes as hex, mode
//! 600, atomic; kept across starts so a linked phone stays linked) and
//! phone-link.json ({on, use}: whether it's on, and whether the code
//! holds the MagicDNS name or the IP). On at the last stop, it's on again
//! as the engine starts. "New key" writes a new key and starts the
//! listeners again on it: every phone holding the old one is cut and
//! refused.
//!
//! ```text
//! PhoneLinkStatus  {}                 → status
//! SetPhoneLink     {on?, use?}        → status (fails "no_tailnet" turning it on without one)
//! NewPhoneLinkKey  {}                 → status
//! status: {on, use, name?, ips, host?, port, listening, key? (on only), error?}
//! ```
//!
//! All three are local-only (zend_rpc::local_only): the key is this
//! device's window's alone.

use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use zend_rpc::{RpcError, RpcReply, RpcService, methods};

/// The key's file in the data dir.
pub const KEY_FILE: &str = "phone-link-key";
/// What's kept of the switch in the data dir.
pub const STATE_FILE: &str = "phone-link.json";

/// This device on the tailnet: its MagicDNS name (no trailing dot) and its
/// addresses there.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Tailnet {
    pub name: Option<String>,
    pub ips: Vec<IpAddr>,
}

/// tailscale status --json's answer as this device on the tailnet, if
/// it's on one (running, with a tailnet address).
pub fn parse_status(json: &str) -> Option<Tailnet> {
    let status: Value = serde_json::from_str(json).ok()?;
    if status["BackendState"].as_str().is_some_and(|state| state != "Running") {
        return None;
    }
    let me = &status["Self"];
    let ips: Vec<IpAddr> = me["TailscaleIPs"]
        .as_array()?
        .iter()
        .filter_map(|ip| ip.as_str()?.parse().ok())
        .filter(|ip| zend_rpc::in_tailnet(*ip))
        .collect();
    if ips.is_empty() {
        return None;
    }
    let name = me["DNSName"]
        .as_str()
        .map(|name| name.trim_end_matches('.').to_ascii_lowercase())
        .filter(|name| valid_name(name));
    Some(Tailnet { name, ips })
}

/// A DNS name a code may hold: labels of letters, digits and dashes.
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 253
        && name.split('.').all(|label| {
            (1..=63).contains(&label.len()) && label.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
}

/// This device on the tailnet: Tailscale's own word (`tailscale status
/// --json`), or none. Never an address read off an interface: one in the
/// tailnet's ranges needn't be Tailscale's (a carrier's CGNAT uses the same
/// 100.64.0.0/10), and a link bound there would face that network.
pub async fn discover() -> Option<Tailnet> {
    let out = tailscale(&["status", "--json"]).await?;
    parse_status(&out)
}

/// `tailscale <args>`'s output, when it ran and succeeded within 3 s.
async fn tailscale(args: &[&str]) -> Option<String> {
    let asked = tokio::time::timeout(
        Duration::from_secs(3),
        tokio::process::Command::new("tailscale")
            .args(args)
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .output(),
    )
    .await;
    match asked {
        Ok(Ok(out)) if out.status.success() => Some(String::from_utf8_lossy(&out.stdout).into_owned()),
        _ => None,
    }
}

/// Whether `ip`, a connecting peer's address, is a node of this tailnet
/// as Tailscale itself says (`tailscale whois`), not only an address in
/// its ranges: a packet from another interface with a tailnet source gets
/// no answer back through it, and anything Tailscale can't vouch for
/// (no CLI, not running, no such node) is refused.
pub fn tailnet_peer(ip: IpAddr) -> std::pin::Pin<Box<dyn std::future::Future<Output = bool> + Send>> {
    Box::pin(async move {
        if !zend_rpc::in_tailnet(ip) {
            return false;
        }
        let ip_text = ip.to_string();
        match tailscale(&["whois", "--json", &ip_text]).await {
            Some(out) => whois_names(&out, ip),
            None => false,
        }
    })
}

/// Whether `tailscale whois --json` output names a node that holds `ip`.
pub(crate) fn whois_names(out: &str, ip: IpAddr) -> bool {
    let Ok(answer) = serde_json::from_str::<Value>(out) else {
        return false;
    };
    answer["Node"]["Addresses"].as_array().is_some_and(|addresses| {
        addresses.iter().filter_map(Value::as_str).any(|a| {
            a.split('/').next().and_then(|a| a.parse::<IpAddr>().ok()) == Some(ip)
        })
    })
}

/// Which address the code holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Use {
    #[default]
    Name,
    Ip,
}

#[derive(Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
struct Saved {
    #[serde(default)]
    on: bool,
    #[serde(default, rename = "use")]
    use_: Use,
}

type Found = futures::future::BoxFuture<'static, Option<Tailnet>>;
type Bound = futures::future::BoxFuture<'static, std::io::Result<tokio::net::TcpListener>>;

/// How the link finds the tailnet, binds and admits: the real ones, or a
/// test's (loopback).
#[derive(Clone, Copy)]
pub struct Seams {
    pub discover: fn() -> Found,
    pub bind: fn(SocketAddr) -> Bound,
    pub admits: zend_rpc::remote::Admits,
}

impl Seams {
    pub fn real() -> Self {
        Self {
            discover: || Box::pin(discover()),
            bind: |addr| Box::pin(zend_rpc::bind_remote(addr)),
            admits: tailnet_peer,
        }
    }
}

#[derive(Default)]
struct Running {
    tailnet: Option<Tailnet>,
    listeners: Vec<tokio::task::JoinHandle<()>>,
    listening: Vec<SocketAddr>,
    error: Option<String>,
}

impl Running {
    fn stop(&mut self) {
        for listener in self.listeners.drain(..) {
            listener.abort();
        }
        self.listening.clear();
    }
}

/// The phone link of one engine.
pub struct PhoneLink {
    data_dir: PathBuf,
    port: u16,
    service: Arc<dyn RpcService>,
    seams: Seams,
    running: tokio::sync::Mutex<Running>,
}

impl Drop for PhoneLink {
    fn drop(&mut self) {
        self.running.get_mut().stop();
    }
}

impl PhoneLink {
    /// The link of the engine serving service on port from data_dir;
    /// nothing is served until it's on.
    pub fn new(data_dir: &Path, port: u16, service: Arc<dyn RpcService>, seams: Seams) -> Arc<Self> {
        Arc::new(Self {
            data_dir: data_dir.to_path_buf(),
            port,
            service,
            seams,
            running: tokio::sync::Mutex::new(Running::default()),
        })
    }

    fn saved(&self) -> Saved {
        std::fs::read(self.data_dir.join(STATE_FILE))
            .ok()
            .and_then(|raw| serde_json::from_slice(&raw).ok())
            .unwrap_or_default()
    }

    fn save(&self, saved: &Saved) -> Result<(), String> {
        let path = self.data_dir.join(STATE_FILE);
        let tmp = self.data_dir.join(format!(".{STATE_FILE}.tmp"));
        std::fs::create_dir_all(&self.data_dir)
            .and_then(|()| std::fs::write(&tmp, serde_json::to_vec(saved).unwrap_or_default()))
            .and_then(|()| std::fs::rename(&tmp, &path))
            .map_err(|err| format!("couldn't keep the phone link's setting: {err}"))
    }

    /// The key, made (and kept) the first time it's asked for.
    fn key(&self) -> Result<String, String> {
        let path = self.data_dir.join(KEY_FILE);
        match zend_rpc::ipc_auth::read_token(&path) {
            Ok(Some(key)) if key.len() == 64 && key.bytes().all(|b| b.is_ascii_hexdigit()) => Ok(key),
            _ => self.new_key(),
        }
    }

    fn new_key(&self) -> Result<String, String> {
        let key = zend_rpc::ipc_auth::generate_token().map_err(|err| err.to_string())?;
        zend_rpc::ipc_auth::write_token(&self.data_dir.join(KEY_FILE), &key)
            .map_err(|err| format!("couldn't keep the phone-link key: {err}"))?;
        Ok(key)
    }

    /// Serve it as kept: on at the last stop, on again now (Tailscale may
    /// come up after the engine: tried again for a while).
    pub fn resume(self: &Arc<Self>) {
        if !self.saved().on {
            return;
        }
        let link = Arc::downgrade(self);
        tokio::spawn(async move {
            for attempt in 0..20u32 {
                let Some(link) = link.upgrade() else { return };
                if !link.saved().on {
                    return;
                }
                {
                    let mut running = link.running.lock().await;
                    if !running.listening.is_empty() || link.start(&mut running).await.is_ok() {
                        return;
                    }
                }
                drop(link);
                tokio::time::sleep(Duration::from_secs(if attempt < 5 { 5 } else { 30 })).await;
            }
        });
    }

    /// (Re)start the listeners on every tailnet address, with the key.
    async fn start(&self, running: &mut Running) -> Result<(), String> {
        running.stop();
        running.error = None;
        let Some(tailnet) = (self.seams.discover)().await else {
            running.tailnet = None;
            running.error = Some("no_tailnet".into());
            return Err("no_tailnet".into());
        };
        running.tailnet = Some(tailnet.clone());
        let key = self.key()?;
        let mut failed = Vec::new();
        for ip in &tailnet.ips {
            let addr = SocketAddr::new(*ip, self.port);
            match (self.seams.bind)(addr).await {
                Ok(listener) => {
                    let addr = listener.local_addr().unwrap_or(addr);
                    tracing::info!(%addr, "phone link: listening");
                    running.listening.push(addr);
                    running.listeners.push(tokio::spawn(zend_rpc::remote::serve_remote_listener_admitting(
                        listener,
                        self.service.clone(),
                        key.clone(),
                        self.seams.admits,
                    )));
                }
                Err(err) => {
                    tracing::warn!(%addr, error = %err, "phone link: can't listen");
                    failed.push(format!("{addr}: {err}"));
                }
            }
        }
        if running.listening.is_empty() {
            let error = format!("bind: {}", failed.join("; "));
            running.error = Some(error.clone());
            return Err(error);
        }
        Ok(())
    }

    fn status(&self, running: &Running) -> Value {
        let saved = self.saved();
        let on = !running.listening.is_empty();
        let tailnet = running.tailnet.clone().unwrap_or_default();
        let ip = tailnet.ips.iter().find(|ip| ip.is_ipv4()).or(tailnet.ips.first()).copied();
        let host = match (saved.use_, &tailnet.name) {
            (Use::Name, Some(name)) => Some(name.clone()),
            _ => ip.map(|ip| ip.to_string()),
        };
        let mut status = json!({
            "on": on,
            "use": saved.use_,
            "name": tailnet.name,
            "ips": tailnet.ips.iter().map(ToString::to_string).collect::<Vec<_>>(),
            "host": host,
            "port": running.listening.first().map_or(self.port, SocketAddr::port),
            "listening": running.listening.iter().map(ToString::to_string).collect::<Vec<_>>(),
            "error": running.error,
        });
        if on && let Ok(key) = self.key() {
            status["key"] = Value::from(key);
        }
        status
    }

    /// What it is now (the tailnet looked at again while it's off).
    pub async fn get(&self) -> Value {
        let mut running = self.running.lock().await;
        if running.listening.is_empty() {
            running.tailnet = (self.seams.discover)().await;
        }
        self.status(&running)
    }

    /// Turn it on or off, and choose what the code holds.
    pub async fn set(&self, on: Option<bool>, use_: Option<Use>) -> Result<Value, RpcError> {
        let mut running = self.running.lock().await;
        let mut saved = self.saved();
        if let Some(use_) = use_ {
            saved.use_ = use_;
        }
        match on {
            Some(true) => {
                if running.listening.is_empty()
                    && let Err(error) = self.start(&mut running).await
                {
                    saved.on = false;
                    self.save(&saved).map_err(RpcError::Failed)?;
                    return Err(RpcError::Failed(if error == "no_tailnet" { error } else { format!("phone link: {error}") }));
                }
                saved.on = true;
            }
            Some(false) => {
                running.stop();
                running.error = None;
                saved.on = false;
            }
            None => {}
        }
        self.save(&saved).map_err(RpcError::Failed)?;
        Ok(self.status(&running))
    }

    /// A new key: the listeners started again on it, every phone holding
    /// the old one cut.
    pub async fn renew(&self) -> Result<Value, RpcError> {
        let mut running = self.running.lock().await;
        self.new_key().map_err(RpcError::Failed)?;
        if !running.listening.is_empty() {
            let _ = self.start(&mut running).await;
        }
        Ok(self.status(&running))
    }
}

#[derive(Deserialize)]
struct SetParams {
    on: Option<bool>,
    #[serde(rename = "use")]
    use_: Option<Use>,
}

/// inner with the phone link's methods answered by link (and the rest
/// passed on).
pub struct PhoneLinkRpc {
    pub inner: Arc<dyn RpcService>,
    pub link: Arc<PhoneLink>,
}

#[async_trait]
impl RpcService for PhoneLinkRpc {
    async fn handle(&self, method: &str, params: Value) -> Result<RpcReply, RpcError> {
        match method {
            methods::PHONE_LINK_STATUS | methods::SET_PHONE_LINK | methods::NEW_PHONE_LINK_KEY
                if !zend_rpc::caller().is_local() =>
            {
                Err(RpcError::Failed(format!("{method} is this device's own")))
            }
            methods::PHONE_LINK_STATUS => Ok(RpcReply::Value(self.link.get().await)),
            methods::SET_PHONE_LINK => {
                let p: SetParams = zend_rpc::parse_params(if params.is_null() { json!({}) } else { params })?;
                self.link.set(p.on, p.use_).await.map(RpcReply::Value)
            }
            methods::NEW_PHONE_LINK_KEY => self.link.renew().await.map(RpcReply::Value),
            _ => self.inner.handle(method, params).await,
        }
    }
}

/// service with a phone link of its own on port, resumed if it was on.
/// The link serves service itself (what a phone may call of it is
/// [zend_rpc::remote_allowed]).
pub fn wrap(service: Arc<dyn RpcService>, data_dir: &Path, port: u16) -> Arc<dyn RpcService> {
    let link = PhoneLink::new(data_dir, port, service.clone(), Seams::real());
    link.resume();
    Arc::new(PhoneLinkRpc { inner: service, link })
}

#[cfg(test)]
#[path = "phone_link_tests.rs"]
mod tests;
