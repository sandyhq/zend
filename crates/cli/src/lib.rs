//! `zend`'s command line, for every binary that answers to it: the Zend
//! app (`apps/zend`), its engine helper (`zend-engine`) and the zgx
//! interface (`apps/zgx`). One parser, one help text, one set of exit codes,
//! with no engine and no interface linked in.
//!
//! [`run`] parses this process's arguments and runs what needs neither
//! (`update`, `daemon`, `--noop-browser`, help and version). What's left is
//! the caller's:
//!   - no subcommand: open the window ([`Launch::Window`]);
//!   - a subcommand that runs the engine's code here ([`EngineCommand`]:
//!     `headless`, `engine`, `login`, `logout`, `status`, `sync`, `quiesce`,
//!     `mcp`, `plugin`, `template`). A binary that links the engine runs it
//!     itself ([`Engines::InProcess`], `zend_app::run_engine_command`); an
//!     interface that doesn't hands the whole command line to the engine
//!     helper beside it ([`Engines::Helper`]: `Contents/MacOS/Zend Engine`
//!     in a macOS bundle), which replaces this process and runs it there.

pub mod daemon;
mod env;
pub mod logging;
pub mod paths;
pub mod update_cli;

use clap::{CommandFactory as _, FromArgMatches as _, Parser, Subcommand};

pub use env::{
    DEFAULT_EDGE_URL, DEFAULT_IPC_PORT, DEFAULT_WORKOS_CLIENT_ID, config_dir, edge_token_from_env, edge_url_from_env, ipc_port_from,
    ipc_port_from_env, store_edge_config, workos_client_id_from_env, edge_config_in, edge_from_env, edge_record_in, restore_edge_record,
    store_desktop_record, desktop_record_in, clear_desktop_record,
};
pub use logging::{RotatingLog, init_logging, long_running_setup, open_log_file};

#[derive(Parser)]
#[command(
    name = "zend",
    version,
    about = "Multi-device controller for coding agents"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
    /// Open a Zend conversation URL.
    #[arg(value_name = "URL")]
    open_url: Option<String>,
    /// Open with Zend's own interface, whatever template is in use (as
    /// holding Alt, or ⌥ on a Mac, as Zend opens does).
    #[arg(long)]
    safe_mode: bool,
    #[cfg(windows)]
    #[arg(long, hide = true)]
    wait_for_exit: Option<u32>,
}

#[derive(Subcommand)]
enum Command {
    /// Run the engine without a UI (local-only unless a saved session enables sync).
    Headless,
    /// The desktop app's background engine (`zend-engine` where it ships
    /// separately): started by the app, it keeps sessions running with the
    /// app closed.
    #[command(hide = true)]
    Engine,
    /// Sign in and enable sync on the next engine start.
    Login,
    /// Remove the saved session and return to local-only on the next start.
    Logout,
    /// Show workspace mode, optional auth, and engine status.
    Status,
    /// Live sync introspection from the running engine: per-room connection
    /// state, last pushed-frame/ack ages, rejoin/probe/resync counters.
    Sync,
    /// Cloud sleep handshake: ask the running engine to start no new work.
    /// Exits 0 when quiesced, 75 when busy (a turn in flight, a message
    /// waiting) — the edge then keeps the machine awake.
    #[command(hide = true)]
    Quiesce,
    #[cfg(target_os = "linux")]
    /// Trigger an Appshot in the running headed instance (desktop shortcut fallback).
    Appshot,
    /// Serve the Zend MCP (Model Context Protocol) server on stdin/stdout,
    /// proxying to the running engine's IPC. Agents use it to create, read,
    /// and message chats. Logs go to stderr; stdout is the protocol.
    Mcp,
    /// Make, sign, check and list plugins (through the running Zend).
    Plugin {
        #[command(subcommand)]
        command: PluginCommand,
    },
    /// List, make, use and delete templates: whole replacements of Zend's
    /// interface (through the running Zend). `zend template use zend`
    /// puts Zend's own back.
    Template {
        #[command(subcommand)]
        command: TemplateCommand,
    },
    /// Pair this Zend with your other Zend servers and say what each may do
    /// here (through the running Zend, this device only).
    Peer {
        #[command(subcommand)]
        command: PeerCommand,
    },
    /// Podman containers this server runs agents in, each answering it alone
    /// (through the running Zend, this device only).
    Worker {
        #[command(subcommand)]
        command: WorkerCommand,
    },
    /// Manage `zend headless` as a background service (launchd / systemd --user).
    Daemon {
        #[command(subcommand)]
        command: DaemonCommand,
    },
    /// Check for a newer release and apply it (download → verify → swap →
    /// service restart). `--check` only reports (exits 1 when one is available).
    Update {
        #[arg(long)]
        check: bool,
    },
    /// Watches a new version while it proves itself after an update: started by
    /// the old version just before it hands over.
    #[command(name = "update-watchdog", hide = true)]
    UpdateWatchdog,
}

