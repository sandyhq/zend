//! Branches and worktrees named and renamed by the person (plan
//! `plans/branch-naming/plan.md` in zend-home-repo, K1–K6): a worktree takes
//! the name it is given, a new branch is switched to in a local checkout,
//! a branch or worktree is renamed with every chat in it following, never
//! while a turn runs there, and a moved chat's next turn keeps its history.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use futures::stream::BoxStream;

use zend_doc::{MessageRole, MessageStatus, SessionCommandPayload, SessionMessageEntry};
use zend_engine::{EngineCore, HarnessRegistry};
use zend_harness::{Harness, HarnessError, RunControls};
use zend_proto::{
    AgentEvent, DoneStatus, HarnessId, Model, ReasoningLevel, RunRequest, SandboxLevel,
    SteeringMode, WorktreeSpec,
};
use zend_rpc::methods;

/// What one run was started with.
#[derive(Clone, Debug)]
struct Seen {
    cwd: String,
    prompt: String,
    resume: Option<String>,
}

/// Records each run; a prompt saying "hold" waits for `release` before
/// its turn ends (a turn running, for the refusals).
struct Recorder {
    seen: Arc<Mutex<Vec<Seen>>>,
    release: Arc<tokio::sync::Notify>,
}

#[async_trait]
impl Harness for Recorder {
    fn id(&self) -> HarnessId {
        HarnessId::Mock
    }
    fn display_name(&self) -> &str {
        "Recorder"
    }
    fn supports_steering(&self) -> bool {
        false
    }
    fn steering_mode(&self) -> SteeringMode {
        SteeringMode::TurnBoundary
    }
    fn reasoning_levels(&self) -> &[ReasoningLevel] {
        &[ReasoningLevel::Medium]
    }
    async fn models(&self) -> Result<Vec<Model>, HarnessError> {
        Ok(vec![])
    }
    async fn run(
        &self,
        request: RunRequest,
        _controls: RunControls,
    ) -> Result<BoxStream<'static, Result<AgentEvent, HarnessError>>, HarnessError> {
        self.seen.lock().unwrap().push(Seen {
            cwd: request.cwd.clone(),
            prompt: request.prompt.clone(),
            resume: request.resume.clone(),
        });
        let n = self.seen.lock().unwrap().len();
        let session = format!("sess-{n}");
        let hold = request.prompt.contains("hold");
        let release = self.release.clone();
        let start = futures::stream::iter(vec![
            Ok(AgentEvent::SessionStarted {
                harness: HarnessId::Mock,
                model: "mock-1".into(),
                tools: vec![],
                cwd: request.cwd.clone(),
                session_id: session.clone(),
                assistant_message_id: format!("a-{n}"),
            }),
            Ok(AgentEvent::TextDelta { text: format!("ack: {}", request.prompt) }),
        ]);
        let end = futures::stream::once(async move {
            if hold {
                release.notified().await;
            }
            Ok(AgentEvent::Done {
                status: DoneStatus::Completed,
                result: None,
                error: None,
                session_id: Some(session),
            })
        });
        Ok(start.chain(end).boxed())
    }
}

fn git(cwd: &Path, args: &[&str]) {
    let out = Command::new("git").args(args).current_dir(cwd).output().expect("git runs");
    assert!(out.status.success(), "git {args:?} failed: {}", String::from_utf8_lossy(&out.stderr));
}

fn git_out(cwd: &Path, args: &[&str]) -> String {
    let out = Command::new("git").args(args).current_dir(cwd).output().expect("git runs");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

async fn wait_for<F: FnMut() -> bool>(mut predicate: F, what: &str) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    while !predicate() {
        assert!(tokio::time::Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(15)).await;
    }
}

struct Fixture {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    repo: PathBuf,
    worktrees: PathBuf,
    core: EngineCore,
    client: zend_rpc::RpcClient,
    seen: Arc<Mutex<Vec<Seen>>>,
    release: Arc<tokio::sync::Notify>,
}

const SPACE: &str = "space-naming";

