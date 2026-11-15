//! Plugin tools: what Zend plugins offer agents, and the two tools that
//! make and list plugins.
//!
//! A plugin tool's name is dynamic (`<plugin>__<tool>`, the engine's
//! `mcp_name`), so it is not in the static [`catalog`](super::catalog): the
//! listing asks the engine (`ListPluginTools`) every time, a call goes to
//! `CallPluginTool`, and [`Tools::watch_plugin_tools`] follows
//! `WatchPluginTools` so the server can tell the client its list changed.

use std::collections::BTreeSet;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use zend_rpc::{RpcError, methods};

use super::{ToolDef, Tools, parse};

pub(super) const CREATE_PLUGIN: &str = "create_plugin";
pub(super) const LIST_PLUGINS: &str = "list_plugins";
pub(super) const LIST_TEMPLATES: &str = "list_templates";
pub(super) const CREATE_TEMPLATE: &str = "create_template";
pub(super) const USE_TEMPLATE: &str = "use_template";
pub(super) const TEMPLATE_STATUS: &str = "template_status";
pub(super) const DELETE_TEMPLATE: &str = "delete_template";
pub(super) const DESCRIBE_APP: &str = "describe_app";
pub(super) const EXPORT_TEMPLATE: &str = "export_template";
pub(super) const IMPORT_TEMPLATE: &str = "import_template";
pub(super) const DESCRIBE_ELEMENTS: &str = "describe_elements";
pub(super) const PLUGIN_STATUS: &str = "plugin_status";
pub(super) const PLUGIN_LOGS: &str = "plugin_logs";
pub(super) const CHECK_PLUGIN: &str = "check_plugin";
pub(super) const PREVIEW_PLUGIN: &str = "preview_plugin";
pub(super) const PLUGIN_INPUT: &str = "plugin_input";
pub(super) const INSTALL_PLUGIN: &str = "install_plugin";
pub(super) const UPDATE_PLUGIN: &str = "update_plugin";
pub(super) const EXPORT_PLUGIN: &str = "export_plugin";

/// How long a listing waits for the engine's plugin tools before it goes
/// out with the static tools only.
const LIST_TIMEOUT: Duration = Duration::from_secs(5);
/// Gap before re-watching after the stream ends (an engine restart) or the
/// dial fails (no engine yet); doubles up to [`MAX_RETRY`].
const FIRST_RETRY: Duration = Duration::from_millis(300);
const MAX_RETRY: Duration = Duration::from_secs(30);

/// A tool definition with an owned name and description: the shape
/// `tools/list` sends, for static and plugin tools alike.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListedTool {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

impl From<ToolDef> for ListedTool {
    fn from(def: ToolDef) -> Self {
        Self {
            name: def.name.to_owned(),
            description: def.description.to_owned(),
            input_schema: def.input_schema,
        }
    }
}

/// `ListPluginTools` / `WatchPluginTools` reply.
#[derive(Debug, Clone, Default, Deserialize)]
struct PluginTools {
    #[serde(default)]
    device: PluginDevice,
    #[serde(default)]
    tools: Vec<PluginTool>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct PluginDevice {
    #[serde(default)]
    name: String,
}

#[derive(Debug, Clone, Deserialize)]
struct PluginTool {
    plugin: String,
    #[serde(default)]
    plugin_name: String,
    mcp_name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    input: Value,
}

impl PluginTools {
    fn names(&self) -> BTreeSet<String> {
        self.tools.iter().map(|t| t.mcp_name.clone()).collect()
    }

