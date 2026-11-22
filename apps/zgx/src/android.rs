//! Zend on Android: the same app as the desktop's, in a NativeActivity
//! (apps/zgx/android, the APK's library, calls [`main`] from
//! `android_main`). No Kotlin and no Java of its own.
//!
//! - **Files**: the scripts (`ui/`) and Zend's own plugins
//!   (`plugins-builtin/`, each with the SDK's zend.luau) are packed into
//!   the library at build time ([`PACK`], apps/zgx/android/build.rs) and
//!   written out to the app's files folder at start, where [`ui_dir`] finds
//!   them as the desktop finds its packaged copy. The fonts are in the
//!   binary already (lib.rs `fonts()`).
//! - **Home**: `HOME` and `XDG_DATA_HOME` are the app's files folder
//!   (`internalDataPath`), `TMPDIR` its cache: what the window keeps under
//!   home (its saved state, the computer it is linked to) is the app's own,
//!   kept across starts and removed with the app.
//! - **No engine**: the phone is a client of a computer's (plans/client-server
//!   step 1). Unlinked, it shows only Connect to a computer; linked (a v=2
//!   code kept, src/phone_link.rs), every call goes to that computer's engine
//!   over its phone link (src/engine/rpc.rs). No account, no harness here.
//! - **Insets**: the status, navigation and keyboard bars over the window
//!   (Android draws the app under them), for the interface as
//!   `ui.shared("system.insets")` (ui/ds/insets.luau): `{ top, right,
//!   bottom, left, keyboard }` in points.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use zgx_desktop::WindowContext;
use zgx_desktop::android::AndroidApp;
use zgx_script::{ScriptHost, Value};

/// The packed files, as apps/zgx/android/build.rs lays them out: for each,
/// its path's length (u32 LE), its path under the app folder, its length,
/// its bytes. Set by the APK's library before [`main`].
static PACK: OnceLock<&'static [u8]> = OnceLock::new();

/// Where the files were written: `<files>/app`.
static APP_DIR: OnceLock<PathBuf> = OnceLock::new();

/// The app's scripts on this device, once unpacked.
pub fn ui_dir() -> Option<PathBuf> {
    APP_DIR.get().map(|dir| dir.join("ui"))
}

/// The app, from the activity `android_main` was handed, with the files
/// its library packs (`pack`), until its activity is destroyed.
///
/// Android runs this once per activity, and a process can see several, one
/// after the other (a rotation, coming back after Back left the app): what
/// the process sets up (the log, the files, the environment) is done once,
/// the engine keeps running across them, and each activity gets its own
/// window and event loop (zgx-desktop's `android::set_app`).
pub fn main(app: AndroidApp, pack: &'static [u8]) {
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("zend"),
    );
    static PROCESS: OnceLock<Result<PathBuf, String>> = OnceLock::new();
    let files = match PROCESS.get_or_init(|| set_up_process(&app, pack)) {
        Ok(files) => files,
        // (Returning ends the activity: nothing runs without its files.)
        Err(error) => return log::error!("zend: {error}"),
    };
    ask_notifications(&app);
    zgx_desktop::android::set_app(app);
    log::info!("zend: activity starting ({})", files.display());
    // A check of the banners without a session to finish (G5's emulator
    // run: `adb shell run-as sh.zend.zgx touch files/notify-check`): one
    // banner, the file gone after.
    let check = files.join("notify-check");
    if std::fs::remove_file(&check).is_ok() {
        log::info!("zend: notifications: posting a check banner");
        zend_notify::post("Zend", "Notifications reach this device.", None);
    }
    if let Err(error) = crate::launch(crate::Screen::Intro, false, false, None) {
        log::error!("zend: {error}");
    }
    log::info!("zend: activity ended");
}

