//! The phone link: a window on another device (Zend on the owner's phone)
//! driving this engine directly over the tailnet, with no edge between.
//!
//! Three gates, all required, each tested here:
//!
//! 1. Where. The listener binds only a tailnet address ([`bind_remote`]
//!    refuses anything else, `0.0.0.0` included), and a connection from a
//!    peer outside the tailnet's ranges ([`in_tailnet`]) is dropped before
//!    its handshake is read.
//! 2. Who. Both sides prove they hold the phone-link key, a secret of its
//!    own (never the IPC token), without it crossing the wire
//!    ([`crate::link_proof`]): this side answers the handshake's nonce
//!    first, and serves nothing until the phone's first frame proves the
//!    key too. A browser's handshake (`Origin`) is refused as on local IPC.
//! 3. What. Its connections reach only [`remote_allowed`] methods, served as
//!    [`crate::Caller::Remote`]: never [`crate::local_only`] (credentials)
//!    but a plugin's call for its service here and its service's events
//!    (`window_here`: the engine answers and tells only a plugin that's on
//!    and allowed, never another device's, on a channel of the phone's own),
//!    nothing a peer device may not ([`crate::peer_refused`]), and only what
//!    a window needs to show and run chats ([`phone`], [`window`]): a list
//!    of its own, not the agent's token's (an agent may author plugins and
//!    templates, a phone not). No terminals, sign-in, vault, agent accounts,
//!    updates, installs, plugins or templates made or switched, file writes,
//!    folder browsing or stopping the engine.
//!
//! Revoking is the engine's: it stops the listener (every connection it
//! accepted ends with it: they live in its task) and serves a new key.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;

use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::handshake::server::{
    ErrorResponse, Request as HandshakeRequest, Response as HandshakeResponse,
};
use tokio_tungstenite::tungstenite::http::StatusCode;

use crate::{RpcError, RpcService, methods};

/// Whether `ip` is a tailnet address: Tailscale's CGNAT range
/// `100.64.0.0/10` or its ULA range `fd7a:115c:a1e0::/48` (an IPv4 one
/// mapped into IPv6 counts as itself).
pub fn in_tailnet(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => in_tailnet_v4(v4),
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => in_tailnet_v4(v4),
            None => in_tailnet_v6(v6),
        },
    }
}

fn in_tailnet_v4(ip: Ipv4Addr) -> bool {
    let [a, b, ..] = ip.octets();
    a == 100 && (b & 0xc0) == 64
}

fn in_tailnet_v6(ip: Ipv6Addr) -> bool {
    let s = ip.segments();
    s[0] == 0xfd7a && s[1] == 0x115c && s[2] == 0xa1e0
}

/// Bind `addr` for the phone link, only if it's a tailnet address.
pub async fn bind_remote(addr: SocketAddr) -> std::io::Result<TcpListener> {
    if !in_tailnet(addr.ip()) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("{} isn't a tailnet address: the phone link binds only those", addr.ip()),
        ));
    }
    TcpListener::bind(addr).await
}

