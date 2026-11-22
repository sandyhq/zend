//! Connecting a phone to this computer (Settings → Devices).
//!
//! Now (v=2, the phone link): the computer's engine listens on its tailnet
//! address too (crates/engine/src/phone_link.rs, off until the person shows
//! the code or turns it on), and the code holds that address (its MagicDNS
//! name or its IP, as the person chose), the port and the engine's
//! phone-link key: `zend://connect?v=2&host=…&port=…&key=…`. The phone
//! keeps them (`zend_cli`'s `desktop` record, mode 600), runs no engine of
//! its own any more, and is a window onto the desktop's (src/engine/rpc.rs).
//!
//! Before (v=1, still read):
//! The owner's relay is the self-hosted edge (on a VM over Tailscale), with
//! the edge's dev bearer as the account: no hosted sign-in. The computer
//! shows a QR code holding its edge and its own bearer, on a click and for
//! a minute; Zend on the phone scans it (src/android_camera.rs) or takes
//! its text pasted, keeps both in its config folder (`zend_cli`'s
//! `edge-url`, `edge-token`, mode 600) and starts its engine again on them.
//!
//! ```text
//! call    phone.connect_code  (none)    { svg, modules, on, use, name, ip, host, port }:
//!                                       the phone link turned on, its code as a QR
//!                                       (fails "no_tailnet", "offline", "failed")
//! call    phone.link_status   (none)    { on, use, name, ip, host, port, error }
//! call    phone.link_set      { on?, use? }   the same: turned on or off, "name" or "ip"
//! call    phone.link_new_key  (none)    the same: a new key, every phone cut
//! call    phone.connect       { code }  { edge } or { desktop }: the code kept, started on it
//!                                       (Android; fails "bad_code", "store", "engine")
//! call    phone.scan          (none)    the code the camera found (Android; fails
//!                                       "denied", "no_camera", "camera", "stopped")
//! invoke  phone.scan_stop     (none)    stop scanning (Android)
//! shared  phone.scan          { state, frames, detail? }   the camera's progress (Android)
//! ```
//!
//! None of these is offered to plugins (`expose_action`): the code is the
//! computer's bearer. The code never goes to a file or the log; the log
//! says only the edge's scheme and port.

// (Reading a code is the phone's: unused on a computer but in tests.)
#![cfg_attr(not(target_os = "android"), allow(dead_code))]

use zgx_script::{ScriptHost, Value};

/// Which scan is the live one: each scan an attempt of its own, and any
/// newer one (or a stop: the page left, the app sent away) ends the ones
/// before it. A scan still running looks whether it's live between frames,
/// and its answer is dropped once it isn't. Nothing to clear when a scan
/// ends, however it ends (a panic included).
pub struct Attempts(std::sync::atomic::AtomicU64);

impl Attempts {
    pub const fn new() -> Self {
        Self(std::sync::atomic::AtomicU64::new(0))
    }

    /// A new attempt: the live one from now on.
    pub fn begin(&self) -> u64 {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1
    }

    /// No attempt live any more.
    pub fn stop(&self) {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }

    pub fn live(&self, attempt: u64) -> bool {
        self.0.load(std::sync::atomic::Ordering::SeqCst) == attempt
    }
}

/// What a code starts with.
const PREFIX: &str = "zend://connect?";

/// A connect code's contents.
#[derive(Debug, PartialEq, Eq)]
pub struct Connect {
    pub edge: String,
    pub token: String,
}

/// A desktop's phone link (v=2): where on the tailnet the phone dials its
/// engine, and the key that engine takes from phones.
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Direct {
    pub host: String,
    pub port: u16,
    pub key: String,
}

/// What a code holds: a relay to join (v=1), or a desktop to drive (v=2).
#[derive(Debug, PartialEq, Eq)]
pub enum Code {
    Edge(Connect),
    Direct(Direct),
}

/// The code for a desktop's phone link:
/// `zend://connect?v=2&host=…&port=…&key=…`.
pub fn encode_direct(host: &str, port: u16, key: &str) -> String {
    format!("{PREFIX}v=2&host={}&port={port}&key={}", escape(host.trim()), escape(key.trim()))
}

