//! Live visuals (plans/live-visuals G3): what Zend keeps of an agent's
//! visual beyond its script, so the agent can update it in place
//! (`update_visual`) and read what the person entered (`read_visual`).
//!
//! Beside each visual's script in the uploads (uploads.rs) sit:
//! - `{id}-entries.json`: what the person entered, crates/visual `Entries`,
//!   saved by the window (`SaveVisualEntries`). The agent never writes an
//!   entry: they are the person's draft.
//! - `{id}-visual-meta.json`: [`VisualMeta`], the chat that showed it, its
//!   title, how often it was updated, its named inputs, and whether its
//!   entries were sent (the window says so: `MarkVisualSent`).
//!
//! An update swaps the script the way the window does (crates/visual
//! `Visual::swap`): entries kept, an input the new script drops keeps its
//! entry, and an input new with it is `fresh` until the person touches it.

use serde::{Deserialize, Serialize};
use zend_visual as zv;

/// Entries are read, changed and written back under this lock, so an
/// update's fresh marks and the window's save of what was typed don't
/// overwrite one another.
pub(crate) static ENTRIES_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// What Zend keeps of a visual beside its script.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VisualMeta {
    pub id: String,
    /// The chat that showed it: the only one that may update or read it.
    pub chat_id: String,
    pub title: String,
    /// How often the agent updated it (0: as shown).
    #[serde(default)]
    pub rev: u64,
    /// The person sent its entries in a message (the window decides:
    /// `MarkVisualSent`). From then on they bind.
    #[serde(default)]
    pub sent: bool,
    /// When it was shown (ms since the epoch; 0 for one kept before this
    /// was): the newest open ones are named first.
    #[serde(default)]
    pub shown_at: i64,
    /// Its named inputs, as its latest script first draws them.
    #[serde(default)]
    pub inputs: Vec<InputInfo>,
}

impl VisualMeta {
    /// Open (plans/live-visuals D4): it asks the person something and they
    /// haven't sent their answers yet.
    pub fn is_open(&self) -> bool {
        !self.sent && !self.inputs.is_empty()
    }

    pub fn from_json(text: &str) -> Option<Self> {
        serde_json::from_str(text).ok()
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".into())
    }
}

/// One named input of a visual.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InputInfo {
    pub name: String,
    /// field, area, check, checklist, choice, select.
    pub kind: String,
    /// Its label, else its name.
    pub label: String,
    /// The options' values and labels, for those that have them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<OptionInfo>,
    /// Set by the agent: decided elsewhere.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answered: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OptionInfo {
    pub value: String,
    pub label: String,
}

fn info(input: &zv::Input) -> InputInfo {
    InputInfo {
        name: input.name.clone(),
        kind: input.kind.name().to_owned(),
        label: input.title().to_owned(),
        options: input
            .options
            .iter()
            .map(|o| OptionInfo {
                value: o.value.clone(),
                label: o.label.clone(),
            })
            .collect(),
        answered: input.answered.clone(),
    }
}

fn assets_of(assets: &[(String, Vec<u8>)]) -> zv::Assets {
    zv::Assets::new(
        assets
            .iter()
            .map(|(name, bytes)| (name.clone(), bytes.as_slice().into()))
            .collect(),
    )
}

/// The named inputs `source` first draws.
pub fn inputs_of(
    source: &str,
    seed: u64,
    assets: &[(String, Vec<u8>)],
) -> Result<Vec<InputInfo>, zv::VisualError> {
    let mut visual = zv::Visual::load(source, seed)?.with_assets(assets_of(assets));
    let tree = visual.render()?;
    Ok(zv::inputs(&tree).into_iter().map(info).collect())
}

/// What an update did.
#[derive(Debug, Clone, PartialEq)]
pub struct Swapped {
    /// The new script's inputs.
    pub inputs: Vec<InputInfo>,
    /// Names it brings that the last script didn't have.
    pub added: Vec<String>,
    /// Names it drops that hold an entry (kept, shown again if a later
    /// script brings the name back).
    pub dropped_with_entries: Vec<String>,
    /// The entries after it: the same values, the new names fresh.
    pub entries: zv::Entries,
}

