//! What each engine call is to a paired server (plan step 6): the one
//! [`Scope`] that opens it, so [`crate::peer_server_allowed`] and the
//! engine's gate check every call by its area, and the window's side
//! forwards each call a server may take ([`peer_scope`]).
//!
//! Every method of [`crate::methods`] and every `Mutate` op is classed here
//! by name. A name not listed is `None`: unclassed, refused like
//! [`Scope::Never`], and a test fails until it's classed (Rust can't match
//! strings exhaustively, so the tests hold the table to the constants and to
//! the engine's `MutateParams`).
//!
//! The session grants open exactly what they opened before step 6; what
//! else a chat does from the window (its queue, visuals, uploads, forks,
//! tool output) is [`Scope::Full`]'s for now.

use crate::methods as m;

/// The area a call is in, and so which grant opens it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Scope {
    /// `EngineInfo`: open to a session grant or any area grant (not to
    /// `projects.share` or `workers.manage` alone, as before step 6).
    Info,
    /// `sessions.read` or any `sessions.run` (harnesses, models, a
    /// chat's transcript; the engine checks the chat).
    Sessions,
    /// `sessions.read`: the chat and session lists.
    SessionsRead,
    /// `sessions.run[.unconfined]:<p>`: making and running chats; the
    /// engine checks the project and chat.
    SessionsRun,
    /// `projects.share:<p>`: a shared project's files by a path from its root.
    ProjectsShare,
    /// `workers.manage`: this server's workers. Opens nothing yet.
    WorkersManage,
    /// `projects.manage`: projects, repositories, branches, worktrees,
    /// browsing folders, project actions' definitions.
    ProjectsManage,
    /// `files`: any file of the machine, diffs, change requests.
    Files,
    /// `settings`: plugins, templates, themes, Zend's settings, the sidebar.
    Settings,
    /// `logins`: agent accounts and their sign-in, ACP agents, harnesses.
    Logins,
    /// `updates`: Zend's and the harnesses' updates.
    Updates,
    /// `terminal`: terminals, project actions run, background tasks, previews.
    Terminal,
    /// `full` only: what no area above holds (a chat's queue and visuals,
    /// plugin consent and signing, ...).
    Full,
    /// Never a paired server's, whatever it holds: pairing, grants, peer
    /// keys and logins, the account, orgs, Cloud, billing, vault, GitHub,
    /// the phone link, stopping the engine, this engine's own transport.
    Never,
}

impl Scope {
    /// What opens it, in words (the grant's name where there is one).
    pub fn name(self) -> &'static str {
        match self {
            Scope::Info => "any grant",
            Scope::Sessions => "sessions.read or sessions.run",
            Scope::SessionsRead => "sessions.read",
            Scope::SessionsRun => "sessions.run",
            Scope::ProjectsShare => "projects.share",
            Scope::WorkersManage => "workers.manage",
            Scope::ProjectsManage => "projects.manage",
            Scope::Files => "files",
            Scope::Settings => "settings",
            Scope::Logins => "logins",
            Scope::Updates => "updates",
            Scope::Terminal => "terminal",
            Scope::Full => "full",
            Scope::Never => "never",
        }
    }

    /// Every scope a grant can open (all but [`Scope::Never`]).
    pub const GRANTABLE: [Scope; 13] = [
        Scope::Info,
        Scope::Sessions,
        Scope::SessionsRead,
        Scope::SessionsRun,
        Scope::ProjectsShare,
        Scope::WorkersManage,
        Scope::ProjectsManage,
        Scope::Files,
        Scope::Settings,
        Scope::Logins,
        Scope::Updates,
        Scope::Terminal,
        Scope::Full,
    ];

    /// The grantable scope called `name` ([`Scope::name`]); never
    /// [`Scope::Never`].
    pub fn from_name(name: &str) -> Option<Scope> {
        Self::GRANTABLE.into_iter().find(|scope| scope.name() == name)
    }

    /// One of the areas step 6 added (`projects.manage` … `full`).
    pub fn is_area(self) -> bool {
        matches!(
            self,
            Scope::ProjectsManage | Scope::Files | Scope::Settings | Scope::Logins | Scope::Updates | Scope::Terminal | Scope::Full
        )
    }

    /// Logged at INFO on the server at each call (method and caller only).
    pub fn logged(self) -> bool {
        matches!(self, Scope::Files | Scope::Terminal | Scope::Logins | Scope::Full)
    }
}

