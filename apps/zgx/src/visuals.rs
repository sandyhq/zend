//! Agent visuals (`show_visual`) for the scripts: the shipping app's own
//! runtime (crates/visual) run here, its tree handed to `ds/visual` as
//! plain data, and the reader's input routed back to the script.
//!
//! A visual reaches a chat's transcript as a `visual` part: its Luau script
//! stored on the chat's host (crates/engine uploads.rs `store_visual`), its
//! files beside it. The script is read once through the engine
//! (`ReadAttachmentChunk`, from that device when it isn't this one), and
//! runs in crates/visual's sandbox exactly as the shipping app runs it: no
//! I/O, its memory capped, each call on a time budget, `math.random` seeded
//! from the visual's id, so the reader sees the data the agent checked.
//!
//! The calls:
//!   visuals.open { id, path, title, device?, source? }   (`ui.call`)
//!       reads and starts it, once (again for one that failed to read).
//!       `source` stands in for the file (a fixture's, a test's).
//!   visuals.get id                                        (`ui.call_sync`)
//!       { state = "loading" | "ready" | "failed", title, error?, tree? }:
//!       the tree as plain data (nodes `{ k, p, … }`, handlers by number),
//!       `error` the latest handler or draw failure (the last good tree
//!       stays) or why it failed to load.
//!   visuals.event { id, h, arg? }                          (`ui.action`)
//!       a control's handler with its value (nil, a string, a boolean, a
//!       number, `{ x, y }`), then the view rendered again.
//!   visuals.draw { id, h, w, ht, t, px?, py?, down? }     (`ui.call_sync`)
//!       a canvas's drawing at a size and time: `{ cmds, pointer }` (or
//!       `{ error }`), each command `{ k = "s" | "f" | "t", … }`.
//!   visuals.frame { id, p, w, ht, scale, t, pal, yaw, pitch, mouse? }
//!       (`ui.call`) a scene's or shader's frame, rendered off this thread
//!       on the GPU (crates/visual-gpu, the shipping app's), shown from
//!       `src` (`host:visual/<id>/<p>`): `{ src, labels, hints }`.
//!   visuals.release { id, p }                              (`ui.action`)
//!       a scene or shader gone from the screen lets its GPU state go.
//!   visuals.input { id, name, value? }                     (`ui.call`)
//!       the reader entered `value` in the input `name` (a string, a
//!       boolean, a list of strings; nil clears it): kept beside the
//!       visual's script on the chat's host (engine `SaveVisualEntries`,
//!       `{stem}-entries.json`), read back as it opens again.
//!   visuals.focus { id, name? }                            (`ui.action`)
//!       the input `name` took focus (none: focus left): while it has it,
//!       a new script doesn't change what it shows.
//!   visuals.entries id                                     (`ui.call_sync`)
//!       what the reader entered, `{ [name] = value }`.
//!   visuals.answer id                                      (`ui.call_sync`)
//!       its inputs as they show, as words for the composer (crates/visual
//!       `answer_text`: the title, a line per input), for `ui.answer`.
//!   visuals.swap { id, source }                            (`ui.call`)
//!       a new script for a visual that's open, its entries kept (an error
//!       leaves the old one running). (For plans/live-visuals G3.)
//! And the shared state `visuals`, `{ [id] = { rev, generation } }`: `rev` counts
//! changes to the tree's shape (a visual builds again only then), `generation`
//! every render (its canvases draw again). Nothing else is copied into the
//! scripts until they ask, so a visual's change touches no other's.
//!
//! What a visual shows is its tree, plain data held here; its VM is only needed
//! to run it again. So at rest the VM of one with nothing to lose goes
//! ([`Entry::sleep`]) and it starts again from its script when it's next needed
//! ([`Entry::wake`]): the same source and seed make the same tree.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Arc, Mutex, mpsc};

use serde_json::json;
use zend_visual as zv;
use zgx_script::{ScriptHost, Shared, Value};

/// Time spent drawing canvases (this thread) and rendering scene and shader
/// frames (the GPU's thread), in nanoseconds, and frames rendered: for the
/// cost probe (src/visuals_tests.rs `cost_of_each_visual`).
static DRAW_NS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static GPU_NS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static GPU_FRAMES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// The GPU thread's own CPU time (not its waits), and scene frames the
/// software renderer drew.
static GPU_CPU_NS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static SOFTWARE_FRAMES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

#[cfg(test)]
pub fn gpu_thread_costs() -> (u64, u64) {
    use std::sync::atomic::Ordering::Relaxed;
    (GPU_CPU_NS.load(Relaxed), SOFTWARE_FRAMES.load(Relaxed))
}

/// This thread's CPU time, in nanoseconds.
#[cfg(unix)]
fn thread_cpu_ns() -> u64 {
    let mut t = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: a valid timespec to write into.
    unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut t) };
    t.tv_sec as u64 * 1_000_000_000 + t.tv_nsec as u64
}

/// This thread's CPU time, in nanoseconds: its kernel and user times, which
/// Windows counts in 100-nanosecond ticks.
#[cfg(windows)]
fn thread_cpu_ns() -> u64 {
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::System::Threading::{GetCurrentThread, GetThreadTimes};
    let zero = || FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
    let (mut created, mut exited, mut kernel, mut user) = (zero(), zero(), zero(), zero());
    // SAFETY: the current thread's pseudo-handle, and four FILETIMEs to
    // write into.
    unsafe { GetThreadTimes(GetCurrentThread(), &mut created, &mut exited, &mut kernel, &mut user) };
    let ticks = |t: FILETIME| (u64::from(t.dwHighDateTime) << 32) | u64::from(t.dwLowDateTime);
    (ticks(kernel) + ticks(user)) * 100
}

#[cfg(test)]
pub fn costs() -> (u64, u64, u64) {
    use std::sync::atomic::Ordering::Relaxed;
    (DRAW_NS.load(Relaxed), GPU_NS.load(Relaxed), GPU_FRAMES.load(Relaxed))
}

/// The most a visual's files may be in all (crates/visual's own cap, and a
/// little for the manifest).
const MAX_READ: usize = zv::assets::MAX_TOTAL_ASSET_BYTES + zv::MAX_SCRIPT_BYTES + 64 * 1024;
/// A scene's or shader's frame at most this many device pixels a point
/// (the shipping app's mobile cap): sharp at 2x, never a 3x readback.
const MAX_GPU_SCALE: f32 = 2.0;
/// A visual that started within this can start again in a frame unnoticed.
/// One that took longer (it precomputes at its top level) keeps its VM.
const UNFELT_START: std::time::Duration = std::time::Duration::from_millis(8);

#[derive(Clone, Debug, PartialEq)]
enum Load {
    Loading,
    Ready,
    Failed(String),
}

/// A visual's live program and what it last showed.
struct Entry {
    title: String,
    load: Load,
    runtime: Option<zv::Visual>,
    /// What it starts from, again after a sleep.
    source: String,
    assets: zv::Assets,
    seed: u64,
    /// Its script's state may be more than its source gives: a control or
    /// the pointer reached it. Or it took too long to start again unfelt.
    keep_awake: bool,
    tree: Option<zv::Node>,
    /// The tree as the scripts get it, made once per shape.
    value: Value,
    /// The latest handler or draw failure; the last good tree stays.
    error: Option<String>,
    rev: u64,
    generation: u64,
    /// Scenes' drawings, by node path: the render they were drawn at.
    scenes: HashMap<String, (u64, Arc<zv::Scene>)>,
    /// Shaders' frame counts and last times, by node path.
    shaders: HashMap<String, (u32, f64)>,
    /// What the reader entered in its inputs: the runtime's, copied after
    /// each render, so they outlive a sleep.
    kept: zv::Entries,
    /// Its script's path and host, where its entries are saved (none for
    /// one drawn from a source).
    file: Option<(String, Option<String>)>,
    /// A save is under way; another is wanted after it.
    saving: bool,
    unsaved: bool,
    /// The `rev` of its part its script was read at (the agent's updates,
    /// `update_visual`), and the newest its part has shown: when that is
    /// newer, the script is read again and swapped in, entries kept.
    part_rev: u64,
    wanted_rev: u64,
    /// That read is under way.
    rereading: bool,
}

impl Entry {
    fn new(title: String) -> Self {
        Self {
            title,
            load: Load::Loading,
            runtime: None,
            source: String::new(),
            assets: zv::Assets::default(),
            seed: 0,
            keep_awake: false,
            tree: None,
            value: Value::Nil,
            error: None,
            rev: 0,
            generation: 0,
            scenes: HashMap::new(),
            shaders: HashMap::new(),
            kept: zv::Entries::default(),
            file: None,
            saving: false,
            unsaved: false,
            part_rev: 0,
            wanted_rev: 0,
            rereading: false,
        }
    }

    /// Run `source`'s setup and first render, seeded from `id`.
    fn start(&mut self, id: &str, source: String, assets: zv::Assets) {
        let started = std::time::Instant::now();
        let seed = zv::seed_for(id);
        match zv::Visual::load(&source, seed) {
            Ok(runtime) => {
                self.runtime = Some(runtime.with_assets(assets.clone()).with_entries(self.kept.clone()));
                (self.source, self.assets, self.seed) = (source, assets, seed);
                self.load = Load::Ready;
                self.render();
                self.keep_awake = started.elapsed() > UNFELT_START;
            }
            Err(error) => {
                eprintln!("zend-zgx: visual {id} failed to load: {error}");
                self.load = Load::Failed(error.message);
                self.rev += 1;
            }
        }
    }

    /// Run the view again. Its shape counts as changed only if the tree is
    /// another; every render is a new generation (a canvas reading state
    /// the handler changed draws again with the same tree).
    fn render(&mut self) {
        let Some(runtime) = self.runtime.as_mut() else { return };
        self.generation += 1;
        let rendered = runtime.render();
        self.kept = runtime.entries();
        match rendered {
            Ok(tree) => {
                let had_error = self.error.take().is_some();
                if self.tree.as_ref() != Some(&tree) {
                    self.value = node_value(&tree, "0");
                    self.tree = Some(tree);
                    self.rev += 1;
                } else if had_error {
                    self.rev += 1;
                }
            }
            Err(error) => self.fail(error),
        }
    }

    fn fail(&mut self, error: zv::VisualError) {
        if self.error.as_deref() != Some(error.message.as_str()) {
            self.error = Some(error.message);
            self.rev += 1;
        }
    }

    /// At rest, its VM goes if starting again gives back the same thing: its
    /// script holds nothing but what its source makes, and nothing it's
    /// drawing depends on the last draw. (A shader never runs the script.)
    fn sleep(&mut self) {
        let same_again = self.load == Load::Ready && self.error.is_none() && !self.keep_awake;
        if same_again && self.tree.as_ref().is_some_and(|tree| !draws_over_time(tree)) {
            self.runtime = None;
        }
    }

    /// Its VM, started again if it slept: from the same source and seed, so
    /// the same tree and the same handlers. Nothing it shows changes, and
    /// neither does its generation, so nothing draws again for this.
    fn wake(&mut self) -> Result<&mut zv::Visual, String> {
        if self.runtime.is_none() {
            if self.load != Load::Ready {
                return Err("not running".into());
            }
            let loaded = zv::Visual::load(&self.source, self.seed).map_err(|e| e.message)?;
            let mut runtime = loaded.with_assets(self.assets.clone()).with_entries(self.kept.clone());
            let tree = runtime.render().map_err(|e| e.message)?;
            if self.tree.as_ref() != Some(&tree) {
                self.value = node_value(&tree, "0");
                self.tree = Some(tree);
                self.rev += 1;
            }
            self.runtime = Some(runtime);
        }
        self.runtime.as_mut().ok_or_else(|| "not running".into())
    }

