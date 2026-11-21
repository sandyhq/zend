//! What the `zend` app/CLI and the `zend-engine` background engine share
//! that needs the engine: its env-resolved configuration, the allocator, the
//! MCP bridge agents run, the background engine's entry point, and the
//! subcommands that run the engine's code here (`zend_cli::EngineCommand`).
//! The command line itself, paths, logging and the rest are `zend_cli`'s.

#[cfg(target_os = "macos")]
pub mod allocator;
pub mod auth_cli;
pub mod background;
pub mod engine_commands;
pub mod ipc_cli;
pub mod plugin_cli;

pub use engine_commands::run_engine_command;
pub use zend_cli::{
    DEFAULT_EDGE_URL, DEFAULT_WORKOS_CLIENT_ID, edge_token_from_env, edge_url_from_env, init_logging,
    long_running_setup, open_log_file, paths, workos_client_id_from_env,
};

/// mimalloc, macOS only: libmalloc never returns the streaming churn's
/// high-water pages, so transient allocation became permanent RSS
/// (docs/memory-plan.md §1). Pinned to mimalloc v2 in the workspace manifest —
/// the crate's default v3 has the same pathology (churn retained as permanent
/// RSS, ~6x glibc's growth on identical workloads, no idle recovery). Linux
/// measured flat on glibc, so it keeps the system allocator. Its regions are
/// tagged as heap, not GPU memory ([`allocator`]).
#[cfg(target_os = "macos")]
#[global_allocator]
static ALLOC: allocator::Allocator = allocator::Allocator;

/// The env-resolved engine configuration shared by `headless`, `login`,
/// `logout`, and `status` — one resolution so the CLI auth commands always
/// operate on the exact session the daemon will load.
pub fn engine_config_from_env() -> zend_engine::EngineConfig {
    // (Every way an engine starts here resolves its configuration first.)
    #[cfg(target_os = "macos")]
    zend_engine::heap::set_trim(allocator::trim);
    // Dev-mode bearer (no WorkOS): an explicit token enables sync.
    let edge_token = edge_token_from_env();
    // The edge bearer is the engine's alone: read, then out of the
    // environment every agent, terminal and hook it starts would inherit
    // (every entry, headless included, builds its config here first).
    // SAFETY: called at start, before any runtime thread reads the env.
    unsafe {
        std::env::remove_var("ZEND_EDGE_TOKEN");
        std::env::remove_var("ZEND_EDGE_TOKEN_FILE");
    }
    zend_engine::EngineConfig {
        data_dir: paths::data_dir(),
        edge_url: edge_url_from_env(),
        ipc_port: zend_cli::ipc_port_from_env(),
        default_harness: harness_from_env(),
        // WorkOS mode: the signed-in session's org wins; ZEND_ORG_ID (dev
        // default "dev-org") scopes the workspace room otherwise.
        org_id: std::env::var("ZEND_ORG_ID").ok(),
        // Real auth against production by default; see
        // `workos_client_id_from_env` for the dev-mode escape hatches.
        workos_client_id: workos_client_id_from_env(&edge_token),
        edge_token,
    }
}

/// `ZEND_HARNESS` (kebab-case id) picks the default harness for chats without a
/// config row — `mock` powers the e2e smoke; default `claude-code`.
pub fn harness_from_env() -> zend_engine::HarnessId {
    match std::env::var("ZEND_HARNESS").as_deref().map(str::trim) {
        Ok("mock") => zend_engine::HarnessId::Mock,
        Ok("codex") => zend_engine::HarnessId::Codex,
        Ok("cursor") => zend_engine::HarnessId::Cursor,
        Ok("devin") => zend_engine::HarnessId::Devin,
        Ok("grok") => zend_engine::HarnessId::Grok,
        Ok("hermes") => zend_engine::HarnessId::Hermes,
        Ok("pi") => zend_engine::HarnessId::Pi,
        Ok("antigravity") => zend_engine::HarnessId::Antigravity,
        Ok("muse") => zend_engine::HarnessId::Muse,
        _ => zend_engine::HarnessId::ClaudeCode,
    }
}

/// `zend mcp`: one bridge per agent session, and it only relays — stdio
/// requests out to the engine over IPC and replies back. Two workers keep a
/// burst of parallel tool calls (large replies to encode) close to per-core
/// speed without a worker per core in every agent's bridge.
pub fn run_mcp() -> anyhow::Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    let mut config = zend_mcp::McpConfig::from_env();
    // The engine's IPC bearer lives in its data dir (the injected
    // ZEND_IPC_TOKEN_FILE wins when present).
    config.data_dir.get_or_insert_with(paths::data_dir);
    runtime.block_on(zend_mcp::run(config))
}
