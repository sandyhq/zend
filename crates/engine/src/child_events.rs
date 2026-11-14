//! A child chat's settled turns, told to its parent (docs/mcp.md, "Child
//! events").
//!
//! The engine that hosts a child (a chat with `parent_chat_id`) watches its
//! own runs. Each time one of its turns settles (completed, errored,
//! interrupted, or asking for input) the parent gets one line Zend writes, not
//! the person: `[Zend event: chat <title> (<id8>) <outcome>. <its last reply,
//! or the question it asks>. Read more: read_chat <id>.]`. It goes in the way
//! Zend's MCP `send_message` goes in mode auto: a parent at rest starts a
//! turn with it, a working parent reads it at its next input boundary (the
//! live mailbox, or held for the turn's end where the agent can't take it
//! mid-turn), and a parent waiting on the person gets it after that turn.
//! It's a durable chat command, so it's never lost and never holds the child
//! up. An orchestrator ends its turn after launching children and hears from
//! each: nothing waits, nothing polls.
//!
//! - Children of one parent settling within [`WINDOW`] of the first are told
//!   in one message, one line each. The window is fixed from the first, so a
//!   steady stream never holds a line back for long.
//! - Not told: a turn whose sender awaits it (Zend MCP's `wait: true`, which
//!   returns the reply itself: [`zend_doc::SessionDoc::awaited_sends`]), an
//!   archived child, an archived or missing parent, a child whose
//!   `notifyParent` is off (unsaid: on for subagents and child sessions,
//!   off for side chats),
//!   and a turn an event itself started, so an event never makes another.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use chrono::{DateTime, Utc};
use tokio::time::Instant;
use zend_doc::{MessagePart, MessageRole, SessionCommandPayload, SessionMessageEntry};
use zend_proto::{Chat, ChildKind, DoneStatus, Session, SessionStatus};

use crate::doc_host::DocHost;
use crate::sessions::SessionsEngine;
use crate::workspace_host::WorkspaceHost;

/// How every event line starts. Distinct from the `[Message from Zend chat`
/// header of a message one agent sends another.
pub const EVENT_PREFIX: &str = "[Zend event: ";

/// How long after a child settles its parent hears of it, so children
/// settling together arrive together.
pub const WINDOW: Duration = Duration::from_secs(2);

/// How much of a reply an event carries; `read_chat` has the rest.
const EXCERPT_CHARS: usize = 600;

/// How a child's turn settled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Outcome {
    Completed,
    Errored,
    Interrupted,
    AwaitingInput,
}

impl Outcome {
    fn words(self) -> &'static str {
        match self {
            Outcome::Completed => "completed",
            Outcome::Errored => "errored",
            Outcome::Interrupted => "was interrupted",
            Outcome::AwaitingInput => "is waiting for input",
        }
    }
}

/// What the watcher last saw of a session row.
#[derive(Debug, Clone, Default, PartialEq)]
struct Seen {
    status: Option<SessionStatus>,
    finished_at: Option<DateTime<Utc>>,
}

impl Seen {
    fn of(session: &Session) -> Self {
        Self {
            status: Some(session.status),
            finished_at: session.last_turn_outcome.as_ref().map(|o| o.finished_at),
        }
    }
}

/// The turn that settled between `prev` and `session`, if one did. A turn
/// ends with a fresh outcome stamp on a settled row; a parked wait (the agent
/// waits on background work) and a turn carried on by an accepted steer
/// (still Working) haven't settled yet. Asking for input settles a turn too,
/// once per question.
fn settled(prev: &Seen, session: &Session) -> Option<Outcome> {
    match session.status {
        SessionStatus::AwaitingInput => {
            (prev.status != Some(SessionStatus::AwaitingInput)).then_some(Outcome::AwaitingInput)
        }
        SessionStatus::Idle | SessionStatus::Errored => {
            if session.waiting.is_some() {
                return None;
            }
            let outcome = session.last_turn_outcome.as_ref()?;
            if prev.finished_at == Some(outcome.finished_at) {
                return None;
            }
            Some(match outcome.status {
                DoneStatus::Completed => Outcome::Completed,
                DoneStatus::Interrupted => Outcome::Interrupted,
                DoneStatus::Errored => Outcome::Errored,
            })
        }
        SessionStatus::Working => None,
    }
}

