//! A page of a plugin's own, pictured without being open: `plugin_status`
//! with `snapshot` and `page`.
//!
//! The window builds the page a second time, apart from what it shows: the
//! shell mounts a node for it past the window's edge (`plugins.stage`,
//! ui/screens/plugins/stage.luau), the plugin fills it as it fills its page
//! (the same class, the same `page` given), and zgx draws that node alone
//! into a texture (`Ui::draw`). Nothing of it needs the window to show the
//! page, to be in front, or to show at all.
//!
//! `state` stands in for what the plugin keeps (`zend.store`), for the
//! picture alone: the SDK reads it for the nodes built here and no others
//! (`plugin.<id>.stage`), and while it's up nothing is kept under those
//! keys (`Bridge::store_set`), so a page that writes back what it was
//! given changes nothing.

use serde_json::{Value as Json, json};
use zgx_script::Value;

use super::json_value;

/// The stage's node, as the shell names it.
pub const ID: &str = "plugin-stage";
/// A page's size when none is asked, in points, and what one may be.
const WIDTH: f64 = 1000.;
const HEIGHT: f64 = 760.;
const WIDTHS: (f64, f64) = (240., 2400.);
const HEIGHTS: (f64, f64) = (160., 4000.);
/// How many keys `state` may hold, and how large it may be as JSON.
const KEYS: usize = 64;
const STATE_BYTES: usize = 64 * 1024;

/// A page to picture, as an agent asked.
#[derive(Debug, Clone, PartialEq)]
pub struct Asked {
    pub page: String,
    pub width: f64,
    pub height: f64,
    pub state: serde_json::Map<String, Json>,
}

/// What `args` asks (`{ page, width?, height?, state? }`): none with no
/// page, the sizes held to what a page can be.
pub fn asked(args: &Json) -> Result<Option<Asked>, String> {
    let Some(page) = args["page"].as_str().filter(|p| !p.is_empty()) else {
        return Ok(None);
    };
    let size = |key: &str, default: f64, (least, most): (f64, f64)| match &args[key] {
        Json::Null => Ok(default),
        value => value
            .as_f64()
            .filter(|v| v.is_finite())
            .map(|v| v.round().clamp(least, most))
            .ok_or(format!("{key} is a number of points")),
    };
    let state = match &args["state"] {
        Json::Null => serde_json::Map::new(),
        Json::Object(state) => state.clone(),
        _ => return Err("state is an object: what the plugin keeps, by zend.store key".into()),
    };
    if state.len() > KEYS || Json::Object(state.clone()).to_string().len() > STATE_BYTES {
        return Err(format!("state is too large (at most {KEYS} keys, {} KB)", STATE_BYTES / 1024));
    }
    Ok(Some(Asked {
        page: page.to_string(),
        width: size("width", WIDTH, WIDTHS)?,
        height: size("height", HEIGHT, HEIGHTS)?,
        state,
    }))
}

impl Asked {
    /// As it rides the plugin's "where" call, and back (`from_call`).
    pub fn to_call(&self) -> Value {
        json_value(&json!({ "page": self.page, "width": self.width, "height": self.height, "state": self.state }))
    }

    pub fn from_call(call: &Json) -> Option<Self> {
        asked(call).ok().flatten()
    }

    /// What the shell builds (`plugins.stage`): `n` tells one picture from
    /// the last, so each is built anew.
    pub fn shown(&self, plugin: &str, title: &str, n: u64) -> Value {
        json_value(&json!({
            "plugin": plugin, "page": self.page, "title": title,
            "width": self.width, "height": self.height, "n": n,
        }))
    }

    /// What the plugin's SDK reads (`plugin.<id>.stage`).
    pub fn lent(&self) -> Value {
        json_value(&json!({ "page": self.page, "state": self.state }))
    }
}

/// Why `page` can't be pictured, when it isn't among `pages` (the plugin's
/// own, `{ id, title }` each): what it has instead.
pub fn unlisted(name: &str, page: &str, pages: &[Json]) -> String {
    let ids: Vec<&str> = pages.iter().filter_map(|p| p["id"].as_str()).collect();
    match ids.is_empty() {
        true => format!("{name} has no pages (zend.page adds one, with \"pages.add\")"),
        false => format!("{name} has no page {page:?}; its pages: {}", ids.join(", ")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_page_asks_for_no_stage() {
        assert_eq!(asked(&json!({})), Ok(None));
        assert_eq!(asked(&json!({ "page": "", "width": 500 })), Ok(None));
    }

    #[test]
    fn sizes_default_and_are_held_to_what_a_page_can_be() {
        let a = asked(&json!({ "page": "value" })).unwrap().unwrap();
        assert_eq!((a.page.as_str(), a.width, a.height, a.state.len()), ("value", WIDTH, HEIGHT, 0));
        let a = asked(&json!({ "page": "value", "width": 10, "height": 1e9 })).unwrap().unwrap();
        assert_eq!((a.width, a.height), (WIDTHS.0, HEIGHTS.1));
        let a = asked(&json!({ "page": "value", "width": 640.4 })).unwrap().unwrap();
        assert_eq!(a.width, 640.);
        assert!(asked(&json!({ "page": "value", "width": "wide" })).is_err());
    }

    #[test]
    fn state_is_an_object_of_bounded_size() {
        let a = asked(&json!({ "page": "value", "state": { "tab": "usage", "days": 7 } })).unwrap().unwrap();
        assert_eq!(a.state["tab"], "usage");
        assert!(asked(&json!({ "page": "value", "state": ["usage"] })).is_err());
        let big = "x".repeat(STATE_BYTES);
        assert!(asked(&json!({ "page": "value", "state": { "k": big } })).is_err());
    }

    #[test]
    fn what_was_asked_survives_the_plugin_s_call() {
        let a = asked(&json!({ "page": "value", "width": 800, "state": { "tab": "work" } })).unwrap().unwrap();
        let call = super::super::value_json(&a.to_call());
        assert_eq!(Asked::from_call(&call), Some(a));
    }

    #[test]
    fn a_page_that_isn_t_the_plugin_s_names_those_that_are() {
        let pages = [json!({ "id": "value", "title": "Value" }), json!({ "id": "log", "title": "Log" })];
        assert_eq!(unlisted("Value", "other", &pages), "Value has no page \"other\"; its pages: value, log");
        assert!(unlisted("Value", "other", &[]).starts_with("Value has no pages"));
    }
}
