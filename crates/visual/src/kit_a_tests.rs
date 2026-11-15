//! ui.kit batch a (dev kinds): each builds its nodes from data, and bad data
//! fails with "ui.kit.<Kind> ..." at the agent's line.

use super::*;

/// The kinds of kit_a.luau. A new one goes here, gets a case below and an
/// example in visuals/.
pub(crate) const KINDS_A: &[&str] = &[
    "Acceptance",
    "Benchmark",
    "Changes",
    "Code",
    "Commits",
    "Coverage",
    "Dependencies",
    "Diff",
    "Environments",
    "File",
    "Findings",
    "Json",
    "Logs",
    "Pipeline",
    "PullRequest",
    "Snippet",
    "StackTrace",
    "Tests",
];

fn text(script: &str) -> String {
    let node = Visual::load(script, 1)
        .unwrap_or_else(|e| panic!("{script}: {}", e.message))
        .render()
        .unwrap_or_else(|e| panic!("{script}: {}", e.message));
    text_of(&node)
}

fn text_of(node: &Node) -> String {
    match &node.kind {
        NodeKind::Text(text) => text.spans.iter().map(|s| s.text.as_str()).collect(),
        _ => children(node).map(text_of).collect::<Vec<_>>().join("|"),
    }
}

fn has(script: &str, parts: &[&str]) {
    let got = text(script);
    for part in parts {
        assert!(got.contains(part), "{script}\n  missing {part:?} in {got:?}");
    }
}

fn err(script: &str, parts: &[&str]) {
    let e = check(script, 1).unwrap_err();
    for part in parts {
        assert!(e.message.contains(part), "{script}\n  {part:?} not in {:?}", e.message);
    }
}

#[test]
fn acceptance_counts_what_passed_and_shows_the_evidence() {
    has(
        r#"return ui.kit.Acceptance { criteria = {
            { name = "Rate limit", status = "verified", by = "agent", note = "429 after 10", evidence = "docs/e.md" },
            { name = "Screen reader", status = "not tested" },
            { name = "Mail", status = "claimed" } } }"#,
        &["1 of 3 criteria passed", "claimed only", "Rate limit", "agent", "429 after 10", "docs/e.md", "Untested"],
    );
    has(
        r#"return ui.kit.Acceptance { columns = { "Claimed", "Checked" }, criteria = {
            { name = "Parity", cells = { Claimed = "passed", Checked = { status = "failed", note = "3px off" } } } } }"#,
        &["1 of 2 checks passed", "Parity", "Passed", "Failed", "3px off"],
    );
}

#[test]
fn findings_are_sorted_by_severity_with_where_and_fix() {
    let got = text(
        r#"return ui.kit.Findings { title = "Review", items = {
            { severity = "low", title = "Unused option", file = "a.ts", line = 3 },
            { severity = "critical", title = "Path traversal", file = "b.py", line = 48, column = 2, rule = "S1",
              detail = "Resolve first", fix = "--- a/b.py\n+++ b/b.py\n@@ -1 +1 @@\n-x\n+y", status = "open" } } }"#,
    );
    assert!(got.find("Path traversal") < got.find("Unused option"), "{got}");
    for part in ["Review", "1 critical", "b.py:48:2  S1", "Resolve first", "Fix", "+y", "Open"] {
        assert!(got.contains(part), "{part} in {got}");
    }
}

#[test]
fn a_stack_trace_marks_the_error_and_dims_library_frames() {
    has(
        r#"return ui.kit.StackTrace { trace = "TypeError: x is undefined\n    at load (src/app.js:4:9)\n    at run (node_modules/lib/index.js:1:1)\nCaused by: Error: disk" }"#,
        &["TypeError: x is undefined", "at load (src/app.js:4:9)", "2 frames, 1 in your code", "Caused by: Error: disk"],
    );
    // Rust backtraces, with the frame's file on the line after.
    has(
        r#"return ui.kit.StackTrace { trace = "thread 'main' panicked at src/main.rs:6:17:\nboom\nstack backtrace:\n   0: core::panicking::panic\n             at /rustc/abc/library/core/src/panicking.rs:1:1\n   1: app::main\n             at ./src/main.rs:10:3" }"#,
        &["panicked", "app::main", "2 frames, 1 in your code"],
    );
    has(
        r#"return ui.kit.StackTrace { trace = { message = "Boom", frames = { { fn = "go", file = "a.go", line = 3 } } } }"#,
        &["Boom", "at go (a.go:3)"],
    );
}

