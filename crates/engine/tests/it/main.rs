//! All of this crate's integration tests in one binary: each top-level
//! tests/*.rs file would be its own binary, linked against the whole crate
//! (see docs/reference/testing.md). Add a suite as tests/it/<name>.rs plus
//! a mod line here. Only a suite that needs a process of its own (process-wide
//! state that plain `cargo test` would share) stays a top-level tests/<name>.rs.

/// The worktrees this binary's engines make go into its own temp folder,
/// never the real `~/.zend/worktrees` (`ZEND_WORKTREES_DIR`, which
/// `Repos::new` reads). Set as the binary loads, before any test thread,
/// so every suite's engines get it without a call of their own.
/// It's removed as the binary exits (`atexit`, a failed run's too), and a
/// killed run's is removed by the next one.
extern "C" fn isolate_worktrees() {
    if std::env::var_os("ZEND_WORKTREES_DIR").is_some_and(|dir| !dir.is_empty()) {
        return;
    }
    let temp = std::env::temp_dir();
    #[cfg(target_os = "linux")]
    for entry in std::fs::read_dir(&temp).into_iter().flatten().flatten() {
        let name = entry.file_name();
        let pid = name.to_str().and_then(|n| n.strip_prefix(WORKTREES_PREFIX)).and_then(|pid| pid.parse::<u32>().ok());
        if pid.is_some_and(|pid| pid != std::process::id() && !std::path::Path::new("/proc").join(pid.to_string()).exists()) {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
    let dir = temp.join(format!("{WORKTREES_PREFIX}{}", std::process::id()));
    // SAFETY: runs as the binary loads, before main and any other thread.
    unsafe {
        std::env::set_var("ZEND_WORKTREES_DIR", &dir);
        atexit(remove_worktrees);
    }
}

const WORKTREES_PREFIX: &str = "zend-it-worktrees-";

unsafe extern "C" {
    fn atexit(callback: extern "C" fn()) -> std::ffi::c_int;
}

/// This binary's worktrees folder gone as it exits.
extern "C" fn remove_worktrees() {
    let dir = std::env::temp_dir().join(format!("{WORKTREES_PREFIX}{}", std::process::id()));
    let _ = std::fs::remove_dir_all(dir);
}

#[used]
#[cfg_attr(any(target_os = "linux", target_os = "android", target_os = "freebsd"), unsafe(link_section = ".init_array"))]
#[cfg_attr(target_os = "macos", unsafe(link_section = "__DATA,__mod_init_func"))]
#[cfg_attr(windows, unsafe(link_section = ".CRT$XCU"))]
static ISOLATE_WORKTREES: extern "C" fn() = isolate_worktrees;

/// The temp folder [`isolate_worktrees`] gave this binary's worktrees.
#[test]
fn worktrees_stay_out_of_the_real_home() {
    let dir = std::env::var_os("ZEND_WORKTREES_DIR").expect("set as the binary loads");
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from).unwrap_or_default();
    assert!(!std::path::Path::new(&dir).starts_with(home.join(".zend")), "{dir:?}");
}

mod misc;
mod acp_registry_rpc;
mod adapter_chaos_live;
mod auth;
mod big_remote_chats;
mod branch_naming;
mod born_chat2_race;
mod child_events;
mod cloud_attachments;
mod cloud_killed_turn;
mod cloud_rpc;
mod cloud_runner;
mod cloud_sleep_quiesce;
mod cloud_wake_update;
mod codex_catalog;
mod codex_login_resolution;
mod codex_login_resolver;
mod cursor_live;
mod delete_ends_run;
mod device_routing;
mod diff_sync_churn;
mod edit_fork;
mod e2e;
mod local_first;
mod local_import;
mod m5_repos_diffs_terminals;
mod m5c_accounts_uploads_titles;
mod mcp_endpoint;
mod message_queue;
mod peer_grants;
mod peer_window;
mod phone_reads;
mod plugin_services;
mod previews;
mod project_action_commands;
mod projectless;
mod provider_switch;
mod provider_switch_live;
mod queue_attachments;
mod queue_order_live;
mod queued_attachments;
mod registry_login_tunnel;
mod registry_writes_live;
mod relay_delivery;
mod remote_scope;
mod restart_resume;
mod rich_composer_delivery;
mod run_recovery;
mod runtime_survival;
mod runtime_survival_live;
mod sandboxed_chats;
mod self_continued_quiesce;
mod session_publication;
mod side_chats;
mod steer_now;
mod steer_now_live;
mod steer_tool_live;
mod steering_live;
mod subagent_idle_reap;
mod sync_resources;
mod terminal_link_p2p;
mod terminal_query_order;
mod transcript_salvage;
mod turn_quiesce;
mod unarchive_on_send;
mod voice_runtime;
mod workspace_files;
mod workspace_sync;
mod worktree_on_run;
mod agent_lifecycle;
mod launch_agents;
mod warm_start;
mod agents_die_with_engine;
mod worker;
mod worker_window;
