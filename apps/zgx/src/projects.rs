//! Adding a project, as the shipping app does (crates/ui shell/spaces.rs
//! `AddSpaceFlow`): a folder on one of your devices, browsed through the
//! engine (no folder dialog, this Mac's included), or a repository cloned
//! into one; and removing one. Each call names the device it's for and
//! whether that's another one (`remote`), which the engine reaches through
//! the relay (`targetDeviceId`).
//!
//!   projects.folders { device, remote, path? }   `{ path, entries = { { name,
//!                                                is_dir, is_repo } }, truncated }`
//!                                                (no `path`: its home)
//!   projects.drives { device, remote }           `{ { name, path } }`
//!   projects.search_home { device, remote, query } `{ matches = { { path,
//!                                                relative, is_repo } }, indexing }`
//!                                                (an empty query warms it)
//!   projects.release_home { device, remote }     its home search let go
//!   projects.clone { device, remote, url, parent } the repository's `{ path,
//!                                                name }`, cloned under `parent`
//!   projects.create { device, path, git, repo? } the new project's id
//!   projects.delete id                           gone, its sessions with it
//!
//! A refusal comes back as the engine's own words, for the flow to say.

use serde_json::{Value as Json, json};
use zgx_script::{ScriptHost, Value};

use crate::{auth::text, engine};

pub fn install(host: &ScriptHost) {
    host.async_action("projects.folders", |arg| async move {
        let mut params = json!({});
        if let Some(path) = field(&arg, "path") {
            params["path"] = Json::from(path);
        }
        let found = call("ListFolders", params, &arg).await?;
        let entries: Vec<Json> = found["entries"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|e| {
                json!({
                    "name": e["name"],
                    "is_dir": e["isDir"].as_bool().unwrap_or(false),
                    "is_repo": e["isRepo"].as_bool().unwrap_or(false),
                })
            })
            .collect();
        Ok(value(&json!({ "path": found["path"], "entries": entries, "truncated": found["truncated"].as_bool().unwrap_or(false) })))
    });
    host.async_action("projects.drives", |arg| async move {
        let found = call("ListDrives", json!({}), &arg).await?;
        Ok(value(&found["drives"]))
    });
    host.async_action("projects.search_home", |arg| async move {
        let query = field(&arg, "query").unwrap_or_default();
        let found = call("SearchHomeFolders", json!({ "query": query, "limit": 20 }), &arg).await?;
        let matches: Vec<Json> = found["matches"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|m| json!({ "path": m["path"], "relative": m["relative"], "is_repo": m["isRepo"].as_bool().unwrap_or(false) }))
            .collect();
        Ok(value(&json!({ "matches": matches, "indexing": found["indexing"].as_bool().unwrap_or(false) })))
    });
    host.async_action("projects.release_home", |arg| async move {
        let _ = call("ReleaseHomeFolderSearch", json!({}), &arg).await;
        Ok(Value::Nil)
    });
    host.async_action("projects.clone", |arg| async move { crate::plugins::preview::refuse("projects.clone")?;
        let url = field(&arg, "url").ok_or("Type the repository's address")?;
        let parent = field(&arg, "parent").ok_or("Choose where to clone it")?;
        let repo = call("CloneRepo", json!({ "url": url, "parent": parent }), &arg).await?;
        Ok(value(&json!({ "path": repo["path"], "name": repo["name"] })))
    });
    host.async_action("projects.create", |arg| async move {
        let device = field(&arg, "device").ok_or("Choose a device")?;
        let path = field(&arg, "path").ok_or("Choose a folder")?;
        let id = crate::sessions::new_id();
        let params = create_params(&id, &device, &path, arg.field("git").is_some_and(Value::truthy), field(&arg, "repo"));
        engine::get().call("Mutate", params).await?;
        Ok(Value::from(id.as_str()))
    });
    host.async_action("projects.delete", |id| async move {
        crate::plugins::preview::refuse("projects.delete")?;
        let id = text(&id);
        if id.is_empty() {
            return Err("no project".into());
        }
        engine::get().call("Mutate", json!({ "op": "deleteSpace", "spaceId": id })).await?;
        Ok(Value::Nil)
    });
}

/// `method` on the device `arg` names: through the relay when it's another.
async fn call(method: &str, params: Json, arg: &Value) -> Result<Json, String> {
    engine::get().call(method, aimed(params, arg)).await
}

/// params aimed at the device arg names when that's another one (remote):
/// the engine sends it there (a paired server too, under its projects.manage
/// grant), never to this one's own.
fn aimed(mut params: Json, arg: &Value) -> Json {
    if arg.field("remote").is_some_and(Value::truthy)
        && let Some(device) = field(arg, "device")
    {
        params["targetDeviceId"] = Json::from(device);
    }
    params
}

/// createSpace for a folder on a device: named by its device (the new id is
/// no server's yet, so the engine routes it by the device).
fn create_params(id: &str, device: &str, path: &str, git: bool, repo: Option<String>) -> Json {
    let mut params = json!({ "op": "createSpace", "spaceId": id, "deviceId": device, "path": path, "gitDetected": git });
    if let Some(repo) = repo {
        params["githubRepo"] = Json::from(repo);
    }
    params
}

fn field(arg: &Value, key: &str) -> Option<String> {
    arg.field(key).map(text).filter(|s| !s.trim().is_empty())
}

fn value(json: &Json) -> Value {
    Value::from_json(&json.to_string()).unwrap_or(Value::Nil)
}

#[cfg(test)]
mod tests {
    /// A remote device's calls name it (the engine sends them there: a
    /// paired server's too), this one's don't.
    #[test]
    fn another_devices_calls_name_it() {
        let arg = |remote: bool| zgx_script::Value::from_json(&format!(r#"{{"device":"box","remote":{remote}}}"#)).unwrap();
        let params = |remote: bool| super::aimed(serde_json::json!({ "path": "/x" }), &arg(remote));
        assert_eq!(params(true)["targetDeviceId"], "box");
        assert_eq!(params(true)["path"], "/x");
        assert!(params(false).get("targetDeviceId").is_none());
    }

    /// A project made on a server names its device and a new project id,
    /// never a targetDeviceId (the engine routes createSpace by its device:
    /// peer_view, plan step 6c).
    #[test]
    fn a_project_made_on_a_server_names_its_device() {
        let params = super::create_params("p1", "srv", "/home/me/notes", true, Some("acme/notes".into()));
        assert_eq!(params["op"], "createSpace");
        assert_eq!(params["deviceId"], "srv");
        assert_eq!(params["spaceId"], "p1");
        assert_eq!(params["gitDetected"], true);
        assert_eq!(params["githubRepo"], "acme/notes");
        assert!(params.get("targetDeviceId").is_none());
        assert!(super::create_params("p1", "srv", "/x", false, None).get("githubRepo").is_none());
    }
}
