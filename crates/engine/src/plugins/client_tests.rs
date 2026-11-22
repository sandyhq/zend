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


// A plugin's service, called from the phone: answered here for a plugin
// that's on and allowed, with what it was allowed here and nothing the call
// says; refused for one that isn't, and never sent on to another device.

const SERVICE: &str = r#"
local zend = require("./zend")("svc")
zend.serve("hello", function(args, call)
	return { hi = args.name, from = call.from.id }
end, { safe = true })
zend.serve("run", function()
	return zend.run("echo", { "hi" }):await().stdout
end)
"#;

/// `method` as this device's consent window sends it (allowing, signing).
async fn as_window(plugins: &Plugins, method: &str, params: serde_json::Value) -> serde_json::Value {
    let params = super::authority_tests::as_consent_window(plugins, method, params).await;
    match plugins.dispatch(method, params).await {
        Ok(RpcReply::Value(value)) => value,
        other => panic!("{method}: {:?}", other.err()),
    }
}

/// Plugin `svc` with a service and no permission to run programs: yours
/// (`signed`), or unsigned, as one copied in by hand.
async fn with_service(plugins: &Plugins, signed: bool) -> PathBuf {
    let dir = plugins.plugins_dir().join("svc");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("init.luau"), "return nil").unwrap();
    std::fs::write(dir.join("zend.luau"), scaffold::SDK).unwrap();
    std::fs::write(dir.join("service.luau"), SERVICE).unwrap();
    let manifest = serde_json::json!({
        "id": "svc", "name": "Svc", "version": "1.0.0", "entry": "init.luau",
        "permissions": ["service"], "service": "service.luau",
    });
    std::fs::write(dir.join(MANIFEST_FILE), signing::pretty(&manifest)).unwrap();
    if signed {
        let signed = as_window(plugins, methods::SIGN_PLUGIN, serde_json::json!({ "id": "svc" })).await;
        assert!(signed.get("hash").is_some(), "{signed}");
    }
    dir
}

/// Allowed in this device's consent window, as listed now.
async fn allow(plugins: &Plugins) {
    let item = as_window(plugins, methods::LIST_PLUGINS, serde_json::json!({})).await["plugins"][0].clone();
    let decided = as_window(plugins, methods::DECIDE_PLUGIN, serde_json::json!({ "id": "svc", "allow": true, "hash": item["hash"] })).await;
    assert_eq!(decided["status"], "on", "{decided}");
}

async fn phone_asks(plugins: &Plugins, params: serde_json::Value) -> serde_json::Value {
    as_phone(plugins, methods::CALL_PLUGIN_SERVICE, params).await.unwrap_or_else(|e| serde_json::json!({ "refused": e }))
}

#[tokio::test]
async fn a_phone_calls_the_service_of_a_plugin_allowed_here() {
    let f = fixture();
    let hello = serde_json::json!({ "plugin": "svc", "method": "hello", "args": { "name": "Ada" } });
    // Not installed.
    assert_eq!(phone_asks(&f.plugins, hello.clone()).await["reason"], reason::NOT_INSTALLED);
    // Waiting to be allowed.
    with_service(&f.plugins, true).await;
    assert_eq!(phone_asks(&f.plugins, hello.clone()).await["reason"], reason::NOT_APPROVED);
    allow(&f.plugins).await;
    // Allowed: answered here, as this device's own window's call.
    let answered = phone_asks(&f.plugins, hello.clone()).await;
    assert_eq!(answered, serde_json::json!({ "ok": true, "value": { "hi": "Ada", "from": "dev-1" } }));
    // With what it was allowed here, whatever the call says.
    let widened = phone_asks(
        &f.plugins,
        serde_json::json!({ "plugin": "svc", "method": "run", "permissions": ["commands"], "allowed": ["commands"], "grants": { "commands": true } }),
    )
    .await;
    assert_eq!(widened["reason"], reason::FAILED, "{widened}");
    assert!(widened["message"].as_str().unwrap().contains("wasn't allowed to run programs"), "{widened}");
    // Turned off here.
    as_window(&f.plugins, methods::SET_PLUGIN_OFF, serde_json::json!({ "id": "svc", "off": true })).await;
    assert_eq!(phone_asks(&f.plugins, hello.clone()).await["reason"], reason::OFF);
    as_window(&f.plugins, methods::SET_PLUGIN_OFF, serde_json::json!({ "id": "svc", "off": false })).await;
    assert_eq!(phone_asks(&f.plugins, hello).await["ok"], true);
}

#[tokio::test]
async fn a_phone_is_refused_a_plugin_changed_since_it_was_allowed() {
    let f = fixture();
    let hello = serde_json::json!({ "plugin": "svc", "method": "hello", "args": { "name": "Ada" } });
    // Unsigned, allowed as it was, then changed under you: tampered.
    let dir = with_service(&f.plugins, false).await;
    allow(&f.plugins).await;
    assert_eq!(phone_asks(&f.plugins, hello.clone()).await["ok"], true);
    std::fs::write(dir.join("init.luau"), "return 1").unwrap();
    f.plugins.rescan().await;
    let now = as_window(&f.plugins, methods::LIST_PLUGINS, serde_json::json!({})).await["plugins"][0].clone();
    assert_eq!(now["status"], "tampered", "{now}");
    let tampered = phone_asks(&f.plugins, hello).await;
    assert_eq!(tampered["reason"], reason::BROKEN, "{tampered}");
}

#[tokio::test]
async fn a_phone_reaches_this_device_s_services_only() {
    let f = fixture();
    with_service(&f.plugins, true).await;
    allow(&f.plugins).await;
    // Never sent on to another of this device's devices.
    let on = phone_asks(&f.plugins, serde_json::json!({ "plugin": "svc", "method": "hello", "device": "dev-9" })).await;
    assert_eq!(on["reason"], reason::BAD_CALL, "{on}");
    assert!(lock(&f.plugins.inner.watching).is_empty());
    let on = phone_asks(&f.plugins, serde_json::json!({ "plugin": "svc", "method": "@watch", "device": "dev-9" })).await;
    assert_eq!(on["reason"], reason::BAD_CALL, "{on}");
    assert!(lock(&f.plugins.inner.watching).is_empty());
    // This device by its own id is a call here.
    let here = phone_asks(&f.plugins, serde_json::json!({ "plugin": "svc", "method": "hello", "device": "dev-1", "args": { "name": "me" } })).await;
    assert_eq!(here["value"]["hi"], "me", "{here}");
}

