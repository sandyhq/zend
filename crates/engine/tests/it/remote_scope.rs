//! What another Zend reaches of this device's files: a linked phone
//! (`Caller::Remote`, the real phone listener) and another device through
//! the relay (`Caller::Peer`) get one scope, resolved here: a path they name
//! is served only when it resolves (canonical, symlinks followed) inside a
//! folder of one of this device's chats or projects, and that folder isn't
//! the home folder, above it, or one holding Zend's data. Inside it, the
//! diffs, git status, branches, refs, history and files work as for this
//! device's own window, worktree chats included (their `.git` names a git
//! dir elsewhere: git reads it, only the caller's path is fenced).

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use zend_engine::{EngineCore, HarnessRegistry};
use zend_rpc::{RpcClient, RpcError, RpcReply, RpcService, methods};

async fn git(cwd: &Path, args: &[&str]) {
    let output = tokio::process::Command::new("git")
        .args(args)
        .current_dir(cwd)
        .env("GIT_AUTHOR_NAME", "test")
        .env("GIT_AUTHOR_EMAIL", "test@test")
        .env("GIT_COMMITTER_NAME", "test")
        .env("GIT_COMMITTER_EMAIL", "test@test")
        .output()
        .await
        .expect("git spawns");
    assert!(output.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&output.stderr));
}

/// A repo at `dir` with `file` committed as `committed`, then changed to
/// `changed` (so it has a diff).
async fn repo(dir: &Path, file: &str, committed: &str, changed: &str) {
    std::fs::create_dir_all(dir).unwrap();
    git(dir, &["init", "-b", "main"]).await;
    std::fs::write(dir.join(file), committed).unwrap();
    git(dir, &["add", "."]).await;
    git(dir, &["commit", "-m", "initial secret history"]).await;
    std::fs::write(dir.join(file), changed).unwrap();
}

fn assemble(data_dir: &Path) -> EngineCore {
    std::fs::create_dir_all(data_dir).unwrap();
    EngineCore::assemble(data_dir, Arc::new(HarnessRegistry::new()), zend_proto::HarnessId::Mock, None)
        .expect("engine assembles")
}

/// The service as the device-room relay serves it to another device.
struct AsPeer(Arc<dyn RpcService>);

#[async_trait::async_trait]
impl RpcService for AsPeer {
    async fn handle(&self, method: &str, params: serde_json::Value) -> Result<RpcReply, RpcError> {
        let caller = zend_rpc::Caller::Peer { device: Some("device-other".into()) };
        zend_rpc::with_caller(caller, self.0.handle(method, params)).await
    }
}

/// A phone dialled in over the real phone listener (on loopback, admitted
/// as if from the tailnet), and another device as the relay serves it.
async fn others(core: &EngineCore) -> Vec<(&'static str, RpcClient)> {
    let key = zend_rpc::ipc_auth::generate_token().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    tokio::spawn(zend_rpc::remote::serve_remote_listener_admitting(
        listener,
        core.rpc_service(),
        key.clone(),
        |_| Box::pin(std::future::ready(true)),
    ));
    let phone = tokio::time::timeout(Duration::from_secs(5), zend_rpc::connect_phone_link(&format!("ws://{addr}"), &key))
        .await
        .expect("dial timed out")
        .expect("phone dials in");
    let peer = zend_rpc::memory_client(Arc::new(AsPeer(core.rpc_service())));
    vec![("phone", phone), ("peer", peer)]
}