#[derive(Subcommand)]
enum DaemonCommand {
    /// Install, enable, and start the service (captures ZEND_* env).
    Install,
    /// Stop and remove the service.
    Uninstall,
    /// Start the installed service.
    Start,
    /// Stop the service.
    Stop,
    /// Restart the service.
    Restart,
    /// Show the service manager's view of the daemon.
    Status,
}

#[derive(Subcommand)]
pub enum PeerCommand {
    /// Print a one-time pairing code (good for 10 minutes) for the other
    /// server's `zend peer join`.
    Pair,
    /// Pair with the server whose `zend peer pair` printed this code.
    Join {
        /// The code (zend-peer:1:…).
        code: String,
    },
    /// This server's key and the servers paired with it, with their grants.
    List,
    /// Set what a paired server may do here, replacing what it had: none
    /// takes everything away. Grants: sessions.read, sessions.run:<project>,
    /// sessions.run.unconfined:<project>, projects.share:<project>,
    /// workers.manage. sessions.run.unconfined is full trust: an agent it
    /// runs works as you on this server and can read Zend's IPC token.
    /// sessions.run:<project> runs the server's chats sandboxed in a working
    /// copy of that project in Zend's data (a detached git worktree, or a
    /// copy), never in your folder, on the server's own API key (zend peer
    /// key set); what they change reaches your folder only through zend
    /// peer changes apply. In the copy the server's agent can write
    /// AGENTS.md, build scripts and code: applying is a review step, not a
    /// filter. Read the patch before you apply it, and what you apply as if
    /// you had written it yourself.
    Grant {
        /// The server: its key, a key prefix (8+) or its name.
        peer: String,
        grants: Vec<String>,
    },
    /// Forget a paired server: its connections end, it can't dial in again.
    Unpair {
        /// The server: its key, a key prefix (8+) or its name.
        peer: String,
    },
    /// What the chats paired servers run here changed in their working
    /// copies, and bringing it into your project folder.
    #[command(subcommand)]
    Changes(PeerChangesCommand),
    /// The API keys a paired server's sandboxed chats run on, one per model
    /// provider. They never get your own logins.
    #[command(subcommand)]
    Key(PeerKeyCommand),
}

#[derive(Subcommand)]
pub enum WorkerCommand {
    /// Make a worker: a rootless Podman container running Zend's engine with
    /// one project, /work (a volume of its own), answering only this server
    /// over a socket in Zend's data. It reaches the internet, never this
    /// host, its LAN, docker or Podman networks or the tailnet.
    Create {
        /// Its name: lowercase letters, digits and -.
        name: String,
        /// The image (default localhost/zend-worker:latest, infra/worker/build.sh).
        #[arg(long)]
        image: Option<String>,
        /// A model provider whose API key (read from stdin) its agents run on
        /// (anthropic, openai, …); kept as a Podman secret, given as the
        /// provider's variable. No login of yours goes in.
        #[arg(long = "key", value_name = "PROVIDER")]
        key: Option<String>,
        /// Its memory, swap none (default 4g).
        #[arg(long)]
        memory: Option<String>,
        /// Its CPUs (default 2).
        #[arg(long)]
        cpus: Option<String>,
        /// Its processes and threads (default 512).
        #[arg(long)]
        pids: Option<u32>,
    },
    /// This server's workers and their containers' state.
    List,
    /// Start a stopped worker (its networks blocked again before it runs).
    Start { name: String },
    /// Remove a worker: its container, key secrets and folders, and its
    /// volumes (/work's files with them) unless --keep-volumes.
    Rm {
        name: String,
        #[arg(long)]
        keep_volumes: bool,
    },
}

