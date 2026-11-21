//! The Android POC's native side: unpack the app's Luau files, hand winit
//! the activity, run them with zgx-run. The desktop runs the same files
//! through the same [`run`] (`zgx-poc`, src/bin), from the repository.
use std::path::Path;
use zgx_script::{Capability, ScriptHost};

/// The screen: Zend's real sidebar with fixed sessions, as a fixture of
/// Zend's interface (its root, apps/zgx/ui, is the app's).
pub const SCRIPT: &str = "apps/zgx/ui/android_poc_fixture.luau";
/// The plugin that places its button in it with zend.place.
pub const PLUGIN: &str = "apps/zgx/plugins-dev/poc-tap";

/// The plugin's id, as its zend-plugin.json and its `require("./zend")` say.
const PLUGIN_ID: &str = "poc-tap";

/// What Zend's window lets a plugin reach that the SDK (zend.luau) needs to
/// load and to place a node, as apps/zgx's plugins/grants.rs names it: the
/// shared state every plugin reads, its own, and adding by zend.place
/// behind "ui.add". No engine behind it: the plugin's calls go nowhere.
fn zend_surface(host: &ScriptHost, id: &str) {
    host.reserve_prefix("zend");
    host.require_contribution_capability(Capability::Named("zend.ui.add".into()));
    for key in [
        "zend.route",
        "zend.appearance",
        "zend.locale",
        "zend.page_session",
        "zgx.window.shown",
        "zgx.window.active",
        "system.reduced_transparency",
        "system.popups_blur",
        "window.corner_radius",
        "window.blur",
    ] {
        host.expose_shared_read(key, None);
    }
    let own = Capability::Named(format!("plugin:{id}"));
    for name in [
        "calls", "inbox", "records", "github", "store", "settings", "locales", "menu_items",
        "composed", "stage",
    ] {
        host.expose_shared_read(format!("plugin.{id}.{name}"), Some(own.clone()));
    }
    for name in ["result", "ack", "send", "toast", "problem", "cost", "store_set"] {
        let action = format!("plugin.{id}.{name}");
        host.action(action.clone(), |_| {});
        host.expose_action(action, Some(own.clone()));
    }
}

/// Run the screen with its plugin, both under `base` (the repository, or
/// where the APK unpacked them), until its window closes.
pub fn run(base: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let plugin = zgx_script::Plugin {
        name: PLUGIN_ID.into(),
        entry: base.join(PLUGIN).join("init.luau"),
        // "ui.add", as Zend grants it to a plugin allowed it.
        grants: vec![
            Capability::Named("zend.ui.add".into()),
            Capability::Named(format!("plugin:{PLUGIN_ID}")),
        ],
    };
    for bytes in FONTS {
        zgx_run::add_font(zgx_desktop::FontData::new(std::sync::Arc::<[u8]>::from(bytes))?);
    }
    zgx_run::run_hosting(&base.join(SCRIPT), Vec::new(), move |host| {
        zend_surface(host, PLUGIN_ID);
        host.set_plugins(vec![plugin.clone()]);
    })
}

/// Zend's own interface fonts, the files its window carries (apps/zgx
/// src/main.rs `fonts()`): the theme asks for Geist, which neither Android
/// nor most desktops have.
static FONTS: [&[u8]; 9] = [
    include_bytes!("../../zgx/fonts/Geist.ttf"),
    include_bytes!("../../zgx/fonts/Geist-Italic.ttf"),
    include_bytes!("../../zgx/fonts/Geist-Medium.ttf"),
    include_bytes!("../../zgx/fonts/Geist-SemiBold.ttf"),
    include_bytes!("../../zgx/fonts/Geist-Bold.ttf"),
    include_bytes!("../../zgx/fonts/GeistMono.ttf"),
    include_bytes!("../../zgx/fonts/GeistMono-Medium.ttf"),
    include_bytes!("../../zgx/fonts/InstrumentSerif-Regular.ttf"),
    include_bytes!("../../zgx/fonts/InstrumentSerif-Italic.ttf"),
];

static PACK: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/app.pack"));

/// Write the packed folders under `dir` (replacing what was there), at
/// their paths in the repository.
pub fn unpack(dir: &Path) -> std::io::Result<()> {
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
    Ok(())
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
    let files = app
        .internal_data_path()
        .unwrap_or_else(|| std::path::PathBuf::from("/data/local/tmp"));
    let base = files.join("app");
    if let Err(e) = unpack(&base) {
        return log::error!("unpack: {e}");
    }
    // The emulator's Vulkan driver (vulkan.ranchu.so, API 36) crashes in
    // vkSetDebugUtilsObjectNameEXT as zgx-gpu makes its first bind group
    // layout: no debug labels and no debug-utils for the GPU.
    // SAFETY: before any other thread of the app starts.
    unsafe {
        std::env::set_var("WGPU_DEBUG", "0");
        std::env::set_var("WGPU_DISCARD_HAL_LABELS", "1");
        // No dev-lint strip over the app: a debug build lints the scripts
        // it loads from files, which the POC's unpacked ones are.
        std::env::set_var("ZGX_LINT", "0");
    }
    log::info!("running {}", base.join(SCRIPT).display());
    zgx_desktop::android::set_app(app);
    if let Err(e) = run(&base) {
        log::error!("zgx-run: {e}");
    }
    log::info!("zgx-run returned");
}