    fn listed(&self) -> Vec<ListedTool> {
        let device = match self.device.name.trim() {
            "" => "this device",
            name => name,
        };
        self.tools
            .iter()
            .map(|tool| {
                let plugin = match tool.plugin_name.trim() {
                    "" => tool.plugin.as_str(),
                    name => name,
                };
                let origin =
                    format!("(From the {plugin} plugin. Works while Zend is open on {device}.)");
                let description = match tool.description.trim() {
                    "" => origin,
                    text => format!("{text} {origin}"),
                };
                ListedTool {
                    name: tool.mcp_name.clone(),
                    description,
                    input_schema: input_schema(&tool.input),
                }
            })
            .collect()
    }
}

/// MCP wants an object schema; a manifest may leave `input` out or omit its
/// `type`.
fn input_schema(input: &Value) -> Value {
    match input {
        Value::Object(schema) => {
            let mut schema = schema.clone();
            schema
                .entry("type")
                .or_insert_with(|| Value::String("object".into()));
            Value::Object(schema)
        }
        _ => json!({ "type": "object", "properties": {} }),
    }
}

/// Plugin tools are named `<plugin>__<tool>` (the plugin id with `-` → `_`);
/// no static tool has `__` in its name.
pub(crate) fn is_plugin_tool_name(name: &str) -> bool {
    let word = |part: &str| {
        !part.is_empty()
            && part
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    };
    name.split_once("__")
        .is_some_and(|(plugin, tool)| word(plugin) && word(tool))
}

/// The engine's error in words an agent can act on.
fn plain(error: RpcError) -> String {
    match error {
        RpcError::Failed(message) => message,
        RpcError::UnknownMethod(_) => {
            "This Zend doesn't run plugins yet; update Zend to use them.".into()
        }
        other => other.to_string(),
    }
}

pub(super) fn catalog() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: CREATE_PLUGIN,
            description: "Create a Zend plugin on this device: a folder with a manifest (zend-plugin.json), init.luau to edit, and zend.luau, the SDK. Zend signs it as yours and loads it, and re-signs and reloads it as you save; the person allows once, in Zend, any permissions it asks for. `id` is lowercase letters, digits and dashes (2-40). `permissions` are what the plugin may do (ui.add, ui.change, ui.replace, profile.read, sessions.read, messages.read, account.email, files.read, panel.tabs, pages.add, github, github.write, navigate, compose, network, clipboard, files, web, windows, tools, devices, native, commands: with a manifest `commands` list of programs it runs, as you). The reply links the authoring guide (`guide`): read it before writing.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "pattern": "^[a-z0-9][a-z0-9-]{1,39}$", "description": "Folder and manifest id, e.g. focus." },
                    "name": { "type": "string", "description": "Display name, e.g. Focus." },
                    "description": { "type": "string" },
                    "permissions": { "type": "array", "items": { "type": "string" } }
                },
                "required": ["id", "name"]
            }),
        },
        ToolDef {
            name: LIST_PLUGINS,
            description: "Zend plugins on this device: id, name, status (on, off, pending review, tampered, broken), who made it, and its folder.",
            input_schema: json!({ "type": "object", "properties": {} }),
        },
        ToolDef {
            name: PLUGIN_STATUS,
            description: "Where a Zend plugin's UI landed, to check your work without a person: its status (on, off, pending review, tampered, broken, and why), what it was allowed, and, from Zend's window on this device, each zend.place it made (its selector, how, how many elements it matched and their boxes in the window), what its row in Settings says (a node that needs what its place doesn't give, places this template doesn't have, an error that stopped it) and its pages. With snapshot: true, also a PNG of the window cut to around its nodes: read the file at `snapshot` to look. With `page` too, the picture is of that page of the plugin's own instead (an id from `pages`), built apart from the window for the picture alone, so it needn't be open, nor the window in front: `width` and `height` are the page's size in points, and `state` stands in for what the plugin keeps (zend.store) as the page builds, nothing kept changing. Edit the plugin, save, and ask again.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "The plugin's id, from list_plugins." },
                    "snapshot": { "type": "boolean", "description": "Also save a picture of where it shows." },
                    "page": { "type": "string", "description": "With snapshot: picture this page of the plugin's own (its id, as in `pages`), whether or not it's open." },
                    "width": { "type": "number", "description": "With page: the page's width in points (default 1000, 240 to 2400)." },
                    "height": { "type": "number", "description": "With page: the page's height in points (default 760, 160 to 4000)." },
                    "state": { "type": "object", "description": "With page: values standing in for what the plugin keeps, by zend.store key (e.g. {\"tab\": \"usage\"}), read as the page builds. What it keeps doesn't change." },
                    "preview": { "type": "boolean", "description": "Ask its preview (preview_plugin) instead of Zend's window: the same account, from the preview's own window." }
                },
                "required": ["id"]
            }),
        },
        ToolDef {
            name: PREVIEW_PLUGIN,
            description: "Run a Zend plugin folder in a preview, to try a change before it reaches the person's window: a Zend of its own beside theirs, its own engine and a window that never shows on the screen and never takes focus, with a copy of the folder, what the plugin keeps copied from theirs, and allowed what they allowed it (a permission they never allowed stays unallowed). Their plugin, its grants and what it keeps stay as they are, whatever fails in the preview. Called again, the folder is copied in again (edit, then call it, then look). Look and act with plugin_status, plugin_logs and plugin_input, each with preview: true (plugin_input acts only in a preview). `stop` ends it; stop it when you're done (it also stops itself after 30 minutes nothing asked anything of it, and with the engine that started it). With in_window: true instead, `path` (a changed copy of one of the person's own plugins that's on, with the same id) runs in their own window in place of the version they keep, as a preview with a bar: \"Previewing <name>\", Revert and Keep. Keeping is theirs alone, on the bar: you can't keep it for them; ask them to look. If it fails there, the kept version runs again and its log says so. A version that asks for more than they allowed is refused (preview it on its own). Called again, the folder is copied in again; `stop` with in_window reverts it (the kept version, its grants and what it keeps come back as they were).",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "The plugin folder's full path." },
                    "id": { "type": "string", "description": "Or an installed plugin's id (from list_plugins): its folder." },
                    "stop": { "type": "boolean", "description": "End the preview of this plugin (with in_window: revert it in the person's window)." },
                    "in_window": { "type": "boolean", "description": "Run it in the person's own window, as a preview they keep or revert on its bar." }
                }
            }),
        },
        ToolDef {
            name: PLUGIN_INPUT,
            description: "Act in a plugin's preview window (preview_plugin), never in the person's: `click` or `hover` the node a selector finds, as zend.place and describe_elements name them (#id, .class, a text in quotes, role:button \"Name\", `a >> b` for b inside a, [n] or @n for the nth), or `set_state`: values for what the plugin keeps (zend.store keys). A click or hover lands at the node's middle, or at x, y from its top left; hover with x, y and no `at` is a point in the window. An error says why it can't (no such node, more than one, scrolled out of its clip, covered by another). Then picture it: plugin_status with snapshot and preview (e.g. a plugin page opened by clicking its title in the sidebar, \"Value\"; a snapshot when none of the plugin's nodes shows pictures the whole preview window, to find what to click).",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "The plugin's id: the preview's." },
                    "action": { "type": "string", "enum": ["click", "hover", "set_state"] },
                    "at": { "type": "string", "description": "click, hover: a selector." },
                    "x": { "type": "number", "description": "click, hover: points from the node's left (with at), else the window's." },
                    "y": { "type": "number", "description": "click, hover: points from the node's top (with at), else the window's." },
                    "state": { "type": "object", "description": "set_state: zend.store key → value, e.g. {\"tab\": \"models\"}." },
                    "preview": { "type": "boolean", "description": "false: Zend's own window instead, which takes input only when it never shows (a stand-in run offscreen, ZGX_OFFSCREEN), never the window someone works in. Default true." }
                },
                "required": ["id", "action"]
            }),
        },
        ToolDef {
            name: PLUGIN_LOGS,
            description: "A Zend plugin's log on this device, oldest first: what it printed (print) and what failed (error, with the Luau traceback: file, line and function), from its window part (source window) and its service.luau (source service), plus Zend's own notes on it (info, e.g. its service started or didn't load). Each line has seq, at (RFC 3339), source, level and text. Zend keeps its last 500 lines here and writes them to the file at `file`. Pass since (the last seq you saw) to read only what came after. plugin_status carries its last 20 too.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "The plugin's id, from list_plugins." },
                    "since": { "type": "integer", "minimum": 0, "description": "Only lines after this seq." },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 500, "description": "At most this many, the newest (default 100)." },
                    "preview": { "type": "boolean", "description": "Its preview's log (preview_plugin), not the person's." }
                },
                "required": ["id"]
            }),
        },
        ToolDef {
            name: CHECK_PLUGIN,
            description: "Check a Zend plugin's Luau before it runs, as `zend plugin check` does: each .luau file's syntax, and its types with luau-lsp against Zend's window API, its own zend.luau helpers and the @ds modules. A file is checked strictly when it starts with --!strict (a new plugin's init.luau does: a helper given the wrong kind of value, like zend.place { at = 5 }, is an error), else in Luau's default mode, which reports only what's wrong for sure. `strict` checks every file strictly. Each finding has file (relative to the plugin), line, column, severity (error or warning), code (SyntaxError, TypeError, or a lint's name) and message; `ok` is false when one is an error. Only the plugin's own files count (not @ds, not its zend.luau). Without luau-lsp on this device, only the syntax is checked: `types_checked` false, `types_note` says why. Zend also checks an own plugin as you save it and writes its errors to plugin_logs (source check).",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "The plugin's id, from list_plugins." },
                    "path": { "type": "string", "description": "Or a plugin folder's full path." },
                    "strict": { "type": "boolean", "description": "Check every file strictly, not only those starting with --!strict." }
                }
            }),
        },
        ToolDef {
            name: INSTALL_PLUGIN,
            description: "Install someone's Zend plugin from a .zend-plugin file (`path`) or a link (`url`): a link to a .zend-plugin file, a GitHub release (…/releases/latest or …/releases/tag/<tag>, its .zend-plugin asset), or a git repository (owner/repo, its GitHub page, …/tree/<branch>/<folder>, or any https git URL; `ref` a branch or tag, `folder` where the plugin is in it). Zend checks the signature over every file first: an unsigned or changed plugin is refused. Whatever you install waits for the person to allow it in Zend's window, which shows who signed it and what it asks: you can't allow it for them (`waiting` says so). Installing over a plugin already installed is an update, and only when the same key signed both.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "A .zend-plugin file's full path." },
                    "url": { "type": "string", "description": "A package link, a GitHub release, or a git repository (https)." },
                    "ref": { "type": "string", "description": "With a git repository: the branch or tag (default its default branch)." },
                    "folder": { "type": "string", "description": "With a git repository: the plugin's folder in it (default the top)." }
                }
            }),
        },
        ToolDef {
            name: UPDATE_PLUGIN,
            description: "Check whether a plugin installed from someone else has a newer version at the source it came from (its file, link, release or repository): its version and what it changes in what the plugin asks (`added`, `removed`); without an id, check every installed plugin. Installing an update is the person's, in Zend (Settings → Plugins): you can't update it for them, so tell them what you found.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "The plugin's id, from list_plugins; without one, every installed plugin." },
                    "check": { "type": "boolean", "description": "Kept for older callers: this tool only ever checks." }
                }
            }),
        },
        ToolDef {
            name: EXPORT_PLUGIN,
            description: "Share a Zend plugin: save it as one signed .zend-plugin file that anyone can install (install_plugin, or Settings → Plugins), e.g. to attach to a GitHub release. Your own plugin is signed again first, so the file matches its folder as it is now; someone else's keeps their signature. It's saved in Zend's plugin-exports folder as <id>-<version>.zend-plugin (an earlier export of that version there is replaced). Returns the file's `path` and its size in `bytes`. To save it somewhere else, the person runs `zend plugin export <id> --to <path>`.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "The plugin's id, from list_plugins." }
                },
                "required": ["id"]
            }),
        },
        ToolDef {
            name: DESCRIBE_ELEMENTS,
            description: "Where a Zend plugin can place its nodes: Zend's named elements, each with its selector (#id or .class), where it is in words, what it is, what it provides to what's placed in it (session, message, file, project, panel; everything has profile, appearance and route), whether the window running now has it (a template may rename or leave out elements), and whether it's stable (kept across Zend's updates and by every template: place by those where you can). Place with zend.place { at = \"<selector>\", before | after | replace = function() return <node> end }; selectors also take `#a >> .b` (inside) and role:button \"Name\". In the built node read what's provided with zend.context(name) (nil where absent) or zend.need(name) (it draws nothing there, and Settings says why). Records hold only what the plugin was allowed: sessions.read, messages.read, profile.read, account.email, files.read (without it a file is an id of the plugin's own, never its path). The reply links the plugin authoring guide (`guide`).",
            input_schema: json!({ "type": "object", "properties": {} }),
        },
        ToolDef {
            name: LIST_TEMPLATES,
            description: "Zend's templates on this device. A template is a whole copy of Zend's interface (its Luau app: app.luau, screens/, ds/, assets/) that you can rewrite; it inherits Zend's own for any file it lacks. Each one's id, name, folder, whether it's the one in use, and its status: ok, errors (some files fall back to Zend's own; template_status says why), out_of_date (made for another version of Zend: Zend has updated since; template_status names the files whose Zend original changed) or broken. Zend's own comes first. One installed from someone's package (import_template) has `imported` (who signed it); packages waiting for the person to allow them are under `pending`.",
            input_schema: json!({ "type": "object", "properties": {} }),
        },
        ToolDef {
            name: CREATE_TEMPLATE,
            description: "Make a new template on this device: a whole copy of Zend's interface (or of another template, with `from`) in its own folder, to rewrite. Returns its folder, its files and a short map of the app. Delete any file to inherit Zend's own. While Zend is open and the template is in use (use_template), each edit shows live as you save it. A file with an error falls back to Zend's own for that file, and template_status says why. `id` is lowercase letters, digits and dashes (2-40). The reply links the authoring guide (`guide`): read it before writing.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "pattern": "^[a-z0-9][a-z0-9-]{1,39}$", "description": "The template's id and folder name, e.g. midnight." },
                    "name": { "type": "string", "description": "Its display name, e.g. Midnight." },
                    "from": { "type": "string", "description": "A template to start from, from list_templates (default zend, Zend's own)." }
                },
                "required": ["id", "name"]
            }),
        },
        ToolDef {
            name: USE_TEMPLATE,
            description: "Run Zend's window with a template: it switches live while Zend is open. `zend` is Zend's own interface. Only templates installed on this device can be used. Unless the person has kept this template in Zend, it runs as a preview (`preview`): Zend shows them a bar to Keep or Revert it, and holds back what only a kept template may do until they keep it. You can't keep it for them; ask them to look.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "A template's id, from list_templates, or zend." }
                },
                "required": ["id"]
            }),
        },
        ToolDef {
            name: TEMPLATE_STATUS,
            description: "How a template is doing: whether it's in use, the error that broke it (root_error), the files with errors (each falls back to Zend's own, with the error), the files out of date (Zend's own has changed since they were copied), how many files it has, and how many it inherits from Zend.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "A template's id, from list_templates." }
                },
                "required": ["id"]
            }),
        },
        ToolDef {
            name: DELETE_TEMPLATE,
            description: "Delete a template: its folder is set aside in the templates folder's .removed. Deleting the one in use switches the window back to Zend's own. Zend's own can't be deleted.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "A template's id, from list_templates." }
                },
                "required": ["id"]
            }),
        },
        ToolDef {
            name: DESCRIBE_APP,
            description: "What Zend's interface is made of, to find your way in a template: its screens (each one's module, file, what it draws, and the elements it names: #ids and .classes, which plugins place by; keep them to keep plugins working) and a short map of the app's folders. For where plugins can place in the window running now, use describe_elements. The reply links the template authoring guide (`guide`).",
            input_schema: json!({ "type": "object", "properties": {} }),
        },
        ToolDef {
            name: EXPORT_TEMPLATE,
            description: "Share a template: save it as one signed .zend-template file, signed with this device's key (the person's Zend account vouches for it, as for their plugins). It holds the template's own files and its preview; the files no template may replace (screens/plugins, screens/templates, account/, .luaurc) are left out. Zend's own can't be exported. It's saved in Zend's exports folder (an earlier export of it there is replaced). Returns the file's path, how many files it holds and the key that signed it.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "A template's id, from list_templates." }
                },
                "required": ["id"]
            }),
        },
        ToolDef {
            name: IMPORT_TEMPLATE,
            description: "Install a template from a .zend-template file (`path`) or a link (`url`: to a .zend-template file, or a GitHub repository or release with one; a template from a link is checked there for updates). Zend checks its signature over every file first: an unsigned or changed package is refused. One signed on this device installs at once (using it still runs as a preview until the person keeps it). Anyone else's waits for the person: a template runs as Zend's own code with all its powers, so they must allow it in Zend's window (Templates shows it waiting, with who signed it); you can't allow it for them. The reply says which: `installed` (its id), or `pending` with `needs_consent`. Files no template may replace are never installed (`ignored` names them). list_templates shows it once installed.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "The .zend-template file's full path." },
                    "url": { "type": "string", "description": "Or a link: a .zend-template file's, or a GitHub repository or release (e.g. owner/repo) with one." }
                }
            }),
        },
    ]
}

