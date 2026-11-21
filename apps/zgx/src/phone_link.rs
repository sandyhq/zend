//! Connecting a phone to this computer's relay (Settings → Devices).
//!
//! The owner's relay is the self-hosted edge (on a VM over Tailscale), with
//! the edge's dev bearer as the account: no hosted sign-in. The computer
//! shows a QR code holding its edge and its own bearer, on a click and for
//! a minute; Zend on the phone scans it (src/android_camera.rs) or takes
//! its text pasted, keeps both in its config folder (`zend_cli`'s
//! `edge-url`, `edge-token`, mode 600) and starts its engine again on them.
//!
//! ```text
//! call    phone.connect_code  (none)    { svg, modules }: this computer's code, as a QR
//!                                       (fails "no_edge": no relay or no bearer here)
//! call    phone.connect       { code }  { edge }: the code kept, the engine started on it
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

/// What a code starts with.
const PREFIX: &str = "zend://connect?";

/// A connect code's contents.
#[derive(Debug, PartialEq, Eq)]
pub struct Connect {
    pub edge: String,
    pub token: String,
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

/// The edge and bearer a code holds (pasted text may come with spaces or
/// a line break around it), or why it's not one. The edge is checked as a
/// URL (http or https, a host, a valid port, nothing else: no user, path,
/// query or fragment) and given in its canonical form; the bearer is one
/// printable word. Fields other than these three are passed over without
/// decoding them; one of them twice refuses the code.
pub fn decode(text: &str) -> Result<Connect, &'static str> {
    let text = text.trim();
    if text.len() > MAX_CODE {
        return Err("too long");
    }
    let query = text.strip_prefix(PREFIX).ok_or("not a Zend connect code")?;
    let (mut version, mut edge, mut token) = (None, None, None);
    for pair in query.split('&') {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        let slot = match key {
            "v" => &mut version,
            "edge" => &mut edge,
            "token" => &mut token,
            _ => continue,
        };
        if slot.is_some() {
            return Err("a field twice");
        }
        *slot = Some(unescape(value).ok_or("badly encoded")?);
    }
    if version.as_deref() != Some("1") {
        return Err("a code of another version");
    }
    let edge = edge.as_deref().and_then(canonical_edge).ok_or("no relay address in it")?;
    let token = token.filter(|t| valid_token(t)).ok_or("no key in it")?;
    Ok(Connect { edge, token })
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

/// Whether `edge` is a relay this computer is on: an address a phone may
/// keep, not the never-answering default an unconfigured Zend has.
fn is_relay(edge: &str) -> bool {
    canonical_edge(edge).is_some_and(|edge| {
        let host = edge.split_once("://").map_or("", |(_, rest)| rest.rsplit_once(':').map_or(rest, |(host, _)| host));
        !host.ends_with(".invalid")
    })
}

/// This computer's code: the edge its engine uses and its bearer, both as
/// a phone would accept them.
fn this_computers_code() -> Result<String, String> {
    let edge = crate::auth::endpoint().edge;
    let token = zend_cli::edge_token_from_env().filter(|t| valid_token(t)).ok_or("no_edge")?;
    let edge = canonical_edge(&edge).filter(|e| is_relay(e)).ok_or("no_edge")?;
    Ok(encode(&edge, &token))
}

pub fn install(host: &ScriptHost) {
    host.async_action("phone.connect_code", |_| async move {
        crate::plugins::preview::refuse("phone.connect_code")?;
        let code = this_computers_code()?;
        let (svg, modules) = qr_svg(&code).ok_or("too_long")?;
        Ok(Value::record([("svg", Value::from(svg.as_str())), ("modules", Value::from(modules as f64))]))
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

/// Keep the code's edge and bearer as this app's (`zend_cli::config_dir`)
/// and start its engine again on them: the running one is asked to stop,
/// and the supervisor starts the next, which reads them. An engine that
/// won't stop keeps what it had, and so does the config folder.
#[cfg(target_os = "android")]
pub async fn connect(code: &str) -> Result<Value, String> {
    let connect = decode(code).map_err(|why| {
        log::warn!("zend: phone link: not a code ({why})");
        "bad_code".to_owned()
    })?;
    let dir = zend_cli::config_dir().ok_or("store")?;
    log::info!("zend: phone link: keeping {} and its key, then starting the engine on them", edge_diagnostic(&connect.edge));
    keep_and_restart(&dir, &connect, || async {
        match crate::engine::get().stop().await {
            // No connection and no engine thread: none runs, and the next
            // to start reads what's kept.
            Err(error) if error == "offline" && !crate::android::engine_running() => Ok(()),
            other => other,
        }
    })
    .await
    .map_err(|word| {
        log::error!("zend: phone link: {}", if word == "store" { "the code couldn't be kept" } else { "the engine didn't stop: nothing changed" });
        word.to_owned()
    })?;
    Ok(Value::record([("edge", Value::from(shown_edge(&connect.edge).as_str()))]))
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
            Ok(Connect { edge: "http://100.64.0.1:27640".into(), token: "u-phone+1@org".into() })
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
            assert_eq!(decode(&encode(given, "u-x@org")).map(|c| c.edge), Ok(kept.to_owned()), "{given}");
        }
    }

    #[test]
    fn only_a_relay_is_offered() {
        assert!(is_relay("http://100.115.228.74:27640"));
        assert!(is_relay("https://relay.example.com"));
        assert!(!is_relay("https://edge.zend.invalid"));
        assert!(!is_relay(""));
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
