//! Settings → Themes and → Templates on a paired server: its list, use and
//! delete, asked of it over the peer link (`targetDeviceId`, scope `settings`)
//! and never worn or installed here. This device's own themes and templates
//! are `themes.rs` and `plugins/live.rs`, unchanged; consent, import and
//! signing of a template stay local (a server never takes them: they are
//! `full` or never on its side).
//!
//! ```text
//! call  peer_looks.list    { device, kind }       kind = "themes" | "templates": the server's list
//! call  peer_looks.use     { device, kind, id }   a theme worn for both schemes there, a template run there
//! call  peer_looks.delete  { device, kind, id }   a theme or template of its own (the server refuses a built-in one)
//! ```
//!
//! A server that doesn't grant `settings` is asked nothing: the call fails
//! with `not_granted` (the page has said so already, with the command to run
//! on the server, and never shows this as an error). A server that can't be
//! reached fails with `offline`.

use serde_json::{Value as Json, json};
use zgx_script::{ScriptHost, Value};

use crate::auth::text;
use crate::engine;
use crate::plugins::{json_value, remote};

/// What an action asks of the server: the method and its params, addressed.
/// `Err(`not_granted`)` for a server that doesn't grant the area, and an
/// error for what isn't asked.
fn request(kind: &str, op: &str, device: &str, id: Option<&str>) -> Result<(&'static str, Json), String> {
    if device.is_empty() {
        return Err("Say which device.".into());
    }
    if crate::sessions::server_refuses(device, remote::AREA) {
        return Err("not_granted".into());
    }
    let method = match (kind, op) {
        ("themes", "list") => "ListThemes",
        ("themes", "use") => "UseTheme",
        ("themes", "delete") => "DeleteTheme",
        ("templates", "list") => "ListTemplates",
        ("templates", "use") => "UseTemplate",
        ("templates", "delete") => "DeleteTemplate",
        _ => return Err(format!("{op} isn't asked of {kind}")),
    };
    let mut params = json!({});
    if op != "list" {
        let id = id.map(str::trim).filter(|id| !id.is_empty()).ok_or("Say which one.")?;
        params["id"] = json!(id);
        if (kind, op) == ("themes", "use") {
            params["slot"] = json!("both");
        }
    }
    Ok((method, remote::addressed(Some(device), params)))
}

async fn ask(arg: Value, op: &'static str) -> Result<Value, String> {
    let device = remote::device_of(&arg).unwrap_or_default();
    let kind = arg.field("kind").map(text).unwrap_or_default();
    let id = arg.field("id").map(text);
    let (method, params) = request(&kind, op, &device, id.as_deref())?;
    match engine::get().call(method, params).await {
        Ok(reply) => Ok(json_value(&reply)),
        Err(error) if remote::unreachable(&error) => Err("offline".into()),
        Err(error) => Err(error),
    }
}

pub fn install(host: &ScriptHost) {
    host.async_action("peer_looks.list", |arg| ask(arg, "list"));
    host.async_action("peer_looks.use", |arg| ask(arg, "use"));
    host.async_action("peer_looks.delete", |arg| ask(arg, "delete"));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn devices() {
        crate::sessions::set_devices_for_tests(json!([
            { "id": "here", "name": "Mac" },
            { "id": "ai", "capabilities": ["paired-server"], "peerAreas": ["sessions.read", "files"] },
            { "id": "granted", "capabilities": ["paired-server"], "peerAreas": ["settings"] },
            { "id": "other", "capabilities": ["agent-updates"] },
        ]));
    }

    /// Each call names the server's device and its method, and nothing but
    /// what the call needs.
    #[test]
    fn a_server_that_grants_settings_is_asked_by_its_device() {
        devices();
        let (m, p) = request("themes", "list", "granted", None).unwrap();
        assert_eq!((m, p), ("ListThemes", json!({ "targetDeviceId": "granted" })));
        let (m, p) = request("themes", "use", "granted", Some("nord")).unwrap();
        assert_eq!((m, p), ("UseTheme", json!({ "targetDeviceId": "granted", "id": "nord", "slot": "both" })));
        let (m, p) = request("themes", "delete", "granted", Some("nord")).unwrap();
        assert_eq!((m, p), ("DeleteTheme", json!({ "targetDeviceId": "granted", "id": "nord" })));
        let (m, p) = request("templates", "list", "granted", None).unwrap();
        assert_eq!((m, p), ("ListTemplates", json!({ "targetDeviceId": "granted" })));
        let (m, p) = request("templates", "use", "granted", Some("mine")).unwrap();
        assert_eq!((m, p), ("UseTemplate", json!({ "targetDeviceId": "granted", "id": "mine" })));
        let (m, p) = request("templates", "delete", "granted", Some("mine")).unwrap();
        assert_eq!((m, p), ("DeleteTemplate", json!({ "targetDeviceId": "granted", "id": "mine" })));
        // (Another device, not a paired server: the engine's to answer.)
        assert!(request("themes", "list", "other", None).is_ok());
    }

    /// Never to this device's own engine (no device), never what isn't a list,
    /// use or delete (import, consent: local), never without an id.
    #[test]
    fn nothing_is_asked_without_a_device_an_id_or_for_a_local_only_op() {
        devices();
        assert!(request("themes", "list", "", None).is_err());
        assert!(request("themes", "use", "granted", None).is_err());
        assert!(request("templates", "delete", "granted", Some(" ")).is_err());
        assert!(request("templates", "import", "granted", Some("x")).is_err());
        assert!(request("plugins", "list", "granted", None).is_err());
    }

    /// A server without the `settings` grant is asked nothing.
    #[test]
    fn a_server_without_the_settings_grant_is_not_asked_for_themes_or_templates() {
        devices();
        for kind in ["themes", "templates"] {
            for op in ["list", "use", "delete"] {
                assert_eq!(request(kind, op, "ai", Some("x")), Err("not_granted".to_string()), "{kind} {op}");
            }
        }
    }
}
