//! Server side: dispatch loop over string frames + the WebSocket acceptor.

use std::collections::HashMap;
use std::sync::Arc;

use futures::StreamExt;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::handshake::server::{
    ErrorResponse, Request as HandshakeRequest, Response as HandshakeResponse,
};
use tokio_tungstenite::tungstenite::http::StatusCode;

use crate::ipc_auth::IpcTokens;
use crate::{ClientFrame, RpcError, RpcReply, RpcService, ServerFrame};

/// The terminal a call must take effect in order for: its input and size.
/// Each other call runs on a task of its own, and tasks spawned back to back
/// start in no set order (tokio runs the newest first), so two writes would
/// reach the terminal swapped: keys typed or pasted out of order, and a
/// viewer's answers to a program's queries (a colour, then the cursor) read
/// the wrong way round, the rest left as typed keys.
fn ordered(method: &str, params: &serde_json::Value) -> Option<String> {
    matches!(
        method,
        crate::methods::WRITE_TERMINAL | crate::methods::RESIZE_TERMINAL
    )
    .then(|| params["terminalId"].as_str().unwrap_or_default().to_owned())
}

/// One terminal's [`ordered`] calls, in the order the connection sent them.
type Lane = mpsc::UnboundedSender<(u64, String, serde_json::Value)>;

/// Runs a lane's calls one after another; each answer is sent beside it, so
/// a client slow to read answers doesn't hold the next write back. Ends once
/// its sender is gone and what was queued has run.
fn lane(service: Arc<dyn RpcService>, out: mpsc::Sender<String>) -> Lane {
    let (lane, mut queued) = mpsc::unbounded_channel::<(u64, String, serde_json::Value)>();
    tokio::spawn(async move {
        while let Some((id, method, params)) = queued.recv().await {
            let result = service.handle(&method, params).await;
            tokio::spawn(respond(out.clone(), id, method, result));
        }
    });
    lane
}

/// A connection's calls in flight, each on a task of its own: aborted when
/// this goes, however the connection ends. Dropped with the connection's
/// future too (a phone-link key replaced aborts its listener and every
/// connection in it), so no call it started runs on after it.
struct InFlight(HashMap<u64, tokio::task::AbortHandle>);

impl Drop for InFlight {
    fn drop(&mut self) {
        for (_, task) in self.0.drain() {
            task.abort();
        }
    }
}

/// Serve one connection: read client frames from `inbound`, write server frames to `out`.
/// Returns when `inbound` closes; all in-flight request tasks are aborted on exit (or
/// when this future is dropped before), but
/// a terminal's input already sent still goes in ([`ordered`] calls: one lane a
/// terminal, each one after another in the order sent, beside the rest).
pub async fn serve_connection(
    service: Arc<dyn RpcService>,
    out: mpsc::Sender<String>,
    mut inbound: mpsc::Receiver<String>,
) {
    let mut running = InFlight(HashMap::new());
    let mut lanes: HashMap<String, Lane> = HashMap::new();
    while let Some(payload) = inbound.recv().await {
        // ndjson: a transport may batch several frames per message.
        for line in payload.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let frame: ClientFrame = match serde_json::from_str(line) {
                Ok(frame) => frame,
                Err(err) => {
                    tracing::warn!(error = %err, "rpc: dropping malformed client frame");
                    continue;
                }
            };
            running.0.retain(|_, task| !task.is_finished());
            if frame.cancel {
                if let Some(task) = running.0.remove(&frame.id) {
                    task.abort();
                }
                continue;
            }
            let Some(method) = frame.method else {
                tracing::warn!(id = frame.id, "rpc: frame has neither method nor cancel");
                continue;
            };
            if let Some(terminal) = ordered(&method, &frame.params) {
                let queue = lanes
                    .entry(terminal)
                    .or_insert_with(|| lane(service.clone(), out.clone()));
                let _ = queue.send((frame.id, method, frame.params));
                continue;
            }
            // A wire id names one live call: a second call under it would
            // take its slot, and cancelling the id would no longer reach the
            // first. Refused before it runs.
            if running.0.contains_key(&frame.id) {
                tracing::warn!(id = frame.id, method = %method, "rpc: refusing a call that reuses a live wire id");
                let refusal = Err(RpcError::Failed(format!("call id {} is still in use", frame.id)));
                tokio::spawn(respond(out.clone(), frame.id, method, refusal));
                continue;
            }
            let task = tokio::spawn(handle_request(
                service.clone(),
                out.clone(),
                frame.id,
                method,
                frame.params,
            ));
            running.0.insert(frame.id, task.abort_handle());
        }
    }
    drop(running);
    // (Each lane ends once what was queued has gone in.)
    drop(lanes);
}