/// One child's settled turn, waiting for its parent's window to close.
#[derive(Debug, Clone)]
struct Pending {
    child: String,
    outcome: Outcome,
}

/// The decision state, apart from the watch so tests can drive it.
pub(crate) struct ChildEvents {
    sessions: SessionsEngine,
    doc_host: DocHost,
    workspace: WorkspaceHost,
    seen: HashMap<String, Seen>,
    /// Awaited sends already spent: a mark covers the first settle after
    /// it (where `wait: true` returns), not the turn's later ones.
    spent: HashSet<(String, String)>,
    /// By parent: when its window closes, and what it will hear.
    due: HashMap<String, (Instant, Vec<Pending>)>,
}

impl ChildEvents {
    pub(crate) fn new(sessions: SessionsEngine, doc_host: DocHost, workspace: WorkspaceHost) -> Self {
        Self {
            sessions,
            doc_host,
            workspace,
            seen: HashMap::new(),
            spent: HashSet::new(),
            due: HashMap::new(),
        }
    }

    /// Take `sessions` as the baseline without telling anyone (boot, replay).
    fn seed(&mut self, sessions: &[Session]) {
        for session in sessions {
            self.seen.insert(session.chat_id.clone(), Seen::of(session));
        }
    }

    /// The rows changed: note each child turn that settled.
    fn observe(&mut self, sessions: &[Session], now: Instant) {
        for session in sessions {
            let prev = self
                .seen
                .insert(session.chat_id.clone(), Seen::of(session))
                .unwrap_or_default();
            if let Some(outcome) = settled(&prev, session) {
                self.consider(&session.chat_id, outcome, now);
            }
        }
    }

    fn consider(&mut self, child_id: &str, outcome: Outcome, now: Instant) {
        let Ok(Some(chat)) = self.workspace.chat(child_id) else {
            return;
        };
        let Some(parent) = chat.parent_chat_id.clone().filter(|p| !p.is_empty()) else {
            return;
        };
        if chat.archived {
            return;
        }
        let Ok(handle) = self.doc_host.open(child_id) else {
            return;
        };
        let doc = handle.doc();
        let notify = doc
            .notify_parent()
            .unwrap_or(matches!(chat.child_kind, Some(ChildKind::Subagent | ChildKind::Session)));
        if !notify {
            return;
        }
        let entries = doc.read_entries_shared().unwrap_or_default();
        if let Some(trigger) = entries.iter().rev().find(|e| e.role == MessageRole::User) {
            // Started by an event: the parent's own work answers it, not
            // another event.
            if entry_text(trigger).trim_start().starts_with(EVENT_PREFIX) {
                tracing::debug!(chat = %child_id, "child events: a turn an event started tells no one");
                return;
            }
            let key = (child_id.to_owned(), trigger.id.clone());
            if !self.spent.contains(&key)
                && doc
                    .awaited_sends()
                    .get(&trigger.id)
                    .is_some_and(|until| *until >= crate::now_ms())
            {
                self.spent.insert(key);
                tracing::debug!(chat = %child_id, "child events: its sender awaits this turn");
                return;
            }
        }
        tracing::info!(chat = %child_id, %parent, ?outcome, "child events: a child's turn settled");
        self.due
            .entry(parent)
            .or_insert_with(|| (now + WINDOW, Vec::new()))
            .1
            .push(Pending {
                child: child_id.to_owned(),
                outcome,
            });
    }

    /// When the next parent's window closes.
    fn next_due(&self) -> Option<Instant> {
        self.due.values().map(|(at, _)| *at).min()
    }

    /// Tell every parent whose window has closed.
    fn flush(&mut self, now: Instant) {
        let closed: Vec<String> = self
            .due
            .iter()
            .filter(|(_, (at, _))| *at <= now)
            .map(|(parent, _)| parent.clone())
            .collect();
        for parent in closed {
            if let Some((_, items)) = self.due.remove(&parent) {
                self.tell(&parent, items);
            }
        }
    }