async fn fixture() -> Fixture {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    // (A name of its own: this binary's engines share one worktrees folder,
    // `ZEND_WORKTREES_DIR`, set in main.rs, by repository name.)
    let repo = root.join(format!("repo-{}", tmp.path().file_name().unwrap().to_string_lossy()));
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-b", "main"]);
    git(&repo, &["config", "user.email", "t@example.com"]);
    git(&repo, &["config", "user.name", "Test"]);
    std::fs::write(repo.join("a.txt"), "one\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-m", "init"]);
    let seen = Arc::new(Mutex::new(Vec::new()));
    let release = Arc::new(tokio::sync::Notify::new());
    let registry = HarnessRegistry::new();
    registry.register(Arc::new(Recorder { seen: seen.clone(), release: release.clone() }));
    let core = EngineCore::assemble(&root.join("data"), Arc::new(registry), HarnessId::Mock, None)
        .expect("engine assembles");
    core.workspace
        .create_space(SPACE, &core.device_id, &repo.to_string_lossy(), Some("Repo".into()), true)
        .expect("project");
    let client = zend_rpc::memory_client(core.rpc_service());
    let worktrees = PathBuf::from(std::env::var_os("ZEND_WORKTREES_DIR").expect("set in main.rs"))
        .join(repo.file_name().unwrap());
    Fixture { _tmp: tmp, root, repo, worktrees, core, client, seen, release }
}

impl Fixture {
    /// A chat in the project (its folder, or `cwd`), titled so no titler runs.
    async fn chat(&self, id: &str, cwd: Option<&Path>) {
        let mut params = serde_json::json!({
            "op": "createChat", "chatId": id, "deviceId": self.core.device_id, "spaceId": SPACE,
        });
        if let Some(cwd) = cwd {
            params["cwd"] = serde_json::json!(cwd);
        }
        self.client.call(methods::MUTATE, params).await.expect("createChat");
        self.core.workspace.rename_chat(id, &format!("Chat {id}")).unwrap();
    }

    fn row(&self, id: &str) -> zend_proto::Chat {
        self.core.workspace.chat(id).unwrap().expect("chat row")
    }

    fn run(&self, chat: &str, message: &str, prompt: &str, cwd: &str, worktree: Option<WorktreeSpec>) {
        self.core
            .doc_host
            .queue_command(
                chat,
                SessionCommandPayload::Run {
                    request: RunRequest {
                        mcp: None,
                        prompt: prompt.into(),
                        harness: None,
                        model: None,
                        reasoning: None,
                        model_options: Default::default(),
                        cwd: cwd.into(),
                        sandbox: SandboxLevel::WorkspaceWrite,
                        auto_approve: true,
                        access: Default::default(),
                        plan: false,
                        attachments: Vec::new(),
                        resume: None,
                        worktree,
                        env: Default::default(),
                        extra_mcp: Vec::new(),
                        instructions: Vec::new(),
                    },
                    message_id: message.into(),
                },
            )
            .expect("queue run");
    }

    fn replies(&self, chat: &str) -> usize {
        let entries: Vec<SessionMessageEntry> = self
            .core
            .doc_host
            .open(chat)
            .ok()
            .and_then(|h| h.doc().read_entries().ok())
            .unwrap_or_default();
        entries
            .iter()
            .filter(|e| e.role == MessageRole::Assistant && e.status == Some(MessageStatus::Complete))
            .count()
    }

    fn spec(&self, name: Option<&str>, branch: Option<&str>) -> WorktreeSpec {
        WorktreeSpec {
            repo_path: self.repo.to_string_lossy().into(),
            base: "main".into(),
            space_id: Some(SPACE.into()),
            name: name.map(str::to_string),
            branch: branch.map(str::to_string),
        }
    }

    async fn create_worktree(&self, params: serde_json::Value) -> Result<serde_json::Value, String> {
        let mut p = serde_json::json!({ "repoPath": self.repo, "branch": "main" });
        for (k, v) in params.as_object().unwrap() {
            p[k] = v.clone();
        }
        self.client.call(methods::CREATE_WORKTREE, p).await.map_err(|e| e.to_string())
    }
}

