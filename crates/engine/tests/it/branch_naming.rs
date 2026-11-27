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
    AgentEvent, DoneStatus, HarnessId, Model, ProjectActionDraft, ProjectActionIcon, ReasoningLevel,
    RunRequest, SandboxLevel, SteeringMode, WorktreeSpec,
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


/// A reference-transaction hook in `repo`: `script` (sh) runs as a ref
/// update is prepared (a branch renamed), before it is done; its exit
/// status is the hook's.
#[cfg(unix)]
fn ref_hook(repo: &Path, script: &str) {
    use std::os::unix::fs::PermissionsExt;
    let hooks = repo.join(".git").join("hooks");
    std::fs::create_dir_all(&hooks).unwrap();
    let file = hooks.join("reference-transaction");
    std::fs::write(&file, format!("#!/bin/sh\n[ \"$1\" = prepared ] || exit 0\n{script}\n")).unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// A hook that stops a branch rename half way until `dir`/go exists, saying
/// so in `dir`/reached.
#[cfg(unix)]
fn ref_hook_waiting(repo: &Path, dir: &Path) {
    let d = dir.display();
    ref_hook(
        repo,
        &format!("touch '{d}/reached'; i=0; while [ ! -e '{d}/go' ] && [ $i -lt 400 ]; do sleep 0.05; i=$((i+1)); done; exit 0"),
    );
}

/// Review round 1 (P1): a worktree rename that fails after its folder
/// moved isn't left half done. A branch in the way (`topic` for
/// `topic/sub`, `topic/sub` for `topic`) is refused before anything
/// moves; a branch rename git refuses after the move (a ref locked) moves
/// the folder back; and when it can't go back, its chats follow it where it
/// is and the error says so.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_worktree_rename_that_fails_after_the_move_is_not_left_half_done() {
    let f = fixture().await;
    let made = f.create_worktree(serde_json::json!({ "name": "before" })).await.unwrap();
    let old = PathBuf::from(made["path"].as_str().unwrap());
    f.chat("chat-wt", Some(&old)).await;
    git(&f.repo, &["branch", "topic"]);
    git(&f.repo, &["branch", "deep/inner"]);
    let rename = |name: &str| {
        f.client
            .call(methods::RENAME_WORKTREE, serde_json::json!({ "worktreePath": old, "name": name }))
    };

    // A branch it would sit under, and one under it: refused, nothing moved.
    for (name, other) in [("topic/sub", "topic"), ("deep", "deep/inner")] {
        let error = rename(name).await.unwrap_err().to_string();
        assert!(error.contains(&format!("Branch {other} is in the way of {name}")), "{error}");
        assert!(old.is_dir(), "{name}: the folder stays");
        assert_eq!(git_out(&old, &["branch", "--show-current"]), "before");
        assert_eq!(PathBuf::from(f.row("chat-wt").cwd.unwrap()), old);
    }

    // Git refuses the branch after the move (its ref locked): moved back.
    let lock = f.repo.join(".git/refs/heads/after.lock");
    std::fs::write(&lock, "").unwrap();
    let error = rename("after").await.unwrap_err().to_string();
    assert!(error.contains("after.lock"), "git's words: {error}");
    assert!(old.is_dir(), "moved back");
    assert!(!f.worktrees.join("after").exists());
    assert_eq!(git_out(&old, &["branch", "--show-current"]), "before");
    assert!(git_out(&f.repo, &["worktree", "list"]).contains(&*old.to_string_lossy()));
    assert_eq!(PathBuf::from(f.row("chat-wt").cwd.unwrap()), old);
    std::fs::remove_file(&lock).unwrap();

    // And it can't go back (its old place taken meanwhile): the chat works
    // where the folder is, and the error says so.
    ref_hook(&f.repo, &format!("mkdir -p '{0}' && touch '{0}/in-the-way'; exit 1", old.display()));
    let error = rename("after").await.unwrap_err().to_string();
    let new = f.worktrees.join("after");
    assert!(error.contains("it is at") && error.contains(&*new.to_string_lossy()), "{error}");
    assert!(error.contains("its chats work there now"), "{error}");
    assert!(new.join(".git").is_file());
    assert_eq!(PathBuf::from(f.row("chat-wt").cwd.unwrap()), new);
    assert_ne!(f.row("chat-wt").branch.as_deref(), Some("after"), "the branch wasn't renamed");
    assert_eq!(git_out(&new, &["branch", "--show-current"]), "before");
}

