//! `zend worker`: this server's worker containers, over the running
//! engine's local IPC (the methods are this device's own:
//! [`zend_engine::workers`]).

use serde_json::{Value, json};
use zend_cli::{WorkerCommand, terminal_safe};
use zend_rpc::methods;

pub async fn run(command: WorkerCommand, ipc_port: u16, data_dir: &std::path::Path) -> anyhow::Result<()> {
    let client = zend_rpc::connect_ipc(ipc_port, Some(data_dir))
        .await
        .map_err(|e| anyhow::anyhow!("no Zend running here (127.0.0.1:{ipc_port}): {e}"))?;
    let call = |method: &'static str, params: Value| {
        let client = &client;
        async move { client.call(method, params).await.map_err(|e| anyhow::anyhow!(terminal_safe(&e.to_string(), true))) }
    };
    match command {
        WorkerCommand::Create { name, image, key } => {
            let mut keys = serde_json::Map::new();
            if let Some(provider) = key {
                use std::io::{IsTerminal as _, Read as _};
                if std::io::stdin().is_terminal() {
                    eprintln!("Paste the {provider} API key, then Enter and Ctrl-D:");
                }
                let mut value = String::new();
                std::io::stdin().read_to_string(&mut value)?;
                keys.insert(provider, value.trim().into());
            }
            let reply = call(methods::CREATE_WORKER, json!({ "name": name, "image": image, "keys": keys })).await?;
            println!(
                "Worker {} runs ({}), key {}.",
                reply["name"].as_str().unwrap_or_default(),
                reply["container"].as_str().unwrap_or_default(),
                reply["key"].as_str().unwrap_or_default()
            );
        }
        WorkerCommand::List => {
            let reply = call(methods::LIST_WORKERS, json!({})).await?;
            let workers = reply["workers"].as_array().cloned().unwrap_or_default();
            if workers.is_empty() {
                println!("No workers.");
            }
            for w in workers {
                let providers: Vec<&str> = w["providers"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
                let key = w["key"].as_str().unwrap_or_default();
                println!(
                    "{}  {}  {}  key {}…  {}",
                    w["name"].as_str().unwrap_or_default(),
                    terminal_safe(w["state"].as_str().unwrap_or("?"), false),
                    w["image"].as_str().unwrap_or_default(),
                    &key[..key.len().min(16)],
                    if providers.is_empty() { "no API keys".to_string() } else { providers.join(" ") }
                );
            }
        }
        WorkerCommand::Start { name } => {
            let reply = call(methods::START_WORKER, json!({ "name": name })).await?;
            println!("Worker {name}: {}.", reply["state"].as_str().unwrap_or("?"));
        }
        WorkerCommand::Rm { name, keep_volumes } => {
            call(methods::REMOVE_WORKER, json!({ "name": name, "keepVolumes": keep_volumes })).await?;
            println!("Removed worker {name}{}.", if keep_volumes { " (its volumes kept)" } else { " and its volumes" });
        }
    }
    Ok(())
}