#[derive(Deserialize)]
struct PackageArgs {
    id: Option<String>,
    path: Option<String>,
    #[serde(default)]
    url: Option<String>,
}

#[derive(Deserialize)]
struct TemplateArgs {
    id: Option<String>,
    name: Option<String>,
    from: Option<String>,
}

#[derive(Deserialize)]
struct CreatePluginArgs {
    id: String,
    name: String,
    description: Option<String>,
    permissions: Option<Vec<String>>,
}

/// What a plugin listing tracks between `tools/list` and the watch.
#[derive(Default)]
pub(super) struct PluginState {
    /// The plugin tool names the client was last shown (by a listing or the
    /// latest watch item). `None` until either happens.
    advertised: std::sync::Mutex<Option<BTreeSet<String>>>,
}

impl PluginState {
    /// Record `names` as what the client knows now; `true` when that
    /// differs from what it knew before.
    fn advertise(&self, names: BTreeSet<String>) -> bool {
        let mut advertised = self.advertised.lock().unwrap_or_else(|e| e.into_inner());
        advertised
            .replace(names.clone())
            .is_some_and(|before| before != names)
    }
}

impl Tools {
    /// The full `tools/list`: the static catalog, then the engine's plugin
    /// tools. Without an engine (or one without plugins) it is the static
    /// catalog alone.
    pub async fn list_all(&self) -> Vec<ListedTool> {
        let mut tools: Vec<ListedTool> = self.list().into_iter().map(ListedTool::from).collect();
        let reply = tokio::time::timeout(
            LIST_TIMEOUT,
            self.zend.call_rpc(methods::LIST_PLUGIN_TOOLS, json!({})),
        )
        .await;
        match reply {
            Ok(Ok(value)) => match serde_json::from_value::<PluginTools>(value) {
                Ok(plugins) => {
                    self.plugins.advertise(plugins.names());
                    let taken: BTreeSet<String> = tools.iter().map(|t| t.name.clone()).collect();
                    tools.extend(
                        plugins
                            .listed()
                            .into_iter()
                            .filter(|tool| !taken.contains(&tool.name)),
                    );
                }
                Err(error) => tracing::warn!(%error, "mcp: unexpected ListPluginTools reply"),
            },
            Ok(Err(error)) => tracing::debug!(%error, "mcp: no plugin tools"),
            Err(_) => tracing::warn!("mcp: ListPluginTools timed out; listing static tools only"),
        }
        tools
    }

    /// Follow `WatchPluginTools` for as long as the server runs, sending on
    /// `changed` whenever the set of plugin tool names differs from what the
    /// client last saw. Returns when the engine has no such method, or when
    /// `changed` closes.
    pub async fn watch_plugin_tools(&self, changed: mpsc::Sender<()>) {
        let mut retry = FIRST_RETRY;
        loop {
            match self
                .zend
                .subscribe_checked(methods::WATCH_PLUGIN_TOOLS, json!({}))
                .await
            {
                Ok(mut stream) => {
                    retry = FIRST_RETRY;
                    while let Some(item) = stream.recv().await {
                        let names = match serde_json::from_value::<PluginTools>(item) {
                            Ok(plugins) => plugins.names(),
                            Err(error) => {
                                tracing::warn!(%error, "mcp: unexpected WatchPluginTools item");
                                continue;
                            }
                        };
                        if self.plugins.advertise(names) && changed.send(()).await.is_err() {
                            return;
                        }
                    }
                }
                Err(RpcError::UnknownMethod(_)) => return,
                Err(error) => tracing::debug!(%error, "mcp: WatchPluginTools unavailable"),
            }
            if changed.is_closed() {
                return;
            }
            tokio::time::sleep(retry).await;
            retry = (retry * 2).min(MAX_RETRY);
        }
    }

    /// A plugin tool call, attributed to the chat this server speaks for.
    pub(super) async fn call_plugin_tool(&self, name: &str, args: Value) -> Result<Value, String> {
        let mut params = json!({ "mcp_name": name, "args": args });
        if let Some(chat) = &self.zend.origin().chat_id {
            params["chatId"] = json!(chat);
        }
        let reply = self
            .zend
            .call_rpc(methods::CALL_PLUGIN_TOOL, params)
            .await
            .map_err(plain)?;
        if reply.get("ok").and_then(Value::as_bool) == Some(false) {
            return Err(reply
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("The plugin tool failed")
                .to_owned());
        }
        Ok(reply.get("value").cloned().unwrap_or(Value::Null))
    }

    pub(super) async fn create_plugin(&self, args: Value) -> Result<Value, String> {
        let args: CreatePluginArgs = parse(args)?;
        let mut params = json!({ "id": args.id, "name": args.name });
        if let Some(description) = args.description {
            params["description"] = json!(description);
        }
        if let Some(permissions) = args.permissions {
            params["permissions"] = json!(permissions);
        }
        let item = self
            .zend
            .call_rpc(methods::NEW_PLUGIN, params)
            .await
            .map_err(plain)?;
        let id = item.get("id").and_then(Value::as_str).unwrap_or_default();
        let dir = item.get("dir").and_then(Value::as_str).unwrap_or_default();
        let mut reply = json!({
            "id": id,
            "dir": dir,
            "files": scaffold_files(dir).await,
            "status": item.get("status"),
            "hint": format!(
                "Edit {dir}/init.luau; Zend re-signs and reloads it as you save. Use {dir}/zend.luau's helpers."
            ),
        });
        // Its first check (check_plugin's), when the engine gave one.
        if let Some(check) = item.get("check") {
            reply["check"] = check.clone();
        }
        // The authoring guide, written on this device: read it first.
        if let Some(guide) = item.get("guide").and_then(Value::as_str) {
            reply["guide"] = json!(guide);
            reply["guide_summary"] = item.get("guide_summary").cloned().unwrap_or(Value::Null);
            reply["hint"] = json!(format!(
                "Read {guide} first (how to write a plugin, with examples). Then edit {dir}/init.luau; Zend re-signs and reloads it as you save. Check its Luau with check_plugin (file:line for each error), then where it shows with plugin_status (snapshot: true)."
            ));
        }
        Ok(reply)
    }

