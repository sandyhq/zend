//! A linked phone (the phone-link key, `Caller::Remote`) reads only its
//! chats' folders: never a file outside them, and never Zend's own data
//! (the IPC tokens, the phone-link key, agent accounts), even when a chat
//! works in a folder that holds it. Served through the real phone listener.

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use zend_engine::{EngineCore, HarnessRegistry};
use zend_rpc::{RpcClient, RpcError, methods};

fn assemble(data_dir: &Path, device_id: &str) -> EngineCore {
    std::fs::create_dir_all(data_dir).expect("data dir");
    std::fs::write(data_dir.join("device-id"), device_id).expect("device id");
    EngineCore::assemble(data_dir, Arc::new(HarnessRegistry::new()), zend_proto::HarnessId::Mock, None)
        .expect("engine assembles")
}

/// The engine behind the phone listener (on loopback, admitted as if from
/// the tailnet), and a phone dialled in with the key.
async fn phone(core: &EngineCore) -> (RpcClient, tokio::task::JoinHandle<()>) {
    let key = zend_rpc::ipc_auth::generate_token().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    let task = tokio::spawn(zend_rpc::remote::serve_remote_listener_admitting(
        listener,
        core.rpc_service(),
        key.clone(),
        |_| Box::pin(std::future::ready(true)),
    ));
    let client = tokio::time::timeout(Duration::from_secs(5), zend_rpc::connect_phone_link(&format!("ws://{addr}"), &key))
        .await
        .expect("dial timed out")
        .expect("phone dials in");
    (client, task)
}

fn secrets(data: &Path) {
    std::fs::write(data.join("ipc-token"), "IPC-TOKEN-SECRET\n").unwrap();
    std::fs::write(data.join("phone-link-key"), "PHONE-KEY-SECRET\n").unwrap();
    std::fs::create_dir_all(data.join("agent-accounts/codex")).unwrap();
    std::fs::write(data.join("agent-accounts/codex/slot.json"), "{\"token\":\"ACCOUNT-SECRET\"}\n").unwrap();
}

fn leaked(reply: &Result<serde_json::Value, RpcError>) -> bool {
    match reply {
        Ok(value) => {
            let text = value.to_string();
            text.contains("SECRET") || text.contains("U0VDUkVU") // "SECRET" in base64
                || value.get("text").is_some_and(|t| !t.is_null())
                || value.get("data").is_some_and(|d| d.as_str().is_some_and(|d| !d.is_empty()))
        }
        Err(_) => false,
    }
}

/// A chat working in a folder that holds Zend's data dir (a chat from
/// before scratch folders runs in home, where `~/.zend` is): the phone
/// reads none of it, by relative path, absolute path, listing, search or
/// attachment chunk. The computer's own window still can.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_phone_reads_no_secret_even_in_a_folder_that_holds_zend_s_data() {
    let temp = tempfile::tempdir().unwrap();
    let home = std::fs::canonicalize(temp.path()).unwrap().join("home");
    let data = home.join(".zend");
    let core = assemble(&data, "device-phone-reads");
    secrets(&data);
    std::fs::write(home.join("notes.md"), "notes\n").unwrap();
    core.workspace.create_chat("chat-home", None, Some(&core.device_id), None, None).unwrap();
    core.workspace.set_chat_cwd("chat-home", &home.to_string_lossy()).unwrap();
    let (phone, _listener) = phone(&core).await;

    for path in [".zend/ipc-token", ".zend/phone-link-key", ".zend/agent-accounts/codex/slot.json"] {
        let reply = phone.call(methods::READ_WORKSPACE_FILE, serde_json::json!({ "chatId": "chat-home", "path": path })).await;
        assert!(!leaked(&reply), "relative {path}: {reply:?}");
        let absolute = home.join(path).to_string_lossy().into_owned();
        let reply = phone.call(methods::READ_WORKSPACE_FILE, serde_json::json!({ "chatId": "chat-home", "path": absolute })).await;
        assert!(!leaked(&reply), "absolute {path}: {reply:?}");
        let reply = phone.call(methods::READ_ATTACHMENT_CHUNK, serde_json::json!({ "path": absolute, "offset": 0 })).await;
        assert!(!leaked(&reply), "attachment {path}: {reply:?}");
    }
    let listing = phone.call(methods::LIST_WORKSPACE_DIRECTORY, serde_json::json!({ "chatId": "chat-home", "directory": ".zend" })).await;
    assert!(listing.is_err(), "listing .zend: {listing:?}");
    let search = phone
        .call(methods::SEARCH_WORKSPACE_CONTENT, serde_json::json!({ "chatId": "chat-home", "query": "SECRET", "includeIgnored": true }))
        .await;
    assert!(!leaked(&search) && search.as_ref().map_or(true, |s| s["matches"].as_array().is_none_or(Vec::is_empty)), "search: {search:?}");

    // This device's own window is unchanged.
    let local = zend_rpc::memory_client(core.rpc_service());
    let read = local.call(methods::READ_WORKSPACE_FILE, serde_json::json!({ "chatId": "chat-home", "path": "notes.md" })).await.unwrap();
    assert_eq!(read["text"], "notes\n");
}

