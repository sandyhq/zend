//! Run an agent CLI confined to one project folder, under bubblewrap.
//!
//! A chat another server started ([`zend_proto::ChatSandbox::Project`]) must
//! not reach this host's data beyond the project it was granted: its agent
//! could otherwise be asked to read `~/.ssh` or Zend's own data and send it
//! back. The engine hands such a run a [`Sandbox`] ([`crate::RunControls`]);
//! a driver that honors it ([`crate::Harness::runs_sandboxed`]) starts every
//! process of that run through a [`Launch`]:
//!
//! - the system (`/usr`, `/etc`, the `/bin` and `/lib*` links) read-only;
//! - an empty tmpfs as `$HOME` and `/tmp`, its own `/proc` and `/dev`;
//! - read-only in it: the agent's own install ([`TOOL_DIRS`], the
//!   executable and its folder) and its login files ([`logins`]), each
//!   checked so none shows the home, a hidden folder or Zend's data;
//! - the project root read-write, and the chat's own state folder
//!   ([`Sandbox::with_state`]);
//! - a new user, PID, IPC and UTS namespace, a new session (no terminal
//!   to inject into), killed with its parent (`--die-with-parent`);
//! - an environment cut down to an allowlist ([`final_env`]): no `ZEND_*`,
//!   no `SSH_*`, no session bus;
//! - a network namespace of its own, with the internet through `pasta`
//!   (passt): no host loopback, no abstract unix sockets, and no private,
//!   link-local, carrier-grade NAT (the tailnet) or host address or subnet
//!   ([`BLOCKED_NETWORKS`]); DNS through pasta's forwarder.
//!
//! A run on keys of its own ([`Sandbox::with_keys`], a chat another server
//! started) gets none of the owner's login files and, of every credential
//! in its environment, only the keys it was given ([`only_keys`]). It may
//! have an agent folder of its own in its state folder
//! ([`Sandbox::with_agent_dir`], Pi's `PI_CODING_AGENT_DIR`), and in it,
//! read-only, the one short-lived login it may use (plan step 4d-4,
//! [`Sandbox::with_agent_file`]): a host file kept out of its reach, removed
//! when its run is over.
//!
//! Its root is a project folder ([`check_root`]) or a working copy Zend
//! made for it in its own data ([`check_copy_root`]): a chat another server
//! started works there, never in the owner's folder.
//!
//! It never falls back to running unconfined: no bubblewrap or pasta, a
//! root outside the allowed folders ([`check_root`]), or a launch that can't
//! set up its namespaces (checked by starting it once, [`Launch::new`]) is
//! an error.
use std::ffi::{OsStr, OsString};
use std::path::{Component, Path, PathBuf};

use crate::HarnessError;
use crate::process::Command;
use zend_proto::HarnessId;

/// Folders below the home the run may never see, nor have as its root or
/// inside it: keys, Zend's data and settings, cloud and cluster logins.
pub const HIDDEN_HOME_DIRS: [&str; 9] = [
    ".ssh",
    ".gnupg",
    ".zend",
    ".config/zend",
    ".aws",
    ".kube",
    ".docker",
    ".config/gh",
    ".password-store",
];

/// Install folders below the home, bound read-only where they exist: where
/// agent CLIs and the tools they call (node, pnpm, cargo) are installed.
/// Programs, not data: none holds a login. Not `~/.local/bin` or `~/bin`:
/// those hold the owner's own scripts ([`SHARED_BIN_DIRS`]).
pub const TOOL_DIRS: [&str; 14] = [
    ".local/share/pnpm",
    ".local/share/claude",
    ".local/share/fnm",
    ".local/share/mise",
    ".local/share/vite-plus",
    ".nvm",
    ".npm-global",
    ".bun/bin",
    ".volta",
    ".cargo/bin",
    ".rustup",
    ".codex/packages",
    ".pi/agent/install",
    ".opencode/bin",
];

/// Folders below the home that hold the owner's scripts beside agent
/// links: never bound whole, only the agent's own file in them.
pub const SHARED_BIN_DIRS: [&str; 2] = [".local/bin", "bin"];

/// System paths, read-only. A path that is a symlink (`/bin` → `usr/bin`)
/// is made the same symlink inside.
const SYSTEM: [&str; 8] = ["/usr", "/etc", "/bin", "/sbin", "/lib", "/lib64", "/lib32", "/libx32"];

/// IPv4 ranges the sandbox can't reach, beyond the host's own addresses and
/// subnets: this network, private ranges, carrier-grade NAT (Tailscale's
/// tailnet), link-local (cloud metadata), IETF, benchmarking, multicast
/// and reserved. IPv6 is off (`pasta -4`).
pub const BLOCKED_NETWORKS: [&str; 10] = [
    "0.0.0.0/8",
    "10.0.0.0/8",
    "100.64.0.0/10",
    "169.254.0.0/16",
    "172.16.0.0/12",
    "192.0.0.0/24",
    "192.168.0.0/16",
    "198.18.0.0/15",
    "224.0.0.0/4",
    "240.0.0.0/4",
];

/// The sandbox's side of pasta's link: its address and its gateway, which
/// is also its DNS server (pasta forwards to the host's resolver). A /30
/// inside link-local, so nothing else of that range is on the link.
const GUEST_ADDRESS: &str = "169.254.2.2";
const GATEWAY: &str = "169.254.2.1";
const GUEST_NET: [u8; 3] = [169, 254, 2];

/// The host side of a sandbox: whose home it is, where bubblewrap and
/// pasta are, and what else must stay hidden (the engine's data dir).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Host {
    pub home: PathBuf,
    /// bubblewrap itself; `None` looks it up (`/usr/bin/bwrap`, then PATH).
    pub bwrap: Option<PathBuf>,
    /// pasta (passt), the sandbox's way out; `None` looks it up.
    pub pasta: Option<PathBuf>,
    /// Beyond [`HIDDEN_HOME_DIRS`]: folders the root may not be, hold or be in.
    pub hidden: Vec<PathBuf>,
}

impl Host {
    /// This process's home, bubblewrap and pasta looked up, nothing more hidden.
    pub fn current() -> Self {
        Self {
            home: crate::executable::home_or_current_dir(),
            bwrap: None,
            pasta: None,
            hidden: Vec::new(),
        }
    }
}

/// One run's confinement: its project root, checked against the host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sandbox {
    root: PathBuf,
    host: Host,
    /// The home, canonical.
    home: PathBuf,
    /// Every hidden folder, canonical where it exists.
    hidden: Vec<PathBuf>,
    bwrap: PathBuf,
    pasta: PathBuf,
    ip: PathBuf,
    state: Option<PathBuf>,
    /// The run's own keys ([`Self::with_keys`]): no owner login, only these.
    keys: Option<std::collections::BTreeMap<String, String>>,
    /// The agent's own config folder ([`Self::with_agent_dir`]).
    agent_dir: Option<PathBuf>,
    /// A host file it sees read-only in that folder
    /// ([`Self::with_agent_file`]): the file, and where it shows.
    agent_file: Option<(std::sync::Arc<LentFile>, PathBuf)>,
}

/// A file of the host's a sandbox sees read-only ([`Sandbox::with_agent_file`]):
/// kept where no sandbox can reach it (Zend's data), mounted read-only
/// into the run, and removed from the host when the last sandbox and launch
/// holding it are dropped (its run is over). Its `Debug` shows its path only.
#[derive(Debug, PartialEq, Eq)]
pub struct LentFile {
    path: PathBuf,
}