/// Whether `ip` is a tailnet address (Tailscale's CGNAT range, or its
/// IPv6 prefix).
pub fn in_tailnet(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
            a == 100 && (64..128).contains(&b)
        }
        std::net::IpAddr::V6(v6) => {
            let s = v6.segments();
            (s[0], s[1], s[2]) == (0xfd7a, 0x115c, 0xa1e0) || v6.to_ipv4_mapped().is_some_and(|v4| in_tailnet(v4.into()))
        }
    }
}

/// `host` as a code may hold it, canonical: a tailnet address (an IPv6
/// one without brackets), or a DNS name (MagicDNS's), in lower case.
pub fn canonical_host(host: &str) -> Option<String> {
    let host = host.trim().trim_start_matches('[').trim_end_matches(']').to_ascii_lowercase();
    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        return in_tailnet(ip).then(|| ip.to_string());
    }
    let name = host.strip_suffix('.').unwrap_or(&host);
    let valid = !name.is_empty()
        && name.len() <= 253
        && !name.bytes().all(|b| b.is_ascii_digit() || b == b'.')
        && name.split('.').all(|label| {
            (1..=63).contains(&label.len())
                && label.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                && !label.starts_with('-')
                && !label.ends_with('-')
        });
    valid.then(|| name.to_owned())
}

fn valid_key(key: &str) -> bool {
    key.len() == 64 && key.bytes().all(|b| b.is_ascii_hexdigit())
}

/// `ws://host:port`, the address a phone dials (an IPv6 host bracketed).
pub fn ws_url(host: &str, port: u16) -> String {
    if host.contains(':') { format!("ws://[{host}]:{port}") } else { format!("ws://{host}:{port}") }
}

/// The code for `edge` and `token`: `zend://connect?v=1&edge=…&token=…`,
/// both percent-encoded.
pub fn encode(edge: &str, token: &str) -> String {
    format!("{PREFIX}v=1&edge={}&token={}", escape(edge.trim()), escape(token.trim()))
}

/// The longest code read (a QR code holds at most 2953 bytes): longer
/// text is refused before any of it is decoded.
const MAX_CODE: usize = 3000;
/// The longest edge and bearer a code may hold.
const MAX_EDGE: usize = 256;
const MAX_TOKEN: usize = 1024;

/// What a code holds (pasted text may come with spaces or a line break
/// around it), or why it's not one. v=1: a relay's edge, checked as a URL
/// (http or https, a host, a valid port, nothing else: no user, path,
/// query or fragment) and given in its canonical form, and a bearer of one
/// printable word. v=2: a desktop's host on the tailnet ([canonical_host]),
/// its port (1 to 65535) and its key (64 hex digits). Other fields are
/// passed over without decoding them; one of them twice refuses the code.
pub fn decode(text: &str) -> Result<Code, &'static str> {
    let text = text.trim();
    if text.len() > MAX_CODE {
        return Err("too long");
    }
    let query = text.strip_prefix(PREFIX).ok_or("not a Zend connect code")?;
    let (mut version, mut edge, mut token, mut host, mut port, mut key) = (None, None, None, None, None, None);
    for pair in query.split('&') {
        let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
        let slot = match name {
            "v" => &mut version,
            "edge" => &mut edge,
            "token" => &mut token,
            "host" => &mut host,
            "port" => &mut port,
            "key" => &mut key,
            _ => continue,
        };
        if slot.is_some() {
            return Err("a field twice");
        }
        *slot = Some(unescape(value).ok_or("badly encoded")?);
    }
    match version.as_deref() {
        Some("1") => {
            let edge = edge.as_deref().and_then(canonical_edge).ok_or("no relay address in it")?;
            let token = token.filter(|t| valid_token(t)).ok_or("no key in it")?;
            Ok(Code::Edge(Connect { edge, token }))
        }
        Some("2") => {
            let host = host.as_deref().and_then(canonical_host).ok_or("no tailnet address in it")?;
            let port = port
                .filter(|p| (1..=5).contains(&p.len()) && p.bytes().all(|b| b.is_ascii_digit()))
                .and_then(|p| p.parse::<u16>().ok())
                .filter(|p| *p != 0)
                .ok_or("no port in it")?;
            let key = key.filter(|k| valid_key(k)).ok_or("no key in it")?.to_ascii_lowercase();
            Ok(Code::Direct(Direct { host, port, key }))
        }
        _ => Err("a code of another version"),
    }
}

