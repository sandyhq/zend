//! The edge and sign-in client `zend` resolves from its environment.

/// Production edge (Cloudflare Worker + Durable Objects on the zend.invalid zone).
/// `ZEND_EDGE_URL` overrides (local dev / self-hosting).
pub const DEFAULT_EDGE_URL: &str = "https://edge.zend.invalid";

/// Production WorkOS AuthKit client id — public knowledge (it appears in every
/// authorize URL), so baking it in is safe. Overridden by `ZEND_WORKOS_CLIENT_ID`;
/// set it to the empty string — or set a dev bearer via `ZEND_EDGE_TOKEN` — to
/// force dev-mode auth instead.
pub const DEFAULT_WORKOS_CLIENT_ID: &str = "client_01KWD0EAKZKD50YCQJNYSRE4BY";

/// zend's own settings, beside nothing else: `$XDG_CONFIG_HOME/zend` or
/// `~/.config/zend` (`%APPDATA%\\zend` on Windows). It holds `edge-url` (the
/// self-hosted edge, e.g. `http://100.115.228.74:27640`) and `edge-token`
/// (the edge's dev bearer `u-…@org`, mode 600), so neither has to sit in an
/// environment every process inherits.
pub fn config_dir() -> Option<std::path::PathBuf> {
    let var = |key: &str| std::env::var_os(key).filter(|v| !v.is_empty()).map(std::path::PathBuf::from);
    if cfg!(windows) {
        return var("APPDATA").map(|dir| dir.join("zend"));
    }
    var("XDG_CONFIG_HOME")
        .or_else(|| var("HOME").map(|home| home.join(".config")))
        .map(|dir| dir.join("zend"))
}

fn config_file(name: &str) -> Option<String> {
    let text = std::fs::read_to_string(config_dir()?.join(name)).ok()?;
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

/// The edge's dev bearer: `ZEND_EDGE_TOKEN`, else the file at
/// `ZEND_EDGE_TOKEN_FILE`, else [`config_dir`]'s `edge-token`.
pub fn edge_token_from_env() -> Option<String> {
    if let Some(token) = std::env::var("ZEND_EDGE_TOKEN").ok().filter(|t| !t.trim().is_empty()) {
        return Some(token.trim().to_owned());
    }
    if let Some(path) = std::env::var_os("ZEND_EDGE_TOKEN_FILE").filter(|p| !p.is_empty()) {
        return std::fs::read_to_string(path).ok().map(|t| t.trim().to_owned()).filter(|t| !t.is_empty());
    }
    config_file("edge-token")
}

/// The edge: `ZEND_EDGE_URL`, else [`config_dir`]'s `edge-url`, else
/// [`DEFAULT_EDGE_URL`] (which never resolves: offline).
pub fn edge_url_from_env() -> String {
    if let Some(url) = std::env::var("ZEND_EDGE_URL").ok().filter(|s| !s.trim().is_empty()) {
        return url;
    }
    if let Some(url) = config_file("edge-url") {
        return url;
    }
    std::env::var("ZEND_EDGE_URL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_EDGE_URL.into())
}

/// WorkOS client id resolution: explicit env wins (empty string = dev mode);
/// otherwise a `ZEND_EDGE_TOKEN` dev bearer keeps dev mode (smoke tests,
/// local wrangler); otherwise the baked production client id makes optional
/// sync available while a bare start remains local-only.
pub fn workos_client_id_from_env(edge_token: &Option<String>) -> Option<String> {
    match std::env::var("ZEND_WORKOS_CLIENT_ID") {
        Ok(v) if v.trim().is_empty() => None,
        Ok(v) => Some(v),
        // zend signs in against its own edge's dev bearers only: no
        // hosted sign-in client unless one is named.
        Err(_) => {
            let _ = edge_token;
            None
        }
    }
}

/// The engine's IPC port: `ZEND_IPC_PORT`, else 27871.
pub fn ipc_port_from_env() -> u16 {
    ipc_port_from(std::env::var("ZEND_IPC_PORT").ok().as_deref())
}

/// [`ipc_port_from_env`] from the variable's value.
pub fn ipc_port_from(value: Option<&str>) -> u16 {
    value.and_then(|p| p.parse().ok()).unwrap_or(DEFAULT_IPC_PORT)
}

/// The production engine's IPC port.
/// zend's own: not 27654/27655, the ports of the app it was forked from,
/// which may run beside it.
pub const DEFAULT_IPC_PORT: u16 = 27871;
