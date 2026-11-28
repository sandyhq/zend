//! A worker (crates/engine/src/workers.rs, worker.rs): a real Podman
//! container on this host, from the worker image (infra/worker/build.sh;
//! `ZEND_WORKER_IMAGE` names another). Ignored by default: it needs rootless
//! Podman, pasta and the image. Run it with
//! `cargo test -p zend-engine --test it worker -- --ignored --test-threads=1`.
//!
//! 1. It answers only its server: another key is refused at the handshake,
//!    and its server reaches only what a worker serves.
//! 2. It reaches neither this host (a listener on all its addresses, the
//!    loopback one too) nor the tailnet, cloud metadata or docker's bridges,
//!    while the internet answers (control: the same listener answers from
//!    the host).
//! 3. A session runs in /work: a scripted agent's run writes there.
//!
//! And after review: its limits hold (cgroup), its root isn't this user
//! (`--userns=auto`), it resolves no tailnet name, a start blocks the
//! host's networks as they are then (not as written at create), a symlink
//! it puts for its socket isn't followed, it carries no git identity, and
//! a create that fails leaves nothing behind (`a_failed_create_…`).

use std::path::Path;
use std::time::Duration;

use serde_json::{Value, json};
use zend_doc::commands::SessionCommandPayload;
use zend_engine::workers::{NewWorker, Workers};
use zend_proto::{HarnessId, RunRequest, SandboxLevel};
use zend_rpc::methods;
use zend_rpc::peer_link::{NodeKey, connect_peer_unix};

/// Takes the container and its volumes away however the test ends.
struct Cleanup(String);

impl Drop for Cleanup {
    fn drop(&mut self) {
        let podman = |args: &[&str]| {
            let _ = std::process::Command::new("podman").args(args).output();
        };
        podman(&["rm", "--force", "--time", "0", &format!("zend-worker-{}", self.0)]);
        for what in ["work", "data"] {
            podman(&["volume", "rm", "--force", &format!("zend-worker-{}-{what}", self.0)]);
        }
        podman(&["secret", "rm", "--ignore", &format!("zend-worker-{}-anthropic", self.0)]);
    }
}

fn exec(container: &str, args: &[&str]) -> (bool, String) {
    let out = std::process::Command::new("podman").arg("exec").arg(container).args(args).output().expect("podman exec");
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

/// This host's IPv4 addresses, loopback first.
fn host_addresses() -> Vec<String> {
    let out = std::process::Command::new("ip").args(["-4", "-o", "addr", "show"]).output().expect("ip addr");
    let mut addrs = vec!["127.0.0.1".to_string()];
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let mut words = line.split_whitespace();
        while let Some(word) = words.next() {
            if word == "inet"
                && let Some(addr) = words.next().and_then(|a| a.split('/').next())
                && !addrs.iter().any(|a| a == addr)
            {
                addrs.push(addr.to_string());
            }
        }
    }
    addrs
}

/// From inside `container`: each name's DNS lookup, its address or the error.
fn resolve(container: &str, names: &[&str]) -> Vec<String> {
    let list = serde_json::to_string(names).unwrap();
    let script = format!(
        "const dns=require('dns');const n={list};let out={{}};\
         process.on('exit',()=>console.log(JSON.stringify(n.map(x=>out[x]||'?'))));\
         for(const x of n)dns.lookup(x,{{family:4}},(e,a)=>{{out[x]=e?(e.code||String(e)):a;}});"
    );
    let (ok, out) = exec(container, &["node", "-e", &script]);
    assert!(ok, "resolve: {out}");
    serde_json::from_str(out.trim()).unwrap_or_else(|e| panic!("{e}: {out}"))
}

/// This host's own tailnet name, if it is on one.
fn tailnet_name() -> Option<String> {
    let out = std::process::Command::new("tailscale").args(["status", "--self", "--json"]).output().ok()?;
    let status: Value = serde_json::from_slice(&out.stdout).ok()?;
    status["Self"]["DNSName"].as_str().map(|n| n.trim_end_matches('.').to_string()).filter(|n| !n.is_empty())
}