/// `edge` in its canonical form (`scheme://host[:port]`, lower case, no
/// default port), if it's a relay's address and nothing more.
pub fn canonical_edge(edge: &str) -> Option<String> {
    if edge.is_empty() || edge.len() > MAX_EDGE || !edge.bytes().all(|b| b.is_ascii_graphic() && b != b'\\') {
        return None;
    }
    // The parser is lenient with what follows the scheme (`http:/a`,
    // `http:///a`): two slashes and a host right after them, or nothing.
    let lower = edge.to_ascii_lowercase();
    let rest = lower.strip_prefix("http://").or_else(|| lower.strip_prefix("https://"))?;
    if rest.is_empty() || rest.starts_with(['/', '?', '#', '@', ':']) {
        return None;
    }
    let url = url::Url::parse(edge).ok()?;
    let plain = matches!(url.scheme(), "http" | "https")
        && url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none()
        && url.path() == "/"
        && url.port() != Some(0);
    let host = url.host_str().filter(|host| {
        !host.is_empty() && host.len() <= 253 && (host.starts_with('[') || host.split('.').all(|label| label.len() <= 63))
    });
    (plain && host.is_some()).then(|| url.origin().ascii_serialization())
}

fn valid_token(token: &str) -> bool {
    (1..=MAX_TOKEN).contains(&token.len()) && token.bytes().all(|b| b.is_ascii_graphic())
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn unescape(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// `text` as a QR code drawn in SVG: dark modules on a light ground, with
/// the four-module quiet zone scanners need, one unit a module. Its width
/// in modules, quiet zone included, beside it.
pub fn qr_svg(text: &str) -> Option<(String, usize)> {
    use qrcodegen::{QrCode, QrCodeEcc};
    let code = QrCode::encode_text(text, QrCodeEcc::Medium).ok()?;
    let n = code.size();
    let quiet = 4;
    let total = n + 2 * quiet;
    let mut path = String::new();
    for y in 0..n {
        for x in 0..n {
            if code.get_module(x, y) {
                path.push_str(&format!("M{} {}h1v1h-1z", x + quiet, y + quiet));
            }
        }
    }
    let svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {total} {total}\" shape-rendering=\"crispEdges\">\
         <rect width=\"{total}\" height=\"{total}\" fill=\"#ffffff\"/><path d=\"{path}\" fill=\"#000000\"/></svg>"
    );
    Some((svg, total as usize))
}

/// The code a desktop's phone-link status (the engine's PhoneLinkStatus
/// or SetPhoneLink reply) gives: v=2, its chosen host, port and key.
fn direct_code(status: &serde_json::Value) -> Result<String, String> {
    let host = status["host"].as_str().and_then(canonical_host).ok_or("no_tailnet")?;
    let port = status["port"].as_u64().and_then(|p| u16::try_from(p).ok()).filter(|p| *p != 0).ok_or("failed")?;
    let key = status["key"].as_str().filter(|k| valid_key(k)).ok_or("failed")?;
    Ok(encode_direct(&host, port, key))
}

/// The phone link's status as the page sees it: everything but the key.
fn shown_status(status: &serde_json::Value) -> Vec<(&'static str, Value)> {
    let text = |name: &str| Value::from(status[name].as_str().unwrap_or_default());
    let ips: Vec<&str> = status["ips"].as_array().map(|ips| ips.iter().filter_map(|ip| ip.as_str()).collect()).unwrap_or_default();
    vec![
        ("on", Value::Bool(status["on"].as_bool().unwrap_or(false))),
        ("use", text("use")),
        ("name", text("name")),
        ("ip", Value::from(ips.iter().find(|ip| !ip.contains(':')).or(ips.first()).copied().unwrap_or_default())),
        ("host", text("host")),
        ("port", Value::from(status["port"].as_f64().unwrap_or(0.0))),
        ("error", text("error")),
    ]
}

/// What the page is told of an engine's refusal: a fixed word.
fn link_error(error: &str) -> String {
    if error.contains("no_tailnet") {
        "no_tailnet".into()
    } else if crate::engine::outcome(error) == "offline" {
        "offline".into()
    } else {
        "failed".into()
    }
}

async fn link_call(method: &str, params: serde_json::Value) -> Result<serde_json::Value, String> {
    crate::engine::get().call(method, params).await.map_err(|error| link_error(&error))
}

pub fn install(host: &ScriptHost) {
    // This computer's code: its phone link turned on (the tailnet found
    // and listened on), as a QR. The key goes into the code alone.
    host.async_action("phone.connect_code", |_| async move {
        crate::plugins::preview::refuse("phone.connect_code")?;
        let status = link_call("SetPhoneLink", serde_json::json!({ "on": true })).await?;
        let code = direct_code(&status)?;
        let (svg, modules) = qr_svg(&code).ok_or("too_long")?;
        let mut fields = shown_status(&status);
        fields.push(("svg", Value::from(svg.as_str())));
        fields.push(("modules", Value::from(modules as f64)));
        Ok(Value::record(fields))
    });
    host.async_action("phone.link_status", |_| async move {
        crate::plugins::preview::refuse("phone.link_status")?;
        Ok(Value::record(shown_status(&link_call("PhoneLinkStatus", serde_json::json!({})).await?)))
    });
    // { on?, use? = "name" | "ip" }
    host.async_action("phone.link_set", |arg| async move {
        crate::plugins::preview::refuse("phone.link_set")?;
        let mut params = serde_json::Map::new();
        if let Some(on) = arg.field("on") {
            params.insert("on".into(), serde_json::Value::Bool(on.truthy()));
        }
        if let Some(choice) = arg.field("use").map(crate::auth::text).filter(|c| c == "name" || c == "ip") {
            params.insert("use".into(), serde_json::Value::from(choice));
        }
        Ok(Value::record(shown_status(&link_call("SetPhoneLink", serde_json::Value::Object(params)).await?)))
    });
    host.async_action("phone.link_new_key", |_| async move {
        crate::plugins::preview::refuse("phone.link_new_key")?;
        Ok(Value::record(shown_status(&link_call("NewPhoneLinkKey", serde_json::json!({})).await?)))
    });
    #[cfg(target_os = "android")]
    crate::android_camera::install(host);
}

/// What the log says of an edge: a fixed word, never text of the edge
/// itself (a code is anyone's to make).
pub fn edge_diagnostic(edge: &str) -> &'static str {
    match canonical_edge(edge) {
        Some(edge) if edge.starts_with("https://") => "an https relay",
        Some(_) => "an http relay",
        None => "no relay address",
    }
}

