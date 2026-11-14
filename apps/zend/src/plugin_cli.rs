//! `zend plugin …`: make, sign, check and list plugins through the running
//! Zend (the same local connection `zend mcp` uses). `zend template …`
//! does the same for templates (whole replacements of Zend's interface).

use serde_json::{Value, json};
use zend_cli::{PluginCommand, TemplateCommand};
use zend_rpc::methods;

async fn connect(
    ipc_port: u16,
    data_dir: &std::path::Path,
) -> anyhow::Result<zend_rpc::RpcClient> {
    zend_rpc::connect_ipc(ipc_port, Some(data_dir))
        .await
        .map_err(|_| {
            anyhow::anyhow!("Zend isn't running here. Open it, or start it with `zend headless`.")
        })
}

async fn call(
    client: &zend_rpc::RpcClient,
    method: &'static str,
    params: Value,
) -> anyhow::Result<Value> {
    client
        .call(method, params)
        .await
        .map_err(|e| anyhow::anyhow!("{}", plain(e)))
}

pub async fn run_template(
    command: TemplateCommand,
    ipc_port: u16,
    data_dir: &std::path::Path,
) -> anyhow::Result<()> {
    use zend_engine::plugins::{TEMPLATES_DIR, templates};
    // The escape hatch: putting Zend's own back needs no running Zend and
    // nothing set up; the app picks up `.active` when it next starts.
    let client = match connect(ipc_port, data_dir).await {
        Ok(client) => client,
        Err(_) if matches!(&command, TemplateCommand::Use { id } if id.trim() == templates::ZEND) =>
        {
            templates::use_template(&data_dir.join(TEMPLATES_DIR), templates::ZEND, None)
                .map_err(anyhow::Error::msg)?;
            println!("Using Zend's own interface.");
            return Ok(());
        }
        Err(error) => return Err(error),
    };
    match command {
        TemplateCommand::List => {
            let list = call(&client, methods::LIST_TEMPLATES, json!({})).await?;
            for item in list["templates"].as_array().cloned().unwrap_or_default() {
                println!("{}", template_line(&item));
            }
            println!("Folder: {}", str_of(&list, "dir"));
        }
        TemplateCommand::Use { id } => {
            let reply = call(&client, methods::USE_TEMPLATE, json!({ "id": id })).await?;
            match str_of(&reply, "active") {
                templates::ZEND => println!("Using Zend's own interface."),
                active => println!("Using {active}."),
            }
        }
        TemplateCommand::Create { id, name, from } => {
            let mut params = json!({ "id": id, "name": name.unwrap_or_else(|| id.clone()) });
            if let Some(from) = from {
                params["from"] = json!(from);
            }
            let reply = call(&client, methods::CREATE_TEMPLATE, params).await?;
            let files = reply["files"].as_array().map_or(0, Vec::len);
            println!(
                "Made {} in {} ({files} file{}).",
                str_of(&reply, "id"),
                str_of(&reply, "dir"),
                if files == 1 { "" } else { "s" }
            );
            println!("Use it with `zend template use {}`.", str_of(&reply, "id"));
        }
        TemplateCommand::Status { id } => {
            let reply = call(&client, methods::TEMPLATE_STATUS, json!({ "id": id })).await?;
            let files = reply["files"].as_u64().unwrap_or(0);
            let inherits = reply["inherits"].as_u64().unwrap_or(0);
            println!(
                "{}{}: {files} file{}, {inherits} inherited from Zend.",
                str_of(&reply, "id"),
                if reply["active"] == true {
                    " (in use)"
                } else {
                    ""
                },
                if files == 1 { "" } else { "s" },
            );
            if let Some(error) = reply["root_error"].as_str() {
                println!("Broken: {error}");
            }
            if let Some(errors) = reply["file_errors"].as_object() {
                for (rel, error) in errors {
                    println!(
                        "Error in {rel} (Zend's own shows instead): {}",
                        error.as_str().unwrap_or("")
                    );
                }
            }
            for rel in reply["out_of_date"].as_array().cloned().unwrap_or_default() {
                println!(
                    "Out of date: {} (Zend's own has changed since it was copied)",
                    rel.as_str().unwrap_or("")
                );
            }
        }
        TemplateCommand::Delete { id } => {
            let reply = call(&client, methods::DELETE_TEMPLATE, json!({ "id": id })).await?;
            println!("Deleted {}.", str_of(&reply, "deleted"));
            if str_of(&reply, "active") == templates::ZEND {
                println!("Using Zend's own interface.");
            }
        }
    }
    Ok(())
}