/// Once per process: the log, the packed files unpacked, the environment.
/// The app's private files folder or nothing: without it (`internalDataPath`
/// missing or not a folder of its own) it doesn't start rather than keep
/// its engine's data and keys somewhere shared.
fn set_up_process(app: &AndroidApp, pack: &'static [u8]) -> Result<PathBuf, String> {
    std::panic::set_hook(Box::new(|info| log::error!("panic: {info}")));
    stderr_to_log();
    let _ = PACK.set(pack);
    let files = app.internal_data_path().ok_or("no private files folder (internalDataPath): not starting")?;
    let private = std::fs::metadata(&files).is_ok_and(|meta| {
        use std::os::unix::fs::MetadataExt as _;
        // SAFETY: getuid has no preconditions.
        // (Android makes it 0771: others may pass through, never list or
        // write; its SELinux label keeps other apps out besides.)
        meta.is_dir() && meta.uid() == unsafe { libc::getuid() } && meta.mode() & 0o006 == 0
    });
    if !private {
        return Err(format!("{} isn't this app's private folder: not starting", files.display()));
    }
    let cache = files.parent().map(|p| p.join("cache")).filter(|p| p.is_dir()).unwrap_or_else(|| files.clone());
    let dir = files.join("app");
    unpack(pack, &dir).map_err(|error| format!("unpacking the app's files into {}: {error}", dir.display()))?;
    let _ = APP_DIR.set(dir);
    // (What builds with an engine of their own left in <files>/.zend stays
    // there unread: removing the phone's old workspace is the person's
    // call, not an upgrade's.)
    // SAFETY: no other thread of the app runs yet: this is the first
    // activity's `android_main`, before its window.
    unsafe {
        std::env::set_var("HOME", &files);
        std::env::set_var("XDG_DATA_HOME", &files);
        std::env::set_var("XDG_CONFIG_HOME", files.join(".config"));
        std::env::set_var("TMPDIR", &cache);
        // The emulator's Vulkan driver (vulkan.ranchu.so) crashes in
        // vkSetDebugUtilsObjectNameEXT as zgx-gpu makes its first bind
        // group layout: no debug labels and no debug-utils for the GPU.
        std::env::set_var("WGPU_DEBUG", "0");
        std::env::set_var("WGPU_DISCARD_HAL_LABELS", "1");
        // A debug build lints the scripts it loads from files, which these
        // unpacked ones are: no dev-lint strip over the app.
        std::env::set_var("ZGX_LINT", "0");
    }
    Ok(files)
}

/// Write the packed files under `dir`, replacing what an earlier version
/// left there, unless they are there already: the pack's `.pack-stamp`
/// (its first entry, apps/zgx/android/build.rs) matches the one in `dir`,
/// which is written last, so an unpacking cut short is done again.
fn unpack(mut rest: &[u8], dir: &Path) -> std::io::Result<()> {
    fn take<'a>(rest: &mut &'a [u8], n: usize) -> std::io::Result<&'a [u8]> {
        if rest.len() < n {
            return Err(std::io::Error::other("the packed files end early"));
        }
        let (s, after) = rest.split_at(n);
        *rest = after;
        Ok(s)
    }
    fn len(rest: &mut &[u8]) -> std::io::Result<usize> {
        Ok(u32::from_le_bytes(take(rest, 4)?.try_into().unwrap()) as usize)
    }
    const STAMP: &str = ".pack-stamp";
    let mut stamp = None;
    {
        let mut peek = rest;
        let n = len(&mut peek)?;
        if take(&mut peek, n)? == STAMP.as_bytes() {
            let n = len(&mut peek)?;
            let value = take(&mut peek, n)?;
            if std::fs::read(dir.join(STAMP)).is_ok_and(|kept| kept == value) {
                log::info!("zend: files in {} are this build's", dir.display());
                return Ok(());
            }
            stamp = Some(value);
            rest = peek;
        }
    }
    let _ = std::fs::remove_dir_all(dir);
    let mut count = 0;
    while !rest.is_empty() {
        let n = len(&mut rest)?;
        let rel = String::from_utf8_lossy(take(&mut rest, n)?).into_owned();
        let n = len(&mut rest)?;
        let path = dir.join(&rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, take(&mut rest, n)?)?;
        count += 1;
    }
    if let Some(stamp) = stamp {
        std::fs::write(dir.join(STAMP), stamp)?;
    }
    log::info!("zend: {count} files unpacked into {}", dir.display());
    Ok(())
}

