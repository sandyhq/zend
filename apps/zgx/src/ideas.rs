//! Ideas & Intents (ui/screens/ideas): typed ideas and voice memos, kept on
//! this device in `<data>/ideas` (src/ideas/store.rs), no project, chat or
//! engine involved.
//!
//! A memo is recorded only between an explicit Record and Stop. Its audio
//! is written to disk as it was recorded the moment the microphone closes,
//! and only then transcribed, on a worker of its own, with the same local
//! model dictation uses (Settings → Voice). Whatever the transcription does
//! (no model, a failure, the app quitting), the audio stays and plays.
//!
//! Dictation in the composer is untouched: it still keeps no audio
//! (src/dictation.rs). Only this page writes recordings, and only here.
//!
//! The window's scripts call `ideas.call { op, … }` and read the shared
//! `ideas` (`{ ready, error?, model, items, recording, playing, play_error }`),
//! and, while recording, the shared `ideas_levels`: the microphone's loudness
//! as dictation's waveform draws it (src/dictation/meter.rs, ds/voice_wave),
//! from the samples the recording takes, never a stream of its own.
use serde_json::{Value as Json, json};
use std::{
    cell::RefCell,
    collections::BTreeSet,
    path::PathBuf,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU32, Ordering},
        mpsc,
    },
    time::Duration,
};
use zgx_desktop::WindowContext;
use zgx_script::{ScriptHost, Value};

use crate::plugins::{json_value, value_json};

mod store;
mod wav;
pub use store::{Idea, Store, Transcript};

/// A recording: mono samples and their rate.
pub type Samples = (Vec<f32>, u32);

/// Where a memo's sound comes from. Runs on a thread of its own: sets
/// `opened` once it hears, keeps the loudest RMS of what it hears in `level`
/// (f32 bits, as `zend_voice::Capture` does; the page's waveform takes it),
/// and returns what it heard once `stop` is set (or it can take no more). The microphone, or for an offscreen test
/// instance a WAV file played into it in real time (`ZEND_IDEAS_TEST_MIC`).
pub type Source = Arc<dyn Fn(Option<String>, &AtomicBool, &AtomicBool, &Arc<AtomicU32>) -> Result<Samples, String> + Send + Sync>;

/// Why a memo has no transcript.
#[derive(Debug, Clone, PartialEq)]
pub enum Failure {
    NoModel,
    Failed(String),
}

/// Turns a memo's audio into words, on the transcription worker.
pub trait Transcribe {
    fn transcribe(&mut self, samples: Vec<f32>, rate: u32) -> Result<String, Failure>;
    /// Nothing asked for a while: let go of what is loaded.
    fn idle(&mut self) {}
}
/// Makes the transcriber, on the worker (a model is loaded where it runs).
pub type MakeTranscriber = Box<dyn FnOnce() -> Box<dyn Transcribe> + Send>;

/// A memo playing; dropping it stops it.
pub trait Playing {
    fn ended(&self) -> bool;
}
pub type Speaker = Box<dyn Fn(Vec<f32>, u32) -> Result<Box<dyn Playing>, String>>;

/// What the service is made of: the real ones in `install`, fakes in tests.
pub struct Parts {
    pub dir: Option<PathBuf>,
    pub source: Source,
    pub transcriber: MakeTranscriber,
    pub speaker: Speaker,
    /// Whether the speech model is on this device.
    pub model: Box<dyn Fn() -> bool>,
    /// Asks for the microphone where the system wants that asked (macOS).
    pub authorize: Box<dyn Fn() -> Result<(), String>>,
}

/// Parakeet v3, as dictation runs it, loaded on first use and let go when idle.
struct Parakeet {
    dir: Option<PathBuf>,
    loaded: Option<zend_voice::Recognizer>,
}
impl Transcribe for Parakeet {
    fn transcribe(&mut self, samples: Vec<f32>, rate: u32) -> Result<String, Failure> {
        let dir = self.dir.as_ref().ok_or_else(|| Failure::Failed("Zend's data folder isn't known".into()))?;
        if !zend_voice::installed(dir) {
            self.loaded = None;
            return Err(Failure::NoModel);
        }
        if self.loaded.is_none() {
            let model = zend_voice::Recognizer::load(dir).map_err(|e| Failure::Failed(format!("Couldn't load the speech model: {e}")))?;
            self.loaded = Some(model);
        }
        let model = self.loaded.as_mut().expect("loaded above");
        model.transcribe(samples, rate).map_err(|e| Failure::Failed(e.to_string()))
    }
    fn idle(&mut self) {
        self.loaded = None;
    }
}