    /// The template tools: the engine's `ListTemplates`, `CreateTemplate`,
    /// `UseTemplate`, `TemplateStatus`, `DeleteTemplate` and `DescribeApp`,
    /// as they answer.
    pub(super) async fn templates(&self, tool: &str, args: Value) -> Result<Value, String> {
        let args: TemplateArgs = parse(args)?;
        let (method, needs_id) = match tool {
            LIST_TEMPLATES => (methods::LIST_TEMPLATES, false),
            DESCRIBE_APP => (methods::DESCRIBE_APP, false),
            CREATE_TEMPLATE => (methods::CREATE_TEMPLATE, true),
            USE_TEMPLATE => (methods::USE_TEMPLATE, true),
            TEMPLATE_STATUS => (methods::TEMPLATE_STATUS, true),
            _ => (methods::DELETE_TEMPLATE, true),
        };
        let mut params = json!({});
        if needs_id {
            let Some(id) = args.id.filter(|id| !id.trim().is_empty()) else {
                return Err("Say which template: `id`, from list_templates.".into());
            };
            params["id"] = json!(id);
        }
        if method == methods::CREATE_TEMPLATE {
            let Some(name) = args.name.filter(|name| !name.trim().is_empty()) else {
                return Err("Give the template a `name`.".into());
            };
            params["name"] = json!(name);
            if let Some(from) = args.from.filter(|from| !from.trim().is_empty()) {
                params["from"] = json!(from);
            }
        }
        let mut reply = self.zend.call_rpc(method, params).await.map_err(plain)?;
        if method == methods::USE_TEMPLATE && reply["preview"] == json!(true) {
            reply["next"] = json!(
                "Unless the person has kept this template, Zend runs it as a preview with a bar to Keep or Revert it. Ask them to look; you can't keep it for them."
            );
        }
        Ok(reply)
    }

    /// `export_template` and `import_template`: the engine's
    /// `ExportTemplate` and `ImportTemplate`. Allowing someone else's is the
    /// person's, in Zend's own window: no tool answers for them.
    pub(super) async fn template_packages(&self, tool: &str, args: Value) -> Result<Value, String> {
        let args: PackageArgs = parse(args)?;
        let (method, params) = if tool == EXPORT_TEMPLATE {
            let Some(id) = args.id.filter(|id| !id.trim().is_empty()) else {
                return Err("Say which template: `id`, from list_templates.".into());
            };
            // Always to Zend's exports folder (the reply says where): a
            // path of an agent's choosing is the person's, and refused.
            (methods::EXPORT_TEMPLATE, json!({ "id": id }))
        } else {
            let path = args.path.filter(|p| !p.trim().is_empty());
            match (path, args.url.filter(|u| !u.trim().is_empty())) {
                (Some(path), None) => (methods::IMPORT_TEMPLATE, json!({ "path": path })),
                (None, Some(url)) => (methods::IMPORT_TEMPLATE, json!({ "url": url })),
                _ => return Err("Say what to import: `path`, a .zend-template file, or `url`, a link to one.".into()),
            }
        };
        let mut reply = self.zend.call_rpc(method, params).await.map_err(plain)?;
        if reply["needs_consent"] == json!(true) {
            reply["next"] = json!(
                "It's waiting for the person to allow it in Zend (Templates shows it, with who signed it). Ask them to look; you can't allow it for them."
            );
        }
        Ok(reply)
    }

    pub(super) async fn list_plugins(&self) -> Result<Value, String> {
        let reply = self
            .zend
            .call_rpc(methods::LIST_PLUGINS, json!({}))
            .await
            .map_err(plain)?;
        let plugins = reply
            .get("plugins")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default()
            .iter()
            .map(|p| {
                let mut out = json!({
                    "id": p.get("id"),
                    "name": p.get("name"),
                    "status": p.get("status"),
                    "author": p.get("author"),
                    "dir": p.get("dir"),
                });
                // Someone else's: where it came from, and a newer version there.
                for key in ["source", "update"] {
                    if let Some(value) = p.get(key).filter(|v| !v.is_null()) {
                        out[key] = value.clone();
                    }
                }
                out
            })
            .collect::<Vec<_>>();
        Ok(json!({ "plugins": plugins }))
    }

    /// The engine's `PluginStatus`.
    pub(super) async fn plugin_status(&self, args: Value) -> Result<Value, String> {
        #[derive(Deserialize)]
        struct Args {
            id: String,
            #[serde(default)]
            snapshot: bool,
            #[serde(default)]
            page: Option<String>,
            #[serde(default)]
            width: Option<f64>,
            #[serde(default)]
            height: Option<f64>,
            #[serde(default)]
            state: Option<serde_json::Map<String, Value>>,
            #[serde(default)]
            preview: bool,
        }
        let args: Args = parse(args)?;
        if !args.snapshot && (args.page.is_some() || args.width.is_some() || args.height.is_some() || args.state.is_some()) {
            return Err("page, width, height and state are for a picture: pass snapshot: true".into());
        }
        if args.page.is_none() && (args.width.is_some() || args.height.is_some() || args.state.is_some()) {
            return Err("width, height and state are for a page's picture: pass page".into());
        }
        let mut params = json!({ "id": args.id, "snapshot": args.snapshot });
        if let Some(page) = args.page {
            params["page"] = json!(page);
        }
        if let Some(width) = args.width {
            params["width"] = json!(width);
        }
        if let Some(height) = args.height {
            params["height"] = json!(height);
        }
        if let Some(state) = args.state {
            params["state"] = Value::Object(state);
        }
        if args.preview {
            params["preview"] = json!(true);
        }
        self.zend
            .call_rpc(methods::PLUGIN_STATUS, params)
            .await
            .map_err(plain)
    }

    /// `plugin_logs`: the engine's `PluginLogs`.
    pub(super) async fn plugin_logs(&self, args: Value) -> Result<Value, String> {
        #[derive(Deserialize)]
        struct Args {
            id: Option<String>,
            since: Option<u64>,
            limit: Option<u64>,
            #[serde(default)]
            preview: bool,
        }
        let args: Args = parse(args)?;
        let Some(id) = args.id.filter(|id| !id.trim().is_empty()) else {
            return Err("Say which plugin: `id`, from list_plugins.".into());
        };
        let mut params = json!({ "id": id.trim(), "since": args.since, "limit": args.limit.map(|n| n.clamp(1, 500)) });
        if args.preview {
            params["preview"] = json!(true);
        }
        self.zend.call_rpc(methods::PLUGIN_LOGS, params).await.map_err(plain)
    }

    /// `preview_plugin`: the engine's `PluginPreview`.
    pub(super) async fn preview_plugin(&self, args: Value) -> Result<Value, String> {
        #[derive(Deserialize)]
        struct Args {
            id: Option<String>,
            path: Option<String>,
            #[serde(default)]
            stop: bool,
            #[serde(default)]
            in_window: bool,
        }
        let args: Args = parse(args)?;
        let given = |s: Option<String>| s.map(|s| s.trim().to_owned()).filter(|s| !s.is_empty());
        let mut params = match (given(args.id), given(args.path)) {
            (Some(id), None) => json!({ "id": id }),
            (None, Some(path)) => json!({ "path": path }),
            _ => return Err("Give one of `path` (a plugin folder's full path) or `id` (from list_plugins).".into()),
        };
        if args.stop {
            params["stop"] = json!(true);
        }
        if args.in_window {
            params["in_window"] = json!(true);
        }
        self.zend.call_rpc(methods::PLUGIN_PREVIEW, params).await.map_err(plain)
    }