/// What a window needs beyond what an agent's token reaches: showing chats
/// and their workspace, and running them (messages queued, edited, sent,
/// stopped; attachments uploaded; visuals answered).
fn window(method: &str) -> bool {
    matches!(
        method,
        methods::ENGINE_READY
            | methods::WATCH_QUEUE
            | methods::WATCH_HELD_SENDS
            | methods::WATCH_WAITING_MESSAGES
            | methods::WATCH_SIDEBAR_PREFERENCES
            | methods::WATCH_CONNECTIVITY
            | methods::WATCH_TRANSFERS
            | methods::WATCH_PREVIEWS
            | methods::FOCUS_CHAT
            | methods::SET_VIEWPORT_FOCUS
            | methods::STOP_WAITING
            | methods::RETRY_DELIVERY
            | methods::REMOVE_HELD_SEND
            | methods::MOVE_QUEUED_MESSAGE
            | methods::REMOVE_QUEUED_MESSAGE
            | methods::UPDATE_QUEUED_MESSAGE
            | methods::BEGIN_QUEUED_MESSAGE_EDIT
            | methods::RENEW_QUEUED_MESSAGE_EDIT
            | methods::FINISH_QUEUED_MESSAGE_EDIT
            | methods::SEND_QUEUED_MESSAGE_NOW
            | methods::STEER_QUEUED_MESSAGE_NOW
            | methods::FORK_SIDE_CHAT
            | methods::REWIND_CHAT
            | methods::LIST_OPEN_VISUALS
            | methods::MARK_VISUAL_SENT
            | methods::SAVE_VISUAL_ENTRIES
            | methods::READ_ATTACHMENT_CHUNK
            | methods::READ_CHAT_IMAGE_CHUNK
            | methods::READ_TOOL_IMAGE_CHUNK
            | methods::READ_TOOL_EXCHANGE
            | methods::READ_TOOL_OUTPUT
            | methods::READ_BACKGROUND_TASK_OUTPUT
            | methods::STOP_BACKGROUND_TASK
            | methods::FETCH_TOOL_BLOB
            | methods::UPLOAD_CHUNK
            | methods::UPLOAD_COMMIT
            | methods::LIST_COMMANDS
            | methods::LIST_SKILLS
            | methods::GET_NATIVE_SUBAGENTS
            | methods::GET_TITLE_SETTINGS
            | methods::GET_WORKTREE_SETTINGS
            | methods::LIST_BRANCHES
            | methods::LIST_REFS
            | methods::LIST_PROJECT_ACTIONS
            // The workspace, read: files, search, diffs, history.
            | methods::LIST_WORKSPACE_DIRECTORY
            | methods::READ_WORKSPACE_FILE
            | methods::READ_WORKSPACE_IMAGE
            | methods::SEARCH_WORKSPACE_FILES
            | methods::SEARCH_WORKSPACE_CONTENT
            | methods::WARM_WORKSPACE_SEARCH
            | methods::WATCH_WORKSPACE_FILES
            | methods::WATCH_WORKSPACE_GIT_STATUS
            | methods::WATCH_CHECKOUT_DIFFS
            | methods::GET_CHECKOUT_DIFF
            | methods::GET_CHECKOUT_FILE_DIFF_TEXT
            | methods::LIST_GIT_HISTORY
            | methods::SEARCH_GIT_HISTORY
            | methods::RESOLVE_GIT_AVATARS
            // State a window reads to draw itself.
            | methods::AUTH_STATUS
            | methods::UPDATE_STATUS
            | methods::WATCH_PLUGINS
            // A plugin's window part, file by file (by id and relative path).
            | methods::READ_PLUGIN_FILE
            | methods::VOICE_CAPABILITIES_V2
    )
}

/// What a window asks of this device alone (`local_only`, never for the
/// relay's peers) that a phone's window may ask too: it runs this device's
/// plugins. A plugin's window part asking its service here, and hearing
/// its events: the engine answers only a plugin that's on and allowed
/// here, with what it was allowed here, never sends the call on to another
/// device, and sends the phone only this device's services' events of
/// plugins on and allowed here, on a channel of its own.
fn window_here(method: &str) -> bool {
    matches!(method, methods::CALL_PLUGIN_SERVICE | methods::WATCH_PLUGIN_CHANNEL)
}

/// What a window shows its chats with and runs them by: the devices,
/// spaces, chats and their transcripts, the harnesses and models to pick,
/// chats changed (`Mutate`), messages sent and commands queued (a run, a
/// steer, a stop, an answer: `QueueCommand`, without which a phone can't
/// drive a chat), and the plugins listed (read only).
fn phone(method: &str) -> bool {
    matches!(
        method,
        methods::ENGINE_INFO
            | methods::LOCAL_DEVICE
            | methods::WATCH_DEVICES
            | methods::WATCH_SPACES
            | methods::WATCH_CHATS
            | methods::WATCH_SESSIONS
            | methods::WATCH_DOC_MESSAGES
            | methods::LIST_HARNESSES
            | methods::LIST_MODELS
            | methods::MUTATE
            | methods::QUEUE_MESSAGE
            | methods::QUEUE_COMMAND
            | methods::LIST_PLUGINS
    )
}

