//! What the reader enters in a visual's inputs, kept apart from its script.
//!
//! A visual's inputs (`ui.field`, `ui.area`, `ui.check`, `ui.checklist`,
//! `ui.choice`, `ui.select`) each have a `name`, unique in the script.
//! What the reader enters is kept here by that name, outside the script's
//! Lua state, so it survives every render, the VM going to sleep, a restart
//! (the host saves [`Entries::to_json`] beside the script) and the agent
//! replacing the script ([`crate::Visual::swap`]).
//!
//! The rules (plans/live-visuals design D1, D2):
//! - an entry the reader made wins over the `value` a script gives: a
//!   script's `value` is only where an input starts;
//! - the input that has focus shows what it showed when it took focus until
//!   it gives it up, whatever a new script says;
//! - an input a new script drops keeps its entry, and shows it again if a
//!   later script brings the name back;
//! - an input that first appears with a swapped script is `fresh` until the
//!   reader touches it.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

/// One input's value: a text (field, area, select, single choice), a
/// checkbox, or a list of option values (checklist, multiple choice).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum EntryValue {
    Bool(bool),
    Text(String),
    List(Vec<String>),
}

impl EntryValue {
    /// Whether this is the shape an input of `kind` takes.
    pub fn fits(&self, shape: Shape) -> bool {
        matches!(
            (self, shape),
            (Self::Bool(_), Shape::Bool) | (Self::Text(_), Shape::Text) | (Self::List(_), Shape::List)
        )
    }
}

/// What kind of value an input holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    Bool,
    Text,
    List,
}

/// The longest text one entry keeps.
pub const MAX_ENTRY_CHARS: usize = 8000;
/// The most entries one visual keeps (dropped inputs' included).
pub const MAX_ENTRIES: usize = 200;

/// A visual's entries, by input name.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Entries {
    /// What the reader entered.
    #[serde(default)]
    values: BTreeMap<String, EntryValue>,
    /// Every input name a script of this visual has shown.
    #[serde(default)]
    known: BTreeSet<String>,
    /// Inputs a swapped script brought that the reader hasn't touched.
    #[serde(default)]
    fresh: BTreeSet<String>,
    /// The input with focus and what it showed as it took it (not saved).
    #[serde(skip)]
    focus: Option<(String, Option<EntryValue>)>,
    /// The next render follows a swap: new names are fresh.
    #[serde(skip)]
    swapped: bool,
}

impl Entries {
    pub fn new() -> Self {
        Self::default()
    }

    /// Entries saved by [`Entries::to_json`]; anything unreadable is none.
    pub fn from_json(text: &str) -> Self {
        serde_json::from_str(text).unwrap_or_default()
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".into())
    }

    /// What the reader entered in `name`.
    pub fn get(&self, name: &str) -> Option<&EntryValue> {
        self.values.get(name)
    }

    /// Every entry, by name.
    pub fn values(&self) -> &BTreeMap<String, EntryValue> {
        &self.values
    }

    /// Keep what the reader entered in `name` (`None` clears it, back to
    /// the script's `value`). Touching an input makes it no longer fresh.
    pub fn set(&mut self, name: &str, value: Option<EntryValue>) {
        self.fresh.remove(name);
        match value {
            Some(value) => {
                if self.values.len() >= MAX_ENTRIES && !self.values.contains_key(name) {
                    return;
                }
                self.values.insert(name.to_owned(), clip_value(value));
            }
            None => {
                self.values.remove(name);
            }
        }
    }

    /// The input `name` took focus, showing `shown`; `None`: focus left.
    pub fn focus(&mut self, name: Option<&str>, shown: Option<EntryValue>) {
        self.focus = name.map(|name| (name.to_owned(), shown));
    }

    pub fn focused(&self) -> Option<&str> {
        self.focus.as_ref().map(|(name, _)| name.as_str())
    }

    pub fn is_fresh(&self, name: &str) -> bool {
        self.fresh.contains(name)
    }

    /// Names a swapped-in script brings are fresh after its first render.
    pub(crate) fn mark_swap(&mut self) {
        self.swapped = true;
    }

    /// A render showed these input names.
    pub(crate) fn saw(&mut self, names: &[String]) {
        let swapped = std::mem::take(&mut self.swapped);
        for name in names {
            if self.known.insert(name.clone()) && swapped {
                self.fresh.insert(name.clone());
            }
        }
    }

    /// What input `name` shows: the reader's entry, else (while it has
    /// focus) what it showed as it took focus, else the script's value.
    /// The second is whether the reader's entry is what shows.
    pub fn resolve(&self, name: &str, shape: Shape, script: EntryValue) -> (EntryValue, bool) {
        if let Some(kept) = self.values.get(name).filter(|v| v.fits(shape)) {
            return (kept.clone(), true);
        }
        if let Some((_, Some(held))) = self.focus.as_ref().filter(|(n, _)| n == name)
            && held.fits(shape)
        {
            return (held.clone(), false);
        }
        (script, false)
    }
}

fn clip_value(value: EntryValue) -> EntryValue {
    let clip = |mut text: String| {
        if text.len() > MAX_ENTRY_CHARS {
            let mut end = MAX_ENTRY_CHARS;
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            text.truncate(end);
        }
        text
    };
    match value {
        EntryValue::Text(text) => EntryValue::Text(clip(text)),
        EntryValue::List(items) => EntryValue::List(items.into_iter().take(200).map(clip).collect()),
        other => other,
    }
}