    /// A control's handler, then the view again.
    fn event(&mut self, handler: u32, arg: zv::HandlerArg) {
        self.keep_awake = true;
        let runtime = match self.wake() {
            Ok(runtime) => runtime,
            Err(why) => return self.fail(zv::VisualError::script(why)),
        };
        match runtime.call(zv::HandlerId(handler), arg) {
            Ok(()) => self.render(),
            Err(error) => self.fail(error),
        }
    }

    /// The reader entered `value` in input `name` (`None`: cleared it).
    fn set_entry(&mut self, name: &str, value: Option<zv::EntryValue>) {
        match self.wake() {
            Ok(runtime) => runtime.set_entry(name, value),
            Err(why) => return self.fail(zv::VisualError::script(why)),
        }
        self.render();
    }

    /// The input `name` took focus, or (`None`) focus left.
    fn focus(&mut self, name: Option<&str>) {
        if self.kept.focused() == name {
            return;
        }
        match self.wake() {
            Ok(runtime) => runtime.focus(name),
            Err(why) => return self.fail(zv::VisualError::script(why)),
        }
        self.render();
    }

    /// A new script, its entries kept; on an error the old one runs on.
    fn swap(&mut self, id: &str, source: String) -> Result<(), String> {
        if self.load != Load::Ready {
            let assets = self.assets.clone();
            self.start(id, source, assets);
            return match &self.load {
                Load::Failed(why) => Err(why.clone()),
                _ => Ok(()),
            };
        }
        self.wake()?.swap(&source).map_err(|e| e.message)?;
        self.source = source;
        // Its state is new: the old script's is gone.
        self.keep_awake = false;
        self.render();
        Ok(())
    }

    /// Its inputs as words for the composer (`ui.answer`, D8), as they
    /// show now: none before it has drawn.
    fn answer(&self) -> Option<String> {
        self.tree.as_ref().map(|tree| zv::answer_text(&self.title, tree))
    }

    fn snapshot(&self) -> Value {
        let state = match &self.load {
            Load::Loading => "loading",
            Load::Ready => "ready",
            Load::Failed(_) => "failed",
        };
        let error = match &self.load {
            Load::Failed(why) => Some(why.as_str()),
            _ => self.error.as_deref(),
        };
        let mut fields = vec![("state", Value::from(state)), ("title", Value::from(self.title.as_str()))];
        if let Some(error) = error {
            fields.push(("error", Value::from(error)));
        }
        if self.tree.is_some() {
            fields.push(("tree", self.value.clone()));
        }
        Value::record(fields)
    }

    /// A canvas's drawing.
    fn draw(&mut self, arg: &Value) -> Value {
        let n = |k: &str| arg.field(k).and_then(Value::as_f64);
        let (w, h) = (n("w").unwrap_or(0.) as f32, n("ht").unwrap_or(0.) as f32);
        let pointer = match (n("px"), n("py")) {
            (Some(x), Some(y)) => Some(zv::Pointer {
                x: x as f32,
                y: y as f32,
                down: arg.field("down").is_some_and(Value::truthy),
            }),
            _ => None,
        };
        self.keep_awake |= pointer.is_some();
        let runtime = match self.wake() {
            Ok(runtime) => runtime,
            Err(why) => return Value::record([("error", Value::from(why.as_str()))]),
        };
        let handler = zv::HandlerId(n("h").unwrap_or(-1.) as u32);
        match runtime.draw_canvas_traced(handler, w, h, n("t").unwrap_or(0.), pointer) {
            Ok(drawing) => Value::record([
                ("cmds", Value::List(drawing.commands.iter().map(command_value).collect::<Vec<_>>().into())),
                ("pointer", Value::Bool(drawing.reads_pointer)),
            ]),
            Err(error) => Value::record([("error", Value::from(error.message.as_str()))]),
        }
    }

    /// The node at `path` ("0.2.1"): its children by index.
    fn node_at(&self, path: &str) -> Option<&zv::Node> {
        let mut node = self.tree.as_ref()?;
        for step in path.split('.').skip(1) {
            node = zv::children(node).nth(step.parse().ok()?)?;
        }
        Some(node)
    }
}

/// Whether a canvas or scene in `node` draws as time passes: its script may
/// carry something from one draw to the next.
fn draws_over_time(node: &zv::Node) -> bool {
    let here = match &node.kind {
        zv::NodeKind::Canvas(canvas) => canvas.animate,
        zv::NodeKind::Scene(scene) => scene.animate,
        _ => false,
    };
    here || zv::children(node).any(draws_over_time)
}

/// Every visual this app shows, and each window's shared `visuals`.
#[derive(Default)]
struct Visuals {
    entries: RefCell<HashMap<String, Entry>>,
    /// Each shared `visuals`, with the id prefix it shows (a plugin's own:
    /// its ids without it), or none (the app's: every visual).
    shared: RefCell<Vec<(Option<String>, Shared)>>,
    /// What lets VMs go as each window goes idle.
    rests: RefCell<Vec<zgx::idle::IdleTrim>>,
}

thread_local! {
    static VISUALS: Rc<Visuals> = Rc::default();
}

impl Visuals {
    fn revs(&self, scope: Option<&str>) -> Value {
        let entries = self.entries.borrow();
        let shown = entries.iter().filter_map(|(id, e)| match scope {
            None => Some((id.as_str(), e)),
            Some(prefix) => id.strip_prefix(prefix).map(|local| (local, e)),
        });
        Value::record(shown.map(|(id, e)| {
            (
                id,
                Value::record([
                    ("rev", Value::Num(e.rev as f64)),
                    ("gen", Value::Num(e.generation as f64)),
                    // Its part says the agent updated it since it was read.
                    ("stale", Value::Bool(e.wanted_rev > e.part_rev && e.file.is_some())),
                ]),
            )
        }))
    }

    fn publish(&self) {
        let all = self.revs(None);
        for (scope, shared) in self.shared.borrow().iter() {
            match scope {
                None => {
                    shared.set(all.clone());
                }
                Some(prefix) => {
                    shared.set(self.revs(Some(prefix)));
                }
            }
        }
    }

    fn with<R>(&self, id: &str, f: impl FnOnce(&mut Entry) -> R) -> Option<R> {
        let mut entries = self.entries.borrow_mut();
        entries.get_mut(id).map(f)
    }

    fn sleep(&self) {
        self.entries.borrow_mut().values_mut().for_each(Entry::sleep);
    }
}

/// The scripts' calls, on every window but the intro's.
pub fn install(cx: &mut zgx_desktop::WindowContext, host: &ScriptHost) {
    crate::engine::install(cx);
    // `ui.window.shown`: a visual stops animating while its window is
    // hidden, minimized or covered.
    host.window_state(&cx.ui);
    register(host);
    let visuals = VISUALS.with(Rc::clone);
    let resting = visuals.clone();
    let rest = zgx::idle::on_idle(&cx.ui.runtime, move || resting.sleep());
    visuals.rests.borrow_mut().push(rest);
}

/// The calls on `host` (a test's, with no window, too).
pub fn register(host: &ScriptHost) {
    let visuals = VISUALS.with(Rc::clone);
    visuals.shared.borrow_mut().push((None, host.shared("visuals", visuals.revs(None))));
    calls(host, "visuals", &visuals, None);
}

/// Plugin `plugin`'s own visuals (zend): the same calls as
/// `plugin.<plugin>.visuals.open` and the rest, and its shared
/// `plugin.<plugin>.visuals`. Its ids are its own: each is kept here as
/// `plugin:<plugin>:<id>`, so it reaches no chat's visual and no other
/// plugin's. It runs scripts it gives (`source`) and reads no file
/// (`path`, `device` are refused).
pub fn register_plugin(host: &ScriptHost, plugin: &str) {
    let visuals = VISUALS.with(Rc::clone);
    let scope = format!("plugin:{plugin}:");
    let key = format!("plugin.{plugin}.visuals");
    visuals.shared.borrow_mut().push((Some(scope.clone()), host.shared(&key, visuals.revs(Some(&scope)))));
    calls(host, &key, &visuals, Some(scope));
}

/// The names a plugin's own visuals are called by (for exposing them).
pub const PLUGIN_CALLS: &[&str] = &["open", "get", "event", "draw", "frame", "release"];

/// `arg` with its `id` in `scope` (a record's field, or `arg` itself as text).
fn scoped(arg: Value, scope: &Option<String>) -> Value {
    let Some(scope) = scope else { return arg };
    match &arg {
        Value::Map(fields) => Value::record(fields.iter().map(|(k, v)| {
            if &**k == "id" {
                (k.clone(), Value::from(format!("{scope}{}", text(Some(v))).as_str()))
            } else {
                (k.clone(), v.clone())
            }
        })),
        _ => Value::from(format!("{scope}{}", crate::auth::text(&arg)).as_str()),
    }
}

fn calls(host: &ScriptHost, prefix: &str, visuals: &Rc<Visuals>, scope: Option<String>) {
    let name = |call: &str| format!("{prefix}.{call}");

    let (v, sc) = (visuals.clone(), scope.clone());
    host.async_action(name("open"), move |arg| {
        let v = v.clone();
        let sc = sc.clone();
        async move {
            if sc.is_some() {
                if arg.field("source").is_none() {
                    return Err("a plugin's visual is drawn from its source".into());
                }
                if arg.field("path").is_some() || arg.field("device").is_some() {
                    return Err("a plugin's visual reads no file".into());
                }
            }
            open(v, scoped(arg, &sc)).await
        }
    });
    let (v, sc) = (visuals.clone(), scope.clone());
    host.function(name("get"), move |arg| {
        let id = crate::auth::text(&scoped(arg, &sc));
        v.with(&id, |e| e.snapshot()).unwrap_or(Value::Nil)
    });
    let (v, sc) = (visuals.clone(), scope.clone());
    host.action(name("event"), move |arg| {
        let arg = scoped(arg, &sc);
        let id = text(arg.field("id"));
        let handler = arg.field("h").and_then(Value::as_f64).unwrap_or(-1.);
        if handler < 0. {
            return;
        }
        let changed = v
            .with(&id, |e| {
                let before = (e.rev, e.generation);
                e.event(handler as u32, handler_arg(arg.field("arg")));
                (e.rev, e.generation) != before
            })
            .unwrap_or(false);
        if changed {
            v.publish();
        }
    });
    let (v, sc) = (visuals.clone(), scope.clone());
    host.function(name("draw"), move |arg| {
        let arg = scoped(arg, &sc);
        let id = text(arg.field("id"));
        let start = std::time::Instant::now();
        let drawn = v.with(&id, |e| e.draw(&arg)).unwrap_or(Value::Nil);
        DRAW_NS.fetch_add(start.elapsed().as_nanos() as u64, std::sync::atomic::Ordering::Relaxed);
        drawn
    });
    let (v, sc) = (visuals.clone(), scope.clone());
    let frames_host = host.clone();
    host.async_action(name("frame"), move |arg| {
        let (v, host) = (v.clone(), frames_host.clone());
        let arg = scoped(arg, &sc);
        async move { frame(v, host, arg).await }
    });
    let (v, sc) = (visuals.clone(), scope.clone());
    host.async_action(name("input"), move |arg| {
        let v = v.clone();
        let arg = scoped(arg, &sc);
        async move {
            let (id, input) = (text(arg.field("id")), text(arg.field("name")));
            let changed = v
                .with(&id, |e| {
                    let before = e.rev;
                    e.set_entry(&input, entry_value(arg.field("value")));
                    e.rev != before
                })
                .ok_or("no visual")?;
            if changed {
                v.publish();
            }
            save_entries(&v, &id).await;
            Ok(Value::Nil)
        }
    });
    let (v, sc) = (visuals.clone(), scope.clone());
    host.action(name("focus"), move |arg| {
        let arg = scoped(arg, &sc);
        let id = text(arg.field("id"));
        let input = arg.field("name").map(crate::auth::text).filter(|n| !n.is_empty());
        let changed = v
            .with(&id, |e| {
                let before = e.rev;
                e.focus(input.as_deref());
                e.rev != before
            })
            .unwrap_or(false);
        if changed {
            v.publish();
        }
    });
    let (v, sc) = (visuals.clone(), scope.clone());
    host.function(name("entries"), move |arg| {
        let id = crate::auth::text(&scoped(arg, &sc));
        v.with(&id, |e| entries_value(&e.kept)).unwrap_or(Value::Nil)
    });
    let (v, sc) = (visuals.clone(), scope.clone());
    host.function(name("answer"), move |arg| {
        let id = crate::auth::text(&scoped(arg, &sc));
        v.with(&id, |e| e.answer()).flatten().map_or(Value::Nil, |t| Value::from(t.as_str()))
    });
    let (v, sc) = (visuals.clone(), scope.clone());
    host.async_action(name("swap"), move |arg| {
        let v = v.clone();
        let arg = scoped(arg, &sc);
        async move {
            let id = text(arg.field("id"));
            let source = arg.field("source").map(crate::auth::text).ok_or("no source")?;
            v.with(&id, |e| e.swap(&id, source)).ok_or("no visual")??;
            v.publish();
            Ok(Value::Nil)
        }
    });
    let (v, sc) = (visuals.clone(), scope);
    host.action(name("release"), move |arg| {
        let arg = scoped(arg, &sc);
        let (id, path) = (text(arg.field("id")), text(arg.field("p")));
        v.with(&id, |e| {
            e.scenes.remove(&path);
            e.shaders.remove(&path);
        });
        gpu_send(Job::Release { key: surface_key(&id, &path) });
    });
}

