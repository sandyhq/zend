//! Check a plugin's Luau before Zend runs it (`zend plugin check`, the
//! `CheckPlugin` RPC, MCP `check_plugin`, and after each save of an own
//! plugin, into its log).
//!
//! Two passes over the plugin's `.luau` files:
//!
//! - Syntax, with the Luau compiler Zend already links (mlua): always runs.
//! - Types, with `luau-lsp analyze` against zgx's definitions (the `ui`
//!   global the window gives) and Zend's own `@ds/…` modules, the new
//!   solver. The same tool `zgx check` uses: the Luau crate Zend links has
//!   no type checker in it. Zend doesn't ship it: the first check that
//!   finds none fetches a pinned release into `<data>/tools` (`fetch`),
//!   and its report says so. Without it (offline, the fetch failed) the
//!   types are skipped, and the report says why.
//!
//! A file is checked in the mode it asks for (`--!strict` at its top), else
//! Luau's own default, nonstrict, which reports only what is wrong for
//! sure: untyped code passes. `strict` checks every file strictly.
//!
//! Only the plugin's own files count: what luau-lsp finds in `@ds`
//! modules, and in the plugin's copy of Zend's SDK (`zend.luau`), is left
//! out.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::Serialize;

pub mod fetch;

/// zgx's window API, as luau-lsp definitions (`ui`, its nodes and states).
const DEFINITIONS: &str = include_str!("../../../../third_party/zgx/crates/zgx-script/types/zgx.d.luau");

/// How the root `init.luau` is named in the copy luau-lsp reads: an
/// `init.luau` is its folder's module to luau-lsp, so its `./zend` would
/// be looked for beside the folder.
const INIT_AS: &str = "__zend_plugin_init__.luau";

/// Files a plugin may hold at most for a check (a plugin is small).
const MAX_FILES: usize = 400;
/// A run of luau-lsp at most.
const TIMEOUT: Duration = Duration::from_secs(90);

/// What to say when luau-lsp isn't on this machine.
pub const INSTALL: &str = "Types not checked: luau-lsp isn't on this machine. Put it on the PATH, beside Zend, \
or in <data>/tools (or set LUAU_LSP): https://github.com/JohnnyMorganz/luau-lsp/releases";

/// What to say when luau-lsp isn't on this machine and fetching it failed.
fn fetch_failed(why: &str) -> String {
    format!(
        "Types not checked: luau-lsp isn't on this machine, and fetching luau-lsp {} failed: {why}. Put it on the PATH, \
beside Zend, or in <data>/tools (or set LUAU_LSP): https://github.com/JohnnyMorganz/luau-lsp/releases",
        fetch::VERSION
    )
}

#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Every file strictly, not only those that ask (`--!strict`).
    pub strict: bool,
    /// luau-lsp itself; [`find_luau_lsp`] when `None`.
    pub luau_lsp: Option<PathBuf>,
    /// Zend's `ds` modules (what `@ds/x` names); [`ds_dir`] when `None`.
    pub ds: Option<PathBuf>,
    /// Zend's data folder (where a luau-lsp in `tools/` is looked for).
    pub data_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Finding {
    /// The file, relative to the plugin's folder (`init.luau`).
    pub file: String,
    pub line: u32,
    pub column: u32,
    pub severity: Severity,
    /// `SyntaxError`, `TypeError`, or a lint's name (`LocalUnused`…).
    pub code: String,
    pub message: String,
}

impl std::fmt::Display for Finding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let severity = match self.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        write!(f, "{}:{}:{}: {severity}[{}]: {}", self.file, self.line, self.column, self.code, self.message)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    /// The plugin's folder.
    pub dir: String,
    /// The `.luau` files checked, relative to it.
    pub files: Vec<String>,
    pub findings: Vec<Finding>,
    pub errors: usize,
    pub warnings: usize,
    /// Whether the types were checked (else only the syntax), and why not.
    pub types_checked: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub types_note: Option<String>,
    /// That luau-lsp was fetched for this check, and where to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fetched: Option<String>,
}