/// Review round 1 (P2): a branch rename holds the checkout: a turn sent to
/// a chat there while git renames it, and one in a chat made there
/// meanwhile, starts only once the rename and the chats' branch are done.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn no_turn_starts_in_a_checkout_while_its_branch_is_renamed() {
    let f = fixture().await;
    f.chat("chat-a", None).await;
    let signal = f.root.join("signal");
    std::fs::create_dir_all(&signal).unwrap();
    ref_hook_waiting(&f.repo, &signal);
    let cwd = f.repo.to_string_lossy().into_owned();
    let rename = f.client.call(methods::RENAME_BRANCH, serde_json::json!({ "cwd": f.repo, "name": "renamed" }));
    let probe = async {
        wait_for(|| signal.join("reached").exists(), "the rename half way").await;
        let before = f.seen.lock().unwrap().len();
        f.run("chat-a", "m-1", "words", &cwd, None);
        f.chat("chat-new", Some(&f.repo)).await;
        f.run("chat-new", "m-2", "more words", &cwd, None);
        tokio::time::sleep(Duration::from_millis(700)).await;
        let started = f.seen.lock().unwrap().len() - before;
        std::fs::write(signal.join("go"), "").unwrap();
        started
    };
    let (renamed, started) = tokio::join!(rename, probe);
    assert_eq!(renamed.expect("RenameBranch")["branch"], "renamed");
    assert_eq!(started, 0, "no turn started while the branch was renamed");
    wait_for(|| f.replies("chat-a") == 1 && f.replies("chat-new") == 1, "both turns, after").await;
    assert_eq!(f.row("chat-a").branch.as_deref(), Some("renamed"));
}

/// Review round 1 (P2): a chat made in a worktree while it moves, its turn
/// sent meanwhile: it waits, and runs in the new folder.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_turn_sent_while_its_worktree_moves_runs_in_the_new_folder() {
    let f = fixture().await;
    let made = f.create_worktree(serde_json::json!({ "name": "wt-old" })).await.unwrap();
    let old = PathBuf::from(made["path"].as_str().unwrap());
    let signal = f.root.join("signal");
    std::fs::create_dir_all(&signal).unwrap();
    ref_hook_waiting(&f.repo, &signal);
    let rename = f
        .client
        .call(methods::RENAME_WORKTREE, serde_json::json!({ "worktreePath": old, "name": "wt-new" }));
    let probe = async {
        wait_for(|| signal.join("reached").exists(), "the move half way").await;
        f.chat("chat-new", Some(&old)).await;
        f.run("chat-new", "m-1", "words", &old.to_string_lossy(), None);
        tokio::time::sleep(Duration::from_millis(700)).await;
        let started = f.seen.lock().unwrap().len();
        std::fs::write(signal.join("go"), "").unwrap();
        started
    };
    let (moved, started) = tokio::join!(rename, probe);
    let new = f.worktrees.join("wt-new");
    assert_eq!(PathBuf::from(moved.expect("RenameWorktree")["path"].as_str().unwrap()), new);
    assert_eq!(started, 0, "no turn started while the folder moved");
    wait_for(|| f.replies("chat-new") == 1, "its turn, after").await;
    assert_eq!(PathBuf::from(&f.seen.lock().unwrap()[0].cwd), new);
    assert_eq!(PathBuf::from(f.row("chat-new").cwd.unwrap()), new);
}


// ── Round 2 (plan "Runde 2"): an open session's chip menu ─────────────────

impl Fixture {
    /// A bare repository `name` beside the repo, added to it as remote
    /// `name`, with main pushed.
    fn remote(&self, name: &str) -> PathBuf {
        let bare = self.root.join(format!("{name}.git"));
        std::fs::create_dir_all(&bare).unwrap();
        git(&bare, &["init", "--bare", "-b", "main"]);
        git(&self.repo, &["remote", "add", name, &bare.to_string_lossy()]);
        git(&self.repo, &["push", "-q", name, "main"]);
        bare
    }

