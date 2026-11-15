//! Agent visuals in the design system (ds/visual) on the real host
//! (src/visuals.rs, the shipping app's crates/visual): ui/visuals_fixture's
//! scripts, every node type, mounted headless and used as a reader uses
//! them. `cargo test --bin zend-zgx visuals_tests`.

use zgx_script::{ScriptHost, testing::TestApp};

fn mount(only: Option<&str>) -> TestApp {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("ui/visuals_fixture.luau");
    let only = only.map(|name| format!("only={name}"));
    let mut app = TestApp::new(|runtime| {
        let host = ScriptHost::new(runtime, path);
        let host = match only {
            Some(arg) => host.args([arg]),
            None => host,
        };
        crate::visuals::register(&host);
        host
    });
    app.ui.scene.borrow_mut().resize(860., 3200.);
    app.frames(10);
    // (Each starts once its script has run: at once, from `source`.)
    app.wait_until(|app| app.find("~\"Loading \"").is_none());
    app.frames(20);
    assert_eq!(app.error(), None, "the fixture failed");
    app
}

fn has(app: &mut TestApp, target: &str) -> bool {
    app.find(target).is_some()
}

#[test]
fn every_node_type_draws_with_the_design_system() {
    let mut app = mount(None);
    let mut missing = Vec::new();
    for target in [
        // Stats, tabs, a chart, a slider, a toggle, a button.
        "#visual-dashboard >> \"158 ms\"",
        "#visual-dashboard >> \"p95 latency\"",
        "#visual-dashboard >> \"−8%\"",
        "#visual-dashboard >> role:radio \"30d\"",
        "#visual-dashboard >> .chart-plot",
        "#visual-dashboard >> \"Smoothing window\"",
        "#visual-dashboard >> role:switch",
        "#visual-dashboard >> role:switch \"Show table\"",
        "#visual-dashboard >> role:button \"Reset\"",
        // Bars, a grid, a rule, a line with a gap, a scatter.
        "#visual-charts >> \"Where the build's time goes\"",
        "#visual-charts >> .visual-grid",
        "#visual-charts >> \"Cache hit rate\"",
        // Text in spans, a row to click, a link.
        "#visual-words >> ~\"What changed in\"",
        "#visual-words >> \"Faster cold starts\"",
        "#visual-words >> role:link \"Read the full notes\"",
        // A canvas, a scene, a shader.
        "#visual-canvas >> .visual-canvas",
        "#visual-scene >> .visual-scene",
        "#visual-shader >> .visual-shader",
        // A runaway script: stopped, with its message, and nothing else
        // broken round it.
        "#visual-broken >> \"Couldn't show this visual\"",
    ] {
        if !has(&mut app, target) {
            missing.push(target);
        }
    }
    assert!(missing.is_empty(), "missing: {missing:#?}\n{}", app.tree());
    // Nothing raw in the reply: the runaway's message is behind Copy error.
    assert!(!has(&mut app, "~\"time budget\""), "{}", app.tree());
    // Every chart's plot: the dashboard's area, the bars, a line, a scatter.
    assert!(has(&mut app, ".chart-plot[4]") && !has(&mut app, ".chart-plot[5]"), "{}", app.tree());
}

#[test]
fn controls_reach_the_script_and_its_view_follows() {
    let mut app = mount(Some("dashboard"));
    assert!(!has(&mut app, ".data-table"), "the table starts hidden");
    // A toggle shows the table; a tab changes its numbers.
    app.click("#visual-dashboard >> role:switch \"Show table\"");
    app.wait_until(|app| app.find(".data-table >> \"Mon\"").is_some());
    assert!(has(&mut app, ".data-table >> \"182\""), "7d's Monday p95:\n{}", app.tree());
    app.click("#visual-dashboard >> role:radio \"30d\"");
    app.wait_until(|app| app.find(".data-table >> \"210\"").is_some());
    assert!(has(&mut app, ".data-table >> \"210\"") && !has(&mut app, ".data-table >> \"182\""), "30d's Monday p95:\n{}", app.tree());
}

#[test]
fn a_value_changing_keeps_what_is_drawn() {
    let mut app = mount(Some("dashboard"));
    let slider = app.find(".visual-slider").expect("the slider");
    let plot = app.find(".chart-plot").expect("the chart");
    // The tab's value changes the tree's data, not its shape: the slider
    // and the chart stay the very nodes they were (a drag would go on).
    app.click("#visual-dashboard >> role:radio \"30d\"");
    app.frames(20);
    assert_eq!(app.find(".visual-slider"), Some(slider));
    assert_eq!(app.find(".chart-plot"), Some(plot));
    // The table coming changes its shape: built again, and right.
    app.click("#visual-dashboard >> role:switch \"Show table\"");
    app.wait_until(|app| app.find(".data-table >> \"Mon\"").is_some());
    assert!(has(&mut app, ".data-table >> \"210\""));
}

#[test]
fn a_form_draws_every_input_kind_and_keeps_what_is_entered() {
    let mut app = mount(Some("form"));
    let mut missing = Vec::new();
    for target in [
        "#visual-form >> role:textbox \"Goal\"",
        "#visual-form >> role:textbox \"Notes\"",
        "#visual-form >> role:checkbox \"Downtime is fine\"",
        "#visual-form >> role:checkbox \"Verify\"",
        "#visual-form >> role:radio \"This weekend\"",
        "#visual-form >> role:checkbox \"On-call\"",
        "#visual-form >> role:button \"Region\"",
        "#visual-form >> \"Database: Postgres 16 (from the research chat)\"",
        "#visual-form >> role:button \"Change\"",
        "#visual-form >> \"Goal so far: (none)\"",
    ] {
        if !has(&mut app, target) {
            missing.push(target);
        }
    }
    assert!(missing.is_empty(), "missing: {missing:#?}\n{}", app.tree());
    // A box ticked and a choice made reach the entries, outside the script.
    app.click("#visual-form >> role:checkbox \"Downtime is fine\"");
    app.click("#visual-form >> role:radio \"This weekend\"");
    app.type_text("#visual-form >> role:textbox \"Notes\"", "after 22:00");
    app.wait_until(|app| app.find("#visual-form >> \"Goal so far: (none)\"").is_some());
    app.frames(10);
    let entries = crate::visuals::entries_of("v:fixture-form");
    assert_eq!(entries.field("downtime"), Some(&zgx_script::Value::Bool(true)), "{entries:?}");
    assert_eq!(
        entries.field("when").map(crate::auth::text).as_deref(),
        Some("This weekend"),
        "{entries:?}"
    );
    assert_eq!(entries.field("notes").map(crate::auth::text).as_deref(), Some("after 22:00"), "{entries:?}");
    // An answered input opens again on Change.
    app.click("#visual-form >> role:button \"Change\"");
    app.wait_until(|app| app.find("#visual-form >> role:textbox \"Database\"").is_some());
    // A new script for the visual keeps them, and draws them.
    crate::visuals::swap_source(
        "v:fixture-form",
        r#"return function() return ui.column {
            ui.check { name = "downtime", label = "Downtime is fine" },
            ui.field { name = "owner", label = "Owner" },
            ui.text { "when: " .. (ui.entry("when") or "-") },
        } end"#,
    )
    .unwrap();
    app.wait_until(|app| app.find("#visual-form >> \"when: This weekend\"").is_some());
    assert!(has(&mut app, "#visual-form >> .visual-input-new"), "the new field says so:\n{}", app.tree());
    assert_eq!(app.error(), None);
}