impl Report {
    pub fn ok(&self) -> bool {
        self.errors == 0
    }

    /// One line a person reads: "2 errors, 1 warning", or that it's clean.
    pub fn summary(&self) -> String {
        let n = |count: usize, word: &str| format!("{count} {word}{}", if count == 1 { "" } else { "s" });
        let what = if self.types_checked { "syntax and types" } else { "syntax only" };
        if self.findings.is_empty() {
            format!("No problems ({what}, {}).", n(self.files.len(), "file"))
        } else {
            format!("{}, {} ({what}).", n(self.errors, "error"), n(self.warnings, "warning"))
        }
    }
}

/// `LUAU_LSP`, beside this executable, in `<data>/tools`, or on the PATH.
pub fn find_luau_lsp(data_dir: Option<&Path>) -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("LUAU_LSP").map(PathBuf::from)
        && path.is_file()
    {
        return Some(path);
    }
    let name = fetch::binary_name();
    let beside = std::env::current_exe().ok().and_then(|exe| exe.parent().map(|dir| dir.join(name)));
    let tools = data_dir.map(|dir| dir.join("tools").join(name));
    let path = std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).map(|dir| dir.join(name)).collect::<Vec<_>>())
        .unwrap_or_default();
    beside.into_iter().chain(tools).chain(path).find(|path| path.is_file())
}

/// Zend's `ds` modules: `ZEND_ZGX_UI`'s, the installed app's (`ui` beside
/// the executable, a macOS bundle's Resources), the copy the app writes in
/// `<data>/templates/.zend`, or the source tree's.
pub fn ds_dir(data_dir: Option<&Path>) -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(ui) = std::env::var_os("ZEND_ZGX_UI").filter(|v| !v.is_empty()) {
        candidates.push(PathBuf::from(ui).join("ds"));
    }
    if let Some(dir) = std::env::current_exe().ok().and_then(|exe| exe.parent().map(Path::to_path_buf)) {
        candidates.push(dir.join("ui").join("ds"));
        if let Some(contents) = dir.parent() {
            candidates.push(contents.join("Resources").join("ui").join("ds"));
        }
    }
    if let Some(data) = data_dir {
        candidates.push(data.join("templates").join(".zend").join("ds"));
    }
    candidates.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/zgx/ui/ds"));
    candidates.into_iter().find(|dir| dir.join("text.luau").is_file())
}

/// Check the plugin in `dir`. Fails only when there's nothing to check.
pub fn check_plugin(dir: &Path, options: &Options) -> Result<Report, String> {
    if !dir.is_dir() {
        return Err(format!("{} isn't a folder", dir.display()));
    }
    let files = scripts(dir);
    if files.is_empty() {
        return Err(format!("{} holds no .luau files", dir.display()));
    }
    if files.len() > MAX_FILES {
        return Err(format!("{} holds more than {MAX_FILES} .luau files", dir.display()));
    }
    let mut findings = Vec::new();
    let mut sdk_copies = Vec::new();
    for rel in &files {
        let Ok(source) = std::fs::read(dir.join(rel)) else {
            continue;
        };
        if is_sdk_copy(&source) {
            sdk_copies.push(rel.clone());
        }
        if let Some(finding) = syntax(rel, &source) {
            findings.push(finding);
        }
    }
    let mut fetched = None;
    let (types_checked, types_note) = match types(dir, &files, &sdk_copies, options, &mut fetched) {
        Ok(found) => {
            // The compiler said the syntax errors already.
            findings.extend(found.into_iter().filter(|f| f.code != "SyntaxError"));
            (true, None)
        }
        Err(note) => (false, Some(note)),
    };
    findings.sort_by(|a, b| (&a.file, a.line, a.column).cmp(&(&b.file, b.line, b.column)));
    findings.dedup();
    let errors = findings.iter().filter(|f| f.severity == Severity::Error).count();
    Ok(Report {
        dir: dir.display().to_string(),
        files,
        warnings: findings.len() - errors,
        errors,
        findings,
        types_checked,
        types_note,
        fetched,
    })
}