/// (For tests: how many frames each shader of visual `id` has drawn, and
/// which scenes have a drawing, by node path.)
#[cfg(test)]
pub fn surfaces(id: &str) -> (Vec<(String, u32)>, Vec<String>) {
    VISUALS.with(|v| {
        v.with(id, |e| {
            (
                e.shaders.iter().map(|(p, (n, _))| (p.clone(), *n)).collect(),
                e.scenes.keys().cloned().collect(),
            )
        })
        .unwrap_or_default()
    })
}

/// (For tests: visual `id` started from `source`, as a part naming it
/// would find it already read.)
#[cfg(test)]
pub fn preload(id: &str, title: &str, source: &str) {
    VISUALS.with(|v| {
        let mut e = Entry::new(title.into());
        e.start(id, source.into(), zv::Assets::default());
        v.entries.borrow_mut().insert(id.into(), e);
        v.publish();
    });
}

/// (For tests: what a window going idle does here.)
#[cfg(test)]
pub fn rest() {
    VISUALS.with(|v| v.sleep());
}

/// (For tests: whether visual `id` has its VM.)
#[cfg(test)]
pub fn awake(id: &str) -> bool {
    VISUALS.with(|v| v.with(id, |e| e.runtime.is_some()).unwrap_or(false))
}

/// (For tests: visual `id` as the scripts get it.)
#[cfg(test)]
pub fn snapshot(id: &str) -> Value {
    VISUALS.with(|v| v.with(id, |e| e.snapshot()).unwrap_or(Value::Nil))
}

/// (For tests: its rev and generation, which say what the scripts rebuild.)
#[cfg(test)]
pub fn versions(id: &str) -> (u64, u64) {
    VISUALS.with(|v| v.with(id, |e| (e.rev, e.generation)).unwrap_or_default())
}

/// (For tests: a control's handler, as `visuals.event` runs it.)
#[cfg(test)]
/// (For tests: what the reader entered in visual `id`.)
pub fn entries_of(id: &str) -> Value {
    VISUALS.with(|v| v.with(id, |e| entries_value(&e.kept))).unwrap_or(Value::Nil)
}

/// Visual `id`'s part shows update `rev` (chat.rs, as it builds the
/// block): one already read at an older one is marked stale, and its
/// component asks `visuals.open` to read it again.
pub fn seen_rev(id: &str, rev: u64) {
    VISUALS.with(|v| {
        let stale = v
            .with(id, |e| {
                let newer = rev > e.wanted_rev;
                e.wanted_rev = e.wanted_rev.max(rev);
                newer && e.wanted_rev > e.part_rev
            })
            .unwrap_or(false);
        if stale {
            v.publish();
        }
    });
}

/// `visuals.open` for one read already whose part was updated: read its
/// script again and swap it in, what the reader entered kept (the input
/// they're in left as it is). New files the update brought show as it's
/// opened again.
async fn reread(v: Rc<Visuals>, id: String) -> Result<Value, String> {
    let Some((want, file)) = v
        .with(&id, |e| {
            if e.rereading || e.wanted_rev <= e.part_rev {
                return None;
            }
            e.rereading = true;
            Some((e.wanted_rev, e.file.clone()?))
        })
        .flatten()
    else {
        return Ok(Value::Nil);
    };
    let (path, device) = file;
    let read = read_visual(&path, device).await;
    v.with(&id, |e| {
        e.rereading = false;
        match read {
            Ok((source, _)) => {
                e.part_rev = want;
                if let Err(why) = e.swap(&id, source) {
                    eprintln!("zend-zgx: visual {id}'s update failed: {why}");
                }
            }
            Err(why) => {
                // Asked again the next time its part changes.
                e.part_rev = want;
                eprintln!("zend-zgx: visual {id}'s update unavailable: {why}");
            }
        }
    });
    v.publish();
    Ok(Value::Nil)
}

/// (For tests: a new script for visual `id`, as `visuals.swap` gives one.)
pub fn swap_source(id: &str, source: &str) -> Result<(), String> {
    VISUALS.with(|v| {
        let swapped = v.with(id, |e| e.swap(id, source.into())).ok_or("no visual")?;
        v.publish();
        swapped
    })
}

pub fn event(id: &str, handler: u32) {
    VISUALS.with(|v| v.with(id, |e| e.event(handler, zv::HandlerArg::None)));
}

/// (For tests: a canvas's drawing, as `visuals.draw` makes it.)
#[cfg(test)]
pub fn draw(id: &str, arg: Value) -> Value {
    VISUALS.with(|v| v.with(id, |e| e.draw(&arg)).unwrap_or(Value::Nil))
}

fn text(value: Option<&Value>) -> String {
    value.map(crate::auth::text).unwrap_or_default()
}

/// `visuals.open`: read the script and its files, then start it.
async fn open(v: Rc<Visuals>, arg: Value) -> Result<Value, String> {
    let id = text(arg.field("id"));
    if id.is_empty() {
        return Err("no visual".into());
    }
    let title = text(arg.field("title"));
    let part_rev = arg.field("rev").and_then(Value::as_f64).unwrap_or(0.).max(0.) as u64;
    {
        let mut entries = v.entries.borrow_mut();
        match entries.get_mut(&id) {
            // Read, or being read: once, unless the agent updated it since.
            Some(e) if e.load == Load::Ready && e.wanted_rev.max(part_rev) > e.part_rev => {
                e.wanted_rev = e.wanted_rev.max(part_rev);
                drop(entries);
                return reread(v, id).await;
            }
            Some(e) if matches!(e.load, Load::Loading | Load::Ready) => {
                e.wanted_rev = e.wanted_rev.max(part_rev);
                return Ok(Value::Nil);
            }
            // (One that failed is read again, as the chat opens again.)
            // (One that failed is read again, as the chat opens again.)
            _ => {
                let mut entry = Entry::new(title);
                (entry.part_rev, entry.wanted_rev) = (part_rev, part_rev);
                entries.insert(id.clone(), entry);
            }
        }
    }
    v.publish();
    let (path, device) = (text(arg.field("path")), arg.field("device").map(crate::auth::text));
    let device = device.filter(|d| !d.is_empty());
    let loaded = match arg.field("source") {
        Some(source) => Ok((crate::auth::text(source), zv::Assets::default(), None)),
        None => read_visual(&path, device.clone()).await.map(|(source, assets)| (source, assets, Some(path))),
    };
    // What the reader entered before, kept beside the script.
    let kept = match loaded.as_ref().ok().and_then(|(_, _, path)| path.as_deref()) {
        Some(path) => {
            let stem = path.trim_end_matches("-visual.luau");
            match read_file(&format!("{stem}-entries.json"), device.as_deref()).await {
                Ok(bytes) => zv::Entries::from_json(&String::from_utf8_lossy(&bytes)),
                Err(_) => zv::Entries::default(),
            }
        }
        None => zv::Entries::default(),
    };
    v.with(&id, |e| match loaded {
        Ok((source, assets, path)) => {
            e.kept = kept;
            e.file = path.map(|path| (path, device));
            e.start(&id, source, assets)
        }
        Err(why) => {
            eprintln!("zend-zgx: visual {id} unavailable: {why}");
            e.load = Load::Failed(why);
            e.rev += 1;
        }
    });
    v.publish();
    Ok(Value::Nil)
}

/// A visual's script and its files (listed in `{stem}-assets.json` beside
/// it, absent for one without), as the shipping app reads them.
async fn read_visual(path: &str, device: Option<String>) -> Result<(String, zv::Assets), String> {
    if !path.ends_with("-visual.luau") {
        return Err("That isn't a visual's script.".into());
    }
    let device = device.filter(|d| !d.is_empty());
    let bytes = read_file(path, device.as_deref()).await?;
    let source = String::from_utf8(bytes).map_err(|_| "The visual's script isn't text.".to_string())?;
    let mut files = HashMap::new();
    let stem = path.trim_end_matches("-visual.luau");
    if let Ok(manifest) = read_file(&format!("{stem}-assets.json"), device.as_deref()).await
        && let Ok(names) = serde_json::from_slice::<Vec<String>>(&manifest)
    {
        for name in names.into_iter().filter(|n| zv::assets::valid_name(n)).take(zv::assets::MAX_ASSETS) {
            if let Ok(bytes) = read_file(&format!("{stem}-asset-{name}"), device.as_deref()).await {
                files.insert(name, Arc::<[u8]>::from(bytes));
            }
        }
    }
    Ok((source, zv::Assets::new(files)))
}

/// A file the engine serves as an attachment (the uploads, where visuals
/// are kept), a chunk at a time, from `device` when it isn't this one.
async fn read_file(path: &str, device: Option<&str>) -> Result<Vec<u8>, String> {
    use base64::Engine as _;
    let mut bytes = Vec::new();
    let mut offset = 0u64;
    loop {
        let mut params = json!({ "path": path, "offset": offset });
        if let Some(device) = device {
            params["targetDeviceId"] = json!(device);
        }
        let chunk = crate::engine::get().call("ReadAttachmentChunk", params).await?;
        let data = base64::engine::general_purpose::STANDARD
            .decode(chunk["data"].as_str().unwrap_or(""))
            .map_err(|_| "The visual came back garbled.".to_string())?;
        bytes.extend_from_slice(&data);
        if bytes.len() > MAX_READ {
            return Err("The visual is too large.".into());
        }
        if chunk["done"].as_bool().unwrap_or(false) {
            return Ok(bytes);
        }
        let next = chunk["nextOffset"].as_u64().unwrap_or(0);
        if next <= offset {
            return Err("The visual changed while it was read.".into());
        }
        offset = next;
    }
}

