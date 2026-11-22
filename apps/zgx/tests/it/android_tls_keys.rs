//! On Android every std `thread_local!` static takes a pthread key of its
//! own (the target has no ELF TLS in stable Rust), and bionic aborts past
//! 128 keys for the whole process: "out of TLS keys" on a Galaxy S25. This
//! crate's thread-locals go through `zgx::tls::thread_local!` (one key for
//! all of them on Android, std's elsewhere). The count in the built library
//! is checked by android/count-tls-keys.sh, which build-apk.sh runs.

use std::path::{Path, PathBuf};

/// The crate to scan (`ZEND_ZGX_TLS_SCAN`: another checkout's apps/zgx,
/// e.g. an older commit's, to see this fail there).
fn root() -> PathBuf {
    std::env::var_os("ZEND_ZGX_TLS_SCAN")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")))
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// `thread_local!` calls that are std's (bare or `std::`), not zgx's.
fn std_thread_locals(source: &str) -> Vec<usize> {
    source
        .lines()
        .enumerate()
        .filter(|(_, line)| !line.trim_start().starts_with("//"))
        .filter(|(_, line)| {
            line.match_indices("thread_local!").any(|(at, _)| {
                let before = &line[..at];
                !before.ends_with("tls::") && !before.ends_with(|c: char| c.is_alphanumeric() || c == '_')
            })
        })
        .map(|(index, _)| index + 1)
        .collect()
}

#[test]
fn every_thread_local_in_the_app_takes_no_os_key_of_its_own_on_android() {
    let root = root();
    let mut files = Vec::new();
    for dir in ["src", "android"] {
        rust_files(&root.join(dir), &mut files);
    }
    assert!(files.len() > 50, "no sources under {}", root.display());
    let mut found = Vec::new();
    for file in &files {
        let source = std::fs::read_to_string(file).unwrap();
        for line in std_thread_locals(&source) {
            found.push(format!("{}:{line}", file.strip_prefix(&root).unwrap().display()));
        }
    }
    assert!(
        found.is_empty(),
        "{} std thread_local! (one Android OS key each; use zgx::tls::thread_local!):\n{}",
        found.len(),
        found.join("\n")
    );
}

#[test]
fn the_scan_tells_std_s_from_zgx_s() {
    assert_eq!(std_thread_locals("thread_local! {\n"), vec![1]);
    assert_eq!(std_thread_locals("    std::thread_local! {"), vec![1]);
    assert!(std_thread_locals("zgx::tls::thread_local! {").is_empty());
    assert!(std_thread_locals("/// a `thread_local!` here").is_empty());
}