    /// A branch `branch` only on `remote` (pushed, its local one gone):
    /// with `edit`, a commit changing a.txt to it.
    fn remote_only(&self, remote: &str, branch: &str, edit: Option<&str>) {
        git(&self.repo, &["switch", "-q", "-c", branch]);
        if let Some(text) = edit {
            std::fs::write(self.repo.join("a.txt"), text).unwrap();
            git(&self.repo, &["commit", "-q", "-am", branch]);
        }
        git(&self.repo, &["push", "-q", remote, branch]);
        git(&self.repo, &["switch", "-q", "main"]);
        git(&self.repo, &["branch", "-q", "-D", branch]);
    }

    async fn refs(&self) -> Vec<serde_json::Value> {
        let refs = self
            .client
            .call(methods::LIST_REFS, serde_json::json!({ "repoPath": self.repo }))
            .await
            .expect("ListRefs");
        refs.as_array().unwrap().clone()
    }

    async fn call(&self, method: &str, params: serde_json::Value) -> Result<serde_json::Value, String> {
        self.client.call(method, params).await.map_err(|e| e.to_string())
    }
}

fn named<'a>(refs: &'a [serde_json::Value], name: &str) -> Option<&'a serde_json::Value> {
    refs.iter().find(|r| r["name"] == name)
}

/// R1-1, with R1-4's engine side: an open session's "New branch…" makes the
/// branch off its current one and checks it out in its folder, every chat
/// there following; never while a turn runs in one of them (nor a switch).
#[tokio::test(flavor = "multi_thread")]
async fn r1_a_new_branch_from_an_open_session_moves_its_folder_never_mid_turn() {
    let f = fixture().await;
    git(&f.repo, &["branch", "other"]);
    f.chat("chat-a", None).await;
    f.chat("chat-b", None).await;
    let cwd = f.repo.to_string_lossy().into_owned();
    f.run("chat-a", "m-1", "first words", &cwd, None);
    wait_for(|| f.replies("chat-a") == 1, "chat-a's turn").await;

    // A turn running in chat-b: refused, nothing made or switched.
    f.run("chat-b", "m-hold", "hold on", &cwd, None);
    wait_for(|| f.core.sessions.turn_in_flight("chat-b"), "a running turn").await;
    let error = f
        .call(methods::CREATE_BRANCH, serde_json::json!({ "repoPath": f.repo, "name": "topic", "base": "" }))
        .await
        .unwrap_err();
    assert!(error.contains("A turn is running in Chat chat-b"), "{error}");
    let error = f
        .call(methods::SWITCH_REF, serde_json::json!({ "repoPath": f.repo, "refName": "other" }))
        .await
        .unwrap_err();
    assert!(error.contains("A turn is running in Chat chat-b"), "{error}");
    assert_eq!(git_out(&f.repo, &["branch", "--show-current"]), "main");
    assert!(!git_out(&f.repo, &["branch", "--list"]).contains("topic"));
    f.release.notify_one();
    wait_for(
        || !f.core.sessions.turn_in_flight("chat-b") && !f.core.sessions.turn_starting("chat-b") && f.replies("chat-b") == 1,
        "the turn's end",
    )
    .await;

    let made = f
        .call(methods::CREATE_BRANCH, serde_json::json!({ "repoPath": f.repo, "name": "topic", "base": "" }))
        .await
        .expect("CreateBranch");
    assert_eq!(made["branch"], "topic");
    assert_eq!(git_out(&f.repo, &["branch", "--show-current"]), "topic");
    assert_eq!(git_out(&f.repo, &["rev-parse", "topic"]), git_out(&f.repo, &["rev-parse", "main"]));
    assert_eq!(f.row("chat-a").branch.as_deref(), Some("topic"));
    assert_eq!(f.row("chat-b").branch.as_deref(), Some("topic"));
}

/// R1-2: a new branch's name taken, not git's, or in the way of another
/// (topic vs topic/sub) is said, and nothing is made.
#[tokio::test(flavor = "multi_thread")]
async fn r1_a_new_branch_named_wrong_is_said_and_nothing_made() {
    let f = fixture().await;
    git(&f.repo, &["branch", "topic"]);
    git(&f.repo, &["branch", "deep/inner"]);
    f.chat("chat-a", None).await;
    let before = git_out(&f.repo, &["branch", "--list"]);
    for (name, said) in [
        ("main", "Branch main already exists".to_string()),
        ("topic/sub", "Branch topic is in the way of topic/sub".to_string()),
        ("deep", "Branch deep/inner is in the way of deep".to_string()),
        ("bad..name", "isn't a name git takes".to_string()),
        ("a b", "isn't a name git takes".to_string()),
        ("-x", "isn't a name git takes".to_string()),
    ] {
        let error = f
            .call(methods::CREATE_BRANCH, serde_json::json!({ "repoPath": f.repo, "name": name, "base": "" }))
            .await
            .unwrap_err();
        assert!(error.contains(&said), "{name}: {error}");
    }
    assert_eq!(git_out(&f.repo, &["branch", "--list"]), before);
    assert_eq!(git_out(&f.repo, &["branch", "--show-current"]), "main");
    assert_ne!(f.row("chat-a").branch.as_deref(), Some("topic/sub"));
}

