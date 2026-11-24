//! The peer link of an engine: its node key, pairing by code, the book
//! (peers.json), grants and unpairing, over loopback (admitted as if from
//! the tailnet).

use super::*;
use zend_rpc::{Caller, RpcClient, with_caller};

/// Answers every call with the caller it was served as.
struct Echo;

#[async_trait]
impl RpcService for Echo {
    async fn handle(&self, method: &str, _params: Value) -> Result<RpcReply, RpcError> {
        Ok(RpcReply::Value(json!({ "method": method, "caller": format!("{:?}", zend_rpc::caller()) })))
    }
}

fn loopback() -> Seams {
    Seams {
        discover: || {
            Box::pin(std::future::ready(Some(Tailnet { name: None, ips: vec!["127.0.0.1".parse().unwrap()] })))
        },
        bind: |addr| Box::pin(tokio::net::TcpListener::bind(addr)),
        admits: |_| Box::pin(std::future::ready(true)),
    }
}

struct Side {
    _dir: tempfile::TempDir,
    data: PathBuf,
    link: Arc<PeerLink>,
    rpc: RpcClient,
}

fn side() -> Side {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().to_path_buf();
    let link = PeerLink::new(&data, 0, Arc::new(Echo), loopback());
    let rpc = zend_rpc::memory_client(Arc::new(PeerLinkRpc { inner: Arc::new(Echo), link: link.clone() }));
    Side { _dir: dir, data, link, rpc }
}

async fn pair(a: &Side, b: &Side) -> String {
    let code = a.rpc.call(methods::PEER_PAIR, json!({})).await.unwrap();
    let code = code["code"].as_str().unwrap().to_string();
    b.rpc.call(methods::PEER_JOIN, json!({ "code": code })).await.unwrap();
    code
}

fn kept(data: &Path) -> Vec<String> {
    let saved: Saved = serde_json::from_slice(&std::fs::read(data.join(PEERS_FILE)).unwrap()).unwrap();
    saved.peers.into_iter().map(|p| p.key).collect()
}