/// Every path-taking git call a window makes, for `folder`.
async fn git_calls(client: &RpcClient, folder: &str, checkout_id: &str, checksum: &str, file: &str) -> Vec<(&'static str, Result<serde_json::Value, RpcError>)> {
    use serde_json::json;
    vec![
        ("ListBranches", client.call(methods::LIST_BRANCHES, json!({ "repoPath": folder })).await),
        ("ListRefs", client.call(methods::LIST_REFS, json!({ "repoPath": folder })).await),
        ("GetCheckoutDiff", client.call(methods::GET_CHECKOUT_DIFF, json!({ "cwd": folder })).await),
        (
            "GetCheckoutFileDiffText",
            client
                .call(
                    methods::GET_CHECKOUT_FILE_DIFF_TEXT,
                    json!({ "checkoutId": checkout_id, "cwd": folder, "path": file, "diffChecksum": checksum }),
                )
                .await,
        ),
        ("ListGitHistory", client.call(methods::LIST_GIT_HISTORY, json!({ "cwd": folder })).await),
        ("SearchGitHistory", client.call(methods::SEARCH_GIT_HISTORY, json!({ "cwd": folder, "query": "secret" })).await),
        (
            "ResolveGitAvatars",
            client.call(methods::RESOLVE_GIT_AVATARS, json!({ "cwd": folder, "authors": [] })).await,
        ),
    ]
}

/// The checkout id and diff checksum this device's own window gets for `folder`.
async fn local_diff(core: &EngineCore, folder: &str) -> (String, String) {
    let local = zend_rpc::memory_client(core.rpc_service());
    let diff = local
        .call(methods::GET_CHECKOUT_DIFF, serde_json::json!({ "cwd": folder }))
        .await
        .expect("the window gets the diff");
    (diff["checkoutId"].as_str().unwrap().into(), diff["checksum"].as_str().unwrap().into())
}

/// Frames of a diff watch, until one passes `until` or `wait` is up.
async fn diff_frames(
    client: &RpcClient,
    chat: &str,
    wait: Duration,
    until: impl Fn(&[zend_proto::CheckoutDiff]) -> bool,
) -> Vec<Vec<zend_proto::CheckoutDiff>> {
    let mut stream = client
        .subscribe(methods::WATCH_CHECKOUT_DIFFS, serde_json::json!({ "chatId": chat }))
        .await
        .unwrap();
    let mut frames = Vec::new();
    let deadline = tokio::time::Instant::now() + wait;
    while let Ok(Some(value)) = tokio::time::timeout_at(deadline, stream.recv()).await {
        let frame: Vec<zend_proto::CheckoutDiff> = serde_json::from_value(value).unwrap();
        let done = until(&frame);
        frames.push(frame);
        if done {
            break;
        }
    }
    frames
}

/// The first git status a watch of `chat`'s file browser sends, if any
/// arrives within `wait`.
async fn git_status(client: &RpcClient, chat: &str, wait: Duration) -> Option<zend_proto::CheckoutGitStatus> {
    let mut stream = client
        .subscribe_checked(methods::WATCH_WORKSPACE_GIT_STATUS, serde_json::json!({ "chatId": chat }))
        .await
        .unwrap_or_else(|error| panic!("git status of {chat}: {error:?}"));
    tokio::time::timeout(wait, async {
        loop {
            let frame: zend_proto::WorkspaceGitStatusFrame =
                serde_json::from_value(stream.recv().await?).unwrap();
            if frame.status.is_some() {
                return frame.status;
            }
        }
    })
    .await
    .ok()
    .flatten()
}

