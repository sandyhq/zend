//! Handing a link or a file to another app: the system's opener on the
//! desktop (`open`: `xdg-open`, `open`, `start`), an intent on Android
//! (zgx-desktop's `android::open_uri`), where `open`'s launchers don't
//! exist. Every place the app opens something outside itself comes here.

use std::ffi::OsStr;

/// Open `target` (a URL, or a file or folder on this device) in the app
/// the system has for it, without waiting for that app.
#[cfg(not(target_os = "android"))]
pub fn open(target: impl AsRef<OsStr>) -> std::io::Result<()> {
    open::that_detached(target)
}

/// Android: a link (`https:`, `mailto:`…) goes to its app as an
/// `ACTION_VIEW` intent. A file of the app's own can't: other apps can't
/// read the app's private folder (that needs a content provider, which
/// takes Java), so the interface offers no Reveal or Open outside on
/// Android, and this says why if asked.
#[cfg(target_os = "android")]
pub fn open(target: impl AsRef<OsStr>) -> std::io::Result<()> {
    let target = target.as_ref().to_string_lossy();
    let scheme = target.split_once(':').map(|(scheme, _)| scheme).filter(|scheme| {
        scheme.len() > 1 && scheme.starts_with(|c: char| c.is_ascii_alphabetic())
            && scheme.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
    });
    if scheme.is_none() || scheme == Some("file") {
        return Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "files don't open in other apps on Android"));
    }
    log::info!("zend: opening a {} link in its app", scheme.unwrap_or_default());
    zgx_desktop::android::open_uri(&target).map_err(std::io::Error::other)
}
