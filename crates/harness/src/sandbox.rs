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
//!   executable's folder) and its login files ([`logins`]), nothing else;
//! - the project root read-write;
//! - a new PID, IPC and UTS namespace, a new session (no terminal to inject
//!   into), killed with its parent (`--die-with-parent`);
//! - an environment cut down to an allowlist ([`final_env`]): no `ZEND_*`,
//!   no `SSH_*`, no session bus;
//! - the network as the host has it (the agent needs its model API).
//!
//! It never falls back to running unconfined: no bubblewrap, a root that
//! would expose the home or Zend's data, or a bubblewrap that can't set up
//! its namespaces (checked by starting it once, [`Launch::new`]) is an error.
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

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
/// Programs, not data: none holds a login.
pub const TOOL_DIRS: [&str; 15] = [
    ".local/bin",
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

/// System paths, read-only. A path that is a symlink (`/bin` → `usr/bin`)
/// is made the same symlink inside.
const SYSTEM: [&str; 8] = ["/usr", "/etc", "/bin", "/sbin", "/lib", "/lib64", "/lib32", "/libx32"];

/// The host side of a sandbox: whose home it is, where bubblewrap is, and
/// what else must stay hidden (the engine's data dir).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Host {
    pub home: PathBuf,
    /// bubblewrap itself; `None` looks it up (`/usr/bin/bwrap`, then PATH).
    pub bwrap: Option<PathBuf>,
    /// Beyond [`HIDDEN_HOME_DIRS`]: folders the root may not be, hold or be in.
    pub hidden: Vec<PathBuf>,
}

impl Host {
    /// This process's home, bubblewrap looked up, nothing more hidden.
    pub fn current() -> Self {
        Self {
            home: crate::executable::home_or_current_dir(),
            bwrap: None,
            hidden: Vec::new(),
        }
    }
}

/// One run's confinement: its project root, checked against the host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sandbox {
    root: PathBuf,
    host: Host,
    bwrap: PathBuf,
}

fn refuse(why: impl std::fmt::Display) -> HarnessError {
    HarnessError::Sandbox(format!(
        "This session must run sandboxed and was not started: {why}"
    ))
}