#[test]
fn dependencies_say_how_far_behind() {
    has(
        r#"return ui.kit.Dependencies { report = {
            { name = "next", current = "15.0.0-rc.0", latest = "15.0.0", severity = "high", note = "advisory" },
            { name = "react", current = "18.2.0", wanted = "18.3.1", latest = "19.0.0" },
            { name = "vitest", current = "2.1.1", latest = "2.1.4", type = "dev" },
            { name = "ts", current = "5.4.5", latest = "5.6.3" },
            { name = "left-pad", latest = "1.3.0" } } }"#,
        &["1 major", "1 minor", "1 patch", "1 pre-release", "1 not installed", "18.3.1", "major", "minor", "patch", "not installed", "next: advisory", "dev"],
    );
}

#[test]
fn benchmark_judges_changes_by_which_way_is_better() {
    has(
        r#"return ui.kit.Benchmark { results = {
            { name = "Cold start", before = 412, after = 298, unit = "ms" },
            { name = "Throughput", before = 1840, after = 2310, unit = "req/s", better = "higher" },
            { name = "p99", before = 402, after = 455, unit = "ms", stddev = 12 },
            { name = "Memory", before = 212, after = 213, unit = "MB", note = "peak" } } }"#,
        &["2 better", "1 worse", "1 unchanged", "−27.7%  better", "+25.5%  better", "+13.2%  worse", "unchanged", "Memory: peak"],
    );
    has(
        r#"return ui.kit.Benchmark { results = {
            { name = "iPhone", metrics = { fps = 59.5, jank = 2 }, status = "passed" },
            { name = "Pixel", metrics = { fps = 41 }, status = "failed" } } }"#,
        &["iPhone", "59.5", "Pixel", "41.0", "Passed", "Failed"],
    );
    err(r#"return ui.kit.Benchmark { results = { { name = "x", before = 1, after = 2, better = "up" } } }"#, &["unknown better 'up'"]);
}

#[test]
fn a_pipeline_opens_only_what_failed() {
    let got = text(
        r#"return ui.kit.Pipeline { run = { name = "Deploy web", branch = "release/2.4", sha = "9f1c2ab", stages = {
            { name = "Build", status = "passed", duration = "1m" },
            { name = "Test", status = "failed", jobs = {
                { name = "unit", status = "passed", steps = { { name = "hidden step", status = "passed" } } },
                { name = "e2e", status = "failed", steps = { { name = "Run", status = "failed", message = "3 failed" } } } } },
            { name = "Deploy", status = "skipped" } } } }"#,
    );
    for part in ["Deploy web", "Failed", "release/2.4 · 9f1c2ab", "Build", "unit", "e2e", "Run", "3 failed", "Deploy"] {
        assert!(got.contains(part), "{part} in {got}");
    }
    assert!(!got.contains("hidden step"), "{got}");
    // A GitHub run's conclusion counts.
    has(
        r#"return ui.kit.Pipeline { { { name = "lint", status = "completed", conclusion = "success" } } }"#,
        &["Passed", "lint"],
    );
}

#[test]
fn tests_show_failures_first_and_fold_the_passes() {
    let got = text(
        r#"return ui.kit.Tests { title = "Unit", results = {
            { name = "adds", status = "pass", ms = 4 },
            { name = "coupon", status = "fail", file = "cart.test.ts", line = 42, message = "\27[32mExpected 90\27[39m\nReceived 81" },
            { name = "retry", status = "skip" } } }"#,
    );
    for part in ["1 passed", "1 failed", "1 skipped", "cart.test.ts:42", "Expected 90", "Received 81"] {
        assert!(got.contains(part), "{part} in {got}");
    }
    assert!(!got.contains('\u{1b}'), "{got:?}");
    assert!(got.find("coupon") < got.find("retry"));
    assert!(!got.contains("adds"), "passes stay folded: {got}");
}