/// The plugin's `.luau` files, relative and sorted (`/` between parts),
/// skipping hidden folders and `node_modules`.
fn scripts(dir: &Path) -> Vec<String> {
    fn walk(root: &Path, dir: &Path, depth: usize, out: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || out.len() > MAX_FILES {
                continue;
            }
            let Ok(kind) = entry.file_type() else { continue };
            let path = entry.path();
            if kind.is_dir() {
                if depth < 8 && name != "node_modules" {
                    walk(root, &path, depth + 1, out);
                }
            } else if kind.is_file() && name.ends_with(".luau") {
                if let Ok(rel) = path.strip_prefix(root) {
                    let parts: Vec<String> =
                        rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
                    out.push(parts.join("/"));
                }
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, 0, &mut out);
    out.sort();
    out
}

/// Zend's SDK, as `zend plugin new` copies it (its header says so).
fn is_sdk_copy(source: &[u8]) -> bool {
    let head = String::from_utf8_lossy(&source[..source.len().min(400)]).into_owned();
    head.lines().take(3).any(|line| line.starts_with("-- Zend's helpers for a plugin."))
}

/// The compiler's word on `source`, if it doesn't parse.
fn syntax(rel: &str, source: &[u8]) -> Option<Finding> {
    let error = mlua::chunk::Compiler::new().compile(source).err()?;
    let text = match &error {
        mlua::Error::SyntaxError { message, .. } => message.clone(),
        other => other.to_string(),
    };
    let (line, message) = compiler_message(&text);
    Some(Finding {
        file: rel.to_owned(),
        line,
        column: 1,
        severity: Severity::Error,
        code: "SyntaxError".into(),
        message,
    })
}

/// The compiler's `12: message` (or `[string "…"]:12: message`): its line
/// and its words.
fn compiler_message(text: &str) -> (u32, String) {
    let text = text.trim();
    let after_chunk = match text.strip_prefix("[string \"") {
        Some(rest) => rest.split_once("\"]").map(|(_, r)| r).unwrap_or(rest),
        None => text,
    };
    let rest = after_chunk.strip_prefix(':').unwrap_or(after_chunk);
    if let Some((line, message)) = rest.split_once(':')
        && let Ok(line) = line.trim().parse::<u32>()
    {
        return (line, message.trim().to_owned());
    }
    (1, text.to_owned())
}

/// luau-lsp's findings in the plugin's own files (not its SDK copy), or
/// why the types weren't checked.
fn types(
    dir: &Path,
    files: &[String],
    sdk_copies: &[String],
    options: &Options,
    fetched: &mut Option<String>,
) -> Result<Vec<Finding>, String> {
    let data_dir = options.data_dir.as_deref();
    let luau_lsp = match options.luau_lsp.clone().or_else(|| find_luau_lsp(data_dir)) {
        Some(found) => found,
        // None here: fetched into Zend's data folder, the first time.
        None => match data_dir.filter(|_| fetch::allowed()) {
            None => return Err(INSTALL.to_owned()),
            Some(data) => match fetch::fetch(data) {
                Ok((path, false)) => path,
                Ok((path, true)) => {
                    *fetched = Some(format!(
                        "Fetched luau-lsp {} into {} (its SHA-256 checked), for this check and the next ones.",
                        fetch::VERSION,
                        path.display()
                    ));
                    path
                }
                Err(why) => return Err(fetch_failed(&why)),
            },
        },
    };
    let ds = options.ds.clone().or_else(|| ds_dir(data_dir));

    // A copy luau-lsp reads, the root init.luau renamed (see INIT_AS).
    let work = tempfile::Builder::new()
        .prefix("zend-plugin-check-")
        .tempdir()
        .map_err(|e| format!("Types not checked: no temporary folder ({e})"))?;
    let copy = work.path().join("plugin");
    let named = |rel: &str| if rel == "init.luau" { INIT_AS.to_owned() } else { rel.to_owned() };
    for rel in files {
        let to = copy.join(named(rel));
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("Types not checked: {e}"))?;
        }
        std::fs::copy(dir.join(rel), &to).map_err(|e| format!("Types not checked: {e}"))?;
    }
    // Its own .luaurc (a mode, its aliases), where it has one.
    if dir.join(".luaurc").is_file() {
        let _ = std::fs::copy(dir.join(".luaurc"), copy.join(".luaurc"));
    }
    let definitions = work.path().join("zgx.d.luau");
    std::fs::write(&definitions, DEFINITIONS).map_err(|e| format!("Types not checked: {e}"))?;
    let mut base = serde_json::json!({ "languageMode": if options.strict { "strict" } else { "nonstrict" } });
    if let Some(ds) = &ds {
        let ds = ds.canonicalize().unwrap_or_else(|_| ds.clone());
        base["aliases"] = serde_json::json!({ "ds": ds.display().to_string() });
    }
    let base_rc = work.path().join("base.luaurc");
    std::fs::write(&base_rc, base.to_string()).map_err(|e| format!("Types not checked: {e}"))?;

    let mut command = Command::new(&luau_lsp);
    command
        .current_dir(&copy)
        .arg("analyze")
        .arg("--platform=standard")
        .arg("--flag:LuauSolverV2=true")
        .arg(format!("--definitions={}", definitions.display()))
        .arg("--formatter=gnu")
        .arg(format!("--base-luaurc={}", base_rc.display()));
    for rel in files.iter().filter(|rel| !sdk_copies.contains(rel)) {
        command.arg(named(rel));
    }
    if files.iter().all(|rel| sdk_copies.contains(rel)) {
        return Ok(Vec::new());
    }
    let _turn = turn();
    let output = run(&mut command, &luau_lsp)?;
    let found = parse(&output);
    let ours = found
        .into_iter()
        .filter_map(|mut f| {
            let rel = own_file(&f.file, &copy)?;
            let rel = if rel == INIT_AS { "init.luau".to_owned() } else { rel };
            if sdk_copies.contains(&rel) {
                return None;
            }
            f.file = rel;
            Some(f)
        })
        .collect();
    Ok(ours)
}