#[test]
fn numbers_lists_and_rows_are_drawn_and_what_is_entered_is_kept() {
    use zgx_script::Value;
    let mut app = mount(Some("form_e"));
    let mut missing = Vec::new();
    for target in [
        "#visual-form_e >> role:button \"Fit: 5 / 5\"",
        "#visual-form_e >> role:button \"More\"",
        "#visual-form_e >> role:slider \"Hours, 9\"",
        "#visual-form_e >> role:textbox \"Risks 1\"",
        "#visual-form_e >> role:textbox \"Add a risk\"",
        "#visual-form_e >> role:textbox \"Item 1\"",
        "#visual-form_e >> role:checkbox \"Approved 1\"",
        "#visual-form_e >> \"Seats so far: -\"",
    ] {
        if !has(&mut app, target) {
            missing.push(target);
        }
    }
    assert!(missing.is_empty(), "missing: {missing:#?}\n{}", app.tree());
    app.click("#visual-form_e >> role:button \"Fit: 4 / 5\"");
    app.click("#visual-form_e >> role:button \"More\"");
    app.click("#visual-form_e >> role:button \"More\"");
    app.wait_until(|app| app.find("#visual-form_e >> \"Seats so far: 3\"").is_some());
    app.type_text("#visual-form_e >> role:textbox \"Add a risk\"", "Vendor late");
    app.frames(5);
    app.click("#visual-form_e >> role:button \"Add\"");
    app.click("#visual-form_e >> role:button \"Add row\"");
    app.wait_until(|app| app.find("#visual-form_e >> role:textbox \"Item 2\"").is_some());
    app.type_text("#visual-form_e >> role:textbox \"Item 2\"", "Desks");
    app.click("#visual-form_e >> role:checkbox \"Approved 2\"");
    app.click("#visual-form_e >> role:button \"Remove\"[1]");
    app.frames(10);
    let entries = crate::visuals::entries_of("v:fixture-form_e");
    let texts = |v: Option<&Value>| v.map(|v| v.items().iter().map(crate::auth::text).collect::<Vec<_>>());
    assert_eq!(entries.field("fit").map(crate::auth::text).as_deref(), Some("4"), "{entries:?}");
    assert_eq!(entries.field("seats").map(crate::auth::text).as_deref(), Some("3"), "{entries:?}");
    assert_eq!(texts(entries.field("risks")), Some(vec!["Vendor late".to_owned()]), "{entries:?}");
    let rows: Vec<Vec<String>> =
        entries.field("budget").expect("budget").items().iter().map(|r| texts(Some(r)).unwrap()).collect();
    assert_eq!(rows, [["Laptops", "true"], ["Desks", "true"]], "{entries:?}");
    assert_eq!(app.error(), None);
}

#[test]
fn a_failing_handler_is_contained_with_its_message() {
    let mut app = mount(Some("handler"));
    assert!(has(&mut app, "\"100 / 2 = 50\""));
    app.click("#visual-handler >> role:button \"Divide by nil\"");
    app.wait_until(|app| app.find("#visual-handler >> \"The visual stopped\"").is_some());
    assert!(has(&mut app, "#visual-handler >> \"The visual stopped\""), "{}", app.tree());
    // Nothing raw in the reply (the script's words are behind Copy error).
    assert!(!has(&mut app, "~\"attempt to\""), "the script's words shown:\n{}", app.tree());
    // The last good view stays.
    assert!(has(&mut app, "\"100 / 2 = 50\""));
    assert_eq!(app.error(), None);
}

#[test]
fn scenes_and_shaders_render_on_the_gpu_thread() {
    let mut app = mount(None);
    // A machine with no GPU adapter at all (a CI runner without a Vulkan
    // driver): a scene falls back to the software renderer and shows; a
    // shader can't, and says so in place, as a reader's would.
    if let Some(why) = zend_visual_gpu::unavailable_reason() {
        eprintln!("no GPU here ({why}): checking the software scene and the shader's notice");
        let error = "#visual-shader >> .visual-error";
        app.wait_until(|app| app.find(error).is_some());
        assert!(has(&mut app, error), "the shader without a GPU: no notice\n{}", app.tree());
        assert!(!has(&mut app, "#visual-scene >> .visual-error"), "the scene without a GPU\n{}", app.tree());
        assert!(has(&mut app, "#visual-scene >> img \"Materials\""), "the scene without a GPU: not drawn\n{}", app.tree());
        return;
    }
    let drawn = |id: &str| crate::visuals::surfaces(id);
    let ready = app.wait_until(|_| {
        let (shaders, _) = drawn("v:fixture-shader");
        let (_, scenes) = drawn("v:fixture-scene");
        shaders.iter().any(|(_, n)| *n >= 2) && !scenes.is_empty()
    });
    assert!(ready, "frames: shader {:?}, scene {:?}", drawn("v:fixture-shader"), drawn("v:fixture-scene"));
    assert!(!has(&mut app, "#visual-shader >> .visual-error"), "{}", app.tree());
    assert!(!has(&mut app, "#visual-scene >> .visual-error"), "{}", app.tree());
}

/// What the three tests above died of now and then on Linux (SIGSEGV, an
/// abort in malloc, a hang in `exit`): the test over, its process exited
/// with the GPU's thread still on a frame, and `exit` took the Vulkan
/// libraries' state from under it. A thread that used the GPU's thread
/// waits for it as it ends, as long as the frame takes, so nothing is on
/// the GPU by then. (Not on Windows, where a process exiting ends its other
/// threads first and only `stop_gpu` waits.)
#[cfg(unix)]
#[test]
fn a_thread_that_used_the_gpu_s_thread_ends_only_once_its_frame_has() {
    use std::sync::atomic::{AtomicBool, Ordering::SeqCst};
    use std::sync::{Arc, mpsc};
    let done = Arc::new(AtomicBool::new(false));
    let frame = done.clone();
    let user = std::thread::spawn(move || {
        let (started, on_it) = mpsc::channel();
        crate::visuals::gpu_probe(move || {
            let _ = started.send(());
            std::thread::sleep(std::time::Duration::from_millis(300));
            frame.store(true, SeqCst);
        });
        // The GPU's thread is on the frame as this thread (a test's, the
        // app's) ends.
        on_it.recv().expect("the GPU's thread took the frame");
    });
    user.join().expect("the thread ended");
    assert!(done.load(SeqCst), "the thread ended with the GPU's thread still on its frame");
}

