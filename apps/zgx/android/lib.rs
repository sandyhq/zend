//! The APK's entry: NativeActivity calls `android_main` on a thread of its
//! own, which runs the app (zend-zgx src/android.rs) with the files packed
//! at build time (build.rs).

#[cfg(target_os = "android")]
static PACK: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/app.pack"));

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(app: zgx_desktop::android::AndroidApp) {
    zend_zgx::android_main(app, PACK);
}
