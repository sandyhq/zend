use super::*;

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
        discover: || Box::pin(async { Some(Tailnet { name: Some("omarchy.tailf80f26.ts.net".into()), ips: vec!["127.0.0.1".parse().unwrap()] }) }),
        bind: |addr| Box::pin(tokio::net::TcpListener::bind(addr)),
        admits: |_| true,
    }
}

fn no_tailnet() -> Seams {
    Seams { discover: || Box::pin(async { None }), ..loopback() }
}

fn make_link(dir: &Path, seams: Seams) -> Arc<PhoneLink> {
    // (Port 0: the system's choice; the status says which.)
    PhoneLink::new(dir, 0, Arc::new(Echo), seams)
}

async fn dial(status: &Value, key: &str) -> Result<zend_rpc::RpcClient, RpcError> {
    let url = format!("ws://{}", status["listening"][0].as_str().unwrap());
    tokio::time::timeout(Duration::from_secs(5), zend_rpc::connect_ws(&url, Some(key))).await.expect("dial timed out")
}

#[test]
fn the_tailnet_is_read_from_tailscale_s_status() {
    let status = r#"{"BackendState":"Running","Self":{"DNSName":"omarchy.tailf80f26.ts.net.","TailscaleIPs":["100.102.122.72","fd7a:115c:a1e0::6532:7a49"]}}"#;
    assert_eq!(
        parse_status(status),
        Some(Tailnet {
            name: Some("omarchy.tailf80f26.ts.net".into()),
            ips: vec!["100.102.122.72".parse().unwrap(), "fd7a:115c:a1e0::6532:7a49".parse().unwrap()],
        })
    );
    // Stopped, logged out, or no tailnet address: no tailnet.
    assert_eq!(parse_status(r#"{"BackendState":"Stopped","Self":{"TailscaleIPs":["100.102.122.72"]}}"#), None);
    assert_eq!(parse_status(r#"{"BackendState":"Running","Self":{"TailscaleIPs":["192.168.1.2"]}}"#), None);
    assert_eq!(parse_status("not json"), None);
    // A name a code can't hold is left out; the address stays.
    let odd = parse_status(r#"{"Self":{"DNSName":"a b/c.","TailscaleIPs":["100.64.0.9"]}}"#).unwrap();
    assert_eq!(odd, Tailnet { name: None, ips: vec!["100.64.0.9".parse().unwrap()] });
}

#[tokio::test]
async fn turned_on_it_serves_its_own_key_scoped_as_a_phone() {
    let dir = tempfile::tempdir().unwrap();
    let link = make_link(dir.path(), loopback());
    let off = link.get().await;
    assert_eq!(off["on"], false);
    assert!(off.get("key").is_none(), "{off}");
    assert_eq!(off["name"], "omarchy.tailf80f26.ts.net");

    let on = link.set(Some(true), None).await.unwrap();
    assert_eq!(on["on"], true);
    assert_eq!(on["host"], "omarchy.tailf80f26.ts.net");
    let key = on["key"].as_str().unwrap().to_owned();
    assert!(key.len() == 64 && key.bytes().all(|b| b.is_ascii_hexdigit()), "{key}");
    // Its own key, not the IPC token, owner-only.
    assert!(!dir.path().join("ipc-token").exists());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(dir.path().join(KEY_FILE)).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    let phone = dial(&on, &key).await.unwrap();
    let answer = phone.call(methods::WATCH_SESSIONS, json!({})).await.unwrap();
    assert_eq!(answer["caller"], "Remote");
    // Credentials and this device's own (the link's key itself) stay here.
    for local in [methods::ACCESS_TOKEN, methods::PHONE_LINK_STATUS, methods::NEW_PHONE_LINK_KEY, methods::STOP_ENGINE] {
        assert!(phone.call(local, json!({})).await.is_err(), "{local}");
    }
    assert!(dial(&on, &"0".repeat(64)).await.is_err());

    // The IP instead of the name, kept.
    let ip = link.set(None, Some(Use::Ip)).await.unwrap();
    assert_eq!(ip["host"], "127.0.0.1");
    assert_eq!(ip["key"], key.as_str());
    let again = make_link(dir.path(), loopback());
    assert_eq!(again.saved(), Saved { on: true, use_: Use::Ip });
    assert_eq!(again.key().unwrap(), key);
}

#[tokio::test]
async fn a_new_key_cuts_and_refuses_the_old_phone() {
    let dir = tempfile::tempdir().unwrap();
    let link = make_link(dir.path(), loopback());
    let on = link.set(Some(true), None).await.unwrap();
    let old = on["key"].as_str().unwrap().to_owned();
    let phone = dial(&on, &old).await.unwrap();
    assert!(phone.call(methods::ENGINE_INFO, json!({})).await.is_ok());

    let renewed = link.renew().await.unwrap();
    let new = renewed["key"].as_str().unwrap().to_owned();
    assert_ne!(new, old);
    tokio::time::timeout(Duration::from_secs(5), phone.closed()).await.expect("the old phone wasn't cut");
    assert!(dial(&renewed, &old).await.is_err(), "the old key still opens it");
    let fresh = dial(&renewed, &new).await.unwrap();
    assert!(fresh.call(methods::ENGINE_INFO, json!({})).await.is_ok());
}

#[tokio::test]
async fn turned_off_nothing_listens_and_without_a_tailnet_nothing_starts() {
    let dir = tempfile::tempdir().unwrap();
    let link = make_link(dir.path(), loopback());
    let on = link.set(Some(true), None).await.unwrap();
    let key = on["key"].as_str().unwrap().to_owned();
    let off = link.set(Some(false), None).await.unwrap();
    assert_eq!(off["on"], false);
    assert!(off.get("key").is_none());
    assert!(dial(&on, &key).await.is_err(), "still listening when off");
    assert!(!link.saved().on);

    let dir = tempfile::tempdir().unwrap();
    let lonely = make_link(dir.path(), no_tailnet());
    let refused = lonely.set(Some(true), None).await.unwrap_err();
    assert_eq!(refused.to_string(), "no_tailnet");
    let status = lonely.get().await;
    assert_eq!(status["on"], false);
    assert!(status["listening"].as_array().unwrap().is_empty());
    assert!(!lonely.saved().on);
}

#[tokio::test]
async fn on_at_the_last_stop_it_is_on_again_at_the_start() {
    let dir = tempfile::tempdir().unwrap();
    {
        let link = make_link(dir.path(), loopback());
        link.set(Some(true), None).await.unwrap();
    }
    let link = make_link(dir.path(), loopback());
    link.resume();
    let mut status = link.get().await;
    for _ in 0..50 {
        if status["on"] == true {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
        status = link.get().await;
    }
    assert_eq!(status["on"], true, "{status}");
}

#[tokio::test]
async fn only_this_device_s_window_asks_for_the_key() {
    let dir = tempfile::tempdir().unwrap();
    let rpc = PhoneLinkRpc { inner: Arc::new(Echo), link: make_link(dir.path(), loopback()) };
    for caller in [zend_rpc::Caller::Remote, zend_rpc::Caller::Agent { chat: None }, zend_rpc::Caller::Peer { device: None }] {
        for method in [methods::PHONE_LINK_STATUS, methods::SET_PHONE_LINK, methods::NEW_PHONE_LINK_KEY] {
            let asked = zend_rpc::with_caller(caller.clone(), rpc.handle(method, json!({ "on": true }))).await;
            assert!(asked.is_err(), "{caller:?} {method}");
        }
    }
    let RpcReply::Value(status) = rpc.handle(methods::PHONE_LINK_STATUS, json!({})).await.unwrap() else { panic!() };
    assert_eq!(status["on"], false);
}