    /// `plugin_input`: the engine's `PluginInput` (a preview's window).
    pub(super) async fn plugin_input(&self, args: Value) -> Result<Value, String> {
        #[derive(Deserialize)]
        struct Args {
            id: Option<String>,
            action: Option<String>,
            at: Option<String>,
            x: Option<f64>,
            y: Option<f64>,
            state: Option<serde_json::Map<String, Value>>,
            preview: Option<bool>,
        }
        let args: Args = parse(args)?;
        let Some(id) = args.id.map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()) else {
            return Err("Say which plugin's preview: `id`.".into());
        };
        let action = args.action.unwrap_or_default();
        match action.as_str() {
            "click" | "hover" if args.at.is_some() || (action == "hover" && args.x.is_some() && args.y.is_some()) => {}
            "click" | "hover" => return Err(format!("`{action}` takes `at`: a selector (hover also takes x, y alone)")),
            "set_state" if args.state.as_ref().is_some_and(|s| !s.is_empty()) => {}
            "set_state" => return Err("`set_state` takes `state`: zend.store key → value".into()),
            _ => return Err("`action` is click, hover or set_state".into()),
        }
        let mut params = json!({ "id": id, "action": action });
        if let Some(at) = args.at {
            params["at"] = json!(at);
        }
        for (key, value) in [("x", args.x), ("y", args.y)] {
            if let Some(value) = value {
                params[key] = json!(value);
            }
        }
        if let Some(state) = args.state {
            params["state"] = Value::Object(state);
        }
        if let Some(preview) = args.preview {
            params["preview"] = json!(preview);
        }
        self.zend.call_rpc(methods::PLUGIN_INPUT, params).await.map_err(plain)
    }

    /// `check_plugin`: the engine's `CheckPlugin`, as it answers.
    pub(super) async fn check_plugin(&self, args: Value) -> Result<Value, String> {
        #[derive(Deserialize)]
        struct Args {
            id: Option<String>,
            path: Option<String>,
            strict: Option<bool>,
        }
        let args: Args = parse(args)?;
        let given = |s: Option<String>| s.map(|s| s.trim().to_owned()).filter(|s| !s.is_empty());
        let mut params = match (given(args.id), given(args.path)) {
            (Some(id), None) => json!({ "id": id }),
            (None, Some(path)) => json!({ "path": path }),
            _ => return Err("Give one of `id` (from list_plugins) or `path` (a plugin folder's full path).".into()),
        };
        params["strict"] = json!(args.strict.unwrap_or(false));
        self.zend.call_rpc(methods::CHECK_PLUGIN, params).await.map_err(plain)
    }

    /// `install_plugin`: the engine's `InstallPlugin`. Allowing someone
    /// else's is the person's, in Zend's window.
    pub(super) async fn install_plugin(&self, args: Value) -> Result<Value, String> {
        #[derive(Deserialize)]
        struct Args {
            path: Option<String>,
            url: Option<String>,
            #[serde(rename = "ref")]
            git_ref: Option<String>,
            folder: Option<String>,
        }
        let args: Args = parse(args)?;
        let given = |s: Option<String>| s.filter(|s| !s.trim().is_empty());
        let params = match (given(args.path), given(args.url)) {
            (Some(path), None) => json!({ "path": path }),
            (None, Some(url)) => json!({ "url": url, "ref": given(args.git_ref), "folder": given(args.folder) }),
            _ => return Err("Give one of `path` (a .zend-plugin file) or `url` (a link or repository).".into()),
        };
        let reply = self.zend.call_rpc(methods::INSTALL_PLUGIN, params).await.map_err(plain)?;
        Ok(installed(reply))
    }

    /// `update_plugin`: `CheckPluginUpdates` only. Installing an update is
    /// the person's (`UpdatePlugin` is closed to agents).
    pub(super) async fn update_plugin(&self, args: Value) -> Result<Value, String> {
        #[derive(Deserialize)]
        struct Args {
            id: Option<String>,
            #[serde(default)]
            check: bool,
        }
        let args: Args = parse(args)?;
        let _ = args.check;
        let id = args.id.filter(|id| !id.trim().is_empty());
        // Checking only: installing an update is the person's, in Zend's
        // window (an agent's token can't update a plugin).
        let mut reply = self
            .zend
            .call_rpc(methods::CHECK_PLUGIN_UPDATES, json!({ "id": id }))
            .await
            .map_err(plain)?;
        if reply["updates"].as_array().is_some_and(|u| !u.is_empty()) {
            reply["next"] = json!(
                "To install an update, the person updates the plugin in Zend (Settings → Plugins); you can't update it for them. Tell them what you found."
            );
        }
        Ok(reply)
    }

    /// `export_plugin`: the engine's `ExportPlugin`, always to Zend's
    /// plugin-exports folder (R29: a path of an agent's choosing is the
    /// person's, and the engine refuses it).
    pub(super) async fn export_plugin(&self, args: Value) -> Result<Value, String> {
        #[derive(Deserialize)]
        struct Args {
            id: Option<String>,
        }
        let args: Args = parse(args)?;
        let Some(id) = args.id.filter(|id| !id.trim().is_empty()) else {
            return Err("Say which plugin: `id`, from list_plugins.".into());
        };
        let mut reply = self
            .zend
            .call_rpc(methods::EXPORT_PLUGIN, json!({ "id": id.trim() }))
            .await
            .map_err(plain)?;
        if let Some(path) = reply["path"].as_str() {
            reply["next"] = json!(format!(
                "Attach {path} to a release, or send it: anyone installs it with install_plugin (or Settings → Plugins), and it waits for them to allow it."
            ));
        }
        Ok(reply)
    }

    /// The engine's `DescribeElements`, as the window wrote it.
    pub(super) async fn describe_elements(&self) -> Result<Value, String> {
        self.zend
            .call_rpc(methods::DESCRIBE_ELEMENTS, json!({}))
            .await
            .map_err(plain)
    }
}

/// An install's reply, said for an agent: the plugin's id, name, version
/// and status, what changed, and what's next.
fn installed(reply: Value) -> Value {
    let plugin = &reply["plugin"];
    let mut out = json!({
        "id": plugin["id"],
        "name": plugin["name"],
        "version": plugin["version"],
        "status": plugin["status"],
        "author": plugin["author"],
        "update": reply["update"],
        "added": reply["added"],
        "removed": reply["removed"],
        "waiting": reply["waiting"],
    });
    if let Some(from) = reply.get("from") {
        out["from"] = from.clone();
    }
    if reply["waiting"] == json!(true) {
        out["next"] = json!(
            "It's waiting for the person to allow it in Zend (a notice asks them, and Settings → Plugins shows who signed it and what it asks). Ask them to look; you can't allow it for them."
        );
    }
    out
}

