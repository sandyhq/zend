//! Ideas & Intents (src/ideas.rs) with a fake microphone, transcriber and
//! speaker: a memo's audio is on disk before it is transcribed, a failed or
//! impossible transcription keeps it, it plays after a restart, and the
//! page (ui/screens/ideas) is reached from the sidebar of the real shell.
use super::*;
use std::{
    cell::RefCell,
    rc::Rc,
    sync::{Mutex, mpsc},
    time::{Duration, Instant},
};
use zgx_script::testing::TestApp;

/// A microphone that hears `samples` (or fails with `error`).
fn mic(heard: Result<Samples, String>) -> Source {
    let heard = Mutex::new(Some(heard));
    Arc::new(move |_device, stop: &AtomicBool, opened: &AtomicBool| {
        let heard = heard.lock().unwrap().take().unwrap_or_else(|| Ok((vec![0.2; 16_000], 16_000)));
        let samples = heard?;
        opened.store(true, Ordering::Release);
        while !stop.load(Ordering::Acquire) {
            std::thread::sleep(Duration::from_millis(2));
        }
        Ok(samples)
    })
}

/// A transcriber that answers each memo only when the test says what.
struct Gate(mpsc::Receiver<Result<String, Failure>>);
impl Transcribe for Gate {
    fn transcribe(&mut self, _samples: Vec<f32>, _rate: u32) -> Result<String, Failure> {
        self.0.recv().unwrap_or_else(|_| Err(Failure::Failed("test over".into())))
    }
}

struct Never;
impl Playing for Never {
    fn ended(&self) -> bool {
        false
    }
}

struct Fake {
    answer: mpsc::Sender<Result<String, Failure>>,
    played: Rc<RefCell<Vec<(usize, u32)>>>,
}

fn parts(dir: &std::path::Path, heard: Result<Samples, String>, model: bool) -> (Parts, Fake) {
    let (answer, inbox) = mpsc::channel();
    let played = Rc::new(RefCell::new(Vec::new()));
    let p = played.clone();
    let parts = Parts {
        dir: Some(dir.to_path_buf()),
        source: mic(heard),
        transcriber: Box::new(move || Box::new(Gate(inbox)) as Box<dyn Transcribe>),
        speaker: Box::new(move |samples, rate| {
            p.borrow_mut().push((samples.len(), rate));
            Ok(Box::new(Never) as Box<dyn Playing>)
        }),
        model: Box::new(move || model),
        authorize: Box::new(|| Ok(())),
    };
    (parts, Fake { answer, played })
}