impl LentFile {
    /// Take charge of `path`, a file the caller made where no sandbox can
    /// write: it is removed when this is dropped.
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for LentFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// One normal path component: no separator, `.` or `..`.
fn plain_name(name: &str) -> bool {
    let mut parts = Path::new(name).components();
    matches!((parts.next(), parts.next()), (Some(std::path::Component::Normal(_)), None)) && !name.contains('/')
}

fn refuse(why: impl std::fmt::Display) -> HarnessError {
    HarnessError::Sandbox(format!(
        "This session must run sandboxed and was not started: {why}"
    ))
}

fn canonical_or(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn overlaps(a: &Path, b: &Path) -> bool {
    a.starts_with(b) || b.starts_with(a)
}

/// The hidden folders for `home` (canonical): [`HIDDEN_HOME_DIRS`] and
/// `extra`, canonical where they exist.
fn hidden_dirs(home: &Path, extra: &[PathBuf]) -> Vec<PathBuf> {
    HIDDEN_HOME_DIRS
        .iter()
        .map(|dir| canonical_or(&home.join(dir)))
        .chain(extra.iter().map(|dir| canonical_or(dir)))
        .collect()
}

/// Whether `rel` (a path below the home) passes through a hidden
/// (dot) folder.
fn has_dot_component(rel: &Path) -> bool {
    rel.components().any(|part| match part {
        Component::Normal(name) => name.to_string_lossy().starts_with('.'),
        _ => true,
    })
}

/// The project folder a sandbox may have as its root, canonical: a folder
/// inside the home, none of whose parts below the home is hidden (`~/.pi`,
/// `~/.local`, `~/Projects/.x`), that is not, holds or lies in a hidden
/// folder (`hidden`: Zend's data, beyond [`HIDDEN_HOME_DIRS`]). Anything else is
/// refused: the home itself, `/`, a folder outside the home.
pub fn check_root(root: &Path, home: &Path, hidden: &[PathBuf]) -> Result<PathBuf, HarnessError> {
    let canonical = std::fs::canonicalize(root)
        .map_err(|e| refuse(format!("its project folder {}: {e}", root.display())))?;
    if !canonical.is_dir() {
        return Err(refuse(format!("{} is not a folder", canonical.display())));
    }
    let home = canonical_or(home);
    let Ok(rel) = canonical.strip_prefix(&home) else {
        return Err(refuse(format!(
            "its project folder {} is not inside the home folder",
            canonical.display()
        )));
    };
    if rel.as_os_str().is_empty() {
        return Err(refuse(format!(
            "its project folder {} is the home folder",
            canonical.display()
        )));
    }
    if has_dot_component(rel) {
        return Err(refuse(format!(
            "its project folder {} lies in a hidden folder",
            canonical.display()
        )));
    }
    for dir in hidden_dirs(&home, hidden) {
        if overlaps(&canonical, &dir) {
            return Err(refuse(format!(
                "its project folder {} would show {}",
                canonical.display(),
                dir.display()
            )));
        }
    }
    Ok(canonical)
}

/// A working copy's root, canonical: a folder directly in `copies` (where
/// Zend keeps the working copies of chats other servers start, in its own
/// data: hidden, so [`check_root`] refuses it), its name not hidden, that
/// is not and holds neither the home nor a hidden folder. Only that folder
/// is bound: the rest of Zend's data stays out of sight.
pub fn check_copy_root(root: &Path, copies: &Path, home: &Path, hidden: &[PathBuf]) -> Result<PathBuf, HarnessError> {
    let canonical = std::fs::canonicalize(root)
        .map_err(|e| refuse(format!("its working copy {}: {e}", root.display())))?;
    if !canonical.is_dir() {
        return Err(refuse(format!("{} is not a folder", canonical.display())));
    }
    let copies = std::fs::canonicalize(copies)
        .map_err(|e| refuse(format!("the working copies' folder {}: {e}", copies.display())))?;
    let named = canonical.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if canonical.parent() != Some(copies.as_path()) || named.is_empty() || named.starts_with('.') {
        return Err(refuse(format!(
            "{} is not a working copy in {}",
            canonical.display(),
            copies.display()
        )));
    }
    let home = canonical_or(home);
    if home.starts_with(&canonical) {
        return Err(refuse(format!("its working copy {} would show the home folder", canonical.display())));
    }
    for dir in hidden_dirs(&home, hidden) {
        if dir.starts_with(&canonical) {
            return Err(refuse(format!(
                "its working copy {} would show {}",
                canonical.display(),
                dir.display()
            )));
        }
    }
    Ok(canonical)
}

/// bubblewrap, pasta and `ip`, or why the run can't be sandboxed.
fn tools_of(host: &Host) -> Result<(PathBuf, PathBuf, PathBuf), HarnessError> {
    if !cfg!(target_os = "linux") {
        return Err(refuse("sandboxed sessions need Linux with bubblewrap"));
    }
    let bwrap = tool(&host.bwrap, "bwrap", "bubblewrap (bwrap)")?;
    let pasta = tool(&host.pasta, "pasta", "pasta (passt), the sandbox's network,")?;
    let ip = tool(&None, "ip", "ip (iproute2)")?;
    Ok((bwrap, pasta, ip))
}

impl Sandbox {
    /// Confine to `root` (the chat's granted project folder). Refused, never
    /// relaxed: no bubblewrap, pasta or `ip`; a root [`check_root`] refuses.
    pub fn project(root: &Path, host: Host) -> Result<Self, HarnessError> {
        let tools = tools_of(&host)?;
        let root = check_root(root, &host.home, &host.hidden)?;
        Ok(Self::new(root, host, tools))
    }

    /// Confine to `root`, a working copy in `copies` ([`check_copy_root`]).
    /// Refused as [`Self::project`] is.
    pub fn working_copy(root: &Path, copies: &Path, host: Host) -> Result<Self, HarnessError> {
        let tools = tools_of(&host)?;
        let root = check_copy_root(root, copies, &host.home, &host.hidden)?;
        Ok(Self::new(root, host, tools))
    }

    fn new(root: PathBuf, host: Host, (bwrap, pasta, ip): (PathBuf, PathBuf, PathBuf)) -> Self {
        let home = canonical_or(&host.home);
        let hidden = hidden_dirs(&home, &host.hidden);
        Self {
            root,
            host,
            home,
            hidden,
            bwrap,
            pasta,
            ip,
            state: None,
            keys: None,
            agent_dir: None,
            agent_file: None,
        }
    }

    /// Run on these keys of its own (variable name → key, each one of
    /// [`PROVIDER_KEYS`]): none of the owner's login files is bound,
    /// whatever its driver asks for, and of the credentials in its
    /// environment only these reach it ([`only_keys`]). Empty: none at all.
    pub fn with_keys(mut self, keys: std::collections::BTreeMap<String, String>) -> Result<Self, HarnessError> {
        if let Some(name) = keys.keys().find(|name| !PROVIDER_KEYS.contains(&name.as_str())) {
            return Err(refuse(format!("{name} is not a provider's API key variable")));
        }
        self.keys = Some(keys);
        Ok(self)
    }

    /// The keys it runs on ([`Self::with_keys`]), if it runs on its own.
    pub fn own_keys(&self) -> Option<&std::collections::BTreeMap<String, String>> {
        self.keys.as_ref()
    }

    /// Also give the run `dir`, read-write at the same path: what the agent
    /// keeps of this chat between its processes (its sessions, to resume).
    /// Made if missing; refused if it is, holds or lies in the home's open
    /// part or the project (it belongs in Zend's data, outside both).
    pub fn with_state(mut self, dir: &Path) -> Result<Self, HarnessError> {
        std::fs::create_dir_all(dir)
            .map_err(|e| refuse(format!("its state folder {}: {e}", dir.display())))?;
        let dir = std::fs::canonicalize(dir)
            .map_err(|e| refuse(format!("its state folder {}: {e}", dir.display())))?;
        if self.home.starts_with(&dir) || overlaps(&dir, &self.root) {
            return Err(refuse(format!(
                "its state folder {} would show the home or the project",
                dir.display()
            )));
        }
        self.state = Some(dir);
        Ok(self)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn host(&self) -> &Host {
        &self.host
    }

    /// The chat's own folder ([`Self::with_state`]), if it has one.
    pub fn state(&self) -> Option<&Path> {
        self.state.as_deref()
    }

    /// Give the agent its own config folder (Pi: `PI_CODING_AGENT_DIR`), in
    /// place of the host's: `name` in its state folder ([`Self::with_state`],
    /// set first). Only for a run on its own keys ([`Self::with_keys`]),
    /// which gets none of the owner's login files.
    ///
    /// Nothing is done there from the host: the folder is the sandbox's to
    /// write (and to replace, with a link anywhere), so the host never acts
    /// by its path. Bubblewrap makes it inside the sandbox if missing.
    pub fn with_agent_dir(mut self, name: &str) -> Result<Self, HarnessError> {
        let Some(state) = self.state.clone() else {
            return Err(refuse("its agent folder needs a state folder"));
        };
        if self.keys.is_none() {
            return Err(refuse("an agent folder of its own is only for a run on its own keys"));
        }
        if !plain_name(name) {
            return Err(refuse(format!("its agent folder {name:?} is not a plain name")));
        }
        self.agent_dir = Some(state.join(name));
        Ok(self)
    }

    /// Show `file` (a host file in a folder no sandbox can reach) read-only
    /// as `name` in its agent folder ([`Self::with_agent_dir`], set first):
    /// bubblewrap binds it there inside the sandbox, which can read it but
    /// neither change, replace nor remove it, and refuses to start when the
    /// sandbox put a link on the way. The host file goes when `file` is
    /// dropped (with this sandbox and its launches: the run is over).
    pub fn with_agent_file(mut self, name: &str, file: LentFile) -> Result<Self, HarnessError> {
        let Some(dir) = self.agent_dir.clone() else {
            return Err(refuse("a file in its agent folder needs an agent folder"));
        };
        if !plain_name(name) {
            return Err(refuse(format!("its agent file {name:?} is not a plain name")));
        }
        let writable = [Some(&self.root), self.state.as_ref()];
        if writable.into_iter().flatten().any(|dir| file.path().starts_with(dir)) || !file.path().is_absolute() {
            return Err(refuse("its agent file must be kept outside its sandbox"));
        }
        self.agent_file = Some((std::sync::Arc::new(file), dir.join(name)));
        Ok(self)
    }

    /// The agent's own config folder ([`Self::with_agent_dir`]), if set.
    pub fn agent_dir(&self) -> Option<&Path> {
        self.agent_dir.as_deref()
    }

    /// Whether the host may read `path` for this run (an attachment it
    /// names): only a file inside the project, symlinks resolved.
    pub fn may_read(&self, path: &Path) -> bool {
        std::fs::canonicalize(path).is_ok_and(|path| path.starts_with(&self.root))
    }

    /// A folder to bind read-only, canonical, or why not: it may not be or
    /// hold the home, be a whole hidden folder of the home (`~/.pi`) unless
    /// it is one [`TOOL_DIRS`] lists, as it is (`~/.nvm`, not a link to
    /// `~/.pi`), or be, hold or lie in a hidden folder or the project.
    fn bindable(&self, dir: &Path) -> Result<PathBuf, HarnessError> {
        let canonical = std::fs::canonicalize(dir)
            .map_err(|e| refuse(format!("{}: {e}", dir.display())))?;
        let listed = TOOL_DIRS.iter().any(|tool| self.home.join(tool) == canonical);
        let whole_dot = !listed
            && canonical
                .strip_prefix(&self.home)
                .is_ok_and(|rel| rel.components().count() == 1 && has_dot_component(rel));
        if self.home.starts_with(&canonical) || whole_dot {
            return Err(refuse(format!(
                "{} would show the home folder",
                dir.display()
            )));
        }
        for hidden in self.hidden.iter().chain(std::iter::once(&self.root)) {
            if overlaps(&canonical, hidden) {
                return Err(refuse(format!(
                    "{} would show {}",
                    dir.display(),
                    hidden.display()
                )));
            }
        }
        Ok(canonical)
    }
}

/// Names in a project folder the owner's own tools run or load from,
/// unconfined, once the sandboxed run is over: a confined agent gets them
/// read-only ([`protected_in`]). Git's folder (hooks, `core.fsmonitor`),
/// the agents' project settings and extensions, editor tasks, direnv and
/// hook managers.
pub const PROTECTED: &[&str] = &[
    ".git", ".pi", ".claude", ".codex", ".opencode", ".cursor", ".vscode", ".idea", ".envrc",
    ".githooks", ".husky",
];

/// The [`PROTECTED`] entries that exist in `root` and in each folder
/// from it down to `cwd` (where a tool started in `cwd` looks), each
/// resolved and inside `root`; a `.git` file (a worktree's or a
/// submodule's) adds the git folder it names, when that is inside `root`
/// too. Nothing is created: a missing name stays missing (and writable).
pub fn protected_in(root: &Path, cwd: &Path) -> Vec<PathBuf> {
    let mut dirs = vec![root.to_path_buf()];
    if let Ok(rest) = cwd.strip_prefix(root) {
        let mut dir = root.to_path_buf();
        for part in rest.components() {
            dir.push(part);
            dirs.push(dir.clone());
        }
    }
    let mut out: Vec<PathBuf> = Vec::new();
    let add = |path: PathBuf, out: &mut Vec<PathBuf>| {
        if path.starts_with(root) && !out.contains(&path) {
            out.push(path);
        }
    };
    for dir in &dirs {
        for name in PROTECTED {
            let Ok(path) = std::fs::canonicalize(dir.join(name)) else { continue };
            if *name == ".git" && path.is_file() {
                let named = std::fs::read_to_string(&path).ok().and_then(|text| {
                    let line = text.lines().find_map(|l| l.strip_prefix("gitdir:"))?.trim().to_string();
                    let target = Path::new(&line);
                    let target = if target.is_absolute() { target.to_path_buf() } else { dir.join(target) };
                    std::fs::canonicalize(target).ok()
                });
                if let Some(git_dir) = named {
                    add(git_dir, &mut out);
                }
            }
            add(path, &mut out);
        }
    }
    out
}

fn tool(explicit: &Option<PathBuf>, name: &str, what: &str) -> Result<PathBuf, HarnessError> {
    match explicit {
        Some(path) if path.is_file() => Ok(path.clone()),
        Some(path) => Err(refuse(format!("{what} was not found at {}", path.display()))),
        None => find_tool(name).ok_or_else(|| refuse(format!("{what} is not installed on this device"))),
    }
}

fn find_tool(name: &str) -> Option<PathBuf> {
    ["/usr/bin", "/usr/sbin", "/usr/local/bin", "/bin", "/sbin"]
        .iter()
        .map(|dir| Path::new(dir).join(name))
        .find(|p| p.is_file())
        .or_else(|| {
            std::env::var_os("PATH").and_then(|path| {
                std::env::split_paths(&path)
                    .map(|dir| dir.join(name))
                    .find(|p| p.is_absolute() && p.is_file())
            })
        })
}

/// The login files `harness` reads to sign in, read-only in the sandbox
/// (those that exist). `env` is the environment it will run with, for the
/// config folders a user moved.
pub fn logins(harness: HarnessId, home: &Path, env: &dyn Fn(&str) -> Option<OsString>) -> Vec<PathBuf> {
    let dir = |var: &str, default: &str| -> PathBuf {
        env(var).map(PathBuf::from).unwrap_or_else(|| home.join(default))
    };
    match harness {
        // Pi: its auth (OAuth and API keys) and its providers and models.
        HarnessId::Pi => {
            let agent = dir("PI_CODING_AGENT_DIR", ".pi/agent");
            vec![agent.join("auth.json"), agent.join("models.json")]
        }
        // Claude Code on Linux keeps its OAuth login in .credentials.json.
        HarnessId::ClaudeCode => {
            let config = dir("CLAUDE_CONFIG_DIR", ".claude");
            vec![config.join(".credentials.json")]
        }
        // Codex: its ChatGPT or API-key login, and its provider config.
        HarnessId::Codex => {
            let codex = dir("CODEX_HOME", ".codex");
            vec![codex.join("auth.json"), codex.join("config.toml")]
        }
        // OpenCode: its auth in the XDG data folder, its config in XDG config.
        HarnessId::Opencode => {
            let data = dir("XDG_DATA_HOME", ".local/share").join("opencode");
            let config = dir("XDG_CONFIG_HOME", ".config").join("opencode");
            vec![
                data.join("auth.json"),
                config.join("opencode.json"),
                config.join("opencode.jsonc"),
                config.join("config.json"),
            ]
        }
        _ => Vec::new(),
    }
}

/// Variables any agent keeps: how to find programs, speak, show text and
/// reach the network (certificates, proxies).
const KEPT: [&str; 31] = [
    "PATH",
    "LANG",
    "LANGUAGE",
    "TERM",
    "COLORTERM",
    "NO_COLOR",
    "FORCE_COLOR",
    "TZ",
    "USER",
    "LOGNAME",
    "SHELL",
    "HOME",
    "TMPDIR",
    "XDG_CONFIG_HOME",
    "XDG_DATA_HOME",
    "XDG_CACHE_HOME",
    "XDG_STATE_HOME",
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
    "NODE_EXTRA_CA_CERTS",
    "REQUESTS_CA_BUNDLE",
    "CURL_CA_BUNDLE",
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "NO_PROXY",
    "ALL_PROXY",
    "http_proxy",
    "https_proxy",
    "no_proxy",
    "all_proxy",
    "CLAUDE_CODE_ENTRYPOINT",
];

/// Never kept, whatever else allows them: Zend's tokens and paths, the
/// SSH agent and every other socket to the host's session.
const DENIED_PREFIXES: [&str; 3] = ["ZEND_", "SSH_", "GPG_"];
const DENIED: [&str; 7] = [
    "DBUS_SESSION_BUS_ADDRESS",
    "XDG_RUNTIME_DIR",
    "DISPLAY",
    "WAYLAND_DISPLAY",
    "DOCKER_HOST",
    "KUBECONFIG",
    "GNUPGHOME",
];

/// The model providers' API keys Pi reads from its environment (pi-ai's
/// `env-api-keys`, Pi 1.5.2), for Pi and OpenCode. Not a cloud account
/// (`AWS_*`, `GOOGLE_APPLICATION_CREDENTIALS`), a GitHub or Hugging Face
/// token, nor a variable that names a file.
pub const PROVIDER_KEYS: [&str; 38] = [
    "AI_GATEWAY_API_KEY",
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "ANTHROPIC_OAUTH_TOKEN",
    "ANT_LING_API_KEY",
    "AZURE_OPENAI_API_KEY",
    "BASETEN_API_KEY",
    "CEREBRAS_API_KEY",
    "CLOUDFLARE_API_KEY",
    "DEEPSEEK_API_KEY",
    "FIREWORKS_API_KEY",
    "GEMINI_API_KEY",
    "GOOGLE_CLOUD_API_KEY",
    "GROQ_API_KEY",
    "KIMI_API_KEY",
    "META_API_KEY",
    "MINIMAX_API_KEY",
    "MINIMAX_CN_API_KEY",
    "MISTRAL_API_KEY",
    "MOONSHOT_API_KEY",
    "NVIDIA_API_KEY",
    "OPENAI_API_KEY",
    "OPENCODE_API_KEY",
    "OPENROUTER_API_KEY",
    "QWEN_TOKEN_PLAN_API_KEY",
    "QWEN_TOKEN_PLAN_CN_API_KEY",
    "RADIUS_API_KEY",
    "TOGETHER_API_KEY",
    "TYPESAFE_API_KEY",
    "XAI_API_KEY",
    "XIAOMI_API_KEY",
    "XIAOMI_TOKEN_PLAN_AMS_API_KEY",
    "XIAOMI_TOKEN_PLAN_CN_API_KEY",
    "XIAOMI_TOKEN_PLAN_SGP_API_KEY",
    "ZAI_API_KEY",
    "ZAI_CODING_CN_API_KEY",
    "ANTHROPIC_ORGANIZATION_ID",
    "ANTHROPIC_WORKSPACE_ID",
];

/// The variable a model provider's API key is read from (pi-ai's
/// `env-api-keys`, Pi 1.5.2), for the providers [`PROVIDER_KEYS`] lists;
/// `None` for one that signs in only with a login (`openai-codex`,
/// `github-copilot`) or isn't known.
pub fn provider_key_var(provider: &str) -> Option<&'static str> {
    Some(match provider {
        "anthropic" => "ANTHROPIC_API_KEY",
        "openai" => "OPENAI_API_KEY",
        "azure" => "AZURE_OPENAI_API_KEY",
        "google" => "GEMINI_API_KEY",
        "google-vertex" => "GOOGLE_CLOUD_API_KEY",
        "ant-ling" => "ANT_LING_API_KEY",
        "qwen-token-plan" | "qwen-token-plan-individual" => "QWEN_TOKEN_PLAN_API_KEY",
        "qwen-token-plan-cn" | "qwen-token-plan-individual-cn" => "QWEN_TOKEN_PLAN_CN_API_KEY",
        "nvidia" => "NVIDIA_API_KEY",
        "deepseek" => "DEEPSEEK_API_KEY",
        "groq" => "GROQ_API_KEY",
        "cerebras" => "CEREBRAS_API_KEY",
        "xai" => "XAI_API_KEY",
        "typesafe" => "TYPESAFE_API_KEY",
        "radius" => "RADIUS_API_KEY",
        "openrouter" => "OPENROUTER_API_KEY",
        "vercel-ai-gateway" => "AI_GATEWAY_API_KEY",
        "zai" => "ZAI_API_KEY",
        "zai-coding-cn" => "ZAI_CODING_CN_API_KEY",
        "mistral" => "MISTRAL_API_KEY",
        "minimax" => "MINIMAX_API_KEY",
        "minimax-cn" => "MINIMAX_CN_API_KEY",
        "moonshotai" | "moonshotai-cn" => "MOONSHOT_API_KEY",
        "fireworks" => "FIREWORKS_API_KEY",
        "together" => "TOGETHER_API_KEY",
        "baseten" => "BASETEN_API_KEY",
        "opencode" | "opencode-go" => "OPENCODE_API_KEY",
        "kimi-coding" => "KIMI_API_KEY",
        "meta" => "META_API_KEY",
        "cloudflare-workers-ai" | "cloudflare-ai-gateway" => "CLOUDFLARE_API_KEY",
        "xiaomi" => "XIAOMI_API_KEY",
        "xiaomi-token-plan-cn" => "XIAOMI_TOKEN_PLAN_CN_API_KEY",
        "xiaomi-token-plan-ams" => "XIAOMI_TOKEN_PLAN_AMS_API_KEY",
        "xiaomi-token-plan-sgp" => "XIAOMI_TOKEN_PLAN_SGP_API_KEY",
        _ => return None,
    })
}

/// Whether `key` names a credential: a provider key ([`PROVIDER_KEYS`]) or
/// any variable whose name says key, token, secret, password, credential
/// or auth.
fn is_credential(key: &str) -> bool {
    let upper = key.to_ascii_uppercase();
    PROVIDER_KEYS.contains(&key)
        || ["KEY", "TOKEN", "SECRET", "PASSWORD", "CREDENTIAL", "AUTH"]
            .iter()
            .any(|word| upper.contains(word))
}

/// `env` for a run on its own keys ([`Sandbox::with_keys`]): every
/// credential taken out ([`is_credential`]), whoever set it, then `keys`
/// put in.
pub fn only_keys(
    mut env: std::collections::BTreeMap<OsString, OsString>,
    keys: &std::collections::BTreeMap<String, String>,
) -> std::collections::BTreeMap<OsString, OsString> {
    env.retain(|key, _| key.to_str().is_some_and(|key| !is_credential(key)));
    for (key, value) in keys {
        env.insert(key.into(), value.into());
    }
    env
}

/// The agent's own variables: its settings and, for the agents that sign
/// in with API keys, those keys.
fn harness_prefixes(harness: HarnessId) -> &'static [&'static str] {
    match harness {
        HarnessId::Pi => &["PI_"],
        HarnessId::ClaudeCode => &["CLAUDE_", "ANTHROPIC_"],
        HarnessId::Codex => &["CODEX_", "OPENAI_"],
        HarnessId::Opencode => &["OPENCODE_"],
        _ => &[],
    }
}

fn reads_provider_keys(harness: HarnessId) -> bool {
    matches!(harness, HarnessId::Pi | HarnessId::Opencode)
}

/// Whether a sandboxed `harness` may get `key`.
pub fn allowed_env(harness: HarnessId, key: &str) -> bool {
    if DENIED.contains(&key) || DENIED_PREFIXES.iter().any(|p| key.starts_with(p)) {
        return false;
    }
    KEPT.contains(&key)
        || key.starts_with("LC_")
        || harness_prefixes(harness).iter().any(|p| key.starts_with(p))
        || (reads_provider_keys(harness) && PROVIDER_KEYS.contains(&key))
}

/// The whole environment a sandboxed `harness` starts with: what it would
/// inherit and what its driver set (`None`: removed), each only where
/// [`allowed_env`] lets it, with `HOME` and `TMPDIR` the sandbox's own.
pub fn final_env(
    harness: HarnessId,
    home: &Path,
    inherited: impl IntoIterator<Item = (OsString, OsString)>,
    explicit: impl IntoIterator<Item = (OsString, Option<OsString>)>,
) -> std::collections::BTreeMap<OsString, OsString> {
    let allowed = |key: &OsStr| key.to_str().is_some_and(|key| allowed_env(harness, key));
    let mut env: std::collections::BTreeMap<OsString, OsString> = inherited
        .into_iter()
        .filter(|(key, _)| allowed(key))
        .collect();
    for (key, value) in explicit {
        match value {
            Some(value) if allowed(&key) => {
                env.insert(key, value);
            }
            _ => {
                env.remove(&key);
            }
        }
    }
    env.insert("HOME".into(), home.into());
    env.insert("TMPDIR".into(), "/tmp".into());
    env
}

/// The host's own IPv4 addresses (each as a /32) and the subnets they are
/// in, loopback left out: what the sandbox may not reach beside
/// [`BLOCKED_NETWORKS`] (a service bound to the host's public address, its
/// LAN neighbours).
#[cfg(unix)]
pub fn host_networks() -> Vec<String> {
    let mut out = Vec::new();
    let mut list: *mut libc::ifaddrs = std::ptr::null_mut();
    // SAFETY: getifaddrs fills `list`, freed below with freeifaddrs; each
    // node and its addresses stay valid until then.
    unsafe {
        if libc::getifaddrs(&mut list) != 0 {
            return out;
        }
        let mut node = list;
        while !node.is_null() {
            let addr = (*node).ifa_addr;
            let mask = (*node).ifa_netmask;
            if !addr.is_null() && i32::from((*addr).sa_family) == libc::AF_INET {
                let ip = u32::from_be((*(addr as *const libc::sockaddr_in)).sin_addr.s_addr);
                let bits = if mask.is_null() {
                    32
                } else {
                    u32::from_be((*(mask as *const libc::sockaddr_in)).sin_addr.s_addr).count_ones()
                };
                if let Some(nets) = networks_of(ip, bits) {
                    out.extend(nets);
                }
            }
            node = (*node).ifa_next;
        }
        libc::freeifaddrs(list);
    }
    out
}

#[cfg(not(unix))]
pub fn host_networks() -> Vec<String> {
    Vec::new()
}

/// `ip`/`bits` as routes to block: the address and its subnet. None for
/// loopback and for the sandbox's own link.
fn networks_of(ip: u32, bits: u32) -> Option<[String; 2]> {
    let octets = ip.to_be_bytes();
    if octets[0] == 127 || octets[..3] == GUEST_NET || bits == 0 {
        return None;
    }
    let bits = bits.min(32);
    let mask = if bits == 0 { 0 } else { u32::MAX << (32 - bits) };
    let net = std::net::Ipv4Addr::from(ip & mask);
    Some([
        format!("{}/32", std::net::Ipv4Addr::from(ip)),
        format!("{net}/{bits}"),
    ])
}

/// One process start under a [`Sandbox`]: pasta, then the routes it may
/// not take, then bubblewrap with the mounts for the agent `harness` is, then
/// the agent; and its environment rule.
#[derive(Clone, Debug)]
pub struct Launch {
    harness: HarnessId,
    pasta: PathBuf,
    /// pasta's arguments through bubblewrap's, up to the agent program.
    args: Vec<OsString>,
    /// bubblewrap's own arguments (for tests and logs).
    bwrap_args: Vec<OsString>,
    home: PathBuf,
    /// Its own keys ([`Sandbox::with_keys`]): only these credentials.
    keys: Option<std::collections::BTreeMap<String, String>>,
    /// Its agent folder ([`Sandbox::with_agent_dir`]): its PI_CODING_AGENT_DIR.
    agent_dir: Option<PathBuf>,
    /// The file it sees read-only there, kept until its launches are gone.
    _agent_file: Option<std::sync::Arc<LentFile>>,
}

impl Launch {
    /// The launch of `exe` (the agent CLI) for `harness`, in `cwd` (inside
    /// the sandbox's root), with `logins` (from [`logins`]) read-only, none
    /// for a sandbox on its own keys ([`Sandbox::with_keys`]).
    /// Starts it once with the same mounts and network (running `true`), so a
    /// device where it can't make its namespaces fails here, plainly,
    /// rather than as an agent that exits at once.
    pub fn new(
        sandbox: &Sandbox,
        harness: HarnessId,
        exe: &Path,
        cwd: &Path,
        logins: &[PathBuf],
    ) -> Result<Self, HarnessError> {
        let cwd = std::fs::canonicalize(cwd)
            .map_err(|e| refuse(format!("its folder {}: {e}", cwd.display())))?;
        if !cwd.starts_with(&sandbox.root) {
            return Err(refuse(format!(
                "its folder {} is outside its project {}",
                cwd.display(),
                sandbox.root.display()
            )));
        }
        let home = sandbox.host.home.clone();
        let (uid, gid) = ids();
        let mut args: Vec<OsString> = [
            "--unshare-user",
            "--uid",
            &uid,
            "--gid",
            &gid,
            "--disable-userns",
            "--die-with-parent",
            "--new-session",
            "--unshare-pid",
            "--unshare-ipc",
            "--unshare-uts",
            "--unshare-cgroup-try",
        ]
        .iter()
        .map(OsString::from)
        .collect();
        let mut push = |parts: &[&OsStr]| args.extend(parts.iter().map(|p| p.to_os_string()));
        for path in SYSTEM {
            let path = Path::new(path);
            match std::fs::symlink_metadata(path) {
                Ok(meta) if meta.file_type().is_symlink() => {
                    if let Ok(target) = std::fs::read_link(path) {
                        push(&["--symlink".as_ref(), target.as_os_str(), path.as_os_str()]);
                    }
                }
                Ok(_) => push(&["--ro-bind".as_ref(), path.as_os_str(), path.as_os_str()]),
                Err(_) => {}
            }
        }
        push(&["--proc".as_ref(), "/proc".as_ref()]);
        push(&["--dev".as_ref(), "/dev".as_ref()]);
        push(&["--tmpfs".as_ref(), "/tmp".as_ref()]);
        push(&["--tmpfs".as_ref(), home.as_os_str()]);
        // DNS: pasta's forwarder, written by the prologue to fd 9. Where
        // /etc/resolv.conf is a link (into /run, systemd-resolved), at its
        // target, which /etc's link then finds.
        let resolv = std::fs::canonicalize("/etc/resolv.conf")
            .ok()
            .filter(|path| !path.starts_with("/etc") && !path.starts_with("/usr"))
            .unwrap_or_else(|| PathBuf::from("/etc/resolv.conf"));
        push(&["--ro-bind-data".as_ref(), "9".as_ref(), resolv.as_os_str()]);
        // Read-only binds, each checked (Sandbox::bindable): source, target.
        let mut binds: Vec<(PathBuf, PathBuf)> = Vec::new();
        for dir in TOOL_DIRS.iter().map(|dir| home.join(dir)).filter(|dir| dir.is_dir()) {
            let canonical = sandbox.bindable(&dir)?;
            binds.push((canonical.clone(), dir.clone()));
            if canonical != dir {
                binds.push((canonical.clone(), canonical));
            }
        }
        let in_system = |path: &Path| SYSTEM.iter().any(|dir| path.starts_with(dir));
        let covered = |binds: &[(PathBuf, PathBuf)], path: &Path| {
            binds.iter().any(|(_, target)| path.starts_with(target))
        };
        if let Ok(real) = std::fs::canonicalize(exe) {
            // The agent's real file, and its folder unless that is one the
            // owner keeps scripts in (or the home): then only the file.
            if !in_system(&real) && !covered(&binds, &real) {
                let dir = real.parent().map(Path::to_path_buf).unwrap_or_default();
                let shared = dir == sandbox.home
                    || SHARED_BIN_DIRS.iter().any(|d| sandbox.home.join(d) == dir);
                if shared {
                    let file = sandbox.bindable(&real)?;
                    binds.push((file.clone(), file));
                } else {
                    let dir = sandbox.bindable(&dir)?;
                    binds.push((dir.clone(), dir));
                }
            }
            // The path it is started by (a link in ~/.local/bin): that file.
            if exe.is_absolute() && exe != real && !in_system(exe) && !covered(&binds, exe) {
                binds.push((real.clone(), exe.to_path_buf()));
            }
        }
        for (source, target) in &binds {
            push(&["--ro-bind".as_ref(), source.as_os_str(), target.as_os_str()]);
        }
        let logins = if sandbox.keys.is_some() { &[][..] } else { logins };
        for file in logins {
            if let Ok(real) = std::fs::canonicalize(file) {
                if let Some(hidden) = sandbox.hidden.iter().find(|dir| real.starts_with(dir)) {
                    return Err(refuse(format!(
                        "its login {} would show {}",
                        file.display(),
                        hidden.display()
                    )));
                }
                push(&["--ro-bind".as_ref(), real.as_os_str(), file.as_os_str()]);
            }
        }
        let root = sandbox.root.as_os_str();
        push(&["--bind".as_ref(), root, root]);
        // What the owner's own tools run or load from this folder later,
        // unconfined (git hooks and config, agent extensions): read-only.
        for path in protected_in(&sandbox.root, &cwd) {
            push(&["--ro-bind".as_ref(), path.as_os_str(), path.as_os_str()]);
        }
        if let Some(state) = &sandbox.state {
            push(&["--bind".as_ref(), state.as_os_str(), state.as_os_str()]);
        }
        // After the state folder it lies in: bubblewrap resolves the target
        // inside the new root and refuses a link on its way.
        if let Some((file, target)) = &sandbox.agent_file {
            push(&["--ro-bind".as_ref(), file.path().as_os_str(), target.as_os_str()]);
        }
        push(&["--chdir".as_ref(), cwd.as_os_str()]);
        push(&["--".as_ref()]);
        let bwrap_args = args;

        let mut blocked: Vec<String> = BLOCKED_NETWORKS.iter().map(|s| s.to_string()).collect();
        blocked.extend(host_networks());
        let mut seen = std::collections::BTreeSet::new();
        blocked.retain(|net| seen.insert(net.clone()));
        let routes = blocked
            .iter()
            .map(|net| format!("'{}' -4 route add unreachable {net}", sandbox.ip.display()))
            .collect::<Vec<_>>()
            .join(" && ");
        let prologue = format!(
            "{{ {routes}; }} || {{ echo 'the sandbox network could not be set up' >&2; exit 125; }}\n\
             exec \"$@\" 9<<'ZEND_RESOLV'\nnameserver {GATEWAY}\nZEND_RESOLV\n"
        );
        let mut args: Vec<OsString> = [
            "--config-net",
            "--foreground",
            "--quiet",
            "--ipv4-only",
            "-t",
            "none",
            "-u",
            "none",
            "-T",
            "none",
            "-U",
            "none",
            "--no-map-gw",
            "--address",
            GUEST_ADDRESS,
            "--netmask",
            "30",
            "--gateway",
            GATEWAY,
            "--dns-forward",
            GATEWAY,
            "--",
            "/bin/sh",
            "-c",
        ]
        .iter()
        .map(OsString::from)
        .collect();
        args.push(prologue.into());
        args.push("sh".into());
        args.push(sandbox.bwrap.clone().into_os_string());
        args.extend(bwrap_args.iter().cloned());
        let launch = Self {
            harness,
            pasta: sandbox.pasta.clone(),
            args,
            bwrap_args,
            home,
            keys: sandbox.keys.clone(),
            agent_dir: sandbox.agent_dir.clone(),
            _agent_file: sandbox.agent_file.as_ref().map(|(file, _)| file.clone()),
        };
        launch.preflight()?;
        Ok(launch)
    }