async fn handle_request(
    service: Arc<dyn RpcService>,
    out: mpsc::Sender<String>,
    id: u64,
    method: String,
    params: serde_json::Value,
) {
    let result = service.handle(&method, params).await;
    respond(out, id, method, result).await;
}

/// Send a call's answer: its value, its stream's items, or its error.
async fn respond(
    out: mpsc::Sender<String>,
    id: u64,
    method: String,
    result: Result<RpcReply, RpcError>,
) {
    let send = |frame: ServerFrame| {
        let out = out.clone();
        async move {
            match serde_json::to_string(&frame) {
                Ok(json) => out.send(json).await.map_err(|_| RpcError::Closed),
                Err(err) => {
                    tracing::error!(error = %err, "rpc: failed to serialize server frame");
                    Err(RpcError::Closed)
                }
            }
        }
    };
    match result {
        Ok(RpcReply::Value(value)) => {
            let _ = send(ServerFrame {
                id,
                ok: Some(value),
                ..Default::default()
            })
            .await;
        }
        Ok(RpcReply::Stream(mut stream)) => {
            // Only the versioned checkout-PR stream uses an explicit readiness
            // frame. Sending it for legacy streams would make older clients remove
            // their pending stream as if it were a unary response.
            if method == crate::methods::WATCH_CHECKOUT_CHANGE_REQUEST
                && send(ServerFrame {
                    id,
                    ok: Some(serde_json::json!({ "stream": true })),
                    ..Default::default()
                })
                .await
                .is_err()
            {
                return;
            }
            while let Some(item) = stream.next().await {
                if send(ServerFrame {
                    id,
                    item: Some(item),
                    ..Default::default()
                })
                .await
                .is_err()
                {
                    return; // connection gone
                }
            }
            let _ = send(ServerFrame {
                id,
                done: true,
                ..Default::default()
            })
            .await;
        }
        Err(err) => {
            let _ = send(ServerFrame {
                id,
                err: Some(err.to_string()),
                ..Default::default()
            })
            .await;
        }
    }
}

/// Accept WebSocket connections forever, serving each with `service`.
///
/// Every handshake must present `Authorization: Bearer {token}` — the
/// per-start secret the engine published at `{data_dir}/ipc-token`
/// ([`crate::ipc_auth`]). Loopback alone is not an identity: any local
/// process (another OS user, a sandboxed app, a container sharing the host
/// network) can open this port, and the engine behind it can start agents and
/// broker vault grants.
pub async fn serve_ws_listener(listener: TcpListener, service: Arc<dyn RpcService>, token: String) {
    // One full token; an agent token no one holds.
    let tokens = match crate::ipc_auth::generate_token() {
        Ok(agent) => IpcTokens { full: token, agent },
        Err(err) => {
            tracing::error!(error = %err, "rpc: no randomness for the agent token; not serving");
            return;
        }
    };
    serve_ws_listener_scoped(listener, service, tokens).await
}