/// K1: a worktree takes the name it is given, folder and branch, with no
/// `zend/` prefix; a name with slashes keeps them in its branch alone; the
/// branch can be named apart. Over CreateWorktree and over a run's spec.
#[tokio::test(flavor = "multi_thread")]
async fn a_worktree_takes_the_name_it_is_given() {
    let f = fixture().await;
    let made = f.create_worktree(serde_json::json!({ "name": "fix-login" })).await.expect("named");
    assert_eq!(PathBuf::from(made["path"].as_str().unwrap()), f.worktrees.join("fix-login"));
    assert_eq!(made["branch"], "fix-login");
    assert_eq!(git_out(Path::new(made["path"].as_str().unwrap()), &["branch", "--show-current"]), "fix-login");

    let made = f.create_worktree(serde_json::json!({ "name": "feat/login" })).await.expect("slashed");
    assert_eq!(PathBuf::from(made["path"].as_str().unwrap()), f.worktrees.join("feat-login"));
    assert_eq!(made["branch"], "feat/login");

    let made = f
        .create_worktree(serde_json::json!({ "name": "wt-x", "branchName": "topic/x" }))
        .await
        .expect("branch apart");
    assert_eq!(PathBuf::from(made["path"].as_str().unwrap()), f.worktrees.join("wt-x"));
    assert_eq!(made["branch"], "topic/x");

    // Unnamed: as before.
    let made = f.create_worktree(serde_json::json!({})).await.expect("unnamed");
    assert!(made["branch"].as_str().unwrap().starts_with("zend/"));

    // A run's spec: the chat works in the worktree it named.
    f.chat("chat-spec", None).await;
    f.run("chat-spec", "m-1", "first", &f.repo.to_string_lossy(), Some(f.spec(Some("from-run"), None)));
    wait_for(|| f.replies("chat-spec") == 1, "the run's turn").await;
    let row = f.row("chat-spec");
    assert_eq!(PathBuf::from(row.cwd.unwrap()), f.worktrees.join("from-run"));
    assert_eq!(row.branch.as_deref(), Some("from-run"));
}

/// K2: a name already taken (folder or branch) or not one git or a folder
/// takes is said, naming it, and nothing is made.
#[tokio::test(flavor = "multi_thread")]
async fn a_name_taken_or_not_gits_is_said_and_nothing_made() {
    let f = fixture().await;
    f.create_worktree(serde_json::json!({ "name": "fix-login" })).await.expect("first");
    let worktrees_before = git_out(&f.repo, &["worktree", "list"]);
    let branches_before = git_out(&f.repo, &["branch", "--list"]);

    let error = f.create_worktree(serde_json::json!({ "name": "fix-login", "branchName": "other" })).await.unwrap_err();
    assert!(error.contains("fix-login") && error.contains("already exists"), "{error}");
    let error = f.create_worktree(serde_json::json!({ "name": "main" })).await.unwrap_err();
    assert!(error.contains("Branch main already exists"), "{error}");
    for bad in ["a b", "..", "-x", ".hidden", "x\\y", "x:y"] {
        let error = f.create_worktree(serde_json::json!({ "name": bad })).await.unwrap_err();
        assert!(error.contains("can't name a worktree"), "{bad}: {error}");
    }
    let error = f.create_worktree(serde_json::json!({ "name": "ok-name", "branchName": "bad..name" })).await.unwrap_err();
    assert!(error.contains("isn't a name git takes"), "{error}");

    assert_eq!(git_out(&f.repo, &["worktree", "list"]), worktrees_before);
    assert_eq!(git_out(&f.repo, &["branch", "--list"]), branches_before);
    assert!(!f.worktrees.join("ok-name").exists());
}

/// K3: a new branch in a local checkout is made off its base and checked
/// out there; every chat in the folder says it; one already there is said.
#[tokio::test(flavor = "multi_thread")]
async fn a_new_branch_is_switched_to_in_the_local_checkout() {
    let f = fixture().await;
    f.chat("chat-a", None).await;
    f.chat("chat-b", None).await;
    let made = f
        .client
        .call(methods::CREATE_BRANCH, serde_json::json!({ "repoPath": f.repo, "name": "topic", "base": "main" }))
        .await
        .expect("CreateBranch");
    assert_eq!(made["branch"], "topic");
    assert_eq!(git_out(&f.repo, &["branch", "--show-current"]), "topic");
    assert_eq!(f.row("chat-a").branch.as_deref(), Some("topic"));
    assert_eq!(f.row("chat-b").branch.as_deref(), Some("topic"));

    let error = f
        .client
        .call(methods::CREATE_BRANCH, serde_json::json!({ "repoPath": f.repo, "name": "main", "base": "topic" }))
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("Branch main already exists"), "{error}");
    assert_eq!(git_out(&f.repo, &["branch", "--show-current"]), "topic");
}