struct Played(zend_voice::Playback);
impl Playing for Played {
    fn ended(&self) -> bool {
        self.0.ended()
    }
}

/// The microphone, until stopped, at most `MAX_SECONDS` (the model's limit).
fn microphone(device: Option<String>, stop: &AtomicBool, opened: &AtomicBool, level: &Arc<AtomicU32>) -> Result<Samples, String> {
    // (Left or discarded before it began: the microphone isn't opened at all.)
    if stop.load(Ordering::Acquire) {
        return Ok((Vec::new(), 16_000));
    }
    // (The level is the capture's own, from the samples it records.)
    let capture = zend_voice::Capture::start(device.as_deref(), level.clone()).map_err(|e| e.to_string())?;
    opened.store(true, Ordering::Release);
    let started = std::time::Instant::now();
    while !stop.load(Ordering::Acquire) && !capture.ended() && started.elapsed().as_secs() < zend_voice::MAX_SECONDS as u64 {
        std::thread::sleep(Duration::from_millis(10));
    }
    // (The voice crate's words are dictation's: a draft. Here it's a memo.)
    capture.finish().map_err(|_| "The microphone disconnected while recording".to_string())
}

/// A WAV file as if it were said into the microphone: as much of it as has
/// "been said" by the time Stop comes (all of it at its end). Only for an
/// offscreen test instance with no microphone (`ZEND_IDEAS_TEST_MIC`).
fn file_source(path: PathBuf) -> Source {
    Arc::new(move |_device, stop: &AtomicBool, opened: &AtomicBool, level: &Arc<AtomicU32>| {
        let bytes = std::fs::read(&path).map_err(|e| format!("No microphone available ({}: {e})", path.display()))?;
        let (samples, rate) = wav::decode(&bytes)?;
        let max = rate as usize * zend_voice::MAX_SECONDS;
        opened.store(true, Ordering::Release);
        let started = std::time::Instant::now();
        let mut metered = 0;
        loop {
            let said = (started.elapsed().as_secs_f64() * rate as f64) as usize;
            let n = said.min(samples.len()).min(max);
            if n > metered {
                level.fetch_max(rms(&samples[metered..n]).to_bits(), Ordering::Relaxed);
                metered = n;
            }
            if stop.load(Ordering::Acquire) || said >= samples.len().min(max) {
                return Ok((samples[..n].to_vec(), rate));
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    })
}

/// The loudness of a stretch of samples, as the voice crate's capture measures it.
pub(crate) fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let r = (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt();
    if r.is_finite() { r } else { 0.0 }
}

impl Parts {
    fn real() -> Self {
        let data = crate::engine::data_dir();
        let model_dir = data.as_ref().map(|d| d.join("models").join(crate::dictation::MODEL));
        let source = match std::env::var_os("ZEND_IDEAS_TEST_MIC") {
            Some(path) if !path.is_empty() => file_source(PathBuf::from(path)),
            _ => Arc::new(microphone) as Source,
        };
        let for_worker = model_dir.clone();
        Self {
            dir: data.map(|d| d.join("ideas")),
            source,
            transcriber: Box::new(move || Box::new(Parakeet { dir: for_worker, loaded: None }) as Box<dyn Transcribe>),
            speaker: Box::new(|samples, rate| {
                zend_voice::Playback::start(samples, rate)
                    .map(|p| Box::new(Played(p)) as Box<dyn Playing>)
                    .map_err(|e| e.to_string())
            }),
            model: Box::new(move || model_dir.as_deref().is_some_and(zend_voice::installed)),
            authorize: Box::new(crate::dictation::authorize),
        }
    }
}

struct Recorder {
    stop: Arc<AtomicBool>,
    opened: Arc<AtomicBool>,
    /// The loudest RMS heard since the last poll (f32 bits).
    level: Arc<AtomicU32>,
    result: mpsc::Receiver<Result<Samples, String>>,
}

/// The transcription worker: one memo at a time, in the order saved.
struct Worker {
    jobs: mpsc::Sender<(String, Samples)>,
    results: mpsc::Receiver<(String, Result<String, Failure>)>,
}
impl Worker {
    fn spawn(make: MakeTranscriber) -> Self {
        let (jobs, inbox) = mpsc::channel::<(String, Samples)>();
        let (tell, results) = mpsc::channel();
        std::thread::Builder::new()
            .name("ideas-transcribe".into())
            .spawn(move || {
                let mut transcriber = make();
                loop {
                    let (id, (samples, rate)) = match inbox.recv_timeout(Duration::from_secs(60)) {
                        Ok(job) => job,
                        Err(mpsc::RecvTimeoutError::Timeout) => {
                            transcriber.idle();
                            continue;
                        }
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    };
                    // A native panic fails this memo, not every later one.
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| transcriber.transcribe(samples, rate)))
                        .unwrap_or_else(|_| {
                            transcriber.idle();
                            Err(Failure::Failed("Transcription stopped unexpectedly".into()))
                        });
                    if tell.send((id, result)).is_err() {
                        break;
                    }
                }
            })
            .expect("a thread for transcription");
        Self { jobs, results }
    }
}

