//! The files the app reads at run time, packed into the library
//! (`OUT_DIR/app.pack`) for src/android.rs to write out at start, in the
//! layout an installed desktop app has beside its executable
//! (scripts/package-linux.sh): `ui/` without its tests and zgx's generated
//! types, and `plugins-builtin/` with the SDK's zend.luau in each plugin.
//! Each file: its path's length (u32 LE), the path, its length, its bytes.
//! The first is `.pack-stamp`, the rest's FNV-1a hash: the app writes the
//! files out only when it differs from the one it wrote last.
use std::{
    env, fs,
    path::{Path, PathBuf},
};

/// Left out wherever they are: zgx's generated types, editor and OS litter.
const SKIPPED: &[&str] = &[".zgx", ".DS_Store", ".lock"];

fn pack(dir: &Path, at: &str, skip: &[&str], out: &mut Vec<u8>) {
    let mut entries: Vec<_> = fs::read_dir(dir).unwrap().flatten().collect();
    entries.sort_by_key(|e| e.path());
    for entry in entries {
        let name = entry.file_name().to_string_lossy().into_owned();
        let rel = format!("{at}/{name}");
        if SKIPPED.contains(&name.as_str()) || skip.contains(&rel.as_str()) {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            pack(&path, &rel, skip, out);
        } else {
            file(&rel, &fs::read(&path).unwrap(), out);
        }
    }
}

fn file(rel: &str, bytes: &[u8], out: &mut Vec<u8>) {
    out.extend((rel.len() as u32).to_le_bytes());
    out.extend(rel.as_bytes());
    out.extend((bytes.len() as u32).to_le_bytes());
    out.extend(bytes);
}

fn main() {
    let zgx = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("..");
    let mut out = Vec::new();
    for dir in ["ui", "plugins-builtin", "plugins-sdk"] {
        println!("cargo:rerun-if-changed={}", zgx.join(dir).display());
    }
    pack(&zgx.join("ui"), "ui", &["ui/tests"], &mut out);
    pack(&zgx.join("plugins-builtin"), "plugins-builtin", &[], &mut out);
    let sdk = fs::read(zgx.join("plugins-sdk/zend.luau")).unwrap();
    let mut plugins: Vec<_> = fs::read_dir(zgx.join("plugins-builtin")).unwrap().flatten().filter(|e| e.path().is_dir()).collect();
    plugins.sort_by_key(|e| e.path());
    for plugin in plugins {
        file(&format!("plugins-builtin/{}/zend.luau", plugin.file_name().to_string_lossy()), &sdk, &mut out);
    }
    let hash = out.iter().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| (h ^ *b as u64).wrapping_mul(0x0100_0000_01b3));
    let mut packed = Vec::with_capacity(out.len() + 64);
    file(".pack-stamp", format!("{hash:016x}").as_bytes(), &mut packed);
    packed.extend(out);
    fs::write(Path::new(&env::var("OUT_DIR").unwrap()).join("app.pack"), packed).unwrap();
}