/// `midnight  Midnight 1.0.0  in use  errors (ds/tokens.luau: …)`.
fn template_line(item: &Value) -> String {
    let mut line = format!(
        "{}  {} {}",
        str_of(item, "id"),
        str_of(item, "name"),
        str_of(item, "version")
    )
    .trim_end()
    .to_owned();
    if item["active"] == true {
        line.push_str("  in use");
    }
    let status = str_of(item, "status");
    line.push_str(&format!("  {}", status.replace('_', " ")));
    if let Some(error) = item["error"].as_str() {
        line.push_str(&format!(" ({error})"));
    }
    line
}

pub async fn run(
    command: PluginCommand,
    ipc_port: u16,
    data_dir: &std::path::Path,
) -> anyhow::Result<()> {
    // Checking needs no Zend: it runs here.
    if let PluginCommand::Check { target, strict, json } = command {
        return check(&target, strict, json, data_dir);
    }
    let client = connect(ipc_port, data_dir).await?;
    let call = |method: &'static str, params: Value| call(&client, method, params);
    match command {
        PluginCommand::New {
            id,
            name,
            permissions,
        } => {
            let item = call(
                methods::NEW_PLUGIN,
                json!({ "id": id, "name": name.unwrap_or_else(|| id.clone()), "permissions": permissions }),
            )
            .await?;
            println!("Made {} in {}.", str_of(&item, "id"), str_of(&item, "dir"));
            println!("{}", line(&item));
        }
        PluginCommand::Sign { target } => {
            let path = std::path::Path::new(&target);
            let params = if target.contains('/') || path.is_dir() && !looks_like_id(&target) {
                let absolute = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
                json!({ "path": absolute.display().to_string() })
            } else {
                json!({ "id": target })
            };
            let reply = call(methods::SIGN_PLUGIN, params).await?;
            let files = reply["files"].as_u64().unwrap_or(0);
            let name = reply["id"].as_str().map(str::to_owned).unwrap_or(target);
            println!(
                "Signed {name} as {} ({files} file{}).",
                author(&reply["author"]),
                if files == 1 { "" } else { "s" }
            );
        }
        PluginCommand::Verify { path } => {
            let absolute = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone().into());
            let reply = call(
                methods::VERIFY_PLUGIN,
                json!({ "path": absolute.display().to_string() }),
            )
            .await?;
            let status = str_of(&reply, "status");
            let by = author(&reply["author"]);
            match status {
                "ok" => println!("Signed by {by}. Its files match."),
                "unsigned" => println!("It isn't signed."),
                "tampered" => println!(
                    "{}.",
                    reply["author"]["reason"]
                        .as_str()
                        .unwrap_or("Its files don't match its signature")
                ),
                _ => println!("It's broken: {}", str_of(&reply, "error")),
            }
            if let Some(reason) = reply["author"]["reason"].as_str()
                && status == "ok"
            {
                println!("{reason}.");
            }
            if status != "ok" {
                std::process::exit(1);
            }
        }
        PluginCommand::Check { .. } => unreachable!("checked above, without Zend"),
        PluginCommand::List => {
            let list = call(methods::LIST_PLUGINS, json!({})).await?;
            let plugins = list["plugins"].as_array().cloned().unwrap_or_default();
            if plugins.is_empty() {
                println!("No plugins yet. Make one with `zend plugin new <id>`.");
            }
            for item in &plugins {
                println!("{}", line(item));
            }
        }
        PluginCommand::Export { id, to } => {
            // `to` is already a full path (the parser made it one).
            let mut params = json!({ "id": id });
            if let Some(to) = to {
                params["to"] = json!(to.display().to_string());
            }
            let reply = call(methods::EXPORT_PLUGIN, params).await?;
            println!(
                "Saved {id} to {} ({}).",
                str_of(&reply, "path"),
                size(reply["bytes"].as_u64().unwrap_or(0))
            );
            println!("Share that file: anyone installs it from the file or a link to it.");
        }
        PluginCommand::Preview { target, stop } => {
            let path = std::path::Path::new(&target);
            let mut params = if target.contains('/') || path.is_dir() && !looks_like_id(&target) {
                let absolute = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
                json!({ "path": absolute.display().to_string() })
            } else {
                json!({ "id": target })
            };
            if stop {
                params["stop"] = json!(true);
                let reply = call(methods::PLUGIN_PREVIEW, params).await?;
                let id = str_of(&reply, "id");
                if reply["stopped"] == true {
                    println!("Stopped the preview of {id}.");
                } else {
                    println!("No preview of {id} ran.");
                }
                return Ok(());
            }
            let reply = call(methods::PLUGIN_PREVIEW, params).await?;
            let id = str_of(&reply, "id");
            if reply["started"] == true {
                println!("Previewing {id} from {} (its own engine on port {}, a window that never shows).", str_of(&reply, "source"), reply["port"]);
            } else {
                println!("Copied {} into the preview of {id} again.", str_of(&reply, "source"));
            }
            println!("Its data: {}", str_of(&reply, "data_dir"));
            println!("Look: `zend plugin logs {id} --preview`, MCP plugin_status/plugin_input with preview. Stop: `zend plugin preview {id} --stop`.");
        }
        PluginCommand::Logs { id, follow, lines, preview } => {
            if preview {
                if follow {
                    anyhow::bail!("--follow reads your log only: ask again for the preview's");
                }
                let reply = call(methods::PLUGIN_LOGS, json!({ "id": id, "limit": lines.clamp(1, 500), "preview": true })).await?;
                for item in reply["lines"].as_array().cloned().unwrap_or_default() {
                    print!("{}", log_line(&item));
                }
                return Ok(());
            }
            let reply = call(methods::PLUGIN_LOGS, json!({ "id": id, "limit": lines.clamp(1, 500) })).await?;
            let shown = reply["lines"].as_array().cloned().unwrap_or_default();
            if shown.is_empty() && !follow {
                println!("{id} hasn't logged anything since Zend started.");
            }
            let mut last = 0;
            for item in &shown {
                print!("{}", log_line(item));
                last = last.max(item["seq"].as_u64().unwrap_or(0));
            }
            if !follow {
                if let Some(file) = reply["file"].as_str().filter(|_| !shown.is_empty()) {
                    println!("(Earlier lines: {file})");
                }
                return Ok(());
            }
            let mut stream = client
                .subscribe_checked(methods::WATCH_PLUGIN_LOGS, json!({ "id": id }))
                .await
                .map_err(|e| anyhow::anyhow!("{}", plain(e)))?;
            // Lines between the read and the watch: once more, after `last`.
            let caught = call(methods::PLUGIN_LOGS, json!({ "id": id, "since": last, "limit": 500 })).await?;
            for item in caught["lines"].as_array().cloned().unwrap_or_default() {
                print!("{}", log_line(&item));
                last = last.max(item["seq"].as_u64().unwrap_or(0));
            }
            while let Some(item) = stream.recv().await {
                if let Some(missed) = item["missed"].as_u64() {
                    println!("({missed} lines missed: too many at once)");
                    continue;
                }
                let line = &item["line"];
                if line["seq"].as_u64().unwrap_or(0) > last {
                    print!("{}", log_line(line));
                    last = line["seq"].as_u64().unwrap_or(last);
                }
            }
            anyhow::bail!("Zend stopped.");
        }
    }
    Ok(())
}