#[cfg(unix)]
fn mode(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

/// The node key is made once, kept mode 600, and the same across starts.
#[tokio::test]
async fn the_node_key_is_kept_owner_only_and_stays() {
    let a = side();
    let first = a.link.node_key().unwrap().public().to_string();
    #[cfg(unix)]
    assert_eq!(mode(&a.data.join(KEY_FILE)), 0o600);
    let again = PeerLink::new(&a.data, 0, Arc::new(Echo), loopback());
    assert_eq!(again.node_key().unwrap().public(), first);
    assert!(zend_rpc::peer_link::valid_key(&first));
}

#[test]
fn a_code_reads_back_and_rejects_what_isn_t_one() {
    let code = Code { key: "ab".repeat(32), host: "fd7a:115c::1".into(), port: 4801, secret: "s".repeat(64) };
    let parsed: Code = code.to_string().parse().unwrap();
    assert_eq!(parsed, code);
    assert_eq!(parsed.url(), "ws://[fd7a:115c::1]:4801");
    for bad in ["", "zend-peer:1:", "zend-peer:2:x", &code.to_string().replace(&"ab".repeat(32), "nothex")] {
        assert!(bad.parse::<Code>().is_err(), "{bad}");
    }
}

/// A code pairs: both sides keep each other's key in peers.json (mode
/// 600), with no grants; the joiner then dials in as itself.
#[tokio::test]
async fn joining_with_a_code_pairs_both_sides() {
    let (a, b) = (side(), side());
    pair(&a, &b).await;
    let (ka, kb) = (a.link.node_key().unwrap(), b.link.node_key().unwrap());
    assert_eq!(kept(&a.data), vec![kb.public().to_string()]);
    assert_eq!(kept(&b.data), vec![ka.public().to_string()]);
    #[cfg(unix)]
    assert_eq!((mode(&a.data.join(PEERS_FILE)), mode(&b.data.join(PEERS_FILE))), (0o600, 0o600));
    assert!(a.link.peers()[0].grants.is_empty() && b.link.peers()[0].grants.is_empty());

    let listing = b.rpc.call(methods::LIST_PEERS, json!({})).await.unwrap();
    let peer = &listing["peers"][0];
    let url = format!("ws://{}:{}", peer["host"].as_str().unwrap(), peer["port"]);
    a.rpc.call(methods::SET_PEER_GRANTS, json!({ "peer": &kb.public()[..8], "grants": ["sessions.read"] })).await.unwrap();
    let client = zend_rpc::connect_peer(&url, &kb, ka.public()).await.unwrap();
    let reply = client.call(methods::WATCH_CHATS, json!({})).await.unwrap();
    assert!(reply["caller"].as_str().unwrap().contains(kb.public()), "{reply}");
}

/// A code works once; an expired one never; a made-up one never.
#[tokio::test]
async fn a_reused_expired_or_wrong_code_is_refused() {
    let (a, b, c) = (side(), side(), side());
    let code = pair(&a, &b).await;
    let reused = c.rpc.call(methods::PEER_JOIN, json!({ "code": code })).await;
    assert!(reused.is_err(), "{reused:?}");

    let fresh = a.rpc.call(methods::PEER_PAIR, json!({})).await.unwrap();
    for (_, until) in a.link.book.codes.lock().unwrap().iter_mut() {
        *until = Instant::now() - Duration::from_secs(1);
    }
    let expired = c.rpc.call(methods::PEER_JOIN, json!({ "code": fresh["code"] })).await;
    assert!(expired.is_err(), "{expired:?}");

    let mut wrong: Code = fresh["code"].as_str().unwrap().parse().unwrap();
    wrong.secret = "0".repeat(64);
    let wrong = c.rpc.call(methods::PEER_JOIN, json!({ "code": wrong.to_string() })).await;
    assert!(wrong.is_err(), "{wrong:?}");
    assert_eq!(kept(&a.data).len(), 1, "only b paired");
    assert!(!c.data.join(PEERS_FILE).exists() || kept(&c.data).is_empty());
}

/// A code naming another key than the server at its address holds: the
/// joiner pairs nothing (the callee proves its key first).
#[tokio::test]
async fn a_code_with_the_wrong_key_pairs_nothing() {
    let (a, b) = (side(), side());
    let code = a.rpc.call(methods::PEER_PAIR, json!({})).await.unwrap();
    let mut code: Code = code["code"].as_str().unwrap().parse().unwrap();
    code.key = NodeKey::generate().unwrap().public().to_string();
    let refused = b.rpc.call(methods::PEER_JOIN, json!({ "code": code.to_string() })).await;
    assert!(refused.as_ref().is_err_and(|e| e.to_string().contains("didn't prove")), "{refused:?}");
    assert!(a.link.peers().is_empty() && b.link.peers().is_empty());
}

/// Unpairing removes the entry and ends the server's live connection.
#[tokio::test]
async fn unpairing_removes_the_entry_and_drops_its_connections() {
    let (a, b) = (side(), side());
    pair(&a, &b).await;
    let (ka, kb) = (a.link.node_key().unwrap(), b.link.node_key().unwrap());
    a.link.set_grants(kb.public(), &["sessions.read".into()]).unwrap();
    let port = a.link.status().await.unwrap()["port"].as_u64().unwrap();
    let client = zend_rpc::connect_peer(&format!("ws://127.0.0.1:{port}"), &kb, ka.public()).await.unwrap();
    assert!(client.call(methods::WATCH_CHATS, json!({})).await.is_ok());
    a.rpc.call(methods::UNPAIR_PEER, json!({ "peer": kb.public() })).await.unwrap();
    assert!(kept(&a.data).is_empty());
    let mut dropped = false;
    for _ in 0..100 {
        if client.call(methods::WATCH_CHATS, json!({})).await.is_err() {
            dropped = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(dropped, "the connection outlived unpairing");
    assert!(zend_rpc::connect_peer(&format!("ws://127.0.0.1:{port}"), &kb, ka.public()).await.is_err());
}

/// Pairing and grants are this device's own: refused from an agent's
/// token, a paired server, a phone or the relay, even with every grant.
#[tokio::test]
async fn changing_grants_is_local_only() {
    let (a, b) = (side(), side());
    pair(&a, &b).await;
    let kb = b.link.node_key().unwrap().public().to_string();
    let all = Grants::new(["sessions.read", "sessions.run:p", "projects.share:p", "workers.manage"].iter().map(|g| g.parse().unwrap()));
    let service = PeerLinkRpc { inner: Arc::new(Echo), link: a.link.clone() };
    for caller in [
        Caller::Agent { chat: Some("chat-1".into()) },
        Caller::PeerServer { peer: kb.clone(), grants: all.clone() },
        Caller::Remote,
        Caller::Peer { device: Some("device-2".into()) },
    ] {
        for (method, params) in [
            (methods::SET_PEER_GRANTS, json!({ "peer": kb, "grants": ["sessions.run:p"] })),
            (methods::UNPAIR_PEER, json!({ "peer": kb })),
            (methods::PEER_PAIR, json!({})),
            (methods::LIST_PEERS, json!({})),
        ] {
            let refused = with_caller(caller.clone(), service.handle(method, params)).await;
            assert!(refused.as_ref().is_err_and(|e| e.to_string().contains("this device's own")), "{caller:?} {method}: {:?}", refused.err());
        }
    }
    assert!(a.link.peers()[0].grants.is_empty());
    // Over a real peer connection too, with every grant.
    a.link.set_grants(&kb, &all.iter().map(ToString::to_string).collect::<Vec<_>>()).unwrap();
    let port = a.link.status().await.unwrap()["port"].as_u64().unwrap();
    let ka = a.link.node_key().unwrap().public().to_string();
    let client = zend_rpc::connect_peer(&format!("ws://127.0.0.1:{port}"), &b.link.node_key().unwrap(), &ka).await.unwrap();
    let refused = client.call(methods::SET_PEER_GRANTS, json!({ "peer": kb, "grants": [] })).await;
    assert!(refused.is_err(), "{refused:?}");
    assert_eq!(a.link.peers()[0].grants, all);
    // And from this device's own window it works.
    a.rpc.call(methods::SET_PEER_GRANTS, json!({ "peer": kb, "grants": [] })).await.unwrap();
    assert!(a.link.peers()[0].grants.is_empty());
}
