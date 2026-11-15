//! luau-lsp fetched for the first type check that finds none: Zend
//! doesn't ship it. One pinned release ([`VERSION`]), this platform's zip
//! from GitHub, checked against the SHA-256 written here before anything
//! of it is unpacked, put in `<data>/tools`, where the lookup finds it
//! from then on. Offline, or anything else failing, the check says why and
//! checks the syntax alone, as without it.
//!
//! `ZEND_LUAU_LSP_FETCH=0` turns it off; tests never fetch.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};

/// The release fetched (the one zgx's own checks use).
pub const VERSION: &str = "1.70.1";
/// Its zips, by platform: asset name and SHA-256 (GitHub's digest of each,
/// checked against a download).
const ASSETS: &[(&str, &str, &str, &str)] = &[
    ("linux", "x86_64", "luau-lsp-linux-x86_64.zip", "1a2ea1ae4f98f8946cefd970a4b54853e11ab4775e6932faa7f60cf920346567"),
    ("linux", "aarch64", "luau-lsp-linux-arm64.zip", "6062345b95124ca937b8c324b74bc3fddb1d6919ba133c58f8b17945ce1acbc2"),
    // (One universal binary for both.)
    ("macos", "x86_64", "luau-lsp-macos.zip", "7d3936e8dec6dc77547abd061d2e950da392562a5f94858b880f23dba50d84cd"),
    ("macos", "aarch64", "luau-lsp-macos.zip", "7d3936e8dec6dc77547abd061d2e950da392562a5f94858b880f23dba50d84cd"),
    ("windows", "x86_64", "luau-lsp-win64.zip", "64225b9738102a30de6ee09fb28e5a3827edc402ecf2a6364af570312468159f"),
];
/// A zip bigger than this isn't the one.
const MAX_ZIP: u64 = 32 * 1024 * 1024;
/// The whole download at most.
const TIMEOUT: Duration = Duration::from_secs(120);
/// After a fetch failed, how long this process doesn't try again (a save's
/// check each second offline shouldn't each wait on the network).
const RETRY_AFTER: Duration = Duration::from_secs(10 * 60);

static FAILED: LazyLock<Mutex<Option<(Instant, String)>>> = LazyLock::new(Mutex::default);
/// One fetch at a time: a check that comes while one fetches waits for it.
static FETCHING: Mutex<()> = Mutex::new(());

/// Whether a check may fetch luau-lsp (not in tests, not with
/// `ZEND_LUAU_LSP_FETCH=0`).
pub fn allowed() -> bool {
    !cfg!(test) && std::env::var("ZEND_LUAU_LSP_FETCH").map_or(true, |v| v.trim() != "0")
}

/// This platform's zip and its SHA-256, if the release has one.
fn asset() -> Option<(&'static str, &'static str)> {
    ASSETS
        .iter()
        .find(|(os, arch, _, _)| *os == std::env::consts::OS && *arch == std::env::consts::ARCH)
        .map(|(_, _, name, sha)| (*name, *sha))
}

/// The binary's name, in the zip and in `tools`.
pub fn binary_name() -> &'static str {
    if cfg!(windows) { "luau-lsp.exe" } else { "luau-lsp" }
}

/// Fetch luau-lsp into `<data_dir>/tools`: where it is now and whether
/// this call fetched it (false: another check just did), or why not.
/// Blocks (its own thread and runtime: callable from anywhere).
pub fn fetch(data_dir: &Path) -> Result<(PathBuf, bool), String> {
    let _one = FETCHING.lock().unwrap_or_else(|e| e.into_inner());
    let there = data_dir.join("tools").join(binary_name());
    if there.is_file() {
        return Ok((there, false));
    }
    if let Some((at, why)) = FAILED.lock().unwrap_or_else(|e| e.into_inner()).as_ref()
        && at.elapsed() < RETRY_AFTER
    {
        return Err(why.clone());
    }
    let (name, sha) = asset().ok_or_else(|| {
        format!("luau-lsp {VERSION} has no build for {} {}", std::env::consts::OS, std::env::consts::ARCH)
    })?;
    let url = format!("https://github.com/JohnnyMorganz/luau-lsp/releases/download/{VERSION}/{name}");
    let tools = data_dir.join("tools");
    let result = std::thread::spawn(move || {
        let bytes = download(&url)?;
        install(&bytes, sha, &tools)
    })
    .join()
    .unwrap_or_else(|_| Err("its download stopped".into()));
    if let Err(why) = &result {
        *FAILED.lock().unwrap_or_else(|e| e.into_inner()) = Some((Instant::now(), why.clone()));
    }
    result.map(|path| (path, true))
}