#[derive(Subcommand)]
pub enum PeerChangesCommand {
    /// Every working copy: its chat, server, project and changed files.
    List,
    /// What a working copy changed since it was made (or last applied), as a patch.
    Show {
        /// The chat the copy was made for.
        chat: String,
    },
    /// Write exactly the changes you were shown (by their tree, from show or
    /// list) into your project folder: all or nothing, refused when the
    /// copy changed since, or a chat in it runs. Applying is a review step,
    /// not a filter: AGENTS.md, build scripts and code are applied as you
    /// read them, so read the patch first. Always refused: .git and the
    /// agents' and tools' settings folders (.pi, .claude, .vscode, …), files
    /// your tools run on their own (.gitattributes, .gitmodules, hook
    /// managers, flake.nix, shell.nix, .mcp.json, opencode.json, .zed,
    /// .devcontainer, .github/workflows), symbolic links, paths through a
    /// link in your folder, and files you changed since the copy was made.
    /// Files that instruct your agents (AGENTS.md, CLAUDE.md, GEMINI.md,
    /// .cursorrules, .windsurfrules) only with --allow <path>.
    Apply {
        /// The chat the copy was made for.
        chat: String,
        /// The tree you were shown (zend peer changes show prints it).
        tree: String,
        /// A flagged path (AGENTS.md and the like) to apply too; repeat it.
        #[arg(long = "allow", value_name = "PATH")]
        allow: Vec<String>,
    },
    /// Throw a working copy away: its chats stop running.
    Discard {
        /// The chat the copy was made for.
        chat: String,
    },
}

/// `text` safe to print to a terminal: control characters (escape
/// sequences, carriage returns, …) and the characters that reorder text
/// (bidirectional overrides and isolates) shown as `\u{…}`; newlines and
/// tabs kept when `lines` is true.
pub fn terminal_safe(text: &str, lines: bool) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        let reorders = matches!(c, '\u{200E}' | '\u{200F}' | '\u{061C}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}');
        if (lines && (c == '\n' || c == '\t')) || !(c.is_control() || reorders) {
            out.push(c);
        } else {
            out.push_str(&format!("\\u{{{:04x}}}", c as u32));
        }
    }
    out
}

#[derive(Subcommand)]
pub enum PeerKeyCommand {
    /// Keep the API key (read from stdin) a server's chats use for a
    /// provider (anthropic, openai, google, openrouter, …). Kept owner-only
    /// in Zend's data, never synced, never sent anywhere but into those
    /// chats' sandbox, as the provider's variable (ANTHROPIC_API_KEY, …).
    Set {
        /// The server: its key, a key prefix (8+) or its name.
        peer: String,
        /// The model provider, as in Pi's model ids (anthropic/claude-…).
        provider: String,
    },
    /// Forget a server's key for a provider: its runs on it stop.
    Rm {
        /// The server: its key, a key prefix (8+) or its name.
        peer: String,
        provider: String,
    },
    /// The providers each server has a key for (names only).
    List,
}

