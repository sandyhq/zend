//! A chat this host must sandbox (`SessionsEngine::sandbox_chat`): its runs
//! get the confinement, or don't start. Every other chat runs as before.
use async_trait::async_trait;
use futures::{StreamExt, stream::BoxStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use zend_engine::{EngineCore, HarnessRegistry};
use zend_harness::{Harness, HarnessError, RunControls};
use zend_proto::{
    AgentEvent, ChatSandbox, DoneStatus, HarnessId, Model, ReasoningLevel, RunRequest,
    SandboxLevel, SteeringMode,
};

/// Each run's sandbox root (None: unconfined).
type Runs = Arc<Mutex<Vec<Option<PathBuf>>>>;

struct Capture {
    runs: Runs,
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
    async fn models(&self) -> Result<Vec<Model>, HarnessError> {
        Ok(vec![])
    }
    async fn run(
        &self,
        _: RunRequest,
        controls: RunControls,
    ) -> Result<BoxStream<'static, Result<AgentEvent, HarnessError>>, HarnessError> {
        self.runs
            .lock()
            .unwrap()
            .push(controls.sandbox.map(|s| s.root().to_path_buf()));
        Ok(futures::stream::iter(vec![Ok(AgentEvent::Done {
            status: DoneStatus::Completed,
            result: None,
            error: None,
            session_id: None,
        })])
        .boxed())
    }
}

struct Rig {
    core: EngineCore,
    runs: Runs,
    project: PathBuf,
    _dir: tempfile::TempDir,
    _project: tempfile::TempDir,
}

fn rig(sandboxed: bool) -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let runs = Runs::default();
    let registry = HarnessRegistry::new();
    registry.register(Arc::new(Capture {
        runs: runs.clone(),
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
        project: path,
        _dir: dir,
        _project: project,
    }
}

fn request(cwd: &std::path::Path) -> RunRequest {
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
async fn a_sandboxed_chat_runs_confined_to_its_project() {
    if !std::path::Path::new("/usr/bin/bwrap").is_file() {
        eprintln!("bwrap not installed: skipped");
        return;
    }
    let rig = rig(true);
    rig.core.sessions.sandbox_chat("c", ChatSandbox::Project).await.unwrap();
    assert_eq!(rig.core.sessions.chat_sandbox("c"), Some(ChatSandbox::Project));
    rig.core
        .sessions
        .dispatch("c", HarnessId::Mock, request(&rig.project), None)
        .await
        .unwrap();
    wait_runs(&rig, 1).await;
    assert_eq!(*rig.runs.lock().unwrap(), vec![Some(rig.project.clone())]);
}

#[tokio::test]
async fn a_sandboxed_chat_on_an_agent_that_cannot_be_confined_is_refused() {
    let rig = rig(false);
    rig.core.sessions.sandbox_chat("c", ChatSandbox::Project).await.unwrap();
    let error = rig
        .core
        .sessions
        .dispatch("c", HarnessId::Mock, request(&rig.project), None)
        .await
        .unwrap_err()
        .to_string();
    println!("{error}");
    assert!(error.contains("must run sandboxed and was not started"), "{error}");
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert!(rig.runs.lock().unwrap().is_empty(), "it ran unconfined");
}

#[tokio::test]
async fn a_sandboxed_chat_in_the_home_folder_is_refused() {
    let rig = rig(true);
    rig.core.sessions.sandbox_chat("c", ChatSandbox::Project).await.unwrap();
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap();
    let error = rig
        .core
        .sessions
        .dispatch("c", HarnessId::Mock, request(&home), None)
        .await
        .unwrap_err()
        .to_string();
    println!("{error}");
    assert!(error.contains("must run sandboxed and was not started"), "{error}");
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert!(rig.runs.lock().unwrap().is_empty(), "it ran unconfined");
}
