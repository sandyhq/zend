//! Plugins' instructions ([`zend_proto::RunRequest::instructions`]) for this
//! Pi process: an extension puts each, in full, in a system-prompt section of
//! its own (`plugin-<id>`) before every agent turn. It reads them from a
//! file in the run's scratch folder, which a reconfigure in place rewrites,
//! so a change reaches the live process at its next turn.
use std::path::{Path, PathBuf};

use crate::{HarnessError, process::Command, scratch::ScratchDir};
use zend_proto::SessionInstruction;

/// The extension reads its file's path from this variable.
const FILE_ENV: &str = "ZEND_PI_INSTRUCTIONS";
const EXTENSION: &str = "zend-instructions.mjs";
const FILE: &str = "zend-instructions.json";

/// Load the extension into `cmd` (always: instructions given later reach a
/// process started without any), with `instructions` as they are now. Its
/// files go in `scratch` (made when there's none yet). The file's path.
pub(super) fn configure(
    cmd: &mut Command,
    scratch: &mut Option<ScratchDir>,
    instructions: &[SessionInstruction],
) -> Result<PathBuf, HarnessError> {
    let dir = match scratch {
        Some(dir) => dir,
        None => scratch.insert(ScratchDir::new("pi-instructions")?),
    };
    let extension = dir.path().join(EXTENSION);
    std::fs::write(&extension, include_str!("instructions.mjs"))?;
    let file = dir.path().join(FILE);
    write(&file, instructions)?;
    cmd.arg("--extension").arg(extension).env(FILE_ENV, &file);
    Ok(file)
}

/// What the extension reads: each plugin's section name and text, in order.
pub(super) fn write(file: &Path, instructions: &[SessionInstruction]) -> std::io::Result<()> {
    let list: Vec<serde_json::Value> = instructions
        .iter()
        .filter(|i| !i.text.trim().is_empty())
        .map(|i| serde_json::json!({ "section": section(&i.plugin), "text": i.text }))
        .collect();
    let tmp = file.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec(&list).expect("serializable instructions"))?;
    std::fs::rename(tmp, file)
}

/// `plugin-<id>`, its id kept to what a tag name may hold.
fn section(plugin: &str) -> String {
    let id: String = plugin
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '-' })
        .collect();
    format!("plugin-{id}")
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn the_extension_reads_the_file_it_was_given_and_a_rewrite_replaces_it() {
        assert!(include_str!("instructions.mjs").contains("process.env.ZEND_PI_INSTRUCTIONS"));
        let zebra = SessionInstruction {
            plugin: "zebra".into(),
            name: "Zebra".into(),
            text: "Antworte immer mit dem Wort ZEBRA am Ende".into(),
        };
        let mut cmd = Command::new("pi");
        let mut scratch = None;
        let file = configure(&mut cmd, &mut scratch, std::slice::from_ref(&zebra)).unwrap();
        let dir = scratch.as_ref().expect("a scratch folder").path().to_path_buf();
        assert!(dir.join(EXTENSION).exists());
        let args: Vec<String> = cmd.as_std().get_args().map(|a| a.to_string_lossy().into_owned()).collect();
        assert_eq!(args, vec!["--extension".to_string(), dir.join(EXTENSION).display().to_string()]);
        let env = cmd
            .as_std()
            .get_envs()
            .find(|(k, _)| *k == FILE_ENV)
            .and_then(|(_, v)| v)
            .map(|v| v.to_string_lossy().into_owned());
        assert_eq!(env.as_deref(), Some(file.to_str().unwrap()));
        let read = |f: &Path| serde_json::from_slice::<serde_json::Value>(&std::fs::read(f).unwrap()).unwrap();
        assert_eq!(
            read(&file),
            serde_json::json!([{ "section": "plugin-zebra", "text": "Antworte immer mit dem Wort ZEBRA am Ende" }])
        );
        write(&file, &[]).unwrap();
        assert_eq!(read(&file), serde_json::json!([]));
        assert_eq!(section("My_Plugin"), "plugin-my-plugin");
    }
}