/// [`serve_ws_listener`] answering two tokens: the full one, and the
/// agent-scoped one, whose connections reach only
/// [`crate::agent_allowed`] methods and are served as
/// [`crate::Caller::Agent`].
pub async fn serve_ws_listener_scoped(listener: TcpListener, service: Arc<dyn RpcService>, tokens: IpcTokens) {
    let tokens = Arc::new(tokens);
    loop {
        match listener.accept().await {
            Ok((stream, peer)) => {
                tracing::debug!(%peer, "rpc: connection accepted");
                // Keystrokes and their echoes are tiny frames: Nagle would
                // hold each one behind the peer's delayed ACK of the previous.
                let _ = stream.set_nodelay(true);
                tokio::spawn(serve_ws_socket(stream, service.clone(), tokens.clone()));
            }
            Err(err) => {
                tracing::warn!(error = %err, "rpc: accept failed");
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        }
    }
}

/// An agent-token connection's view of the service: only what
/// [`crate::agent_allowed`] lists, each served as [`crate::Caller::Agent`].
pub(crate) struct AgentGuard {
    pub(crate) inner: Arc<dyn RpcService>,
    pub(crate) chat: Option<String>,
    /// `chat` is verified (the engine minted the caller's credential for
    /// it), so the methods that act on the calling chat act on it alone.
    pub(crate) pinned: bool,
}

/// Methods that act on the calling chat, named by `chatId`: for a pinned
/// agent ([`agent_service`]), only its own.
const CALLING_CHAT_METHODS: [&str; 3] = [
    crate::methods::PUBLISH_VISUAL,
    crate::methods::PUBLISH_IMAGE,
    crate::methods::AWAIT_BACKGROUND,
];

/// The service as one run's agent reaches it: only [`crate::agent_allowed`]
/// methods, served as [`crate::Caller::Agent`] for `chat` — which, unlike
/// the WS handshake's claimed `x-zend-chat`, the engine itself vouches for
/// (the run's own MCP token named it). What acts on "the calling chat"
/// ([`CALLING_CHAT_METHODS`]) is refused for any other chat.
pub fn agent_service(inner: Arc<dyn RpcService>, chat: String) -> Arc<dyn RpcService> {
    Arc::new(AgentGuard {
        inner,
        chat: Some(chat),
        pinned: true,
    })
}

#[async_trait::async_trait]
impl RpcService for AgentGuard {
    async fn handle(&self, method: &str, params: serde_json::Value) -> Result<crate::RpcReply, RpcError> {
        if !crate::agent_allowed(method) {
            tracing::warn!(method, chat = ?self.chat, "rpc: refused an agent's call outside its scope");
            return Err(RpcError::Failed(format!(
                "{method} isn't open to agents: only the person can do that, in Zend"
            )));
        }
        if self.pinned
            && CALLING_CHAT_METHODS.contains(&method)
            && params.get("chatId").and_then(serde_json::Value::as_str) != self.chat.as_deref()
        {
            tracing::warn!(method, chat = ?self.chat, "rpc: refused an agent's call on another chat");
            return Err(RpcError::Failed(format!(
                "{method} acts on the calling chat only"
            )));
        }
        let caller = crate::Caller::Agent { chat: self.chat.clone() };
        crate::with_caller(caller, self.inner.handle(method, params)).await
    }
}

/// [`serve_ws_listener_scoped`] without a listener: each stream handed in
/// is one connection, its handshake checked as a dialled one is. An app
/// embedding the engine (Android) hands in one end of a socket pair per dial,
/// so no port is open and nothing outside the process can reach it. Serves
/// until every sender is gone.
pub async fn serve_ws_streams<S>(mut streams: mpsc::UnboundedReceiver<S>, service: Arc<dyn RpcService>, tokens: IpcTokens)
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let tokens = Arc::new(tokens);
    while let Some(stream) = streams.recv().await {
        tracing::debug!("rpc: in-process connection");
        tokio::spawn(serve_ws_socket(stream, service.clone(), tokens.clone()));
    }
}