    fn tell(&self, parent_id: &str, items: Vec<Pending>) {
        let parent = match self.workspace.chat(parent_id) {
            Ok(Some(parent)) if !parent.archived => parent,
            _ => {
                tracing::debug!(parent = %parent_id, "child events: the parent is archived or gone");
                return;
            }
        };
        let lines: Vec<String> = items
            .iter()
            .filter_map(|item| {
                let chat = self.workspace.chat(&item.child).ok().flatten()?;
                if chat.archived {
                    return None;
                }
                Some(self.line(&chat, item.outcome))
            })
            .collect();
        if lines.is_empty() {
            return;
        }
        let text = lines.join("\n");
        match self.deliver(&parent, &text) {
            Ok(how) => tracing::info!(parent = %parent_id, events = lines.len(), how, "child events: told the parent"),
            Err(error) => tracing::warn!(parent = %parent_id, %error, "child events: couldn't tell the parent"),
        }
    }

    /// The event line for `chat`'s settled turn.
    fn line(&self, chat: &Chat, outcome: Outcome) -> String {
        let entries = self
            .doc_host
            .open(&chat.id)
            .and_then(|h| h.doc().read_entries_shared().map_err(Into::into))
            .unwrap_or_default();
        event_line(chat, outcome, &entries)
    }

    /// In as Zend's MCP `send_message` mode auto would put it: a steer into
    /// a working parent, held for the end of a turn waiting on the person,
    /// a new turn otherwise. How it went in.
    fn deliver(&self, parent: &Chat, text: &str) -> Result<&'static str, crate::EngineError> {
        // This device's own row, else the one its host mirrors.
        let status = self
            .sessions
            .session_status(&parent.id)
            .or_else(|| {
                self.workspace
                    .read_sessions()
                    .ok()?
                    .into_iter()
                    .find(|s| s.chat_id == parent.id)
            })
            .map(|s| s.status);
        match status {
            Some(SessionStatus::Working) => {
                let payload = SessionCommandPayload::Steer {
                    prompt: text.to_owned(),
                    message_id: Some(crate::new_id()),
                };
                self.doc_host.queue_command_with_transfers(&parent.id, payload, Vec::new())?;
                Ok("steer")
            }
            Some(SessionStatus::AwaitingInput) => {
                self.doc_host.queue_message_with_behavior(&parent.id, text, Vec::new(), true)?;
                Ok("queue")
            }
            _ => {
                let Some(mut request) = self.doc_host.request_from_chat_row(&parent.id, text) else {
                    return Err(crate::EngineError::Other(format!("no chat {}", parent.id)));
                };
                if request.cwd.is_empty() {
                    request.cwd = parent
                        .space_id
                        .as_deref()
                        .and_then(|id| self.workspace.space(id).ok().flatten())
                        .map(|space| space.path)
                        .unwrap_or_else(|| "~".into());
                }
                let payload = SessionCommandPayload::Run {
                    request,
                    message_id: crate::new_id(),
                };
                self.doc_host.queue_command_with_transfers(&parent.id, payload, Vec::new())?;
                Ok("run")
            }
        }
    }
}

/// Watch this engine's runs and tell parents of their children's settled
/// turns until the sessions engine goes away.
pub(crate) fn spawn(sessions: SessionsEngine, doc_host: DocHost, workspace: WorkspaceHost) -> tokio::task::JoinHandle<()> {
    let mut rows = sessions.watch_sessions();
    let mut events = ChildEvents::new(sessions, doc_host, workspace);
    events.seed(&rows.borrow_and_update());
    tokio::spawn(async move {
        loop {
            let due = events.next_due();
            tokio::select! {
                changed = rows.changed() => {
                    if changed.is_err() {
                        break;
                    }
                    let list = rows.borrow_and_update().clone();
                    events.observe(&list, Instant::now());
                }
                () = async {
                    match due {
                        Some(at) => tokio::time::sleep_until(at).await,
                        None => std::future::pending().await,
                    }
                } => events.flush(Instant::now()),
            }
        }
    })
}

/// A chat as an event names it: `Title (id8)`, on one line.
fn label(chat: &Chat) -> String {
    let short: String = chat.id.chars().take(8).collect();
    let title = chat
        .title
        .as_deref()
        .map(one_line)
        .filter(|t| !t.is_empty());
    match title {
        Some(title) => format!("{title} ({short})"),
        None => short,
    }
}

/// `text` with every run of whitespace one space, so nothing it says can
/// start a line of its own (an event line is one line).
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let mut out: String = text.chars().take(max).collect();
    out.push('…');
    out
}

