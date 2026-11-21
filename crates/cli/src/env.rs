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

/// The file that holds the edge and its bearer as one record (written by
/// [`store_edge_config`]): a first line naming the format, the edge, the
/// bearer. Read before the older pair of files `edge-url` and `edge-token`
/// (written by hand, or by an older Zend), which are read only when it's
/// absent.
const EDGE_RECORD: &str = "edge";
const EDGE_RECORD_HEAD: &str = "zend-edge 1";

/// Keep the edge config in `dir` (a [`config_dir`]): the edge and its
/// bearer as one record, `edge`, written whole or not at all (a new file
/// beside it, flushed, renamed over) and readable by this user alone (mode
/// 600 from its creation, so the bearer is never readable by others even
/// for a moment). A store that fails leaves what was kept before, both
/// halves of it. What the phone keeps after scanning the desktop's code.
pub fn store_edge_config(dir: &std::path::Path, url: &str, token: &str) -> std::io::Result<()> {
    let (url, token) = (url.trim(), token.trim());
    if url.is_empty() || token.is_empty() || url.contains(['\n', '\r']) || token.contains(['\n', '\r']) {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "an edge and a bearer on one line each"));
    }
    std::fs::create_dir_all(dir)?;
    write_private(&dir.join(EDGE_RECORD), &format!("{EDGE_RECORD_HEAD}\n{url}\n{token}"))
}

/// The record [`store_edge_config`] keeps in `dir`: its edge and bearer.
/// None when there's none, or it isn't one (then nothing of it is used).
pub fn edge_record_in(dir: &std::path::Path) -> Option<(String, String)> {
    let text = std::fs::read_to_string(dir.join(EDGE_RECORD)).ok()?;
    let mut lines = text.lines();
    if lines.next()? != EDGE_RECORD_HEAD {
        return None;
    }
    let url = lines.next()?.trim();
    let token = lines.next()?.trim();
    (!url.is_empty() && !token.is_empty() && lines.all(|l| l.trim().is_empty())).then(|| (url.to_owned(), token.to_owned()))
}

/// Put back what `dir` kept before a [`store_edge_config`]: that record,
/// or (none) no record, so the older files, if any, are read again.
pub fn restore_edge_record(dir: &std::path::Path, before: Option<&(String, String)>) -> std::io::Result<()> {
    match before {
        Some((url, token)) => store_edge_config(dir, url, token),
        None => match std::fs::remove_file(dir.join(EDGE_RECORD)) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
            _ => Ok(()),
        },
    }
}

/// The edge and bearer kept in `dir`, read as one: the record, else the
/// older two files (the edge none: nothing kept).
pub fn edge_config_in(dir: &std::path::Path) -> Option<(String, Option<String>)> {
    if dir.join(EDGE_RECORD).exists() {
        return edge_record_in(dir).map(|(url, token)| (url, Some(token)));
    }
    let read = |name: &str| std::fs::read_to_string(dir.join(name)).ok().map(|t| t.trim().to_owned()).filter(|t| !t.is_empty());
    Some((read("edge-url")?, read("edge-token")))
}

/// The edge and its bearer as [`edge_url_from_env`] and
/// [`edge_token_from_env`] resolve them, with the config folder read once:
/// a pair kept together is never seen half old, half new.
pub fn edge_from_env() -> (String, Option<String>) {
    let kept = config_dir().and_then(|dir| edge_config_in(&dir));
    let url = env_url().or_else(|| kept.as_ref().map(|(url, _)| url.clone())).unwrap_or_else(|| DEFAULT_EDGE_URL.into());
    let token = env_token().or_else(|| kept.and_then(|(_, token)| token));
    (url, token)
}