/// (The GPU's thread is the process's one: this counts on a process of its
/// own, as nextest gives each test.)
#[test]
fn the_gpu_s_thread_stays_for_a_thread_still_using_it_and_starts_again_for_the_next() {
    // (In a process of its own: it needs the GPU's thread to itself, and
    // the other tests in this binary use it in parallel.)
    let (status, out, err) = in_a_process_of_its_own("visuals_tests::the_gpu_s_thread_stays_for_a_thread_still_using_it_and_starts_again_for_the_next_alone");
    assert_eq!(status, Some(0), "{out}\n{err}");
    assert!(out.contains("1 passed"), "{out}\n{err}");
}

/// (Run by the test above, in a process of its own.)
#[test]
#[ignore = "needs the GPU's thread to itself: run by the_gpu_s_thread_stays_for_a_thread_still_using_it_and_starts_again_for_the_next"]
fn the_gpu_s_thread_stays_for_a_thread_still_using_it_and_starts_again_for_the_next_alone() {
    use std::sync::mpsc;
    let probe = || {
        let (did, done) = mpsc::channel();
        crate::visuals::gpu_probe(move || {
            let _ = did.send(std::thread::current().id());
        });
        done.recv_timeout(std::time::Duration::from_secs(5)).expect("the GPU's thread did the work")
    };
    // This thread uses it; another comes and goes.
    let first = probe();
    std::thread::spawn(probe).join().expect("the other thread ended");
    assert!(crate::visuals::gpu_thread_runs(), "one thread ending stopped the GPU's thread under another");
    assert_eq!(probe(), first, "the same thread, its surfaces kept");
    // This one done with it (as the app is when it quits): it ends, and
    // starts again when next needed.
    assert!(crate::visuals::stop_gpu(), "its last user done with it, and it hasn't left");
    assert!(!crate::visuals::gpu_thread_runs());
    assert_ne!(probe(), first, "a new thread for the next frame");
    assert!(crate::visuals::stop_gpu());
}

/// A frame sent as the GPU's thread leaves (its last user gone, the frame
/// that user asked for still on it) goes to that thread, after the frame
/// it's on. A second GPU's thread is never started beside it.
#[test]
fn a_frame_sent_as_the_gpu_s_thread_leaves_is_drawn_by_it_after_the_frame_it_is_on() {
    // (In a process of its own: it needs the GPU's thread to itself, and
    // the other tests in this binary use it in parallel.)
    let (status, out, err) = in_a_process_of_its_own("visuals_tests::a_frame_sent_as_the_gpu_s_thread_leaves_is_drawn_by_it_after_the_frame_it_is_on_alone");
    assert_eq!(status, Some(0), "{out}\n{err}");
    assert!(out.contains("1 passed"), "{out}\n{err}");
}

/// (Run by the test above, in a process of its own.)
#[test]
#[ignore = "needs the GPU's thread to itself: run by a_frame_sent_as_the_gpu_s_thread_leaves_is_drawn_by_it_after_the_frame_it_is_on"]
fn a_frame_sent_as_the_gpu_s_thread_leaves_is_drawn_by_it_after_the_frame_it_is_on_alone() {
    use std::sync::mpsc;
    use std::time::{Duration, Instant};
    let (started, on_frame) = mpsc::channel();
    let (finish, may_finish) = mpsc::channel::<()>();
    let (go, may_go) = mpsc::channel::<()>();
    // One thread's frame, held, and that thread done with the GPU's thread
    // once the frame is on it.
    let first_user = std::thread::spawn(move || {
        crate::visuals::gpu_probe(move || {
            let _ = started.send(std::thread::current().id());
            let _ = may_finish.recv_timeout(Duration::from_secs(8));
        });
        let _ = may_go.recv_timeout(Duration::from_secs(5));
        crate::visuals::stop_gpu()
    });
    let first = on_frame.recv_timeout(Duration::from_secs(5)).expect("the GPU's thread took the frame");
    let _ = go.send(());
    let limit = Instant::now() + Duration::from_secs(5);
    while crate::visuals::gpu_thread_users() > 0 {
        assert!(Instant::now() < limit, "the first thread never let go of the GPU's thread");
        std::thread::yield_now();
    }
    // Another thread's frame now.
    let (drew, drawn) = mpsc::channel();
    let (leave, may_leave) = mpsc::channel::<()>();
    let next_user = std::thread::spawn(move || {
        crate::visuals::gpu_probe(move || {
            let _ = drew.send(std::thread::current().id());
        });
        let _ = may_leave.recv_timeout(Duration::from_secs(10));
        crate::visuals::stop_gpu()
    });
    let beside = drawn.recv_timeout(Duration::from_millis(500)).ok();
    let _ = finish.send(());
    let second = beside.or_else(|| drawn.recv_timeout(Duration::from_secs(5)).ok());
    let first_left = first_user.join().expect("the first thread ended");
    let _ = leave.send(());
    let next_left = next_user.join().expect("the next thread ended");
    assert_eq!(beside, None, "a second GPU's thread drew beside the one still on a frame");
    assert_eq!(second, Some(first), "the frame sent as it left wasn't drawn by it");
    assert!(!first_left, "stop_gpu said the GPU's thread had left, with another thread's frame on it");
    assert!(next_left, "the GPU's thread didn't leave once its last user had");
}

/// Quitting waits only so long for a frame. `stop_gpu` then says the GPU's
/// thread still runs (the process must not `exit` under it), and the GPU's
/// thread stays known: a later wait is for that frame, not for nothing.
#[test]
fn stop_gpu_says_the_gpu_s_thread_is_still_on_a_frame_and_a_later_one_waits_for_it() {
    // (In a process of its own: it needs the GPU's thread to itself, and
    // the other tests in this binary use it in parallel.)
    let (status, out, err) = in_a_process_of_its_own("visuals_tests::stop_gpu_says_the_gpu_s_thread_is_still_on_a_frame_and_a_later_one_waits_for_it_alone");
    assert_eq!(status, Some(0), "{out}\n{err}");
    assert!(out.contains("1 passed"), "{out}\n{err}");
}