/// Every `Mutate` op (the engine's `MutateParams` tags).
pub const MUTATE_OPS: [&str; 18] = [
    "createChat",
    "createSpace",
    "renameSpace",
    "setSpaceIcon",
    "deleteSpace",
    "renameChat",
    "setChatBranch",
    "setChatCwd",
    "setChatActivity",
    "setChatHost",
    "setChatArchived",
    "changeSidebarPin",
    "setChatConfig",
    "deleteChat",
    "renameDevice",
    "removeDevice",
    "markChatSeen",
    "setChatSleep",
];

/// The scope of a `Mutate` op; `None`: no such op.
pub fn mutate_scope(op: &str) -> Option<Scope> {
    use Scope::*;
    Some(match op {
        "createChat" | "renameChat" | "setChatArchived" | "setChatConfig" | "deleteChat" | "markChatSeen" => SessionsRun,
        "createSpace" | "renameSpace" | "setSpaceIcon" | "deleteSpace" => ProjectsManage,
        "changeSidebarPin" | "renameDevice" => Settings,
        // Where and when a chat runs: the owner's.
        "setChatBranch" | "setChatCwd" | "setChatActivity" | "setChatSleep" => Full,
        // Another device out of the account, a chat moved to another
        // device: the account's.
        "removeDevice" | "setChatHost" => Never,
        _ => return None,
    })
}