fn write_private(path: &std::path::Path, text: &str) -> std::io::Result<()> {
    use std::io::Write as _;
    let name = path.file_name().and_then(|name| name.to_str()).unwrap_or("edge");
    let temp = path.with_file_name(format!(".{name}.{}.new", std::process::id()));
    let _ = std::fs::remove_file(&temp);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let written = (|| {
        let mut file = options.open(&temp)?;
        file.write_all(text.as_bytes())?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        std::fs::rename(&temp, path)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    written
}

/// The edge's dev bearer: `ZEND_EDGE_TOKEN`, else the file at
/// `ZEND_EDGE_TOKEN_FILE`, else the one kept in [`config_dir`] (its `edge`
/// record, else `edge-token`).
pub fn edge_token_from_env() -> Option<String> {
    env_token().or_else(|| config_dir().and_then(|dir| edge_config_in(&dir)).and_then(|(_, token)| token))
}

fn env_token() -> Option<String> {
    if let Some(token) = std::env::var("ZEND_EDGE_TOKEN").ok().filter(|t| !t.trim().is_empty()) {
        return Some(token.trim().to_owned());
    }
    if let Some(path) = std::env::var_os("ZEND_EDGE_TOKEN_FILE").filter(|p| !p.is_empty()) {
        return std::fs::read_to_string(path).ok().map(|t| t.trim().to_owned()).filter(|t| !t.is_empty());
    }
    None
}

/// The edge: `ZEND_EDGE_URL`, else the one kept in [`config_dir`] (its
/// `edge` record, else `edge-url`), else [`DEFAULT_EDGE_URL`] (which never
/// resolves: offline).
pub fn edge_url_from_env() -> String {
    env_url()
        .or_else(|| config_dir().and_then(|dir| edge_config_in(&dir)).map(|(url, _)| url))
        .unwrap_or_else(|| DEFAULT_EDGE_URL.into())
}

fn env_url() -> Option<String> {
    std::env::var("ZEND_EDGE_URL").ok().filter(|s| !s.trim().is_empty())
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

#[cfg(test)]
mod tests {
    /// A store that fails part way (here: the new file can't be made) leaves
    /// the pair kept before, whole: never the new edge with the old bearer.
    #[cfg(unix)]
    #[test]
    fn a_failed_store_never_leaves_a_mixed_pair() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("zend");
        super::store_edge_config(&config, "http://100.64.0.1:27640", "u-old@org").unwrap();
        // Whatever the store writes second can't be made.
        for name in ["edge", "edge-token"] {
            std::fs::create_dir_all(config.join(format!(".{name}.{}.new", std::process::id()))).unwrap();
        }
        assert!(super::store_edge_config(&config, "http://100.64.0.2:27641", "u-new@org").is_err());
        assert_eq!(
            super::edge_config_in(&config),
            Some(("http://100.64.0.1:27640".to_owned(), Some("u-old@org".to_owned())))
        );
    }

    #[cfg(unix)]
    #[test]
    fn the_edge_config_is_written_whole_and_private() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("zend");
        super::store_edge_config(&config, " http://100.64.0.1:27640 ", "u-phone@org\n").unwrap();
        super::store_edge_config(&config, "http://100.64.0.1:27640", "u-phone2@org").unwrap();
        let path = config.join("edge");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "zend-edge 1\nhttp://100.64.0.1:27640\nu-phone2@org\n");
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(super::edge_record_in(&config), Some(("http://100.64.0.1:27640".into(), "u-phone2@org".into())));
        // No new file left beside it.
        let names: Vec<_> = std::fs::read_dir(&config).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names, ["edge"]);
        // A bearer can't carry a line of its own into the record.
        assert!(super::store_edge_config(&config, "http://a", "u\nx").is_err());
    }

    /// An install that kept the older two files reads them while it has no
    /// record; once it has one, the record alone; restoring "none" goes back.
    #[test]
    fn the_older_two_files_are_read_until_there_is_a_record() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path();
        assert_eq!(super::edge_config_in(config), None);
        std::fs::write(config.join("edge-url"), "http://100.64.0.1:27640\n").unwrap();
        std::fs::write(config.join("edge-token"), "u-old@org\n").unwrap();
        assert_eq!(super::edge_config_in(config), Some(("http://100.64.0.1:27640".into(), Some("u-old@org".into()))));
        super::store_edge_config(config, "http://100.64.0.2:27641", "u-new@org").unwrap();
        assert_eq!(super::edge_config_in(config), Some(("http://100.64.0.2:27641".into(), Some("u-new@org".into()))));
        super::restore_edge_record(config, None).unwrap();
        assert_eq!(super::edge_config_in(config), Some(("http://100.64.0.1:27640".into(), Some("u-old@org".into()))));
        // A record that isn't one is nothing: not half of it, not the old files.
        std::fs::write(config.join("edge"), "zend-edge 1\nhttp://x\n").unwrap();
        assert_eq!(super::edge_config_in(config), None);
    }
}