#[derive(Subcommand)]
pub enum PluginCommand {
    /// Make a new plugin in Zend's plugins folder, signed as you.
    New {
        /// Its id: lowercase letters, digits and dashes.
        id: String,
        /// Its name (defaults to the id).
        #[arg(long)]
        name: Option<String>,
        /// A permission it asks for (repeat for more).
        #[arg(long = "permission", value_name = "P")]
        permissions: Vec<String>,
    },
    /// Sign a plugin as you: an id in the plugins folder, or a folder's path.
    Sign { target: String },
    /// Check a plugin folder's signature and who made it.
    Verify { path: String },
    /// Check a plugin's Luau before Zend runs it: its syntax, and its
    /// types with luau-lsp (each file in the mode it asks for, `--!strict`
    /// or Luau's default). Each problem is said as file:line:column; exits
    /// 1 when there's an error. Runs here, without Zend open.
    Check {
        /// An id in the plugins folder, or a folder's path.
        target: String,
        /// Check every file strictly, not only those that ask.
        #[arg(long)]
        strict: bool,
        /// Print the report as JSON.
        #[arg(long)]
        json: bool,
    },
    /// List the plugins Zend knows.
    List,
    /// Save a plugin as one signed .zend-plugin file to share (attach it
    /// to a GitHub release, or send it): anyone installs it from that file
    /// or its link. Your own plugin is signed again first.
    Export {
        /// The plugin's id (`zend plugin list` names them).
        id: String,
        /// Where to save it: a folder, or a file ending .zend-plugin
        /// (defaults to Zend's plugin-exports folder).
        #[arg(long, value_name = "PATH", value_parser = absolute_path)]
        to: Option<std::path::PathBuf>,
    },
    /// Show a plugin's log here: what its window part and its service
    /// printed, and what failed, with the traceback.
    Logs {
        /// The plugin's id (`zend plugin list` names them).
        id: String,
        /// Keep showing new lines as they come (Ctrl-C stops).
        #[arg(long, short)]
        follow: bool,
        /// How many of the last lines to show first.
        #[arg(long, short = 'n', default_value_t = 100)]
        lines: usize,
        /// Its preview's log (`zend plugin preview`), not yours.
        #[arg(long)]
        preview: bool,
    },
    /// Run a plugin folder in a preview: a Zend of its own beside yours
    /// (its own engine and a window that never shows), with a copy of the
    /// folder, what it keeps copied, allowed what you allowed it. Agents
    /// try a change there (MCP `plugin_input`, `plugin_status` with
    /// `preview`); yours stays as it is, whatever fails there. Run it
    /// again to copy the folder in again. With --in-window, a changed copy
    /// of one of your plugins runs in your own window instead of the one
    /// you keep, with a bar to Keep or Revert it.
    Preview {
        /// A plugin folder's path, or an id in the plugins folder.
        target: String,
        /// End the preview (with --in-window: revert it).
        #[arg(long)]
        stop: bool,
        /// In your own window, as a preview you keep or revert on its bar.
        #[arg(long)]
        in_window: bool,
    },
}

/// A path as given, made absolute against the current folder (the engine
/// takes full paths only).
fn absolute_path(text: &str) -> Result<std::path::PathBuf, String> {
    if text.trim().is_empty() {
        return Err("give a path".into());
    }
    std::path::absolute(text).map_err(|e| e.to_string())
}

#[derive(Subcommand)]
pub enum TemplateCommand {
    /// List the templates on this device, Zend's own first.
    List,
    /// Run Zend's window with a template. `zend template use zend` puts
    /// Zend's own back, even with Zend closed.
    Use { id: String },
    /// Make a new template: a full copy of Zend's interface to rewrite.
    Create {
        /// Its id: lowercase letters, digits and dashes.
        id: String,
        /// Its name (defaults to the id).
        #[arg(long)]
        name: Option<String>,
        /// A template to start from (defaults to Zend's own).
        #[arg(long)]
        from: Option<String>,
    },
    /// Show a template's errors, out-of-date files and what it inherits.
    Status { id: String },
    /// Delete a template (it's set aside in the templates folder's .removed).
    Delete { id: String },
}

/// A subcommand that runs the engine's own code in this process: what the
/// engine helper (or a binary linking the engine) does with it.
pub enum EngineCommand {
    Headless,
    Engine,
    Login,
    Logout,
    Status,
    Sync,
    Quiesce,
    Mcp,
    Plugin(PluginCommand),
    Template(TemplateCommand),
    Peer(PeerCommand),
    Worker(WorkerCommand),
}

/// Where this binary runs [`EngineCommand`]s.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Engines {
    /// Here: the binary links the engine (`zend`, `zend-engine`).
    InProcess,
    /// In the engine helper beside this binary, which replaces this process
    /// with the same arguments (an interface that doesn't link the engine).
    Helper,
}

