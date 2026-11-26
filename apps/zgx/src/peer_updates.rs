//! Settings → About & updates for a paired server: the update status of its
//! engine, and updating it, asked of it over the peer link (`targetDeviceId`,
//! scope `updates`). This device's own About page (src/about.rs) is
//! unchanged and shows when no server is chosen.
//!
//! ```text
//! shared `about_device`   { device, state, current?, latest?, available, checked_at?, error? }
//!                         state: "idle" (no server chosen) | "refused" (it doesn't grant
//!                         `updates`: nothing is asked) | "looking" | "ready"
//! action peer_updates.show(device)    watch that server's `UpdateStatus` ("" = none); asked
//!                                     again, it reads the status again (the engine there checks
//!                                     the release feed itself: there's no way to ask for a check)
//! call   peer_updates.apply { device }  `ApplyUpdate` there; rejects with not_granted, offline,
//!                                     unmanaged, current or failed
//! ```

use std::cell::RefCell;
use std::rc::Rc;

use serde_json::{Value as Json, json};
use zgx_script::{ScriptHost, Shared, Value};

use crate::auth::text;
use crate::engine::{self, rpc::Watch};
use crate::plugins::remote;

/// The area a server grants for updates.
pub(crate) const AREA: &str = "updates";

#[derive(Default)]
struct Device {
    /// The server shown ("" none).
    id: RefCell<String>,
    /// What its engine last said of its update.
    status: RefCell<Option<Json>>,
    watch: RefCell<Option<Watch>>,
    shared: RefCell<Vec<Shared>>,
}

zgx::tls::thread_local! {
    static DEVICE: Rc<Device> = Rc::default();
}

/// What the page is told, from the device, whether it grants `updates` and
/// what its engine said last.
fn view(device: &str, refused: bool, status: Option<&Json>) -> Value {
    let state = if device.is_empty() {
        "idle"
    } else if refused {
        "refused"
    } else if status.is_some() {
        "ready"
    } else {
        "looking"
    };
    let mut fields = vec![("device", Value::from(device)), ("state", Value::from(state))];
    let mut available = false;
    if let (Some(status), "ready") = (status, state) {
        if let Some(v) = status["currentVersion"].as_str() {
            fields.push(("current", Value::from(v)));
        }
        if let Some(v) = status["latestVersion"].as_str() {
            fields.push(("latest", Value::from(v)));
        }
        if let Some(at) = status["checkedAt"].as_i64() {
            fields.push(("checked_at", Value::from(at as f64)));
        }
        if let Some(e) = status["error"].as_str() {
            fields.push(("error", Value::from(e)));
        }
        available = status["updateAvailable"].as_bool().unwrap_or(false);
    }
    fields.push(("available", Value::Bool(available)));
    Value::record(fields)
}

impl Device {
    fn publish(&self) {
        let id = self.id.borrow().clone();
        let value = view(&id, crate::sessions::server_refuses(&id, AREA), self.status.borrow().as_ref());
        for shared in self.shared.borrow().iter() {
            shared.set(value.clone());
        }
    }
}

/// Shows that server (or none, ""): what was shown goes; a server that grants
/// `updates` is watched, one that doesn't is asked nothing.
fn show(device: &Rc<Device>, id: &str) {
    device.watch.borrow_mut().take();
    device.status.borrow_mut().take();
    *device.id.borrow_mut() = id.to_string();
    device.publish();
    if id.is_empty() || crate::sessions::server_refuses(id, AREA) {
        return;
    }
    let Some(rpc) = engine::get().current() else { return };
    let heard = Rc::downgrade(device);
    let want = id.to_string();
    let watch = rpc.watch("UpdateStatus", remote::addressed(Some(id), json!({})), move |item| {
        if let Some(device) = heard.upgrade()
            && *device.id.borrow() == want
        {
            *device.status.borrow_mut() = Some(item);
            device.publish();
        }
    });
    *device.watch.borrow_mut() = Some(watch);
}

/// The request for updating `device`: its params, or why nothing is sent.
fn apply_request(device: &str) -> Result<Json, String> {
    if device.is_empty() {
        return Err("Say which device.".into());
    }
    if crate::sessions::server_refuses(device, AREA) {
        return Err("not_granted".into());
    }
    Ok(remote::addressed(Some(device), json!({})))
}

pub fn install(host: &ScriptHost) {
    let device = DEVICE.with(Rc::clone);
    device.shared.borrow_mut().push(host.shared("about_device", view("", false, None)));
    let d = device.clone();
    host.action("peer_updates.show", move |id| show(&d, &text(&id)));
    // (A reconnect: the watch started again.)
    let weak = Rc::downgrade(&device);
    engine::get().on_connect(move |_| {
        if let Some(device) = weak.upgrade() {
            let id = device.id.borrow().clone();
            if !id.is_empty() {
                show(&device, &id);
            }
        }
    });
    host.async_action("peer_updates.apply", |arg| async move {
        let id = remote::device_of(&arg).unwrap_or_default();
        let params = apply_request(&id)?;
        match engine::get().call("ApplyUpdate", params).await {
            Ok(answer) => Ok(Value::record([("version", Value::from(answer["version"].as_str().unwrap_or("")))])),
            Err(error) if remote::unreachable(&error) => Err("offline".into()),
            Err(error) => Err(crate::about::apply_outcome(&error).to_string()),
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn devices() {
        crate::sessions::set_devices_for_tests(json!([
            { "id": "here", "name": "Mac" },
            { "id": "ai", "capabilities": ["paired-server"], "peerAreas": ["sessions.read", "logins"] },
            { "id": "granted", "capabilities": ["paired-server"], "peerAreas": ["updates"] },
        ]));
    }

    fn field(v: &Value, name: &str) -> Option<String> {
        v.field(name).map(text)
    }

    /// Updating a server is `updates`', and asked of it by its device: not
    /// without a device, and nothing is sent to one that doesn't grant it.
    #[test]
    fn a_server_is_updated_only_where_it_grants_updates() {
        devices();
        assert_eq!(apply_request("granted"), Ok(json!({ "targetDeviceId": "granted" })));
        assert_eq!(apply_request("ai"), Err("not_granted".to_string()));
        assert!(apply_request("").is_err());
    }

    #[test]
    fn the_page_is_told_what_the_server_said_or_that_it_refuses() {
        devices();
        let status = json!({ "currentVersion": "0.2.130", "latestVersion": "0.2.131", "updateAvailable": true, "checkedAt": 7 });
        let v = view("granted", false, Some(&status));
        assert_eq!(field(&v, "state").as_deref(), Some("ready"));
        assert_eq!(field(&v, "current").as_deref(), Some("0.2.130"));
        assert_eq!(field(&v, "latest").as_deref(), Some("0.2.131"));
        assert!(v.field("available").is_some_and(Value::truthy));
        assert_eq!(field(&view("granted", false, None), "state").as_deref(), Some("looking"));
        // (Said by the grants, whatever it said before.)
        let v = view("ai", true, Some(&status));
        assert_eq!(field(&v, "state").as_deref(), Some("refused"));
        assert!(field(&v, "current").is_none());
        assert_eq!(field(&view("", false, None), "state").as_deref(), Some("idle"));
    }
}
