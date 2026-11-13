//! The `zend` binary answers with the shared command line (`zend_cli`):
//! every subcommand's help as the library renders it, and its version. The
//! zgx interface runs the same checks against its own binary
//! (apps/zgx/tests/cli.rs), so the two can't drift apart.

use std::path::PathBuf;
use std::process::Command;

/// The binary under the name people type: clap names it after argv[0].
fn zend(dir: &std::path::Path) -> PathBuf {
    let link = dir.join("zend");
    #[cfg(unix)]
    std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_zend"), &link).unwrap();
    #[cfg(not(unix))]
    std::fs::copy(env!("CARGO_BIN_EXE_zend"), &link).unwrap();
    link
}

#[cfg(unix)]
#[test]
fn every_subcommand_s_help_is_the_shared_command_line_s() {
    let dir = tempfile::tempdir().unwrap();
    let zend = zend(dir.path());
    let version = env!("CARGO_PKG_VERSION");
    for path in zend_cli::subcommand_paths(version) {
        let out = Command::new(&zend)
            .args(&path)
            .arg("--help")
            .env("ZEND_DATA_DIR", dir.path())
            .output()
            .unwrap();
        assert!(out.status.success(), "{path:?}: {out:?}");
        assert_eq!(
            String::from_utf8_lossy(&out.stdout).trim_end(),
            zend_cli::help_for(version, &path).trim_end(),
            "zend {} --help",
            path.join(" ")
        );
    }
}

#[test]
fn version_is_zend_and_the_workspace_s() {
    let out = Command::new(env!("CARGO_BIN_EXE_zend"))
        .arg("--version")
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        format!("zend {}", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn a_bad_argument_exits_as_clap_does() {
    let out = Command::new(env!("CARGO_BIN_EXE_zend"))
        .arg("--no-such-flag")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("--no-such-flag"));
}
