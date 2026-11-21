//! The Android POC's native side: unpack the app's Luau folder, hand winit
//! the activity, run it with zgx-run as the desktop runs a folder.
use std::path::{Path, PathBuf};

static PACK: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/app.pack"));

/// Write the packed app folder under `dir` (replacing what was there) and
/// return its `app.luau`.
pub fn unpack(dir: &Path) -> std::io::Result<PathBuf> {
    let _ = std::fs::remove_dir_all(dir);
    fn take<'a>(rest: &mut &'a [u8], n: usize) -> &'a [u8] {
        let (s, after) = rest.split_at(n);
        *rest = after;
        s
    }
    fn len(rest: &mut &[u8]) -> usize {
        u32::from_le_bytes(take(rest, 4).try_into().unwrap()) as usize
    }
    let mut rest = PACK;
    while !rest.is_empty() {
        let n = len(&mut rest);
        let rel = String::from_utf8_lossy(take(&mut rest, n)).into_owned();
        let n = len(&mut rest);
        let path = dir.join(rel);
        std::fs::create_dir_all(path.parent().unwrap())?;
        std::fs::write(path, take(&mut rest, n))?;
    }
    Ok(dir.join("app.luau"))
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(app: zgx_desktop::android::AndroidApp) {
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("zgxpoc"),
    );
    std::panic::set_hook(Box::new(|info| log::error!("panic: {info}")));
    let files = app.internal_data_path().unwrap_or_else(|| PathBuf::from("/data/local/tmp"));
    let script = match unpack(&files.join("app")) {
        Ok(script) => script,
        Err(e) => return log::error!("unpack: {e}"),
    };
    // The emulator's Vulkan driver (vulkan.ranchu.so, API 36) crashes in
    // vkSetDebugUtilsObjectNameEXT as zgx-gpu makes its first bind group
    // layout: no debug labels and no debug-utils for the GPU.
    // SAFETY: before any other thread of the app starts.
    unsafe {
        std::env::set_var("WGPU_DEBUG", "0");
        std::env::set_var("WGPU_DISCARD_HAL_LABELS", "1");
    }
    log::info!("running {}", script.display());
    zgx_desktop::android::set_app(app);
    if let Err(e) = zgx_run::run(&script) {
        log::error!("zgx-run: {e}");
    }
    log::info!("zgx-run returned");
}