/// What's left for the binary once [`run`] has parsed its arguments.
pub enum Launch {
    /// A command ran here; exit with its result.
    Done(anyhow::Result<()>),
    /// Run this in-process ([`Engines::InProcess`] only); logging is set up.
    Engine(EngineCommand),
    /// No subcommand: open the window, on a conversation URL if one was
    /// given; `safe_mode` with Zend's own interface (`--safe-mode`).
    Window { open_url: Option<String>, safe_mode: bool },
    /// Ask the running headed instance for an Appshot (the interface's own).
    #[cfg(target_os = "linux")]
    Appshot,
}

/// The parser, as `binary`'s `--version` names it: `version` is the build's
/// (the workspace's, or a beta interface's own, `0.2.136-beta.1`).
pub fn command(version: &'static str) -> clap::Command {
    Cli::command().version(version)
}

/// Parse this process's arguments and run what needs neither the engine nor
/// the interface. Help, `--version` and argument errors print and exit here,
/// as clap does.
pub fn run(version: &'static str, engines: Engines) -> Launch {
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("--noop-browser")) {
        return Launch::Done(Ok(()));
    }
    #[cfg(windows)]
    attach_parent_console();
    let cli = match Cli::from_arg_matches(&command(version).get_matches()) {
        Ok(cli) => cli,
        Err(error) => error.exit(),
    };
    if engines == Engines::Helper && cli.command.as_ref().is_some_and(Command::runs_engine) {
        // The helper parses the same arguments and does all of the below
        // itself (logging, the command). Windows' image cleanup is this
        // executable's, though (the helper isn't the installed zend.exe): an
        // install that's only ever run headless must lose the image an update
        // set aside too.
        #[cfg(windows)]
        if matches!(&cli.command, Some(Command::Headless) | Some(Command::Engine)) {
            zend_update::windows::cleanup_previous_image();
        }
        let exe = match std::env::current_exe() {
            Ok(exe) => exe,
            Err(error) => return Launch::Done(Err(error.into())),
        };
        let error = zend_launch::exec_helper(&exe, std::env::args_os().skip(1));
        return Launch::Done(Err(anyhow::anyhow!("couldn't start the engine: {error}")));
    }
    #[cfg(windows)]
    if let Some(pid) = cli.wait_for_exit {
        if let Err(error) = zend_update::windows::wait_for_exit(pid) {
            return Launch::Done(Err(error));
        }
    } else if matches!(
        &cli.command,
        None | Some(Command::Headless) | Some(Command::Engine)
    ) {
        zend_update::windows::cleanup_previous_image();
    }
    let long_running = matches!(&cli.command, None | Some(Command::Headless));
    let mode = if cli.command.is_some() {
        "headless"
    } else {
        "headed"
    };
    // `zend mcp` owns stdout for the protocol: a single log line on it
    // would corrupt the JSON-RPC stream, so its diagnostics go to stderr.
    // The background engine logs on its own (`zend_app::background`).
    if !matches!(&cli.command, Some(Command::Engine)) {
        init_logging(
            long_running,
            mode,
            matches!(&cli.command, Some(Command::Mcp | Command::Plugin { .. } | Command::Template { .. } | Command::Peer { .. } | Command::Worker { .. })),
        );
    }
    if long_running {
        long_running_setup();
    }

    let engine = |command| match engines {
        Engines::InProcess => Launch::Engine(command),
        // (Handed to the helper above.)
        Engines::Helper => unreachable!("engine commands run in the helper"),
    };
    match cli.command {
        Some(Command::Headless) => engine(EngineCommand::Headless),
        Some(Command::Engine) => engine(EngineCommand::Engine),
        Some(Command::Login) => engine(EngineCommand::Login),
        Some(Command::Logout) => engine(EngineCommand::Logout),
        Some(Command::Status) => engine(EngineCommand::Status),
        Some(Command::Sync) => engine(EngineCommand::Sync),
        Some(Command::Quiesce) => engine(EngineCommand::Quiesce),
        Some(Command::Mcp) => engine(EngineCommand::Mcp),
        Some(Command::Plugin { command }) => engine(EngineCommand::Plugin(command)),
        Some(Command::Template { command }) => engine(EngineCommand::Template(command)),
        Some(Command::Peer { command }) => engine(EngineCommand::Peer(command)),
        Some(Command::Worker { command }) => engine(EngineCommand::Worker(command)),
        #[cfg(target_os = "linux")]
        Some(Command::Appshot) => Launch::Appshot,
        Some(Command::Update { check }) => Launch::Done(
            tokio::runtime::Runtime::new()
                .map_err(anyhow::Error::from)
                .and_then(|runtime| runtime.block_on(update_cli::update(&edge_url_from_env(), check))),
        ),
        Some(Command::UpdateWatchdog) => Launch::Done(zend_update::transaction::run_watchdog()),
        Some(Command::Daemon { command }) => Launch::Done(match command {
            DaemonCommand::Install => daemon::install(&paths::data_dir()),
            DaemonCommand::Uninstall => daemon::uninstall(),
            DaemonCommand::Start => daemon::start(),
            DaemonCommand::Stop => daemon::stop(),
            DaemonCommand::Restart => daemon::restart(),
            DaemonCommand::Status => daemon::status(),
        }),
        None => Launch::Window {
            open_url: cli.open_url,
            safe_mode: cli.safe_mode,
        },
    }
}