/// K4: renaming a checkout's branch renames it for every chat working there
/// (not one in another worktree); while a turn runs in one, it is refused.
#[tokio::test(flavor = "multi_thread")]
async fn renaming_a_branch_moves_every_chat_in_the_checkout_never_mid_turn() {
    let f = fixture().await;
    f.chat("chat-a", None).await;
    f.chat("chat-b", Some(&f.repo)).await;
    let other = f.create_worktree(serde_json::json!({ "name": "elsewhere" })).await.unwrap();
    let other_path = PathBuf::from(other["path"].as_str().unwrap());
    f.chat("chat-wt", Some(&other_path)).await;
    f.core.workspace.set_chat_branch("chat-wt", "elsewhere").unwrap();

    // A turn running in chat-a: refused, the branch as it was.
    f.run("chat-a", "m-hold", "hold on", &f.repo.to_string_lossy(), None);
    wait_for(|| f.core.sessions.turn_in_flight("chat-a"), "a running turn").await;
    let error = f
        .client
        .call(methods::RENAME_BRANCH, serde_json::json!({ "cwd": f.repo, "name": "renamed" }))
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("A turn is running in Chat chat-a"), "{error}");
    assert_eq!(git_out(&f.repo, &["branch", "--show-current"]), "main");
    f.release.notify_one();
    wait_for(|| !f.core.sessions.turn_in_flight("chat-a") && !f.core.sessions.turn_starting("chat-a") && f.replies("chat-a") == 1, "the turn's end").await;

    let renamed = f
        .client
        .call(methods::RENAME_BRANCH, serde_json::json!({ "cwd": f.repo, "name": "renamed" }))
        .await
        .expect("RenameBranch");
    assert_eq!(renamed["branch"], "renamed");
    assert_eq!(git_out(&f.repo, &["branch", "--show-current"]), "renamed");
    assert_eq!(f.row("chat-a").branch.as_deref(), Some("renamed"));
    assert_eq!(f.row("chat-b").branch.as_deref(), Some("renamed"));
    assert_eq!(f.row("chat-wt").branch.as_deref(), Some("elsewhere"));

    // A name taken: said.
    let error = f
        .client
        .call(methods::RENAME_BRANCH, serde_json::json!({ "cwd": f.repo, "name": "elsewhere" }))
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("Branch elsewhere already exists"), "{error}");
}