/// What the app writes to stderr (`eprintln!`), into the log (logcat, tag `zend`), line by line.
fn stderr_to_log() {
    use std::io::BufRead;
    use std::os::fd::FromRawFd;
    let mut fds = [0; 2];
    // SAFETY: pipe(2) fills both; dup2 points stderr at the write end.
    if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 || unsafe { libc::dup2(fds[1], libc::STDERR_FILENO) } < 0 {
        return;
    }
    unsafe { libc::close(fds[1]) };
    // SAFETY: the read end is this thread's alone from here.
    let read = unsafe { std::fs::File::from_raw_fd(fds[0]) };
    let _ = std::thread::Builder::new().name("stderr".into()).spawn(move || {
        for line in std::io::BufReader::new(read).lines().map_while(Result::ok) {
            log::info!("{line}");
        }
    });
}

/// `system.insets` for this window's scripts, read again every so often
/// (the bars change with rotation and the keyboard).
pub fn install(cx: &mut WindowContext, host: &ScriptHost) {
    let shared = host.shared("system.insets", value(Insets::default()));
    let mut last = None;
    cx.tasks.spawn(async move {
        loop {
            if let Some(app) = zgx_desktop::android::app() {
                match insets(&app) {
                    Ok(Some(now)) if last != Some(now) => {
                        log::info!("zend: insets {now:?}");
                        shared.set(value(now));
                        last = Some(now);
                    }
                    Ok(_) => {}
                    Err(error) => log::warn!("zend: insets: {error}"),
                }
            }
            zgx::timer::sleep(Duration::from_millis(if last.is_some() { 500 } else { 100 })).await;
        }
    });
}

/// The bars over the window, in points.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Insets {
    top: f32,
    right: f32,
    bottom: f32,
    left: f32,
    /// The soft keyboard's height from the window's bottom (0 when hidden).
    keyboard: f32,
}

fn value(insets: Insets) -> Value {
    Value::record(vec![
        ("top", Value::from(insets.top as f64)),
        ("right", Value::from(insets.right as f64)),
        ("bottom", Value::from(insets.bottom as f64)),
        ("left", Value::from(insets.left as f64)),
        ("keyboard", Value::from(insets.keyboard as f64)),
    ])
}

/// Android 13 and later: ask once whether the app may post notifications
/// (a session done or waiting, crates/notify's banners), unless it may
/// already; the system shows its own dialog and remembers the answer (a
/// "Don't allow" isn't asked again: Android stops showing the dialog).
///
/// Asked on Java's main thread, as `Activity.requestPermissions` must be
/// (android_main is a thread of its own): queued there, it runs once the
/// activity's looper takes it, after this returns.
fn ask_notifications(app: &AndroidApp) {
    let activity = app.clone();
    app.run_on_java_main_thread(Box::new(move || {
        if let Err(error) = request_notifications(&activity) {
            log::warn!("zend: notifications: {error}");
        }
    }));
}

/// The asking itself, on the thread `ask_notifications` runs it on.
fn request_notifications(app: &AndroidApp) -> jni::errors::Result<()> {
    use jni::objects::{JObject, JValue};
    const PERMISSION: &str = "android.permission.POST_NOTIFICATIONS";
    // SAFETY: the activity's VM and its `Activity` object, live while it is.
    let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr().cast()) }?;
    // (The main thread is the VM's own: attached already, and stays so.)
    let mut env = vm.attach_current_thread()?;
    let activity = unsafe { JObject::from_raw(app.activity_as_ptr().cast()) };
    let result = env.with_local_frame(8, |env| -> jni::errors::Result<()> {
        if env.get_static_field("android/os/Build$VERSION", "SDK_INT", "I")?.i()? < 33 {
            return Ok(());
        }
        let name = env.new_string(PERMISSION)?;
        // PackageManager.PERMISSION_GRANTED is 0.
        if env.call_method(&activity, "checkSelfPermission", "(Ljava/lang/String;)I", &[JValue::Object(&name)])?.i()? == 0 {
            log::info!("zend: notifications: allowed already");
            return Ok(());
        }
        let names = env.new_object_array(1, "java/lang/String", &name)?;
        env.call_method(&activity, "requestPermissions", "([Ljava/lang/String;I)V", &[JValue::Object(&names), JValue::Int(1)])?;
        log::info!("zend: notifications: asked the person (on {:?})", std::thread::current().name());
        Ok(())
    });
    if env.exception_check().unwrap_or(false) {
        let _ = env.exception_describe();
        let _ = env.exception_clear();
    }
    result
}