impl Sandbox {
    /// Confine to `root` (the run's project folder). Refused, never relaxed:
    /// no bubblewrap; a root that doesn't exist, is `/`, is the home or
    /// holds it; a root that is, holds or lies in a hidden folder.
    pub fn project(root: &Path, host: Host) -> Result<Self, HarnessError> {
        if !cfg!(target_os = "linux") {
            return Err(refuse("sandboxed sessions need Linux with bubblewrap"));
        }
        let bwrap = match &host.bwrap {
            Some(path) if path.is_file() => path.clone(),
            Some(path) => {
                return Err(refuse(format!(
                    "bubblewrap (bwrap) was not found at {}",
                    path.display()
                )));
            }
            None => find_bwrap().ok_or_else(|| {
                refuse("bubblewrap (bwrap) is not installed on this device")
            })?,
        };
        let root = std::fs::canonicalize(root)
            .map_err(|e| refuse(format!("its project folder {}: {e}", root.display())))?;
        if !root.is_dir() {
            return Err(refuse(format!("{} is not a folder", root.display())));
        }
        let home = std::fs::canonicalize(&host.home).unwrap_or_else(|_| host.home.clone());
        if root.parent().is_none() || home.starts_with(&root) {
            return Err(refuse(format!(
                "its project folder {} is or holds the home folder",
                root.display()
            )));
        }
        let hidden = HIDDEN_HOME_DIRS
            .iter()
            .map(|dir| home.join(dir))
            .chain(host.hidden.iter().map(|dir| {
                std::fs::canonicalize(dir).unwrap_or_else(|_| dir.clone())
            }));
        for dir in hidden {
            if root.starts_with(&dir) || dir.starts_with(&root) {
                return Err(refuse(format!(
                    "its project folder {} would show {}",
                    root.display(),
                    dir.display()
                )));
            }
        }
        Ok(Self { root, host, bwrap })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn host(&self) -> &Host {
        &self.host
    }
}

fn find_bwrap() -> Option<PathBuf> {
    ["/usr/bin/bwrap", "/usr/local/bin/bwrap", "/bin/bwrap"]
        .iter()
        .map(PathBuf::from)
        .find(|p| p.is_file())
        .or_else(|| {
            std::env::var_os("PATH").and_then(|path| {
                std::env::split_paths(&path)
                    .map(|dir| dir.join("bwrap"))
                    .find(|p| p.is_file())
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

fn reads_api_keys(harness: HarnessId) -> bool {
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
        || (reads_api_keys(harness) && key.ends_with("_API_KEY"))
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

/// One process start under a [`Sandbox`]: the bubblewrap arguments for the
/// agent `harness` is, and its environment rule.
#[derive(Clone, Debug)]
pub struct Launch {
    harness: HarnessId,
    bwrap: PathBuf,
    home: PathBuf,
    args: Vec<OsString>,
}

impl Launch {
    /// The launch of `exe` (the agent CLI) for `harness`, in `cwd` (inside
    /// the sandbox's root), with `logins` (from [`logins`]) read-only.
    /// Starts bubblewrap once with the same mounts (running `true`), so a
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
        let mut args: Vec<OsString> = [
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
        // DNS: /etc/resolv.conf often points into /run (systemd-resolved).
        if let Ok(resolv) = std::fs::canonicalize("/etc/resolv.conf")
            && !resolv.starts_with("/etc")
            && !resolv.starts_with("/usr")
            && let Some(dir) = resolv.parent()
        {
            push(&["--ro-bind-try".as_ref(), dir.as_os_str(), dir.as_os_str()]);
        }
        push(&["--proc".as_ref(), "/proc".as_ref()]);
        push(&["--dev".as_ref(), "/dev".as_ref()]);
        push(&["--tmpfs".as_ref(), "/tmp".as_ref()]);
        push(&["--tmpfs".as_ref(), home.as_os_str()]);
        let mut tools: Vec<PathBuf> = TOOL_DIRS
            .iter()
            .map(|dir| home.join(dir))
            .filter(|dir| dir.is_dir())
            .collect();
        // The agent's own folder, where none of those holds it (a CLI in
        // /opt, a pinned path): its real file's folder.
        if let Ok(real) = std::fs::canonicalize(exe)
            && !SYSTEM.iter().any(|dir| real.starts_with(dir))
            && !tools.iter().any(|dir| real.starts_with(dir))
            && let Some(dir) = real.parent()
        {
            tools.push(dir.to_path_buf());
        }
        for dir in &tools {
            push(&["--ro-bind".as_ref(), dir.as_os_str(), dir.as_os_str()]);
        }
        for file in logins {
            push(&["--ro-bind-try".as_ref(), file.as_os_str(), file.as_os_str()]);
        }
        let root = sandbox.root.as_os_str();
        push(&["--bind".as_ref(), root, root]);
        push(&["--chdir".as_ref(), cwd.as_os_str()]);
        let launch = Self {
            harness,
            bwrap: sandbox.bwrap.clone(),
            home,
            args,
        };
        launch.preflight()?;
        Ok(launch)
    }

    fn preflight(&self) -> Result<(), HarnessError> {
        let output = std::process::Command::new(&self.bwrap)
            .args(&self.args)
            .arg("--")
            .arg("true")
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .stdin(std::process::Stdio::null())
            .output()
            .map_err(|e| refuse(format!("bubblewrap ({}) could not start: {e}", self.bwrap.display())))?;
        if output.status.success() {
            return Ok(());
        }
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(refuse(format!(
            "bubblewrap failed ({}): {}",
            output.status,
            stderr.trim()
        )))
    }

    /// A command that runs `program` inside the sandbox: bubblewrap with
    /// this launch's mounts, then `program`. Arguments added to it go to
    /// `program`. Start it with [`Launch::spawn`], which cuts its
    /// environment down.
    pub fn command(&self, program: impl AsRef<OsStr>) -> Command {
        let mut command = Command::new(&self.bwrap);
        command.args(&self.args).arg("--").arg(program);
        command
    }

    /// The arguments bubblewrap gets before `--` (for tests and logs).
    pub fn bwrap_args(&self) -> &[OsString] {
        &self.args
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
        let env = final_env(self.harness, &self.home, std::env::vars_os(), explicit);
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
    }

    #[test]
    fn a_root_that_would_show_the_home_or_zend_data_is_refused() {
        let home = tempfile::tempdir().unwrap();
        let data = home.path().join("data");
        for dir in [".ssh", ".zend/scratch/x", ".config/zend", "projects/a", "data"] {
            std::fs::create_dir_all(home.path().join(dir)).unwrap();
        }
        // A bwrap path that exists: these refusals come before it is run.
        let host = Host {
            home: home.path().into(),
            bwrap: Some(PathBuf::from("/bin/sh")),
            hidden: vec![data.clone()],
        };
        let refused = |root: &Path| Sandbox::project(root, host.clone()).unwrap_err().to_string();
        assert!(refused(home.path()).contains("home folder"));
        assert!(refused(Path::new("/")).contains("home folder"));
        assert!(refused(&home.path().join(".ssh")).contains(".ssh"));
        assert!(refused(&home.path().join(".zend/scratch/x")).contains(".zend"));
        assert!(refused(&home.path().join(".config")).contains(".config/zend"));
        assert!(refused(&data).contains("data"));
        assert!(refused(&home.path().join("missing")).contains("missing"));
        assert!(Sandbox::project(&home.path().join("projects/a"), host.clone()).is_ok());
        let no_bwrap = Host {
            bwrap: Some(home.path().join("no-bwrap")),
            ..host
        };
        let error = Sandbox::project(&home.path().join("projects/a"), no_bwrap)
            .unwrap_err()
            .to_string();
        assert!(error.contains("bubblewrap (bwrap) was not found"), "{error}");
    }
}