    fn preflight(&self) -> Result<(), HarnessError> {
        let output = std::process::Command::new(&self.pasta)
            .args(&self.args)
            .arg("true")
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .stdin(std::process::Stdio::null())
            .output()
            .map_err(|e| refuse(format!("pasta ({}) could not start: {e}", self.pasta.display())))?;
        if output.status.success() {
            return Ok(());
        }
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(refuse(format!(
            "the sandbox failed to start ({}): {}",
            output.status,
            stderr.trim()
        )))
    }

    /// A command that runs `program` inside the sandbox: pasta, bubblewrap
    /// with this launch's mounts, then `program`. Arguments added to it go to
    /// `program`. Start it with [`Launch::spawn`], which cuts its environment
    /// down.
    pub fn command(&self, program: impl AsRef<OsStr>) -> Command {
        let mut command = Command::new(&self.pasta);
        command.args(&self.args).arg(program);
        command
    }

    /// The arguments bubblewrap gets, up to and with `--` (for tests and logs).
    pub fn bwrap_args(&self) -> &[OsString] {
        &self.bwrap_args
    }

    /// Cut `command`'s environment down to [`final_env`]'s: what it would
    /// have inherited and what was set on it, allowed ones only.
    #[cfg(unix)]
    pub fn scrub_env(&self, command: &mut Command) {
        let explicit: Vec<(OsString, Option<OsString>)> = command
            .as_std()
            .get_envs()
            .map(|(k, v)| (k.to_os_string(), v.map(OsStr::to_os_string)))
            .collect();
        let mut env = final_env(self.harness, &self.home, std::env::vars_os(), explicit);
        if let Some(keys) = &self.keys {
            env = only_keys(env, keys);
        }
        // Its own agent folder (Pi reads PI_CODING_AGENT_DIR), whatever the
        // driver or the engine's environment named (the host's).
        if let Some(dir) = &self.agent_dir {
            env.insert("PI_CODING_AGENT_DIR".into(), dir.into());
        }
        let std = command.as_std_mut();
        std.env_clear();
        std.envs(env);
    }