fn podman_out(args: &[&str]) -> String {
    let out = std::process::Command::new("podman").args(args).output().expect("podman");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// From inside `container`: each target's TCP connect, `REACHED` or the error.
fn probe(container: &str, targets: &[(String, u16)]) -> Vec<(String, String)> {
    let list = serde_json::to_string(&targets.iter().map(|(h, p)| json!([h, p])).collect::<Vec<_>>()).unwrap();
    let script = format!(
        "const net=require('net');const t={list};let out={{}};\
         process.on('exit',()=>console.log(JSON.stringify(out)));\
         for(const [h,p] of t){{const k=h+':'+p;const s=net.connect({{host:h,port:p,timeout:4000}});\
         const done=r=>{{if(out[k])return;out[k]=r;s.destroy();}};\
         s.on('connect',()=>done('REACHED'));s.on('timeout',()=>done('TIMEOUT'));s.on('error',e=>done(e.code||String(e)));}}"
    );
    let (ok, out) = exec(container, &["node", "-e", &script]);
    assert!(ok, "probe: {out}");
    let map: serde_json::Map<String, Value> = serde_json::from_str(out.trim()).unwrap_or_else(|e| panic!("{e}: {out}"));
    targets
        .iter()
        .map(|(h, p)| {
            let k = format!("{h}:{p}");
            let v = map.get(&k).and_then(Value::as_str).unwrap_or("?").to_string();
            (k, v)
        })
        .collect()
}

fn run(chat: &str, prompt: &str) -> Value {
    let request = RunRequest {
        mcp: None,
        prompt: prompt.into(),
        harness: Some(HarnessId::Mock),
        model: None,
        reasoning: None,
        model_options: Default::default(),
        cwd: "/work".into(),
        sandbox: SandboxLevel::DangerFullAccess,
        auto_approve: true,
        access: Default::default(),
        plan: false,
        resume: None,
        attachments: vec![],
        worktree: None,
        env: Default::default(),
        extra_mcp: Vec::new(),
        instructions: Vec::new(),
        fork_from: None,
    };
    json!({ "chatId": chat, "command": SessionCommandPayload::Run { request, message_id: format!("m-{chat}") } })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs rootless podman and the worker image (infra/worker/build.sh)"]
async fn a_worker_answers_its_server_alone_reaches_no_host_and_runs_in_work() {
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().join("server");
    std::fs::create_dir_all(&data).unwrap();
    let name = format!("t{}", std::process::id());
    let _cleanup = Cleanup(name.clone());
    let workers = Workers::new(&data);
    let image = std::env::var("ZEND_WORKER_IMAGE").ok();
    let worker = workers
        .create(NewWorker { name: name.clone(), image, ..Default::default() })
        .await
        .expect("worker made and answering");
    assert_eq!(workers.list(), vec![worker.clone()]);
    assert_eq!(workers.state(&name).await, "running");
    let socket = workers.socket(&name);
    let server = zend_engine::peer_link::load_node_key(&data).unwrap();

    // 1. Its server, and only its server.
    let client = workers.connect(&name).await.expect("its server dials in");
    let info = client.call(methods::ENGINE_INFO, json!({})).await.expect("engine info for its server");
    // (Its own word for its device; its server shows it as its own word.)
    assert!(info["deviceId"].as_str().is_some_and(|d| !d.is_empty()), "{info}");
    let worker_device = worker.device_id();
    for method in [methods::LIST_PEERS, methods::PEER_PAIR, methods::CREATE_WORKER, methods::SET_PEER_KEY, methods::PEER_LIST_PROJECT_FILES] {
        assert!(client.call(method, json!({})).await.is_err(), "{method} answered its server");
    }
    let stranger = NodeKey::generate().unwrap();
    let refused = connect_peer_unix(&socket, &stranger, &worker.key).await;
    match refused {
        Err(err) => eprintln!("another key: {err}"),
        Ok(other) => {
            let reply = other.call(methods::ENGINE_INFO, json!({})).await;
            panic!("another key got in: {reply:?}");
        }
    }
    // (And the server checks the worker: another key expected, not a word sent.)
    assert!(connect_peer_unix(&socket, &server, stranger.public()).await.is_err(), "a worker not pinned accepted");

    // Its limits hold (before: none, "max").
    let cgroup = |file: &str| exec(&worker.container, &["cat", &format!("/sys/fs/cgroup/{file}")]).1.trim().to_string();
    assert_eq!(
        (cgroup("memory.max"), cgroup("memory.swap.max"), cgroup("cpu.max"), cgroup("pids.max")),
        ((4u64 << 30).to_string(), "0".into(), "200000 100000".into(), "512".into())
    );
    // Its root isn't this user: its namespace maps 0 to a subordinate id.
    let pid = podman_out(&["inspect", "--format", "{{.State.Pid}}", &worker.container]);
    let map = std::fs::read_to_string(format!("/proc/{pid}/uid_map")).unwrap();
    let me = podman_out(&["unshare", "cat", "/proc/self/uid_map"]);
    let my_uid = me.split_whitespace().nth(1).unwrap().to_string();
    let root_is = map.split_whitespace().nth(1).unwrap().to_string();
    assert_ne!(root_is, my_uid, "the worker's root is this user: {map}");
    // No git identity of anyone's.
    assert!(!exec(&worker.container, &["test", "-e", "/etc/gitconfig"]).0, "/etc/gitconfig in the worker");
    let (_, identity) = exec(&worker.container, &["git", "config", "--get", "user.email"]);
    assert!(identity.trim().is_empty(), "a git identity in the worker: {identity}");

    // 2. No host, no tailnet; the internet yes.
    let listener = std::net::TcpListener::bind("0.0.0.0:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let host = host_addresses();
    for addr in host.iter() {
        assert!(
            std::net::TcpStream::connect_timeout(&format!("{addr}:{port}").parse().unwrap(), Duration::from_secs(2)).is_ok(),
            "control: {addr}:{port} doesn't answer from the host"
        );
    }
    let mut targets: Vec<(String, u16)> = host.iter().map(|a| (a.clone(), port)).collect();
    targets.extend([
        ("100.100.100.100".to_string(), 53),
        ("100.64.0.1".to_string(), 80),
        ("169.254.169.254".to_string(), 80),
        ("169.254.2.1".to_string(), port),
        ("172.17.0.1".to_string(), port),
        ("10.0.0.1".to_string(), 80),
        ("192.168.1.1".to_string(), 80),
    ]);
    let mut seen = std::collections::BTreeSet::new();
    targets.retain(|t| seen.insert(t.clone()));
    let probed = probe(&worker.container, &targets);
    eprintln!("from the worker: {probed:?}");
    for (target, result) in &probed {
        assert_ne!(result, "REACHED", "the worker reached {target}");
    }
    let internet = probe(&worker.container, &[("1.1.1.1".to_string(), 443)]);
    assert_eq!(internet[0].1, "REACHED", "control: no internet from the worker: {internet:?}");
    // DNS: a public name resolves, this host's tailnet name doesn't
    // (pasta asks a public resolver, not the host's MagicDNS).
    let tailnet = tailnet_name();
    let mut names = vec!["api.anthropic.com"];
    names.extend(tailnet.as_deref());
    let resolved = resolve(&worker.container, &names);
    eprintln!("resolved by the worker: {names:?} -> {resolved:?}");
    assert!(resolved[0].parse::<std::net::Ipv4Addr>().is_ok(), "control: no DNS in the worker: {resolved:?}");
    if tailnet.is_some() {
        assert!(resolved[1].parse::<std::net::Ipv4Addr>().is_err(), "the worker resolved this host's tailnet name: {resolved:?}");
    } else {
        eprintln!("not on a tailnet: no tailnet name to resolve");
    }
    // Control: the same image and pasta options without the blocked routes
    // reach this host (what this test catches), and the worker's engine
    // there refuses to start.
    let bare = format!("zend-worker-{name}-bare");
    let _bare = Cleanup(format!("{name}-bare"));
    let podman = |args: &[&str]| std::process::Command::new("podman").args(args).output().expect("podman");
    let made = podman(&["run", "-d", "--name", &bare, "--network", zend_engine::workers::PASTA, "--entrypoint", "sleep", &worker.image, "120"]);
    assert!(made.status.success(), "{}", String::from_utf8_lossy(&made.stderr));
    let unblocked = probe(&bare, &targets);
    eprintln!("without the routes: {unblocked:?}");
    let leaked: Vec<&String> = unblocked.iter().filter(|(_, r)| r == "REACHED").map(|(t, _)| t).collect();
    assert!(!leaked.is_empty(), "control: without the routes nothing was reached: {unblocked:?}");
    podman(&["rm", "--force", "--time", "0", &bare]);
    if let Some(tailnet) = &tailnet {
        // Control: pasta sending DNS on to the host's resolver (before) resolves it.
        let old = zend_engine::workers::PASTA.replace(&format!(",--dns-host,{}", zend_engine::workers::DNS_HOST), "");
        let made = podman(&["run", "-d", "--name", &bare, "--network", &old, "--dns", "169.254.2.1", "--entrypoint", "sleep", &worker.image, "120"]);
        assert!(made.status.success(), "{}", String::from_utf8_lossy(&made.stderr));
        let resolved = resolve(&bare, &[tailnet]);
        assert!(resolved[0].parse::<std::net::Ipv4Addr>().is_ok(), "control: the host's resolver didn't name {tailnet}: {resolved:?}");
        podman(&["rm", "--force", "--time", "0", &bare]);
    }
    let conf = data.join("workers").join(&name).join("conf");
    let started = podman(&[
        "run", "--rm", "--network", zend_engine::workers::PASTA, "--user", "1000:1000",
        "--tmpfs", "/data:rw,mode=1777", "--mount", &format!("type=bind,src={},dst=/etc/zend-worker,ro", conf.display()),
        &worker.image,
    ]);
    let said = format!("{}{}", String::from_utf8_lossy(&started.stdout), String::from_utf8_lossy(&started.stderr));
    assert!(!started.status.success() && said.contains("no unreachable route"), "a worker started without its routes: {said}");
    drop(listener);

    // 3. A session in /work: its server makes the chat in project work and
    //    queues a scripted run, which writes where it runs.
    let created = client
        .call(
            methods::MUTATE,
            json!({ "op": "createChat", "chatId": "w-chat", "spaceId": worker.project(), "config": { "harness": HarnessId::Mock, "sandbox": "danger-full-access", "access": "full-access" } }),
        )
        .await
        .expect("chat in work");
    eprintln!("created: {created}");
    assert!(
        client.call(methods::MUTATE, json!({ "op": "createChat", "chatId": "w-out", "spaceId": "elsewhere" })).await.is_err(),
        "a chat outside work"
    );
    client.call(methods::QUEUE_COMMAND, run("w-chat", "/pwd")).await.expect("run queued");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        let (ok, out) = exec(&worker.container, &["cat", "/work/.zend-mock-ran"]);
        if ok {
            assert!(out.contains("the mock ran here"), "{out}");
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            let logs = std::process::Command::new("podman").args(["logs", "--tail", "60", &worker.container]).output().unwrap();
            panic!(
                "no run in /work: {out}\n--- its log:\n{}{}",
                String::from_utf8_lossy(&logs.stdout),
                String::from_utf8_lossy(&logs.stderr)
            );
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }

    // 4. The same from its server's window: the worker shows as a device
    //    with its project, a chat made there by its device runs in /work,
    //    and its reply reads back through the window.
    let (ok, out) = exec(&worker.container, &["rm", "/work/.zend-mock-ran"]);
    assert!(ok, "{out}");
    std::fs::write(data.join("device-id"), "device-the-server").unwrap();
    let server_core = zend_engine::EngineCore::assemble(
        &data,
        std::sync::Arc::new(zend_engine::HarnessRegistry::new()),
        HarnessId::Mock,
        None,
    )
    .expect("the server's engine");
    let link = zend_engine::peer_link::PeerLink::new(&data, 0, server_core.rpc_service(), zend_engine::peer_link::Seams::real());
    let window = zend_rpc::memory_client(std::sync::Arc::new(zend_engine::peer_link::PeerLinkRpc {
        inner: server_core.rpc_service(),
        link: link.clone(),
    }));
    let mut spaces = window.subscribe_checked(methods::WATCH_SPACES, json!({})).await.expect("spaces");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        let rows = tokio::time::timeout_at(deadline, spaces.recv()).await.expect("its project in the window in 30 s").expect("watch");
        if let Some(p) = rows.as_array().into_iter().flatten().find(|p| p["id"].as_str() == Some(worker.project().as_str())) {
            assert_eq!((p["deviceId"].as_str(), p["path"].as_str()), (Some(worker_device.as_str()), Some("")), "{p}");
            break;
        }
    }
    window
        .call(
            methods::MUTATE,
            json!({ "op": "createChat", "chatId": "win-chat", "deviceId": worker_device, "cwd": data.display().to_string(), "config": { "harness": HarnessId::Mock, "sandbox": "danger-full-access", "access": "full-access" } }),
        )
        .await
        .expect("made from the window");
    window.call(methods::QUEUE_COMMAND, run("win-chat", "/pwd")).await.expect("queued from the window");
    let mut said = window.subscribe_checked(methods::WATCH_DOC_MESSAGES, json!({ "chatId": "win-chat" })).await.expect("its transcript");
    let mut all = String::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    while !all.contains("wrote .zend-mock-ran") {
        let frame = tokio::time::timeout_at(deadline, said.recv()).await.unwrap_or_else(|_| panic!("no reply in 30 s: {all}")).expect("transcript");
        all.push_str(&frame.to_string());
    }
    assert!(all.contains("cwd /work"), "{all}");
    let (ok, out) = exec(&worker.container, &["cat", "/work/.zend-mock-ran"]);
    assert!(ok && out.contains("the mock ran here"), "{out}");
    drop((said, spaces, window, link, server_core));

    // A start blocks the host's networks as they are then: a blocked list
    // gone stale (here: only 10/8 left) is written afresh, and the host's
    // addresses stay out of reach (before: the stale list was applied).
    drop(client);
    let blocked = conf.join("blocked");
    std::fs::write(&blocked, "10.0.0.0/8\n").unwrap();
    let stopped = podman(&["stop", "--time", "5", &worker.container]);
    assert!(stopped.status.success(), "{}", String::from_utf8_lossy(&stopped.stderr));
    workers.start(&name).await.expect("started again");
    assert_eq!(std::fs::read_to_string(&blocked).unwrap(), zend_engine::workers::blocked_networks().join("\n") + "\n");
    let listener = std::net::TcpListener::bind("0.0.0.0:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let again: Vec<(String, u16)> = host.iter().map(|a| (a.clone(), port)).collect();
    let probed = probe(&worker.container, &again);
    eprintln!("after a start: {probed:?}");
    for (target, result) in &probed {
        assert_ne!(result, "REACHED", "after a start the worker reached {target}");
    }
    drop(listener);
    workers.connect(&name).await.expect("its server dials in again");

    // Its socket swapped for a symlink (the worker can write where it is):
    // its server doesn't follow it.
    let (ok, out) = exec(
        &worker.container,
        &["sh", "-c", "mv /run/zend-worker/worker.sock /run/zend-worker/real.sock && ln -s real.sock /run/zend-worker/worker.sock"],
    );
    assert!(ok, "{out}");
    let refused = workers.connect(&name).await;
    assert!(refused.as_ref().is_err_and(|e| e.to_string().contains("not a socket")), "a symlink was followed: {:?}", refused.err());

    // And away again.
    workers.remove(&name, false).await.expect("removed");
    assert!(workers.list().is_empty());
    assert!(!Path::new(&data.join("workers").join(&name)).exists());
    let (ok, _) = exec(&worker.container, &["true"]);
    assert!(!ok, "its container is still there");
}

/// A create that fails (here at the image, after its key secret is made)
/// leaves nothing behind: no secret, container, folder or entry, and a
/// volume that was there before it (kept by `rm --keep-volumes`) stays.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs rootless podman"]
async fn a_failed_create_leaves_nothing_behind() {
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().join("server");
    std::fs::create_dir_all(&data).unwrap();
    let name = format!("f{}", std::process::id());
    let _cleanup = Cleanup(name.clone());
    let kept = format!("zend-worker-{name}-work");
    let made = std::process::Command::new("podman").args(["volume", "create", &kept]).output().unwrap();
    assert!(made.status.success());
    let workers = Workers::new(&data);
    let failed = workers
        .create(NewWorker {
            name: name.clone(),
            image: Some("localhost/zend-worker-not-there:none".into()),
            keys: vec![("anthropic".into(), "sk-ant-not-a-key".into())],
            ..Default::default()
        })
        .await;
    assert!(failed.is_err(), "made from an image that isn't there");
    eprintln!("failed: {}", failed.unwrap_err());
    let exists = |what: &[&str]| std::process::Command::new("podman").args(what).output().unwrap().status.success();
    assert!(!exists(&["secret", "exists", &format!("zend-worker-{name}-anthropic")]), "its key secret is left");
    assert!(!exists(&["container", "exists", &format!("zend-worker-{name}")]), "its container is left");
    assert!(!exists(&["volume", "exists", &format!("zend-worker-{name}-data")]), "a volume it made is left");
    assert!(exists(&["volume", "exists", &kept]), "a volume from before is gone");
    assert!(workers.list().is_empty());
    assert!(!data.join("workers").join(&name).exists(), "its folder is left");
    // Refused before anything is made: an option for an image.
    let refused = workers.create(NewWorker { name: name.clone(), image: Some("--privileged".into()), ..Default::default() }).await;
    assert!(refused.is_err_and(|e| e.contains("not an option")));
}