/// (Run by the test above, in a process of its own.)
#[test]
#[ignore = "needs the GPU's thread to itself: run by stop_gpu_says_the_gpu_s_thread_is_still_on_a_frame_and_a_later_one_waits_for_it"]
fn stop_gpu_says_the_gpu_s_thread_is_still_on_a_frame_and_a_later_one_waits_for_it_alone() {
    use std::sync::atomic::{AtomicBool, Ordering::SeqCst};
    use std::sync::{Arc, mpsc};
    use std::time::Duration;
    let done = Arc::new(AtomicBool::new(false));
    let frame = done.clone();
    let (started, on_frame) = mpsc::channel();
    let (finish, may_finish) = mpsc::channel::<()>();
    crate::visuals::gpu_probe(move || {
        let _ = started.send(());
        let _ = may_finish.recv_timeout(Duration::from_secs(20));
        std::thread::sleep(Duration::from_millis(200));
        frame.store(true, SeqCst);
    });
    on_frame.recv_timeout(Duration::from_secs(5)).expect("the GPU's thread took the frame");
    let left = crate::visuals::stop_gpu_within(Duration::from_millis(300));
    assert!(!left, "stop_gpu said the GPU's thread had left, with its frame still running");
    assert!(crate::visuals::gpu_thread_runs(), "the GPU's thread, still on a frame, was forgotten");
    let _ = finish.send(());
    let left = crate::visuals::stop_gpu_within(Duration::from_secs(10));
    assert!(left, "a later stop_gpu didn't wait for the frame");
    assert!(done.load(SeqCst), "stop_gpu said the GPU's thread had left before its frame ended");
    assert!(!crate::visuals::gpu_thread_runs());
}

/// What an `exit` under a frame would have run (a library's destructors
/// run there), said on stderr.
const EXIT_RAN: &str = "exit ran under the frame";

/// The app quitting with a frame that outlasts the wait: the process ends
/// without `exit`, whose destructors would run under that frame, and with
/// what it had written flushed. (In a process of its own, since it ends
/// it: the test below, run by this one.)
#[test]
fn quitting_with_the_gpu_s_thread_still_on_a_frame_ends_the_process_without_exit() {
    let (status, out, err) = in_a_process_of_its_own("visuals_tests::quits_with_the_gpu_s_thread_still_on_a_frame");
    assert!(!err.contains(EXIT_RAN), "the process went through exit under the frame:\n{err}");
    assert_eq!(status, Some(0), "{err}");
    assert!(out.contains("quitting under a frame"), "what it had written wasn't flushed: {out:?}");
    assert!(err.contains("still on a frame"), "{err}");
}

