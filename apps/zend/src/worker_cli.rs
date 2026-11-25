//! `zend worker`: this server's worker containers, over the running
//! engine's local IPC (the methods are this device's own:
//! [`zend_engine::workers`]).

use serde_json::{Value, json};
use zend_cli::{WorkerCommand, WorkerLoginCommand, terminal_safe};
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
        WorkerCommand::Create { name, image, key, memory, cpus, pids } => {
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
            let reply = call(methods::CREATE_WORKER, json!({ "name": name, "image": image, "keys": keys, "memory": memory, "cpus": cpus, "pids": pids })).await?;
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
                let limits = &w["limits"];
                println!(
                    "{}  {}  {}  device {}  key {}…  {} memory, {} cpus, {} pids  {}",
                    w["name"].as_str().unwrap_or_default(),
                    terminal_safe(w["state"].as_str().unwrap_or("?"), false),
                    w["image"].as_str().unwrap_or_default(),
                    w["deviceId"].as_str().unwrap_or_default(),
                    &key[..key.len().min(16)],
                    limits["memory"].as_str().unwrap_or("?"),
                    limits["cpus"].as_str().unwrap_or("?"),
                    limits["pids"],
                    if providers.is_empty() { "no API keys".to_string() } else { providers.join(" ") }
                );
            }
        }
        WorkerCommand::Run { name, prompt, harness, model, timeout } => {
            let worker = call(methods::LIST_WORKERS, json!({})).await?["workers"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|w| w["name"].as_str() == Some(name.as_str()))
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("no worker {name} (zend worker list)"))?;
            let state = worker["state"].as_str().unwrap_or("?");
            if state != "running" {
                anyhow::bail!("worker {name} is {}: zend worker start {name}", terminal_safe(state, false));
            }
            let project = worker["project"].as_str().unwrap_or(zend_engine::worker::PROJECT).to_string();
            let harness: zend_proto::HarnessId = serde_json::from_value(json!(harness.as_deref().unwrap_or("pi")))
                .map_err(|_| anyhow::anyhow!("no agent {:?}", harness.unwrap_or_default()))?;
            let reply = run_in_worker(client, &name, &project, &prompt, harness, model, std::time::Duration::from_secs(timeout)).await?;
            println!("{}", terminal_safe(&reply, true));
            return Ok(());
        }
        WorkerCommand::Start { name } => {
            let reply = call(methods::START_WORKER, json!({ "name": name })).await?;
            println!("Worker {name}: {}.", reply["state"].as_str().unwrap_or("?"));
        }
        WorkerCommand::Stop { name } => {
            let reply = call(methods::STOP_WORKER, json!({ "name": name })).await?;
            println!("Worker {name}: {}.", terminal_safe(reply["state"].as_str().unwrap_or("?"), false));
        }
        WorkerCommand::Login(WorkerLoginCommand::Allow { name, provider }) => {
            let reply = call(methods::ALLOW_WORKER_LOGIN, json!({ "name": name, "provider": provider })).await?;
            println!(
                "Allowed. Worker {name}'s Pi turns on {provider} models run on this server's {provider} login: each turn gets its access token only, never the refresh token."
            );
            if reply["keyWins"].as_bool() == Some(true) {
                println!("It was made with a {provider} API key: the key wins over the login.");
            }
            if reply["hostSignedIn"].as_bool() == Some(false) {
                eprintln!("This server's pi has no {provider} login yet: sign in (pi, /login) or its turns are refused.");
            }
        }
        WorkerCommand::Login(WorkerLoginCommand::Rm { name, provider }) => {
            let reply = call(methods::REMOVE_WORKER_LOGIN, json!({ "name": name, "provider": provider })).await?;
            if reply["removed"].as_bool() == Some(true) {
                println!("Taken back: the token left worker {name}, its next turn on {provider} models is refused.");
            } else {
                println!("Worker {name} wasn't allowed this server's {provider} login.");
            }
        }
        WorkerCommand::Login(WorkerLoginCommand::List) => {
            let reply = call(methods::LIST_WORKER_LOGINS, json!({})).await?;
            let logins = reply["logins"].as_array().cloned().unwrap_or_default();
            if logins.is_empty() {
                println!("No worker may use this server's logins.");
            }
            for row in logins {
                let providers: Vec<&str> = row["providers"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
                println!("{}  {}", terminal_safe(row["name"].as_str().unwrap_or_default(), false), providers.join(" "));
            }
        }
        WorkerCommand::Rm { name, keep_volumes } => {
            call(methods::REMOVE_WORKER, json!({ "name": name, "keepVolumes": keep_volumes })).await?;
            println!("Removed worker {name}{}.", if keep_volumes { " (its volumes kept)" } else { " and its volumes" });
        }
    }
    Ok(())
}