/// R1-3: a worktree session's new branch is made in its folder alone; the
/// project's folder and its chats stay on their branch.
#[tokio::test(flavor = "multi_thread")]
async fn r1_a_new_branch_in_a_worktree_session_stays_in_its_folder() {
    let f = fixture().await;
    let made = f.create_worktree(serde_json::json!({ "name": "wt-a" })).await.unwrap();
    let wt = PathBuf::from(made["path"].as_str().unwrap());
    f.chat("chat-wt", Some(&wt)).await;
    f.core.workspace.set_chat_branch("chat-wt", "wt-a").unwrap();
    f.chat("chat-main", None).await;
    f.core.workspace.set_chat_branch("chat-main", "main").unwrap();
    let made = f
        .call(methods::CREATE_BRANCH, serde_json::json!({ "repoPath": wt, "name": "wt-topic", "base": "" }))
        .await
        .expect("CreateBranch");
    assert_eq!(made["branch"], "wt-topic");
    assert_eq!(git_out(&wt, &["branch", "--show-current"]), "wt-topic");
    assert_eq!(git_out(&f.repo, &["branch", "--show-current"]), "main");
    assert_eq!(f.row("chat-wt").branch.as_deref(), Some("wt-topic"));
    assert_eq!(f.row("chat-main").branch.as_deref(), Some("main"));
}

/// R2-1, R2-2: a branch only on a remote is listed, marked with its remote;
/// switched to, it becomes a local branch tracking it (origin's or another
/// remote's); changes in the way stop it in git's words, nothing made.
#[tokio::test(flavor = "multi_thread")]
async fn r2_a_remote_only_branch_is_listed_and_switched_to_as_a_tracking_branch() {
    let f = fixture().await;
    f.remote("origin");
    f.remote_only("origin", "feature", None);
    f.remote_only("origin", "clash", Some("changed on clash\n"));
    f.remote("upstream");
    f.remote_only("upstream", "theirs", None);
    f.chat("chat-a", None).await;

    let refs = f.refs().await;
    assert_eq!(named(&refs, "feature").expect("feature listed")["remote"], "origin", "{refs:?}");
    assert_eq!(named(&refs, "theirs").expect("theirs listed")["remote"], "upstream", "{refs:?}");
    assert!(named(&refs, "main").unwrap().get("remote").is_none(), "a local branch isn't remote: {refs:?}");

    // Changes in the way: git's words, no branch made, still on main.
    std::fs::write(f.repo.join("a.txt"), "uncommitted\n").unwrap();
    let error = f
        .call(methods::SWITCH_REF, serde_json::json!({ "repoPath": f.repo, "refName": "clash" }))
        .await
        .unwrap_err();
    assert!(error.contains("would be overwritten"), "git's words: {error}");
    assert_eq!(git_out(&f.repo, &["branch", "--show-current"]), "main");
    assert!(!git_out(&f.repo, &["branch", "--list"]).contains("clash"));
    assert_eq!(std::fs::read_to_string(f.repo.join("a.txt")).unwrap(), "uncommitted\n");
    git(&f.repo, &["checkout", "--", "a.txt"]);

    for (branch, remote) in [("feature", "origin"), ("theirs", "upstream")] {
        let switched = f
            .call(methods::SWITCH_REF, serde_json::json!({ "repoPath": f.repo, "refName": branch }))
            .await
            .expect("SwitchRef");
        assert_eq!(switched["branch"], branch);
        assert_eq!(git_out(&f.repo, &["branch", "--show-current"]), branch);
        assert_eq!(
            git_out(&f.repo, &["rev-parse", "--abbrev-ref", &format!("{branch}@{{upstream}}")]),
            format!("{remote}/{branch}")
        );
        assert_eq!(f.row("chat-a").branch.as_deref(), Some(branch));
    }
    let refs = f.refs().await;
    assert!(named(&refs, "feature").unwrap().get("remote").is_none(), "local now: {refs:?}");
}