/// A repo outside every chat's and project's folder (standing in for
/// `~/.dotfiles`), named directly or through a symlink inside a project:
/// neither the phone nor another device reads its branches, refs, diff,
/// history or files. This device's own window still does.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_repo_outside_every_chat_s_folder_is_refused_to_phone_and_peer() {
    let temp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(temp.path()).unwrap();
    let dotfiles = root.join("dotfiles");
    repo(&dotfiles, "secret.txt", "DOTFILES-SECRET\n", "DOTFILES-SECRET-CHANGED\n").await;
    let project = root.join("project");
    repo(&project, "a.txt", "one\n", "two\n").await;
    std::os::unix::fs::symlink(&dotfiles, project.join("escape")).unwrap();
    let core = assemble(&root.join("data"));
    core.workspace.create_space("space", &core.device_id, &project.to_string_lossy(), None, true).unwrap();
    core.workspace.create_chat("chat", Some("space"), None, None, None).unwrap();
    core.workspace.set_chat_cwd("chat", &project.to_string_lossy()).unwrap();

    let outside = dotfiles.to_string_lossy().into_owned();
    let (checkout_id, checksum) = local_diff(&core, &outside).await;
    let through_link = project.join("escape").to_string_lossy().into_owned();
    let mut served = Vec::new();
    for (who, client) in others(&core).await {
        for (via, folder) in [("direct", &outside), ("symlink", &through_link)] {
            for (method, reply) in git_calls(&client, folder, &checkout_id, &checksum, "secret.txt").await {
                if reply.is_ok() {
                    served.push(format!("{who} {method} {via}"));
                }
            }
        }
        // A host file outside the chat's folder, by absolute path.
        let secret = dotfiles.join("secret.txt").to_string_lossy().into_owned();
        let read = client
            .call(methods::READ_WORKSPACE_FILE, serde_json::json!({ "chatId": "chat", "path": secret }))
            .await;
        if read.as_ref().is_ok_and(|value| value.to_string().contains("SECRET")) {
            served.push(format!("{who} ReadWorkspaceFile absolute"));
        }
    }
    assert!(served.is_empty(), "served outside the scope: {served:?}");
    let local = zend_rpc::memory_client(core.rpc_service());
    let branches = local.call(methods::LIST_BRANCHES, serde_json::json!({ "repoPath": outside })).await;
    assert!(branches.is_ok(), "the window lists them: {branches:?}");
}

/// A chat working in a folder that holds Zend's data (a chat from before
/// scratch folders runs in home, which may be a git checkout of dotfiles):
/// its diff, git status and history stay on this device, from the watch
/// that streams every checkout too.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_too_wide_chat_folder_s_diffs_stay_on_this_device() {
    let temp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(temp.path()).unwrap();
    let home = root.join("home");
    repo(&home, "notes.md", "HOME-SECRET\n", "HOME-SECRET-CHANGED\n").await;
    std::fs::write(home.join(".gitignore"), ".zend/\n").unwrap();
    let core = assemble(&home.join(".zend"));
    core.workspace.create_chat("chat-home", None, Some(&core.device_id), None, None).unwrap();
    core.workspace.set_chat_cwd("chat-home", &home.to_string_lossy()).unwrap();

    let local = zend_rpc::memory_client(core.rpc_service());
    let frames = diff_frames(&local, "chat-home", Duration::from_secs(15), |frame| {
        frame.iter().any(|diff| diff.patch.contains("HOME-SECRET"))
    })
    .await;
    assert!(
        frames.iter().flatten().any(|diff| diff.patch.contains("HOME-SECRET")),
        "the window sees the home chat's diff: {frames:?}"
    );
    let folder = home.to_string_lossy().into_owned();
    let (checkout_id, checksum) = local_diff(&core, &folder).await;
    let mut served = Vec::new();
    for (who, client) in others(&core).await {
        for (method, reply) in git_calls(&client, &folder, &checkout_id, &checksum, "notes.md").await {
            if reply.is_ok() {
                served.push(format!("{who} {method}"));
            }
        }
        let frames = diff_frames(&client, "chat-home", Duration::from_secs(2), |_| false).await;
        if frames.iter().flatten().any(|diff| diff.patch.contains("SECRET")) {
            served.push(format!("{who} WatchCheckoutDiffs"));
        }
        let status = client
            .subscribe_checked(methods::WATCH_WORKSPACE_GIT_STATUS, serde_json::json!({ "chatId": "chat-home" }))
            .await;
        if status.is_ok() {
            served.push(format!("{who} WatchWorkspaceGitStatus"));
        }
    }
    assert!(served.is_empty(), "served from a too wide folder: {served:?}");
}