/// The methods a phone-link connection may call (see the module's notes).
/// An allow-list of its own: a method added later, to the agent's token's
/// list or anywhere, stays this device's until it's listed here.
pub fn remote_allowed(method: &str, params: &serde_json::Value) -> bool {
    if window_here(method) {
        return true;
    }
    if crate::peer_refused(method, params) {
        return false;
    }
    phone(method) || window(method)
}

/// A phone-link connection's view of the service.
pub(crate) struct RemoteGuard {
    pub(crate) inner: Arc<dyn RpcService>,
}

#[async_trait::async_trait]
impl RpcService for RemoteGuard {
    async fn handle(&self, method: &str, params: serde_json::Value) -> Result<crate::RpcReply, RpcError> {
        if !remote_allowed(method, &params) {
            tracing::warn!(method, "rpc: refused a phone link's call outside its scope");
            return Err(RpcError::Failed(format!(
                "{method} isn't open to a linked phone: do that on the computer"
            )));
        }
        crate::with_caller(crate::Caller::Remote, self.inner.handle(method, params)).await
    }
}

/// Whether a connecting peer may be heard: asked of its address before its
/// handshake is read (the engine asks Tailscale whether it's a node of the
/// tailnet; tests let loopback in).
pub type Admits = fn(IpAddr) -> std::pin::Pin<Box<dyn std::future::Future<Output = bool> + Send>>;

/// Serve the phone link on `listener` (from [`bind_remote`]) with `key`:
/// only peers in the tailnet's ranges, only with the key, only
/// [`remote_allowed`]. Every connection lives in this future: dropping it
/// (aborting its task) ends them all, which is how a new key revokes the old.
pub async fn serve_remote_listener(listener: TcpListener, service: Arc<dyn RpcService>, key: String) {
    serve_remote_listener_admitting(listener, service, key, |ip| Box::pin(std::future::ready(in_tailnet(ip)))).await
}

