//! The phone link: a window on another device (Zend on the owner's phone)
//! driving this engine directly over the tailnet, with no edge between.
//!
//! Three gates, all required, each tested here:
//!
//! 1. Where. The listener binds only a tailnet address ([`bind_remote`]
//!    refuses anything else, `0.0.0.0` included), and a connection from a
//!    peer outside the tailnet's ranges ([`in_tailnet`]) is dropped before
//!    its handshake is read.
//! 2. Who. The handshake must carry `Authorization: Bearer <key>`: the
//!    phone-link key, a secret of its own (never the IPC token), compared in
//!    constant time. A browser's handshake (`Origin`) is refused as on
//!    local IPC.
//! 3. What. Its connections reach only [`remote_allowed`] methods, served as
//!    [`crate::Caller::Remote`]: never [`crate::local_only`] (credentials),
//!    nothing a peer device may not ([`crate::peer_refused`]), and beyond
//!    what an agent's token reaches only what a window needs to show and
//!    run chats ([`WINDOW`]). No terminals, sign-in, vault, updates,
//!    installs, file writes, folder browsing or stopping the engine.
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
            | methods::VOICE_CAPABILITIES_V2
    )
}

/// The methods a phone-link connection may call (see the module's notes).
/// An allow-list: a method added later stays this device's until it's
/// listed.
pub fn remote_allowed(method: &str, params: &serde_json::Value) -> bool {
    if crate::peer_refused(method, params) {
        return false;
    }
    crate::agent_allowed(method) || window(method)
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

/// Serve the phone link on `listener` (from [`bind_remote`]) with `key`:
/// only peers in the tailnet, only with the key, only [`remote_allowed`].
/// Every connection lives in this future: dropping it (aborting its task)
/// ends them all, which is how a new key revokes the old.
pub async fn serve_remote_listener(listener: TcpListener, service: Arc<dyn RpcService>, key: String) {
    serve_remote_listener_admitting(listener, service, key, in_tailnet).await
}

/// [`serve_remote_listener`] with the peer check given (tests dial from
/// loopback).
pub async fn serve_remote_listener_admitting(
    listener: TcpListener,
    service: Arc<dyn RpcService>,
    key: String,
    admits: fn(IpAddr) -> bool,
) {
    let service: Arc<dyn RpcService> = Arc::new(RemoteGuard { inner: service });
    let key: Arc<str> = key.into();
    let mut connections = tokio::task::JoinSet::new();
    loop {
        while connections.try_join_next().is_some() {}
        match listener.accept().await {
            Ok((stream, peer)) => {
                if !admits(peer.ip()) {
                    tracing::warn!(%peer, "rpc: phone link: refused a peer outside the tailnet");
                    drop(stream);
                    continue;
                }
                tracing::info!(%peer, "rpc: phone link: connection");
                let _ = stream.set_nodelay(true);
                connections.spawn(serve_remote_socket(stream, service.clone(), key.clone()));
            }
            Err(err) => {
                tracing::warn!(error = %err, "rpc: phone link: accept failed");
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        }
    }
}

async fn serve_remote_socket<S>(stream: S, service: Arc<dyn RpcService>, key: Arc<str>)
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    #[allow(clippy::result_large_err)]
    let authorize = |req: &HandshakeRequest, resp: HandshakeResponse| {
        if req.headers().get("origin").is_some() {
            let mut err = ErrorResponse::new(Some("origin not allowed".to_string()));
            *err.status_mut() = StatusCode::FORBIDDEN;
            return Err(err);
        }
        let presented = req
            .headers()
            .get(tokio_tungstenite::tungstenite::http::header::AUTHORIZATION)
            .map(|value| value.as_bytes());
        if !crate::ipc_auth::bearer_matches(&key, presented) {
            // (Never what was presented.)
            tracing::warn!(presented = presented.is_some(), "rpc: phone link: refused a handshake without the key");
            let mut err = ErrorResponse::new(Some("wrong or missing phone-link key".to_string()));
            *err.status_mut() = StatusCode::UNAUTHORIZED;
            return Err(err);
        }
        Ok(resp)
    };
    let ws = match tokio_tungstenite::accept_hdr_async_with_config(stream, authorize, Some(crate::ws::config())).await {
        Ok(ws) => ws,
        Err(err) => {
            tracing::debug!(error = %err, "rpc: phone link: handshake failed");
            return;
        }
    };
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
            "SomethingAddedLater",
        ] {
            assert!(!remote_allowed(closed, &none), "{closed}");
        }
        // Every local_only method, whatever the lists say.
        let src = include_str!("lib.rs");
        let consts: std::collections::HashMap<&str, &str> = src
            .lines()
            .filter_map(|l| l.trim().strip_prefix("pub const ")?.split_once(": &str = \""))
            .map(|(name, rest)| (name, rest.trim_end_matches("\";")))
            .collect();
        let local: Vec<&str> = consts.values().copied().filter(|m| crate::local_only(m)).collect();
        assert!(local.len() > 30, "{}", local.len());
        for m in local {
            assert!(!remote_allowed(m, &none), "{m}");
        }
        // A peer's rule: no path on this device to install from.
        assert!(!remote_allowed(methods::INSTALL_PLUGIN, &serde_json::json!({ "path": "/tmp/x" })));
    }

    async fn serving(key: &str, admits: fn(IpAddr) -> bool) -> (SocketAddr, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let task = tokio::spawn(serve_remote_listener_admitting(listener, Arc::new(Echo), key.into(), admits));
        (addr, task)
    }

    async fn dial(
        addr: SocketAddr,
        bearer: Option<&str>,
    ) -> Result<
        tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
        tokio_tungstenite::tungstenite::Error,
    > {
        let mut request = format!("ws://{addr}/").into_client_request().unwrap();
        if let Some(bearer) = bearer {
            request
                .headers_mut()
                .insert(http::header::AUTHORIZATION, http::HeaderValue::from_str(&format!("Bearer {bearer}")).unwrap());
        }
        tokio::time::timeout(Duration::from_secs(5), tokio_tungstenite::connect_async(request))
            .await
            .expect("dial timed out")
            .map(|(ws, _)| ws)
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
        let (addr, _task) = serving(&key, in_tailnet).await;
        assert!(dial(addr, Some(&key)).await.is_err());
    }

    #[tokio::test]
    async fn only_the_key_opens_the_link_and_its_caller_is_remote() {
        let key = crate::ipc_auth::generate_token().unwrap();
        let ipc = crate::ipc_auth::generate_token().unwrap();
        let (addr, _task) = serving(&key, |_| true).await;
        for wrong in [None, Some(ipc.as_str()), Some(&key[..63])] {
            let refused = dial(addr, wrong).await;
            assert!(
                matches!(&refused, Err(tokio_tungstenite::tungstenite::Error::Http(r)) if r.status() == 401),
                "{refused:?}"
            );
        }
        let mut ws = dial(addr, Some(&key)).await.unwrap();
        let open = ask(&mut ws, 1, methods::WATCH_SESSIONS).await;
        assert_eq!(open["ok"]["caller"], "Remote", "{open}");
        let closed = ask(&mut ws, 2, methods::ACCESS_TOKEN).await;
        assert!(closed["err"].is_object() || closed["err"].is_string(), "{closed}");
        assert!(closed.get("ok").is_none(), "{closed}");
    }

    /// Ending the listener (a new key) ends every connection it took.
    #[tokio::test]
    async fn stopping_the_listener_cuts_its_phones() {
        let key = crate::ipc_auth::generate_token().unwrap();
        let (addr, task) = serving(&key, |_| true).await;
        let mut ws = dial(addr, Some(&key)).await.unwrap();
        assert_eq!(ask(&mut ws, 1, methods::ENGINE_INFO).await["id"], 1);
        task.abort();
        let _ = task.await;
        let next = tokio::time::timeout(Duration::from_secs(5), ws.next()).await.expect("connection not cut");
        assert!(!matches!(next, Some(Ok(Message::Text(_)))), "{next:?}");
        assert!(dial(addr, Some(&key)).await.is_err());
    }
}
