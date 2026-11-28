//! Host-local UUID -> native file lookup. Never rewrite Pi's conversation files.
use crate::HarnessError;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
};

#[derive(Clone)]
pub(super) struct Store {
    root: PathBuf,
    agent: PathBuf,
    legacy: PathBuf,
    /// A sandboxed chat's state folder ([`crate::sandbox::Sandbox::state`]):
    /// its sessions are kept there and found only there, never in Pi's
    /// own folders or one a project's settings name.
    confine: Option<PathBuf>,
}
/// Pi's settings directory: an explicit override, else `PI_CODING_AGENT_DIR`,
/// else `~/.pi/agent`.
pub(super) fn agent_dir(explicit: Option<PathBuf>) -> PathBuf {
    explicit.unwrap_or_else(|| {
        crate::model_context::root(
            "PI_CODING_AGENT_DIR",
            crate::executable::home_or_current_dir().join(".pi/agent"),
        )
    })
}
impl Store {
    pub fn new(root: Option<PathBuf>, agent: Option<PathBuf>) -> Self {
        let home = crate::executable::home_or_current_dir();
        let agent = agent_dir(agent);
        Self {
            root: root.unwrap_or_else(|| agent.join("zend-sessions")),
            agent,
            legacy: home.join(".pi/pi-acp/session-map.json"),
            confine: None,
        }
    }
    /// The store of a sandboxed chat whose state folder is `state`
    /// (canonical): its index and Pi's session files both in there.
    pub fn confined(self, state: &Path) -> Self {
        Self {
            root: state.join("zend-sessions"),
            confine: Some(state.to_path_buf()),
            ..self
        }
    }
    /// Where a confined store's Pi keeps its sessions (`--session-dir`).
    pub fn session_dir(&self) -> Option<PathBuf> {
        self.confine.as_ref().map(|state| state.join("sessions"))
    }
    /// Whether `file` may be a session of this store: anywhere unconfined,
    /// else inside the state folder (no `..`).
    pub(super) fn admits(&self, file: &Path) -> bool {
        self.confine.as_ref().is_none_or(|state| {
            file.starts_with(state)
                && !file
                    .components()
                    .any(|part| matches!(part, std::path::Component::ParentDir))
        })
    }
    /// Whether a steering mode is already set in the settings Pi loads for
    /// `cwd` (global, then project).
    pub fn steering_mode_configured(&self, cwd: &Path) -> bool {
        [
            self.agent.join("settings.json"),
            cwd.join(".pi/settings.json"),
        ]
        .iter()
        .any(|settings| json_file(settings).get("steeringMode").is_some())
    }
    fn key(&self, id: &str) -> PathBuf {
        self.root
            .join(format!("{:x}.json", Sha256::digest(id.as_bytes())))
    }
    pub fn remember(&self, id: &str, file: &Path) -> Result<(), HarnessError> {
        if !file.is_absolute() {
            return Err(HarnessError::Protocol(
                "Pi returned a non-absolute session file".into(),
            ));
        }
        if !self.admits(file) {
            return Err(HarnessError::Protocol(format!(
                "Pi kept its session at {}, outside the sandbox's state folder",
                file.display()
            )));
        }
        self.write(id, &json!({"sessionId":id,"sessionFile":file}))
    }
    fn write(&self, id: &str, value: &Value) -> Result<(), HarnessError> {
        std::fs::create_dir_all(&self.root)?;
        let path = self.key(id);
        if json_file(&path) == *value {
            return Ok(());
        }
        let temp = self.root.join(format!("{}.tmp", uuid::Uuid::new_v4()));
        std::fs::write(&temp, serde_json::to_vec(value).unwrap())?;
        let result = std::fs::rename(&temp, &path);
        if result.is_err() {
            let _ = std::fs::remove_file(&temp);
        }
        result.map_err(Into::into)
    }
    pub fn mark_submitted(&self, id: &str) -> Result<(), HarnessError> {
        let mut record = json_file(&self.key(id));
        if let Some(map) = record.as_object_mut() {
            if map.remove("emptyState").is_some() {
                self.write(id, &record)?;
            }
        }
        Ok(())
    }
    pub fn remember_empty(&self, state: &Value, cwd: &Path) -> Result<(), HarnessError> {
        let id = state["sessionId"]
            .as_str()
            .ok_or_else(|| HarnessError::Protocol("Pi omitted sessionId".into()))?;
        let mut record = json_file(&self.key(id));
        if record.is_object() {
            record["emptyState"] = state.clone();
            record["cwd"] = json!(canonical(cwd)?);
            self.write(id, &record)?;
        }
        Ok(())
    }
    pub fn resume_args(&self, id: &str, cwd: &Path) -> Result<Vec<String>, HarnessError> {
        match self.resolve(id, cwd) {
            Ok(file) => Ok(vec!["--session".into(), file.display().to_string()]),
            Err(error) => {
                let record = json_file(&self.key(id));
                let state = &record["emptyState"];
                let file = record["sessionFile"].as_str().map(Path::new);
                // Only our own post-ACK empty-session proof permits recreation.
                // Deleted history, custom extension entries, and uncertain crashed
                // prompts must never become a fresh conversation silently.
                if state.is_null()
                    || record["cwd"] != json!(canonical(cwd)?)
                    || !file.is_some_and(|p| {
                        p.is_absolute()
                            && p.metadata()
                                .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound)
                    })
                {
                    return Err(error);
                }
                let mut args = vec![
                    "--session-id".into(),
                    id.into(),
                    "--session-dir".into(),
                    file.unwrap().parent().unwrap().display().to_string(),
                ];
                for (flag, value) in [
                    ("--provider", &state["model"]["provider"]),
                    ("--model", &state["model"]["id"]),
                    ("--thinking", &state["thinkingLevel"]),
                ] {
                    if let Some(value) = value.as_str() {
                        args.extend([flag.into(), value.into()]);
                    }
                }
                Ok(args)
            }
        }
    }
    pub fn resolve(&self, id: &str, cwd: &Path) -> Result<PathBuf, HarnessError> {
        self.lookup(id, Some(cwd))
    }
    /// A session this host's Pi keeps for itself, by id: the index's record,
    /// else Pi's own folders (and the one its global settings name), never
    /// one a project's settings name. A fork's source: the only session file
    /// a fork, sandboxed or not, may be copied from.
    pub(super) fn locate(&self, id: &str) -> Result<PathBuf, HarnessError> {
        self.lookup(id, None)
    }
    fn lookup(&self, id: &str, cwd: Option<&Path>) -> Result<PathBuf, HarnessError> {
        let mut candidates = vec![];
        if let Some(file) = json_file(&self.key(id))["sessionFile"].as_str() {
            candidates.push(PathBuf::from(file));
        }
        if let Some(dir) = self.session_dir() {
            candidates.retain(|file| self.admits(file));
            return self.search(id, candidates, vec![dir]);
        }
        if let Some(file) = json_file(&self.legacy)["sessions"][id]["sessionFile"].as_str() {
            candidates.push(PathBuf::from(file));
        }
        let mut roots = vec![self.agent.join("sessions")];
        let mut settings = vec![self.agent.join("settings.json")];
        settings.extend(cwd.map(|cwd| cwd.join(".pi/settings.json")));
        for settings in settings {
            if let Some(root) = json_file(&settings)["sessionDir"].as_str() {
                let path = if let Some(rest) = root.strip_prefix("~/") {
                    crate::executable::home_or_current_dir().join(rest)
                } else if let Some(cwd) = cwd {
                    cwd.join(root)
                } else if Path::new(root).is_absolute() {
                    PathBuf::from(root)
                } else {
                    continue;
                };
                roots.push(path);
            }
        }
        self.search(id, candidates, roots)
    }
    fn search(
        &self,
        id: &str,
        candidates: Vec<PathBuf>,
        roots: Vec<PathBuf>,
    ) -> Result<PathBuf, HarnessError> {
        for file in candidates {
            if matches_id(&file, id) {
                let file = canonical(&file)?;
                if self.admits(&file) {
                    return Ok(file);
                }
            }
        }
        for root in roots {
            // Pi stores session files under per-project directories. No symlink traversal.
            let mut dirs = vec![(root, 0)];
            let mut seen = 0;
            while let Some((dir, depth)) = dirs.pop() {
                let Ok(entries) = std::fs::read_dir(dir) else {
                    continue;
                };
                for entry in entries.flatten() {
                    seen += 1;
                    if seen > 20000 {
                        return Err(HarnessError::Protocol(
                            "Pi session search exceeded 20000 entries".into(),
                        ));
                    }
                    let Ok(kind) = entry.file_type() else {
                        continue;
                    };
                    let path = entry.path();
                    if kind.is_dir() && depth < 2 {
                        dirs.push((path, depth + 1));
                    } else if kind.is_file()
                        && path.extension().is_some_and(|e| e == "jsonl")
                        && matches_id(&path, id)
                    {
                        return Ok(canonical(&path)?);
                    }
                }
            }
        }
        Err(HarnessError::Protocol(format!(
            "native session file for {id} was not found"
        )))
    }
}
/// `canonicalize`, minus the `\\?\` verbatim prefix it adds on Windows: the
/// path goes to Pi as `--session`, and Node handles the verbatim form
/// unreliably; a record compared against a plain spelling never matches it.
fn canonical(path: &Path) -> std::io::Result<PathBuf> {
    let path = path.canonicalize()?;
    Ok(if cfg!(windows) { plain(path) } else { path })
}
/// The plain drive or UNC form of a verbatim path.
fn plain(path: PathBuf) -> PathBuf {
    if let Some(rest) = path.to_str().and_then(|p| p.strip_prefix(r"\\?\")) {
        return match rest.strip_prefix(r"UNC\") {
            Some(share) => PathBuf::from(format!(r"\\{share}")),
            None => PathBuf::from(rest),
        };
    }
    path
}
fn json_file(path: &Path) -> Value {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or(Value::Null)
}
fn matches_id(path: &Path, id: &str) -> bool {
    use std::io::Read;
    let Ok(file) = std::fs::File::open(path) else {
        return false;
    };
    let mut line = String::new();
    if BufReader::new(file.take(65536))
        .read_line(&mut line)
        .is_err()
    {
        return false;
    }
    serde_json::from_str::<Value>(&line).is_ok_and(|v| v["type"] == "session" && v["id"] == id)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn verbatim_paths_lose_their_prefix() {
        assert_eq!(
            plain(PathBuf::from(r"\\?\C:\Users\me\s.jsonl")),
            PathBuf::from(r"C:\Users\me\s.jsonl")
        );
        assert_eq!(
            plain(PathBuf::from(r"\\?\UNC\host\share\s.jsonl")),
            PathBuf::from(r"\\host\share\s.jsonl")
        );
        assert_eq!(
            plain(PathBuf::from("/home/me/s.jsonl")),
            PathBuf::from("/home/me/s.jsonl")
        );
    }
    #[cfg(unix)]
    #[test]
    fn a_sandboxed_chats_store_keeps_and_finds_sessions_only_in_its_state_folder() {
        let dir = tempfile::tempdir().unwrap();
        let agent = dir.path().join("agent");
        let state = dir.path().join("state");
        std::fs::create_dir_all(agent.join("sessions/p")).unwrap();
        std::fs::create_dir_all(state.join("sessions/p")).unwrap();
        let line = |id: &str| format!("{{\"type\":\"session\",\"id\":\"{id}\"}}\n");
        // A session of the owner's, in Pi's own folder, and a link to it.
        let host = agent.join("sessions/p/host.jsonl");
        std::fs::write(&host, line("host")).unwrap();
        std::os::unix::fs::symlink(&host, state.join("sessions/p/link.jsonl")).unwrap();
        let mine = state.join("sessions/p/mine.jsonl");
        std::fs::write(&mine, line("mine")).unwrap();
        let open = Store::new(Some(dir.path().join("index")), Some(agent.clone()));
        assert_eq!(open.resolve("host", dir.path()).unwrap(), host.canonicalize().unwrap());
        let store = Store::new(None, Some(agent.clone())).confined(&state);
        assert_eq!(store.session_dir(), Some(state.join("sessions")));
        assert!(store.remember("host", &host).is_err());
        assert!(store.remember("x", &state.join("sessions/../../agent/x.jsonl")).is_err());
        assert!(store.resolve("host", dir.path()).is_err());
        store.remember("mine", &mine).unwrap();
        assert!(state.join("zend-sessions").is_dir());
        assert_eq!(store.resolve("mine", dir.path()).unwrap(), mine.canonicalize().unwrap());
        // An index entry naming the link is refused: it resolves outside.
        store.remember("host", &state.join("sessions/p/link.jsonl")).unwrap();
        assert!(store.resolve("host", dir.path()).is_err());
    }
    #[test]
    fn only_proven_empty_local_sessions_can_be_recreated_and_submission_revokes_it() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::new(Some(dir.path().join("index")), None);
        store.agent = dir.path().join("agent");
        store.legacy = dir.path().join("legacy.json");
        let file = dir.path().join("session.jsonl");
        let state = json!({"sessionId":"empty","sessionFile":file,"model":{"provider":"mock","id":"test"},"thinkingLevel":"off"});
        store.remember("empty", &file).unwrap();
        assert!(store.resume_args("empty", dir.path()).is_err());
        store.remember_empty(&state, dir.path()).unwrap();
        assert_eq!(
            store.resume_args("empty", dir.path()).unwrap(),
            vec![
                "--session-id",
                "empty",
                "--session-dir",
                dir.path().to_str().unwrap(),
                "--provider",
                "mock",
                "--model",
                "test",
                "--thinking",
                "off"
            ]
        );
        let other = tempfile::tempdir().unwrap();
        assert!(store.resume_args("empty", other.path()).is_err());
        store.mark_submitted("empty").unwrap();
        assert!(store.resume_args("empty", dir.path()).is_err());
        std::fs::write(&file, "{\"type\":\"session\",\"id\":\"empty\"}\n").unwrap();
        assert_eq!(
            store.resume_args("empty", dir.path()).unwrap()[0],
            "--session"
        );
        std::fs::remove_file(&file).unwrap();
        assert!(
            store.resume_args("empty", dir.path()).is_err(),
            "missing persisted history must not be recreated"
        );
    }

    #[test]
    fn an_explicit_steering_mode_in_either_settings_file_is_respected() {
        let dir = tempfile::tempdir().unwrap();
        let agent = dir.path().join("agent");
        let cwd = dir.path().join("project");
        std::fs::create_dir_all(&agent).unwrap();
        std::fs::create_dir_all(cwd.join(".pi")).unwrap();
        let store = Store::new(Some(dir.path().join("index")), Some(agent.clone()));
        assert!(!store.steering_mode_configured(&cwd));
        std::fs::write(
            agent.join("settings.json"),
            r#"{"retry":{"enabled":false}}"#,
        )
        .unwrap();
        assert!(!store.steering_mode_configured(&cwd));
        std::fs::write(
            cwd.join(".pi/settings.json"),
            r#"{"steeringMode":"one-at-a-time"}"#,
        )
        .unwrap();
        assert!(store.steering_mode_configured(&cwd));
        std::fs::remove_file(cwd.join(".pi/settings.json")).unwrap();
        std::fs::write(agent.join("settings.json"), r#"{"steeringMode":"all"}"#).unwrap();
        assert!(store.steering_mode_configured(&cwd));
    }

    #[test]
    fn resolves_legacy_and_native_files_and_rejects_wrong_identity() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session with spaces.jsonl");
        std::fs::write(&path, "{\"type\":\"session\",\"id\":\"old-id\"}\n").unwrap();
        // Windows expands short names on canonicalization; the resolved path
        // is its plain form, without the verbatim prefix (see `canonical`).
        let expected = canonical(&path).unwrap();
        let mut store = Store::new(Some(dir.path().join("index")), None);
        store.agent = dir.path().join("agent");
        store.legacy = dir.path().join("legacy.json");
        std::fs::write(
            &store.legacy,
            json!({"version":1,"sessions":{"old-id":{"sessionFile":path}}}).to_string(),
        )
        .unwrap();
        assert_eq!(store.resolve("old-id", dir.path()).unwrap(), expected);
        store.remember("old-id", &path).unwrap();
        std::fs::remove_file(&store.legacy).unwrap();
        assert_eq!(store.resolve("old-id", dir.path()).unwrap(), expected);
        store.remember("wrong-id", &path).unwrap();
        assert!(store.resolve("wrong-id", dir.path()).is_err());
    }
}
