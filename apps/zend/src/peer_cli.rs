//! `zend peer`: pair this Zend with the owner's other servers and set what
//! each may do here, over the running engine's local IPC (the methods are
//! this device's own: [`zend_engine::peer_link`]).

use serde_json::{Value, json};
use zend_cli::PeerCommand;
use zend_rpc::methods;

pub async fn run(command: PeerCommand, ipc_port: u16, data_dir: &std::path::Path) -> anyhow::Result<()> {
    let client = zend_rpc::connect_ipc(ipc_port, Some(data_dir))
        .await
        .map_err(|e| anyhow::anyhow!("no Zend running here (127.0.0.1:{ipc_port}): {e}"))?;
    let call = |method: &'static str, params: Value| {
        let client = &client;
        async move { client.call(method, params).await.map_err(|e| anyhow::anyhow!("{e}")) }
    };
    match command {
        PeerCommand::Pair => {
            let reply = call(methods::PEER_PAIR, json!({})).await?;
            println!("{}", reply["code"].as_str().unwrap_or_default());
            eprintln!(
                "On the other server: zend peer join <code> (good once, for 10 minutes). This server listens on {}:{}.",
                reply["host"].as_str().unwrap_or("?"),
                reply["port"]
            );
        }
        PeerCommand::Join { code } => {
            let reply = call(methods::PEER_JOIN, json!({ "code": code })).await?;
            println!("Paired with {}. It may do nothing here until you grant it something (zend peer grant).", who(&reply["peer"]));
        }
        PeerCommand::List => {
            let reply = call(methods::LIST_PEERS, json!({})).await?;
            println!("This server: {}", reply["key"].as_str().unwrap_or_default());
            match reply["listening"].as_array().filter(|l| !l.is_empty()) {
                Some(addrs) => println!("Listening on {}", addrs.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", ")),
                None => println!("Not listening{}", reply["error"].as_str().map(|e| format!(" ({e})")).unwrap_or_default()),
            }
            let peers = reply["peers"].as_array().cloned().unwrap_or_default();
            if peers.is_empty() {
                println!("No paired servers.");
            }
            for peer in peers {
                let grants: Vec<&str> = peer["grants"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
                println!("{}  grants: {}", who(&peer), if grants.is_empty() { "none".into() } else { grants.join(" ") });
            }
        }
        PeerCommand::Grant { peer, grants } => {
            let grants: Vec<String> = grants.into_iter().filter(|g| g != "none").collect();
            let reply = call(methods::SET_PEER_GRANTS, json!({ "peer": peer, "grants": grants })).await?;
            println!("{} may now: {}", who(&reply["peer"]), if grants.is_empty() { "nothing".into() } else { grants.join(" ") });
        }
        PeerCommand::Unpair { peer } => {
            let reply = call(methods::UNPAIR_PEER, json!({ "peer": peer })).await?;
            println!("Unpaired {}.", reply["removed"].as_str().unwrap_or_default());
        }
    }
    Ok(())
}

fn who(peer: &Value) -> String {
    let key = peer["key"].as_str().unwrap_or_default();
    let short = &key[..key.len().min(16)];
    match peer["name"].as_str() {
        Some(name) => format!("{name} ({short}…)"),
        None => format!("{short}…"),
    }
}