/// What the page shows of a kept edge (canonical, so only a scheme and a
/// port of digits): its scheme and port, not where it is.
pub fn shown_edge(edge: &str) -> String {
    let Some(edge) = canonical_edge(edge) else { return String::new() };
    let (scheme, rest) = edge.split_once("://").unwrap_or(("", ""));
    match rest.rsplit_once(':').filter(|(host, port)| !host.ends_with(']') || !port.is_empty()) {
        Some((_, port)) if port.bytes().all(|b| b.is_ascii_digit()) => format!("{scheme}://…:{port}"),
        _ => format!("{scheme}://…"),
    }
}

/// Keep `connect` in `dir` as the edge and bearer, then `restart` the
/// engine on them. A restart that fails puts back what was kept before
/// (both halves, or none), so the running engine and the kept pair agree.
pub async fn keep_and_restart<F, Fut>(dir: &std::path::Path, connect: &Connect, restart: F) -> Result<(), &'static str>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<(), String>>,
{
    let before = zend_cli::edge_record_in(dir);
    zend_cli::store_edge_config(dir, &connect.edge, &connect.token).map_err(|_| "store")?;
    if restart().await.is_err() {
        if zend_cli::restore_edge_record(dir, before.as_ref()).is_err() {
            eprintln!("zend: phone link: the engine didn't stop, and the relay kept before couldn't be put back");
        }
        return Err("engine");
    }
    Ok(())
}

/// The desktop this app is a window onto, if it keeps one (a v=2 code
/// scanned): then no engine runs here, and every dial goes to that one.
pub fn kept_desktop() -> Option<Direct> {
    kept_desktop_in(&zend_cli::config_dir()?)
}

/// The desktop kept in `dir`: a record holding a tailnet host and a key
/// (anything else is none, and the app runs its own engine).
pub fn kept_desktop_in(dir: &std::path::Path) -> Option<Direct> {
    let (host, port, key) = zend_cli::desktop_record_in(dir)?;
    let host = canonical_host(&host)?;
    valid_key(&key).then_some(Direct { host, port, key })
}