impl Command {
    /// Whether it runs the engine's code ([`EngineCommand`]).
    fn runs_engine(&self) -> bool {
        match self {
            Self::Headless
            | Self::Engine
            | Self::Login
            | Self::Logout
            | Self::Status
            | Self::Sync
            | Self::Quiesce
            | Self::Mcp
            | Self::Plugin { .. }
            | Self::Template { .. }
            | Self::Peer { .. }
            | Self::Worker { .. } => true,
            #[cfg(target_os = "linux")]
            Self::Appshot => false,
            Self::Update { .. } | Self::UpdateWatchdog | Self::Daemon { .. } => false,
        }
    }
}

#[cfg(windows)]
fn attach_parent_console() {
    use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
    use windows_sys::Win32::System::Console::{
        ATTACH_PARENT_PROCESS, AttachConsole, GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE,
        STD_OUTPUT_HANDLE, SetStdHandle,
    };

    // The GUI subsystem prevents Explorer from creating a console at startup.
    // Reuse an existing parent's console for CLI output and cargo run, without
    // allocating one. Attach before Clap so help and argument errors work too.
    // Preserve redirected pipes/files: attaching may replace standard handles.
    unsafe {
        let saved = [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE]
            .map(|id| (id, GetStdHandle(id)));
        if AttachConsole(ATTACH_PARENT_PROCESS) != 0 {
            for (id, handle) in saved {
                if !handle.is_null() && handle != INVALID_HANDLE_VALUE {
                    SetStdHandle(id, handle);
                }
            }
        }
    }
}

/// Every subcommand path of the command line (`[]`, `["plugin"]`,
/// `["plugin", "new"]`, …), hidden ones included: what the parity checks of
/// each binary that runs it walk (`apps/zend/tests/cli.rs`,
/// `apps/zgx/tests/cli.rs`).
pub fn subcommand_paths(version: &'static str) -> Vec<Vec<String>> {
    fn walk(command: &clap::Command, path: &mut Vec<String>, out: &mut Vec<Vec<String>>) {
        out.push(path.clone());
        for sub in command.get_subcommands() {
            if sub.get_name() == "help" {
                continue;
            }
            path.push(sub.get_name().to_owned());
            walk(sub, path, out);
            path.pop();
        }
    }
    let mut out = Vec::new();
    walk(&command(version), &mut Vec::new(), &mut out);
    out
}