/// [`serve_remote_listener`] with the peer check given.
pub async fn serve_remote_listener_admitting(
    listener: TcpListener,
    service: Arc<dyn RpcService>,
    key: String,
    admits: Admits,
) {
    let service: Arc<dyn RpcService> = Arc::new(RemoteGuard { inner: service });
    let key: Arc<str> = key.into();
    let mut connections = tokio::task::JoinSet::new();
    loop {
        while connections.try_join_next().is_some() {}
        match listener.accept().await {
            Ok((stream, peer)) => {
                let (service, key) = (service.clone(), key.clone());
                // (In the set: a new key cuts one still being admitted too.)
                connections.spawn(async move {
                    if !admits(peer.ip()).await {
                        tracing::warn!(%peer, "rpc: phone link: refused a peer outside the tailnet");
                        return;
                    }
                    tracing::info!(%peer, "rpc: phone link: connection");
                    let _ = stream.set_nodelay(true);
                    serve_remote_socket(stream, service, key).await;
                });
            }
            Err(err) => {
                tracing::warn!(error = %err, "rpc: phone link: accept failed");
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        }
    }
}

/// How long a phone has to send its proof once the handshake is done.
const PROOF_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

async fn serve_remote_socket<S>(stream: S, service: Arc<dyn RpcService>, key: Arc<str>)
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    use crate::link_proof;
    use futures::StreamExt as _;
    use tokio_tungstenite::tungstenite::{Message, http::HeaderValue};

    // What the phone's first frame must carry, once the handshake set it.
    let expected = std::sync::Mutex::new(None::<String>);
    #[allow(clippy::result_large_err)]
    let authorize = |req: &HandshakeRequest, mut resp: HandshakeResponse| {
        let refuse = |status: StatusCode, why: &str| {
            let mut err = ErrorResponse::new(Some(why.to_string()));
            *err.status_mut() = status;
            err
        };
        if req.headers().get("origin").is_some() {
            return Err(refuse(StatusCode::FORBIDDEN, "origin not allowed"));
        }
        let Some(phone_nonce) = req
            .headers()
            .get(link_proof::NONCE_HEADER)
            .and_then(|v| v.to_str().ok())
            .filter(|n| link_proof::valid_nonce(n))
        else {
            // (A phone that sends the key as a bearer predates the
            // challenge; whatever it sent isn't looked at.)
            tracing::warn!("rpc: phone link: refused a handshake without a challenge");
            return Err(refuse(StatusCode::UNAUTHORIZED, "the phone link needs its challenge: update Zend on the phone"));
        };
        let Ok(desktop_nonce) = link_proof::nonce() else {
            return Err(refuse(StatusCode::INTERNAL_SERVER_ERROR, "no randomness"));
        };
        let proof = link_proof::desktop_proof(&key, phone_nonce, &desktop_nonce);
        for (name, value) in [(link_proof::NONCE_HEADER, &desktop_nonce), (link_proof::PROOF_HEADER, &proof)] {
            let value = HeaderValue::from_str(value).map_err(|_| refuse(StatusCode::INTERNAL_SERVER_ERROR, "header"))?;
            resp.headers_mut().insert(name, value);
        }
        *expected.lock().unwrap() = Some(link_proof::phone_proof(&key, phone_nonce, &desktop_nonce));
        Ok(resp)
    };
    let mut ws = match tokio_tungstenite::accept_hdr_async_with_config(stream, authorize, Some(crate::ws::config())).await {
        Ok(ws) => ws,
        Err(err) => {
            tracing::debug!(error = %err, "rpc: phone link: handshake failed");
            return;
        }
    };
    let Some(expected) = expected.into_inner().ok().flatten() else {
        return;
    };
    let first = tokio::time::timeout(PROOF_TIMEOUT, ws.next()).await;
    if !matches!(&first, Ok(Some(Ok(Message::Text(frame)))) if link_proof::frame_proves(frame, &expected)) {
        // (Never what was presented.)
        tracing::warn!("rpc: phone link: refused a phone that didn't prove the key");
        let _ = ws.close(None).await;
        return;
    }
    let (out_tx, out_rx) = mpsc::channel::<String>(256);
    let (in_tx, in_rx) = mpsc::channel::<String>(256);
    let mut pump = tokio::spawn(async move {
        if let Some(reason) = crate::ws::pump(ws, out_rx, in_tx).await {
            tracing::info!(%reason, "rpc: phone link: connection lost");
        }
    });
    // (Aborted with this task: the pump goes too.)
    struct AbortOnDrop(tokio::task::AbortHandle);
    impl Drop for AbortOnDrop {
        fn drop(&mut self) {
            self.0.abort();
        }
    }
    let _pump_guard = AbortOnDrop(pump.abort_handle());
    tokio::select! {
        () = crate::server::serve_connection(service, out_tx, in_rx) => {}
        _ = &mut pump => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RpcReply;
    use futures::{SinkExt as _, StreamExt as _};
    use std::time::Duration;
    use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest, http};

    /// Answers every call with the caller it was served as.
    struct Echo;

    #[async_trait::async_trait]
    impl RpcService for Echo {
        async fn handle(&self, method: &str, _params: serde_json::Value) -> Result<RpcReply, RpcError> {
            Ok(RpcReply::Value(serde_json::json!({ "method": method, "caller": format!("{:?}", crate::caller()) })))
        }
    }

    #[test]
    fn only_tailnet_addresses_are_the_tailnet() {
        for ip in ["100.64.0.1", "100.102.122.72", "100.127.255.254", "fd7a:115c:a1e0::1", "fd7a:115c:a1e0:ab12::5", "::ffff:100.100.1.1"] {
            assert!(in_tailnet(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["0.0.0.0", "127.0.0.1", "100.63.255.255", "100.128.0.1", "192.168.1.5", "10.0.0.1", "::", "::1", "fd7a:115c:a1e1::1", "fe80::1", "::ffff:127.0.0.1"] {
            assert!(!in_tailnet(ip.parse().unwrap()), "{ip}");
        }
    }

    #[tokio::test]
    async fn the_link_binds_no_address_outside_the_tailnet() {
        for addr in ["0.0.0.0:0", "127.0.0.1:0", "[::]:0", "192.168.1.5:0"] {
            let refused = bind_remote(addr.parse().unwrap()).await;
            assert_eq!(refused.unwrap_err().kind(), std::io::ErrorKind::InvalidInput, "{addr}");
        }
    }

    #[test]
    fn a_linked_phone_reaches_a_window_s_methods_and_no_credentials() {
        let none = serde_json::Value::Null;
        for open in [methods::WATCH_SESSIONS, methods::WATCH_CHATS, methods::WATCH_DOC_MESSAGES, methods::MUTATE, methods::QUEUE_MESSAGE, methods::ENGINE_INFO, methods::READ_TOOL_OUTPUT] {
            assert!(remote_allowed(open, &none), "{open}");
        }
        for closed in [
            methods::ACCESS_TOKEN,
            methods::STEP_UP,
            methods::DELETE_ACCOUNT,
            methods::SIGN_IN,
            methods::SIGN_OUT,
            methods::STOP_ENGINE,
            methods::OPEN_TERMINAL,
            methods::WRITE_TERMINAL,
            methods::VAULT_STATUS,
            methods::VAULT_AUTHORIZE,
            methods::APPLY_UPDATE,
            methods::INSTALL_HARNESS,
            methods::WRITE_WORKSPACE_FILE,
            methods::LIST_FOLDERS,
            methods::DECIDE_PLUGIN,
            methods::NEW_PLUGIN,
            methods::PLUGIN_SIGNING_KEY,
            methods::PHONE_LINK_STATUS,
            methods::SET_PHONE_LINK,
            methods::NEW_PHONE_LINK_KEY,
            // What an agent's token reaches and a window doesn't need.
            methods::LIST_AGENT_ACCOUNTS,
            methods::INSTALL_PLUGIN,
            methods::EXPORT_PLUGIN,
            methods::PLUGIN_INPUT,
            methods::CALL_PLUGIN_TOOL,
            methods::CREATE_TEMPLATE,
            methods::USE_TEMPLATE,
            methods::IMPORT_TEMPLATE,
            methods::DELETE_TEMPLATE,
            methods::CREATE_THEME,
            methods::DELETE_THEME,
            methods::PUBLISH_VISUAL,
            methods::DEVICE_STATS,
            "SomethingAddedLater",
        ] {
            assert!(!remote_allowed(closed, &none), "{closed}");
        }
        // Every local_only method, whatever the lists say, but a plugin's
        // service here (its engine's checks: plugins/client_tests.rs).
        let src = include_str!("lib.rs");
        let consts: std::collections::HashMap<&str, &str> = src
            .lines()
            .filter_map(|l| l.trim().strip_prefix("pub const ")?.split_once(": &str = \""))
            .map(|(name, rest)| (name, rest.trim_end_matches("\";")))
            .collect();
        let local: Vec<&str> = consts.values().copied().filter(|m| crate::local_only(m)).collect();
        assert!(local.len() > 30, "{}", local.len());
        for m in local {
            assert_eq!(remote_allowed(m, &none), window_here(m), "{m}");
        }
        for m in [methods::CALL_PLUGIN_SERVICE, methods::WATCH_PLUGIN_CHANNEL] {
            assert!(window_here(m) && crate::local_only(m), "{m}: still never the relay's");
        }
        // A peer's rule: no path on this device to install from.
        assert!(!remote_allowed(methods::INSTALL_PLUGIN, &serde_json::json!({ "path": "/tmp/x" })));
    }

    fn anyone(_: IpAddr) -> std::pin::Pin<Box<dyn std::future::Future<Output = bool> + Send>> {
        Box::pin(std::future::ready(true))
    }

    fn tailnet(ip: IpAddr) -> std::pin::Pin<Box<dyn std::future::Future<Output = bool> + Send>> {
        Box::pin(std::future::ready(in_tailnet(ip)))
    }

    async fn serving(key: &str, admits: Admits) -> (SocketAddr, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let task = tokio::spawn(serve_remote_listener_admitting(listener, Arc::new(Echo), key.into(), admits));
        (addr, task)
    }

    type Ws = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

    /// The handshake as a phone before the challenge made it: the key as a
    /// bearer.
    async fn dial_with_bearer(addr: SocketAddr, bearer: &str) -> Result<Ws, tokio_tungstenite::tungstenite::Error> {
        let mut request = format!("ws://{addr}/").into_client_request().unwrap();
        request
            .headers_mut()
            .insert(http::header::AUTHORIZATION, http::HeaderValue::from_str(&format!("Bearer {bearer}")).unwrap());
        tokio::time::timeout(Duration::from_secs(5), tokio_tungstenite::connect_async(request))
            .await
            .expect("dial timed out")
            .map(|(ws, _)| ws)
    }

    /// A handshake with the phone's nonce: the socket, its nonce and what
    /// the desktop answered (its nonce and proof).
    async fn challenge(addr: SocketAddr) -> Result<(Ws, String, Option<String>, Option<String>), tokio_tungstenite::tungstenite::Error> {
        let nonce = crate::link_proof::nonce().unwrap();
        let mut request = format!("ws://{addr}/").into_client_request().unwrap();
        request.headers_mut().insert(crate::link_proof::NONCE_HEADER, http::HeaderValue::from_str(&nonce).unwrap());
        let (ws, response) = tokio::time::timeout(Duration::from_secs(5), tokio_tungstenite::connect_async(request))
            .await
            .expect("dial timed out")?;
        let header = |name: &str| response.headers().get(name).map(|v| v.to_str().unwrap().to_owned());
        Ok((ws, nonce, header(crate::link_proof::NONCE_HEADER), header(crate::link_proof::PROOF_HEADER)))
    }

    /// Dial as a phone holding `key` does: the desktop's proof checked,
    /// then the phone's sent. Err("unproved") when the desktop's doesn't
    /// match (nothing sent).
    async fn link(addr: SocketAddr, key: &str) -> Result<Ws, String> {
        let (mut ws, nonce, desktop_nonce, proof) = challenge(addr).await.map_err(|e| e.to_string())?;
        let (Some(desktop_nonce), Some(proof)) = (desktop_nonce, proof) else {
            return Err("no challenge".into());
        };
        if !crate::link_proof::proof_matches(&crate::link_proof::desktop_proof(key, &nonce, &desktop_nonce), &proof) {
            return Err("unproved".into());
        }
        ws.send(Message::text(crate::link_proof::phone_frame(key, &nonce, &desktop_nonce))).await.map_err(|e| e.to_string())?;
        Ok(ws)
    }

    /// Whether the socket ends without a text frame.
    async fn cut<S>(ws: &mut tokio_tungstenite::WebSocketStream<S>) -> bool
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        let next = tokio::time::timeout(Duration::from_secs(5), ws.next()).await.expect("connection not cut");
        !matches!(next, Some(Ok(Message::Text(_))))
    }

    async fn ask<S>(ws: &mut tokio_tungstenite::WebSocketStream<S>, id: u64, method: &str) -> serde_json::Value
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        ws.send(Message::text(serde_json::json!({ "id": id, "method": method, "params": {} }).to_string()))
            .await
            .unwrap();
        let reply = tokio::time::timeout(Duration::from_secs(5), ws.next()).await.unwrap().unwrap().unwrap();
        serde_json::from_str(reply.to_text().unwrap().lines().next().unwrap()).unwrap()
    }

    #[tokio::test]
    async fn a_peer_outside_the_tailnet_is_refused_even_with_the_key() {
        let key = crate::ipc_auth::generate_token().unwrap();
        let (addr, _task) = serving(&key, tailnet).await;
        assert!(link(addr, &key).await.is_err());
    }

    #[tokio::test]
    async fn only_the_key_opens_the_link_and_its_caller_is_remote() {
        let key = crate::ipc_auth::generate_token().unwrap();
        let ipc = crate::ipc_auth::generate_token().unwrap();
        let (addr, _task) = serving(&key, anyone).await;
        // The key as a bearer (a phone before the challenge): refused, as
        // is a handshake with no challenge at all.
        for bearer in [key.as_str(), ipc.as_str()] {
            let refused = dial_with_bearer(addr, bearer).await;
            assert!(
                matches!(&refused, Err(tokio_tungstenite::tungstenite::Error::Http(r)) if r.status() == 401),
                "{refused:?}"
            );
        }
        // A phone holding another key: this desktop doesn't prove it, so
        // the phone sends nothing.
        for wrong in [ipc.as_str(), &key[..63]] {
            assert_eq!(link(addr, wrong).await.err().as_deref(), Some("unproved"));
        }
        let mut ws = link(addr, &key).await.unwrap();
        let open = ask(&mut ws, 1, methods::WATCH_SESSIONS).await;
        assert_eq!(open["ok"]["caller"], "Remote", "{open}");
        let closed = ask(&mut ws, 2, methods::ACCESS_TOKEN).await;
        assert!(closed["err"].is_object() || closed["err"].is_string(), "{closed}");
        assert!(closed.get("ok").is_none(), "{closed}");
        // The async client does the same.
        let client = crate::connect_phone_link(&format!("ws://{addr}"), &key).await.unwrap();
        assert_eq!(client.call(methods::ENGINE_INFO, serde_json::json!({})).await.unwrap()["caller"], "Remote");
        assert!(crate::connect_phone_link(&format!("ws://{addr}"), &ipc).await.is_err());
    }

    /// The answer to the challenge holds no key, and a phone that doesn't
    /// prove the key (no proof, a wrong one, a call instead) is served
    /// nothing: its socket ends.
    #[tokio::test]
    async fn a_phone_that_doesn_t_prove_the_key_is_served_nothing() {
        let key = crate::ipc_auth::generate_token().unwrap();
        let (addr, _task) = serving(&key, anyone).await;
        for first in [
            |_: &str, _: &str, _: &str| serde_json::json!({ "id": 1, "method": methods::ENGINE_INFO, "params": {} }).to_string(),
            |_: &str, np: &str, nd: &str| crate::link_proof::phone_frame(&crate::ipc_auth::generate_token().unwrap(), np, nd),
            // The desktop's proof sent back: the sides are labelled apart.
            |key: &str, np: &str, nd: &str| serde_json::json!({ "linkProof": crate::link_proof::desktop_proof(key, np, nd) }).to_string(),
        ] {
            let (mut ws, nonce, desktop_nonce, proof) = challenge(addr).await.unwrap();
            let (desktop_nonce, proof) = (desktop_nonce.unwrap(), proof.unwrap());
            assert!(!proof.contains(&key) && !desktop_nonce.contains(&key));
            assert!(crate::link_proof::proof_matches(&crate::link_proof::desktop_proof(&key, &nonce, &desktop_nonce), &proof));
            ws.send(Message::text(first(&key, &nonce, &desktop_nonce))).await.unwrap();
            let _ = ws.send(Message::text(serde_json::json!({ "id": 2, "method": methods::ENGINE_INFO, "params": {} }).to_string())).await;
            assert!(cut(&mut ws).await);
        }
    }

    /// Dialling something that isn't the desktop (a wrong DNS answer, a
    /// stranger at the address) gives it neither the key nor a proof made
    /// with it: the phone checks the desktop's proof before it sends.
    #[tokio::test]
    async fn an_impostor_desktop_learns_nothing_of_the_key() {
        let key = crate::ipc_auth::generate_token().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let heard = Arc::new(std::sync::Mutex::new(String::new()));
        let record = heard.clone();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let record = record.clone();
                tokio::spawn(async move {
                    let headers = record.clone();
                    let note = move |req: &HandshakeRequest, mut resp: HandshakeResponse| {
                        headers.lock().unwrap().push_str(&format!("{:?}", req.headers()));
                        // Answers the challenge as it can: without the key.
                        let fake = crate::link_proof::nonce().unwrap();
                        resp.headers_mut().insert(crate::link_proof::NONCE_HEADER, http::HeaderValue::from_str(&fake).unwrap());
                        resp.headers_mut().insert(crate::link_proof::PROOF_HEADER, http::HeaderValue::from_str(&fake).unwrap());
                        Ok::<_, ErrorResponse>(resp)
                    };
                    let Ok(mut ws) = tokio_tungstenite::accept_hdr_async(stream, note).await else { return };
                    while let Some(Ok(frame)) = ws.next().await {
                        record.lock().unwrap().push_str(&format!("{frame:?}"));
                    }
                });
            }
        });
        let url = format!("ws://{addr}");
        assert!(crate::connect_phone_link(&url, &key).await.is_err());
        assert_eq!(link(addr, &key).await.err().as_deref(), Some("unproved"));
        tokio::time::sleep(Duration::from_millis(200)).await;
        let heard = heard.lock().unwrap().clone();
        assert!(heard.contains(crate::link_proof::NONCE_HEADER), "{heard}");
        assert!(!heard.contains(&key), "the key reached the impostor: {heard}");
        assert!(!heard.to_ascii_lowercase().contains("authorization"), "{heard}");
        assert!(!heard.contains("linkProof"), "a proof reached the impostor: {heard}");
    }

    /// Holds every call until it's dropped, and says so.
    struct Blocks(Arc<std::sync::atomic::AtomicUsize>, Arc<std::sync::atomic::AtomicUsize>);

    #[async_trait::async_trait]
    impl RpcService for Blocks {
        async fn handle(&self, _method: &str, _params: serde_json::Value) -> Result<RpcReply, RpcError> {
            struct Count(Arc<std::sync::atomic::AtomicUsize>);
            impl Drop for Count {
                fn drop(&mut self) {
                    self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }
            }
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let _ended = Count(self.1.clone());
            std::future::pending::<()>().await;
            unreachable!()
        }
    }

    /// A call still running as the listener ends (a new key) ends with it:
    /// nothing a revoked phone started runs on.
    #[tokio::test]
    async fn a_call_in_flight_ends_with_the_listener() {
        use std::sync::atomic::Ordering::SeqCst;
        let key = crate::ipc_auth::generate_token().unwrap();
        let (started, ended) = (Arc::new(std::sync::atomic::AtomicUsize::new(0)), Arc::new(std::sync::atomic::AtomicUsize::new(0)));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let task = tokio::spawn(serve_remote_listener_admitting(
            listener,
            Arc::new(Blocks(started.clone(), ended.clone())),
            key.clone(),
            anyone,
        ));
        let mut ws = link(addr, &key).await.unwrap();
        ws.send(Message::text(serde_json::json!({ "id": 1, "method": methods::WATCH_SESSIONS, "params": {} }).to_string()))
            .await
            .unwrap();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while started.load(SeqCst) == 0 {
            assert!(tokio::time::Instant::now() < deadline, "the call never started");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(ended.load(SeqCst), 0);
        task.abort();
        let _ = task.await;
        while ended.load(SeqCst) == 0 {
            assert!(tokio::time::Instant::now() < deadline, "the call outlived its listener");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(cut(&mut ws).await);
    }

    /// Ending the listener (a new key) ends every connection it took.
    #[tokio::test]
    async fn stopping_the_listener_cuts_its_phones() {
        let key = crate::ipc_auth::generate_token().unwrap();
        let (addr, task) = serving(&key, anyone).await;
        let mut ws = link(addr, &key).await.unwrap();
        assert_eq!(ask(&mut ws, 1, methods::ENGINE_INFO).await["id"], 1);
        task.abort();
        let _ = task.await;
        assert!(cut(&mut ws).await);
        assert!(link(addr, &key).await.is_err());
    }
}
