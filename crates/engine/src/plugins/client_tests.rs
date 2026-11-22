//! A linked phone (`Caller::Remote`) shows the desktop's plugins: it gets
//! the list without this device's paths, and each plugin's files by id and
//! relative path, never anything outside the plugin's checked snapshot.

use super::*;

use base64::Engine as _;

struct Fixture {
    _dir: tempfile::TempDir,
    data: PathBuf,
    plugins: Plugins,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().to_path_buf();
    let plugins = Plugins::new(&data, "dev-1");
    std::fs::create_dir_all(plugins.plugins_dir()).unwrap();
    Fixture { _dir: dir, data, plugins }
}

/// `method` as the phone link serves it: refused outside its allow-list,
/// else answered as `Caller::Remote`.
async fn as_phone(plugins: &Plugins, method: &str, params: serde_json::Value) -> Result<serde_json::Value, String> {
    if !zend_rpc::remote::remote_allowed(method, &params) {
        return Err(format!("{method} isn't open to a linked phone"));
    }
    match zend_rpc::with_caller(zend_rpc::Caller::Remote, plugins.dispatch(method, params)).await {
        Ok(RpcReply::Value(value)) => Ok(value),
        Ok(RpcReply::Stream(mut stream)) => Ok(stream.next().await.expect("a first item")),
        Err(error) => Err(error.to_string()),
    }
}

/// A plugin of yours that's on, with a module beside its entry and an asset.
async fn made(f: &Fixture) -> serde_json::Value {
    let made = match f.plugins.dispatch(methods::NEW_PLUGIN, serde_json::json!({ "id": "side", "name": "Side" })).await {
        Ok(RpcReply::Value(value)) => value,
        other => panic!("NewPlugin: {:?}", other.err()),
    };
    assert_eq!(made["status"], "on", "{made}");
    made
}

fn listed<'a>(list: &'a serde_json::Value, id: &str) -> &'a serde_json::Value {
    list["plugins"].as_array().unwrap().iter().find(|p| p["id"] == id).unwrap_or_else(|| panic!("no {id} in {list}"))
}

#[tokio::test]
async fn a_phone_lists_plugins_without_desktop_paths_and_reads_their_window_files() {
    let f = fixture();
    let made = made(&f).await;
    let data = f.data.display().to_string();
    for method in [methods::LIST_PLUGINS, methods::WATCH_PLUGINS] {
        let list = as_phone(&f.plugins, method, serde_json::json!({})).await.unwrap();
        let text = list.to_string();
        assert!(!text.contains(&data), "{method} told the phone this device's paths: {text}");
        assert!(list.get("dirs").is_none(), "{method}: {list}");
        let side = listed(&list, "side");
        assert_eq!(side["status"], "on");
        assert_eq!(side["hash"], made["hash"]);
        assert!(side.get("dir").is_none() && side.get("snapshot").is_none(), "{side}");
        // What it's made of, by relative path and hash: what the phone fetches.
        let files = side["files"].as_object().unwrap_or_else(|| panic!("{method}: no files in {side}"));
        assert!(files.contains_key("init.luau") && files.contains_key("zend.luau") && files.contains_key(MANIFEST_FILE), "{side}");
    }
    let list = as_phone(&f.plugins, methods::LIST_PLUGINS, serde_json::json!({})).await.unwrap();
    let side = listed(&list, "side");
    let hash = side["hash"].as_str().unwrap();
    for path in ["init.luau", "zend.luau", "locales/en.json", MANIFEST_FILE] {
        let read = as_phone(&f.plugins, "ReadPluginFile", serde_json::json!({ "id": "side", "hash": hash, "path": path }))
            .await
            .unwrap_or_else(|e| panic!("{path}: {e}"));
        let bytes = base64::engine::general_purpose::STANDARD.decode(read["data"].as_str().unwrap()).unwrap();
        assert_eq!(bytes, std::fs::read(f.plugins.plugins_dir().join("side").join(path)).unwrap(), "{path}");
        assert_eq!(read["sha256"], side["files"][path], "{path}");
        assert_eq!(signing::sha256_hex(&bytes), side["files"][path].as_str().unwrap(), "{path}");
    }
    // This device's own window still gets its paths.
    let RpcReply::Value(local) = f.plugins.dispatch(methods::LIST_PLUGINS, serde_json::json!({})).await.unwrap() else { panic!() };
    assert!(listed(&local, "side")["snapshot"].is_string());
}

#[tokio::test]
async fn a_phone_reads_nothing_outside_a_plugin_s_snapshot() {
    let f = fixture();
    let made = made(&f).await;
    let hash = made["hash"].as_str().unwrap().to_owned();
    std::fs::write(f.data.join("secret.txt"), "token").unwrap();
    let snapshot = PathBuf::from(made["snapshot"].as_str().unwrap());
    let read = |path: String, hash: String| {
        let plugins = f.plugins.clone();
        async move { as_phone(&plugins, "ReadPluginFile", serde_json::json!({ "id": "side", "hash": hash, "path": path })).await }
    };
    let outside = [
        "../secret.txt".to_string(),
        "../../secret.txt".into(),
        "locales/../../../secret.txt".into(),
        f.data.join("secret.txt").display().to_string(),
        snapshot.join("init.luau").display().to_string(),
        "/etc/passwd".into(),
        "".into(),
        ".".into(),
        "locales".into(),
        "missing.luau".into(),
        ".storage/side.json".into(),
        "C:\\Windows\\win.ini".into(),
        "init.luau/".into(),
        "./init.luau".into(),
    ];
    for path in outside {
        assert!(read(path.clone(), hash.clone()).await.is_err(), "{path:?} was read");
    }
    // Another plugin's id, one that isn't there, or a hash it no longer has.
    assert!(as_phone(&f.plugins, "ReadPluginFile", serde_json::json!({ "id": "../side", "hash": hash, "path": "init.luau" })).await.is_err());
    assert!(as_phone(&f.plugins, "ReadPluginFile", serde_json::json!({ "id": "nope", "hash": hash, "path": "init.luau" })).await.is_err());
    assert!(read("init.luau".into(), "0".repeat(64)).await.is_err());
    assert!(read("init.luau".into(), hash.clone()).await.is_ok());

    // A link planted in the snapshot (out to a secret) is never followed.
    let planted = snapshot.join("init.luau");
    let mut perms = std::fs::metadata(&snapshot).unwrap().permissions();
    #[allow(clippy::permissions_set_readonly_false)]
    perms.set_readonly(false);
    std::fs::set_permissions(&snapshot, perms).unwrap();
    std::fs::remove_file(&planted).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(f.data.join("secret.txt"), &planted).unwrap();
    #[cfg(unix)]
    {
        let refused = read("init.luau".into(), hash.clone()).await.unwrap_err();
        assert!(!refused.contains("token"), "{refused}");
    }
    // A file changed in place (not what was checked) isn't sent either.
    std::fs::write(snapshot.join("zend.luau"), "changed").unwrap_or_else(|_| {
        let p = snapshot.join("zend.luau");
        let mut perms = std::fs::metadata(&p).unwrap().permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        perms.set_readonly(false);
        std::fs::set_permissions(&p, perms).unwrap();
        std::fs::write(&p, "changed").unwrap();
    });
    assert!(read("zend.luau".into(), hash.clone()).await.is_err());
}