fn entry_text(entry: &SessionMessageEntry) -> String {
    entry
        .parts
        .iter()
        .filter_map(|part| match part {
            MessagePart::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn entry_error(entry: &SessionMessageEntry) -> Option<String> {
    entry.parts.iter().rev().find_map(|part| match part {
        MessagePart::Error { message, .. } => Some(message.clone()),
        _ => None,
    })
}

/// The event line itself, from the child's transcript.
fn event_line(chat: &Chat, outcome: Outcome, entries: &[std::sync::Arc<SessionMessageEntry>]) -> String {
    // This turn: everything after its last user message.
    let start = entries
        .iter()
        .rposition(|e| e.role == MessageRole::User)
        .map_or(0, |i| i + 1);
    let turn = &entries[start..];
    let body = match outcome {
        Outcome::AwaitingInput => {
            let asked = entries.iter().rev().flat_map(|e| e.parts.iter().rev()).find_map(|part| match part {
                MessagePart::Input {
                    request_id,
                    questions,
                    resolved: false,
                    ..
                } => Some((request_id.clone(), questions.clone())),
                _ => None,
            });
            match asked {
                Some((request_id, questions)) => {
                    let asks = questions
                        .iter()
                        .map(|q| {
                            let options = if q.options.is_empty() {
                                String::new()
                            } else {
                                format!(" [{}]", q.options.join(" / "))
                            };
                            format!("{}{options}", q.question)
                        })
                        .collect::<Vec<_>>()
                        .join(" | ");
                    format!(
                        "It asks: {} (request {request_id}; answer with respond_to_input).",
                        clip(&one_line(&asks), EXCERPT_CHARS)
                    )
                }
                None => "It is waiting for an answer.".into(),
            }
        }
        _ => {
            let reply = turn
                .iter()
                .rev()
                .filter(|e| e.role == MessageRole::Assistant)
                .map(|e| one_line(&entry_text(e)))
                .find(|t| !t.is_empty());
            let error = (outcome == Outcome::Errored)
                .then(|| turn.iter().rev().find_map(|e| entry_error(e)))
                .flatten()
                .map(|e| one_line(&e))
                .filter(|e| !e.is_empty());
            match (reply, error) {
                (Some(reply), Some(error)) => format!(
                    "Error: {} Last reply: {}",
                    clip(&error, 200),
                    clip(&reply, EXCERPT_CHARS)
                ),
                (Some(reply), None) => format!("Last reply: {}", clip(&reply, EXCERPT_CHARS)),
                (None, Some(error)) => format!("Error: {}", clip(&error, EXCERPT_CHARS)),
                (None, None) => "No reply text.".into(),
            }
        }
    };
    format!(
        "{EVENT_PREFIX}chat {} {}. {body} Read more: read_chat {}.]",
        label(chat),
        outcome.words(),
        chat.id
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use zend_proto::NativeTurnOutcome;

    fn row(status: SessionStatus, outcome: Option<(DoneStatus, i64)>) -> Session {
        Session {
            last_completed_turn: None,
            chat_id: "c".into(),
            device_id: "d".into(),
            status,
            started_at: None,
            updated_at: Utc::now(),
            last_turn_outcome: outcome.map(|(status, at)| NativeTurnOutcome {
                status,
                finished_at: DateTime::from_timestamp_millis(at).unwrap(),
            }),
            terminal_activity: None,
            running_subagents: 0,
            background_tasks: Vec::new(),
            waiting: None,
        }
    }

    #[test]
    fn a_turn_settles_once_per_outcome_stamp_and_per_question() {
        let working = row(SessionStatus::Working, None);
        let done = row(SessionStatus::Idle, Some((DoneStatus::Completed, 1)));
        assert_eq!(settled(&Seen::of(&working), &done), Some(Outcome::Completed));
        assert_eq!(settled(&Seen::of(&done), &done), None, "the same stamp again");
        // A watch that coalesced the Working row away still sees the new stamp.
        let again = row(SessionStatus::Idle, Some((DoneStatus::Interrupted, 2)));
        assert_eq!(settled(&Seen::of(&done), &again), Some(Outcome::Interrupted));
        let failed = row(SessionStatus::Errored, Some((DoneStatus::Errored, 3)));
        assert_eq!(settled(&Seen::of(&working), &failed), Some(Outcome::Errored));
        let asks = row(SessionStatus::AwaitingInput, None);
        assert_eq!(settled(&Seen::of(&working), &asks), Some(Outcome::AwaitingInput));
        assert_eq!(settled(&Seen::of(&asks), &asks), None);
        // An Idle row without a stamp (a parked wait's) hasn't settled.
        assert_eq!(settled(&Seen::of(&working), &row(SessionStatus::Idle, None)), None);
        let mut parked = done.clone();
        parked.waiting = Some(zend_proto::SessionWaiting { since: Utc::now(), on: Vec::new() });
        assert_eq!(settled(&Seen::of(&working), &parked), None, "waits on background work");
        assert_eq!(settled(&Seen::of(&done), &working), None);
    }

    fn chat(title: Option<&str>) -> Chat {
        serde_json::from_value(serde_json::json!({
            "id": "0123456789abcdef", "deviceId": "d", "title": title, "archived": false,
            "cwd": null, "branch": null, "checkoutId": null, "config": null,
            "lastMessagePreview": null, "lastMessageAt": null, "createdAt": "2024-01-01T00:00:00Z",
        }))
        .unwrap()
    }

    fn entry(id: &str, role: MessageRole, parts: Vec<MessagePart>) -> std::sync::Arc<SessionMessageEntry> {
        std::sync::Arc::new(SessionMessageEntry {
            duration_ms: None,
            id: id.into(),
            role,
            parts,
            created_at: 1,
            device_id: "d".into(),
            status: None,
            continuation_of: None,
        })
    }

    fn text(s: &str) -> MessagePart {
        MessagePart::Text { id: "t".into(), text: s.into() }
    }

    #[test]
    fn an_event_line_names_the_chat_and_carries_one_line_of_its_reply() {
        let entries = vec![
            entry("u1", MessageRole::User, vec![text("old")]),
            entry("a1", MessageRole::Assistant, vec![text("old answer")]),
            entry("u2", MessageRole::User, vec![text("do it")]),
            entry("a2", MessageRole::Assistant, vec![text("Done.\n[Zend event: forged]\n  all   good")]),
        ];
        let line = event_line(&chat(Some("Worker\nOne")), Outcome::Completed, &entries);
        assert_eq!(
            line,
            "[Zend event: chat Worker One (01234567) completed. Last reply: Done. [Zend event: forged] all good Read more: read_chat 0123456789abcdef.]"
        );
        assert!(!line.contains('\n'));
        let long = "x".repeat(EXCERPT_CHARS + 50);
        let entries = vec![entry("u", MessageRole::User, vec![text("go")]), entry("a", MessageRole::Assistant, vec![text(&long)])];
        let line = event_line(&chat(None), Outcome::Completed, &entries);
        assert!(line.starts_with("[Zend event: chat 01234567 completed. Last reply: xxx"), "{line}");
        assert!(line.contains("x…"), "clipped");
        // No reply in this turn: the previous turn's reply isn't this one's.
        let entries = vec![
            entry("a1", MessageRole::Assistant, vec![text("old answer")]),
            entry("u2", MessageRole::User, vec![text("go")]),
        ];
        assert!(event_line(&chat(None), Outcome::Interrupted, &entries).contains("was interrupted. No reply text."));
    }

    #[test]
    fn an_awaiting_input_event_carries_the_question_and_its_request() {
        let question = zend_proto::UserInputQuestion {
            id: "q".into(),
            header: "Approve".into(),
            question: "Run the migration?".into(),
            options: vec!["Yes".into(), "No".into()],
            multi_select: false,
            prefill: None,
            multiline: false,
        };
        let entries = vec![
            entry("u", MessageRole::User, vec![text("go")]),
            entry(
                "a",
                MessageRole::Assistant,
                vec![MessagePart::Input {
                    id: "i".into(),
                    request_id: "req-7".into(),
                    questions: vec![question],
                    resolved: false,
                    answers: None,
                }],
            ),
        ];
        let line = event_line(&chat(Some("W")), Outcome::AwaitingInput, &entries);
        assert_eq!(
            line,
            "[Zend event: chat W (01234567) is waiting for input. It asks: Run the migration? [Yes / No] (request req-7; answer with respond_to_input). Read more: read_chat 0123456789abcdef.]"
        );
    }
}