/// A test run in a process of its own (one that ends its process, or needs
/// the GPU's thread to itself): its exit status, and what it wrote to stdout and stderr. (Each
/// holds a frame 30 s: a process that waits it out is stopped at 20 s.)
fn in_a_process_of_its_own(test: &str) -> (Option<i32>, String, String) {
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};
    let exe = std::env::current_exe().expect("this test's binary");
    let mut process = Command::new(exe)
        .args(["--exact", test, "--ignored", "--nocapture"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the process started");
    let limit = Instant::now() + Duration::from_secs(20);
    while process.try_wait().expect("the process").is_none() {
        if Instant::now() >= limit {
            let _ = process.kill();
            panic!("the process was still waiting for the frame after 20 s");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let output = process.wait_with_output().expect("the process's output");
    let text = |bytes: &[u8]| String::from_utf8_lossy(bytes).into_owned();
    (output.status.code(), text(&output.stdout), text(&output.stderr))
}

/// What an `exit` would run, as a library's destructors are, says so.
fn say_if_exit_runs() {
    extern "C" fn at_exit() {
        eprintln!("{EXIT_RAN}");
    }
    // SAFETY: `at_exit` is a plain function that lives as long as the process.
    unsafe { libc::atexit(at_exit) };
}

/// The GPU's thread on a frame that outlasts every wait here.
fn hold_a_frame() {
    let (started, on_frame) = std::sync::mpsc::channel();
    crate::visuals::gpu_probe(move || {
        let _ = started.send(());
        std::thread::sleep(std::time::Duration::from_secs(30));
    });
    on_frame.recv_timeout(std::time::Duration::from_secs(5)).expect("the GPU's thread took the frame");
}

/// A panic out of `main` with a frame that outlasts the wait: the process
/// ends as it does quitting, without `exit` and without waiting the frame
/// out, with a panic's exit status.
#[test]
fn a_panic_with_the_gpu_s_thread_still_on_a_frame_ends_the_process_without_exit() {
    let (status, _, err) = in_a_process_of_its_own("visuals_tests::panics_with_the_gpu_s_thread_still_on_a_frame");
    assert!(!err.contains(EXIT_RAN), "the process went through exit under the frame:\n{err}");
    assert_eq!(status, Some(101), "{err}");
    assert!(err.contains("a panic out of main"), "{err}");
    assert!(err.contains("still on a frame"), "{err}");
}

/// (Run by the test above, in a process of its own: it ends the process.)
#[test]
#[ignore = "ends its process: run by a_panic_with_the_gpu_s_thread_still_on_a_frame_ends_the_process_without_exit"]
fn panics_with_the_gpu_s_thread_still_on_a_frame() {
    say_if_exit_runs();
    // As `main` holds it (src/main.rs).
    let _stop_gpu = crate::visuals::StopGpuOnPanic::within(std::time::Duration::from_millis(300));
    hold_a_frame();
    panic!("a panic out of main");
}

/// (Run by the test above, in a process of its own: it ends the process.)
#[test]
#[ignore = "ends its process: run by quitting_with_the_gpu_s_thread_still_on_a_frame_ends_the_process_without_exit"]
fn quits_with_the_gpu_s_thread_still_on_a_frame() {
    use std::time::Duration;
    say_if_exit_runs();
    hold_a_frame();
    // (No new line: written out only if the process's end flushes it.)
    print!("quitting under a frame");
    // As `main` quits (src/main.rs).
    if !crate::visuals::stop_gpu_within(Duration::from_millis(300)) {
        crate::visuals::exit_under_gpu(0);
    }
}

const COUNTER: &str = r#"
local count = 0
local scale = math.random(1, 1000000)
return function()
    return ui.column {
        ui.text("count " .. count .. " of " .. scale),
        ui.button { label = "Add", on_click = function() count += 1 end },
    }
end
"#;

#[test]
fn a_visual_at_rest_lets_its_vm_go_and_shows_the_same_thing() {
    crate::visuals::preload("rest-same", "Rest", COUNTER);
    let shown = crate::visuals::snapshot("rest-same");
    let versions = crate::visuals::versions("rest-same");
    assert!(crate::visuals::awake("rest-same"));
    crate::visuals::rest();
    assert!(!crate::visuals::awake("rest-same"), "nothing here to lose");
    assert_eq!(crate::visuals::snapshot("rest-same"), shown);
    assert_eq!(crate::visuals::versions("rest-same"), versions, "nothing rebuilds");
}

#[test]
fn a_control_used_after_rest_acts_on_the_same_visual_and_keeps_its_vm() {
    crate::visuals::preload("rest-control", "Rest", COUNTER);
    let scale = |shown: &zgx_script::Value| {
        let text = format!("{shown:?}");
        text.split("count ").nth(1).and_then(|s| s.split('"').next()).map(str::to_string).expect("its text")
    };
    let first = scale(&crate::visuals::snapshot("rest-control"));
    assert!(first.starts_with("0 of "), "{first}");
    crate::visuals::rest();
    // (Handler 0 is the button: the same number in the tree it restarted to.)
    crate::visuals::event("rest-control", 0);
    let second = scale(&crate::visuals::snapshot("rest-control"));
    assert_eq!(second, first.replacen('0', "1", 1), "the same random data, one click on");
    // It has state its source doesn't give now: rest keeps it.
    crate::visuals::rest();
    assert!(crate::visuals::awake("rest-control"));
    crate::visuals::event("rest-control", 0);
    assert!(scale(&crate::visuals::snapshot("rest-control")).starts_with("2 of "));
}

fn canvas_arg(pointer: bool) -> zgx_script::Value {
    use zgx_script::Value;
    let mut fields = vec![("h", Value::Num(0.)), ("w", Value::Num(100.)), ("ht", Value::Num(50.)), ("t", Value::Num(0.))];
    if pointer {
        fields.extend([("px", Value::Num(1.)), ("py", Value::Num(1.))]);
    }
    Value::record(fields)
}

#[test]
fn a_still_canvas_draws_the_same_after_rest_without_rebuilding() {
    let id = "rest-canvas";
    crate::visuals::preload(
        id,
        "Rest",
        r#"
local dots = {}
for i = 1, 8 do dots[i] = math.random(0, 100) end
return ui.canvas { height = 50, draw = function(g, w, h)
    for i, x in dots do g:circle(x, i, 1) end
end }
"#,
    );
    let before = crate::visuals::draw(id, canvas_arg(false));
    let versions = crate::visuals::versions(id);
    crate::visuals::rest();
    assert!(!crate::visuals::awake(id));
    assert_eq!(crate::visuals::draw(id, canvas_arg(false)), before);
    assert_eq!(crate::visuals::versions(id), versions, "waking draws nothing again by itself");
    // Drawn for the pointer, it may hold what the pointer did.
    crate::visuals::draw(id, canvas_arg(true));
    crate::visuals::rest();
    assert!(crate::visuals::awake(id));
}

#[test]
fn what_draws_over_time_keeps_its_vm_but_a_shader_needs_none() {
    let id = "rest-animated";
    crate::visuals::preload(
        id,
        "Rest",
        r#"
local n = 0
return ui.canvas { height = 50, animate = true, draw = function(g) n += 1 end }
"#,
    );
    crate::visuals::rest();
    assert!(crate::visuals::awake(id), "its script may carry on from the last draw");

    let id = "rest-shader";
    crate::visuals::preload(
        id,
        "Rest",
        r#"
return ui.shader { height = 50, code = "void mainImage(out vec4 c, in vec2 p) { c = vec4(1.0); }" }
"#,
    );
    assert_eq!(crate::visuals::snapshot(id).field("error"), None);
    crate::visuals::rest();
    assert!(!crate::visuals::awake(id), "a shader never runs the script");
}

#[test]
fn a_visual_that_failed_keeps_what_it_has() {
    let id = "rest-failed";
    crate::visuals::preload(id, "Rest", "return function() error('no') end");
    crate::visuals::rest();
    assert!(crate::visuals::awake(id), "its message stays, and so does the VM that gave it");
}

#[test]
fn a_visual_being_read_says_so() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("ui/visuals_waiting_fixture.luau");
    let mut app = TestApp::new(|runtime| {
        let host = ScriptHost::new(runtime, path);
        crate::visuals::register(&host);
        host
    });
    app.ui.scene.borrow_mut().resize(800., 600.);
    app.frames(20);
    assert_eq!(app.error(), None, "{}", app.tree());
    assert!(has(&mut app, "\"Loading Latency…\""), "{}", app.tree());
}

/// Every visual script in `VISUALS_DIR` (`*-visual.luau`, as the engine
/// stores them) drawn as a reply draws it, with no script error and none
/// saying it stopped: for checking a chat's worth of real visuals. Run on
/// purpose:
/// `VISUALS_DIR=… cargo test --bin zend-zgx visuals_tests::scripts_in -- --ignored --nocapture`
#[test]
#[ignore = "run on purpose, on a folder of visual scripts"]
fn scripts_in_a_folder_draw_without_errors() {
    use zgx_script::Value;
    let dir = std::path::PathBuf::from(std::env::var("VISUALS_DIR").expect("VISUALS_DIR"));
    let mut paths: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.path()).collect();
    paths.retain(|p| p.to_string_lossy().ends_with("-visual.luau"));
    paths.sort();
    let sources: Vec<Value> = paths
        .iter()
        .map(|path| {
            let name = path.file_name().unwrap().to_string_lossy();
            let id = format!("v:{}", name.trim_end_matches("-visual.luau").trim_start_matches("v_"));
            Value::record([
                ("id", Value::from(id.as_str())),
                ("source", Value::from(std::fs::read_to_string(path).unwrap().as_str())),
            ])
        })
        .collect();
    let fixture = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("ui/visuals_folder_fixture.luau");
    let mut app = TestApp::new(|runtime| {
        let host = ScriptHost::new(runtime, fixture);
        host.shared("visual_sources", Value::List(sources.into()));
        crate::visuals::register(&host);
        host
    });
    app.ui.scene.borrow_mut().resize(800., 12000.);
    let end = std::time::Instant::now() + std::time::Duration::from_millis(1500);
    while std::time::Instant::now() < end {
        app.frames(2);
    }
    let mut faults = Vec::new();
    for (i, path) in paths.iter().enumerate() {
        let at = format!("#visual-{}", i + 1);
        let stopped = app.find(&format!("{at} >> \"Couldn't show this visual\"")).is_some()
            || app.find(&format!("{at} >> \"The visual stopped\"")).is_some();
        let line = if stopped { "FAULT" } else { "ok   " };
        println!("{line} {}", path.display());
        if stopped {
            faults.push(path.display().to_string());
        }
    }
    let error = app.error();
    // (`VISUALS_SHOT=<file.png>`: a picture of them all, dark, as
    // src/audit_shots.rs draws one.)
    if let Ok(out) = std::env::var("VISUALS_SHOT") {
        shoot(&mut app, std::path::Path::new(&out));
    }
    assert!(faults.is_empty() && error.is_none(), "{faults:?} {error:?}\n{}", app.tree());
}

/// The window drawn by zgx's GPU renderer, its height cut to what shows,
/// written as a PNG over a dark backdrop.
fn shoot(app: &mut TestApp, out: &std::path::Path) {
    let w = 800f32;
    let h = app.bounds("reply").height.ceil().clamp(100., 12000.);
    // (Within the GPU's largest texture.)
    let scale = (8000. / h).min(1.5);
    app.ui.scene.borrow_mut().resize(w, h);
    let (pw, ph) = ((w * scale) as u32, (h * scale) as u32);
    let mut gpu = zgx_gpu::GpuRenderer::new(pw, ph).expect("a GPU");
    gpu.set_scale_factor(scale);
    for font in crate::fonts() {
        font.unwrap().install(&mut gpu.text_system().borrow_mut());
    }
    gpu.install_text(&mut app.ui.scene.borrow_mut());
    app.ui.refresh_text_geometry();
    let end = std::time::Instant::now() + std::time::Duration::from_millis(800);
    while std::time::Instant::now() < end {
        app.frames(2);
    }
    app.ui.prepare_frame();
    let _ = app.ui.scene.borrow_mut().flush();
    let all = [zgx::scene::Rect { x: 0., y: 0., width: w, height: h }];
    gpu.render(&app.ui.scene.borrow(), &all).expect("render");
    let mut rgba = gpu.readback().expect("readback");
    for px in rgba.chunks_exact_mut(4) {
        let a = px[3] as u32;
        for (c, back) in [0x1du32, 0x1d, 0x22].into_iter().enumerate() {
            px[c] = (px[c] as u32 + back * (255 - a) / 255).min(255) as u8;
        }
        px[3] = 255;
    }
    let file = std::fs::File::create(out).unwrap();
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), pw, ph);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header().unwrap().write_image_data(&rgba).unwrap();
}

