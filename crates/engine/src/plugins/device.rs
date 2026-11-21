//! What the device the engine runs on lacks, by permission: Android runs
//! no programs for plugins, has no file picker, no second window and no web
//! view in the app. A permission it lacks is never granted there, whatever
//! was allowed (a consent made before, or on a desktop and carried over):
//! every entry's `allowed` drops it as the plugins are scanned, so a
//! service's `zend.run`, the native MCP servers handed to agents and the
//! window's grants all hold the same line. The window says why from the
//! same list (apps/zgx/src/plugins/grants.rs, ui/ds/platform.luau).

/// What Android lacks, by permission, and why, in words a person reads.
pub const ANDROID_LACKS: &[(&str, &str)] = &[
    ("commands", "Android runs no programs for plugins (no git or shell in the app)"),
    ("native", "Android runs no programs for plugins (no native server beside agents)"),
    ("files", "no file picker in the app (the system's answers through Java)"),
    ("windows", "Android shows the app in one window"),
    ("web", "no web view in the app"),
];

/// Whether this build runs on Android.
pub const ANDROID: bool = cfg!(target_os = "android");

/// Why a device lacks `permission` (`android`: whether it's Android).
pub fn lacks_on(android: bool, permission: &str) -> Option<&'static str> {
    if !android {
        return None;
    }
    ANDROID_LACKS.iter().find(|(p, _)| *p == permission).map(|(_, why)| *why)
}

/// Why this device lacks `permission`, if it does.
pub fn lacks(permission: &str) -> Option<&'static str> {
    lacks_on(ANDROID, permission)
}

/// What was allowed, less what the device lacks.
pub fn keep_what_it_has(android: bool, allowed: &mut Vec<String>) {
    allowed.retain(|p| lacks_on(android, p).is_none());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_desktop_lacks_nothing_and_android_lacks_what_it_names() {
        let all = |android| {
            let mut allowed: Vec<String> = ["commands", "native", "files", "windows", "web", "clipboard", "files.read", "network"]
                .map(String::from)
                .to_vec();
            keep_what_it_has(android, &mut allowed);
            allowed
        };
        assert_eq!(all(false).len(), 8, "the desktop keeps everything");
        assert_eq!(all(true), ["clipboard", "files.read", "network"]);
        assert!(lacks_on(true, "commands").unwrap().contains("no programs"));
        assert_eq!(lacks_on(false, "commands"), None);
    }
}