/// Swap `old` for `new` over `entries`, as the window does.
pub fn swap(
    old: &str,
    new: &str,
    seed: u64,
    assets: &[(String, Vec<u8>)],
    entries: zv::Entries,
    before: &[InputInfo],
) -> Result<Swapped, zv::VisualError> {
    let files = assets_of(assets);
    // The old script's render first, so the entries know its names; one
    // that no longer runs is simply skipped.
    let mut visual = match zv::Visual::load(old, seed) {
        Ok(visual) => {
            let mut visual = visual.with_assets(files.clone()).with_entries(entries.clone());
            let _ = visual.render();
            visual
        }
        Err(_) => zv::Visual::load(new, seed)?.with_assets(files.clone()).with_entries(entries.clone()),
    };
    visual.swap(new)?;
    let tree = visual.render()?;
    let inputs: Vec<InputInfo> = zv::inputs(&tree).into_iter().map(info).collect();
    let entries = visual.entries();
    let had = |name: &str| before.iter().any(|i| i.name == name);
    let has = |name: &str| inputs.iter().any(|i| i.name == name);
    Ok(Swapped {
        added: inputs.iter().filter(|i| !had(&i.name)).map(|i| i.name.clone()).collect(),
        dropped_with_entries: before
            .iter()
            .filter(|i| !has(&i.name) && entries.get(&i.name).is_some())
            .map(|i| i.name.clone())
            .collect(),
        inputs,
        entries,
    })
}

/// A visual as `read_visual` and `update_visual` give it to the agent: its
/// inputs, each with the person's entry (if any) and whether it's new, the
/// entries by name (dropped inputs' too), and whether it's open or sent.
pub fn describe(meta: &VisualMeta, entries: &zv::Entries) -> serde_json::Value {
    let inputs: Vec<serde_json::Value> = meta
        .inputs
        .iter()
        .map(|input| {
            let mut v = serde_json::to_value(input).unwrap_or_default();
            if let Some(entry) = entries.get(&input.name) {
                v["entry"] = serde_json::to_value(entry).unwrap_or_default();
            }
            if entries.is_fresh(&input.name) {
                v["new"] = true.into();
            }
            v
        })
        .collect();
    serde_json::json!({
        "id": meta.id,
        "title": meta.title,
        "rev": meta.rev,
        "open": meta.is_open(),
        "sent": meta.sent,
        "inputs": inputs,
        "entries": entries.values(),
    })
}

/// What the agent is told with every read and update.
pub const ENTRIES_NOTE: &str = "Entries are the person's draft, read-only: \
don't answer, decide or ask again what they are filling in, and don't put \
their entries into a script (Zend keeps them across updates). They bind only \
once sent is true.";

#[cfg(test)]
mod tests {
    use super::*;

    const FORM: &str = r#"
return function()
  return ui.column {
    ui.field { name = "goal", label = "Goal" },
    ui.check { name = "backup", label = "Back up first" },
  }
end"#;
    const NEXT: &str = r#"
return function()
  return ui.column {
    ui.field { name = "goal", label = "Goal", value = "from the script" },
    ui.choice { name = "when", label = "When", options = { "Now", "Tonight" } },
  }
end"#;

    #[test]
    fn an_update_keeps_entries_marks_the_new_input_and_names_the_dropped_one() {
        let before = inputs_of(FORM, 1, &[]).unwrap();
        assert_eq!(
            before.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(),
            ["goal", "backup"]
        );
        let mut entries = zv::Entries::new();
        entries.set("goal", Some(zv::EntryValue::Text("ship it".into())));
        entries.set("backup", Some(zv::EntryValue::Bool(true)));
        let swapped = swap(FORM, NEXT, 1, &[], entries, &before).unwrap();
        assert_eq!(swapped.added, ["when"]);
        assert_eq!(swapped.dropped_with_entries, ["backup"]);
        assert_eq!(
            swapped.entries.get("goal"),
            Some(&zv::EntryValue::Text("ship it".into()))
        );
        assert_eq!(swapped.entries.get("backup"), Some(&zv::EntryValue::Bool(true)));
        assert!(swapped.entries.is_fresh("when"));
        assert!(!swapped.entries.is_fresh("goal"));

        let meta = VisualMeta {
            id: "v:1".into(),
            chat_id: "c".into(),
            title: "Plan".into(),
            rev: 1,
            sent: false,
            shown_at: 0,
            inputs: swapped.inputs.clone(),
        };
        let told = describe(&meta, &swapped.entries);
        assert_eq!(told["open"], true);
        assert_eq!(told["sent"], false);
        assert_eq!(told["inputs"][0]["entry"], "ship it");
        assert_eq!(told["inputs"][1]["new"], true);
        assert_eq!(told["entries"]["backup"], true);
    }

    #[test]
    fn a_visual_without_inputs_or_sent_is_not_open() {
        let mut meta = VisualMeta::default();
        assert!(!meta.is_open());
        meta.inputs = inputs_of(FORM, 1, &[]).unwrap();
        assert!(meta.is_open());
        meta.sent = true;
        assert!(!meta.is_open());
        assert_eq!(VisualMeta::from_json(&meta.to_json()), Some(meta));
    }
}