/// `17:04:00 window error kaputt`, a traceback's lines indented under it.
fn log_line(item: &Value) -> String {
    let at = str_of(item, "at");
    // The time of day (the engine's is this device's zone):
    // `2026-11-14T17:04:00.123+01:00` → `17:04:00`.
    let time = at.split_once('T').and_then(|(_, t)| t.get(..8)).unwrap_or(at);
    let mut out = format!("{time} {} {}", str_of(item, "source"), str_of(item, "level"));
    for (i, part) in str_of(item, "text").lines().enumerate() {
        out.push_str(if i == 0 { " " } else { "\n    " });
        out.push_str(part);
    }
    out.push('\n');
    out
}

/// `512 bytes`, `12.3 KB`, `4.1 MB`.
fn size(bytes: u64) -> String {
    match bytes {
        1 => "1 byte".into(),
        0..1024 => format!("{bytes} bytes"),
        1024..1_048_576 => format!("{:.1} KB", bytes as f64 / 1024.0),
        _ => format!("{:.1} MB", bytes as f64 / 1_048_576.0),
    }
}

/// `zend plugin check`: its findings, one a line, and the exit code 1
/// when one is an error.
fn check(target: &str, strict: bool, as_json: bool, data_dir: &std::path::Path) -> anyhow::Result<()> {
    use zend_engine::plugins::check;
    let in_folder = data_dir.join(zend_engine::plugins::PLUGINS_DIR).join(target);
    let dir = if looks_like_id(target) && in_folder.is_dir() {
        in_folder
    } else {
        std::path::absolute(target)?
    };
    let options = check::Options { strict, data_dir: Some(data_dir.to_path_buf()), ..Default::default() };
    let report = check::check_plugin(&dir, &options).map_err(anyhow::Error::msg)?;
    if as_json {
        let mut out = serde_json::to_value(&report)?;
        out["ok"] = json!(report.ok());
        out["summary"] = json!(report.summary());
        println!("{}", serde_json::to_string_pretty(&out)?);
    } else {
        for finding in &report.findings {
            let mut text = finding.to_string();
            text = text.replace('\n', "\n    ");
            // Paths a terminal (or an editor) opens: from where you are.
            println!("{}", shown_path(&dir, &text));
        }
        if let Some(note) = &report.types_note {
            eprintln!("{note}");
        }
        println!("{}", report.summary());
    }
    if !report.ok() {
        use std::io::Write;
        let _ = std::io::stdout().flush();
        std::process::exit(1);
    }
    Ok(())
}