/// Save visual `id`'s entries beside its script, one save at a time: one
/// asked for while another is under way follows it, with what's kept then.
async fn save_entries(v: &Rc<Visuals>, id: &str) {
    let busy = v.with(id, |e| {
        let busy = e.saving;
        e.unsaved = true;
        e.saving = e.file.is_some();
        busy || e.file.is_none()
    });
    if busy != Some(false) {
        return;
    }
    loop {
        let Some(Some((params, path))) = v.with(id, |e| {
            e.unsaved = false;
            let (path, device) = e.file.clone()?;
            let entries = serde_json::from_str::<serde_json::Value>(&e.kept.to_json()).ok()?;
            let mut params = json!({ "path": path, "entries": entries });
            if let Some(device) = device {
                params["targetDeviceId"] = json!(device);
            }
            Some((params, path))
        }) else {
            break;
        };
        if let Err(why) = crate::engine::get().call("SaveVisualEntries", params).await {
            eprintln!("zend-zgx: entries of visual {id} not saved beside {path}: {why}");
        }
        if v.with(id, |e| e.unsaved) != Some(true) {
            break;
        }
    }
    v.with(id, |e| e.saving = false);
}

/// An entry from what the script sent: a string, a boolean, a list of
/// strings; nil (or nothing) clears it.
fn entry_value(value: Option<&Value>) -> Option<zv::EntryValue> {
    match value? {
        Value::Nil => None,
        Value::Bool(b) => Some(zv::EntryValue::Bool(*b)),
        list @ (Value::List(_) | Value::Map(_)) => {
            Some(zv::EntryValue::List(list.items().iter().map(crate::auth::text).collect()))
        }
        other => Some(zv::EntryValue::Text(crate::auth::text(other))),
    }
}

fn entry_data(value: &zv::EntryValue) -> Value {
    match value {
        zv::EntryValue::Text(t) => Value::from(t.as_str()),
        zv::EntryValue::Bool(b) => Value::Bool(*b),
        zv::EntryValue::List(items) => list(items.iter().map(|i| Value::from(i.as_str())).collect()),
    }
}

fn entries_value(entries: &zv::Entries) -> Value {
    Value::record(entries.values().iter().map(|(name, value)| (name.as_str(), entry_data(value))))
}

/// A handler's argument from what the script sent.
fn handler_arg(value: Option<&Value>) -> zv::HandlerArg {
    match value {
        None | Some(Value::Nil) => zv::HandlerArg::None,
        Some(Value::Bool(b)) => zv::HandlerArg::Bool(*b),
        Some(Value::Num(n)) => zv::HandlerArg::Number(*n),
        Some(point @ Value::Map(_)) => {
            let n = |k: &str| point.field(k).and_then(Value::as_f64).unwrap_or(0.) as f32;
            zv::HandlerArg::Point(n("x"), n("y"))
        }
        Some(other) => zv::HandlerArg::Text(crate::auth::text(other)),
    }
}

// ---- the tree as data -------------------------------------------------------

fn num(n: impl Into<f64>) -> Value {
    Value::Num(n.into())
}

fn list(items: Vec<Value>) -> Value {
    Value::List(items.into())
}

fn token_name(token: zv::Token) -> String {
    match token {
        zv::Token::Text => "text".into(),
        zv::Token::Muted => "muted".into(),
        zv::Token::Faint => "faint".into(),
        zv::Token::Accent => "accent".into(),
        zv::Token::OnAccent => "on-accent".into(),
        zv::Token::Success => "success".into(),
        zv::Token::Warning => "warning".into(),
        zv::Token::Danger => "danger".into(),
        zv::Token::Border => "border".into(),
        zv::Token::Chart(i) => format!("chart{}", i + 1),
        zv::Token::Rgb(rgb) => format!("#{rgb:06x}"),
    }
}

/// A plain token as its name; one mixed or faded as `{ token, mix?, t?, alpha }`.
fn color_value(color: zv::Color) -> Value {
    if color.mix.is_none() && color.alpha >= 1.0 {
        return Value::from(token_name(color.token).as_str());
    }
    let mut fields = vec![("token", Value::from(token_name(color.token).as_str())), ("alpha", num(color.alpha))];
    if let Some((other, t)) = color.mix {
        fields.push(("mix", Value::from(token_name(other).as_str())));
        fields.push(("t", num(t)));
    }
    Value::record(fields)
}

fn align_name(align: zv::Align) -> &'static str {
    match align {
        zv::Align::Start => "start",
        zv::Align::Center => "center",
        zv::Align::End => "end",
    }
}

fn size_name(size: zv::TextSize) -> &'static str {
    match size {
        zv::TextSize::Xs => "xs",
        zv::TextSize::Sm => "sm",
        zv::TextSize::Body => "body",
        zv::TextSize::Lg => "lg",
        zv::TextSize::Xl => "xl",
        zv::TextSize::Xxl => "2xl",
    }
}

fn weight_name(weight: zv::Weight) -> &'static str {
    match weight {
        zv::Weight::Regular => "regular",
        zv::Weight::Medium => "medium",
        zv::Weight::Semibold => "semibold",
        zv::Weight::Bold => "bold",
    }
}

fn format_value(f: &zv::NumberFormat) -> Value {
    let mut fields = vec![("prefix", Value::from(f.prefix.as_str())), ("unit", Value::from(f.unit.as_str()))];
    if let Some(d) = f.decimals {
        fields.push(("decimals", num(d)));
    }
    Value::record(fields)
}

fn handler(id: zv::HandlerId) -> Value {
    num(id.0)
}

fn children_value(children: &[zv::Node], path: &str) -> Value {
    list(children.iter().enumerate().map(|(i, c)| node_value(c, &format!("{path}.{i}"))).collect())
}

/// A node as the scripts get it: `k` its kind, `p` its path in the tree
/// (children by index, a table's cells row by row), and its own fields.
fn node_value(node: &zv::Node, path: &str) -> Value {
    let mut f: Vec<(&str, Value)> = vec![("p", Value::from(path))];
    if let Some(w) = node.width {
        f.push(("w", num(w)));
    }
    if node.grow {
        f.push(("grow", Value::Bool(true)));
    }
    if let Some(key) = &node.key {
        f.push(("key", Value::from(key.as_str())));
    }
    match &node.kind {
        zv::NodeKind::Stack(s) => {
            let kind = if s.axis == zv::Axis::Horizontal { "row" } else { "column" };
            f.push(("k", Value::from(kind)));
            f.push(("gap", num(s.gap)));
            f.push(("align", Value::from(align_name(s.align))));
            f.push((
                "justify",
                Value::from(match s.justify {
                    zv::Justify::Start => "start",
                    zv::Justify::Center => "center",
                    zv::Justify::End => "end",
                    zv::Justify::Between => "between",
                }),
            ));
            f.push(("wrap", Value::Bool(s.wrap)));
            f.push(("hover", Value::Bool(s.hover)));
            if let Some(click) = s.on_click {
                f.push(("click", handler(click)));
            }
            f.push(("children", children_value(&s.children, path)));
        }
        zv::NodeKind::Grid { columns, gap, children } => {
            f.push(("k", Value::from("grid")));
            f.push(("columns", num(*columns)));
            f.push(("gap", num(*gap)));
            f.push(("children", children_value(children, path)));
        }
        zv::NodeKind::Text(t) => {
            f.push(("k", Value::from("text")));
            f.push(("spans", list(t.spans.iter().map(span_value).collect())));
            f.push(("size", Value::from(size_name(t.size))));
            f.push(("weight", Value::from(weight_name(t.weight))));
            if let Some(color) = t.color {
                f.push(("color", color_value(color)));
            }
            f.push(("mono", Value::Bool(t.mono)));
            f.push(("italic", Value::Bool(t.italic)));
            f.push(("align", Value::from(align_name(t.align))));
            f.push(("truncate", Value::Bool(t.truncate)));
        }
        zv::NodeKind::Rule => f.push(("k", Value::from("rule"))),
        zv::NodeKind::Space(size) => {
            f.push(("k", Value::from("space")));
            f.push(("size", num(*size)));
        }
        zv::NodeKind::Stat { value, label, delta, tone } => {
            f.push(("k", Value::from("stat")));
            f.push(("value", Value::from(value.as_str())));
            f.push(("label", Value::from(label.as_str())));
            if let Some(delta) = delta {
                f.push(("delta", Value::from(delta.as_str())));
            }
            if let Some(tone) = tone {
                f.push(("tone", color_value(*tone)));
            }
        }
        zv::NodeKind::Table(t) => {
            f.push(("k", Value::from("table")));
            f.push((
                "columns",
                list(
                    t.columns
                        .iter()
                        .map(|c| {
                            let mut col = vec![("label", Value::from(c.label.as_str())), ("align", Value::from(align_name(c.align)))];
                            if let Some(w) = c.width {
                                col.push(("w", num(w)));
                            }
                            Value::record(col)
                        })
                        .collect(),
                ),
            ));
            // Cells number on from the row before's (`zv::children` reads a
            // table's cells row by row).
            let mut at = 0usize;
            let rows = t
                .rows
                .iter()
                .map(|row| {
                    list(
                        row.iter()
                            .map(|cell| {
                                let v = node_value(cell, &format!("{path}.{at}"));
                                at += 1;
                                v
                            })
                            .collect(),
                    )
                })
                .collect();
            f.push(("rows", list(rows)));
            f.push(("hover", Value::Bool(t.hover)));
        }
        zv::NodeKind::Chart(c) => {
            f.push(("k", Value::from("chart")));
            chart_fields(c, &mut f);
        }
        zv::NodeKind::Canvas(c) => {
            f.push(("k", Value::from("canvas")));
            f.push(("h", num(c.height)));
            f.push(("draw", handler(c.draw)));
            f.push(("animate", Value::Bool(c.animate)));
            if let Some(click) = c.on_click {
                f.push(("click", handler(click)));
            }
        }
        zv::NodeKind::Scene(s) => {
            f.push(("k", Value::from("scene")));
            f.push(("h", num(s.height)));
            f.push(("animate", Value::Bool(s.animate)));
            f.push(("orbit", Value::Bool(s.orbit)));
            f.push(("spin", num(s.spin)));
            f.push(("yaw", num(s.yaw)));
            f.push(("pitch", num(s.pitch)));
            f.push(("zoom", num(s.zoom)));
        }
        zv::NodeKind::Shader(s) => {
            f.push(("k", Value::from("shader")));
            f.push(("h", num(s.height)));
            // (Buffers feed back frame to frame: it runs on.)
            f.push(("animate", Value::Bool(s.animate || !s.buffers.is_empty())));
            f.push(("orbit", Value::Bool(s.orbit)));
            f.push(("spin", num(s.spin)));
            f.push(("yaw", num(s.yaw)));
            f.push(("pitch", num(s.pitch)));
            f.push(("zoom", num(s.zoom)));
        }
        zv::NodeKind::Button { label, on_click, primary } => {
            f.push(("k", Value::from("button")));
            f.push(("label", Value::from(label.as_str())));
            f.push(("click", handler(*on_click)));
            f.push(("primary", Value::Bool(*primary)));
        }
        zv::NodeKind::Tabs { options, value, on_change } => {
            f.push(("k", Value::from("tabs")));
            f.push(("options", list(options.iter().map(|o| Value::from(o.as_str())).collect())));
            f.push(("value", Value::from(value.as_str())));
            f.push(("change", handler(*on_change)));
        }
        zv::NodeKind::Toggle { label, value, on_change } => {
            f.push(("k", Value::from("toggle")));
            f.push(("label", Value::from(label.as_str())));
            f.push(("value", Value::Bool(*value)));
            f.push(("change", handler(*on_change)));
        }
        zv::NodeKind::Slider { label, value, min, max, step, on_change, format } => {
            f.push(("k", Value::from("slider")));
            if let Some(label) = label {
                f.push(("label", Value::from(label.as_str())));
            }
            f.push(("value", num(*value)));
            f.push(("min", num(*min)));
            f.push(("max", num(*max)));
            if *step > 0. {
                f.push(("step", num(*step)));
            }
            f.push(("change", handler(*on_change)));
            f.push(("fmt", format_value(format)));
        }
        zv::NodeKind::Answer { label } => {
            f.push(("k", Value::from("answer")));
            if let Some(label) = label {
                f.push(("label", Value::from(label.as_str())));
            }
        }
        zv::NodeKind::Link { label, url } => {
            f.push(("k", Value::from("link")));
            f.push(("label", Value::from(label.as_str())));
            f.push(("url", Value::from(url.as_str())));
        }
        zv::NodeKind::Input(input) => {
            let (kind, many) = match input.kind {
                zv::InputKind::Field => ("field", false),
                zv::InputKind::Area => ("area", false),
                zv::InputKind::Check => ("check", false),
                zv::InputKind::Checklist => ("checklist", true),
                zv::InputKind::Choice { many } => ("choice", many),
                zv::InputKind::Select => ("select", false),
            };
            f.push(("k", Value::from("input")));
            f.push(("kind", Value::from(kind)));
            f.push(("many", Value::Bool(many)));
            f.push(("name", Value::from(input.name.as_str())));
            // Its control is the same one whatever moves around it.
            if node.key.is_none() {
                f.push(("key", Value::from(format!("input:{}", input.name).as_str())));
            }
            for (field, text) in [
                ("label", &input.label),
                ("description", &input.description),
                ("placeholder", &input.placeholder),
                ("answered", &input.answered),
            ] {
                if let Some(text) = text {
                    f.push((field, Value::from(text.as_str())));
                }
            }
            if !input.options.is_empty() {
                f.push((
                    "options",
                    list(
                        input
                            .options
                            .iter()
                            .map(|o| {
                                let mut of = vec![
                                    ("value", Value::from(o.value.as_str())),
                                    ("label", Value::from(o.label.as_str())),
                                ];
                                if let Some(d) = &o.description {
                                    of.push(("description", Value::from(d.as_str())));
                                }
                                Value::record(of)
                            })
                            .collect(),
                    ),
                ));
            }
            f.push(("value", entry_data(&input.value)));
            f.push(("kept", Value::Bool(input.kept)));
            f.push(("fresh", Value::Bool(input.fresh)));
        }
    }
    Value::record(f)
}

