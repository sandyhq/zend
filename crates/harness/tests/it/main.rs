//! All of this crate's integration tests in one binary: each top-level
//! tests/*.rs file would be its own binary, linked against the whole crate
//! (see docs/reference/testing.md). Add a suite as tests/it/<name>.rs plus
//! a mod line here. Only a suite that needs a process of its own (process-wide
//! state that plain `cargo test` would share) stays a top-level tests/<name>.rs.

mod misc;
mod acp;
mod acp_quiet;
mod acp_registry;
mod acp_registry_live;
mod acp_registry_sign_in;
mod acp_stall;
mod acp_tool_fixture;
mod claude;
mod claude_refresh_lock;
mod codex;
mod codex_availability;
mod cursor;
mod cursor_shim;
mod managed_install;
mod managed_install_failure;
mod muse;
mod muse_account_live;
mod opencode;
mod pi_live;
mod pi_rpc;
mod real_acp_lifecycle;
mod real_quiet_survey;
mod sandbox;
mod shell_env_resolution;
mod windows_native;
