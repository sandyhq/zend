//! The git commit this engine is built from, for its build id
//! (`zend_engine::build_id`): `ZEND_GIT_COMMIT`, or "unknown" outside a
//! checkout. And when it was built, where the executable's own time says
//! nothing (Android runs the app as `app_process`): `ZEND_BUILD_TIME`, the
//! seconds a packaging script (apps/zgx/android/build-apk.sh) sets, else
//! empty.
use std::path::PathBuf;

fn git(args: &[&str]) -> Option<String> {
    std::process::Command::new("git")
        .args(args)
        .output()
        .ok()
        .filter(|out| out.status.success())
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}

fn main() {
    let commit = git(&["rev-parse", "--short=12", "HEAD"]).unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=ZEND_GIT_COMMIT={commit}");
    // Run again as the commit moves: HEAD (a worktree's own), the branches
    // it can point at, and the packed ones. (The build's own time is the
    // executable's, read as it runs, so a rebuild changes the id without
    // running this again.)
    println!("cargo:rerun-if-changed=build.rs");
    let here = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap_or_default());
    let absolute = |p: String| {
        let p = PathBuf::from(p);
        if p.is_absolute() { p } else { here.join(p) }
    };
    if let Some(head) = git(&["rev-parse", "--git-path", "HEAD"]) {
        println!("cargo:rerun-if-changed={}", absolute(head).display());
    }
    if let Some(common) = git(&["rev-parse", "--git-common-dir"]) {
        let common = absolute(common);
        println!("cargo:rerun-if-changed={}", common.join("refs/heads").display());
        println!("cargo:rerun-if-changed={}", common.join("packed-refs").display());
    }
    let built = std::env::var("ZEND_BUILD_TIME").unwrap_or_default();
    println!("cargo:rustc-env=ZEND_BUILD_TIME={}", built.trim());
    println!("cargo:rerun-if-env-changed=ZEND_BUILD_TIME");
}
