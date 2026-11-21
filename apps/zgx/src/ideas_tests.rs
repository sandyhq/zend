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
    Arc::new(move |_device, stop: &AtomicBool, opened: &AtomicBool, _level: &Arc<AtomicU32>| {
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

/// A microphone that hears 160 samples (10 ms at 16 kHz) a beat for as long
/// as it's on, counting them in `heard`; it opens only once `open` lets
/// it, and not at all when it was stopped by then, as the real one does.
fn streaming(heard: Arc<std::sync::atomic::AtomicUsize>, open: Option<mpsc::Receiver<()>>) -> Source {
    let open = Mutex::new(open);
    Arc::new(move |_device, stop: &AtomicBool, opened: &AtomicBool, _level: &Arc<AtomicU32>| {
        if let Some(gate) = open.lock().unwrap().take() {
            let _ = gate.recv();
        }
        if stop.load(Ordering::Acquire) {
            return Ok((Vec::new(), 16_000));
        }
        opened.store(true, Ordering::Release);
        let mut n = 0;
        while !stop.load(Ordering::Acquire) {
            n += 160;
            heard.store(n, Ordering::Release);
            std::thread::sleep(Duration::from_millis(10));
        }
        Ok((vec![0.2; n], 16_000))
    })
}

fn memo_len(service: &Service, id: &str) -> usize {
    service.store().unwrap().audio(id).unwrap().0.len()
}

fn files(dir: &std::path::Path) -> usize {
    std::fs::read_dir(dir).unwrap().count()
}

#[test]
fn leaving_the_page_stops_and_saves_a_recording_and_none_starts_until_it_shows() {
    let tmp = tempfile::tempdir().unwrap();
    let heard = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let (mut p, _fake) = parts(tmp.path(), Ok((Vec::new(), 16_000)), true);
    p.source = streaming(heard.clone(), None);
    let mut service = Service::new(p);
    service.record_start(None).unwrap();
    until(&mut service, "recording", |s| phase(s) == "recording");
    until(&mut service, "some audio", |_| heard.load(Ordering::Acquire) >= 3_200);
    service.leave();
    until(&mut service, "saved", |s| phase(s) == "saved");
    let id = service.state()["recording"]["id"].as_str().unwrap().to_owned();
    let at_stop = heard.load(Ordering::Acquire);
    // The microphone is off: nothing it would hear later is taken.
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(heard.load(Ordering::Acquire), at_stop, "the microphone stopped");
    assert_eq!(memo_len(&service, &id), at_stop, "what was said until it left, no more");
    // Away, Record is refused; shown again, it records.
    let refused = service.record_start(None).unwrap_err();
    assert!(refused.contains("Open Ideas"), "{refused}");
    assert_eq!(phase(&service), "saved", "a refused start changes nothing");
    service.here();
    service.record_start(None).unwrap();
    until(&mut service, "recording again", |s| phase(s) == "recording");
    service.record_cancel();
}

#[test]
fn leaving_while_the_microphone_starts_lets_it_go_and_keeps_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let heard = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let (open, gate) = mpsc::channel();
    let (mut p, _fake) = parts(tmp.path(), Ok((Vec::new(), 16_000)), true);
    p.source = streaming(heard.clone(), Some(gate));
    let mut service = Service::new(p);
    service.record_start(None).unwrap();
    service.poll();
    assert_eq!(phase(&service), "starting");
    service.leave();
    assert_eq!(phase(&service), "idle");
    // The microphone would have been ready only now: it doesn't go on.
    open.send(()).unwrap();
    std::thread::sleep(Duration::from_millis(100));
    service.poll();
    assert_eq!(heard.load(Ordering::Acquire), 0, "it never listened");
    assert_eq!(phase(&service), "idle");
    assert_eq!(files(tmp.path()), 0, "nothing kept");
    assert!(service.record_start(None).is_err(), "nor does a start asked after it");
}

/// The page's recording, as the host has it.
fn host_phase(ideas: &Ideas) -> String {
    phase(&ideas.service.borrow())
}

#[test]
fn a_save_that_fails_keeps_the_draft_and_what_was_typed_since() {
    let tmp = tempfile::tempdir().unwrap();
    let (p, _fake) = parts(tmp.path(), Ok((vec![0.2; 16_000], 16_000)), true);
    let (mut app, ideas) = mount(Service::new(p));
    app.click("#sidebar-ideas");
    wait(&mut app, &ideas, "the page", |a| a.find("#ideas-page").is_some());
    let box_ = "#ideas-page >> role:textbox";
    // The folder can't be written: the idea can't be kept.
    let mut ro = std::fs::metadata(tmp.path()).unwrap().permissions();
    ro.set_readonly(true);
    std::fs::set_permissions(tmp.path(), ro.clone()).unwrap();
    app.fill(box_, "call the plumber");
    app.click("#ideas-save");
    wait(&mut app, &ideas, "the failure", |a| a.texts().iter().any(|t| t == "Couldn't do that"));
    assert_eq!(app.text(box_), "call the plumber", "the draft stays");
    app.type_text(box_, " about the sink");
    app.click("#ideas-save");
    app.frames(10);
    assert_eq!(app.text(box_), "call the plumber about the sink", "and what was typed since");
    assert!(app.find("#ideas-empty").is_some(), "nothing was saved");
    // Writable again: saved, and only then is the box cleared.
    ro.set_readonly(false);
    std::fs::set_permissions(tmp.path(), ro).unwrap();
    app.click("#ideas-save");
    wait(&mut app, &ideas, "the idea", |a| a.texts().iter().any(|t| t == "call the plumber about the sink") && a.find(".idea-typed").is_some());
    assert_ne!(app.text(box_), "call the plumber about the sink", "the box is cleared once it's kept");
    assert_eq!(app.error(), None);
}

#[test]
fn going_to_settings_a_chat_or_back_stops_and_saves_the_recording() {
    let tmp = tempfile::tempdir().unwrap();
    let heard = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let (mut p, _fake) = parts(tmp.path(), Ok((Vec::new(), 16_000)), true);
    p.source = streaming(heard.clone(), None);
    let (mut app, ideas) = mount(Service::new(p));
    let memos = |ideas: &Ideas| ideas.service.borrow().state()["items"].as_array().unwrap().len();
    let record = |app: &mut TestApp, what: &str| {
        app.click("#ideas-record");
        wait(app, &ideas, what, |a| a.find("#ideas-stop").is_some() && a.text("#ideas-recording-status").starts_with("Recording"));
        let before = heard.load(Ordering::Acquire);
        wait(app, &ideas, "some audio", |_| heard.load(Ordering::Acquire) >= before + 1_600);
    };
    let left = |app: &mut TestApp, ideas: &Ideas, n: usize, what: &str| {
        wait(app, ideas, what, |_| host_phase(ideas) == "saved" && memos(ideas) == n);
        let at_stop = heard.load(Ordering::Acquire);
        std::thread::sleep(Duration::from_millis(60));
        ideas.tick();
        assert_eq!(heard.load(Ordering::Acquire), at_stop, "{what}: the microphone is off");
    };

    // Settings, and back from it.
    app.click("#sidebar-ideas");
    wait(&mut app, &ideas, "the page", |a| a.find("#ideas-page").is_some());
    record(&mut app, "recording");
    app.click("role:checkbox \"Settings\"");
    left(&mut app, &ideas, 1, "into Settings");
    app.click("role:checkbox \"Settings\"");
    wait(&mut app, &ideas, "the page again", |a| a.shown("#ideas-page"));

    // Another destination: a chat.
    record(&mut app, "recording again");
    let chat = "\"Write the docs\"";
    assert!(app.find(chat).is_some(), "{:?}", app.texts());
    app.click(chat);
    left(&mut app, &ideas, 2, "into a chat");
    assert!(ideas.service.borrow_mut().record_start(None).is_err(), "no start while it isn't shown");

    // Back to the page (the title bar's Back): it records again.
    app.click("#titlebar >> role:button \"Back\"");
    wait(&mut app, &ideas, "back", |a| a.shown("#ideas-page"));
    record(&mut app, "recording after back");
    app.click("#ideas-stop");
    left(&mut app, &ideas, 3, "stopped by hand");
    assert_eq!(app.error(), None);
}

/// A microphone that hears silence until `loud` is set, then a 440 Hz sine
/// of amplitude `TONE`, 160 samples (10 ms) a beat, and meters each beat
/// as the real capture does (its RMS into `level`).
const TONE: f32 = 0.1;
fn signal(loud: Arc<AtomicBool>) -> Source {
    Arc::new(move |_device, stop: &AtomicBool, opened: &AtomicBool, level: &Arc<AtomicU32>| {
        opened.store(true, Ordering::Release);
        let mut all = Vec::new();
        while !stop.load(Ordering::Acquire) {
            let on = loud.load(Ordering::Acquire);
            let beat: Vec<f32> = (0..160)
                .map(|i| if on { TONE * (std::f32::consts::TAU * 440.0 * (all.len() + i) as f32 / 16_000.0).sin() } else { 0.0 })
                .collect();
            level.fetch_max(rms(&beat).to_bits(), Ordering::Relaxed);
            all.extend(beat);
            std::thread::sleep(Duration::from_millis(10));
        }
        Ok((all, 16_000))
    })
}

/// Polls for `ms` as the window's loop does, and the bars published last.
fn poll_for(service: &mut Service, ms: u64, bars: &mut Vec<f32>) {
    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(ms) {
        service.poll();
        if let Some(b) = service.take_levels() {
            *bars = b;
        }
        std::thread::sleep(service.poll_wait());
    }
}

#[test]
fn the_waveform_follows_what_the_memo_hears_and_goes_when_it_ends() {
    use crate::dictation::meter::{BARS, normalize};
    let tmp = tempfile::tempdir().unwrap();
    let loud = Arc::new(AtomicBool::new(false));
    let (mut p, _fake) = parts(tmp.path(), Ok((Vec::new(), 16_000)), true);
    p.source = signal(loud.clone());
    let mut service = Service::new(p);
    let tone = normalize(TONE / std::f32::consts::SQRT_2);
    for end in ["stop", "discard", "leave"] {
        service.here();
        service.record_start(None).unwrap();
        let mut bars = vec![1.0];
        until(&mut service, "recording", |s| phase(s) == "recording");
        // Silence: bars at a slot each, every one quiet.
        poll_for(&mut service, 400, &mut bars);
        assert!(bars.len() >= 4 && bars.len() <= BARS, "{end}: {bars:?}");
        assert!(bars.iter().all(|b| *b == 0.0), "{end}: silence is quiet: {bars:?}");
        // The signal: the newest bars are its loudness, the older ones still quiet.
        loud.store(true, Ordering::Release);
        poll_for(&mut service, 300, &mut bars);
        assert!((bars[0] - tone).abs() < 0.02, "{end}: {} against {tone}", bars[0]);
        assert_eq!(*bars.last().unwrap(), 0.0, "{end}: the silence scrolled on");
        loud.store(false, Ordering::Release);
        match end {
            "stop" => service.record_stop(),
            "discard" => service.record_cancel(),
            _ => service.leave(),
        }
        assert_eq!(service.take_levels(), Some(vec![]), "{end}: the waveform is gone");
        bars.clear();
        poll_for(&mut service, 100, &mut bars);
        assert!(bars.is_empty(), "{end}: nothing comes back after: {bars:?}");
        until(&mut service, "ended", |s| s.recorder.is_none());
        assert!(service.take_levels().unwrap_or_default().is_empty(), "{end}: nothing after the end");
    }
}

/// Bar `i`'s height in the page's waveform (1: the newest), in points.
fn bar_h(app: &mut TestApp, i: usize) -> f32 {
    app.bounds(&format!("#ideas-voice >> #voice-bar-{i}")).height as f32
}

#[test]
fn the_page_shows_dictation_s_waveform_while_it_records_and_not_after() {
    let tmp = tempfile::tempdir().unwrap();
    let loud = Arc::new(AtomicBool::new(false));
    let (mut p, _fake) = parts(tmp.path(), Ok((Vec::new(), 16_000)), true);
    p.source = signal(loud.clone());
    let (mut app, ideas) = mount(Service::new(p));
    app.click("#sidebar-ideas");
    wait(&mut app, &ideas, "the page", |a| a.find("#ideas-page").is_some());
    assert!(app.find("#ideas-voice").is_none(), "no waveform before Record");
    for (end, target) in [("stop", "#ideas-stop"), ("discard", "#ideas-discard"), ("leave", "role:checkbox \"Settings\"")] {
        app.click("#ideas-record");
        wait(&mut app, &ideas, "recording", |a| a.text("#ideas-recording-status").starts_with("Recording") && a.shown("#voice-wave"));
        let start = Instant::now();
        wait(&mut app, &ideas, "silent slots", |_| start.elapsed() > Duration::from_millis(300));
        assert_eq!(bar_h(&mut app, 1), 3.0, "{end}: silence draws the baseline's dot");
        loud.store(true, Ordering::Release);
        let start = Instant::now();
        wait(&mut app, &ideas, "loud slots", |a| start.elapsed() > Duration::from_millis(250) && bar_h(a, 1) > 3.0);
        // 0.08 + 0.92 × the tone's level, of 16 points.
        let expect = (0.08 + 0.92 * crate::dictation::meter::normalize(TONE / std::f32::consts::SQRT_2)) * 16.0;
        let h = bar_h(&mut app, 1);
        assert!((h - expect).abs() < 0.6, "{end}: {h} against {expect}");
        loud.store(false, Ordering::Release);
        app.click(target);
        wait(&mut app, &ideas, "gone", |a| a.find("#ideas-voice").is_none() && ideas.service.borrow().recorder.is_none());
        if end == "leave" {
            app.click("role:checkbox \"Settings\"");
            wait(&mut app, &ideas, "the page again", |a| a.shown("#ideas-page"));
        }
        assert!(app.find("#ideas-voice").is_none(), "{end}: no waveform after");
    }
    assert_eq!(app.error(), None);
}