/// Keep what the code holds as this app's (zend_cli::config_dir) and
/// start again on it: the running engine (or connection) is asked to stop,
/// and the supervisor dials the next. A v=1 code keeps a relay and starts
/// this app's own engine on it (no desktop kept any more); a v=2 code keeps
/// the desktop, and the app becomes a window onto that desktop's engine.
/// An engine that won't stop keeps what it had, and so does the folder.
#[cfg(target_os = "android")]
pub async fn connect(code: &str) -> Result<Value, String> {
    let code = decode(code).map_err(|why| {
        log::warn!("zend: phone link: not a code ({why})");
        "bad_code".to_owned()
    })?;
    let dir = zend_cli::config_dir().ok_or("store")?;
    let stop = || async {
        match crate::engine::get().stop().await {
            // No connection and no engine thread: none runs, and the next
            // to start reads what's kept.
            Err(error) if error == "offline" && !crate::android::engine_running() => Ok(()),
            other => other,
        }
    };
    let failed = |word: &'static str| {
        log::error!("zend: phone link: {}", if word == "store" { "the code couldn't be kept" } else { "the engine didn't stop: nothing changed" });
        word.to_owned()
    };
    match code {
        Code::Edge(connect) => {
            log::info!("zend: phone link: keeping {} and its key, then starting the engine on them", edge_diagnostic(&connect.edge));
            zend_cli::clear_desktop_record(&dir).map_err(|_| failed("store"))?;
            keep_and_restart(&dir, &connect, stop).await.map_err(failed)?;
            Ok(Value::record([("edge", Value::from(shown_edge(&connect.edge).as_str()))]))
        }
        Code::Direct(desktop) => {
            // (The key is never logged; the host is the person's own desktop.)
            log::info!("zend: phone link: keeping the desktop at {}:{}, then driving its engine", desktop.host, desktop.port);
            keep_desktop_and_restart(&dir, &desktop, stop).await.map_err(failed)?;
            Ok(Value::record([("desktop", Value::from(desktop.host.as_str()))]))
        }
    }
}

