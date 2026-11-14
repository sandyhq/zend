//! The window's half of each plugin's log: what a plugin's window part
//! prints and the errors it hits (with their tracebacks), sent to the
//! engine (`PluginLog`), which keeps them with its service's lines for
//! `zend plugin logs`, `plugin_logs` and `plugin_status`.
//!
//! Lines wait here until the one send in flight is done, so a burst goes
//! as a few batches; without a connection the last [`HELD`] wait for one.

use std::cell::RefCell;
use std::collections::VecDeque;

use serde_json::{Value as Json, json};

use crate::engine;

/// Lines held while nothing takes them (the oldest go first).
const HELD: usize = 500;
/// Lines one send takes at most (the engine's own limit).
const BATCH: usize = 200;

struct Line {
    id: String,
    level: &'static str,
    text: String,
}

#[derive(Default)]
struct Queue {
    lines: VecDeque<Line>,
    /// A send is on its way, and takes what comes meanwhile.
    sending: bool,
    /// Lines dropped since the last send (too many while nothing took them).
    dropped: usize,
}

thread_local! {
    static QUEUE: RefCell<Queue> = RefCell::default();
}

/// Plugin `id` printed `line`.
pub(crate) fn print(id: &str, line: &str) {
    push(id, "print", line);
}

/// A note on plugin `id` from the window itself (its preview withdrawn).
pub(crate) fn info(id: &str, line: &str) {
    push(id, "info", line);
}

/// Plugin `id` failed: `error`, its traceback with it.
pub(crate) fn error(id: &str, error: &str) {
    push(id, "error", error);
}

fn push(id: &str, level: &'static str, text: &str) {
    QUEUE.with(|q| {
        let mut q = q.borrow_mut();
        if q.lines.len() >= HELD {
            q.lines.pop_front();
            q.dropped += 1;
        }
        q.lines.push_back(Line { id: id.to_owned(), level, text: text.to_owned() });
    });
    flush();
}

/// Send what waits, unless a send is on its way (it takes it) or there's no
/// engine to send to (the next connection does: `connected`).
pub(crate) fn flush() {
    let start = QUEUE.with(|q| {
        let mut q = q.borrow_mut();
        let start = !q.sending && !q.lines.is_empty() && engine::get().current().is_some();
        q.sending |= start;
        start
    });
    if !start {
        return;
    }
    engine::get().spawn(async {
        loop {
            let Some((id, lines)) = next_batch() else { break };
            let said = engine::get().call("PluginLog", json!({ "id": id, "lines": lines })).await;
            if let Err(error) = said
                && std::env::var_os("ZEND_ZGX_PLUGIN_LOG").is_some()
            {
                eprintln!("zend-zgx: plugin {id}'s log lines didn't reach Zend: {error}");
            }
        }
    });
}

/// The next plugin's lines, in order, up to [`BATCH`]; nothing (and no send
/// on its way any more) once all went.
fn next_batch() -> Option<(String, Vec<Json>)> {
    QUEUE.with(|q| {
        let mut q = q.borrow_mut();
        let Some(id) = q.lines.front().map(|l| l.id.clone()) else {
            q.sending = false;
            return None;
        };
        let mut lines = Vec::new();
        if q.dropped > 0 {
            lines.push(json!({ "level": "info", "text": format!("{} log lines dropped while Zend was away", q.dropped) }));
            q.dropped = 0;
        }
        while lines.len() < BATCH && q.lines.front().is_some_and(|l| l.id == id) {
            let line = q.lines.pop_front().expect("there");
            lines.push(json!({ "level": line.level, "text": line.text }));
        }
        Some((id, lines))
    })
}

/// A connection came: what waited goes now.
pub(crate) fn connected() {
    QUEUE.with(|q| q.borrow_mut().sending = false);
    flush();
}