/// `file` as luau-lsp printed it, relative to the plugin's copy, if it is
/// one of the plugin's files (not a `@ds` module's).
fn own_file(file: &str, copy: &Path) -> Option<String> {
    let path = Path::new(file);
    let rel = if path.is_absolute() {
        let copy = copy.canonicalize().unwrap_or_else(|_| copy.to_path_buf());
        let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        path.strip_prefix(&copy).ok()?.to_path_buf()
    } else {
        path.to_path_buf()
    };
    let parts: Vec<String> = rel
        .components()
        .map(|c| match c {
            std::path::Component::Normal(part) => Some(part.to_string_lossy().into_owned()),
            std::path::Component::CurDir => Some(String::new()),
            _ => None,
        })
        .collect::<Option<Vec<_>>>()?;
    let parts: Vec<String> = parts.into_iter().filter(|p| !p.is_empty()).collect();
    (!parts.is_empty()).then(|| parts.join("/"))
}

/// Run luau-lsp, its output (stdout and stderr) once it's done, killed
/// past [`TIMEOUT`].
fn run(command: &mut Command, luau_lsp: &Path) -> Result<String, String> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Types not checked: couldn't run {} ({e})", luau_lsp.display()))?;
    let drain = |pipe: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut text = Vec::new();
            if let Some(mut pipe) = pipe {
                let _ = pipe.read_to_end(&mut text);
            }
            text
        })
    };
    let out = drain(child.stdout.take().map(|p| Box::new(p) as Box<dyn Read + Send>));
    let err = drain(child.stderr.take().map(|p| Box::new(p) as Box<dyn Read + Send>));
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if started.elapsed() > TIMEOUT => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(_) => break None,
        }
    };
    let mut text = String::from_utf8_lossy(&out.join().unwrap_or_default()).into_owned();
    text.push('\n');
    text.push_str(&String::from_utf8_lossy(&err.join().unwrap_or_default()));
    match status {
        None => Err(format!("Types not checked: luau-lsp took longer than {} s", TIMEOUT.as_secs())),
        // It exits 1 when it found something; anything else is its failure.
        Some(status) if status.code().is_some_and(|code| code <= 1) => Ok(text),
        Some(status) => {
            let tail: String = text.trim().lines().rev().take(3).collect::<Vec<_>>().join(" / ");
            Err(format!("Types not checked: luau-lsp failed ({status}): {tail}"))
        }
    }
}