pub(crate) struct Service {
    store: Result<Store, String>,
    source: Source,
    speaker: Speaker,
    model_check: Box<dyn Fn() -> bool>,
    authorize: Box<dyn Fn() -> Result<(), String>>,
    make: Option<MakeTranscriber>,
    worker: Option<Worker>,
    recorder: Option<Recorder>,
    recording: Json,
    started: Option<std::time::Instant>,
    transcribing: BTreeSet<String>,
    playing: Option<(String, Box<dyn Playing>)>,
    play_error: Option<(String, String)>,
    model: bool,
    items: Vec<Idea>,
    /// The page isn't shown (`leave`): nothing starts recording until it is again.
    away: bool,
    /// The waveform while recording (dictation's), and whether its bars
    /// changed since they were last published (`ideas_levels`).
    meter: crate::dictation::meter::Meter,
    levels_changed: bool,
}

impl Service {
    pub(crate) fn new(parts: Parts) -> Self {
        let store = match parts.dir {
            Some(dir) => Store::open(dir),
            None => Err("Zend's data folder isn't known".into()),
        };
        let items = store.as_ref().map(Store::list).unwrap_or_default();
        let model = (parts.model)();
        Self {
            store,
            source: parts.source,
            speaker: parts.speaker,
            model_check: parts.model,
            authorize: parts.authorize,
            make: Some(parts.transcriber),
            worker: None,
            recorder: None,
            recording: json!({"phase":"idle"}),
            started: None,
            transcribing: BTreeSet::new(),
            playing: None,
            play_error: None,
            model,
            items,
            away: false,
            meter: Default::default(),
            levels_changed: false,
        }
    }

    /// The waveform's bars, newest first, once each time they change.
    pub(crate) fn take_levels(&mut self) -> Option<Vec<f32>> {
        std::mem::take(&mut self.levels_changed).then(|| self.meter.newest())
    }

    /// No waveform: not recording any more (stopped, discarded, left, ended).
    fn clear_levels(&mut self) {
        self.meter.reset();
        self.levels_changed = true;
    }

    /// How long the loop waits: to the next slot while recording (the bars
    /// move once a slot, as dictation's do), else its usual beat.
    pub(crate) fn poll_wait(&self) -> Duration {
        const BEAT: Duration = Duration::from_millis(100);
        self.meter
            .until_next(std::time::Instant::now())
            .map_or(BEAT, |d| (d + Duration::from_millis(1)).min(Duration::from_millis(40)))
    }

    fn store(&self) -> Result<&Store, String> {
        self.store.as_ref().map_err(Clone::clone)
    }

    fn reload(&mut self) {
        self.items = self.store.as_ref().map(Store::list).unwrap_or_default();
    }

    pub(crate) fn needs_poll(&self) -> bool {
        self.recorder.is_some() || !self.transcribing.is_empty() || self.playing.is_some()
    }