/// The decor view's root insets (system bars and display cutout; the
/// keyboard apart), through JNI; None before the view is attached.
fn insets(app: &AndroidApp) -> jni::errors::Result<Option<Insets>> {
    use jni::objects::{JObject, JValue};
    // SAFETY: the activity's VM and its `Activity` object, live while it is.
    let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr().cast()) }?;
    let mut env = vm.attach_current_thread_permanently()?;
    let activity = unsafe { JObject::from_raw(app.activity_as_ptr().cast()) };
    let read = |env: &mut jni::JNIEnv| -> jni::errors::Result<Option<Insets>> {
        let window = env.call_method(&activity, "getWindow", "()Landroid/view/Window;", &[])?.l()?;
        let decor = env.call_method(&window, "getDecorView", "()Landroid/view/View;", &[])?.l()?;
        let root = env.call_method(&decor, "getRootWindowInsets", "()Landroid/view/WindowInsets;", &[])?.l()?;
        if root.is_null() {
            return Ok(None);
        }
        let resources = env.call_method(&activity, "getResources", "()Landroid/content/res/Resources;", &[])?.l()?;
        let metrics = env.call_method(&resources, "getDisplayMetrics", "()Landroid/util/DisplayMetrics;", &[])?.l()?;
        let density = env.get_field(&metrics, "density", "F")?.f()?.max(0.1);
        let sdk = env.get_static_field("android/os/Build$VERSION", "SDK_INT", "I")?.i()?;
        let sides = |env: &mut jni::JNIEnv, of: &JObject| -> jni::errors::Result<[f32; 4]> {
            let mut out = [0.0; 4];
            for (i, name) in ["top", "right", "bottom", "left"].into_iter().enumerate() {
                out[i] = env.get_field(of, name, "I")?.i()? as f32 / density;
            }
            Ok(out)
        };
        let (bars, keyboard) = if sdk >= 30 {
            let class = "android/view/WindowInsets$Type";
            let system = env.call_static_method(class, "systemBars", "()I", &[])?.i()?
                | env.call_static_method(class, "displayCutout", "()I", &[])?.i()?;
            let ime = env.call_static_method(class, "ime", "()I", &[])?.i()?;
            let bars = env.call_method(&root, "getInsets", "(I)Landroid/graphics/Insets;", &[JValue::Int(system)])?.l()?;
            let bars = sides(env, &bars)?;
            let ime = env.call_method(&root, "getInsets", "(I)Landroid/graphics/Insets;", &[JValue::Int(ime)])?.l()?;
            (bars, sides(env, &ime)?[2])
        } else {
            let mut out = [0.0; 4];
            for (i, name) in ["getSystemWindowInsetTop", "getSystemWindowInsetRight", "getSystemWindowInsetBottom", "getSystemWindowInsetLeft"]
                .into_iter()
                .enumerate()
            {
                out[i] = env.call_method(&root, name, "()I", &[])?.i()? as f32 / density;
            }
            (out, 0.0)
        };
        Ok(Some(Insets { top: bars[0], right: bars[1], bottom: bars[2], left: bars[3], keyboard }))
    };
    let read = env.with_local_frame(32, |env| read(env));
    if env.exception_check()? {
        env.exception_describe()?;
        env.exception_clear()?;
    }
    read
}