/// luau-lsp's `--formatter=gnu` output: `file:line.col-line.col: Code:
/// message`, a message's further lines following it (as zgx's check).
fn parse(text: &str) -> Vec<Finding> {
    let mut out: Vec<Finding> = Vec::new();
    let mut continuing = false;
    for line in text.lines() {
        if line.starts_with('[') && line.contains(']') {
            continuing = false;
            continue;
        }
        match parse_line(line) {
            Some(f) => {
                out.push(f);
                continuing = true;
            }
            None if continuing && !line.trim().is_empty() => {
                if let Some(last) = out.last_mut() {
                    last.message.push('\n');
                    last.message.push_str(line.trim_end());
                }
            }
            None => continuing = false,
        }
    }
    out
}

fn parse_line(line: &str) -> Option<Finding> {
    let (head, rest) = line.split_once(": ")?;
    let colon = head.rfind(':')?;
    let range = &head[colon + 1..];
    let (start, _) = range.split_once('-').unwrap_or((range, ""));
    let (l, c) = start.split_once('.')?;
    let line_no: u32 = l.parse().ok()?;
    let column: u32 = c.parse().ok()?;
    let (code, message) = rest.split_once(": ")?;
    if code.is_empty() || !code.chars().all(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    Some(Finding {
        file: head[..colon].to_owned(),
        line: line_no,
        column,
        severity: if code.ends_with("Error") { Severity::Error } else { Severity::Warning },
        code: code.to_owned(),
        message: message.trim_end().to_owned(),
    })
}

/// The machine's turn for a plugin check: one luau-lsp at a time however
/// many saves, engines and `zend plugin check`s ask (an advisory lock on a
/// file in the temporary folder; without one, it runs anyway).
fn turn() -> Option<std::fs::File> {
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(std::env::temp_dir().join("zend-plugin-check.lock"))
        .ok()?;
    let started = Instant::now();
    loop {
        match file.try_lock() {
            Ok(()) => return Some(file),
            Err(std::fs::TryLockError::WouldBlock) if started.elapsed() < TIMEOUT => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(_) => return None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plugin(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (name, text) in files {
            let path = dir.path().join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        dir
    }

    /// luau-lsp, where this machine has it (the type tests say they're
    /// skipped otherwise).
    fn with_types() -> Option<Options> {
        let found = find_luau_lsp(None);
        if found.is_none() {
            eprintln!("luau-lsp not found: type checks skipped");
        }
        found.map(|path| Options { luau_lsp: Some(path), ..Options::default() })
    }

    #[test]
    fn a_syntax_error_is_named_with_its_file_and_line() {
        let dir = plugin(&[("init.luau", "local x = 1\nlocal y = (\nprint(x)\n"), ("lib/ok.luau", "return 1\n")]);
        let options = Options { luau_lsp: Some(PathBuf::from("/nonexistent/luau-lsp")), ..Options::default() };
        let report = check_plugin(dir.path(), &options).unwrap();
        assert_eq!(report.files, vec!["init.luau", "lib/ok.luau"]);
        assert_eq!(report.errors, 1, "{:?}", report.findings);
        let f = &report.findings[0];
        assert_eq!((f.file.as_str(), f.code.as_str()), ("init.luau", "SyntaxError"));
        assert!((2..=4).contains(&f.line), "{f}");
        assert!(!report.types_checked);
        assert!(report.types_note.as_deref().unwrap().starts_with("Types not checked"));
    }

    #[test]
    fn the_compilers_words_give_the_line() {
        assert_eq!(compiler_message("[string \"init\"]:3: Expected ')'"), (3, "Expected ')'".into()));
        assert_eq!(compiler_message(":12: Incomplete statement"), (12, "Incomplete statement".into()));
        assert_eq!(compiler_message("4: Expected ')' (to close '(' at line 2), got <eof>").0, 4);
        assert_eq!(compiler_message("odd"), (1, "odd".into()));
    }

    #[test]
    fn gnu_lines_parse_with_their_continuations() {
        let text = "[INFO] x\ninit.luau:4.19-4.19: TypeError: Expected this to be\n\t'string'\nbut got 'number'\n\
                    ../ds/text.luau:1.1-1.2: LocalUnused: Variable 'a' is never used\n";
        let found = parse(text);
        assert_eq!(found.len(), 2);
        assert_eq!((found[0].line, found[0].column, found[0].severity), (4, 19, Severity::Error));
        assert!(found[0].message.ends_with("but got 'number'"));
        assert_eq!(found[1].severity, Severity::Warning);
        let copy = Path::new("/tmp/x/plugin");
        assert_eq!(own_file("init.luau", copy).as_deref(), Some("init.luau"));
        assert_eq!(own_file("./lib/a.luau", copy).as_deref(), Some("lib/a.luau"));
        assert_eq!(own_file("../ds/text.luau", copy), None);
        assert_eq!(own_file("/elsewhere/ds/text.luau", copy), None);
    }

    #[test]
    fn a_number_where_a_place_goes_is_a_type_error_in_a_strict_file() {
        let Some(options) = with_types() else { return };
        let sdk = super::super::scaffold::SDK;
        let dir = plugin(&[
            ("zend.luau", sdk),
            (
                "init.luau",
                "--!strict\nlocal zend = require(\"./zend\")(\"demo\")\n\nzend.place { at = 5, after = function() return nil :: any end }\n",
            ),
        ]);
        let report = check_plugin(dir.path(), &options).unwrap();
        assert!(report.types_checked, "{:?}", report.types_note);
        let errors: Vec<_> = report.findings.iter().filter(|f| f.severity == Severity::Error).collect();
        assert_eq!(errors.len(), 1, "{:?}", report.findings);
        assert_eq!((errors[0].file.as_str(), errors[0].line, errors[0].code.as_str()), ("init.luau", 4, "TypeError"));
        assert!(errors[0].message.contains("'string'"), "{}", errors[0]);
    }

    #[test]
    fn untyped_code_and_a_new_plugin_pass() {
        let Some(options) = with_types() else { return };
        let sdk = super::super::scaffold::SDK;
        let untyped = plugin(&[
            ("zend.luau", sdk),
            (
                "init.luau",
                "local zend = require(\"./zend\")(\"demo\")\nlocal t = {}\nt.x = 1\nlocal function f(a, b) return a .. b end\nprint(f(t.x, \"y\"))\n",
            ),
        ]);
        let report = check_plugin(untyped.path(), &options).unwrap();
        assert_eq!(report.errors, 0, "{:?}", report.findings);
        let fresh = plugin(&[("zend.luau", sdk), ("init.luau", &super::super::scaffold::init_luau("demo", "Demo"))]);
        let report = check_plugin(fresh.path(), &options).unwrap();
        assert_eq!(report.errors, 0, "{:?}", report.findings);
        assert!(report.types_checked);
    }
}