/// Keep desktop in dir, then restart: one that fails puts back the
/// desktop kept before (or none).
pub async fn keep_desktop_and_restart<F, Fut>(dir: &std::path::Path, desktop: &Direct, restart: F) -> Result<(), &'static str>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<(), String>>,
{
    let before = zend_cli::desktop_record_in(dir);
    zend_cli::store_desktop_record(dir, &desktop.host, desktop.port, &desktop.key).map_err(|_| "store")?;
    if restart().await.is_err() {
        let put_back = match &before {
            Some((host, port, key)) => zend_cli::store_desktop_record(dir, host, *port, key),
            None => zend_cli::clear_desktop_record(dir),
        };
        if put_back.is_err() {
            eprintln!("zend: phone link: the engine didn't stop, and the desktop kept before couldn't be put back");
        }
        return Err("engine");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_code_holds_its_edge_and_key_and_nothing_else_is_one() {
        let code = encode("http://100.64.0.1:27640/", "u-phone+1@org");
        assert_eq!(code, "zend://connect?v=1&edge=http%3A%2F%2F100.64.0.1%3A27640%2F&token=u-phone%2B1%40org");
        assert_eq!(
            decode(&format!("  {code}\n")),
            Ok(Code::Edge(Connect { edge: "http://100.64.0.1:27640".into(), token: "u-phone+1@org".into() }))
        );
        for bad in [
            "",
            "http://100.64.0.1:27640",
            "zend://connect?v=2&edge=http%3A%2F%2Fa&token=t",
            "zend://connect?v=1&edge=ftp%3A%2F%2Fa&token=t",
            "zend://connect?v=1&edge=http%3A%2F%2Fa&token=",
            "zend://connect?v=1&edge=http%3A%2F%2Fa&token=a%20b",
            "zend://connect?v=1&edge=http%3A%2F%2Fa&token=%zz",
        ] {
            assert!(decode(bad).is_err(), "{bad}");
        }
    }

    /// Codes made to hurt: each refused before anything is kept.
    #[test]
    fn a_malicious_code_is_refused() {
        let code = |edge: &str| encode(edge, "u-x@org");
        let long_field = format!("zend://connect?v=1&junk={}&edge=http%3A%2F%2Fa&token=t", "A".repeat(100_000));
        for bad in [
            code("http://user:pw@relay.example:27640"),
            code("http://user@relay.example"),
            code("http://relay.example:27640/?next=x"),
            code("http://relay.example:27640#frag"),
            code("http://relay.example/api"),
            code("http:///"),
            code("http://"),
            code("https:////relay.example"),
            code("http://relay.example:99999"),
            code("http://relay.example:0"),
            code("http://relay.example:12ab"),
            code("http:\\\\relay.example"),
            code("http://relay.example\n:27640"),
            code("http://rel ay.example"),
            code("javascript://relay.example"),
            code(&format!("http://{}.example", "a".repeat(300))),
            "zend://connect?v=1&edge=http%3A%2F%2Fa&edge=http%3A%2F%2Fb&token=t".to_owned(),
            "zend://connect?v=1&edge=http%3A%2F%2Fa&token=t&token=u".to_owned(),
            "zend://connect?v=1&edge=http%3A%2F%2Fa&token=t%0Ax".to_owned(),
            encode("http://a", &"t".repeat(5000)),
            long_field,
        ] {
            assert!(decode(&bad).is_err(), "accepted {:.80}", bad);
        }
    }

    /// A stop, or a scan started after it, ends a scan; a scan's panic
    /// leaves nothing to clear (the next begins anyway).
    #[test]
    fn a_stop_or_a_newer_scan_ends_the_one_running() {
        let scans = Attempts::new();
        let first = scans.begin();
        assert!(scans.live(first));
        scans.stop();
        assert!(!scans.live(first));
        let second = scans.begin();
        let third = scans.begin();
        assert!(!scans.live(second) && scans.live(third));
        let panicked = std::thread::spawn(move || -> () { panic!("scan {third} panics") }).join();
        assert!(panicked.is_err());
        let fourth = scans.begin();
        assert!(scans.live(fourth));
    }

    /// The log's words for an edge are fixed, whatever the code held.
    #[test]
    fn the_log_says_nothing_of_the_edge_itself() {
        for edge in ["http://evil.example:1/\nzend: engine: connected", "https://secret-host.example", "http://a b", "x"] {
            let said = edge_diagnostic(edge);
            assert!(["an https relay", "an http relay", "no relay address"].contains(&said));
            assert!(!said.contains("example") && !said.contains("evil"));
        }
        assert_eq!(shown_edge("http://100.64.0.1:27640"), "http://…:27640");
        assert_eq!(shown_edge("https://relay.example"), "https://…");
        assert_eq!(shown_edge("http://a\n:1"), "");
    }

    fn block_on<F: std::future::Future>(f: F) -> F::Output {
        let waker = std::task::Waker::noop();
        let mut cx = std::task::Context::from_waker(&waker);
        let mut f = std::pin::pin!(f);
        loop {
            if let std::task::Poll::Ready(out) = f.as_mut().poll(&mut cx) {
                return out;
            }
        }
    }

    /// An engine that won't stop leaves the pair kept before, whole; one
    /// that stops keeps the new pair.
    #[test]
    fn a_failed_restart_puts_back_the_relay_kept_before() {
        let dir = tempfile::tempdir().unwrap();
        let old = Connect { edge: "http://100.64.0.1:27640".into(), token: "u-old@org".into() };
        let new = Connect { edge: "http://100.64.0.2:27641".into(), token: "u-new@org".into() };
        block_on(keep_and_restart(dir.path(), &old, || async { Ok(()) })).unwrap();
        let failed = block_on(keep_and_restart(dir.path(), &new, || async { Err("busy".to_owned()) }));
        assert_eq!(failed, Err("engine"));
        assert_eq!(zend_cli::edge_record_in(dir.path()), Some((old.edge.clone(), old.token.clone())));
        // With nothing kept before, nothing is kept after.
        let empty = tempfile::tempdir().unwrap();
        let _ = block_on(keep_and_restart(empty.path(), &new, || async { Err("busy".to_owned()) }));
        assert_eq!(zend_cli::edge_config_in(empty.path()), None);
        block_on(keep_and_restart(dir.path(), &new, || async { Ok(()) })).unwrap();
        assert_eq!(zend_cli::edge_record_in(dir.path()), Some((new.edge, new.token)));
    }

    /// What's kept is the edge's canonical form: scheme and host in lower
    /// case, a default port dropped, no trailing slash.
    #[test]
    fn the_edge_kept_is_canonical() {
        for (given, kept) in [
            ("http://100.64.0.1:27640/", "http://100.64.0.1:27640"),
            ("HTTPS://Relay.Example:443", "https://relay.example"),
            ("http://relay.example:80/", "http://relay.example"),
            ("http://[fd7a:115c::1]:27640", "http://[fd7a:115c::1]:27640"),
        ] {
            let edge = match decode(&encode(given, "u-x@org")) {
                Ok(Code::Edge(c)) => Ok(c.edge),
                other => Err(format!("{other:?}")),
            };
            assert_eq!(edge, Ok(kept.to_owned()), "{given}");
        }
    }

    /// A v=2 code holds a desktop's tailnet host, port and key, and nothing
    /// else is one: a host off the tailnet, a bad port or key is refused.
    #[test]
    fn a_direct_code_holds_a_tailnet_host_a_port_and_a_key() {
        let key = "0123456789abcdef".repeat(4);
        let code = encode_direct("omarchy.tailf80f26.ts.net", 27871, &key);
        assert_eq!(code, format!("zend://connect?v=2&host=omarchy.tailf80f26.ts.net&port=27871&key={key}"));
        let want = |host: &str| Ok(Code::Direct(Direct { host: host.into(), port: 27871, key: key.clone() }));
        assert_eq!(decode(&format!(" {code}\n")), want("omarchy.tailf80f26.ts.net"));
        assert_eq!(decode(&encode_direct("100.102.122.72", 27871, &key)), want("100.102.122.72"));
        assert_eq!(decode(&encode_direct("[FD7A:115C:A1E0::1]", 27871, &key.to_uppercase())), want("fd7a:115c:a1e0::1"));
        assert_eq!(decode(&encode_direct("Omarchy.TS.net.", 27871, &key)), want("omarchy.ts.net"));
        for (host, port, k) in [
            ("192.168.1.5", "27871", key.as_str()),
            ("127.0.0.1", "27871", key.as_str()),
            ("0.0.0.0", "27871", key.as_str()),
            ("8.8.8.8", "27871", key.as_str()),
            ("::1", "27871", key.as_str()),
            ("1.2.3", "27871", key.as_str()),
            ("a b", "27871", key.as_str()),
            ("evil.example/path", "27871", key.as_str()),
            ("user@host", "27871", key.as_str()),
            ("-bad.example", "27871", key.as_str()),
            ("", "27871", key.as_str()),
            ("omarchy", "0", key.as_str()),
            ("omarchy", "65536", key.as_str()),
            ("omarchy", "+80", key.as_str()),
            ("omarchy", "", key.as_str()),
            ("omarchy", "27871", "short"),
            ("omarchy", "27871", &"g".repeat(64)),
            ("omarchy", "27871", ""),
        ] {
            let bad = format!("zend://connect?v=2&host={}&port={port}&key={}", escape(host), escape(k));
            assert!(decode(&bad).is_err(), "accepted {bad}");
        }
        assert!(decode(&format!("{code}&host=other")).is_err(), "a field twice");
        assert!(decode(&format!("zend://connect?v=3&host=omarchy&port=1&key={key}")).is_err());
        // A v=1 code still reads.
        assert!(matches!(decode(&encode("http://100.64.0.1:27640", "u-x@org")), Ok(Code::Edge(_))));
        assert_eq!(ws_url("fd7a:115c:a1e0::1", 1), "ws://[fd7a:115c:a1e0::1]:1");
        assert_eq!(ws_url("omarchy.ts.net", 27871), "ws://omarchy.ts.net:27871");
    }

    /// The code a status gives is v=2 with its host, and the page is told
    /// everything but the key.
    #[test]
    fn the_shown_code_comes_from_the_engine_s_status_and_the_page_never_sees_the_key() {
        let key = "ab".repeat(32);
        let status = serde_json::json!({ "on": true, "use": "ip", "name": "omarchy.ts.net", "ips": ["fd7a:115c:a1e0::9", "100.102.122.72"], "host": "100.102.122.72", "port": 27871, "key": key });
        let code = direct_code(&status).unwrap();
        assert_eq!(decode(&code), Ok(Code::Direct(Direct { host: "100.102.122.72".into(), port: 27871, key: key.clone() })));
        let shown = Value::record(shown_status(&status)).to_json();
        assert!(!shown.contains(&key), "{shown}");
        assert!(shown.contains("100.102.122.72") && shown.contains("omarchy.ts.net"), "{shown}");
        assert_eq!(direct_code(&serde_json::json!({ "on": false, "port": 1 })), Err("no_tailnet".into()));
        assert_eq!(link_error("rpc error: no_tailnet"), "no_tailnet");
    }

    /// A desktop kept, then a restart that fails: the one kept before (or
    /// none) is put back.
    #[test]
    fn a_failed_restart_puts_back_the_desktop_kept_before() {
        let dir = tempfile::tempdir().unwrap();
        let old = Direct { host: "100.64.0.1".into(), port: 27871, key: "a".repeat(64) };
        let new = Direct { host: "omarchy.ts.net".into(), port: 27872, key: "b".repeat(64) };
        block_on(keep_desktop_and_restart(dir.path(), &old, || async { Ok(()) })).unwrap();
        assert_eq!(block_on(keep_desktop_and_restart(dir.path(), &new, || async { Err("busy".to_owned()) })), Err("engine"));
        assert_eq!(zend_cli::desktop_record_in(dir.path()), Some((old.host.clone(), old.port, old.key.clone())));
        let empty = tempfile::tempdir().unwrap();
        let _ = block_on(keep_desktop_and_restart(empty.path(), &new, || async { Err("busy".to_owned()) }));
        assert_eq!(zend_cli::desktop_record_in(empty.path()), None);
    }

    /// A phone keeping a desktop is a window onto it (src/engine/rpc.rs
    /// dials it, src/engine/manager.rs starts no engine); a record that
    /// isn't one leaves the app on its own engine.
    #[test]
    fn the_remote_mode_is_chosen_from_the_record() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(kept_desktop_in(dir.path()), None);
        let desktop = Direct { host: "omarchy.tailf80f26.ts.net".into(), port: 27871, key: "c".repeat(64) };
        block_on(keep_desktop_and_restart(dir.path(), &desktop, || async { Ok(()) })).unwrap();
        assert_eq!(kept_desktop_in(dir.path()), Some(desktop));
        zend_cli::store_desktop_record(dir.path(), "192.168.1.9", 27871, &"c".repeat(64)).unwrap();
        assert_eq!(kept_desktop_in(dir.path()), None, "a host off the tailnet");
        zend_cli::store_desktop_record(dir.path(), "100.64.0.1", 27871, "short").unwrap();
        assert_eq!(kept_desktop_in(dir.path()), None, "a key that isn't one");
        zend_cli::clear_desktop_record(dir.path()).unwrap();
        assert_eq!(kept_desktop_in(dir.path()), None);
    }

    /// The code drawn as the page draws it (`ui.svg`, rasterized) reads back
    /// as the code: decoded from its pixels with the phone's decoder.
    #[test]
    fn the_drawn_code_reads_back_from_its_pixels() {
        let code = encode("http://100.64.0.1:27640", "u-fake-g3@invalid");
        let (svg, modules) = qr_svg(&code).unwrap();
        assert!((29..=61).contains(&modules), "{modules} modules");
        assert_eq!(decode_png_pixels(&rasterize(&svg, 240)), Some(code));
    }

    /// `svg` drawn `size` pixels square, as greyscale.
    pub(crate) fn rasterize(svg: &str, size: u32) -> (usize, usize, Vec<u8>) {
        use resvg::{tiny_skia, usvg};
        let tree = usvg::Tree::from_str(svg, &usvg::Options::default()).unwrap();
        let mut pixmap = tiny_skia::Pixmap::new(size, size).unwrap();
        let scale = size as f32 / tree.size().width();
        resvg::render(&tree, tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
        let grey = pixmap.pixels().iter().map(|p| ((p.red() as u32 + p.green() as u32 + p.blue() as u32) / 3) as u8).collect();
        (size as usize, size as usize, grey)
    }

    /// The text of the one QR code in a greyscale picture.
    pub(crate) fn decode_png_pixels((w, h, grey): &(usize, usize, Vec<u8>)) -> Option<String> {
        let mut image = rqrr::PreparedImage::prepare_from_greyscale(*w, *h, |x, y| grey[y * w + x]);
        let grids = image.detect_grids();
        grids.first()?.decode().ok().map(|(_, text)| text)
    }
}
