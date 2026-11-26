//! API keys per paired server (plan step 4d-3): a chat another server
//! starts here confined runs on a key the owner set for that server and the
//! model's provider (`zend peer key set <server> <provider>`), never on the
//! owner's own logins. Kept in the data dir, `peer-keys.json`, owner-only
//! (0600): never synced, never answered over any connection (listing gives
//! the providers' names only). Unpairing a server forgets its keys.
//!
//! Read at each run: a key taken away (`rm`, unpair) refuses the next run
//! at once, and ends its server's live runs that needed it.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub const FILE: &str = "peer-keys.json";

/// One change at a time.
static LOCK: Mutex<()> = Mutex::new(());

/// server key → provider → API key.
type Book = BTreeMap<String, BTreeMap<String, String>>;

#[derive(Clone, Debug)]
pub struct PeerKeys {
    path: PathBuf,
}

/// The provider a run of `harness` on `model` signs in with: the prefix of
/// a `provider/model` id (Pi's), else the harness's own provider.
pub fn provider_of(harness: zend_proto::HarnessId, model: Option<&str>) -> Option<String> {
    if let Some((provider, _)) = model.and_then(|m| m.split_once('/')).filter(|(p, _)| !p.is_empty()) {
        return Some(provider.to_string());
    }
    match harness {
        zend_proto::HarnessId::ClaudeCode => Some("anthropic".into()),
        zend_proto::HarnessId::Codex => Some("openai".into()),
        _ => None,
    }
}

fn valid_provider(provider: &str) -> bool {
    !provider.is_empty()
        && provider.len() <= 64
        && provider.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

impl PeerKeys {
    pub fn new(data_dir: &Path) -> Self {
        Self { path: data_dir.join(FILE) }
    }

    fn load(&self) -> Book {
        std::fs::read(&self.path)
            .ok()
            .and_then(|raw| serde_json::from_slice(&raw).ok())
            .unwrap_or_default()
    }

    fn change<T>(&self, f: impl FnOnce(&mut Book) -> Result<T, String>) -> Result<T, String> {
        let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut book = self.load();
        let out = f(&mut book)?;
        book.retain(|_, keys| !keys.is_empty());
        let bytes = serde_json::to_vec_pretty(&book).map_err(|e| e.to_string())?;
        crate::runner::write_private_atomic(&self.path, &bytes).map_err(|e| e.to_string())?;
        Ok(out)
    }

    /// Keep `key` for `peer`'s runs on `provider` (replacing one kept).
    /// A provider with no API-key variable is refused.
    pub fn set(&self, peer: &str, provider: &str, key: &str) -> Result<&'static str, String> {
        if !valid_provider(provider) {
            return Err(format!("{provider}: not a provider's name"));
        }
        let var = zend_harness::sandbox::provider_key_var(provider)
            .ok_or_else(|| format!("{provider}: no API key known for it (a provider that signs in only with a login can't be used)"))?;
        let key = key.trim();
        if key.is_empty() || key.len() > 4096 || key.chars().any(char::is_control) {
            return Err("the key is empty, too long or holds control characters".into());
        }
        self.change(|book| {
            book.entry(peer.to_string()).or_default().insert(provider.to_string(), key.to_string());
            Ok(var)
        })
    }

    /// Forget `peer`'s key for `provider`: whether there was one.
    pub fn remove(&self, peer: &str, provider: &str) -> Result<bool, String> {
        self.change(|book| Ok(book.get_mut(peer).and_then(|keys| keys.remove(provider)).is_some()))
    }

    /// Forget all of `peer`'s keys (it was unpaired).
    pub fn forget(&self, peer: &str) -> Result<(), String> {
        if !self.load().contains_key(peer) {
            return Ok(());
        }
        self.change(|book| {
            book.remove(peer);
            Ok(())
        })
    }

    /// The providers `peer` has a key for (names only).
    pub fn providers(&self, peer: &str) -> Vec<String> {
        self.load().get(peer).map(|keys| keys.keys().cloned().collect()).unwrap_or_default()
    }

    /// Every server's providers (names only).
    pub fn list(&self) -> BTreeMap<String, Vec<String>> {
        self.load().into_iter().map(|(peer, keys)| (peer, keys.into_keys().collect())).collect()
    }

    /// For a run of `peer`'s on `provider`: its variable and key, or why
    /// there is none.
    pub fn for_run(&self, peer: &str, provider: Option<&str>) -> Result<(String, String), String> {
        let provider = provider.ok_or(
            "no model with a known provider is chosen for it, and a paired server's chat runs only on the API key kept for that server and the model's provider",
        )?;
        let var = zend_harness::sandbox::provider_key_var(provider)
            .ok_or_else(|| format!("its model's provider {provider} has no API key to run on (a paired server's chat never uses this device's logins)"))?;
        let key = self.load().get(peer).and_then(|keys| keys.get(provider)).cloned().ok_or_else(|| {
            format!("no API key is kept for {peer} and {provider}: set one with `zend peer key set {peer} {provider}`")
        })?;
        Ok((var.to_string(), key))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_kept_owner_only_and_listed_by_name() {
        let dir = tempfile::tempdir().unwrap();
        let keys = PeerKeys::new(dir.path());
        assert_eq!(keys.set("srv", "anthropic", " sk-1\n").unwrap(), "ANTHROPIC_API_KEY");
        assert!(keys.set("srv", "openai-codex", "x").is_err());
        assert!(keys.set("srv", "Bad/Name", "x").is_err());
        assert!(keys.set("srv", "openai", "").is_err());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(dir.path().join(FILE)).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        assert_eq!(keys.providers("srv"), ["anthropic"]);
        assert!(!format!("{:?}", keys.list()).contains("sk-1"));
        assert_eq!(keys.for_run("srv", Some("anthropic")).unwrap(), ("ANTHROPIC_API_KEY".into(), "sk-1".into()));
        assert!(keys.for_run("srv", Some("openai")).unwrap_err().contains("zend peer key set"));
        assert!(keys.for_run("other", Some("anthropic")).is_err());
        assert!(keys.for_run("srv", None).is_err());
        assert!(keys.remove("srv", "anthropic").unwrap());
        assert!(keys.for_run("srv", Some("anthropic")).is_err());
        keys.set("srv", "openai", "k").unwrap();
        keys.forget("srv").unwrap();
        assert!(keys.list().is_empty());
        assert_eq!(provider_of(zend_proto::HarnessId::Pi, Some("anthropic/claude")), Some("anthropic".into()));
        assert_eq!(provider_of(zend_proto::HarnessId::Pi, None), None);
        assert_eq!(provider_of(zend_proto::HarnessId::ClaudeCode, Some("opus")), Some("anthropic".into()));
    }
}