fn span_value(span: &zv::Span) -> Value {
    let mut f = vec![("t", Value::from(span.text.as_str()))];
    if let Some(weight) = span.weight {
        f.push(("weight", Value::from(weight_name(weight))));
    }
    if let Some(color) = span.color {
        f.push(("color", color_value(color)));
    }
    if span.mono {
        f.push(("mono", Value::Bool(true)));
    }
    if span.italic {
        f.push(("italic", Value::Bool(true)));
    }
    Value::record(f)
}

/// An axis over `lo..hi`: round ticks (crates/visual's `nice_ticks`), the
/// range widened to the outer ones unless the script fixed that end, and
/// each tick's label in the chart's format.
fn axis(mut lo: f64, mut hi: f64, fixed: (Option<f64>, Option<f64>), format: &zv::NumberFormat) -> Value {
    if let Some(min) = fixed.0 {
        lo = min;
    }
    if let Some(max) = fixed.1 {
        hi = max;
    }
    if !lo.is_finite() || !hi.is_finite() {
        (lo, hi) = (0., 1.);
    }
    if (hi - lo).abs() < f64::EPSILON {
        let pad = if lo == 0. { 1. } else { lo.abs() * 0.1 };
        (lo, hi) = (lo - pad, hi + pad);
    }
    let mut ticks: Vec<f64> = zv::nice_ticks(lo, hi, 4);
    // The data's ends inside round ticks: one more step past an end the
    // ticks stop short of (a 190 peak under a 200 line, not over 150).
    if ticks.len() >= 2 {
        let step = ticks[1] - ticks[0];
        if fixed.1.is_none() && ticks[ticks.len() - 1] < hi - step * 1e-6 {
            ticks.push(ticks[ticks.len() - 1] + step);
        }
        if fixed.0.is_none() && ticks[0] > lo + step * 1e-6 {
            ticks.insert(0, ticks[0] - step);
        }
    }
    if fixed.0.is_none()
        && let Some(first) = ticks.first()
    {
        lo = lo.min(*first);
    }
    if fixed.1.is_none()
        && let Some(last) = ticks.last()
    {
        hi = hi.max(*last);
    }
    let shown: Vec<f64> = ticks.into_iter().filter(|t| *t >= lo - 1e-9 && *t <= hi + 1e-9).collect();
    Value::record([
        ("lo", num(lo)),
        ("hi", num(hi)),
        ("ticks", list(shown.iter().map(|t| num(*t)).collect())),
        ("labels", list(shown.iter().map(|t| Value::from(format.format(*t).as_str())).collect())),
    ])
}

fn chart_fields(c: &zv::Chart, f: &mut Vec<(&str, Value)>) {
    let kind = match c.kind {
        zv::ChartKind::Bar => "bar",
        zv::ChartKind::Line => "line",
        zv::ChartKind::Area => "area",
        zv::ChartKind::Scatter => "scatter",
    };
    f.push(("type", Value::from(kind)));
    f.push(("h", num(c.height)));
    f.push(("x", list(c.x.iter().map(|x| Value::from(x.as_str())).collect())));
    f.push(("stacked", Value::Bool(c.stacked)));
    f.push(("horizontal", Value::Bool(c.horizontal && c.kind == zv::ChartKind::Bar)));
    f.push(("legend", Value::Bool(c.legend)));
    f.push(("fmt", format_value(&c.format)));
    f.push(("xfmt", format_value(&c.x_format)));
    f.push((
        "series",
        list(
            c.series
                .iter()
                .map(|s| {
                    let mut points = Vec::with_capacity(s.points.len() * 2);
                    for (x, y) in &s.points {
                        points.push(num(*x));
                        points.push(num(*y));
                    }
                    Value::record([
                        ("name", Value::from(s.name.as_str())),
                        ("color", color_value(s.color)),
                        // (A gap is `false`: a list can't hold nil.)
                        ("values", list(s.values.iter().map(|v| v.map_or(Value::Bool(false), num)).collect())),
                        ("points", list(points)),
                    ])
                })
                .collect(),
        ),
    ));
    // The value axis: from the values (stacked, each category's sums
    // above and below zero), bars and areas from zero.
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    if c.kind == zv::ChartKind::Scatter {
        let (mut xlo, mut xhi) = (f64::INFINITY, f64::NEG_INFINITY);
        for s in &c.series {
            for (x, y) in &s.points {
                (xlo, xhi) = (xlo.min(*x), xhi.max(*x));
                (lo, hi) = (lo.min(*y), hi.max(*y));
            }
        }
        // (Points clear of the plot's edges.)
        let pad = |lo: f64, hi: f64| {
            let d = if (hi - lo).abs() < f64::EPSILON { 1. } else { (hi - lo) * 0.05 };
            (lo - d, hi + d)
        };
        let (xlo, xhi) = pad(xlo, xhi);
        (lo, hi) = pad(lo, hi);
        f.push(("xaxis", axis(xlo, xhi, (None, None), &c.x_format)));
    } else if c.stacked {
        let longest = c.series.iter().map(|s| s.values.len()).max().unwrap_or(0);
        for i in 0..longest {
            let (mut up, mut down) = (0f64, 0f64);
            for s in &c.series {
                match s.values.get(i).copied().flatten() {
                    Some(v) if v >= 0. => up += v,
                    Some(v) => down += v,
                    None => {}
                }
            }
            (lo, hi) = (lo.min(down), hi.max(up));
        }
    } else {
        for v in c.series.iter().flat_map(|s| s.values.iter().flatten()) {
            (lo, hi) = (lo.min(*v), hi.max(*v));
        }
    }
    if matches!(c.kind, zv::ChartKind::Bar | zv::ChartKind::Area) {
        (lo, hi) = (lo.min(0.), hi.max(0.));
    }
    f.push(("yaxis", axis(lo, hi, (c.y_min, c.y_max), &c.format)));
}

fn points_value(points: &[[f32; 2]]) -> Value {
    let mut out = Vec::with_capacity(points.len() * 2);
    for [x, y] in points {
        out.push(num(*x));
        out.push(num(*y));
    }
    list(out)
}

/// A canvas command: a stroke (`s`), a fill (`f`) or a line of text (`t`);
/// and the shapes zgx's canvas draws on the GPU as they are, with no path
/// to build or rasterize: a circle (`c`), a rectangle (`r`), a line (`l`).
fn command_value(cmd: &zv::DrawCmd) -> Value {
    match cmd {
        zv::DrawCmd::Fill { points, color } if let Some((x, y, r)) = zend_zgx_pixels::circle_of(points) => Value::record([
            ("k", Value::from("c")),
            ("x", num(x)),
            ("y", num(y)),
            ("r", num(r)),
            ("c", color_value(*color)),
        ]),
        zv::DrawCmd::Fill { points, color } if let Some((x, y, w, h)) = zend_zgx_pixels::rect_of(points) => Value::record([
            ("k", Value::from("r")),
            ("x", num(x)),
            ("y", num(y)),
            ("w", num(w)),
            ("h", num(h)),
            ("c", color_value(*color)),
        ]),
        zv::DrawCmd::Stroke { points, closed: false, color, width, dash } if points.len() == 2 => {
            let mut f = vec![
                ("k", Value::from("l")),
                ("pts", points_value(points)),
                ("c", color_value(*color)),
                ("w", num(*width)),
            ];
            if let Some(dash) = dash {
                f.push(("dash", num(*dash)));
            }
            Value::record(f)
        }
        // A long solid stroke that turns back (a curve, a spiral): its runs
        // that don't (`m`), each a stroke the canvas draws on the GPU.
        zv::DrawCmd::Stroke { points, closed, color, width, dash: None } if points.len() >= 8 => {
            let mut all = points.clone();
            if *closed {
                all.push(points[0]);
            }
            let runs = zend_zgx_pixels::monotone_runs(&all);
            Value::record([
                ("k", Value::from("m")),
                ("runs", list(runs.iter().map(|&(a, b)| points_value(&all[a..=b])).collect())),
                ("c", color_value(*color)),
                ("w", num(*width)),
            ])
        }
        zv::DrawCmd::Stroke { points, closed, color, width, dash } => {
            let mut f = vec![
                ("k", Value::from("s")),
                ("pts", points_value(points)),
                ("closed", Value::Bool(*closed)),
                ("c", color_value(*color)),
                ("w", num(*width)),
            ];
            if let Some(dash) = dash {
                f.push(("dash", num(*dash)));
            }
            Value::record(f)
        }
        zv::DrawCmd::Fill { points, color } => {
            Value::record([("k", Value::from("f")), ("pts", points_value(points)), ("c", color_value(*color))])
        }
        zv::DrawCmd::Text(t) => Value::record([
            ("k", Value::from("t")),
            ("x", num(t.x)),
            ("y", num(t.y)),
            ("t", Value::from(t.text.as_str())),
            ("size", Value::from(size_name(t.size))),
            ("weight", Value::from(weight_name(t.weight))),
            ("c", color_value(t.color)),
            ("mono", Value::Bool(t.mono)),
            ("align", Value::from(align_name(t.align))),
            ("base", Value::from(align_name(t.baseline))),
        ]),
    }
}

// ---- scenes and shaders on the GPU ---------------------------------------------

/// The theme's colours as the scene renderer takes them: each token's
/// straight RGBA, from the `#rrggbbaa` the script resolved.
#[derive(Clone, Copy)]
struct Palette {
    tokens: [[f32; 4]; 9],
    chart: [[f32; 4]; 6],
}