    /// What the page shows.
    pub(crate) fn state(&self) -> Json {
        let items: Vec<Json> = self
            .items
            .iter()
            .map(|i| {
                let busy = self.transcribing.contains(&i.id);
                // (Pending with nothing working on it: the app quit while it was.)
                let transcript = match i.transcript {
                    None => Json::Null,
                    Some(_) if busy => json!("transcribing"),
                    Some(Transcript::Pending) => json!("interrupted"),
                    Some(t) => json!(t),
                };
                json!({
                    "id": i.id,
                    "created": i.created,
                    "text": i.text,
                    "memo": i.audio.is_some(),
                    "seconds": i.seconds,
                    "transcript": transcript,
                    "error": i.error,
                })
            })
            .collect();
        json!({
            "ready": self.store.is_ok(),
            "error": self.store.as_ref().err(),
            "model": self.model,
            "items": items,
            "recording": self.recording,
            "playing": self.playing.as_ref().map(|(id, _)| id),
            "play_error": self.play_error.as_ref().map(|(id, e)| json!({"id": id, "error": e})),
        })
    }

    pub(crate) fn add(&mut self, text: &str) -> Result<Json, String> {
        let idea = self.store()?.add_text(text)?;
        self.reload();
        Ok(json!({"id": idea.id}))
    }

    pub(crate) fn delete(&mut self, id: &str) -> Result<(), String> {
        if self.playing.as_ref().is_some_and(|(p, _)| p == id) {
            self.playing = None;
        }
        self.store()?.delete(id)?;
        self.transcribing.remove(id);
        self.reload();
        Ok(())
    }