/// What each visual in `VISUALS_DIR` costs, alone in a window drawn by
/// zgx's GPU renderer every frame: idle (1.5 s of real time, its own
/// animation running if it has one) and with the pointer sweeping across
/// it. Prints ms of this thread a frame and script calls. Run on purpose:
/// `VISUALS_DIR=… cargo test --bin zend-zgx visuals_tests::cost -- --ignored --nocapture`
#[test]
#[ignore = "run on purpose: a cost probe"]
fn cost_of_each_visual() {
    use std::time::{Duration, Instant};
    use zgx_script::Value;
    let dir = std::path::PathBuf::from(std::env::var("VISUALS_DIR").expect("VISUALS_DIR"));
    let mut paths: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.path()).collect();
    paths.retain(|p| p.to_string_lossy().ends_with("-visual.luau"));
    paths.sort();
    let fixture = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("ui/visuals_folder_fixture.luau");
    for path in paths {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let id = format!("v:cost-{}", &name[2..10]);
        let source = std::fs::read_to_string(&path).unwrap();
        let sources = vec![Value::record([("id", Value::from(id.as_str())), ("source", Value::from(source.as_str()))])];
        let mut app = TestApp::new(|runtime| {
            let host = ScriptHost::new(runtime, fixture.clone());
            host.shared("visual_sources", Value::List(sources.into()));
            crate::visuals::register(&host);
            host
        });
        let (w, h) = (800f32, 700f32);
        app.ui.scene.borrow_mut().resize(w, h);
        let mut gpu = zgx_gpu::GpuRenderer::new((w * 2.) as u32, (h * 2.) as u32).expect("a GPU");
        gpu.set_scale_factor(2.);
        for font in crate::fonts() {
            font.unwrap().install(&mut gpu.text_system().borrow_mut());
        }
        gpu.install_text(&mut app.ui.scene.borrow_mut());
        app.ui.refresh_text_geometry();
        let all = [zgx::scene::Rect { x: 0., y: 0., width: w, height: h }];
        let render_ns = std::cell::Cell::new(0u128);
        let mut draw = |app: &mut TestApp| -> Duration {
            let start = Instant::now();
            app.frame();
            app.ui.prepare_frame();
            let dirty = app.ui.scene.borrow_mut().flush();
            let _ = dirty;
            let r = Instant::now();
            gpu.render(&app.ui.scene.borrow(), &all).expect("render");
            render_ns.set(render_ns.get() + r.elapsed().as_nanos());
            start.elapsed()
        };
        // Settled: read, rendered, first frames in.
        let end = Instant::now() + Duration::from_millis(600);
        while Instant::now() < end {
            draw(&mut app);
        }
        let measure = |app: &mut TestApp, draw: &mut dyn FnMut(&mut TestApp) -> Duration, act: &mut dyn FnMut(&mut TestApp, usize)| {
            let calls = app.calls();
            let (draw0, gpu0, gframes0) = crate::visuals::costs();
            let (gcpu0, soft0) = crate::visuals::gpu_thread_costs();
            let (mut busy, mut frames) = (Duration::ZERO, 0usize);
            let end = Instant::now() + Duration::from_millis(1500);
            while Instant::now() < end {
                act(app, frames);
                busy += draw(app);
                frames += 1;
                std::thread::sleep(Duration::from_millis(16));
            }
            let (draw1, gpu1, gframes1) = crate::visuals::costs();
            let (gcpu1, soft1) = crate::visuals::gpu_thread_costs();
            if gframes1 > gframes0 {
                println!(
                    "    GPU thread CPU {:.2} ms/frame, {} of {} frames by the software renderer",
                    (gcpu1 - gcpu0) as f64 / 1e6 / (gframes1 - gframes0) as f64,
                    soft1 - soft0,
                    gframes1 - gframes0
                );
            }
            let secs = 1.5;
            println!(
                "    {frames} frames: main {:.2} ms/frame ({:.2} of it canvas draws), {:.1} script calls/frame; GPU thread {:.1} frames/s at {:.2} ms each",
                busy.as_secs_f64() * 1000. / frames as f64,
                (draw1 - draw0) as f64 / 1e6 / frames as f64,
                (app.calls() - calls) as f64 / frames as f64,
                (gframes1 - gframes0) as f64 / secs,
                if gframes1 > gframes0 { (gpu1 - gpu0) as f64 / 1e6 / (gframes1 - gframes0) as f64 } else { 0. },
            );
            (busy.as_secs_f64() * 1000. / frames as f64, (app.calls() - calls) as f64 / frames as f64)
        };
        println!("{name} (idle, then the pointer sweeping across it; render = zgx's renderer, rasterizing included)");
        render_ns.set(0);
        let (idle_ms, idle_calls) = measure(&mut app, &mut draw, &mut |_, _| {});
        println!("    of which render {:.2} ms/frame", render_ns.get() as f64 / 1e6 / (1500. / 17.));
        let (hover_ms, hover_calls) = measure(&mut app, &mut draw, &mut |app, i| {
            let x = 40. + (i % 60) as f32 * 12.;
            zgx_script::driver::hover_at(&mut app.ui, (x, 300.));
        });
        println!(
            "{name:<55} idle {idle_ms:6.2} ms/frame {idle_calls:6.1} calls/frame | hover {hover_ms:6.2} ms/frame {hover_calls:6.1} calls/frame"
        );
    }
}