#[test]
fn logs_colour_levels_and_fold_repeats() {
    let got = text(
        r#"return ui.kit.Logs { command = "npm test", exit = 1, output = "[info] up\n[info] GET /health\n[info] GET /health\n[info] GET /health\n[error] boom\n    at f (a.js:1)" }"#,
    );
    for part in ["$ npm test", "exit 1", "[info] up", "×3", "[error] boom", "at f (a.js:1)"] {
        assert!(got.contains(part), "{part} in {got}");
    }
    has(r#"return ui.kit.Logs { output = { "a", "b" } }"#, &["a|b"]);
    err(r#"return ui.kit.Logs { command = "x" }"#, &["ui.kit.Logs needs output"]);
}

#[test]
fn coverage_totals_and_lists_the_weakest_files_first() {
    let stat = |script: &str| {
        let node = Visual::load(script, 1).unwrap().render().unwrap();
        let mut found = Vec::new();
        fn walk(n: &Node, out: &mut Vec<String>) {
            if let NodeKind::Stat { value, .. } = &n.kind {
                out.push(value.clone());
            }
            children(n).for_each(|c| walk(c, out));
        }
        walk(&node, &mut found);
        // One headline figure, then a bar (a canvas) per file.
        assert_eq!(found.len(), 1, "{script}");
        found.remove(0)
    };
    assert_eq!(
        stat(r#"return ui.kit.Coverage { report = { total = 82.4, files = { { path = "a.ts", pct = 90 } } } }"#),
        "82.4%"
    );
    // Without a total: the files' counts, else the mean of their percents.
    assert_eq!(
        stat(r#"return ui.kit.Coverage { report = { files = { { path = "a", covered = 90, total = 100 } } } }"#),
        "90.0%"
    );
    assert_eq!(
        stat(r#"return ui.kit.Coverage { report = { { path = "a", pct = 90 }, { path = "b", pct = 40 } } }"#),
        "65.0%"
    );
    err(r#"return ui.kit.Coverage { report = { files = { { path = "a" } } } }"#, &["needs pct"]);
}

#[test]
fn changes_open_a_diff_and_diff_numbers_its_lines() {
    let patch = "--- a/f.ts\n+++ b/f.ts\n@@ -40,3 +40,3 @@\n fn a() {\n-  old\n+  new\n }";
    let got = text(&format!(
        r#"return ui.kit.Diff {{ patch = "{}" }}"#,
        patch.replace('\n', "\\n")
    ));
    for part in ["--- a/f.ts", "@@ -40,3 +40,3 @@", "  40   40 ", "  41      -  old", "  new", "  42 "] {
        assert!(got.contains(part), "{part:?} in {got:?}");
    }
    let script = format!(
        r#"return function() return ui.kit.Changes {{ files = {{
            {{ path = "src/api/users.ts", added = 3, removed = 1, status = "modified", diff = "{}" }},
            {{ path = "docs/x.md", added = 9, removed = 0, status = "added" }} }} }} end"#,
        patch.replace('\n', "\\n")
    );
    let mut visual = Visual::load(&script, 1).unwrap();
    let closed = text_of(&visual.render().unwrap());
    for part in ["2 files changed", "+12", "−1", "src/api/users.ts", "docs/x.md", "▸"] {
        assert!(closed.contains(part), "{part} in {closed}");
    }
    assert!(!closed.contains("fn a()"), "{closed}");
    err(r#"return ui.kit.Diff { patch = "no hunks" }"#, &["needs patch"]);
}

#[test]
fn commits_pull_requests_and_environments() {
    has(
        r#"return ui.kit.Commits { commits = { { sha = "9f1c2abdeadbeef", title = "Fix cart", author = "Ana", date = "Oct 8", added = 12, removed = 3, files = 2 } } }"#,
        &["1 commit", "9f1c2ab", "Fix cart", "Ana · Oct 8 · 2 files", "+12"],
    );
    has(
        r#"return ui.kit.PullRequest { number = 412, title = "Retry with backoff", state = "open", author = "ana",
            branch = "retry", base = "main", added = 40, removed = 8,
            checks = { { name = "build", status = "success" }, { name = "e2e", status = "failure" } },
            review = { state = "changes_requested", reviewers = { "bo" } } }"#,
        &["#412  Retry with backoff", "Open", "ana · retry → main", "1 of 2 checks failing", "build", "Changes requested · bo"],
    );
    has(
        r#"return ui.kit.Environments { items = { { name = "main", branch = "main", current = true, threads = 2, changes = { files = 3, added = 40, removed = 2 }, pr = { number = 9, state = "merged" } } } }"#,
        &["main", "this chat", "2 threads · 3 files", "#9 Merged"],
    );
    err(r#"return ui.kit.PullRequest { number = 1 }"#, &["ui.kit.PullRequest needs title"]);
    err(r#"return ui.kit.PullRequest { title = "x", state = "wip" }"#, &["unknown state 'wip'"]);
}

#[test]
fn snippet_code_file_and_json() {
    has(
        r#"return ui.kit.Snippet { path = "data/retry.ts", from = 4, code = "a\nb\nc" }"#,
        &["data/retry.ts", "lines 4–6", "4  a", "6  c"],
    );
    err(r#"return ui.kit.Snippet { path = "x" }"#, &["a visual cannot read files"]);
    has(
        r#"return ui.kit.Code { language = "ts", title = "Retry", code = "for (;;) {\n  go();\n}" }"#,
        &["Retry", "ts", "for (;;) {", "  go();"],
    );
    has(r#"return ui.kit.File { path = "src/worker/waitlist.ts", line = 528 }"#, &["src/worker/waitlist.ts:528"]);

    // Json: the first level is open, the next is closed until clicked.
    let script = r#"return function() return ui.kit.Json { label = "event", value = {
        id = "evt_1", data = { amount = 4900, ok = true, lines = { "a", "b" } } } } end"#;
    let mut visual = Visual::load(script, 1).unwrap();
    let got = text_of(&visual.render().unwrap());
    for part in ["event", "{2}", "data", "{3}", "id", "\"evt_1\""] {
        assert!(got.contains(part), "{part} in {got}");
    }
    assert!(!got.contains("4900"), "depth 1 is closed: {got}");
    has(
        r#"return ui.kit.Json { open = 3, value = { data = { amount = 4900, lines = { "a" } } } }"#,
        &["4900", "[1]", "\"a\""],
    );
}

#[test]
fn errors_name_the_kind_and_the_agents_line() {
    for kind in KINDS_A {
        let script = format!("\n\nreturn ui.kit.{kind} {{ colour = \"red\" }}");
        let e = check(&script, 1).unwrap_err();
        assert!(e.message.contains("visual:3"), "{kind}: {}", e.message);
        assert!(
            e.message.contains(&format!("ui.kit.{kind} has no prop 'colour'")),
            "{kind}: {}",
            e.message
        );
    }
    err(r#"return ui.kit.Findings { items = { { severity = "bad", title = "x" } } }"#, &["ui.kit.Findings", "unknown severity 'bad'"]);
    err(r#"return ui.kit.Tests { results = { { name = "x", status = "maybe" } } }"#, &["unknown status 'maybe'"]);
    err(r#"return ui.kit.Pipeline { run = { stages = { { name = "x", status = "huh" } } } }"#, &["unknown status 'huh'", "passed, failed, running"]);
    err(r#"return ui.kit.Acceptance { criteria = { { name = "x", status = "huh" } } }"#, &["unknown status 'huh'"]);
    err(r#"return ui.kit.Dependencies { report = { { current = "1" } } }"#, &["ui.kit.Dependencies needs every package"]);
    err(r#"return ui.kit.StackTrace { trace = 5 }"#, &["needs trace"]);
}