/// R3-1: Fetch brings a branch pushed elsewhere into the list, and drops one
/// deleted on the remote (pruned), without a restart.
#[tokio::test(flavor = "multi_thread")]
async fn r3_fetch_brings_new_remote_branches_and_drops_deleted_ones() {
    let f = fixture().await;
    let bare = f.remote("origin");
    f.remote_only("origin", "gone", None);
    let other = f.root.join("other");
    let out = Command::new("git").args(["clone", "-q", &bare.to_string_lossy(), &other.to_string_lossy()]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    git(&other, &["push", "-q", "origin", "HEAD:refs/heads/fresh"]);
    git(&other, &["push", "-q", "origin", "--delete", "gone"]);

    let refs = f.refs().await;
    assert!(named(&refs, "fresh").is_none() && named(&refs, "gone").is_some(), "{refs:?}");
    f.call(methods::FETCH_ALL, serde_json::json!({ "repoPath": f.repo, "prune": true })).await.expect("FetchAll");
    let refs = f.refs().await;
    assert_eq!(named(&refs, "fresh").expect("fetched")["remote"], "origin", "{refs:?}");
    assert!(named(&refs, "gone").is_none(), "pruned: {refs:?}");
}

/// R4-1: "Move to a new worktree…": a local checkout's session moves alone
/// into a new worktree on a new branch off its current one; the others stay;
/// its next turn runs there, fresh, with its history. Never mid-turn, never
/// from a worktree, a name taken said.
#[tokio::test(flavor = "multi_thread")]
async fn r4_moving_a_session_to_a_new_worktree_takes_only_it_and_keeps_its_history() {
    let f = fixture().await;
    git(&f.repo, &["switch", "-q", "-c", "work"]);
    f.chat("chat-a", None).await;
    f.chat("chat-b", None).await;
    let cwd = f.repo.to_string_lossy().into_owned();
    f.run("chat-a", "m-1", "first words", &cwd, None);
    wait_for(|| f.replies("chat-a") == 1, "the first turn").await;

    let move_to = |chat: &str, name: &str, branch: Option<&str>| {
        f.call(methods::MOVE_CHAT_TO_WORKTREE, serde_json::json!({ "chatId": chat, "name": name, "branch": branch }))
    };
    let y = f.worktrees.join("y");

    // Mid-turn: refused, nothing made.
    f.run("chat-a", "m-hold", "hold on", &cwd, None);
    wait_for(|| f.core.sessions.turn_in_flight("chat-a"), "a running turn").await;
    let error = move_to("chat-a", "y", None).await.unwrap_err();
    assert!(error.contains("A turn is running"), "{error}");
    assert!(!y.exists());
    f.release.notify_one();
    wait_for(
        || !f.core.sessions.turn_in_flight("chat-a") && !f.core.sessions.turn_starting("chat-a") && f.replies("chat-a") == 2,
        "the turn's end",
    )
    .await;

    let moved = move_to("chat-a", "y", None).await.expect("MoveChatToWorktree");
    assert_eq!(PathBuf::from(moved["path"].as_str().unwrap()), y);
    assert_eq!(moved["branch"], "y");
    assert_eq!(moved["chats"], serde_json::json!(["chat-a"]));
    assert_eq!(git_out(&y, &["branch", "--show-current"]), "y");
    assert_eq!(git_out(&y, &["rev-parse", "HEAD"]), git_out(&f.repo, &["rev-parse", "work"]));
    assert_eq!(git_out(&f.repo, &["branch", "--show-current"]), "work", "the project's folder stays");
    let a = f.row("chat-a");
    assert_eq!(PathBuf::from(a.cwd.unwrap()), y);
    assert_eq!(a.branch.as_deref(), Some("y"));
    let b = f.row("chat-b");
    assert!(b.cwd.is_none_or(|c| Path::new(&c) == f.repo), "chat-b stays");
    assert_ne!(b.branch.as_deref(), Some("y"));

    // Its next turn: in the new folder, a fresh session, the history given.
    let before = f.seen.lock().unwrap().len();
    // (The history replayed says "hold on": the recorder waits on it, so
    // its release is given ahead.)
    f.release.notify_one();
    f.run("chat-a", "m-2", "second words", &y.to_string_lossy(), None);
    wait_for(|| f.replies("chat-a") == 3, "the next turn").await;
    let next = f.seen.lock().unwrap()[before].clone();
    assert_eq!(PathBuf::from(&next.cwd), y);
    assert_eq!(next.resume, None, "a session kept by the old folder isn't resumed");
    assert!(next.prompt.contains("first words"), "history replayed: {}", next.prompt);
    assert!(next.prompt.contains("second words"));

    // From a worktree: refused. A name taken: said, chat-b stays.
    let error = move_to("chat-a", "z", None).await.unwrap_err();
    assert!(error.contains("already works in a worktree"), "{error}");
    let error = move_to("chat-b", "y", None).await.unwrap_err();
    assert!(error.contains("already exists"), "{error}");
    assert!(f.row("chat-b").cwd.is_none_or(|c| Path::new(&c) == f.repo));
    // Its branch apart from its name.
    let moved = move_to("chat-b", "wt-b", Some("topic/b")).await.expect("named apart");
    assert_eq!(PathBuf::from(moved["path"].as_str().unwrap()), f.worktrees.join("wt-b"));
    assert_eq!(moved["branch"], "topic/b");
    assert_eq!(f.row("chat-b").branch.as_deref(), Some("topic/b"));
}

/// R4, a send racing the move (review round 1, P1): a message sent with the
/// old folder while the move is under way waits for it and runs in the new
/// worktree, with the history; another chat's send to the old folder stays
/// there (only the moved chat is retargeted).
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn r4_a_send_racing_the_move_runs_in_the_new_worktree() {
    let f = fixture().await;
    f.chat("chat-a", None).await;
    f.chat("chat-b", None).await;
    let cwd = f.repo.to_string_lossy().into_owned();
    f.run("chat-a", "m-1", "first words", &cwd, None);
    wait_for(|| f.replies("chat-a") == 1, "the first turn").await;

    // `git worktree add` waits in the checkout's post-checkout hook.
    let reached = f.root.join("hook-reached");
    let go = f.root.join("hook-go");
    let hook = f.repo.join(".git/hooks/post-checkout");
    script(
        &hook,
        &format!("touch '{}'\nwhile [ ! -e '{}' ]; do sleep 0.02; done", reached.display(), go.display()),
    );
    let moving = f.call(methods::MOVE_CHAT_TO_WORKTREE, serde_json::json!({ "chatId": "chat-a", "name": "race" }));
    let racing = async {
        wait_for(|| reached.exists(), "the move inside git worktree add").await;
        // Sent before the window heard of the move: the old folder.
        f.run("chat-a", "m-2", "second words", &cwd, None);
        tokio::time::sleep(Duration::from_millis(300)).await;
        std::fs::write(&go, "").unwrap();
    };
    let (moved, ()) = tokio::join!(moving, racing);
    moved.expect("MoveChatToWorktree");
    std::fs::remove_file(&hook).unwrap();
    wait_for(|| f.replies("chat-a") == 2, "the raced turn").await;
    let raced = f.seen.lock().unwrap().last().cloned().unwrap();
    assert_eq!(PathBuf::from(&raced.cwd), f.worktrees.join("race"), "the raced send runs in the new worktree");
    assert!(raced.prompt.contains("first words"), "history replayed: {}", raced.prompt);
    assert_eq!(PathBuf::from(f.row("chat-a").cwd.unwrap()), f.worktrees.join("race"));

    f.run("chat-b", "m-b", "b words", &cwd, None);
    wait_for(|| f.replies("chat-b") == 1, "chat-b's turn").await;
    let b = f.seen.lock().unwrap().last().cloned().unwrap();
    assert_eq!(PathBuf::from(&b.cwd), f.repo, "chat-b stays in the project folder");
}

/// R4, a move that fails after its worktree is made (review round 1, P2):
/// the worktree and branch it made are taken away, the chat stays where it
/// was, and the same name works when tried again.
#[tokio::test(flavor = "multi_thread")]
async fn r4_a_move_that_fails_after_its_worktree_is_made_leaves_nothing() {
    let f = fixture().await;
    f.chat("chat-a", None).await;
    let undone = f.worktrees.join("undone");
    let move_to = || f.call(methods::MOVE_CHAT_TO_WORKTREE, serde_json::json!({ "chatId": "chat-a", "name": "undone" }));
    f.core.sessions.fail_next_forget_folder("chat-a");
    let error = move_to().await.unwrap_err();
    assert!(error.contains("couldn't be let go"), "{error}");
    assert!(!undone.exists(), "the worktree made is taken away");
    assert_eq!(git_out(&f.repo, &["branch", "--list", "undone"]), "", "and its branch");
    assert!(!git_out(&f.repo, &["worktree", "list"]).contains("undone"));
    assert!(f.row("chat-a").cwd.is_none_or(|c| Path::new(&c) == f.repo), "the chat stays");
    move_to().await.expect("the same name, again");
    assert!(undone.is_dir());
    assert_eq!(PathBuf::from(f.row("chat-a").cwd.unwrap()), undone);
}

/// R4: a moved chat's new worktree runs the project's setup action, as a
/// new chat's worktree does (owner, review round 1).
#[tokio::test(flavor = "multi_thread")]
async fn r4_a_moved_session_s_worktree_runs_the_project_s_setup_action() {
    let f = fixture().await;
    f.chat("chat-a", None).await;
    f.call(
        methods::UPSERT_PROJECT_ACTION,
        serde_json::json!({
            "spaceId": SPACE,
            "action": ProjectActionDraft {
                name: "Setup".into(),
                command: "printf '%s' \"$ZEND_WORKTREE_PATH\" > setup-marker".into(),
                icon: ProjectActionIcon::Configure,
                run_on_worktree_create: true,
            }
        }),
    )
    .await
    .expect("setup action");
    let moved = f
        .call(methods::MOVE_CHAT_TO_WORKTREE, serde_json::json!({ "chatId": "chat-a", "name": "set-up" }))
        .await
        .expect("MoveChatToWorktree");
    let at = f.worktrees.join("set-up");
    wait_for(|| at.join("setup-marker").is_file(), "the setup action in the new worktree").await;
    assert_eq!(PathBuf::from(std::fs::read_to_string(at.join("setup-marker")).unwrap()), at);
    assert!(moved["setupAction"].is_object(), "{moved}");
    assert!(moved["setupError"].is_null(), "{moved}");
    // (Its terminal lives on with the engine: shut down, as worktree_on_run.)
    f.core.shutdown().await;
}

/// R3, Fetch never asks (review round 1, P2): no askpass program for a
/// remote that wants a password, ssh in batch mode (no passphrase or host
/// key question); the fetch fails and says so instead.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn r3_fetch_never_asks_for_a_password_nor_lets_ssh_ask() {
    let f = fixture().await;
    // An HTTP remote that wants a password: 401 to everything.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut buf = [0u8; 8192];
            let _ = std::io::Read::read(&mut stream, &mut buf);
            let _ = std::io::Write::write_all(
                &mut stream,
                b"HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Basic realm=\"zend\"\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            );
        }
    });
    let asked = f.root.join("askpass-asked");
    let askpass = f.root.join("askpass.sh");
    script(&askpass, &format!("echo asked >> '{}'\necho secret", asked.display()));
    git(&f.repo, &["config", "core.askPass", &askpass.to_string_lossy()]);
    git(&f.repo, &["remote", "add", "web", &format!("http://127.0.0.1:{port}/repo.git")]);
    let fetch = || f.call(methods::FETCH_ALL, serde_json::json!({ "repoPath": f.repo, "prune": true }));
    let error = fetch().await.unwrap_err();
    assert!(!asked.exists(), "askpass was asked; the fetch said: {error}");
    git(&f.repo, &["remote", "remove", "web"]);

    let ssh_args = f.root.join("ssh-args");
    let ssh = f.root.join("ssh.sh");
    script(&ssh, &format!("echo \"$@\" >> '{}'\nexit 255", ssh_args.display()));
    git(&f.repo, &["config", "core.sshCommand", &ssh.to_string_lossy()]);
    git(&f.repo, &["remote", "add", "far", "ssh://git@example.invalid/repo.git"]);
    fetch().await.unwrap_err();
    let args = std::fs::read_to_string(&ssh_args).expect("the remote's ssh command ran");
    assert!(args.contains("BatchMode=yes"), "ssh may ask: {args}");
}

/// An executable shell script at `path`.
#[cfg(unix)]
fn script(path: &Path, body: &str) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}
