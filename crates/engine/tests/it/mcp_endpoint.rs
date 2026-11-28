//! A harness run reaches Zend's MCP tools over the engine's own HTTP endpoint
//! ([zend_mcp::http], `SessionsEngine::serve_mcp_endpoint`): the run's
//! request carries the endpoint and a token of its own (no stdio server, no
//! token on a command line), the tools served are Zend's full set (`whoami`, `show_visual`, `update_visual`, `read_visual`),
//! and a call through the token speaks as the run's chat.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use futures::{StreamExt, stream::BoxStream};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use zend_doc::SessionCommandPayload;
use zend_engine::{EngineCore, HarnessRegistry};
use zend_harness::{Harness, HarnessError, RunControls};
use zend_proto::{
    AgentEvent, DoneStatus, HarnessId, Model, ReasoningLevel, RunRequest, SandboxLevel,
    SteeringMode,
};
use zend_rpc::methods;

/// What the mock agent saw on its way through the endpoint.
#[derive(Default, Clone)]
struct Seen {
    request_mcp: Option<zend_proto::McpServer>,
    tool_names: Vec<String>,
    whoami: String,
    bad_token_status: u16,
}

struct Agent(Arc<Mutex<Seen>>);

/// One HTTP/1.1 POST to the endpoint: (status, body).
async fn post(url: &str, token: &str, body: Value) -> (u16, String) {
    let hostport = url.trim_start_matches("http://");
    let (host, path) = hostport.split_once('/').map(|(h, p)| (h, format!("/{p}"))).unwrap();
    let body = body.to_string();
    let mut stream = tokio::net::TcpStream::connect(host).await.unwrap();
    let request = format!(
        "POST {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\nAccept: application/json, text/event-stream\r\nContent-Type: application/json\r\nAuthorization: Bearer {token}\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut raw = String::new();
    tokio::time::timeout(Duration::from_secs(10), stream.read_to_string(&mut raw))
        .await
        .expect("the endpoint answered in time")
        .unwrap();
    let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((&raw, ""));
    (head.split_whitespace().nth(1).unwrap().parse().unwrap(), body.to_owned())
}

#[async_trait]
impl Harness for Agent {
    fn id(&self) -> HarnessId {
        HarnessId::Mock
    }
    fn display_name(&self) -> &str {
        "Agent"
    }
    fn supports_steering(&self) -> bool {
        false
    }
    fn steering_mode(&self) -> SteeringMode {
        SteeringMode::TurnBoundary
    }
    fn reasoning_levels(&self) -> &[ReasoningLevel] {
        &[]
    }
    async fn models(&self) -> Result<Vec<Model>, HarnessError> {
        Ok(vec![])
    }
    async fn run(
        &self,
        request: RunRequest,
        _: RunControls,
    ) -> Result<BoxStream<'static, Result<AgentEvent, HarnessError>>, HarnessError> {
        let seen = self.0.clone();
        let mcp = request.mcp.clone();
        // The agent's side of it, as a driver does: take the endpoint out of
        // the run's request and talk to it.
        let call = async move {
            let mut now = Seen { request_mcp: mcp.clone(), ..Seen::default() };
            if let Some(http) = mcp.and_then(|m| m.http) {
                let token = http.headers["Authorization"].trim_start_matches("Bearer ").to_owned();
                let (status, body) = post(&http.url, "wrong", json!({"jsonrpc":"2.0","id":0,"method":"tools/list"})).await;
                now.bad_token_status = status;
                assert!(!body.contains("show_visual"));
                let (_, body) = post(&http.url, &token, json!({"jsonrpc":"2.0","id":1,"method":"tools/list"})).await;
                let body = body.lines().rev().find(|l| l.contains("\"tools\"")).unwrap_or("").trim_start_matches("data:").trim().to_owned();
                // (Chunked framing may wrap the JSON: take its outermost braces.)
                let json: Value = serde_json::from_str(&body[body.find('{').unwrap()..=body.rfind('}').unwrap()]).unwrap();
                now.tool_names = json["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap().to_owned()).collect();
                let (_, body) = post(&http.url, &token, json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"whoami","arguments":{}}})).await;
                now.whoami = body;
            }
            *seen.lock().unwrap() = now;
        };
        call.await;
        Ok(futures::stream::iter(vec![
            Ok(AgentEvent::TextDelta { text: "done".into() }),
            Ok(AgentEvent::Done { status: DoneStatus::Completed, result: None, error: None, session_id: Some("sess".into()) }),
        ])
        .boxed())
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_harness_run_reaches_the_zend_tools_over_the_engine_s_endpoint() {
    let dir = tempfile::tempdir().unwrap();
    let seen = Arc::new(Mutex::new(Seen::default()));
    let registry = HarnessRegistry::new();
    registry.register(Arc::new(Agent(seen.clone())));
    let core = EngineCore::assemble(dir.path(), Arc::new(registry), HarnessId::Mock, None).unwrap();
    core.sessions.set_no_ipc_port();
    core.sessions.serve_mcp_endpoint(core.rpc_service()).await.unwrap();
    let client = zend_rpc::memory_client(core.rpc_service());
    client
        .call(methods::MUTATE, json!({"op": "createChat", "chatId": "c1", "deviceId": core.device_id, "cwd": "/tmp/c1"}))
        .await
        .unwrap();
    core.workspace.rename_chat("c1", "Chat c1").unwrap();
    let request = RunRequest {
        mcp: None,
        prompt: "go".into(),
        harness: Some(HarnessId::Mock),
        model: None,
        reasoning: None,
        model_options: Default::default(),
        cwd: "/tmp/c1".into(),
        sandbox: SandboxLevel::WorkspaceWrite,
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
    client
        .call(
            methods::QUEUE_COMMAND,
            json!({"chatId": "c1", "command": SessionCommandPayload::Run { request, message_id: "m1".into() }}),
        )
        .await
        .unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    while seen.lock().unwrap().tool_names.is_empty() {
        assert!(tokio::time::Instant::now() < deadline, "the run never reached the endpoint");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let seen = seen.lock().unwrap().clone();
    let mcp = seen.request_mcp.expect("the run carries Zend's MCP server");
    let http = mcp.http.expect("served over HTTP by the engine, not a process per agent");
    assert!(http.url.starts_with("http://127.0.0.1:"), "{}", http.url);
    assert!(!mcp.env.values().chain(mcp.args.iter()).any(|v| v.contains("ipc-token")), "no IPC token handed to the agent: {:?}", mcp.env);
    assert_eq!(seen.bad_token_status, 401);
    for tool in ["whoami", "show_visual", "update_visual", "read_visual", "create_chats", "send_message"] {
        assert!(seen.tool_names.iter().any(|n| n == tool), "{tool} served: {:?}", seen.tool_names);
    }
    assert!(seen.whoami.contains("c1"), "the token speaks as the run's chat: {}", seen.whoami);
    core.shutdown().await;
}
