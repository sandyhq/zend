//! A chat this host must sandbox (`SessionsEngine::sandbox_chat`): its runs
//! get the confinement in its marked root, or don't start; nothing of the
//! host's (git, discovery, warm starts) runs in its project. Every other
//! chat runs as before.
use async_trait::async_trait;
use futures::{StreamExt, stream::BoxStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use zend_engine::{EngineCore, HarnessRegistry};
use zend_harness::{Harness, HarnessError, RunControls};
use zend_proto::{
    AgentEvent, DoneStatus, HarnessId, Model, ReasoningLevel, RunRequest, SandboxLevel,
    SteeringMode,
};

/// Each run's sandbox (None: unconfined): its root and its state folder.
type Runs = Arc<Mutex<Vec<Option<(PathBuf, Option<PathBuf>)>>>>;

#[derive(Default)]
struct Calls {
    commands: usize,
    prewarms: usize,
}

struct Capture {
    runs: Runs,
    calls: Arc<Mutex<Calls>>,
    sandboxed: bool,
}

#[async_trait]
impl Harness for Capture {
    fn id(&self) -> HarnessId {
        HarnessId::Mock
    }
    fn display_name(&self) -> &str {
        "Capture"
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
    fn runs_sandboxed(&self) -> bool {
        self.sandboxed
    }
    fn prewarms(&self) -> bool {
        true
    }
    async fn prewarm(&self, _key: &str, _request: RunRequest) -> Result<(), HarnessError> {
        self.calls.lock().unwrap().prewarms += 1;
        Ok(())
    }
    async fn commands_for(&self, _cwd: &Path) -> Result<Vec<zend_proto::SlashCommand>, HarnessError> {
        self.calls.lock().unwrap().commands += 1;
        Ok(Vec::new())
    }
    async fn models(&self) -> Result<Vec<Model>, HarnessError> {
        Ok(vec![])
    }
    async fn run(
        &self,
        _: RunRequest,
        controls: RunControls,
    ) -> Result<BoxStream<'static, Result<AgentEvent, HarnessError>>, HarnessError> {
        self.runs.lock().unwrap().push(
            controls
                .sandbox
                .map(|s| (s.root().to_path_buf(), s.state().map(Path::to_path_buf))),
        );
        Ok(futures::stream::iter(vec![Ok(AgentEvent::Done {
            status: DoneStatus::Completed,
            result: None,
            error: None,
            session_id: None,
        })])
        .boxed())
    }
}

/// The real home: a sandboxed root must be inside it.
fn home() -> PathBuf {
    std::fs::canonicalize(std::env::var_os("HOME").expect("HOME")).unwrap()
}

/// A project folder inside the home, removed after the test.
fn project_in_home() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("zend-sandbox-test-")
        .tempdir_in(home())
        .unwrap()
}

struct Rig {
    core: EngineCore,
    runs: Runs,
    calls: Arc<Mutex<Calls>>,
    project: PathBuf,
    _dir: tempfile::TempDir,
    _project: tempfile::TempDir,
}

fn rig(sandboxed: bool) -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let project = project_in_home();
    let runs = Runs::default();
    let calls = Arc::new(Mutex::new(Calls::default()));
    let registry = HarnessRegistry::new();
    registry.register(Arc::new(Capture {
        runs: runs.clone(),
        calls: calls.clone(),
        sandboxed,
    }));
    let core = EngineCore::assemble(dir.path(), Arc::new(registry), HarnessId::Mock, None).unwrap();
    let path = std::fs::canonicalize(project.path()).unwrap();
    core.workspace
        .create_chat(
            "c",
            None,
            Some(&core.device_id),
            None,
            Some(path.display().to_string()),
        )
        .unwrap();
    Rig {
        core,
        runs,
        calls,
        project: path,
        _dir: dir,
        _project: project,
    }
}

fn tools_present() -> bool {
    let present = Path::new("/usr/bin/bwrap").is_file() && Path::new("/usr/bin/pasta").is_file();
    if !present {
        eprintln!("bwrap or pasta not installed: skipped");
    }
    present
}

fn request(cwd: &Path) -> RunRequest {
    RunRequest {
        prompt: "hello".into(),
        harness: Some(HarnessId::Mock),
        model: None,
        reasoning: None,
        model_options: Default::default(),
        cwd: cwd.display().to_string(),
        sandbox: SandboxLevel::WorkspaceWrite,
        auto_approve: false,
        access: Default::default(),
        plan: false,
        resume: None,
        attachments: vec![],
        worktree: None,
        mcp: None,
        env: Default::default(),
        extra_mcp: Vec::new(),
        instructions: Vec::new(),
    }
}