/// How long crates/visual-gpu takes for one frame of a shader and a scene at
/// a few sizes, warm: where a frame's time goes. Run on purpose:
/// `cargo test --bin zend-zgx visuals_tests::gpu_frame_bench -- --ignored --nocapture`
#[test]
#[ignore = "run on purpose: a benchmark"]
fn gpu_frame_bench() {
    use std::time::Instant;
    use zend_visual as zv;
    println!("GPU: available {} {:?}", zend_visual_gpu::available(), zend_visual_gpu::unavailable_reason());
    let code = "void mainImage(out vec4 c, in vec2 p) { vec2 uv = p / iResolution.xy; c = vec4(uv, 0.5 + 0.5 * sin(iTime), 1.0); }";
    for (w, h, bloom) in [(800u32, 300u32, 0.0f32), (1600, 600, 0.0), (1600, 600, 0.8), (800, 300, 0.8)] {
        let spec = zend_visual_gpu::ShaderSpec { image: code, buffers: &[], inputs: &[], blend: zv::shader::Blend::Light, bloom, textures: &[] };
        let mut state = None;
        let mut uniforms = zv::shader::ShaderUniforms { resolution: [w as f32, h as f32, 2., 0.], ..Default::default() };
        for _ in 0..3 {
            zend_visual_gpu::render_shader(&spec, &uniforms, w, h, &mut state).unwrap();
        }
        let start = Instant::now();
        for i in 0..20 {
            uniforms.time[0] = i as f32 * 0.03;
            zend_visual_gpu::render_shader(&spec, &uniforms, w, h, &mut state).unwrap();
        }
        println!("shader {w}x{h} bloom {bloom}: {:.2} ms/frame", start.elapsed().as_secs_f64() * 50.);
    }
    let mut visual = zv::Visual::load(
        "return ui.scene { height = 340, draw = function(s) s:box { at = {0,0,0}, size = 1, color = 'accent' } s:sphere { at = {1,0.5,0}, radius = 0.5, color = 'chart2', metal = 1 } end }",
        1,
    )
    .unwrap();
    let tree = visual.render().unwrap();
    let zv::NodeKind::Scene(node) = tree.kind else { panic!() };
    let scene = visual.draw_scene(node.draw, 0.).unwrap();
    let rgba = |_: zv::Color| [0.5f32, 0.5, 0.5, 1.];
    for (w, h, scale) in [(800f32, 340f32, 1f32), (800., 340., 2.)] {
        let mut state = None;
        let camera = zv::Camera { yaw: 0.5, pitch: 0.4, zoom: 1. };
        for _ in 0..3 {
            zend_visual_gpu::render_scene(&scene, camera, w, h, scale, &rgba, &mut state).expect("GPU scene");
        }
        let start = Instant::now();
        for i in 0..20 {
            let camera = zv::Camera { yaw: 0.5 + i as f32 * 0.02, pitch: 0.4, zoom: 1. };
            zend_visual_gpu::render_scene(&scene, camera, w, h, scale, &rgba, &mut state).expect("GPU scene");
        }
        println!("scene {w}x{h} @{scale}x: {:.2} ms/frame", start.elapsed().as_secs_f64() * 50.);
    }
}

/// The BGRA→RGBA swap of one 2x frame (1600 by 680), as built here.
#[test]
#[ignore = "run on purpose: a benchmark"]
fn swap_bench() {
    let bgra = vec![7u8; 1600 * 680 * 4];
    let start = std::time::Instant::now();
    for _ in 0..10 {
        std::hint::black_box(crate::visuals::bench_swap(bgra.clone()));
    }
    println!("swap: {:.2} ms/frame (with the clone)", start.elapsed().as_secs_f64() * 100.);
}

#[test]
fn a_visual_scrolled_away_does_not_animate() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("ui/visuals_fixture.luau");
    let mut app = TestApp::new(|runtime| {
        let host = ScriptHost::new(runtime, path).args(["only=canvas".to_string()]);
        crate::visuals::register(&host);
        host
    });
    // A window too short to show the canvas (under the visual's title).
    app.ui.scene.borrow_mut().resize(800., 60.);
    app.wait_until(|app| app.find(".visual-canvas").is_some());
    // Script calls in half a second of real time (a tick, a draw: each one).
    let draws = |app: &mut TestApp| {
        app.frames(5);
        let before = app.calls();
        let end = std::time::Instant::now() + std::time::Duration::from_millis(500);
        while std::time::Instant::now() < end {
            app.frames(1);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        app.calls() - before
    };
    // (Its first drawing, as it lays out, then nothing.)
    let _first = draws(&mut app);
    let hidden = draws(&mut app);
    app.ui.scene.borrow_mut().resize(800., 800.);
    app.frames(5);
    let shown = draws(&mut app);
    assert_eq!(hidden, 0, "scrolled away, it ran the script");
    assert!(shown > 10, "in view, it animates ({shown} calls)");
}

/// Every visual in `VISUALS_DIR` mounted together in an 800 by 700 window
/// (a long reply: a few show, the rest scrolled away), scrolled from top to
/// bottom and back over 3 s, drawn by zgx's GPU renderer each frame: this
/// thread's ms a frame and the GPU thread's frames. Run on purpose:
/// `VISUALS_DIR=… cargo test --bin zend-zgx visuals_tests::cost_together -- --ignored --nocapture`
#[test]
#[ignore = "run on purpose: a cost probe"]
fn cost_together() {
    use std::time::{Duration, Instant};
    use zgx_script::Value;
    let dir = std::path::PathBuf::from(std::env::var("VISUALS_DIR").expect("VISUALS_DIR"));
    let mut paths: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.path()).collect();
    paths.retain(|p| p.to_string_lossy().ends_with("-visual.luau"));
    paths.sort();
    let sources: Vec<Value> = paths
        .iter()
        .enumerate()
        .map(|(i, p)| {
            Value::record([
                ("id", Value::from(format!("v:together-{i}").as_str())),
                ("source", Value::from(std::fs::read_to_string(p).unwrap().as_str())),
            ])
        })
        .collect();
    let fixture = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("ui/visuals_folder_fixture.luau");
    let mut app = TestApp::new(|runtime| {
        let host = ScriptHost::new(runtime, fixture);
        host.shared("visual_sources", Value::List(sources.into()));
        crate::visuals::register(&host);
        host
    });
    let (w, h) = (800f32, 700f32);
    app.ui.scene.borrow_mut().resize(w, h);
    let mut gpu = zgx_gpu::GpuRenderer::new((w * 2.) as u32, (h * 2.) as u32).expect("a GPU");
    gpu.set_scale_factor(2.);
    for font in crate::fonts() {
        font.unwrap().install(&mut gpu.text_system().borrow_mut());
    }
    gpu.install_text(&mut app.ui.scene.borrow_mut());
    app.ui.refresh_text_geometry();
    let all = [zgx::scene::Rect { x: 0., y: 0., width: w, height: h }];
    let end = Instant::now() + Duration::from_millis(800);
    while Instant::now() < end {
        app.frame();
        app.ui.prepare_frame();
        let _ = app.ui.scene.borrow_mut().flush();
        gpu.render(&app.ui.scene.borrow(), &all).unwrap();
    }
    let (_, gpu0, frames0) = crate::visuals::costs();
    let (gcpu0, _) = crate::visuals::gpu_thread_costs();
    let (mut busy, mut frames) = (Duration::ZERO, 0usize);
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(3) {
        let t = Instant::now();
        app.frame();
        app.ui.prepare_frame();
        let _ = app.ui.scene.borrow_mut().flush();
        gpu.render(&app.ui.scene.borrow(), &all).unwrap();
        busy += t.elapsed();
        frames += 1;
        std::thread::sleep(Duration::from_millis(16));
    }
    let (_, gpu1, frames1) = crate::visuals::costs();
    let (gcpu1, _) = crate::visuals::gpu_thread_costs();
    println!(
        "{} visuals, top of the reply in view: main {:.2} ms/frame over {frames} frames ({:.0}% of a core at 60 fps); GPU thread {:.1} frames/s, {:.2} ms CPU each ({:.0}% of a core), {:.2} ms wall each",
        paths.len(),
        busy.as_secs_f64() * 1000. / frames as f64,
        busy.as_secs_f64() * 1000. / frames as f64 * 6.,
        (frames1 - frames0) as f64 / 3.,
        if frames1 > frames0 { (gcpu1 - gcpu0) as f64 / 1e6 / (frames1 - frames0) as f64 } else { 0. },
        (gcpu1 - gcpu0) as f64 / 1e7 / 3.,
        if frames1 > frames0 { (gpu1 - gpu0) as f64 / 1e6 / (frames1 - frames0) as f64 } else { 0. },
    );
}