fn hex(text: &str) -> Option<[f32; 4]> {
    let h = text.strip_prefix('#')?;
    let byte = |i: usize| u8::from_str_radix(h.get(i..i + 2)?, 16).ok().map(|b| b as f32 / 255.);
    match h.len() {
        6 => Some([byte(0)?, byte(2)?, byte(4)?, 1.]),
        8 => Some([byte(0)?, byte(2)?, byte(4)?, byte(6)?]),
        _ => None,
    }
}

impl Palette {
    fn from(value: Option<&Value>) -> Self {
        let get = |name: &str, fallback: [f32; 4]| {
            value
                .and_then(|v| v.field(name))
                .and_then(|v| hex(&crate::auth::text(v)))
                .unwrap_or(fallback)
        };
        let grey = [0.5, 0.5, 0.5, 1.];
        let names = ["text", "muted", "faint", "accent", "on-accent", "success", "warning", "danger", "border"];
        let mut tokens = [grey; 9];
        for (slot, name) in names.iter().enumerate() {
            tokens[slot] = get(name, grey);
        }
        let mut chart = [grey; 6];
        for (i, slot) in chart.iter_mut().enumerate() {
            *slot = get(&format!("chart{}", i + 1), tokens[3]);
        }
        Self { tokens, chart }
    }

    fn token(&self, token: zv::Token) -> [f32; 4] {
        match token {
            zv::Token::Text => self.tokens[0],
            zv::Token::Muted => self.tokens[1],
            zv::Token::Faint => self.tokens[2],
            zv::Token::Accent => self.tokens[3],
            zv::Token::OnAccent => self.tokens[4],
            zv::Token::Success => self.tokens[5],
            zv::Token::Warning => self.tokens[6],
            zv::Token::Danger => self.tokens[7],
            zv::Token::Border => self.tokens[8],
            zv::Token::Chart(i) => self.chart[usize::from(i) % 6],
            zv::Token::Rgb(rgb) => {
                let c = |shift: u32| ((rgb >> shift) & 0xff) as f32 / 255.;
                [c(16), c(8), c(0), 1.]
            }
        }
    }

    fn color(&self, color: zv::Color) -> [f32; 4] {
        let mut out = self.token(color.token);
        if let Some((other, t)) = color.mix {
            let b = self.token(other);
            for i in 0..4 {
                out[i] += (b[i] - out[i]) * t;
            }
        }
        out[3] *= color.alpha;
        out
    }
}

/// What a scene or shader frame is drawn from, sent to the GPU's thread.
enum Job {
    Scene {
        key: String,
        scene: Arc<zv::Scene>,
        camera: zv::Camera,
        size: (f32, f32),
        scale: f32,
        palette: Palette,
        reply: zgx::task::LatestSender<Result<Rendered, String>>,
    },
    Shader {
        key: String,
        node: Box<zv::ShaderNode>,
        assets: zv::Assets,
        uniforms: zv::shader::ShaderUniforms,
        size: (u32, u32),
        reply: zgx::task::LatestSender<Result<Rendered, String>>,
    },
    Release {
        key: String,
    },
    /// Its last user has gone: it leaves, unless a thread has sent it a job
    /// since.
    Leave,
    /// (For tests: work of a test's own, done there as a frame's is.)
    #[cfg(test)]
    Probe(Box<dyn FnOnce() + Send>),
}

/// A frame as the window shows it: straight RGBA, top row first.
struct Rendered {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

fn surface_key(id: &str, path: &str) -> String {
    format!("visual/{id}/{path}")
}

/// The GPU's thread, from the first job sent it until it has left.
struct GpuThread {
    /// Which one it is: what it says of itself as it ends is said of this
    /// one, never of the one started after it.
    id: u64,
    jobs: mpsc::Sender<Job>,
    /// Its last user gone: it leaves after the job it's on, unless a thread
    /// sends it a job first (it stays then, for that thread).
    leaving: bool,
    /// Past its last job, with nothing held on the GPU any more: only the
    /// thread's own end is left.
    left: bool,
    /// (Taken by the one that waits for the thread's end.)
    handle: Option<std::thread::JoinHandle<()>>,
}

/// The GPU's thread and how many threads use it (the app's own thread; in
/// a test, each test's). The GPU's thread stays here until it has left, so
/// a frame sent as it leaves goes to it: no second one is ever started
/// beside one still on a frame.
struct GpuThreads {
    thread: Option<GpuThread>,
    users: usize,
    /// How many were started.
    started: u64,
}

static GPU_THREAD: Mutex<GpuThreads> = Mutex::new(GpuThreads { thread: None, users: 0, started: 0 });

/// Told when the GPU's thread has left, and when a thread takes it up
/// again as it was leaving.
static GPU_CHANGED: std::sync::Condvar = std::sync::Condvar::new();

/// (What's kept there is whole at each unlock: a panic under the lock
/// spoils nothing.)
fn gpu_threads() -> std::sync::MutexGuard<'static, GpuThreads> {
    GPU_THREAD.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The longest quitting waits for the GPU's thread to finish the job it's
/// on (a first frame makes the device: a second or two on a busy machine).
const GPU_STOP_WAIT: std::time::Duration = std::time::Duration::from_secs(10);

/// A thread's use of the GPU's thread, from its first job until it ends
/// (or [`stop_gpu`]). The last one to go tells the GPU's thread to leave,
/// and waits until it has.
///
/// The process must not exit under the GPU's thread. `exit` runs the
/// destructors of every library loaded (on Linux: the Vulkan driver's, the
/// validation layer's in a debug build, LLVM's) and unloads them while
/// other threads still run, so a frame still rendering then reads freed
/// state: a SIGSEGV, an abort in malloc, or a deadlock on a lock `exit`
/// holds, now and then as a test's process (or the app) ends. A thread's
/// locals are dropped before any of that, so this is where it waits: for
/// as long as the frame takes, so a thread that used the GPU's thread
/// never ends, and a test's process never exits, under its frame. (A
/// frame that never ends is a thread that never does: a test that hangs,
/// not one that dies as its process exits. The app's own end has a limit:
/// quitting, [`stop_gpu`], and a panic out of `main`,
/// [`StopGpuOnPanic`]. Both end the process without `exit` then.)
///
/// (Not on Windows, where a thread's locals are dropped under the loader's
/// lock, which the GPU's thread needs to load the driver and to end: there
/// only [`stop_gpu`] waits, and a process exiting ends its other threads
/// before it unloads anything. For the same reason nothing here waits for
/// a thread's end with [`GPU_THREAD`] locked: a thread ending takes that
/// lock under the loader's.)
struct GpuUser {
    using: std::cell::Cell<bool>,
}

thread_local! {
    static GPU_USER: GpuUser = const { GpuUser { using: std::cell::Cell::new(false) } };
}

impl GpuUser {
    /// This thread's use over: the GPU's thread told to leave, if it was
    /// the last.
    fn release(&self) {
        if !self.using.replace(false) {
            return;
        }
        let mut gpu = gpu_threads();
        gpu.users = gpu.users.saturating_sub(1);
        if gpu.users == 0
            && let Some(thread) = &mut gpu.thread
            && !thread.leaving
            && !thread.left
        {
            thread.leaving = true;
            // (Wakes it. It looks at `leaving` again once it gets there.)
            let _ = thread.jobs.send(Job::Leave);
        }
    }
}

impl Drop for GpuUser {
    fn drop(&mut self) {
        self.release();
        if cfg!(unix) {
            gpu_left(None);
        }
    }
}

/// Waits for the GPU's thread to have left, `at_most` so long (`None`: as
/// long as it takes), and says whether it has: `true` once the process has
/// no GPU's thread. `false` at once while a thread uses it: it's that
/// thread's to wait for.
fn gpu_left(at_most: Option<std::time::Duration>) -> bool {
    let limit = at_most.map(|wait| std::time::Instant::now() + wait);
    let mut gpu = gpu_threads();
    loop {
        let Some(thread) = &mut gpu.thread else { return true };
        if thread.left {
            if let Some(handle) = thread.handle.take() {
                let id = thread.id;
                // (Its end, a moment more, not waited for under the lock:
                // see `GpuUser`.)
                drop(gpu);
                let _ = handle.join();
                let mut gpu = gpu_threads();
                if gpu.thread.as_ref().is_some_and(|thread| thread.id == id) {
                    gpu.thread = None;
                }
                drop(gpu);
                GPU_CHANGED.notify_all();
                return true;
            }
            // (Another thread is waiting out its end, and tells.)
        } else if !thread.leaving {
            return false;
        }
        gpu = match limit {
            None => GPU_CHANGED.wait(gpu).unwrap_or_else(std::sync::PoisonError::into_inner),
            Some(limit) => {
                let now = std::time::Instant::now();
                if now >= limit {
                    return false;
                }
                GPU_CHANGED.wait_timeout(gpu, limit - now).unwrap_or_else(std::sync::PoisonError::into_inner).0
            }
        };
    }
}

/// This thread is done with the GPU's thread (the app quitting, after its
/// windows closed). Once no thread uses it, it finishes the job it's on and
/// leaves; this waits for that, [`GPU_STOP_WAIT`] at most, and says whether
/// it has left. A later frame starts it again.
///
/// `false`: it's still on a frame (or another thread's), and stays known
/// here, so a later call waits for it again. The process must not go
/// through `exit` under it ([`GpuUser`]): quitting ends with
/// [`exit_under_gpu`] then.
#[must_use = "false: the GPU's thread still runs, and the process must not exit under it (exit_under_gpu)"]
pub fn stop_gpu() -> bool {
    stop_gpu_within(GPU_STOP_WAIT)
}

/// [`stop_gpu`], waiting `at_most` so long.
#[must_use]
pub fn stop_gpu_within(at_most: std::time::Duration) -> bool {
    // (A thread on its way out, its locals gone, has let go of it already.)
    let _ = GPU_USER.try_with(GpuUser::release);
    gpu_left(Some(at_most))
}

/// The end of quitting when [`stop_gpu`] said the GPU's thread still runs:
/// the process ends here with `status`, not through `exit`, whose
/// destructors would run under the frame it's on ([`GpuUser`]). What the
/// process still had to write to its output is written first; nothing else
/// that runs at `exit` does.
pub fn exit_under_gpu(status: i32) -> ! {
    use std::io::Write;
    eprintln!("zend-zgx: the visuals' GPU thread is still on a frame: the process ends without waiting for it any longer");
    let _ = std::io::stdout().flush();
    let _ = std::io::stderr().flush();
    // SAFETY: `_exit` takes a status and ends the process at once; nothing
    // of this process runs after it.
    unsafe { libc::_exit(status) }
}

/// Held by `main` as the app runs, for a panic that unwinds out of it.
/// The process ends through `exit` then too, where this thread's
/// [`GpuUser`] waits for the frame without limit: a frame that never ends
/// would keep a crashed app from ever ending. Dropped by a panic, this
/// waits for the GPU's thread only as long as quitting does
/// ([`GPU_STOP_WAIT`]), and ends the process as quitting does, with a
/// panic's exit status.
pub struct StopGpuOnPanic {
    at_most: std::time::Duration,
}

impl StopGpuOnPanic {
    pub fn new() -> Self {
        Self::within(GPU_STOP_WAIT)
    }