/// The files a new plugin's folder holds (top level, no dotfiles), sorted;
/// the scaffold's when the folder can't be read from here.
async fn scaffold_files(dir: &str) -> Vec<String> {
    let mut files = Vec::new();
    if !dir.is_empty()
        && let Ok(mut entries) = tokio::fs::read_dir(dir).await
    {
        while let Ok(Some(entry)) = entries.next_entry().await {
            let name = entry.file_name().to_string_lossy().into_owned();
            let is_file = entry.file_type().await.is_ok_and(|t| t.is_file());
            if is_file && !name.starts_with('.') {
                files.push(name);
            }
        }
    }
    if files.is_empty() {
        files = ["init.luau", "zend-plugin.json", "zend.luau"]
            .map(str::to_owned)
            .to_vec();
    }
    files.sort();
    files
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::zend::{Origin, Zend};
    use async_trait::async_trait;
    use futures::StreamExt;
    use std::sync::{Arc, Mutex};
    use zend_rpc::{RpcReply, RpcService, memory_client};

    fn tools_reply(names: &[&str]) -> Value {
        json!({
            "device": { "id": "dev-local", "name": "Studio" },
            "tools": names.iter().map(|name| json!({
                "plugin": "word-count", "plugin_name": "Word Count", "tool": "count_words",
                "mcp_name": name, "description": "Count words in a text",
                "input": { "type": "object", "properties": { "text": { "type": "string" } }, "required": ["text"] },
                "open": true,
            })).collect::<Vec<_>>(),
        })
    }

    /// The engine's plugin half: canned replies, recorded calls.
    #[derive(Default)]
    struct Engine {
        calls: Mutex<Vec<(String, Value)>>,
        /// `WatchPluginTools` items, in order.
        watch: Vec<Value>,
        call_error: Option<&'static str>,
        /// Answer `UnknownMethod` for every plugin method (an older engine).
        no_plugins: bool,
    }

    #[async_trait]
    impl RpcService for Engine {
        async fn handle(&self, method: &str, params: Value) -> Result<RpcReply, RpcError> {
            self.calls
                .lock()
                .unwrap()
                .push((method.to_owned(), params.clone()));
            if self.no_plugins {
                return Err(RpcError::UnknownMethod(method.into()));
            }
            Ok(match method {
                methods::LIST_PLUGIN_TOOLS => {
                    RpcReply::Value(tools_reply(&["word_count__count_words"]))
                }
                methods::WATCH_PLUGIN_TOOLS => RpcReply::Stream(
                    futures::stream::iter(self.watch.clone())
                        .chain(futures::stream::pending())
                        .boxed(),
                ),
                methods::CALL_PLUGIN_TOOL => match self.call_error {
                    Some(message) => return Err(RpcError::Failed(message.into())),
                    None => RpcReply::Value(json!({ "ok": true, "value": { "count": 3 } })),
                },
                methods::NEW_PLUGIN => RpcReply::Value(json!({
                    "id": params["id"], "name": params["name"], "dir": "/nowhere/plugins/focus",
                    "status": "on", "own": true,
                    "guide": "/nowhere/docs/plugins.md", "guide_summary": "How to write a Zend plugin.",
                })),
                methods::LIST_PLUGINS => RpcReply::Value(json!({
                    "dirs": { "plugins": "/d/plugins", "templates": "/d/templates" },
                    "plugins": [{
                        "id": "focus", "name": "Focus", "version": "1.2.0", "dir": "/d/plugins/focus",
                        "status": "pending", "author": { "kind": "verified", "username": "ada" },
                        "hash": "abc", "permissions": ["ui.add"],
                    }],
                })),
                methods::LIST_TEMPLATES => RpcReply::Value(json!({
                    "dir": "/d/templates", "active": "zend",
                    "templates": [
                        { "id": "zend", "name": "Zend", "builtin": true, "active": true, "status": "ok", "dir": "/d/templates/.zend" },
                        { "id": "midnight", "name": "Midnight", "builtin": false, "active": false, "status": "errors",
                          "error": "ds/tokens.luau: boom", "dir": "/d/templates/midnight" },
                    ],
                })),
                methods::CREATE_TEMPLATE => RpcReply::Value(json!({
                    "id": params["id"], "dir": "/d/templates/midnight", "files": ["app.luau"], "map": "app.luau is the root",
                })),
                methods::USE_TEMPLATE => match params["id"].as_str() {
                    Some("nope") => {
                        return Err(RpcError::Failed(
                            "There's no template nope. list_templates names them.".into(),
                        ));
                    }
                    _ => RpcReply::Value(json!({ "active": params["id"], "preview": params["id"] != "zend" })),
                },
                methods::TEMPLATE_STATUS => RpcReply::Value(json!({
                    "id": params["id"], "active": false, "file_errors": { "ds/tokens.luau": "boom" },
                    "out_of_date": [], "files": 40, "inherits": 2,
                })),
                methods::DELETE_TEMPLATE => {
                    RpcReply::Value(json!({ "deleted": params["id"], "active": "zend" }))
                }
                methods::EXPORT_TEMPLATE => RpcReply::Value(json!({
                    "id": params["id"], "path": params["path"], "files": 12, "key": "k1",
                })),
                methods::IMPORT_TEMPLATE => match params["path"].as_str() {
                    Some("/x/mine.zend-template") => RpcReply::Value(json!({
                        "installed": "midnight", "name": "Midnight", "author": { "kind": "you" }, "ignored": [],
                    })),
                    Some("/x/bad.zend-template") => {
                        return Err(RpcError::Failed(
                            "This template was changed after it was signed, so Zend won't install it.".into(),
                        ));
                    }
                    _ => RpcReply::Value(json!({
                        "pending": "midnight-1", "id": "midnight", "name": "Midnight", "key": "k2",
                        "author": { "kind": "verified", "username": "ada" }, "needs_consent": true,
                    })),
                },
                methods::INSTALL_PLUGIN => RpcReply::Value(json!({
                    "plugin": { "id": "focus", "name": "Focus", "version": "1.0.0", "status": "pending",
                                "author": { "kind": "verified", "username": "ada" } },
                    "update": false, "added": [], "removed": [], "waiting": true,
                })),
                methods::UPDATE_PLUGIN => RpcReply::Value(json!({
                    "plugin": { "id": params["id"], "name": "Focus", "version": "2.0.0", "status": "on",
                                "author": { "kind": "verified", "username": "ada" } },
                    "update": true, "from": "1.0.0", "added": [], "removed": ["links.open"], "waiting": false,
                })),
                methods::CHECK_PLUGIN_UPDATES => RpcReply::Value(json!({
                    "updates": [{ "id": "focus", "name": "Focus", "version": "2.0.0", "added": ["github"], "removed": [] }],
                    "errors": [],
                })),
                methods::EXPORT_PLUGIN => match params["id"].as_str() {
                    Some("nope") => return Err(RpcError::Failed("There's no plugin nope".into())),
                    id => RpcReply::Value(json!({
                        "path": format!("/d/plugin-exports/{}-1.0.0.zend-plugin", id.unwrap_or_default()), "bytes": 2048,
                    })),
                },
                methods::PLUGIN_STATUS => RpcReply::Value(json!({
                    "id": params["id"], "status": "on", "open": true,
                    "placements": [{ "at": ".session-row", "how": "after", "count": 3 }],
                })),
                methods::PLUGIN_LOGS => match params["id"].as_str() {
                    Some("nope") => return Err(RpcError::Failed("There's no plugin nope".into())),
                    _ => RpcReply::Value(json!({
                        "id": params["id"],
                        "file": "/d/logs/plugins/focus.log",
                        "lines": [
                            { "seq": 1, "at": "2026-11-14T17:00:00Z", "source": "window", "level": "print", "text": "hallo" },
                            { "seq": 2, "at": "2026-11-14T17:00:00Z", "source": "window", "level": "error",
                              "text": "focus: init.luau:2: kaputt\nstack traceback:\ninit.luau:2 function boom" },
                        ],
                    })),
                },
                methods::DESCRIBE_ELEMENTS => RpcReply::Value(json!({
                    "elements": [{ "selector": ".session-row", "place": "a sidebar row", "provides": ["session"], "present": true }],
                })),
                methods::DESCRIBE_APP => RpcReply::Value(json!({
                    "screens": [{ "module": "@app/screens/shell/sidebar", "elements": ["#sidebar"] }], "map": "app.luau is the root",
                })),
                other => return Err(RpcError::UnknownMethod(other.into())),
            })
        }
    }

    fn last_call(engine: &Engine) -> (String, Value) {
        engine.calls.lock().unwrap().last().unwrap().clone()
    }

    #[tokio::test]
    async fn template_tools_forward_and_report_errors() {
        let engine = Arc::new(Engine::default());
        let tools = tools_for(engine.clone(), None);

        let listed = tools.call(LIST_TEMPLATES, json!({})).await.unwrap();
        assert_eq!(listed["templates"][1]["error"], "ds/tokens.luau: boom");
        assert_eq!(
            last_call(&engine),
            (methods::LIST_TEMPLATES.to_owned(), json!({}))
        );

        let made = tools
            .call(
                CREATE_TEMPLATE,
                json!({ "id": "midnight", "name": "Midnight", "from": "dusk" }),
            )
            .await
            .unwrap();
        assert_eq!(made["dir"], "/d/templates/midnight");
        assert_eq!(
            last_call(&engine),
            (
                methods::CREATE_TEMPLATE.to_owned(),
                json!({ "id": "midnight", "name": "Midnight", "from": "dusk" })
            )
        );
        tools
            .call(
                CREATE_TEMPLATE,
                json!({ "id": "midnight", "name": "Midnight" }),
            )
            .await
            .unwrap();
        assert_eq!(
            last_call(&engine).1,
            json!({ "id": "midnight", "name": "Midnight" })
        );

        // R7: an agent's switch is a preview until the person keeps it.
        let used = tools
            .call(USE_TEMPLATE, json!({ "id": "midnight" }))
            .await
            .unwrap();
        assert_eq!((used["active"].as_str(), used["preview"].as_bool()), (Some("midnight"), Some(true)));
        assert!(used["next"].as_str().unwrap().contains("can't keep it for them"));
        assert_eq!(
            last_call(&engine),
            (
                methods::USE_TEMPLATE.to_owned(),
                json!({ "id": "midnight" })
            )
        );
        // The engine's words come back as they are.
        assert_eq!(
            tools
                .call(USE_TEMPLATE, json!({ "id": "nope" }))
                .await
                .unwrap_err(),
            "There's no template nope. list_templates names them."
        );

        let status = tools
            .call(TEMPLATE_STATUS, json!({ "id": "midnight" }))
            .await
            .unwrap();
        assert_eq!(status["file_errors"]["ds/tokens.luau"], "boom");
        assert_eq!(last_call(&engine).0, methods::TEMPLATE_STATUS);

        let deleted = tools
            .call(DELETE_TEMPLATE, json!({ "id": "midnight" }))
            .await
            .unwrap();
        assert_eq!(deleted, json!({ "deleted": "midnight", "active": "zend" }));
        assert_eq!(
            last_call(&engine),
            (
                methods::DELETE_TEMPLATE.to_owned(),
                json!({ "id": "midnight" })
            )
        );

        let described = tools.call(DESCRIBE_APP, json!({})).await.unwrap();
        assert_eq!(described["screens"][0]["elements"], json!(["#sidebar"]));
        assert_eq!(
            last_call(&engine),
            (methods::DESCRIBE_APP.to_owned(), json!({}))
        );
    }

    #[tokio::test]
    async fn template_package_tools_forward_and_never_allow() {
        let engine = Arc::new(Engine::default());
        let tools = tools_for(engine.clone(), None);

        let exported = tools
            .call(EXPORT_TEMPLATE, json!({ "id": "midnight", "path": "/Users/x/.ssh/x.zend-template" }))
            .await
            .unwrap();
        assert_eq!(exported["files"], 12);
        // R29: an agent never names where it goes (Zend's exports folder).
        assert_eq!(last_call(&engine), (methods::EXPORT_TEMPLATE.to_owned(), json!({ "id": "midnight" })));

        // Yours: installed.
        let mine = tools
            .call(IMPORT_TEMPLATE, json!({ "path": "/x/mine.zend-template" }))
            .await
            .unwrap();
        assert_eq!(mine["installed"], "midnight");
        assert!(mine.get("next").is_none());
        assert_eq!(
            last_call(&engine),
            (methods::IMPORT_TEMPLATE.to_owned(), json!({ "path": "/x/mine.zend-template" }))
        );
        // Someone else's: waiting for the person, and the reply says so.
        let theirs = tools
            .call(IMPORT_TEMPLATE, json!({ "path": "/x/theirs.zend-template" }))
            .await
            .unwrap();
        assert_eq!(theirs["needs_consent"], true);
        assert!(theirs["next"].as_str().unwrap().contains("can't allow it for them"));
        // Refused, in the engine's words.
        assert!(
            tools
                .call(IMPORT_TEMPLATE, json!({ "path": "/x/bad.zend-template" }))
                .await
                .unwrap_err()
                .contains("changed after it was signed")
        );

        // What's missing never reaches the engine.
        let before = engine.calls.lock().unwrap().len();
        assert!(tools.call(EXPORT_TEMPLATE, json!({ "path": "/x/a.zend-template" })).await.unwrap_err().contains("`id`"));
        assert!(tools.call(IMPORT_TEMPLATE, json!({ "path": " " })).await.unwrap_err().contains("`path`"));
        assert_eq!(engine.calls.lock().unwrap().len(), before);

        // No tool decides for the person.
        let catalog = super::super::catalog();
        assert!(catalog.iter().all(|d| !d.name.contains("decide") && !d.name.contains("allow")));
        let export = catalog.iter().find(|d| d.name == EXPORT_TEMPLATE).unwrap();
        assert!(export.input_schema["properties"].get("path").is_none());
        // Importing takes a file or a link.
        let import = catalog.iter().find(|d| d.name == IMPORT_TEMPLATE).unwrap();
        for field in ["path", "url"] {
            assert!(import.input_schema["properties"].get(field).is_some(), "{field}");
        }
        assert!(tools.call(IMPORT_TEMPLATE, json!({ "path": "/x/a.zend-template", "url": "ada/midnight" })).await.is_err());
        let before = engine.calls.lock().unwrap().len();
        let _ = tools.call(IMPORT_TEMPLATE, json!({ "url": "ada/midnight" })).await;
        let calls = engine.calls.lock().unwrap();
        assert_eq!(calls.len(), before + 1);
        assert!(format!("{:?}", calls.last().unwrap()).contains("ada/midnight"));
    }

    #[tokio::test]
    async fn template_tools_need_an_id() {
        let engine = Arc::new(Engine::default());
        let tools = tools_for(engine.clone(), None);
        for tool in [
            CREATE_TEMPLATE,
            USE_TEMPLATE,
            TEMPLATE_STATUS,
            DELETE_TEMPLATE,
        ] {
            let error = tools
                .call(tool, json!({ "name": "Midnight" }))
                .await
                .unwrap_err();
            assert!(error.contains("`id`"), "{tool}: {error}");
            assert!(
                tools
                    .call(tool, json!({ "id": " ", "name": "Midnight" }))
                    .await
                    .is_err(),
                "{tool}"
            );
        }
        assert!(
            tools
                .call(CREATE_TEMPLATE, json!({ "id": "midnight" }))
                .await
                .unwrap_err()
                .contains("`name`")
        );
        // None of them reached the engine.
        assert!(engine.calls.lock().unwrap().is_empty());
        // Every one is in the catalog with `id` required where it's needed.
        let catalog = super::super::catalog();
        for tool in [
            CREATE_TEMPLATE,
            USE_TEMPLATE,
            TEMPLATE_STATUS,
            DELETE_TEMPLATE,
        ] {
            let def = catalog.iter().find(|d| d.name == tool).unwrap();
            assert!(
                def.input_schema["required"]
                    .as_array()
                    .unwrap()
                    .contains(&json!("id")),
                "{tool}"
            );
        }
        for tool in [LIST_TEMPLATES, DESCRIBE_APP] {
            assert!(catalog.iter().any(|d| d.name == tool), "{tool}");
        }
    }

    fn tools_for(engine: Arc<Engine>, chat: Option<&str>) -> Tools {
        let origin = Origin {
            chat_id: chat.map(str::to_owned),
            ..Origin::default()
        };
        Tools::new(Arc::new(Zend::with_client(memory_client(engine), origin)))
    }

    #[test]
    fn plugin_tool_names_are_told_apart_from_static_ones() {
        assert!(is_plugin_tool_name("word_count__count_words"));
        assert!(!is_plugin_tool_name("list_plugins"));
        assert!(!is_plugin_tool_name("__count"));
        assert!(!is_plugin_tool_name("Word__count"));
        for def in super::super::catalog() {
            assert!(!is_plugin_tool_name(def.name), "{}", def.name);
        }
    }

    #[tokio::test]
    async fn tools_list_includes_plugin_tools() {
        let tools = tools_for(Arc::new(Engine::default()), Some("chat-1"));
        let listed = tools.list_all().await;
        assert!(listed.iter().any(|t| t.name == "whoami"));
        assert!(listed.iter().any(|t| t.name == CREATE_PLUGIN));
        assert!(listed.iter().any(|t| t.name == LIST_PLUGINS));
        let plugin = listed
            .iter()
            .find(|t| t.name == "word_count__count_words")
            .expect("plugin tool listed");
        assert_eq!(
            plugin.description,
            "Count words in a text (From the Word Count plugin. Works while Zend is open on Studio.)"
        );
        assert_eq!(plugin.input_schema["required"], json!(["text"]));
        assert_eq!(listed.len(), tools.list().len() + 1);
    }

    #[tokio::test]
    async fn an_engine_without_plugins_lists_the_static_tools() {
        let tools = tools_for(
            Arc::new(Engine {
                no_plugins: true,
                ..Engine::default()
            }),
            None,
        );
        assert_eq!(tools.list_all().await.len(), tools.list().len());
    }

    #[tokio::test]
    async fn a_plugin_tool_call_forwards_with_the_chat() {
        let engine = Arc::new(Engine::default());
        let tools = tools_for(engine.clone(), Some("chat-1"));
        let value = tools
            .call("word_count__count_words", json!({ "text": "a b c" }))
            .await
            .unwrap();
        assert_eq!(value, json!({ "count": 3 }));
        let calls = engine.calls.lock().unwrap();
        let (method, params) = calls.last().unwrap();
        assert_eq!(method, methods::CALL_PLUGIN_TOOL);
        assert_eq!(
            params,
            &json!({
                "mcp_name": "word_count__count_words",
                "args": { "text": "a b c" },
                "chatId": "chat-1",
            })
        );
    }

    #[tokio::test]
    async fn a_plugin_tool_error_is_the_engine_message_and_an_is_error_result() {
        let engine = Arc::new(Engine {
            call_error: Some("Open Zend on Studio to use count_words"),
            ..Engine::default()
        });
        let tools = tools_for(engine, None);
        assert_eq!(
            tools
                .call("word_count__count_words", json!({}))
                .await
                .unwrap_err(),
            "Open Zend on Studio to use count_words"
        );
        let reply = crate::jsonrpc::handle_request(
            &tools,
            json!(7),
            "tools/call",
            json!({ "name": "word_count__count_words", "arguments": {} }),
        )
        .await;
        assert_eq!(reply["result"]["isError"], true);
        assert_eq!(
            reply["result"]["content"][0]["text"],
            "Open Zend on Studio to use count_words"
        );
    }

    #[tokio::test]
    async fn create_plugin_and_list_plugins_forward() {
        let engine = Arc::new(Engine::default());
        let tools = tools_for(engine.clone(), None);
        let created = tools
            .call(
                CREATE_PLUGIN,
                json!({ "id": "focus", "name": "Focus", "permissions": ["ui.add"] }),
            )
            .await
            .unwrap();
        assert_eq!(created["id"], "focus");
        assert_eq!(created["dir"], "/nowhere/plugins/focus");
        assert_eq!(
            created["files"],
            json!(["init.luau", "zend-plugin.json", "zend.luau"])
        );
        assert_eq!(
            created["hint"],
            "Read /nowhere/docs/plugins.md first (how to write a plugin, with examples). Then edit /nowhere/plugins/focus/init.luau; Zend re-signs and reloads it as you save. Check its Luau with check_plugin (file:line for each error), then where it shows with plugin_status (snapshot: true)."
        );
        assert_eq!(created["guide"], "/nowhere/docs/plugins.md");
        {
            let calls = engine.calls.lock().unwrap();
            let (method, params) = calls.last().unwrap();
            assert_eq!(method, methods::NEW_PLUGIN);
            assert_eq!(
                params,
                &json!({ "id": "focus", "name": "Focus", "permissions": ["ui.add"] })
            );
        }
        let listed = tools.call(LIST_PLUGINS, json!({})).await.unwrap();
        assert_eq!(
            listed,
            json!({ "plugins": [{
                "id": "focus", "name": "Focus", "status": "pending",
                "author": { "kind": "verified", "username": "ada" }, "dir": "/d/plugins/focus",
            }] })
        );
        assert_eq!(
            engine.calls.lock().unwrap().last().unwrap().0,
            methods::LIST_PLUGINS
        );
        let status = tools.call(PLUGIN_STATUS, json!({ "id": "focus", "snapshot": true })).await.unwrap();
        assert_eq!(status["placements"][0]["count"], 3);
        assert_eq!(
            last_call(&engine),
            (methods::PLUGIN_STATUS.to_owned(), json!({ "id": "focus", "snapshot": true }))
        );
        // A page of its own to picture: what's asked goes on as asked, and
        // only with what it belongs to.
        let asked = json!({ "id": "focus", "snapshot": true, "page": "notes", "width": 800.0, "state": { "tab": "all" } });
        tools.call(PLUGIN_STATUS, asked.clone()).await.unwrap();
        assert_eq!(last_call(&engine), (methods::PLUGIN_STATUS.to_owned(), asked));
        let unpictured = tools.call(PLUGIN_STATUS, json!({ "id": "focus", "page": "notes" })).await.unwrap_err();
        assert!(unpictured.contains("snapshot: true"), "{unpictured}");
        let pageless = tools.call(PLUGIN_STATUS, json!({ "id": "focus", "snapshot": true, "width": 800 })).await.unwrap_err();
        assert!(pageless.contains("pass page"), "{pageless}");
        let map = tools.call(DESCRIBE_ELEMENTS, json!({})).await.unwrap();
        assert_eq!(map["elements"][0]["selector"], ".session-row");
        assert_eq!(
            engine.calls.lock().unwrap().last().unwrap().0,
            methods::DESCRIBE_ELEMENTS
        );
    }

    #[tokio::test]
    async fn install_and_update_plugin_leave_consent_to_the_person() {
        let engine = Arc::new(Engine::default());
        let tools = tools_for(engine.clone(), None);
        let installed = tools
            .call(INSTALL_PLUGIN, json!({ "url": "ada/focus", "ref": "v1", "folder": " " }))
            .await
            .unwrap();
        assert_eq!(
            last_call(&engine),
            (methods::INSTALL_PLUGIN.to_owned(), json!({ "url": "ada/focus", "ref": "v1", "folder": null }))
        );
        assert_eq!((installed["status"].as_str(), installed["waiting"].as_bool()), (Some("pending"), Some(true)));
        assert!(installed["next"].as_str().unwrap().contains("can't allow it for them"));
        tools.call(INSTALL_PLUGIN, json!({ "path": "/x/focus.zend-plugin" })).await.unwrap();
        assert_eq!(last_call(&engine), (methods::INSTALL_PLUGIN.to_owned(), json!({ "path": "/x/focus.zend-plugin" })));
        assert!(tools.call(INSTALL_PLUGIN, json!({})).await.unwrap_err().contains("`path`"));
        assert!(tools.call(INSTALL_PLUGIN, json!({ "path": "/a", "url": "b/c" })).await.unwrap_err().contains("one of"));

        let checked = tools.call(UPDATE_PLUGIN, json!({ "check": true })).await.unwrap();
        assert_eq!(checked["updates"][0]["added"], json!(["github"]));
        assert_eq!(last_call(&engine), (methods::CHECK_PLUGIN_UPDATES.to_owned(), json!({ "id": null })));
        // R6: updating is the person's. The tool only checks, and says so.
        let updated = tools.call(UPDATE_PLUGIN, json!({ "id": "focus" })).await.unwrap();
        assert_eq!(updated["updates"][0]["version"], "2.0.0");
        assert!(updated["next"].as_str().unwrap().contains("can't update it for them"));
        assert_eq!(last_call(&engine), (methods::CHECK_PLUGIN_UPDATES.to_owned(), json!({ "id": "focus" })));
        tools.call(UPDATE_PLUGIN, json!({})).await.unwrap();
        assert!(
            engine.calls.lock().unwrap().iter().all(|(method, _)| method != methods::UPDATE_PLUGIN),
            "the tool never installs an update"
        );
    }

    #[tokio::test]
    async fn export_plugin_saves_to_the_exports_folder_only() {
        let engine = Arc::new(Engine::default());
        let tools = tools_for(engine.clone(), None);
        let exported = tools
            .call(EXPORT_PLUGIN, json!({ "id": "focus", "to": "/Users/x/.ssh/x.zend-plugin" }))
            .await
            .unwrap();
        assert_eq!(exported["path"], "/d/plugin-exports/focus-1.0.0.zend-plugin");
        assert_eq!(exported["bytes"], 2048);
        assert!(exported["next"].as_str().unwrap().contains("install_plugin"));
        // R29: an agent never names where it goes (Zend's plugin-exports folder).
        assert_eq!(last_call(&engine), (methods::EXPORT_PLUGIN.to_owned(), json!({ "id": "focus" })));
        // The engine's words come back as they are.
        assert_eq!(
            tools.call(EXPORT_PLUGIN, json!({ "id": "nope" })).await.unwrap_err(),
            "There's no plugin nope"
        );
        // Without an id it never reaches the engine.
        let before = engine.calls.lock().unwrap().len();
        for args in [json!({}), json!({ "id": " " })] {
            assert!(tools.call(EXPORT_PLUGIN, args).await.unwrap_err().contains("`id`"));
        }
        assert_eq!(engine.calls.lock().unwrap().len(), before);
        // In the catalog, `id` required and no path to choose.
        let catalog = super::super::catalog();
        let def = catalog.iter().find(|d| d.name == EXPORT_PLUGIN).unwrap();
        assert_eq!(def.input_schema["required"], json!(["id"]));
        assert!(def.input_schema["properties"].get("to").is_none());
        assert!(def.description.contains(".zend-plugin"));
    }

    #[tokio::test]
    async fn plugin_logs_forwards_and_needs_an_id() {
        let engine = Arc::new(Engine::default());
        let tools = tools_for(engine.clone(), None);
        let logs = tools.call(PLUGIN_LOGS, json!({ "id": " focus ", "since": 0, "limit": 9000 })).await.unwrap();
        assert_eq!(
            last_call(&engine),
            (methods::PLUGIN_LOGS.to_owned(), json!({ "id": "focus", "since": 0, "limit": 500 }))
        );
        let lines = logs["lines"].as_array().unwrap();
        assert_eq!(lines[0]["text"], "hallo");
        assert!(lines[1]["text"].as_str().unwrap().contains("stack traceback:"));
        assert_eq!(
            tools.call(PLUGIN_LOGS, json!({ "id": "nope" })).await.unwrap_err(),
            "There's no plugin nope"
        );
        let before = engine.calls.lock().unwrap().len();
        assert!(tools.call(PLUGIN_LOGS, json!({})).await.unwrap_err().contains("`id`"));
        assert_eq!(engine.calls.lock().unwrap().len(), before);
        let catalog = super::super::catalog();
        let def = catalog.iter().find(|d| d.name == PLUGIN_LOGS).unwrap();
        assert_eq!(def.input_schema["required"], json!(["id"]));
        assert!(def.description.contains("traceback"));
    }

    #[tokio::test]
    async fn create_plugin_lists_the_folder_it_made() {
        let dir = tempfile::tempdir().unwrap();
        for file in ["zend-plugin.json", "init.luau", "zend.luau", ".hidden"] {
            std::fs::write(dir.path().join(file), "").unwrap();
        }
        std::fs::create_dir(dir.path().join("lib")).unwrap();
        assert_eq!(
            scaffold_files(dir.path().to_str().unwrap()).await,
            vec!["init.luau", "zend-plugin.json", "zend.luau"]
        );
    }

    #[tokio::test]
    async fn the_watch_says_changed_only_when_the_names_change() {
        let engine = Arc::new(Engine {
            watch: vec![
                tools_reply(&["a__one"]),
                // Same names (an `open` flip, say): no change.
                tools_reply(&["a__one"]),
                tools_reply(&["a__one", "b__two"]),
            ],
            ..Engine::default()
        });
        let tools = Arc::new(tools_for(engine, None));
        let (tx, mut rx) = mpsc::channel(8);
        let watch = tokio::spawn({
            let tools = tools.clone();
            async move { tools.watch_plugin_tools(tx).await }
        });
        tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("a change in time")
            .expect("one change");
        assert!(
            tokio::time::timeout(Duration::from_millis(200), rx.recv())
                .await
                .is_err(),
            "exactly one change"
        );
        watch.abort();
    }

    #[tokio::test]
    async fn the_watch_stops_on_an_engine_without_plugins() {
        let tools = tools_for(
            Arc::new(Engine {
                no_plugins: true,
                ..Engine::default()
            }),
            None,
        );
        let (tx, _rx) = mpsc::channel(1);
        tokio::time::timeout(Duration::from_secs(5), tools.watch_plugin_tools(tx))
            .await
            .expect("returns at once");
    }
}