/// A reply as the chat bridge lays it out (`crate::chat::message`) drawn by
/// ds/transcript, in ui/display_tools_fixture.luau, its visual already read.
fn transcript_of(entry: serde_json::Value) -> TestApp {
    let shown = crate::chat::message(&entry, None).expect("a reply shows");
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("ui/display_tools_fixture.luau");
    let mut app = TestApp::new(|runtime| {
        let host = ScriptHost::new(runtime, path);
        host.shared("transcript_messages", zgx_script::Value::List(vec![shown].into()));
        crate::visuals::register(&host);
        host
    });
    app.ui.scene.borrow_mut().resize(900., 1400.);
    app.frames(40);
    assert_eq!(app.error(), None, "{}", app.tree());
    app
}

/// The agent updates a visual (`update_visual`): its part's rev grows, and
/// the window holding it reads the script again and swaps it in, what the
/// reader entered kept by input name (plans/live-visuals G3/G4). Its part
/// saying it was sent, it asks no more.
#[test]
fn a_part_whose_rev_grew_is_read_again_and_keeps_the_entries() {
    use serde_json::json;
    const PATH: &str = "/u/v_reread-visual.luau";
    crate::visuals::serve_file(PATH, Some(r#"return function() return ui.column { ui.text "First version", ui.field { name = "goal", label = "Goal" } } end"#));
    crate::visuals::serve_file("/u/v_reread-entries.json", None);
    crate::visuals::serve_file("/u/v_reread-assets.json", None);
    let part = |rev: u64, sent: bool| json!({ "kind": "visual", "id": "v:reread", "path": PATH, "title": "Plan", "rev": rev, "sent": sent });
    let entry = |rev: u64, sent: bool| json!({ "id": "a1", "role": "assistant", "status": "complete", "createdAt": 0, "parts": [ part(rev, sent) ] });
    let mut app = transcript_of(entry(0, false));
    app.wait_until(|app| app.find("\"First version\"").is_some());
    app.type_text("role:textbox \"Goal\"", "ship it");
    app.wait_until(|_| crate::visuals::entries_of("v:reread").field("goal").is_some());

    crate::visuals::serve_file(PATH, Some(r#"return function() return ui.column { ui.text "Second version", ui.field { name = "goal", label = "Goal" }, ui.check { name = "backup", label = "Back up" } } end"#));
    // (The transcript reads the part again as the host sends it: rev 1.)
    let _ = crate::chat::message(&entry(1, false), None);
    let reread = app.wait_until(|app| app.find("\"Second version\"").is_some());
    assert!(reread, "not read again:\n{}", app.tree());
    assert!(app.find("role:checkbox \"Back up\"").is_some());
    assert_eq!(
        crate::visuals::entries_of("v:reread").field("goal").map(crate::auth::text).as_deref(),
        Some("ship it"),
        "the reader's entry kept"
    );
    // The same rev again: nothing read (it stays as it is).
    let _ = crate::chat::message(&entry(1, true), None);
    app.frames(10);
    assert!(app.find("\"Second version\"").is_some());
    assert_eq!(app.error(), None);
}

#[test]
fn a_show_visual_call_leaves_no_tool_row_and_its_visual_sits_in_place() {
    use serde_json::json;
    crate::visuals::preload(
        "v:in-place",
        "Signups",
        r#"return ui.column { ui.stat { value = "1,284", label = "Signups this week" } }"#,
    );
    let mcp = |id: &str, tool: &str, error: bool, output: &str| {
        json!({ "kind": "tool", "id": id, "resolved": true, "isError": error, "output": output,
                "call": { "kind": "mcp", "server": "zend", "tool": tool, "input": {} } })
    };
    let exec = |id: &str| {
        json!({ "kind": "tool", "id": id, "resolved": true, "isError": false,
                "call": { "kind": "exec", "command": "ls" } })
    };
    let entry = json!({ "id": "a1", "role": "assistant", "status": "complete", "createdAt": 0, "parts": [
        { "kind": "text", "id": "t1", "text": "Counting them up." },
        exec("x1"),
        mcp("s1", "show_visual", false, "shown"),
        { "kind": "visual", "id": "v:in-place", "path": "/u/v_in-place-visual.luau", "title": "Signups" },
        exec("x2"),
        mcp("s2", "show_visual", true, "visual script failed: visual:2: attempt to index nil"),
        { "kind": "text", "id": "t2", "text": "That's the week." },
    ]});
    let mut app = transcript_of(entry);
    let tree = app.tree();
    // No row, no chevron, no count for either call.
    assert!(!tree.contains("show_visual"), "a tool row for it:\n{tree}");
    assert!(!tree.contains("called"), "counted in a group's line:\n{tree}");
    // The visual where the call was: after the words and the command before
    // it, before the command after it.
    let y = |app: &mut TestApp, target: &str| app.bounds(target).y;
    let (before, visual, after) = (
        y(&mut app, "\"Counting them up.\""),
        y(&mut app, "\"Signups this week\""),
        y(&mut app, "\"That's the week.\""),
    );
    assert!(before < visual && visual < after, "{before} {visual} {after}\n{tree}");
    // The failed one: only a quiet line in plain words (ds/reply_picture's
    // `Line`), never the script's own error text.
    assert!(app.find("\"Couldn't show this visual\"").is_some(), "{tree}");
    assert!(!tree.contains("attempt to index"), "the script's error in the reply:\n{tree}");
    assert_eq!(tree.matches("Ran 1 command").count(), 2, "each command its own group, split by the visual:\n{tree}");
}