async fn wait_runs(rig: &Rig, n: usize) {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    while rig.runs.lock().unwrap().len() < n {
        assert!(tokio::time::Instant::now() < deadline, "no run started");
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

/// Dispatch in `cwd` and expect a refusal; nothing may have run.
async fn refused(rig: &Rig, cwd: &Path) -> String {
    let error = rig
        .core
        .sessions
        .dispatch("c", HarnessId::Mock, request(cwd), None)
        .await
        .unwrap_err()
        .to_string();
    println!("{error}");
    assert!(error.contains("must run sandboxed and was not started"), "{error}");
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert!(rig.runs.lock().unwrap().is_empty(), "it ran unconfined");
    error
}

#[tokio::test]
async fn an_unmarked_chat_runs_unconfined_as_before() {
    let rig = rig(true);
    assert_eq!(rig.core.sessions.chat_sandbox("c"), None);
    rig.core
        .sessions
        .dispatch("c", HarnessId::Mock, request(&rig.project), None)
        .await
        .unwrap();
    wait_runs(&rig, 1).await;
    assert_eq!(*rig.runs.lock().unwrap(), vec![None]);
}

#[tokio::test]
async fn a_sandboxed_chat_runs_confined_to_its_root_with_a_state_folder_of_its_own() {
    if !tools_present() {
        return;
    }
    let rig = rig(true);
    let sub = rig.project.join("sub");
    std::fs::create_dir_all(&sub).unwrap();
    let root = rig.core.sessions.sandbox_chat("c", &rig.project).await.unwrap();
    assert_eq!(root, rig.project);
    let mark = rig.core.sessions.chat_sandbox("c").unwrap();
    assert_eq!(mark.root.as_deref(), Some(rig.project.as_path()));
    // A turn in a folder inside the root: confined to the root.
    rig.core
        .sessions
        .dispatch("c", HarnessId::Mock, request(&sub), None)
        .await
        .unwrap();
    wait_runs(&rig, 1).await;
    let runs = rig.runs.lock().unwrap().clone();
    let (run_root, state) = runs[0].clone().expect("confined");
    assert_eq!(run_root, rig.project);
    // Its state folder: in the engine's data, made, outside the project.
    let state = state.expect("a state folder");
    println!("state {}", state.display());
    assert!(state.is_dir() && !state.starts_with(&rig.project), "{}", state.display());
    assert!(state.to_string_lossy().ends_with("c.sandbox-state"), "{}", state.display());
}

#[tokio::test]
async fn a_turn_outside_the_sandboxed_root_is_refused() {
    if !tools_present() {
        return;
    }
    let rig = rig(true);
    rig.core.sessions.sandbox_chat("c", &rig.project).await.unwrap();
    let sibling = project_in_home();
    let error = refused(&rig, sibling.path()).await;
    assert!(error.contains("outside its project"), "{error}");
    // Nor the home that holds both.
    let error = refused(&rig, &home()).await;
    assert!(error.contains("outside its project"), "{error}");
}

#[tokio::test]
async fn a_sandboxed_chat_on_an_agent_that_cannot_be_confined_is_refused() {
    let rig = rig(false);
    rig.core.sessions.sandbox_chat("c", &rig.project).await.unwrap();
    refused(&rig, &rig.project).await;
}

#[tokio::test]
async fn a_root_that_is_the_home_or_outside_it_is_refused_and_the_chat_never_runs() {
    let outside = tempfile::tempdir().unwrap();
    let dot = home().join(".zend-sandbox-test-dot");
    std::fs::create_dir_all(&dot).unwrap();
    for (root, why) in [
        (home(), "is the home folder"),
        (outside.path().to_path_buf(), "not inside the home folder"),
        (Path::new("/").to_path_buf(), "not inside the home folder"),
        (dot.clone(), "hidden folder"),
    ] {
        let rig = rig(true);
        let error = rig
            .core
            .sessions
            .sandbox_chat("c", &root)
            .await
            .unwrap_err()
            .to_string();
        println!("{}: {error}", root.display());
        assert!(error.contains(why), "{}: {error}", root.display());
        // Marked all the same, without a root: no run of the chat starts,
        // not even in its own project.
        assert_eq!(rig.core.sessions.chat_sandbox("c").unwrap().root, None);
        let error = refused(&rig, &rig.project).await;
        assert!(error.contains("no valid project folder"), "{error}");
    }
    std::fs::remove_dir_all(&dot).unwrap();
}

fn git(dir: &Path, args: &[&str]) {
    let status = std::process::Command::new("git")
        .args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false"])
        .args(args)
        .current_dir(dir)
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}");
}

/// A repository in `dir` with what a confined agent can write in it: its
/// config (a filter driver, a textconv, a filesystem monitor) and its
/// attributes, each leaving a file in `marks` when it runs; and a change.
fn planted_repo(dir: &Path, marks: &Path) {
    git(dir, &["init", "-q", "-b", "main"]);
    std::fs::write(dir.join("a.txt"), "one\n").unwrap();
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", "first"]);
    let config = dir.join(".git/config");
    let mut text = std::fs::read_to_string(&config).unwrap();
    let m = marks.display();
    text.push_str(&format!(
        "[core]\n\tfsmonitor = \"touch {m}/fsmonitor; false\"\n\
         [filter \"evil\"]\n\tclean = \"touch {m}/clean; cat\"\n\
         [diff \"evil\"]\n\ttextconv = \"touch {m}/textconv; cat\"\n"
    ));
    std::fs::write(&config, text).unwrap();
    std::fs::write(dir.join(".gitattributes"), "* filter=evil diff=evil\n").unwrap();
    std::fs::write(dir.join("a.txt"), "two\n").unwrap();
    std::fs::write(dir.join("new.txt"), "new\n").unwrap();
}

fn ran(marks: &Path) -> Vec<String> {
    let mut ran: Vec<String> = std::fs::read_dir(marks)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    ran.sort();
    ran
}

#[tokio::test]
async fn host_git_never_runs_in_a_sandboxed_chats_project() {
    let rig = rig(true);
    let repos = &rig.core.repos;
    // The same plant in a folder no chat is sandboxed in: the engine's git
    // runs it (what the review reproduced), so the probe is real.
    let control = project_in_home();
    let control_marks = tempfile::tempdir().unwrap();
    planted_repo(control.path(), control_marks.path());
    let _ = zend_engine::snapshot_tree(repos, control.path()).await;
    let _ = zend_engine::capture_diff(repos, control.path()).await;
    println!("unmarked folder ran {:?}", ran(control_marks.path()));
    assert!(!ran(control_marks.path()).is_empty(), "the plant never ran: no proof");

    let marks = tempfile::tempdir().unwrap();
    planted_repo(&rig.project, marks.path());
    rig.core.sessions.sandbox_chat("c", &rig.project).await.unwrap();
    let sub = rig.project.join("sub");
    std::fs::create_dir_all(&sub).unwrap();
    for folder in [&rig.project, &sub] {
        let snapshot = zend_engine::snapshot_tree(repos, folder).await;
        let diff = zend_engine::capture_diff(repos, folder).await;
        let branch = repos.current_branch(folder).await;
        let identity = repos.checkout_identity(folder).await;
        let head = repos.head_sha(folder).await;
        println!(
            "{}: snapshot {:?} diff {:?} branch {:?} identity {:?} head {:?}",
            folder.display(),
            snapshot.as_ref().err().map(ToString::to_string),
            diff.as_ref().err().map(ToString::to_string),
            branch.as_ref().err().map(ToString::to_string),
            identity.as_ref().err().map(ToString::to_string),
            head.as_ref().err().map(ToString::to_string),
        );
        assert!(snapshot.is_err() && diff.is_err() && branch.is_err() && identity.is_err() && head.is_err());
    }
    assert_eq!(ran(marks.path()), Vec::<String>::new(), "planted git config ran");
}

#[tokio::test]
async fn no_discovery_or_warm_start_runs_an_agent_in_a_sandboxed_chats_project() {
    let rig = rig(true);
    // Elsewhere both start the agent (the probe is real).
    let other = project_in_home();
    rig.core.registry.discover_commands(HarnessId::Mock, other.path()).await.unwrap();
    let outcome = rig
        .core
        .sessions
        .warm_start("new-elsewhere", HarnessId::Mock, request(other.path()), false)
        .await
        .unwrap();
    println!("elsewhere: warm start {outcome:?}");
    let (commands, prewarms) = {
        let calls = rig.calls.lock().unwrap();
        (calls.commands, calls.prewarms)
    };
    assert_eq!(commands, 1);
    assert_eq!(prewarms, 1, "{outcome:?}");

    rig.core.sessions.sandbox_chat("c", &rig.project).await.unwrap();
    let sub = rig.project.join("sub");
    std::fs::create_dir_all(&sub).unwrap();
    for folder in [&rig.project, &sub] {
        let commands = rig.core.registry.discover_commands(HarnessId::Mock, folder).await.unwrap();
        let skills = rig.core.registry.discover_skills(HarnessId::Mock, folder).await.unwrap();
        assert!(commands.is_empty() && skills.is_none_or(|s| s.is_empty()));
        // A new chat the window would create in that project.
        let outcome = rig
            .core
            .sessions
            .warm_start("new-in-project", HarnessId::Mock, request(folder), false)
            .await
            .unwrap();
        assert_eq!(outcome, zend_engine::sessions::WarmOutcome::Unsupported);
    }
    let calls = rig.calls.lock().unwrap();
    assert_eq!((calls.commands, calls.prewarms), (1, 1), "an agent started in the project");
}