/// The scope of `method` (of `Mutate`, of its `op`; with no op, `None`).
/// `None`: unclassed, refused.
pub fn peer_scope(method: &str, op: Option<&str>) -> Option<Scope> {
    use Scope::*;
    Some(match method {
        m::MUTATE => return op.and_then(mutate_scope),

        // ---- sessions, as before step 6 ----
        m::ENGINE_INFO => Info,
        m::LIST_HARNESSES | m::LIST_MODELS | m::WATCH_DOC_MESSAGES => Sessions,
        m::WATCH_CHATS | m::WATCH_SESSIONS => SessionsRead,
        m::QUEUE_COMMAND => SessionsRun,
        m::PEER_LIST_PROJECT_FILES | m::PEER_READ_PROJECT_FILE => ProjectsShare,

        // ---- workers (opens nothing yet; their logins are Never) ----
        m::CREATE_WORKER | m::LIST_WORKERS | m::START_WORKER | m::REMOVE_WORKER | m::STOP_WORKER => WorkersManage,

        // ---- projects.manage ----
        m::WATCH_SPACES
        | m::LIST_REPOS
        | m::ADD_REPO
        | m::CLONE_REPO
        | m::CREATE_REPO
        | m::LIST_BRANCHES
        | m::LIST_REFS
        | m::LIST_GIT_HISTORY
        | m::SEARCH_GIT_HISTORY
        | m::RESOLVE_GIT_AVATARS
        | m::FETCH_ALL
        | m::SWITCH_REF
        | m::LIST_FOLDERS
        | m::GET_WORKTREE_SETTINGS
        | m::SET_WORKTREE_SETTINGS
        | m::LIST_DRIVES
        | m::SEARCH_HOME_FOLDERS
        | m::RELEASE_HOME_FOLDER_SEARCH
        | m::CREATE_WORKTREE
        | m::DELETE_WORKTREE
        | m::LIST_PROJECT_ACTIONS => ProjectsManage,

        // ---- files ----
        m::SEARCH_FILES
        | m::LIST_WORKSPACE_DIRECTORY
        | m::SEARCH_WORKSPACE_FILES
        | m::WARM_WORKSPACE_SEARCH
        | m::SEARCH_WORKSPACE_CONTENT
        | m::READ_WORKSPACE_IMAGE
        | m::READ_WORKSPACE_FILE
        | m::MOVE_WORKSPACE_ENTRY
        | m::DELETE_WORKSPACE_ENTRY
        | m::CREATE_WORKSPACE_ENTRY
        | m::WRITE_WORKSPACE_FILE
        | m::WATCH_WORKSPACE_FILES
        | m::WATCH_CHECKOUT_DIFFS
        | m::WATCH_WORKSPACE_GIT_STATUS
        | m::WATCH_CHECKOUT_CHANGE_REQUEST
        | m::LIST_CHANGE_REQUEST_PAGE
        | m::GET_CHANGE_REQUEST_REPOSITORY
        | m::GET_CHANGE_REQUEST
        | m::POST_CHANGE_REQUEST_COMMENT
        | m::POST_CHANGE_REQUEST_REVIEW_COMMENT
        | m::REPLY_CHANGE_REQUEST_THREAD
        | m::RESOLVE_CHANGE_REQUEST_THREAD
        | m::GET_CHANGE_REQUEST_DIFF
        | m::GET_CHECKOUT_DIFF
        | m::DISCARD_WORKING_TREE
        | m::GET_CHECKOUT_FILE_DIFF_TEXT => Files,

        // ---- terminal ----
        m::OPEN_TERMINAL
        | m::SUBSCRIBE_TERMINAL
        | m::WRITE_TERMINAL
        | m::RESIZE_TERMINAL
        | m::CLOSE_TERMINAL
        | m::TERMINAL_STATUS
        // A project's actions are commands it runs (a worktree's setup
        // too), an agent's environment one it starts with (LD_PRELOAD,
        // NODE_OPTIONS): a shell's.
        | m::UPSERT_PROJECT_ACTION
        | m::DELETE_PROJECT_ACTION
        | m::RUN_PROJECT_ACTION
        | m::TAKE_PROJECT_ACTION_SETUP
        | m::SET_ACP_AGENT_ENV
        | m::STOP_BACKGROUND_TASK
        | m::READ_BACKGROUND_TASK_OUTPUT
        | m::WATCH_PREVIEWS => Terminal,

        // ---- settings ----
        m::GET_TITLE_SETTINGS
        | m::SET_TITLE_SETTINGS
        | m::GET_COMMIT_CREDIT
        | m::SET_COMMIT_CREDIT
        | m::GET_NATIVE_SUBAGENTS
        | m::SET_NATIVE_SUBAGENTS
        | m::WATCH_SIDEBAR_PREFERENCES
        | m::DEVICE_STATS
        | m::LIST_PLUGINS
        | m::WATCH_PLUGINS
        | m::READ_PLUGIN_FILE
        | m::SET_PLUGIN_OFF
        | m::REMOVE_PLUGIN
        | m::INSTALL_PLUGIN
        | m::STAGE_PLUGIN_PACKAGE
        | m::CHECK_PLUGIN_UPDATES
        | m::UPDATE_PLUGIN
        | m::LIST_PLUGIN_TOOLS
        | m::WATCH_PLUGIN_TOOLS
        | m::PLUGIN_STATUS
        | m::PLUGIN_LOGS
        | m::WATCH_PLUGIN_LOGS
        | m::DESCRIBE_ELEMENTS
        | m::LIST_TEMPLATES
        | m::CREATE_TEMPLATE
        | m::USE_TEMPLATE
        | m::TEMPLATE_STATUS
        | m::DELETE_TEMPLATE
        | m::CHECK_TEMPLATE_UPDATES
        | m::UPDATE_TEMPLATE
        | m::DESCRIBE_APP
        | m::LIST_THEMES
        | m::GET_THEME
        | m::CREATE_THEME
        | m::UPDATE_THEME
        | m::DELETE_THEME
        | m::USE_THEME
        | m::WATCH_THEMES => Settings,

        // ---- logins ----
        m::INSTALL_HARNESS
        | m::CANCEL_INSTALL
        | m::SET_HARNESS_ENABLED
        | m::LIST_ACP_REGISTRY
        | m::ADD_ACP_AGENT
        | m::REMOVE_ACP_AGENT
        | m::GET_ACP_AGENT_ENV
        | m::GET_ACP_AGENT_SIGN_IN
        | m::SIGN_OUT_ACP_AGENT
        | m::SET_ACP_AGENT_KEY
        | m::LIST_AGENT_ACCOUNTS
        | m::ACTIVATE_AGENT_ACCOUNT
        | m::FORGET_AGENT_ACCOUNT
        | m::START_AGENT_LOGIN
        | m::RELAY_AGENT_LOGIN_CALLBACK
        | m::COMPLETE_AGENT_LOGIN
        | m::POLL_AGENT_LOGIN
        | m::CANCEL_AGENT_LOGIN
        | m::CONSUME_CODEX_RESET_CREDIT => Logins,

        // ---- updates ----
        m::UPDATE_STATUS
        | m::APPLY_UPDATE
        | m::WATCH_HARNESS_UPDATES
        | m::CHECK_HARNESS_UPDATES
        | m::APPLY_HARNESS_UPDATE
        | m::CANCEL_HARNESS_UPDATE
        | m::DISMISS_HARNESS_UPDATE
        | m::SET_HARNESS_UPDATE_POLICY => Updates,

        // ---- full only ----
        // A chat beyond making and running it: its queue, visuals, forks,
        // uploads and attachments, tool output, warm starts, voice.
        m::LIST_SKILLS
        | m::LIST_COMMANDS
        | m::WARM_START
        | m::WARM_STOP
        | m::FORK_SIDE_CHAT
        | m::REWIND_CHAT
        | m::WATCH_QUEUE
        | m::QUEUE_MESSAGE
        | m::UPDATE_QUEUED_MESSAGE
        | m::BEGIN_QUEUED_MESSAGE_EDIT
        | m::RENEW_QUEUED_MESSAGE_EDIT
        | m::FINISH_QUEUED_MESSAGE_EDIT
        | m::MOVE_QUEUED_MESSAGE
        | m::REMOVE_QUEUED_MESSAGE
        | m::SEND_QUEUED_MESSAGE_NOW
        | m::STEER_QUEUED_MESSAGE_NOW
        | m::STOP_WAITING
        | m::WATCH_WAITING_MESSAGES
        | m::AWAIT_BACKGROUND
        | m::READ_TOOL_OUTPUT
        | m::READ_TOOL_EXCHANGE
        | m::READ_TOOL_IMAGE_CHUNK
        | m::UPLOAD_CHUNK
        | m::UPLOAD_COMMIT
        | m::READ_ATTACHMENT_CHUNK
        | m::READ_CHAT_IMAGE_CHUNK
        | m::PUBLISH_VISUAL
        | m::SAVE_VISUAL_ENTRIES
        | m::UPDATE_VISUAL
        | m::READ_VISUAL
        | m::MARK_VISUAL_SENT
        | m::LIST_OPEN_VISUALS
        | m::PUBLISH_IMAGE
        | m::FETCH_TOOL_BLOB
        | m::VOICE_CAPABILITIES_V2
        | m::PREPARE_VOICE_V2
        | m::OWN_VOICE_V2
        | m::NEGOTIATE_VOICE_V2
        | m::CONFIRM_VOICE_MEDIA_V2
        | m::REPORT_VOICE_MEDIA_V2
        | m::STOP_VOICE_V2
        | m::CANCEL_VOICE_ATTEMPT_V2
        // The engine itself and the account's device list.
        | m::LOCAL_DEVICE
        | m::ENGINE_READY
        | m::WATCH_DEVICES
        // An agent of your own (a command of your choice) runs as you.
        | m::ADD_OWN_AGENT
        // Plugins and templates: consent, authoring, signing, running
        // their code, files by path.
        | m::DECIDE_PLUGIN
        | m::REVIEW_PLUGIN
        | m::NEW_PLUGIN
        | m::SIGN_PLUGIN
        | m::VERIFY_PLUGIN
        | m::CHECK_PLUGIN
        | m::CALL_PLUGIN_TOOL
        | m::PLUGIN_SEND
        | m::CALL_PLUGIN_SERVICE
        | m::PLUGIN_AGENTS
        | m::WATCH_PLUGIN_CHANNEL
        // Re-signs a plugin as this device's.
        | m::EXPORT_PLUGIN
        | m::PLUGIN_PREVIEW
        | m::PLUGIN_INPUT
        | m::KEEP_PLUGIN_PREVIEW
        | m::EXPORT_TEMPLATE
        | m::IMPORT_TEMPLATE
        | m::DECIDE_TEMPLATE
        | m::REVIEW_TEMPLATE
        | m::POST_NOTIFICATION => Full,

        // ---- never ----
        // Pairing, grants, servers' working copies, peer keys and logins,
        // workers' logins: this device's own (and a server never approves
        // its own chats' changes).
        m::PEER_PAIR
        | m::PEER_JOIN
        | m::LIST_PEERS
        | m::SET_PEER_GRANTS
        | m::UNPAIR_PEER
        | m::LIST_PEER_CHANGES
        | m::SHOW_PEER_CHANGES
        | m::APPLY_PEER_CHANGES
        | m::DISCARD_PEER_CHANGES
        | m::SET_PEER_KEY
        | m::REMOVE_PEER_KEY
        | m::LIST_PEER_KEYS
        | m::ALLOW_PEER_LOGIN
        | m::REMOVE_PEER_LOGIN
        | m::LIST_PEER_LOGINS
        | m::ALLOW_WORKER_LOGIN
        | m::REMOVE_WORKER_LOGIN
        | m::LIST_WORKER_LOGINS
        // The account.
        | m::AUTH_STATUS
        | m::SIGN_IN
        | m::SIGN_IN_HEADLESS
        | m::COMPLETE_SIGN_IN
        | m::SIGN_OUT
        | m::ACCESS_TOKEN
        | m::STEP_UP
        | m::DELETE_ACCOUNT
        | m::LOCAL_IMPORT_STATUS
        | m::IMPORT_LOCAL_WORKSPACE
        | m::LIST_ORGS
        | m::CREATE_ORG
        | m::SELECT_ORG
        // Cloud, billing, the vault, GitHub.
        | m::CLOUD_STATUS
        | m::CLOUD_ENABLE
        | m::CLOUD_DELETE
        | m::CLOUD_SESSIONS
        | m::CLOUD_SET_MACHINE
        | m::CLOUD_SESSION_WAKE
        | m::CLOUD_SESSION_SLEEP
        | m::CLOUD_SESSION_RETRY
        | m::CLOUD_SESSION_DELETE
        | m::CLOUD_USAGE
        | m::QUIESCE_FOR_SLEEP
        | m::BILLING_STATUS
        | m::BILLING_CHECKOUT
        | m::BILLING_PORTAL
        | m::VAULT_STATUS
        | m::VAULT_CONNECT_CODEX
        | m::VAULT_CONNECT_CLAUDE
        | m::VAULT_PUT_API_KEY
        | m::VAULT_AUTHORIZE
        | m::VAULT_DISCONNECT
        | m::VAULT_REVOKE_DEVICE
        | m::GITHUB_CONNECT_START
        | m::GITHUB_CONNECT_POLL
        | m::GITHUB_ACCOUNTS
        | m::GITHUB_ACCOUNT_FORGET
        | m::GITHUB_ACCOUNT_ACTIVATE
        | m::LIST_GITHUB_REPOS
        | m::LIST_CLOUD_BRANCHES
        // The phone link.
        | m::PHONE_LINK_STATUS
        | m::SET_PHONE_LINK
        | m::NEW_PHONE_LINK_KEY
        // The account's plugin signing keys (every device's, revoked).
        | m::PLUGIN_SIGNING_KEY
        // The engine's life.
        | m::STOP_ENGINE
        // This engine's own transport and window: delivery to the edge,
        // sync, outbox, focus, the window's plugin channel, the device
        // link between the account's devices, the app's plugin folder.
        | m::RELAY_COMMAND
        | m::RETRY_DELIVERY
        | m::PROBE_SYNC
        | m::SYNC_STATUS
        | m::WATCH_CONNECTIVITY
        | m::WATCH_TRANSFERS
        | m::WATCH_HELD_SENDS
        | m::REMOVE_HELD_SEND
        | m::FOCUS_CHAT
        | m::SET_VIEWPORT_FOCUS
        | m::SERVE_PLUGIN_TOOLS
        | m::PLUGIN_TOOL_RESULT
        | m::PLUGIN_LOG
        | m::PLUGIN_DELIVER
        | m::SERVE_PLUGIN_CALL
        | m::SERVE_PLUGIN_AGENTS
        | m::CHECK_PLUGIN_KEY
        | m::SET_BUILTIN_PLUGINS => Never,

        _ => return None,
    })
}

/// The scopes `method` may be in: of `Mutate`, every op's.
pub fn scopes_of(method: &str) -> Vec<Scope> {
    if method == m::MUTATE {
        MUTATE_OPS.iter().filter_map(|op| mutate_scope(op)).collect()
    } else {
        peer_scope(method, None).into_iter().collect()
    }
}