    /// Never reached: [`Sandbox::project`] refuses off Linux.
    #[cfg(not(unix))]
    pub fn scrub_env(&self, _command: &mut Command) {
        unreachable!("sandboxes exist only on Linux")
    }

    /// Start `command` (made by [`Launch::command`]) as an agent
    /// ([`crate::process::spawn_agent`]), its environment scrubbed.
    pub fn spawn(&self, command: &mut Command) -> std::io::Result<crate::process::Child> {
        self.scrub_env(command);
        crate::process::spawn_agent(command)
    }
}

/// This process's user and group ids, which the agent keeps inside.
#[cfg(unix)]
fn ids() -> (String, String) {
    // SAFETY: getuid and getgid can't fail.
    unsafe { (libc::getuid().to_string(), libc::getgid().to_string()) }
}

#[cfg(not(unix))]
fn ids() -> (String, String) {
    ("0".into(), "0".into())
}

/// [`Launch::command`] when sandboxed, else `Command::new(program)` as
/// every driver made it before.
pub fn command(launch: Option<&Launch>, program: impl AsRef<OsStr>) -> Command {
    match launch {
        Some(launch) => launch.command(program),
        None => Command::new(program),
    }
}

/// [`Launch::spawn`] when sandboxed, else [`crate::process::spawn_agent`].
pub fn spawn(launch: Option<&Launch>, command: &mut Command) -> std::io::Result<crate::process::Child> {
    match launch {
        Some(launch) => launch.spawn(command),
        None => crate::process::spawn_agent(command),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_environment_keeps_only_what_the_agent_needs() {
        let inherited = [
            ("PATH", "/usr/bin"),
            ("SSH_AUTH_SOCK", "/run/user/1000/ssh"),
            ("ZEND_IPC_TOKEN", "secret"),
            ("DBUS_SESSION_BUS_ADDRESS", "unix:path=/run/user/1000/bus"),
            ("GH_TOKEN", "gh"),
            ("ANTHROPIC_API_KEY", "a"),
            ("OPENAI_API_KEY", "o"),
            ("PI_CODING_AGENT_DIR", "/home/u/.pi/agent"),
            ("LC_ALL", "C"),
            ("HOME", "/elsewhere"),
        ]
        .map(|(k, v)| (OsString::from(k), OsString::from(v)));
        let explicit = [
            ("ZEND_ORIGIN", Some("chat:x")),
            ("SSH_AGENT_PID", Some("1")),
            ("CODEX_HOME", Some("/c")),
            ("LC_ALL", None),
        ]
        .map(|(k, v)| (OsString::from(k), v.map(OsString::from)));
        let env = final_env(HarnessId::Pi, Path::new("/home/u"), inherited, explicit);
        let keys: Vec<&str> = env.keys().map(|k| k.to_str().unwrap()).collect();
        assert_eq!(
            keys,
            ["ANTHROPIC_API_KEY", "HOME", "OPENAI_API_KEY", "PATH", "PI_CODING_AGENT_DIR", "TMPDIR"]
        );
        assert_eq!(env[OsStr::new("HOME")], "/home/u");
        // Claude gets its own keys, not Pi's or OpenAI's.
        assert!(allowed_env(HarnessId::ClaudeCode, "ANTHROPIC_API_KEY"));
        assert!(!allowed_env(HarnessId::ClaudeCode, "OPENAI_API_KEY"));
        assert!(!allowed_env(HarnessId::Mock, "ANTHROPIC_API_KEY"));
        assert!(!allowed_env(HarnessId::ClaudeCode, "ZEND_EDGE_TOKEN"));
        // Only the providers' keys Pi reads: no cloud account, no other
        // service's key that merely ends like one.
        for key in ["GEMINI_API_KEY", "OPENROUTER_API_KEY", "XAI_API_KEY"] {
            assert!(allowed_env(HarnessId::Pi, key), "{key}");
        }
        for key in [
            "STRIPE_API_KEY",
            "AWS_SECRET_ACCESS_KEY",
            "AWS_BEARER_TOKEN_BEDROCK",
            "GOOGLE_APPLICATION_CREDENTIALS",
            "COPILOT_GITHUB_TOKEN",
            "HF_TOKEN",
            "OPENAI_API_KEY_FILE",
        ] {
            assert!(!allowed_env(HarnessId::Pi, key), "{key}");
        }
    }

    #[test]
    fn the_hosts_addresses_and_subnets_are_blocked_but_not_loopback() {
        let ip = |s: &str| u32::from(s.parse::<std::net::Ipv4Addr>().unwrap());
        assert_eq!(
            networks_of(ip("152.53.94.85"), 22).unwrap(),
            ["152.53.94.85/32".to_string(), "152.53.92.0/22".to_string()]
        );
        assert!(networks_of(ip("127.0.0.1"), 8).is_none());
        assert!(networks_of(ip("169.254.2.2"), 30).is_none());
        assert!(networks_of(ip("10.1.2.3"), 0).is_none());
    }

    #[test]
    fn a_root_that_would_show_the_home_or_zend_data_is_refused() {
        let home = tempfile::tempdir().unwrap();
        let data = home.path().join("data");
        for dir in [".ssh", ".zend/scratch/x", ".config/zend", "projects/a", "data"] {
            std::fs::create_dir_all(home.path().join(dir)).unwrap();
        }
        for dir in [".pi/agent", ".local/share/x", "projects/.hidden/b", "projects/link-target"] {
            std::fs::create_dir_all(home.path().join(dir)).unwrap();
        }
        let outside = tempfile::tempdir().unwrap();
        // A link inside the home to a hidden folder: judged by its target.
        std::os::unix::fs::symlink(home.path().join(".ssh"), home.path().join("projects/keys"))
            .unwrap();
        // Paths that exist: these refusals come before bwrap or pasta run.
        let host = Host {
            home: home.path().into(),
            bwrap: Some(PathBuf::from("/bin/sh")),
            pasta: Some(PathBuf::from("/bin/sh")),
            hidden: vec![data.clone()],
        };
        let refused = |root: &Path| Sandbox::project(root, host.clone()).unwrap_err().to_string();
        assert!(refused(home.path()).contains("is the home folder"));
        assert!(refused(Path::new("/")).contains("not inside the home folder"));
        assert!(refused(outside.path()).contains("not inside the home folder"));
        for hidden in [".ssh", ".zend/scratch/x", ".config", ".pi/agent", ".local/share/x", "projects/.hidden/b"] {
            let error = refused(&home.path().join(hidden));
            assert!(error.contains("hidden folder"), "{hidden}: {error}");
        }
        assert!(refused(&home.path().join("projects/keys")).contains("hidden folder"));
        assert!(refused(&data).contains("data"));
        assert!(refused(&home.path().join("missing")).contains("missing"));
        assert!(Sandbox::project(&home.path().join("projects/a"), host.clone()).is_ok());
        let no_pasta = Host {
            pasta: Some(home.path().join("no-pasta")),
            ..host.clone()
        };
        let error = Sandbox::project(&home.path().join("projects/a"), no_pasta)
            .unwrap_err()
            .to_string();
        assert!(error.contains("pasta (passt)"), "{error}");
        let no_bwrap = Host {
            bwrap: Some(home.path().join("no-bwrap")),
            ..host
        };
        let error = Sandbox::project(&home.path().join("projects/a"), no_bwrap)
            .unwrap_err()
            .to_string();
        assert!(error.contains("bubblewrap (bwrap) was not found"), "{error}");
    }

    /// A working copy is a folder directly in the copies' folder (in Zend's
    /// data, hidden), and nothing else there or around it.
    #[test]
    fn only_a_folder_directly_in_the_copies_folder_is_a_working_copy() {
        let home = tempfile::tempdir().unwrap();
        let home = std::fs::canonicalize(home.path()).unwrap();
        let data = home.join(".zend");
        let copies = data.join("peer-copies");
        for dir in ["c", "c/sub", ".dot", "c.meta"] {
            std::fs::create_dir_all(copies.join(dir)).unwrap();
        }
        std::fs::create_dir_all(home.join("projects/a")).unwrap();
        let hidden = vec![data.clone()];
        let check = |root: &Path| check_copy_root(root, &copies, &home, &hidden);
        assert_eq!(check(&copies.join("c")).unwrap(), copies.join("c"));
        for bad in [copies.clone(), copies.join("c/sub"), copies.join(".dot"), data.clone(), home.clone(), home.join("projects/a")] {
            assert!(check(&bad).is_err(), "{}", bad.display());
        }
        // A link in the copies' folder to the owner's project: judged by its target.
        std::os::unix::fs::symlink(home.join("projects/a"), copies.join("link")).unwrap();
        assert!(check(&copies.join("link")).is_err());
        // The project check still refuses it.
        assert!(check_root(&copies.join("c"), &home, &hidden).is_err());
    }

    /// On its own keys: every credential out, whoever set it; its keys in.
    #[test]
    fn a_run_on_its_own_keys_gets_only_those() {
        let env: std::collections::BTreeMap<OsString, OsString> = [
            ("PATH", "/usr/bin"),
            ("HOME", "/h"),
            ("ANTHROPIC_API_KEY", "owner"),
            ("OPENAI_API_KEY", "owner"),
            ("PI_AUTH_TOKEN", "owner"),
            ("PI_CODING_AGENT_DIR", "/h/.pi/agent"),
            ("ANTHROPIC_ORGANIZATION_ID", "org"),
        ]
        .into_iter()
        .map(|(k, v)| (k.into(), v.into()))
        .collect();
        let keys = [("ANTHROPIC_API_KEY".to_string(), "theirs".to_string())].into_iter().collect();
        let out = only_keys(env, &keys);
        let names: Vec<_> = out.keys().map(|k| k.to_string_lossy().into_owned()).collect();
        assert_eq!(names, ["ANTHROPIC_API_KEY", "HOME", "PATH", "PI_CODING_AGENT_DIR"]);
        assert_eq!(out[OsStr::new("ANTHROPIC_API_KEY")], "theirs");
        assert_eq!(provider_key_var("anthropic"), Some("ANTHROPIC_API_KEY"));
        assert_eq!(provider_key_var("openai-codex"), None);
        for provider in ["openai", "google", "openrouter", "xai", "zai", "mistral", "moonshotai"] {
            let var = provider_key_var(provider).unwrap();
            assert!(PROVIDER_KEYS.contains(&var), "{provider}: {var}");
        }
    }

    #[test]
    fn protected_names_are_found_down_to_the_cwd_and_none_are_made() {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        let outside = tempfile::tempdir().unwrap();
        for d in [".git/worktrees/w", ".pi", "sub/deeper", "sub/.claude", "other/.vscode"] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        std::fs::write(root.join("sub/.envrc"), "x").unwrap();
        // A worktree's .git file inside the root, and one naming a git
        // folder outside it (never bound: not the sandbox's to show).
        std::fs::write(root.join("sub/.git"), format!("gitdir: {}\n", root.join(".git/worktrees/w").display())).unwrap();
        std::fs::write(root.join("sub/deeper/.git"), format!("gitdir: {}\n", outside.path().display())).unwrap();
        let found = protected_in(&root, &root.join("sub/deeper"));
        let rel: Vec<String> = found.iter().map(|p| p.strip_prefix(&root).unwrap().display().to_string()).collect();
        for want in [".git", ".pi", "sub/.git", ".git/worktrees/w", "sub/.claude", "sub/.envrc"] {
            assert!(rel.iter().any(|r| r == want), "{want} missing: {rel:?}");
        }
        // Not on the way to the cwd, and the outside git folder: left out.
        assert!(!rel.iter().any(|r| r.starts_with("other")), "{rel:?}");
        assert!(found.iter().all(|p| p.starts_with(&root)), "{found:?}");
        assert!(rel.iter().any(|r| r == "sub/deeper/.git"), "{rel:?}");
        // Nothing made for names that aren't there.
        assert!(!root.join(".codex").exists() && !root.join("sub/.pi").exists());
    }
}
