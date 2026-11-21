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

/// The edge and bearer a code holds (pasted text may come with spaces or
/// a line break around it), or why it's not one.
pub fn decode(text: &str) -> Result<Connect, &'static str> {
    let query = text.trim().strip_prefix(PREFIX).ok_or("not a Zend connect code")?;
    let (mut version, mut edge, mut token) = (None, None, None);
    for pair in query.split('&') {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        let value = unescape(value).ok_or("badly encoded")?;
        match key {
            "v" => version = Some(value),
            "edge" => edge = Some(value),
            "token" => token = Some(value),
            _ => {}
        }
    }
    if version.as_deref() != Some("1") {
        return Err("a code of another version");
    }
    let edge = edge.filter(|e| valid_edge(e)).ok_or("no relay address in it")?;
    let token = token.filter(|t| valid_token(t)).ok_or("no key in it")?;
    Ok(Connect { edge: edge.trim_end_matches('/').to_owned(), token })
}

fn valid_edge(edge: &str) -> bool {
    let rest = edge.strip_prefix("http://").or_else(|| edge.strip_prefix("https://"));
    rest.is_some_and(|rest| !rest.is_empty() && rest.len() < 2048)
        && edge.chars().all(|c| c.is_ascii_graphic())
}

fn valid_token(token: &str) -> bool {
    (1..4096).contains(&token.len()) && token.chars().all(|c| c.is_ascii_graphic())
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

/// Whether `edge` is a relay this computer is on: not the never-answering
/// default an unconfigured Zend has.
fn is_relay(edge: &str) -> bool {
    let host = edge.split_once("://").map_or("", |(_, rest)| rest.split(['/', ':']).next().unwrap_or(""));
    !host.is_empty() && !host.ends_with(".invalid")
}

/// This computer's code: the edge its engine uses and its bearer.
fn this_computers_code() -> Result<String, String> {
    let edge = crate::auth::endpoint().edge;
    let token = zend_cli::edge_token_from_env().ok_or("no_edge")?;
    if !is_relay(&edge) {
        return Err("no_edge".into());
    }
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

/// Keep the code's edge and bearer as this app's (`zend_cli::config_dir`)
/// and start its engine again on them: the running one is asked to stop,
/// and the supervisor starts the next, which reads them.
#[cfg(target_os = "android")]
pub async fn connect(code: &str) -> Result<Value, String> {
    let Connect { edge, token } = decode(code).map_err(|why| {
        log::warn!("zend: phone link: not a code: {why}");
        "bad_code".to_owned()
    })?;
    let dir = zend_cli::config_dir().ok_or("store")?;
    zend_cli::store_edge_config(&dir, &edge, &token).map_err(|error| {
        log::error!("zend: phone link: keeping the code: {error}");
        "store".to_owned()
    })?;
    log::info!("zend: phone link: kept the relay {} and its key; starting the engine on them", crate::android::redacted(&edge));
    crate::engine::get().stop().await.map_err(|error| {
        log::error!("zend: phone link: the engine didn't stop: {error}");
        "engine".to_owned()
    })?;
    Ok(Value::record([("edge", Value::from(crate::android::redacted(&edge).as_str()))]))
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