/// `zend <path…> --help` as a binary named `zend` prints it: clap's own
/// answer to those arguments.
pub fn help_for(version: &'static str, path: &[String]) -> String {
    let argv = std::iter::once("zend".to_owned())
        .chain(path.iter().cloned())
        .chain(std::iter::once("--help".to_owned()));
    match command(version).try_get_matches_from(argv) {
        Err(help) if help.kind() == clap::error::ErrorKind::DisplayHelp => help.render().to_string(),
        other => panic!("zend {} --help didn't print help: {other:?}", path.join(" ")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_in_a_servers_changes_cant_drive_the_terminal() {
        // A file name an agent chose: an escape sequence that clears the
        // screen and retitles the window, a carriage return that hides the
        // start of the line, and a right-to-left override that shows
        // "evil.sh" as something else.
        let name = "a\u{1b}[2J\u{1b}]0;owned\u{7}\rb\u{202e}hs.live\u{2066}x\u{200f}.txt";
        let shown = terminal_safe(name, false);
        assert!(!shown.chars().any(|c| c.is_control() || ('\u{2066}'..='\u{2069}').contains(&c) || ('\u{202a}'..='\u{202e}').contains(&c) || c == '\u{200f}'), "{shown}");
        assert_eq!(shown, "a\\u{001b}[2J\\u{001b}]0;owned\\u{0007}\\u{000d}b\\u{202e}hs.live\\u{2066}x\\u{200f}.txt");
        // A patch keeps its lines and tabs, nothing else.
        assert_eq!(terminal_safe("+x\t1\n+\u{1b}[31my\n", true), "+x\t1\n+\\u{001b}[31my\n");
        assert_eq!(terminal_safe("a\nb", false), "a\\u{000a}b");
    }

    #[test]
    fn the_command_line_is_well_formed() {
        command("0.0.0").debug_assert();
    }

    #[test]
    fn every_subcommand_is_reachable() {
        let paths = subcommand_paths("0.0.0");
        for expected in [
            &["headless"][..],
            &["engine"],
            &["login"],
            &["logout"],
            &["status"],
            &["sync"],
            &["quiesce"],
            &["mcp"],
            &["plugin", "new"],
            &["plugin", "export"],
            &["template", "use"],
            &["daemon", "install"],
            &["update"],
        ] {
            assert!(
                paths.iter().any(|path| path == expected),
                "{expected:?} missing from {paths:?}"
            );
        }
    }

    /// What runs where: the engine's commands go to the engine (in-process
    /// or the helper), the rest run here.
    #[test]
    fn engine_commands_are_the_ones_that_run_the_engine() {
        let parse = |args: &[&str]| {
            let argv = std::iter::once("zend").chain(args.iter().copied());
            Cli::from_arg_matches(&command("0.0.0").try_get_matches_from(argv).unwrap())
                .unwrap()
                .command
        };
        for args in [
            &["headless"][..],
            &["engine"],
            &["login"],
            &["logout"],
            &["status"],
            &["sync"],
            &["quiesce"],
            &["mcp"],
            &["plugin", "list"],
            &["plugin", "export", "focus"],
            &["template", "list"],
        ] {
            assert!(parse(args).unwrap().runs_engine(), "{args:?}");
        }
        for args in [&["update", "--check"][..], &["daemon", "status"]] {
            assert!(!parse(args).unwrap().runs_engine(), "{args:?}");
        }
        assert!(parse(&[]).is_none());
    }

    /// `--to` reaches the engine as a full path: a relative one is taken
    /// from the current folder.
    #[test]
    fn plugin_export_takes_a_full_path() {
        let export = |args: &[&str]| {
            let argv = ["zend", "plugin", "export"].into_iter().chain(args.iter().copied());
            match Cli::from_arg_matches(&command("0.0.0").try_get_matches_from(argv).unwrap())
                .unwrap()
                .command
            {
                Some(Command::Plugin { command: PluginCommand::Export { id, to } }) => (id, to),
                _ => panic!("not plugin export"),
            }
        };
        assert_eq!(export(&["focus"]), ("focus".to_owned(), None));
        let cwd = std::env::current_dir().unwrap();
        assert_eq!(
            export(&["focus", "--to", "dist/focus.zend-plugin"]).1,
            Some(cwd.join("dist/focus.zend-plugin"))
        );
        assert_eq!(export(&["focus", "--to", "."]).1.unwrap(), cwd);
        let full = if cfg!(windows) { r"C:\out" } else { "/tmp/out" };
        assert_eq!(export(&["focus", "--to", full]).1, Some(std::path::PathBuf::from(full)));
        assert!(
            command("0.0.0")
                .try_get_matches_from(["zend", "plugin", "export", "focus", "--to", ""])
                .is_err()
        );
        assert!(command("0.0.0").try_get_matches_from(["zend", "plugin", "export"]).is_err());
    }

    #[test]
    fn the_version_is_the_build_s() {
        let version = command("0.2.136-beta.1").render_version();
        assert_eq!(version.trim(), "zend 0.2.136-beta.1");
    }
}
