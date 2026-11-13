//! zend — headed by default; `zend headless` runs the engine alone. Both start
//! local-only without credentials. `zend login` and `zend logout` select the
//! profile used by the next engine start without mutating a live runtime.
//!
//! The command line is `zend_cli`'s (shared with the engine helper and the
//! zgx interface). This binary links the engine, so it runs the engine's
//! subcommands itself; with no subcommand it opens the window.

#![cfg_attr(windows, windows_subsystem = "windows")]

use zend_app::{edge_token_from_env, edge_url_from_env, paths, workos_client_id_from_env};
use zend_cli::{Engines, Launch};

fn main() -> anyhow::Result<()> {
    match zend_cli::run(zend_update::current_version(), Engines::InProcess) {
        Launch::Done(result) => result,
        Launch::Engine(command) => zend_app::run_engine_command(command),
        #[cfg(target_os = "linux")]
        Launch::Appshot => {
            zend_ui::appshots::request_running_appshot(&zend_app::engine_config_from_env().data_dir)
                .map_err(anyhow::Error::msg)
        }
        Launch::Window { open_url, .. } => {
            // zend's window is the zgx app beside this binary, when it's there.
            #[cfg(unix)]
            if let Some(app) = std::env::current_exe().ok().and_then(|exe| exe.parent().map(|dir| dir.join("zend-zgx"))).filter(|app| app.exists()) {
                use std::os::unix::process::CommandExt;
                let mut command = std::process::Command::new(&app);
                if let Some(url) = &open_url {
                    command.arg(url);
                }
                return Err(anyhow::Error::from(command.exec()).context(format!("starting {}", app.display())));
            }
            let edge_token = edge_token_from_env();
            // Headed: the UI probes ZEND_IPC_PORT and connects to a running
            // daemon, or embeds the engine in-process (ARCHITECTURE §1).
            zend_ui::run_app(zend_ui::UiConfig {
                data_dir: paths::data_dir(),
                ipc_port: zend_cli::ipc_port_from_env(),
                edge_url: edge_url_from_env(),
                workos_client_id: workos_client_id_from_env(&edge_token),
                edge_token,
                org_id: std::env::var("ZEND_ORG_ID").ok(),
                default_harness: zend_ui::HarnessId::ClaudeCode,
                initial_url: open_url,
            });
            Ok(())
        }
    }
}