/// A finding's line with its file as a path from the current folder (or a
/// full one), not only the plugin's.
fn shown_path(dir: &std::path::Path, line: &str) -> String {
    let here = std::env::current_dir().ok();
    let base = here
        .as_deref()
        .and_then(|here| dir.strip_prefix(here).ok())
        .map(|rel| rel.to_path_buf())
        .unwrap_or_else(|| dir.to_path_buf());
    if base.as_os_str().is_empty() {
        line.to_owned()
    } else {
        format!("{}/{line}", base.display())
    }
}

fn looks_like_id(text: &str) -> bool {
    !text.is_empty()
        && text
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn str_of<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or("")
}

/// `you`, `@ada`, or `someone unknown`.
fn author(author: &Value) -> String {
    match str_of(author, "kind") {
        "you" => "you".into(),
        "verified" => match author["username"].as_str() {
            Some(name) => format!("@{name}"),
            None => "a verified author".into(),
        },
        _ => "someone unknown".into(),
    }
}

/// `focus 1.2.0  by @ada  on`.
fn line(item: &Value) -> String {
    let status = str_of(item, "status");
    let mut line = format!(
        "{} {}  by {}  {}",
        str_of(item, "id"),
        str_of(item, "version"),
        author(&item["author"]),
        if status == "tampered" {
            "changed since you allowed it"
        } else {
            status
        }
    );
    if let Some(ask) = item["ask"]
        .as_array()
        .filter(|a| !a.is_empty() && status == "pending")
    {
        let ask: Vec<&str> = ask.iter().filter_map(Value::as_str).collect();
        line.push_str(&format!("  (asks for {})", ask.join(", ")));
    }
    if let Some(error) = item["error"].as_str() {
        line.push_str(&format!("  ({error})"));
    }
    line
}

/// The engine's message, without transport wrapping.
fn plain(error: zend_rpc::RpcError) -> String {
    match error {
        zend_rpc::RpcError::Failed(message) => message,
        zend_rpc::RpcError::UnknownMethod(_) => {
            "This Zend is too old for plugins. Update it first.".into()
        }
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::{log_line, size};

    #[test]
    fn a_log_line_indents_its_traceback() {
        let item = serde_json::json!({
            "seq": 2, "at": "2026-11-14T17:04:00.123Z", "source": "window", "level": "error",
            "text": "focus: init.luau:2: kaputt\nstack traceback:\ninit.luau:2 function boom",
        });
        let line = log_line(&item);
        assert!(line.starts_with("17:04:00 window error focus"), "{line}");
        assert!(line.contains(" window error focus: init.luau:2: kaputt\n    stack traceback:\n    init.luau:2 function boom\n"), "{line}");
    }

    #[test]
    fn sizes_read_in_plain_words() {
        assert_eq!(size(1), "1 byte");
        assert_eq!(size(512), "512 bytes");
        assert_eq!(size(12_595), "12.3 KB");
        assert_eq!(size(4_299_161), "4.1 MB");
    }
}
