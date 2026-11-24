//! `zend peer`: pair this Zend with the owner's other servers and set what
//! each may do here, over the running engine's local IPC (the methods are
//! this device's own: [`zend_engine::peer_link`]).

use serde_json::{Value, json};
use zend_cli::{PeerChangesCommand, PeerCommand, PeerKeyCommand};
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
        PeerCommand::Changes(PeerChangesCommand::List) => {
            let reply = call(methods::LIST_PEER_CHANGES, json!({})).await?;
            let copies = reply["copies"].as_array().cloned().unwrap_or_default();
            if copies.is_empty() {
                println!("No working copies of servers' chats.");
            }
            for copy in copies {
                let server = who(&json!({ "key": copy["peer"], "name": copy["name"] }));
                let place = copy["branch"].as_str().map(|b| format!("branch {b}")).unwrap_or_else(|| "copy".into());
                println!(
                    "{}  server {server}  project {}  {place}  of {}",
                    copy["chat"].as_str().unwrap_or_default(),
                    copy["project"].as_str().unwrap_or_default(),
                    copy["owner"].as_str().unwrap_or_default()
                );
                match copy["changes"].as_array() {
                    Some(changes) if changes.is_empty() => println!("  no changes"),
                    Some(changes) => {
                        for change in changes {
                            println!("  {} {}", change["status"].as_str().unwrap_or("?"), change["path"].as_str().unwrap_or_default());
                        }
                    }
                    None => println!("  can't read it: {}", copy["error"].as_str().unwrap_or_default()),
                }
            }
        }
        PeerCommand::Changes(PeerChangesCommand::Show { chat }) => {
            let reply = call(methods::SHOW_PEER_CHANGES, json!({ "chat": chat })).await?;
            print!("{}", reply["patch"].as_str().unwrap_or_default());
        }
        PeerCommand::Changes(PeerChangesCommand::Apply { chat }) => {
            let reply = call(methods::APPLY_PEER_CHANGES, json!({ "chat": chat })).await?;
            let applied: Vec<&str> = reply["applied"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
            if applied.is_empty() {
                println!("Nothing to apply.");
            } else {
                println!("Applied {} file(s): {}", applied.len(), applied.join(", "));
            }
        }
        PeerCommand::Changes(PeerChangesCommand::Discard { chat }) => {
            call(methods::DISCARD_PEER_CHANGES, json!({ "chat": chat })).await?;
            println!("Discarded the working copy of chat {chat}; its chats no longer run.");
        }
        PeerCommand::Key(PeerKeyCommand::Set { peer, provider }) => {
            use std::io::{IsTerminal as _, Read as _};
            if std::io::stdin().is_terminal() {
                eprintln!("Paste the {provider} API key, then Enter and Ctrl-D:");
            }
            let mut key = String::new();
            std::io::stdin().read_to_string(&mut key)?;
            let reply = call(methods::SET_PEER_KEY, json!({ "peer": peer, "provider": provider, "key": key.trim() })).await?;
            println!(
                "Kept. Chats of {} on {provider} models run with it as {}.",
                reply["peer"].as_str().map(|k| &k[..k.len().min(16)]).unwrap_or_default(),
                reply["variable"].as_str().unwrap_or_default()
            );
        }
        PeerCommand::Key(PeerKeyCommand::Rm { peer, provider }) => {
            let reply = call(methods::REMOVE_PEER_KEY, json!({ "peer": peer, "provider": provider })).await?;
            if reply["removed"].as_bool() == Some(true) {
                println!("Removed; its chats on {provider} models no longer run.");
            } else {
                println!("No {provider} key was kept for that server.");
            }
        }
        PeerCommand::Key(PeerKeyCommand::List) => {
            let reply = call(methods::LIST_PEER_KEYS, json!({})).await?;
            let keys = reply["keys"].as_array().cloned().unwrap_or_default();
            if keys.is_empty() {
                println!("No API keys kept for servers.");
            }
            for row in keys {
                let providers: Vec<&str> = row["providers"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
                println!("{}  {}", who(&json!({ "key": row["peer"], "name": row["name"] })), providers.join(" "));
            }
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