/// A project chat: the phone reads its files, and nothing outside them by
/// absolute path (another folder, Zend's data dir), as file or image.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_phone_reads_its_chat_s_files_and_nothing_outside() {
    let temp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(temp.path()).unwrap();
    let repo = root.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::write(repo.join("README.md"), "hello\n").unwrap();
    let outside = root.join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("report.md"), "OUTSIDE-SECRET\n").unwrap();
    std::fs::write(outside.join("pixel.png"), b"\x89PNG\r\n\x1a\nSECRET").unwrap();
    let data = root.join("data");
    let core = assemble(&data, "device-phone-outside");
    secrets(&data);
    core.workspace.create_space("space-phone", &core.device_id, &repo.to_string_lossy(), None, true).unwrap();
    core.workspace.create_chat("chat-phone", Some("space-phone"), None, None, None).unwrap();
    let (phone, _listener) = phone(&core).await;

    let read = phone.call(methods::READ_WORKSPACE_FILE, serde_json::json!({ "chatId": "chat-phone", "path": "README.md" })).await.unwrap();
    assert_eq!(read["text"], "hello\n");
    let inside = repo.join("README.md").to_string_lossy().into_owned();
    let read = phone.call(methods::READ_WORKSPACE_FILE, serde_json::json!({ "chatId": "chat-phone", "path": inside })).await.unwrap();
    assert_eq!(read["text"], "hello\n");

    for path in [outside.join("report.md"), data.join("ipc-token"), data.join("phone-link-key"), data.join("agent-accounts/codex/slot.json")] {
        let path = path.to_string_lossy().into_owned();
        let reply = phone.call(methods::READ_WORKSPACE_FILE, serde_json::json!({ "chatId": "chat-phone", "path": path })).await;
        assert!(!leaked(&reply), "{path}: {reply:?}");
        let reply = phone.call(methods::READ_ATTACHMENT_CHUNK, serde_json::json!({ "path": path, "offset": 0 })).await;
        assert!(!leaked(&reply), "attachment {path}: {reply:?}");
    }
    let png = outside.join("pixel.png").to_string_lossy().into_owned();
    let image = phone
        .call(methods::READ_WORKSPACE_IMAGE, serde_json::json!({ "chatId": "chat-phone", "path": png, "expectedCheckoutId": "", "offset": 0 }))
        .await;
    assert!(!leaked(&image), "image: {image:?}");

    // The computer's own window still reads a host file outside, read-only.
    let local = zend_rpc::memory_client(core.rpc_service());
    let path = outside.join("report.md").to_string_lossy().into_owned();
    let read = local.call(methods::READ_WORKSPACE_FILE, serde_json::json!({ "chatId": "chat-phone", "path": path })).await.unwrap();
    assert_eq!(read["text"], "OUTSIDE-SECRET\n");
}
