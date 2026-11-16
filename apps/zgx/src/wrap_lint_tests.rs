//! Text that is cut off stays a decision (docs: apps/zgx/ui/ds/fade_text,
//! the AGENTS rule "wrapping is the rule"): `ui.text` does not wrap unless
//! told to, so a text with `truncate = true` is one line by design (a
//! sidebar row, a chip, a path in a list, a dense table's cell), and says so
//! in a `-- one line: <why>` comment within six lines of it,
//! or carries a `tooltip` with its full words. A new one without either
//! fails here.

use std::path::{Path, PathBuf};

fn luau_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            luau_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "luau") {
            out.push(path);
        }
    }
}

/// Each `truncate = true` in `source`: its line, and whether the lines round
/// it (six above, six below: its call) hold the reason.
fn truncates(source: &str) -> Vec<(usize, bool)> {
    let lines: Vec<&str> = source.lines().collect();
    let mut out = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if !line.contains("truncate = true") {
            continue;
        }
        let near = lines[i.saturating_sub(6)..(i + 7).min(lines.len())].join("\n");
        out.push((i + 1, near.contains("-- one line:") || near.contains("tooltip")));
    }
    out
}

#[test]
fn every_truncated_text_says_why_it_is_one_line() {
    let ui = Path::new(env!("CARGO_MANIFEST_DIR")).join("ui");
    let mut files = Vec::new();
    for dir in ["ds", "screens", "templates"] {
        luau_files(&ui.join(dir), &mut files);
    }
    let mut bare = Vec::new();
    let mut seen = 0;
    for file in &files {
        let source = std::fs::read_to_string(file).unwrap();
        for (line, said) in truncates(&source) {
            seen += 1;
            if !said {
                bare.push(format!("{}:{line}", file.strip_prefix(&ui).unwrap().display()));
            }
        }
    }
    assert!(seen > 0, "no truncate = true found: the lint looks nowhere");
    assert!(
        bare.is_empty(),
        "text cut off at one line without saying why. Let it wrap (`wrap = true`), or keep it one line with a `-- one line: <why>` comment beside it, and its full words in a `tooltip` where it can:\n{}",
        bare.join("\n")
    );
}

#[test]
fn the_lint_sees_a_bare_truncate_and_a_said_one() {
    let bare = "ui.text {\n\tname,\n\ttruncate = true,\n}";
    assert_eq!(truncates(bare), [(3, false)]);
    let said = "-- one line: a chip\nui.text {\n\tname,\n\ttruncate = true,\n}";
    assert_eq!(truncates(said), [(4, true)]);
    let tip = "ui.text {\n\tname,\n\ttruncate = true,\n\ttooltip = name,\n}";
    assert_eq!(truncates(tip), [(3, true)]);
}