/// The phone and another device see what their chats work on as this
/// device's window does: a project chat, a project chat in a worktree of
/// it, and a chat in a worktree whose main repository is no chat's folder
/// (its `.git` file names a git dir outside the chat's folder). Each one's
/// diff, a file's diff text, branches, refs, history, the diff watch, git
/// status and its files; a subfolder of the chat's folder too.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn phone_and_peer_see_their_chats_diffs_files_and_history_worktrees_included() {
    let temp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(temp.path()).unwrap();
    let project = root.join("project");
    repo(&project, "a.txt", "one\n", "project change\n").await;
    std::fs::create_dir_all(project.join("sub")).unwrap();
    let project_tree = root.join("project-tree");
    git(&project, &["worktree", "add", &project_tree.to_string_lossy(), "-b", "tree"]).await;
    std::fs::write(project_tree.join("a.txt"), "project tree change\n").unwrap();
    let main = root.join("main");
    repo(&main, "a.txt", "one\n", "main change\n").await;
    let tree = root.join("tree");
    git(&main, &["worktree", "add", &tree.to_string_lossy(), "-b", "feature"]).await;
    std::fs::write(tree.join("a.txt"), "tree change\n").unwrap();
    for worktree in [&tree, &project_tree] {
        assert!(worktree.join(".git").is_file(), "a worktree's .git is a file");
    }

    let core = assemble(&root.join("data"));
    core.workspace.create_space("space", &core.device_id, &project.to_string_lossy(), None, true).unwrap();
    for (chat, space, folder) in [
        ("chat-project", Some("space"), &project),
        ("chat-project-tree", Some("space"), &project_tree),
        ("chat-tree", None, &tree),
    ] {
        let device = space.is_none().then_some(core.device_id.as_str());
        core.workspace.create_chat(chat, space, device, None, None).unwrap();
        core.workspace.set_chat_cwd(chat, &folder.to_string_lossy()).unwrap();
    }

    let clients = others(&core).await;
    for (chat, folder, change) in [
        ("chat-project", &project, "project change"),
        ("chat-project-tree", &project_tree, "project tree change"),
        ("chat-tree", &tree, "tree change"),
    ] {
        let folder_text = folder.to_string_lossy().into_owned();
        let (checkout_id, checksum) = local_diff(&core, &folder_text).await;
        for (who, client) in &clients {
            for (method, reply) in git_calls(client, &folder_text, &checkout_id, &checksum, "a.txt").await {
                let reply = reply.unwrap_or_else(|error| panic!("{who} {method} {chat}: {error:?}"));
                if method == "GetCheckoutDiff" {
                    assert!(reply["patch"].as_str().unwrap().contains(change), "{who} {chat}: {reply}");
                }
                if method == "ListGitHistory" {
                    assert!(reply.to_string().contains("initial secret history"), "{who} {chat}: {reply}");
                }
            }
            if folder == &project {
                let sub = folder.join("sub").to_string_lossy().into_owned();
                let diff = client.call(methods::GET_CHECKOUT_DIFF, serde_json::json!({ "cwd": sub })).await;
                assert!(diff.is_ok(), "{who} a subfolder: {diff:?}");
            }
            let read = client
                .call(methods::READ_WORKSPACE_FILE, serde_json::json!({ "chatId": chat, "path": "a.txt" }))
                .await
                .unwrap_or_else(|error| panic!("{who} reads {chat}'s file: {error:?}"));
            assert_eq!(read["text"], format!("{change}\n"));
            let frames = diff_frames(client, chat, Duration::from_secs(15), |frame| {
                frame.iter().any(|diff| diff.cwd == folder_text && diff.patch.contains(change))
            })
            .await;
            assert!(
                frames.iter().flatten().any(|diff| diff.cwd == folder_text && diff.patch.contains(change)),
                "{who} watches {chat}'s diff: {frames:?}"
            );
            // Git status as the window gets it (a projectless chat's file
            // browser is a plain folder: none, for the window too).
            let local = zend_rpc::memory_client(core.rpc_service());
            let wait = Duration::from_secs(if chat == "chat-tree" { 3 } else { 15 });
            let expected = git_status(&local, chat, wait).await;
            assert_eq!(expected.is_some(), chat != "chat-tree", "the window's git status of {chat}");
            let got = git_status(client, chat, wait).await;
            assert_eq!(
                got.map(|status| status.files[0].path.clone()),
                expected.map(|status| status.files[0].path.clone()),
                "{who} git status of {chat}"
            );
        }
    }
}
