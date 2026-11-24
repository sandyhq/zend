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
    let worker = workers.create(NewWorker { name: name.clone(), image, keys: vec![] }).await.expect("worker made and answering");
    assert_eq!(workers.list(), vec![worker.clone()]);
    assert_eq!(workers.state(&name).await, "running");
    let socket = workers.socket(&name);
    let server = zend_engine::peer_link::load_node_key(&data).unwrap();

    // 1. Its server, and only its server.
    let client = workers.connect(&name).await.expect("its server dials in");
    client.call(methods::ENGINE_INFO, json!({})).await.expect("engine info for its server");
    for method in [methods::WATCH_CHATS, methods::LIST_PEERS, methods::PEER_PAIR, methods::CREATE_WORKER] {
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
            json!({ "op": "createChat", "chatId": "w-chat", "spaceId": "work", "config": { "harness": HarnessId::Mock, "sandbox": "danger-full-access", "access": "full-access" } }),
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

    // And away again.
    drop(client);
    workers.remove(&name, false).await.expect("removed");
    assert!(workers.list().is_empty());
    assert!(!Path::new(&data.join("workers").join(&name)).exists());
    let (ok, _) = exec(&worker.container, &["true"]);
    assert!(!ok, "its container is still there");
}