async fn serve_ws_socket<S>(stream: S, service: Arc<dyn RpcService>, tokens: Arc<IpcTokens>)
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    // Two independent gates, both required:
    //
    // 1. Origin. Native viewports dial this socket without an `Origin`
    //    header. A browser always attaches `Origin` to a WebSocket handshake
    //    and cannot forge or suppress it from script, and WebSockets are
    //    exempt from the Same-Origin Policy — so only rejecting any handshake
    //    that carries `Origin` keeps a page the user happens to visit from
    //    reaching this local socket. Keep this check.
    // 2. Bearer. Any local process can still dial without `Origin`; only one
    //    that can read the owner-only `{data_dir}/ipc-token` knows the token.
    //    Compared in constant time; the presented value is never logged.
    //
    // The large `Err` (ErrorResponse) is the shape tungstenite's Callback
    // trait requires; it can't be boxed away here.
    // Set by the handshake when the agent-scoped token was presented: the
    // chat it says it serves.
    let scope: std::sync::Mutex<Option<Option<String>>> = std::sync::Mutex::new(None);
    #[allow(clippy::result_large_err)]
    let authorize = |req: &HandshakeRequest, resp: HandshakeResponse| {
        if let Some(origin) = req.headers().get("origin") {
            tracing::warn!(
                origin = %String::from_utf8_lossy(origin.as_bytes()),
                "rpc: rejecting handshake carrying an Origin header (cross-origin browser dial)"
            );
            let mut err = ErrorResponse::new(Some("origin not allowed on local IPC".to_string()));
            *err.status_mut() = StatusCode::FORBIDDEN;
            return Err(err);
        }
        let presented = req
            .headers()
            .get(tokio_tungstenite::tungstenite::http::header::AUTHORIZATION)
            .map(|value| value.as_bytes());
        let Some(agent) = crate::ipc_auth::bearer_scope(&tokens, presented) else {
            tracing::warn!(
                presented = presented.is_some(),
                "rpc: rejecting IPC handshake without the engine's bearer token"
            );
            let mut err = ErrorResponse::new(Some(
                "missing or wrong IPC token (see {data_dir}/ipc-token)".to_string(),
            ));
            *err.status_mut() = StatusCode::UNAUTHORIZED;
            return Err(err);
        };
        if agent {
            let chat = req
                .headers()
                .get(crate::ipc_auth::CHAT_HEADER)
                .and_then(|value| value.to_str().ok())
                .map(|chat| chat.chars().take(128).collect::<String>());
            *scope.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(chat);
        }
        Ok(resp)
    };
    let ws = match tokio_tungstenite::accept_hdr_async_with_config(
        stream,
        authorize,
        Some(crate::ws::config()),
    )
    .await
    {
        Ok(ws) => ws,
        Err(err) => {
            tracing::warn!(error = %err, "rpc: websocket handshake failed");
            return;
        }
    };
    let (out_tx, out_rx) = mpsc::channel::<String>(256);
    let (in_tx, in_rx) = mpsc::channel::<String>(256);

    // Pump: socket <-> string channels. Ends when either side closes.
    let pump = tokio::spawn(async move {
        if let Some(reason) = crate::ws::pump(ws, out_rx, in_tx).await {
            tracing::warn!(%reason, "rpc: lost a viewport connection");
        }
    });

    let service: Arc<dyn RpcService> = match scope.into_inner().unwrap_or_else(std::sync::PoisonError::into_inner) {
        Some(chat) => Arc::new(AgentGuard { inner: service, chat, pinned: false }),
        None => service,
    };
    serve_connection(service, out_tx, in_rx).await;
    pump.abort();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::time::Duration;

    /// Records the order calls reach it in, by terminal; a call for the
    /// terminal "stuck" never finishes (a write to a program that never
    /// reads).
    struct Recorder(Mutex<Vec<(String, u64)>>);

    #[async_trait::async_trait]
    impl RpcService for Recorder {
        async fn handle(
            &self,
            _method: &str,
            params: serde_json::Value,
        ) -> Result<RpcReply, RpcError> {
            let terminal = params["terminalId"].as_str().unwrap_or_default().to_owned();
            if terminal == "stuck" {
                std::future::pending::<()>().await;
            }
            // (A call that yields first, as a real one may.)
            tokio::task::yield_now().await;
            let n = params["n"].as_u64().unwrap_or_default();
            self.0.lock().unwrap().push((terminal, n));
            Ok(RpcReply::Value(serde_json::json!({})))
        }
    }

    fn connect() -> (Arc<Recorder>, mpsc::Sender<String>, mpsc::Receiver<String>) {
        let recorder = Arc::new(Recorder(Mutex::new(Vec::new())));
        let (to_server, server_in) = mpsc::channel::<String>(4096);
        let (server_out, from_server) = mpsc::channel::<String>(4096);
        tokio::spawn(serve_connection(recorder.clone(), server_out, server_in));
        (recorder, to_server, from_server)
    }

    fn call(id: u64, method: &str, terminal: &str, n: u64) -> String {
        serde_json::json!({ "id": id, "method": method, "params": { "terminalId": terminal, "n": n } })
            .to_string()
    }

    fn seen(recorder: &Recorder, terminal: &str) -> Vec<u64> {
        let calls = recorder.0.lock().unwrap();
        calls.iter().filter(|(t, _)| t == terminal).map(|(_, n)| *n).collect()
    }

    /// A stream handed to `serve_ws_streams` (an app's socket pair) is
    /// served as a dialled connection: with the bearer it answers, without
    /// it the handshake is refused.
    #[cfg(unix)]
    #[tokio::test]
    async fn handed_in_streams_are_served_and_their_handshake_checked() {
        use futures::SinkExt as _;
        use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest, http};
        let recorder = Arc::new(Recorder(Mutex::new(Vec::new())));
        let tokens = IpcTokens::generate().unwrap();
        let full = tokens.full.clone();
        let (tx, rx) = mpsc::unbounded_channel();
        tokio::spawn(serve_ws_streams(rx, recorder.clone(), tokens));
        let dial = |bearer: Option<String>| {
            let (ours, theirs) = tokio::net::UnixStream::pair().unwrap();
            tx.send(theirs).unwrap();
            let mut request = "ws://localhost/".into_client_request().unwrap();
            if let Some(bearer) = bearer {
                request.headers_mut().insert(
                    http::header::AUTHORIZATION,
                    http::HeaderValue::from_str(&format!("Bearer {bearer}")).unwrap(),
                );
            }
            tokio_tungstenite::client_async(request, ours)
        };
        let refused = dial(None).await;
        assert!(
            matches!(&refused, Err(tokio_tungstenite::tungstenite::Error::Http(r)) if r.status() == 401),
            "{refused:?}"
        );
        let (mut ws, _) = dial(Some(full)).await.unwrap();
        ws.send(Message::text(call(1, "Anything", "t", 5))).await.unwrap();
        let reply = tokio::time::timeout(Duration::from_secs(5), ws.next()).await.unwrap().unwrap().unwrap();
        let reply: serde_json::Value = serde_json::from_str(reply.to_text().unwrap().lines().next().unwrap()).unwrap();
        assert_eq!(reply["id"], 1, "{reply}");
        assert_eq!(seen(&recorder, "t"), [5]);
    }

    /// A terminal's input and size reach the service in the order sent,
    /// however many come back to back, two terminals' interleaved.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_terminal_s_writes_and_resizes_arrive_in_the_order_sent() {
        let (recorder, to_server, mut from_server) = connect();
        const CALLS: u64 = 400;
        for n in 0..CALLS {
            let method = if n % 7 == 3 {
                crate::methods::RESIZE_TERMINAL
            } else {
                crate::methods::WRITE_TERMINAL
            };
            let terminal = if n % 3 == 0 { "b" } else { "a" };
            to_server.send(call(n + 1, method, terminal, n)).await.unwrap();
        }
        for _ in 0..CALLS {
            tokio::time::timeout(Duration::from_secs(10), from_server.recv())
                .await
                .expect("every call answered")
                .expect("connection up");
        }
        assert_eq!(seen(&recorder, "a"), (0..CALLS).filter(|n| n % 3 != 0).collect::<Vec<_>>());
        assert_eq!(seen(&recorder, "b"), (0..CALLS).filter(|n| n % 3 == 0).collect::<Vec<_>>());
    }

    /// A write that never finishes holds back its own terminal only.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_stuck_terminal_holds_back_no_other() {
        let (recorder, to_server, mut from_server) = connect();
        let write = crate::methods::WRITE_TERMINAL;
        to_server.send(call(1, write, "stuck", 0)).await.unwrap();
        to_server.send(call(2, write, "stuck", 1)).await.unwrap();
        for n in 0..20 {
            to_server.send(call(10 + n, write, "free", n)).await.unwrap();
        }
        for _ in 0..20 {
            tokio::time::timeout(Duration::from_secs(10), from_server.recv())
                .await
                .expect("the free terminal's calls answered")
                .expect("connection up");
        }
        assert_eq!(seen(&recorder, "free"), (0..20).collect::<Vec<_>>());
        assert!(seen(&recorder, "stuck").is_empty());
    }

    /// A call reusing the wire id of one still running is refused before
    /// it runs: it can't take that id's slot, so cancelling the id still
    /// stops the first call (before, the second one ran, took the slot, and
    /// the first could no longer be cancelled).
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_live_wire_id_reused_is_refused_and_the_first_call_still_cancels() {
        type Log = Arc<Mutex<Vec<u64>>>;
        struct OnDrop(u64, Log);
        impl Drop for OnDrop {
            fn drop(&mut self) {
                self.1.lock().unwrap().push(self.0);
            }
        }
        struct Holder {
            started: Log,
            dropped: Log,
        }
        #[async_trait::async_trait]
        impl RpcService for Holder {
            async fn handle(&self, _method: &str, params: serde_json::Value) -> Result<RpcReply, RpcError> {
                let n = params["n"].as_u64().unwrap_or_default();
                self.started.lock().unwrap().push(n);
                let _dropped = OnDrop(n, self.dropped.clone());
                std::future::pending::<()>().await;
                unreachable!()
            }
        }
        let (started, dropped): (Log, Log) = Default::default();
        let holder = Arc::new(Holder { started: started.clone(), dropped: dropped.clone() });
        let (to_server, server_in) = mpsc::channel::<String>(64);
        let (server_out, _from_server) = mpsc::channel::<String>(64);
        tokio::spawn(serve_connection(holder, server_out, server_in));
        let hold = |id: u64, n: u64| serde_json::json!({ "id": id, "method": "Hold", "params": { "n": n } }).to_string();
        let until = |log: &Log, n: u64| {
            let log = log.clone();
            async move {
                let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
                while !log.lock().unwrap().contains(&n) {
                    if tokio::time::Instant::now() > deadline {
                        return false;
                    }
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
                true
            }
        };
        to_server.send(hold(1, 1)).await.unwrap();
        assert!(until(&started, 1).await);
        to_server.send(hold(1, 2)).await.unwrap();
        // A later call: once it runs, the reused id's frame has been read.
        to_server.send(hold(2, 3)).await.unwrap();
        assert!(until(&started, 3).await);
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!started.lock().unwrap().contains(&2), "the reused id ran: {:?}", started.lock().unwrap());
        to_server.send(serde_json::json!({ "id": 1, "cancel": true }).to_string()).await.unwrap();
        assert!(until(&dropped, 1).await, "cancelling id 1 stopped {:?}", dropped.lock().unwrap());
        assert!(!dropped.lock().unwrap().contains(&3));
    }

    /// Input sent just before the connection ends still goes in, in order.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn input_sent_before_the_connection_ends_still_goes_in() {
        let (recorder, to_server, from_server) = connect();
        for n in 0..50 {
            to_server
                .send(call(n + 1, crate::methods::WRITE_TERMINAL, "a", n))
                .await
                .unwrap();
        }
        drop(to_server);
        drop(from_server);
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        while seen(&recorder, "a").len() < 50 {
            assert!(tokio::time::Instant::now() < deadline, "{:?}", seen(&recorder, "a"));
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert_eq!(seen(&recorder, "a"), (0..50).collect::<Vec<_>>());
    }
}
