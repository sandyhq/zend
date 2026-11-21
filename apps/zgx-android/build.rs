//! The POC's Luau files packed into the library (`OUT_DIR/app.pack`), at
//! their paths in the repository: Zend's interface (apps/zgx/ui, whose
//! fixture is the screen; its tests left out) and the poc-tap plugin. On
//! Android they are written out to the app's files folder at start, in the
//! same layout, and run from there as the desktop runs them from the
//! repository.
use std::{
    env, fs,
    path::{Path, PathBuf},
};

const PACKED: &[&str] = &["apps/zgx/ui", "apps/zgx/plugins-dev/poc-tap"];
const LEFT_OUT: &[&str] = &["apps/zgx/ui/tests"];

fn pack(root: &Path, dir: &Path, out: &mut Vec<u8>) {
    let mut entries: Vec<_> = fs::read_dir(dir).unwrap().flatten().collect();
    entries.sort_by_key(|e| e.path());
    for entry in entries {
        let path = entry.path();
        let rel = path.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
        if LEFT_OUT.contains(&rel.as_str()) {
            continue;
        }
        if path.is_dir() {
            pack(root, &path, out);
        } else {
            let bytes = fs::read(&path).unwrap();
            out.extend((rel.len() as u32).to_le_bytes());
            out.extend(rel.as_bytes());
            out.extend((bytes.len() as u32).to_le_bytes());
            out.extend(bytes);
        }
    }
}

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let repo = manifest.join("../..").canonicalize().unwrap();
    let mut out = Vec::new();
    for dir in PACKED {
        println!("cargo:rerun-if-changed={}", repo.join(dir).display());
        pack(&repo, &repo.join(dir), &mut out);
    }
    fs::write(Path::new(&env::var("OUT_DIR").unwrap()).join("app.pack"), out).unwrap();
}
