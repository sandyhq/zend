//! Your plugins' previews in this window (`PluginPreview` with
//! `in_window`, the engine's `window_preview`): a changed version of one
//! of yours runs here instead of the version you keep, until you keep it or
//! revert. The list carries each with its plugin (`preview`); the bridge
//! loads it in place of the kept version (`Bridge::effective`) and says
//! here which run now, for the host's bar (`host_bar`): "Previewing
//! <name>", with Revert and Keep, as a template's preview has. One that
//! failed shows as failed, the kept version running again, with Revert
//! alone.

use std::cell::RefCell;

/// A plugin previewing in the window, as its bar shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct Shown {
    pub id: String,
    pub name: String,
    /// The preview's hash: what Keep keeps (it refuses another).
    pub hash: String,
    /// Why it failed here, when it did (the kept version runs again).
    pub failed: Option<String>,
}

type Heard = Vec<Box<dyn Fn(&[Shown])>>;

zgx::tls::thread_local! {
    static NOW: RefCell<Vec<Shown>> = const { RefCell::new(Vec::new()) };
    static HEARD: RefCell<Heard> = RefCell::default();
}

pub fn current() -> Vec<Shown> {
    NOW.with(|n| n.borrow().clone())
}

/// What previews now; told to those listening when it changed.
pub fn set(list: Vec<Shown>) {
    let changed = NOW.with(|n| {
        let mut n = n.borrow_mut();
        let changed = *n != list;
        *n = list.clone();
        changed
    });
    if changed {
        HEARD.with(|h| h.borrow().iter().for_each(|f| f(&list)));
    }
}

/// `f` hears each change (the host's bar).
pub fn on_change(f: impl Fn(&[Shown]) + 'static) {
    HEARD.with(|h| h.borrow_mut().push(Box::new(f)));
}

/// The id of a plugin's bar: `#host-plugin-preview-<id>`.
pub fn bar_id(id: &str) -> String {
    format!("host-plugin-preview-{id}")
}
