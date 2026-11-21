//! The app's Luau folder (ZGX_POC_APP, default `hello/`) packed into the
//! library (`OUT_DIR/app.pack`): on Android it is written out to the app's
//! files folder at start, where zgx-run loads it from as on the desktop.
use std::{env, fs, path::Path};

fn pack(root: &Path, dir: &Path, out: &mut Vec<u8>) {
    let mut entries: Vec<_> = fs::read_dir(dir).unwrap().flatten().collect();
    entries.sort_by_key(|e| e.path());
    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            pack(root, &path, out);
        } else {
            let rel = path.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
            let bytes = fs::read(&path).unwrap();
            out.extend((rel.len() as u32).to_le_bytes());
            out.extend(rel.as_bytes());
            out.extend((bytes.len() as u32).to_le_bytes());
            out.extend(bytes);
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }
}

fn main() {
    let manifest = env::var("CARGO_MANIFEST_DIR").unwrap();
    let app = env::var("ZGX_POC_APP").unwrap_or_else(|_| format!("{manifest}/hello"));
    println!("cargo:rerun-if-env-changed=ZGX_POC_APP");
    println!("cargo:rerun-if-changed={app}");
    let mut out = Vec::new();
    pack(Path::new(&app), Path::new(&app), &mut out);
    fs::write(Path::new(&env::var("OUT_DIR").unwrap()).join("app.pack"), out).unwrap();
}