fn download(url: &str) -> Result<Vec<u8>, String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("no runtime to fetch it on ({e})"))?;
    runtime.block_on(async {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(TIMEOUT)
            .user_agent(concat!("zend/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| format!("couldn't fetch it ({e})"))?;
        let mut response = client.get(url).send().await.map_err(|e| format!("couldn't reach {url} ({})", plain(&e.without_url())))?;
        if !response.status().is_success() {
            return Err(format!("{url} answered {}", response.status()));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|e| format!("its download broke off ({})", plain(&e.without_url())))? {
            bytes.extend_from_slice(&chunk);
            if bytes.len() as u64 > MAX_ZIP {
                return Err(format!("{url} is bigger than {} MB: not the release", MAX_ZIP / 1024 / 1024));
            }
        }
        Ok(bytes)
    })
}

/// reqwest's error with what caused it (its own words are only "error
/// sending request").
fn plain(error: &reqwest::Error) -> String {
    let mut text = error.to_string();
    let mut source = std::error::Error::source(error);
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}

/// The zip `bytes`, if its SHA-256 is `sha`, unpacked into `tools`: the
/// binary put in place in one rename.
fn install(bytes: &[u8], sha: &str, tools: &Path) -> Result<PathBuf, String> {
    let got = hex(&Sha256::digest(bytes));
    if got != sha {
        return Err(format!("its download isn't the release: SHA-256 {got}, not {sha}"));
    }
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| format!("its zip doesn't open ({e})"))?;
    let name = binary_name();
    let mut file = zip.by_name(name).map_err(|_| format!("its zip holds no {name}"))?;
    let mut binary = Vec::new();
    file.by_ref()
        .take(MAX_ZIP * 4)
        .read_to_end(&mut binary)
        .map_err(|e| format!("its zip doesn't unpack ({e})"))?;
    std::fs::create_dir_all(tools).map_err(|e| format!("couldn't make {} ({e})", tools.display()))?;
    let temp = tools.join(format!(".{name}.{}", crate::new_id()));
    std::fs::write(&temp, &binary).map_err(|e| format!("couldn't write {} ({e})", temp.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&temp, std::fs::Permissions::from_mode(0o755));
    }
    let target = tools.join(name);
    std::fs::rename(&temp, &target).map_err(|e| {
        let _ = std::fs::remove_file(&temp);
        format!("couldn't put it in {} ({e})", target.display())
    })?;
    Ok(target)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zip_of(name: &str, body: &[u8]) -> Vec<u8> {
        let mut out = std::io::Cursor::new(Vec::new());
        let mut zip = zip::ZipWriter::new(&mut out);
        zip.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        std::io::Write::write_all(&mut zip, body).unwrap();
        zip.finish().unwrap();
        out.into_inner()
    }

    #[test]
    fn a_zip_with_the_pinned_checksum_is_put_in_tools_and_another_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let tools = tmp.path().join("tools");
        let zip = zip_of(binary_name(), b"#!/bin/sh\necho 1.70.1\n");
        let sha = hex(&Sha256::digest(&zip));
        let wrong = "0".repeat(64);
        let refused = install(&zip, &wrong, &tools).unwrap_err();
        assert!(refused.contains("isn't the release") && refused.contains(&sha), "{refused}");
        assert!(!tools.join(binary_name()).exists(), "nothing of it is kept");
        let put = install(&zip, &sha, &tools).unwrap();
        assert_eq!(put, tools.join(binary_name()));
        assert_eq!(std::fs::read(&put).unwrap(), b"#!/bin/sh\necho 1.70.1\n");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&put).unwrap().permissions().mode() & 0o777, 0o755);
        }
        // Only the binary: no temporary file left beside it.
        assert_eq!(std::fs::read_dir(&tools).unwrap().count(), 1);
        let other = zip_of("README", b"x");
        let other_sha = hex(&Sha256::digest(&other));
        assert!(install(&other, &other_sha, &tools).unwrap_err().contains("holds no"));
    }

    #[test]
    fn every_platform_zend_ships_on_has_a_pinned_zip() {
        for (os, arch) in [("linux", "x86_64"), ("linux", "aarch64"), ("macos", "x86_64"), ("macos", "aarch64"), ("windows", "x86_64")] {
            let found = ASSETS.iter().find(|a| a.0 == os && a.1 == arch).unwrap_or_else(|| panic!("{os} {arch}"));
            assert_eq!(found.3.len(), 64);
            assert!(found.3.chars().all(|c| c.is_ascii_hexdigit()));
        }
        assert!(!allowed(), "tests never fetch");
    }
}