    /// (Waiting `at_most` so long.)
    pub fn within(at_most: std::time::Duration) -> Self {
        Self { at_most }
    }
}

impl Drop for StopGpuOnPanic {
    fn drop(&mut self) {
        if std::thread::panicking() && !stop_gpu_within(self.at_most) {
            exit_under_gpu(101);
        }
    }
}

/// The GPU's thread: one for every visual, as the shipping app's one
/// device. Each surface's state (its targets, a shader's feedback
/// buffers) stays there between frames. It runs while a thread that sent
/// it a job lives ([`GpuUser`]).
fn gpu_send(job: Job) {
    // (A thread on its way out, its locals already gone, has no frame to show.)
    let Ok(new) = GPU_USER.try_with(|user| !user.using.replace(true)) else { return };
    let mut gpu = gpu_threads();
    let gpu = &mut *gpu;
    if new {
        gpu.users += 1;
    }
    // One that has left is done with the GPU: the next starts in its place.
    // (Its end isn't waited for here, which may be in a thread's own end:
    // see `GpuUser`.)
    if gpu.thread.as_ref().is_some_and(|thread| thread.left) {
        gpu.thread = None;
    }
    let thread = match &mut gpu.thread {
        Some(thread) => {
            // Leaving, perhaps still on a departed thread's frame: it stays
            // and takes this one after it. Never a second one beside it.
            if std::mem::take(&mut thread.leaving) {
                GPU_CHANGED.notify_all();
            }
            thread
        }
        none => {
            gpu.started += 1;
            let id = gpu.started;
            let (jobs, rx) = mpsc::channel::<Job>();
            let spawned = std::thread::Builder::new().name("zend-visual-gpu".into()).spawn(move || gpu_thread(id, rx));
            let Ok(handle) = spawned else { return };
            none.insert(GpuThread { id, jobs, leaving: false, left: false, handle: Some(handle) })
        }
    };
    let _ = thread.jobs.send(job);
}

/// (For tests: `work` done on the GPU's thread, as a frame is.)
#[cfg(test)]
pub fn gpu_probe(work: impl FnOnce() + Send + 'static) {
    gpu_send(Job::Probe(Box::new(work)));
}

/// (For tests: whether the GPU's thread runs.)
#[cfg(test)]
pub fn gpu_thread_runs() -> bool {
    gpu_threads().thread.as_ref().is_some_and(|thread| !thread.left)
}

/// (For tests: how many threads use the GPU's thread.)
#[cfg(test)]
pub fn gpu_thread_users() -> usize {
    gpu_threads().users
}

fn gpu_thread(id: u64, jobs: mpsc::Receiver<Job>) {
    /// Says this one has left, however it ends. (Dropped last, after the
    /// surfaces: what waits for it then finds the GPU let go of.)
    struct Left(u64);
    impl Drop for Left {
        fn drop(&mut self) {
            if let Some(thread) = &mut gpu_threads().thread
                && thread.id == self.0
            {
                thread.left = true;
            }
            GPU_CHANGED.notify_all();
        }
    }
    let _left = Left(id);
    // Whether its last user has gone, and no thread has sent it a job since.
    let leaving = || gpu_threads().thread.as_ref().is_some_and(|thread| thread.id == id && thread.leaving);
    let mut scenes: HashMap<String, Option<zend_visual_gpu::SceneState>> = HashMap::new();
    let mut shaders: HashMap<String, Option<zend_visual_gpu::ShaderState>> = HashMap::new();
    while let Ok(job) = jobs.recv() {
        if leaving() {
            // (Its last user gone: nothing waits for the frames still asked for.)
            if !matches!(job, Job::Leave) {
                continue;
            }
            // What it holds on the GPU goes first, so that it has left only
            // with the GPU let go of.
            scenes.clear();
            shaders.clear();
            let mut gpu = gpu_threads();
            if let Some(thread) = &mut gpu.thread
                && thread.id == id
                && thread.leaving
            {
                thread.left = true;
                drop(gpu);
                GPU_CHANGED.notify_all();
                return;
            }
            // (A thread sent it a job meanwhile: it stays, for that thread.)
            continue;
        }
        let start = std::time::Instant::now();
        let cpu = thread_cpu_ns();
        let frame = matches!(job, Job::Scene { .. } | Job::Shader { .. });
        match job {
            Job::Release { key } => {
                scenes.remove(&key);
                shaders.remove(&key);
            }
            // (Taken up again by a thread since it was told to leave.)
            Job::Leave => {}
            #[cfg(test)]
            Job::Probe(work) => work(),
            Job::Scene { key, scene, camera, size, scale, palette, reply } => {
                let rgba = move |c: zv::Color| palette.color(c);
                let state = scenes.entry(key).or_default();
                let frame = zend_visual_gpu::render_scene(&scene, camera, size.0, size.1, scale, &rgba, state)
                    .map(|f| (f.width, f.height, f.bgra))
                    .unwrap_or_else(|| {
                        // No GPU here: the software renderer.
                        SOFTWARE_FRAMES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let r = scene.render(camera, size.0, size.1, scale, zv::Quality::Final, &rgba);
                        (r.width, r.height, r.bgra)
                    });
                reply.send(Ok(rendered(frame)));
            }
            Job::Shader { key, node, assets, uniforms, size, reply } => {
                let names: Vec<String> = node.inputs.iter().map(|(n, _)| n.clone()).collect();
                let textures: Vec<zv::TextureData> = node
                    .textures
                    .iter()
                    .map(|name| zv::TextureData::Encoded(assets.get(name).unwrap_or_else(|| Arc::from([]))))
                    .collect();
                let spec = zend_visual_gpu::ShaderSpec {
                    image: &node.code,
                    buffers: &node.buffers,
                    inputs: &names,
                    blend: node.blend,
                    bloom: node.bloom,
                    textures: &textures,
                };
                let state = shaders.entry(key).or_default();
                let out = zend_visual_gpu::render_shader(&spec, &uniforms, size.0, size.1, state)
                    .map(|f| rendered((f.width, f.height, f.bgra)));
                reply.send(out);
            }
        }
        if frame {
            use std::sync::atomic::Ordering::Relaxed;
            GPU_NS.fetch_add(start.elapsed().as_nanos() as u64, Relaxed);
            GPU_CPU_NS.fetch_add(thread_cpu_ns().saturating_sub(cpu), Relaxed);
            GPU_FRAMES.fetch_add(1, Relaxed);
        }
    }
}

#[cfg(test)]
pub fn bench_swap(bgra: Vec<u8>) -> usize {
    rendered((1600, 680, bgra)).rgba.len()
}

/// BGRA to RGBA, in place.
fn rendered((width, height, mut bgra): (u32, u32, Vec<u8>)) -> Rendered {
    zend_zgx_pixels::bgra_to_rgba(&mut bgra);
    Rendered { width, height, rgba: bgra }
}

/// `visuals.frame`: a scene's drawing (run here, it's the script's) or a
/// shader's uniforms, rendered on the GPU's thread, shown as a host image.
async fn frame(v: Rc<Visuals>, host: ScriptHost, arg: Value) -> Result<Value, String> {
    let id = text(arg.field("id"));
    let path = text(arg.field("p"));
    let n = |k: &str| arg.field(k).and_then(Value::as_f64);
    let size = (n("w").unwrap_or(0.).max(1.) as f32, n("ht").unwrap_or(0.).max(1.) as f32);
    let scale = (n("scale").unwrap_or(2.) as f32).clamp(1., MAX_GPU_SCALE);
    let time = n("t").unwrap_or(0.);
    let key = surface_key(&id, &path);
    let (reply, mut answer) = zgx::task::latest();
    let mut labels = Value::List(Rc::from([]));
    let mut hints = Value::List(Rc::from([]));
    let job = v
        .with(&id, |e| -> Result<Job, String> {
            let node = e.node_at(&path).cloned().ok_or("that part of the visual is gone")?;
            let generation = e.generation;
            match node.kind {
                zv::NodeKind::Scene(s) => {
                    let fresh = e.scenes.get(&path).is_some_and(|(at, _)| *at == generation && !s.animate);
                    if !fresh {
                        let scene = e.wake()?.draw_scene(s.draw, time).map_err(|e| e.message)?;
                        e.scenes.insert(path.clone(), (generation, Arc::new(scene)));
                    }
                    let scene = e.scenes[&path].1.clone();
                    let camera = zv::Camera {
                        yaw: n("yaw").unwrap_or(s.yaw as f64) as f32,
                        pitch: (n("pitch").unwrap_or(s.pitch as f64) as f32).clamp(-1.5, 1.5),
                        zoom: s.zoom,
                    };
                    let (found_hints, found_labels) = scene.annotations(camera, size.0, size.1);
                    labels = list(
                        found_labels
                            .iter()
                            .map(|l| {
                                Value::record([
                                    ("x", num(l.at[0])),
                                    ("y", num(l.at[1])),
                                    ("t", Value::from(l.text.as_str())),
                                    ("c", color_value(l.color)),
                                    ("size", Value::from(size_name(l.size))),
                                ])
                            })
                            .collect(),
                    );
                    hints = list(
                        found_hints
                            .iter()
                            .map(|(at, r, t)| {
                                Value::record([("x", num(at[0])), ("y", num(at[1])), ("r", num(*r)), ("t", Value::from(t.as_str()))])
                            })
                            .collect(),
                    );
                    Ok(Job::Scene { key: key.clone(), scene, camera, size, scale, palette: Palette::from(arg.field("pal")), reply })
                }
                zv::NodeKind::Shader(s) => {
                    let device = ((size.0 * scale).round().max(1.) as u32, (size.1 * scale).round().max(1.) as u32);
                    let (frames, last) = e.shaders.entry(path.clone()).or_insert((0, 0.));
                    let delta = (time - *last).max(0.);
                    // Shadertoy's mouse: device pixels from the bottom left;
                    // xy where the pointer is, zw where it went down while
                    // it's held (negative otherwise).
                    let at = |x: f64, y: f64| [x as f32 * scale, device.1 as f32 - y as f32 * scale];
                    let mouse = match arg.field("mouse") {
                        Some(m) => {
                            let g = |k: &str| m.field(k).and_then(Value::as_f64);
                            let [x, y] = at(g("x").unwrap_or(0.), g("y").unwrap_or(0.));
                            match (m.field("down").is_some_and(Value::truthy), g("sx"), g("sy")) {
                                (true, Some(sx), Some(sy)) => {
                                    let [sx, sy] = at(sx, sy);
                                    [x, y, sx, sy]
                                }
                                _ => [x, y, -1., -1.],
                            }
                        }
                        None => [0., 0., -1., -1.],
                    };
                    let mut inputs = [[0f32; 4]; zv::shader::MAX_INPUTS / 4];
                    for (i, (_, value)) in s.inputs.iter().enumerate() {
                        inputs[i / 4][i % 4] = *value;
                    }
                    let uniforms = zv::shader::ShaderUniforms {
                        resolution: [device.0 as f32, device.1 as f32, scale, 0.],
                        mouse,
                        camera: [
                            n("yaw").unwrap_or(s.yaw as f64) as f32,
                            (n("pitch").unwrap_or(s.pitch as f64) as f32).clamp(-1.5, 1.5),
                            s.zoom,
                            0.,
                        ],
                        time: [time as f32, delta as f32, *frames as f32, 0.],
                        inputs,
                        channels: Default::default(),
                    };
                    *frames += 1;
                    *last = time;
                    Ok(Job::Shader {
                        key: key.clone(),
                        node: Box::new(s),
                        assets: e.assets.clone(),
                        uniforms,
                        size: device,
                        reply,
                    })
                }
                _ => Err("not a scene or shader".into()),
            }
        })
        .ok_or("no such visual")?;
    let job = match job {
        Ok(job) => job,
        Err(why) => {
            // A scene draw that fails is the script's error, shown as one.
            v.with(&id, |e| e.fail(zv::VisualError::script(why.clone())));
            v.publish();
            return Err(why);
        }
    };
    gpu_send(job);
    let rendered = answer.recv().await.ok_or("the GPU's thread stopped")??;
    let image = zgx::image::ImageData::new(rendered.width, rendered.height, rendered.rgba).map_err(str::to_string)?;
    host.image(&key, Some(Arc::new(image)));
    Ok(Value::record([("src", Value::from(format!("host:{key}").as_str())), ("labels", labels), ("hints", hints)]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn started(id: &str, source: &str) -> Entry {
        let mut e = Entry::new("Test".into());
        e.start(id, source.into(), zv::Assets::default());
        e
    }

    fn tree(e: &Entry) -> Value {
        e.snapshot().field("tree").cloned().expect("a tree")
    }

    const COUNTER: &str = r#"
        local count = 0
        local on = false
        return function()
          return ui.column {
            ui.stat { value = count, label = "Clicks" },
            ui.button { label = "Add", on_click = function() count += 1 end },
            ui.toggle { label = "On", value = on, on_change = function(v) on = v end },
          }
        end
    "#;

    const FORM: &str = r#"
        return function()
          return ui.column {
            ui.field { name = "goal", label = "Goal", value = "ship" },
            ui.checklist { name = "steps", options = { "build", "test" } },
          }
        end
    "#;

    fn input_data(e: &Entry, name: &str) -> Value {
        fn find(v: &Value, name: &str) -> Option<Value> {
            if v.field("k").map(crate::auth::text).as_deref() == Some("input")
                && v.field("name").map(crate::auth::text).as_deref() == Some(name)
            {
                return Some(v.clone());
            }
            v.field("children")?.items().iter().find_map(|c| find(c, name))
        }
        find(&tree(e), name).expect("the input")
    }

    #[test]
    fn inputs_go_as_data_keyed_by_name_with_the_entry_shown() {
        let mut e = started("v:form", FORM);
        let goal = input_data(&e, "goal");
        assert_eq!(goal.field("key").map(crate::auth::text).as_deref(), Some("input:goal"));
        assert_eq!(goal.field("value").map(crate::auth::text).as_deref(), Some("ship"));
        assert_eq!(goal.field("kept"), Some(&Value::Bool(false)));
        e.set_entry("goal", entry_value(Some(&Value::from("mine"))));
        e.set_entry("steps", entry_value(Some(&list(vec![Value::from("test")]))));
        let goal = input_data(&e, "goal");
        assert_eq!(goal.field("value").map(crate::auth::text).as_deref(), Some("mine"));
        assert_eq!(goal.field("kept"), Some(&Value::Bool(true)));
        let steps = input_data(&e, "steps");
        assert_eq!(steps.field("value").map(|v| v.items().len()), Some(1));
        let entries = entries_value(&e.kept);
        assert_eq!(entries.field("goal").map(crate::auth::text).as_deref(), Some("mine"));
        // Nil clears it.
        e.set_entry("goal", entry_value(Some(&Value::Nil)));
        assert_eq!(input_data(&e, "goal").field("value").map(crate::auth::text).as_deref(), Some("ship"));
    }

    #[test]
    fn entries_outlive_a_sleep_and_a_new_script() {
        let mut e = started("v:form", FORM);
        e.set_entry("goal", Some(zv::EntryValue::Text("mine".into())));
        e.keep_awake = false;
        e.sleep();
        assert!(e.runtime.is_none(), "a form with nothing but entries sleeps");
        e.wake().unwrap();
        assert_eq!(input_data(&e, "goal").field("value").map(crate::auth::text).as_deref(), Some("mine"));
        e.swap(
            "v:form",
            r#"return function() return ui.column {
                ui.field { name = "goal", value = "theirs" },
                ui.field { name = "due" },
            } end"#
                .into(),
        )
        .unwrap();
        assert_eq!(input_data(&e, "goal").field("value").map(crate::auth::text).as_deref(), Some("mine"));
        assert_eq!(input_data(&e, "due").field("fresh"), Some(&Value::Bool(true)));
        assert!(e.swap("v:form", "return 5".into()).is_err());
        assert_eq!(input_data(&e, "goal").field("value").map(crate::auth::text).as_deref(), Some("mine"));
    }

    #[test]
    fn the_focused_field_holds_while_the_script_changes() {
        let mut e = started("v:form", FORM);
        e.focus(Some("goal"));
        e.swap("v:form", r#"return function() return ui.field { name = "goal", value = "new" } end"#.into())
            .unwrap();
        assert_eq!(input_data(&e, "goal").field("value").map(crate::auth::text).as_deref(), Some("ship"));
        e.focus(None);
        assert_eq!(input_data(&e, "goal").field("value").map(crate::auth::text).as_deref(), Some("new"));
    }

    #[test]
    fn a_visual_runs_and_its_tree_is_plain_data() {
        let e = started("v:counter", COUNTER);
        assert_eq!(e.load, Load::Ready);
        let t = tree(&e);
        assert_eq!(text(t.field("k")), "column");
        let kids = t.field("children").unwrap().list();
        assert_eq!(text(kids[0].field("k")), "stat");
        assert_eq!(text(kids[0].field("value")), "0");
        assert_eq!(text(kids[1].field("p")), "0.1");
        assert!(kids[1].field("click").and_then(Value::as_f64).is_some());
    }

    #[test]
    fn a_handler_changes_state_and_the_view_renders_again() {
        let mut e = started("v:counter", COUNTER);
        let (rev, generation) = (e.rev, e.generation);
        let click = tree(&e).field("children").unwrap().list()[1].field("click").and_then(Value::as_f64).unwrap();
        e.event(click as u32, zv::HandlerArg::None);
        assert!(e.rev > rev && e.generation > generation);
        let kids = tree(&e).field("children").unwrap().list().to_vec();
        assert_eq!(text(kids[0].field("value")), "1");
        // A toggle's value comes back as its handler set it.
        let change = kids[2].field("change").and_then(Value::as_f64).unwrap();
        e.event(change as u32, zv::HandlerArg::Bool(true));
        assert_eq!(tree(&e).field("children").unwrap().list()[2].field("value"), Some(&Value::Bool(true)));
    }

    #[test]
    fn the_same_tree_keeps_its_shape_but_draws_again() {
        let source = r#"
            local n = 0
            return function()
              return ui.column {
                ui.canvas { height = 80, draw = function(g, w, h) g:rect(0, 0, n, 10, { fill = "accent" }) end },
                ui.button { label = "Grow", on_click = function() n += 10 end },
              }
            end
        "#;
        let mut e = started("v:canvas", source);
        let kids = tree(&e).field("children").unwrap().list().to_vec();
        let (draw, click) = (kids[0].field("draw").and_then(Value::as_f64).unwrap(), kids[1].field("click").and_then(Value::as_f64).unwrap());
        let (rev, generation) = (e.rev, e.generation);
        e.event(click as u32, zv::HandlerArg::None);
        assert_eq!(e.rev, rev, "the tree didn't change shape");
        assert!(e.generation > generation, "but the canvas reads new state");
        let drawn = e.draw(&Value::record([("h", num(draw)), ("w", num(200.)), ("ht", num(80.)), ("t", num(0.))]));
        let cmds = drawn.field("cmds").unwrap().list();
        // The rectangle (a shape the canvas draws as it is) is ten wide now.
        assert_eq!(text(cmds[0].field("k")), "r");
        assert_eq!(cmds[0].field("w").and_then(Value::as_f64), Some(10.));
    }

    #[test]
    fn a_runaway_script_is_stopped_with_its_message() {
        let e = started("v:loop", "while true do end");
        match &e.load {
            Load::Failed(why) => assert!(why.contains("time budget"), "{why}"),
            other => panic!("{other:?}"),
        }
        assert_eq!(text(e.snapshot().field("state")), "failed");
        // A runaway handler keeps the last good tree and says why.
        let mut e = started(
            "v:handler",
            r#"return function() return ui.button { label = "Hang", on_click = function() while true do end end } end"#,
        );
        let click = tree(&e).field("click").and_then(Value::as_f64).unwrap();
        e.event(click as u32, zv::HandlerArg::None);
        let snap = e.snapshot();
        assert!(text(snap.field("error")).contains("time budget"));
        assert_eq!(text(snap.field("tree").unwrap().field("k")), "button");
    }

    #[test]
    fn seeded_randomness_is_the_visuals_own() {
        let source = "local r = math.random(1, 1000000) return function() return ui.text { tostring(r) } end";
        let a = started("v:one", source);
        let b = started("v:one", source);
        let c = started("v:two", source);
        let words = |e: &Entry| text(tree(e).field("spans").unwrap().list()[0].field("t"));
        assert_eq!(words(&a), words(&b));
        assert_ne!(words(&a), words(&c));
    }

    #[test]
    fn charts_carry_round_axes_and_tables_number_their_cells() {
        let source = r#"
            return function()
              return ui.column {
                ui.chart { type = "bar", x = { "a", "b" }, series = { { name = "n", values = { 3, 47 } } }, unit = "ms" },
                ui.table { columns = { "Name", "ms" }, rows = { { "a", 3 }, { "b", 47 } } },
              }
            end
        "#;
        let e = started("v:chart", source);
        let kids = tree(&e).field("children").unwrap().list().to_vec();
        let y = kids[0].field("yaxis").unwrap();
        assert_eq!(y.field("lo").and_then(Value::as_f64), Some(0.));
        assert!(y.field("hi").and_then(Value::as_f64).unwrap() >= 47.);
        assert!(text(y.field("labels").unwrap().list().last()).ends_with(" ms"));
        let rows = kids[1].field("rows").unwrap().list();
        assert_eq!(text(rows[1].list()[0].field("p")), "0.1.2");
        // A numeric column aligns to its end.
        assert_eq!(text(kids[1].field("columns").unwrap().list()[1].field("align")), "end");
    }

    #[test]
    fn circles_rects_and_lines_go_as_shapes_and_polygons_stay_polygons() {
        let source = r#"
            return ui.canvas { height = 100, draw = function(g, w, h)
              g:circle(50, 50, 20, { fill = "accent" })
              g:circle(10, 10, 2, { fill = "accent" })
              g:rect(5, 6, 30, 40, { fill = "muted" })
              g:rect(5, 6, 30, 40, { fill = "muted", radius = 6 })
              g:line(0, 0, 10, 10, { color = "border" })
              g:polygon({ 60, 10, 70, 15, 70, 25, 60, 30, 50, 25, 50, 15 }, { fill = "danger" })
            end }
        "#;
        let mut e = started("v:shapes", source);
        let draw = tree(&e).field("draw").and_then(Value::as_f64).unwrap();
        let drawn = e.draw(&Value::record([("h", num(draw)), ("w", num(100.)), ("ht", num(100.)), ("t", num(0.))]));
        let kinds: Vec<String> = drawn.field("cmds").unwrap().list().iter().map(|c| text(c.field("k"))).collect();
        assert_eq!(kinds, ["c", "c", "r", "f", "l", "f"]);
        let circle = &drawn.field("cmds").unwrap().list()[0];
        assert!((circle.field("r").and_then(Value::as_f64).unwrap() - 20.).abs() < 0.01);
        assert!((circle.field("x").and_then(Value::as_f64).unwrap() - 50.).abs() < 0.01);
    }

    #[test]
    fn colors_are_tokens_or_mixes() {
        assert_eq!(color_value(zv::Color::token(zv::Token::Chart(2))), Value::from("chart3"));
        let faded = color_value(zv::Color::token(zv::Token::Accent).with_alpha(0.5));
        assert_eq!(text(faded.field("token")), "accent");
        assert_eq!(faded.field("alpha").and_then(Value::as_f64), Some(0.5));
        assert_eq!(hex("#ff000080").map(|c| c[3]), Some(128. / 255.));
    }
}