    pub(crate) fn record_start(&mut self, device: Option<String>) -> Result<(), String> {
        self.store()?;
        // (A start that comes after the page was left never records unseen.)
        if self.away {
            return Err("Open Ideas & Intents to record a memo".into());
        }
        if self.recorder.is_some() {
            return Err("Already recording".into());
        }
        let failed = |me: &mut Self, error: String| {
            me.recording = json!({"phase":"failed", "kind":"microphone", "error": error});
            Err(error)
        };
        if zend_voice::busy() {
            return failed(self, "Dictation is using the microphone. Try again when it finishes.".into());
        }
        if let Err(e) = (self.authorize)() {
            return failed(self, e);
        }
        // (Nothing plays over a recording.)
        self.playing = None;
        let stop = Arc::new(AtomicBool::new(false));
        let opened = Arc::new(AtomicBool::new(false));
        let level = Arc::new(AtomicU32::new(0));
        let (tell, result) = mpsc::sync_channel(1);
        let (source, s, o, l) = (self.source.clone(), stop.clone(), opened.clone(), level.clone());
        std::thread::Builder::new()
            .name("ideas-record".into())
            .spawn(move || {
                let heard = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| source(device, &s, &o, &l)))
                    .unwrap_or_else(|_| Err("The microphone stopped unexpectedly".into()));
                let _ = tell.send(heard);
            })
            .map_err(|e| e.to_string())?;
        self.recorder = Some(Recorder { stop, opened, level, result });
        self.started = None;
        self.clear_levels();
        self.recording = json!({"phase":"starting"});
        Ok(())
    }

    /// Stop: what was heard is saved, then transcribed.
    pub(crate) fn record_stop(&mut self) {
        if let Some(r) = &self.recorder {
            r.stop.store(true, Ordering::Release);
            self.recording = json!({"phase":"saving"});
            self.clear_levels();
        }
    }

    /// Discard: nothing heard is kept.
    pub(crate) fn record_cancel(&mut self) {
        if let Some(r) = self.recorder.take() {
            r.stop.store(true, Ordering::Release);
        }
        self.recording = json!({"phase":"idle"});
        self.clear_levels();
    }

    /// The page shows again: Record may start.
    pub(crate) fn here(&mut self) {
        self.away = false;
    }

    /// The page was left (Settings, a chat, another page): no recording goes
    /// on unseen. One that records is stopped and saved, with what was said
    /// until now; one whose microphone is still starting is let go, nothing
    /// kept, and a start asked after this one is refused.
    pub(crate) fn leave(&mut self) {
        self.away = true;
        match self.recording["phase"].as_str() {
            Some("starting") => self.record_cancel(),
            Some("recording") => self.record_stop(),
            _ => {}
        }
    }

    fn queue(&mut self, id: String, audio: Samples) {
        if self.worker.is_none() {
            let Some(make) = self.make.take() else { return };
            self.worker = Some(Worker::spawn(make));
        }
        if let Some(w) = &self.worker {
            if w.jobs.send((id.clone(), audio)).is_ok() {
                self.transcribing.insert(id);
            }
        }
    }

    /// Transcribe a memo again (after the model arrived, or a failure).
    pub(crate) fn transcribe(&mut self, id: &str) -> Result<(), String> {
        if self.transcribing.contains(id) {
            return Ok(());
        }
        let audio = self.store()?.audio(id)?;
        self.queue(id.to_owned(), audio);
        Ok(())
    }

    pub(crate) fn play(&mut self, id: &str) -> Result<(), String> {
        self.playing = None;
        self.play_error = None;
        let started = self.store().and_then(|s| s.audio(id)).and_then(|(samples, rate)| (self.speaker)(samples, rate));
        match started {
            Ok(p) => {
                self.playing = Some((id.to_owned(), p));
                Ok(())
            }
            Err(e) => {
                self.play_error = Some((id.to_owned(), e.clone()));
                Err(e)
            }
        }
    }

    pub(crate) fn play_stop(&mut self) {
        self.playing = None;
    }

    pub(crate) fn refresh(&mut self) {
        self.model = (self.model_check)();
        self.reload();
    }

    pub(crate) fn poll(&mut self) {
        // The recording.
        let mut heard = None;
        if let Some(r) = &self.recorder {
            if self.recording["phase"] == "starting" && r.opened.load(Ordering::Acquire) {
                let now = std::time::Instant::now();
                self.started = Some(now);
                self.recording = json!({"phase":"recording", "seconds": 0});
                // (Heard while opening counts toward the first slot.)
                self.meter.start(now);
            }
            if self.recording["phase"] == "recording" {
                let rms = f32::from_bits(r.level.swap(0, Ordering::Relaxed));
                if self.meter.record(std::time::Instant::now(), rms) {
                    self.levels_changed = true;
                }
            }
            if let Some(started) = self.started.filter(|_| self.recording["phase"] == "recording") {
                self.recording["seconds"] = json!(started.elapsed().as_secs());
            }
            match r.result.try_recv() {
                Ok(result) => heard = Some(result),
                Err(mpsc::TryRecvError::Disconnected) => heard = Some(Err("The microphone stopped unexpectedly".into())),
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if let Some(result) = heard {
            self.recorder = None;
            self.started = None;
            self.clear_levels();
            match result {
                Err(error) => self.recording = json!({"phase":"failed", "kind":"microphone", "error": error}),
                Ok((samples, rate)) if samples.len() < rate as usize / 10 => {
                    self.recording = json!({"phase":"failed", "kind":"empty", "error": "Nothing was recorded"});
                }
                Ok((samples, rate)) => match self.store().and_then(|s| s.add_memo(&samples, rate)) {
                    Err(error) => self.recording = json!({"phase":"failed", "kind":"save", "error": error}),
                    // Saved: only now is it transcribed.
                    Ok(idea) => {
                        self.recording = json!({"phase":"saved", "id": idea.id});
                        self.model = (self.model_check)();
                        self.reload();
                        self.queue(idea.id, (samples, rate));
                    }
                },
            }
        }
        // Transcripts.
        let mut done = Vec::new();
        if let Some(w) = &self.worker {
            while let Ok(r) = w.results.try_recv() {
                done.push(r);
            }
        }
        for (id, result) in done {
            self.transcribing.remove(&id);
            let Ok(store) = self.store() else { continue };
            // (Deleted meanwhile: nothing to write.)
            let Some(mut idea) = store.get(&id) else { continue };
            match result {
                Ok(text) => {
                    idea.text = text.trim().to_owned();
                    idea.transcript = Some(Transcript::Done);
                    idea.error = None;
                }
                Err(Failure::NoModel) => {
                    idea.transcript = Some(Transcript::NoModel);
                    idea.error = None;
                }
                Err(Failure::Failed(e)) => {
                    idea.transcript = Some(Transcript::Failed);
                    idea.error = Some(e);
                }
            }
            let _ = store.put(&idea);
            self.model = (self.model_check)();
            self.reload();
        }
        // Playback.
        if self.playing.as_ref().is_some_and(|(_, p)| p.ended()) {
            self.playing = None;
        }
    }
}

pub(crate) struct Ideas {
    shared: zgx_script::Shared,
    /// The waveform's bars (`ideas_levels`), on their own so readers of
    /// `ideas` don't run again at each slot.
    levels: zgx_script::Shared,
    pub(crate) service: RefCell<Service>,
    published: RefCell<Json>,
}
impl Ideas {
    pub(crate) fn publish(&self) {
        let now = self.service.borrow().state();
        if *self.published.borrow() != now {
            self.shared.set(json_value(&now));
            *self.published.borrow_mut() = now;
        }
        let levels = self.service.borrow_mut().take_levels();
        if let Some(levels) = levels {
            self.levels.set(crate::dictation::levels_value(&levels));
        }
    }
    /// One beat of the loop: what finished, then what shows.
    pub(crate) fn tick(&self) {
        self.service.borrow_mut().poll();
        self.publish();
    }
}

/// `ideas.call` on `host`, answered by `service`; `wake` is told after each
/// call that may have started something to watch.
pub(crate) fn attach(host: &ScriptHost, service: Service, wake: impl Fn() + 'static) -> Rc<Ideas> {
    let me = Rc::new(Ideas {
        shared: host.shared("ideas", Value::Nil),
        levels: host.shared("ideas_levels", json_value(&json!([]))),
        service: RefCell::new(service),
        published: RefCell::new(Json::Null),
    });
    me.publish();
    let weak = Rc::downgrade(&me);
    let wake = Rc::new(wake);
    host.async_action("ideas.call", move |asked: Value| {
        let weak = weak.clone();
        let wake = wake.clone();
        async move {
            let me = weak.upgrade().ok_or("The window closed")?;
            let args = value_json(&asked);
            let id = args["id"].as_str().unwrap_or_default().to_owned();
            let result = {
                let mut s = me.service.borrow_mut();
                match args["op"].as_str() {
                    Some("list") => {
                        s.refresh();
                        Ok(Json::Null)
                    }
                    Some("add") => s.add(args["text"].as_str().unwrap_or_default()),
                    Some("delete") => s.delete(&id).map(|_| Json::Null),
                    Some("record_start") => {
                        let device = args["device"].as_str().filter(|d| !d.is_empty() && *d != "system").map(str::to_owned);
                        s.record_start(device).map(|_| Json::Null)
                    }
                    Some("record_stop") => {
                        s.record_stop();
                        Ok(Json::Null)
                    }
                    Some("record_cancel") => {
                        s.record_cancel();
                        Ok(Json::Null)
                    }
                    Some("here") => {
                        s.here();
                        Ok(Json::Null)
                    }
                    Some("leave") => {
                        s.leave();
                        Ok(Json::Null)
                    }
                    Some("transcribe") => s.transcribe(&id).map(|_| Json::Null),
                    Some("play") => s.play(&id).map(|_| Json::Null),
                    Some("play_stop") => {
                        s.play_stop();
                        Ok(Json::Null)
                    }
                    _ => Err("Unknown ideas operation".to_string()),
                }
            };
            me.publish();
            wake();
            result.map(|v| json_value(&v))
        }
    });
    me
}

/// Ideas & Intents for the window's scripts: `ideas.call` and the shared
/// `ideas`. A recording, a transcription or a memo playing keeps a beat
/// going while it lasts; nothing runs otherwise.
pub fn install(cx: &mut WindowContext, host: &ScriptHost) {
    let (wake, mut work) = zgx::task::latest::<()>();
    let me = attach(host, Service::new(Parts::real()), move || {
        wake.send(());
    });
    cx.tasks.spawn(async move {
        while work.recv().await.is_some() {
            while me.service.borrow().needs_poll() {
                me.tick();
                let wait = me.service.borrow().poll_wait();
                zgx::timer::sleep(wait).await;
            }
            me.tick();
        }
    });
}

#[cfg(test)]
#[path = "ideas_tests.rs"]
mod tests;