fn until(service: &mut Service, what: &str, mut done: impl FnMut(&Service) -> bool) {
    let start = Instant::now();
    while !done(service) {
        assert!(start.elapsed() < Duration::from_secs(5), "timed out waiting for {what}: {}", service.state());
        service.poll();
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn phase(service: &Service) -> String {
    service.state()["recording"]["phase"].as_str().unwrap_or_default().to_owned()
}

fn record(service: &mut Service) -> String {
    service.record_start(None).unwrap();
    until(service, "recording", |s| phase(s) == "recording");
    service.record_stop();
    until(service, "saved", |s| phase(s) == "saved");
    service.state()["recording"]["id"].as_str().unwrap().to_owned()
}

#[test]
fn a_memo_is_saved_before_it_is_transcribed_and_a_failure_keeps_it() {
    let tmp = tempfile::tempdir().unwrap();
    let (p, fake) = parts(tmp.path(), Ok((vec![0.3; 24_000], 16_000)), true);
    let mut service = Service::new(p);
    let id = record(&mut service);
    // Transcribing (the fake holds it there): already on disk, whole.
    let state = service.state();
    assert_eq!(state["items"][0]["transcript"], "transcribing", "{state}");
    assert_eq!(state["items"][0]["seconds"], 1.5);
    assert!(tmp.path().join(format!("{id}.wav")).is_file());
    let record: Idea = serde_json::from_slice(&std::fs::read(tmp.path().join(format!("{id}.json"))).unwrap()).unwrap();
    assert_eq!(record.transcript, Some(Transcript::Pending));
    let (samples, rate) = Store::open(tmp.path().to_path_buf()).unwrap().audio(&id).unwrap();
    assert_eq!((samples.len(), rate), (24_000, 16_000));

    fake.answer.send(Err(Failure::Failed("the model crashed".into()))).unwrap();
    until(&mut service, "the failure", |s| !s.needs_poll());
    let state = service.state();
    assert_eq!(state["items"][0]["transcript"], "failed", "{state}");
    assert_eq!(state["items"][0]["error"], "the model crashed");
    assert!(tmp.path().join(format!("{id}.wav")).is_file(), "a failed transcription keeps the audio");

    // Retried, it gets its words.
    service.transcribe(&id).unwrap();
    fake.answer.send(Ok("  call the plumber ".into())).unwrap();
    until(&mut service, "the transcript", |s| !s.needs_poll());
    let state = service.state();
    assert_eq!(state["items"][0]["transcript"], "done");
    assert_eq!(state["items"][0]["text"], "call the plumber");
}

#[test]
fn after_a_restart_the_memo_is_listed_and_plays_as_recorded() {
    let tmp = tempfile::tempdir().unwrap();
    let (p, fake) = parts(tmp.path(), Ok((vec![0.1; 44_100], 44_100)), true);
    let mut service = Service::new(p);
    let id = record(&mut service);
    service.add("typed after").unwrap();
    // The app quits while it transcribes.
    drop(service);
    drop(fake);

    let (p, fake) = parts(tmp.path(), Ok((Vec::new(), 16_000)), true);
    let mut again = Service::new(p);
    let state = again.state();
    assert_eq!(state["items"].as_array().unwrap().len(), 2);
    assert_eq!(state["items"][0]["text"], "typed after", "newest first");
    assert_eq!(state["items"][1]["id"], id.as_str());
    assert_eq!(state["items"][1]["transcript"], "interrupted");
    again.play(&id).unwrap();
    assert_eq!(*fake.played.borrow(), [(44_100, 44_100)]);
    assert_eq!(again.state()["playing"], id.as_str());
    again.play_stop();
    assert!(again.state()["playing"].is_null());
}

#[test]
fn no_model_keeps_the_audio_and_says_so() {
    let tmp = tempfile::tempdir().unwrap();
    let (p, fake) = parts(tmp.path(), Ok((vec![0.1; 16_000], 16_000)), false);
    let mut service = Service::new(p);
    let id = record(&mut service);
    fake.answer.send(Err(Failure::NoModel)).unwrap();
    until(&mut service, "no model", |s| !s.needs_poll());
    let state = service.state();
    assert_eq!(state["model"], false);
    assert_eq!(state["items"][0]["transcript"], "no_model");
    assert!(tmp.path().join(format!("{id}.wav")).is_file());
}

#[test]
fn a_microphone_that_cant_be_used_saves_nothing_and_says_why() {
    let tmp = tempfile::tempdir().unwrap();
    let (p, _fake) = parts(tmp.path(), Err("No microphone available".into()), true);
    let mut service = Service::new(p);
    service.record_start(None).unwrap();
    until(&mut service, "the failure", |s| phase(s) == "failed");
    let state = service.state();
    assert_eq!(state["recording"]["kind"], "microphone");
    assert_eq!(state["recording"]["error"], "No microphone available");
    assert!(state["items"].as_array().unwrap().is_empty());

    // Refused by the system (macOS's permission): said before anything opens.
    let (mut p, _fake) = parts(tmp.path(), Ok((vec![0.1; 16_000], 16_000)), true);
    p.authorize = Box::new(|| Err("Zend may not use the microphone".into()));
    let mut service = Service::new(p);
    assert!(service.record_start(None).is_err());
    assert_eq!(service.state()["recording"]["kind"], "microphone");
    assert!(!service.needs_poll());
}

#[test]
fn discarding_a_recording_keeps_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let (p, _fake) = parts(tmp.path(), Ok((vec![0.1; 16_000], 16_000)), true);
    let mut service = Service::new(p);
    service.record_start(None).unwrap();
    until(&mut service, "recording", |s| phase(s) == "recording");
    service.record_cancel();
    std::thread::sleep(Duration::from_millis(30));
    service.poll();
    assert_eq!(phase(&service), "idle");
    assert_eq!(std::fs::read_dir(tmp.path()).unwrap().count(), 0);
}

/// The shell's fixture with Ideas & Intents answered by `service`.
fn mount(service: Service) -> (TestApp, Rc<Ideas>) {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("ui/shell_fixture.luau");
    let slot = Rc::new(RefCell::new(None));
    let s = slot.clone();
    let app = TestApp::new(move |runtime| {
        let host = ScriptHost::new(runtime, path);
        *s.borrow_mut() = Some(attach(&host, service, || {}));
        host
    });
    app.ui.scene.borrow_mut().resize(1200., 800.);
    app.frames(30);
    let ideas = slot.borrow_mut().take().unwrap();
    (app, ideas)
}

fn wait(app: &mut TestApp, ideas: &Ideas, what: &str, mut done: impl FnMut(&mut TestApp) -> bool) {
    let start = Instant::now();
    loop {
        ideas.tick();
        app.frames(2);
        if done(app) {
            return;
        }
        assert!(start.elapsed() < Duration::from_secs(10), "timed out waiting for {what}; error {:?}; texts {:?}", app.error(), app.texts());
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn the_page_from_the_sidebar_keeps_typed_ideas_and_memos() {
    let tmp = tempfile::tempdir().unwrap();
    let (p, fake) = parts(tmp.path(), Ok((vec![0.2; 32_000], 16_000)), true);
    let (mut app, ideas) = mount(Service::new(p));
    assert_eq!(app.error(), None);
    assert!(app.find("#sidebar-ideas").is_some(), "the sidebar has its row");
    app.click("#sidebar-ideas");
    wait(&mut app, &ideas, "the page", |a| a.find("#ideas-page").is_some());
    assert!(app.texts().iter().any(|t| t.contains("Ideas & Intents")));
    assert!(app.find("#ideas-empty").is_some());

    app.fill("#ideas-page >> role:textbox", "a sidebar for loose thoughts");
    app.click("#ideas-save");
    wait(&mut app, &ideas, "the typed idea", |a| a.texts().iter().any(|t| t == "a sidebar for loose thoughts"));
    assert_eq!(app.find(".idea-typed").is_some(), true);

    app.click("#ideas-record");
    wait(&mut app, &ideas, "recording", |a| a.find("#ideas-stop").is_some() && a.text("#ideas-recording-status").starts_with("Recording"));
    app.click("#ideas-stop");
    wait(&mut app, &ideas, "the memo", |a| a.find(".idea-memo").is_some());
    wait(&mut app, &ideas, "transcribing", |a| a.texts().iter().any(|t| t.starts_with("Transcribing")));
    fake.answer.send(Err(Failure::Failed("decoder failed".into()))).unwrap();
    wait(&mut app, &ideas, "the failure", |a| a.texts().iter().any(|t| t.contains("decoder failed") && t.contains("recording is kept")));
    assert!(app.find(".idea-transcribe").is_some(), "it offers to try again");
    app.click(".idea-play");
    wait(&mut app, &ideas, "playing", |_| !fake.played.borrow().is_empty());
    assert_eq!(*fake.played.borrow(), [(32_000, 16_000)]);
    // Newest first: the memo above the typed idea.
    let memo = app.bounds(".idea-memo");
    let typed = app.bounds(".idea-typed");
    assert!(memo.y < typed.y, "{memo:?} {typed:?}");
    assert_eq!(app.error(), None);
}
