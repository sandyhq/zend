//! The POC on the desktop: the same screen and plugin as the APK, from
//! the repository (or the folder given), through the same `run`.
//! `--inspect` serves zgx's devtools (`zgx inspect`).
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|a| a == "--inspect") {
        args.remove(0);
        zgx_run::set_inspect(true);
    }
    let base = match args.first() {
        Some(dir) => std::path::PathBuf::from(dir),
        None => std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."),
    };
    let base = std::path::absolute(base)?;
    zend_zgx_android::run(&base)
}