/// `zend worker run`: a chat in worker `name`'s project (as this server's
/// window shows it), a run of `prompt` there, and its reply.
async fn run_in_worker(
    client: zend_rpc::RpcClient,
    name: &str,
    project: &str,
    prompt: &str,
    harness: zend_proto::HarnessId,
    model: Option<String>,
    timeout: std::time::Duration,
) -> anyhow::Result<String> {
    use std::time::{Duration, Instant};
    use zend_mcp::TurnOutcome;
    let zend = zend_mcp::Zend::with_client(client, zend_mcp::Origin::default());
    // Shown once the engine has dialled it (at once after a start, else
    // within its next look at the workers).
    let deadline = Instant::now() + Duration::from_secs(30);
    let device = loop {
        if let Some(space) = zend.spaces().await?.into_iter().find(|s| s.id == project) {
            break space.device_id;
        }
        if Instant::now() >= deadline {
            anyhow::bail!("worker {name} isn't shown by this Zend yet (its project {project}): is it answering? zend worker list");
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    };
    let chat_id = uuid::Uuid::new_v4().to_string();
    let config = json!({ "harness": harness, "model": model, "sandbox": "danger-full-access", "access": "full-access" });
    zend.mutate(json!({ "op": "createChat", "chatId": chat_id, "spaceId": project, "config": config }))
        .await
        .map_err(|e| anyhow::anyhow!(terminal_safe(&e.to_string(), true)))?;
    let request = zend_proto::RunRequest {
        mcp: None,
        prompt: prompt.to_string(),
        harness: Some(harness),
        model,
        reasoning: None,
        model_options: Default::default(),
        // (Its project's folder, on the worker: /work.)
        cwd: ".".into(),
        sandbox: zend_proto::SandboxLevel::DangerFullAccess,
        auto_approve: true,
        access: Default::default(),
        plan: false,
        resume: None,
        attachments: vec![],
        worktree: None,
        env: Default::default(),
        extra_mcp: Vec::new(),
        instructions: Vec::new(),
    };
    let message_id = uuid::Uuid::new_v4().to_string();
    zend.queue_command(&chat_id, &zend_doc::SessionCommandPayload::Run { request, message_id })
        .await
        .map_err(|e| anyhow::anyhow!(terminal_safe(&e.to_string(), true)))?;
    let read_it = format!("chat {chat_id} on worker {name}: open it in Zend's window (device {name}, project work)");
    let started = Instant::now();
    let chat = loop {
        if let Some(chat) = zend.chats().await?.into_iter().find(|c| c.id == chat_id) {
            break chat;
        }
        if started.elapsed() >= timeout.min(Duration::from_secs(30)) {
            anyhow::bail!("its chat isn't listed yet: {read_it}");
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    };
    let left = timeout.saturating_sub(started.elapsed());
    let (outcome, _) = zend.wait_for_turn(&chat, None, true, left).await?;
    match outcome {
        TurnOutcome::Completed => {}
        TurnOutcome::TimedOut => anyhow::bail!("still running after {}s: {read_it}", timeout.as_secs()),
        other => anyhow::bail!("the turn ended {other:?}: {read_it}"),
    }
    let entries = zend.transcript_on(&chat_id, Some(&device)).await?;
    let rendered = zend_mcp::render_entries(&entries, zend_mcp::RenderOptions { include_reasoning: false, include_tools: false });
    let reply = rendered
        .iter()
        .rev()
        .find(|m| m.role == zend_doc::MessageRole::Assistant)
        .map(|m| if m.text.is_empty() { m.errors.join("\n") } else { m.text.clone() })
        .unwrap_or_default();
    Ok(if reply.is_empty() { format!("(no reply text: {read_it})") } else { reply })
}
