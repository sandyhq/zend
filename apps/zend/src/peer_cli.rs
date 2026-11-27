//! `zend peer`: pair this Zend with the owner's other servers and set what
//! each may do here, over the running engine's local IPC (the methods are
//! this device's own: [`zend_engine::peer_link`]).

use serde_json::{Value, json};
use zend_cli::{PeerChangesCommand, PeerCommand, PeerKeyCommand, PeerLoginCommand, terminal_safe};
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
            let parsed = grants.iter().map(|g| g.parse::<zend_rpc::Grant>()).collect::<Result<Vec<_>, _>>().map_err(|e| anyhow::anyhow!(e))?;
            let wide: Vec<String> = parsed.iter().filter(|g| g.is_machine_wide()).map(ToString::to_string).collect();
            if !wide.is_empty() {
                eprintln!(
                    "Warning: {} gives that server the run of this machine as you: any file, a shell, agents started with your logins{}. Unpair it (zend peer unpair) if its key is ever lost.",
                    wide.join(", "),
                    if parsed.contains(&zend_rpc::Grant::Full) { ", every chat here and every project, unconfined" } else { "" }
                );
            }
            let reply = call(methods::SET_PEER_GRANTS, json!({ "peer": peer, "grants": grants })).await?;
            println!("{} may now: {}", who(&reply["peer"]), if grants.is_empty() { "nothing".into() } else { grants.join(" ") });
        }
        PeerCommand::Unpair { peer } => {
            let reply = call(methods::UNPAIR_PEER, json!({ "peer": peer })).await?;
            println!("Unpaired {}.", reply["removed"].as_str().unwrap_or_default());
        }
        // What a server's agent wrote (paths, contents, errors naming them)
        // reaches the terminal escaped: terminal_safe.
        PeerCommand::Changes(PeerChangesCommand::List) => {
            let reply = safe(call(methods::LIST_PEER_CHANGES, json!({})).await)?;
            let copies = reply["copies"].as_array().cloned().unwrap_or_default();
            if copies.is_empty() {
                println!("No working copies of servers' chats.");
            }
            let s = |v: &Value| terminal_safe(v.as_str().unwrap_or_default(), false);
            for copy in copies {
                let server = terminal_safe(&who(&json!({ "key": copy["peer"], "name": copy["name"] })), false);
                println!("{}  server {server}  project {}  of {}", s(&copy["chat"]), s(&copy["project"]), s(&copy["owner"]));
                match copy["changes"].as_array() {
                    Some(changes) if changes.is_empty() => println!("  no changes"),
                    Some(changes) => {
                        for change in changes {
                            print_change(change);
                        }
                        println!("  tree {}", s(&copy["tree"]));
                    }
                    None => println!("  can't read it: {}", s(&copy["error"])),
                }
            }
        }
        PeerCommand::Changes(PeerChangesCommand::Show { chat }) => {
            let reply = safe(call(methods::SHOW_PEER_CHANGES, json!({ "chat": chat })).await)?;
            print!("{}", terminal_safe(reply["patch"].as_str().unwrap_or_default(), true));
            // The summary on stderr: the patch on stdout stays a patch.
            let changes = reply["changes"].as_array().cloned().unwrap_or_default();
            let tree = terminal_safe(reply["tree"].as_str().unwrap_or_default(), false);
            if changes.is_empty() {
                eprintln!("No changes (tree {tree}).");
            } else {
                eprintln!("{} change(s), tree {tree}:", changes.len());
                for change in &changes {
                    print_change_to(change, true);
                }
                let allow: String = changes
                    .iter()
                    .filter(|c| c["flag"].is_string() && !c["refused"].is_string())
                    .filter_map(|c| c["path"].as_str())
                    .map(|p| format!(" --allow '{}'", terminal_safe(p, false)))
                    .collect();
                eprintln!("Apply exactly this: zend peer changes apply {} {tree}{allow}", terminal_safe(&chat, false));
            }
        }
        PeerCommand::Changes(PeerChangesCommand::Apply { chat, tree, allow }) => {
            let reply = safe(call(methods::APPLY_PEER_CHANGES, json!({ "chat": chat, "tree": tree, "allow": allow })).await)?;
            let applied: Vec<String> =
                reply["applied"].as_array().into_iter().flatten().filter_map(Value::as_str).map(|p| terminal_safe(p, false)).collect();
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
        PeerCommand::Login(PeerLoginCommand::Allow { peer, provider }) => {
            let reply = call(methods::ALLOW_PEER_LOGIN, json!({ "peer": peer, "provider": provider })).await?;
            let server = reply["peer"].as_str().map(|k| &k[..k.len().min(16)]).unwrap_or_default().to_string();
            println!(
                "Allowed. Chats of {server}… on {provider} models run on this device's {provider} login: each turn gets its access token only, never the refresh token."
            );
            if reply["keyWins"].as_bool() == Some(true) {
                println!("An API key is kept for it and {provider}: the key wins over the login (zend peer key rm to use the login).");
            }
            if reply["hostSignedIn"].as_bool() == Some(false) {
                eprintln!("This device's pi has no {provider} login yet: sign in (pi, /login) or its chats are refused.");
            }
        }
        PeerCommand::Login(PeerLoginCommand::Rm { peer, provider }) => {
            let reply = call(methods::REMOVE_PEER_LOGIN, json!({ "peer": peer, "provider": provider })).await?;
            if reply["removed"].as_bool() == Some(true) {
                println!("Taken back; its chats on {provider} models no longer run on this device's login, and the token left their sandboxes.");
            } else {
                println!("That server wasn't allowed this device's {provider} login.");
            }
        }
        PeerCommand::Login(PeerLoginCommand::List) => {
            let reply = call(methods::LIST_PEER_LOGINS, json!({})).await?;
            let logins = reply["logins"].as_array().cloned().unwrap_or_default();
            if logins.is_empty() {
                println!("No server may use this device's logins.");
            }
            for row in logins {
                let providers: Vec<&str> = row["providers"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
                println!("{}  {}", who(&json!({ "key": row["peer"], "name": row["name"] })), providers.join(" "));
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

/// A reply's error, escaped for the terminal (it can name the copy's files).
fn safe(reply: anyhow::Result<Value>) -> anyhow::Result<Value> {
    reply.map_err(|e| anyhow::anyhow!(terminal_safe(&format!("{e:#}"), true)))
}

fn print_change(change: &Value) {
    print_change_to(change, false);
}

/// One changed path, with why it is refused or needs --allow.
fn print_change_to(change: &Value, stderr: bool) {
    let path = terminal_safe(change["path"].as_str().unwrap_or_default(), false);
    let mut line = format!("  {} {path}", change["status"].as_str().unwrap_or("?"));
    if change["modeOnly"].as_bool() == Some(true) {
        line.push_str("  (mode only)");
    }
    if let Some(why) = change["refused"].as_str() {
        line.push_str(&format!("  [refused: {}]", terminal_safe(why, false)));
    } else if change["flag"].is_string() {
        line.push_str(&format!("  [{}: --allow {path}]", "instructs or configures your tools"));
    }
    if stderr { eprintln!("{line}") } else { println!("{line}") }
}

fn who(peer: &Value) -> String {
    let key = peer["key"].as_str().unwrap_or_default();
    let short = &key[..key.len().min(16)];
    match peer["name"].as_str() {
        Some(name) => format!("{name} ({short}…)"),
        None => format!("{short}…"),
    }
}