/// K5: renaming a worktree moves its folder and renames its branch, every
/// chat in it following; refused while a turn runs there; the chat's next
/// turn runs in the new folder, fresh, with the chat's history.
#[tokio::test(flavor = "multi_thread")]
async fn renaming_a_worktree_moves_it_and_its_chats_and_keeps_the_history() {
    let f = fixture().await;
    f.chat("chat-wt", None).await;
    f.run("chat-wt", "m-1", "first words", &f.repo.to_string_lossy(), Some(f.spec(Some("wt-one"), None)));
    wait_for(|| f.replies("chat-wt") == 1, "the first turn").await;
    let old = f.worktrees.join("wt-one");
    assert_eq!(PathBuf::from(f.row("chat-wt").cwd.unwrap()), old);
    f.chat("chat-side", Some(&old)).await;

    // Mid-turn: refused, nothing moved.
    f.run("chat-side", "m-hold", "hold on", &old.to_string_lossy(), None);
    wait_for(|| f.core.sessions.turn_in_flight("chat-side"), "a running turn").await;
    let error = f
        .client
        .call(methods::RENAME_WORKTREE, serde_json::json!({ "worktreePath": old, "name": "wt-two" }))
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("A turn is running in Chat chat-side"), "{error}");
    assert!(old.is_dir());
    f.release.notify_one();
    wait_for(|| !f.core.sessions.turn_in_flight("chat-side") && !f.core.sessions.turn_starting("chat-side") && f.replies("chat-side") == 1, "the turn's end").await;

    let moved = f
        .client
        .call(methods::RENAME_WORKTREE, serde_json::json!({ "worktreePath": old, "name": "wt-two" }))
        .await
        .expect("RenameWorktree");
    let new = f.worktrees.join("wt-two");
    assert_eq!(PathBuf::from(moved["path"].as_str().unwrap()), new);
    assert_eq!(moved["branch"], "wt-two");
    assert!(!old.exists() && new.join(".git").is_file());
    assert_eq!(git_out(&new, &["branch", "--show-current"]), "wt-two");
    assert!(git_out(&f.repo, &["worktree", "list"]).contains(&*new.to_string_lossy()));
    for chat in ["chat-wt", "chat-side"] {
        let row = f.row(chat);
        assert_eq!(PathBuf::from(row.cwd.unwrap()), new, "{chat}");
        assert_eq!(row.branch.as_deref(), Some("wt-two"), "{chat}");
    }

    // The next turn: in the new folder, a fresh session, the history given.
    let before = f.seen.lock().unwrap().len();
    f.run("chat-wt", "m-2", "second words", &new.to_string_lossy(), None);
    wait_for(|| f.replies("chat-wt") == 2, "the second turn").await;
    let next = f.seen.lock().unwrap()[before].clone();
    assert_eq!(PathBuf::from(&next.cwd), new);
    assert_eq!(next.resume, None, "a session kept by the old folder isn't resumed");
    assert!(next.prompt.contains("first words"), "history replayed: {}", next.prompt);
    assert!(next.prompt.contains("second words"));

    // A name taken: said, nothing moved.
    f.create_worktree(serde_json::json!({ "name": "taken" })).await.unwrap();
    let error = f
        .client
        .call(methods::RENAME_WORKTREE, serde_json::json!({ "worktreePath": new, "name": "taken" }))
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("already exists"), "{error}");
    assert!(new.is_dir());
    // The project's own folder isn't a worktree to rename.
    let error = f
        .client
        .call(methods::RENAME_WORKTREE, serde_json::json!({ "worktreePath": f.repo, "name": "nope" }))
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("isn't a worktree"), "{error}");
    let _ = &f.root;
}

/// K6: switching a local checkout's branch: every chat there says the new
/// one; changes in the way stop it with git's own words, nothing forced.
#[tokio::test(flavor = "multi_thread")]
async fn switching_a_checkouts_branch_moves_its_chats_and_never_forces() {
    let f = fixture().await;
    git(&f.repo, &["branch", "other"]);
    f.chat("chat-a", None).await;
    f.chat("chat-b", None).await;
    let switched = f
        .client
        .call(methods::SWITCH_REF, serde_json::json!({ "repoPath": f.repo, "refName": "other" }))
        .await
        .expect("SwitchRef");
    assert_eq!(switched["branch"], "other");
    assert_eq!(f.row("chat-a").branch.as_deref(), Some("other"));
    assert_eq!(f.row("chat-b").branch.as_deref(), Some("other"));

    // A change on `other` that `main` doesn't have, then an uncommitted
    // edit of the same file: git won't switch.
    std::fs::write(f.repo.join("a.txt"), "changed on other\n").unwrap();
    git(&f.repo, &["-c", "user.email=t@example.com", "-c", "user.name=T", "commit", "-am", "other"]);
    std::fs::write(f.repo.join("a.txt"), "uncommitted\n").unwrap();
    let error = f
        .client
        .call(methods::SWITCH_REF, serde_json::json!({ "repoPath": f.repo, "refName": "main" }))
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("would be overwritten"), "git's words: {error}");
    assert_eq!(git_out(&f.repo, &["branch", "--show-current"]), "other");
    assert_eq!(std::fs::read_to_string(f.repo.join("a.txt")).unwrap(), "uncommitted\n");
    assert_eq!(f.row("chat-a").branch.as_deref(), Some("other"));
}
