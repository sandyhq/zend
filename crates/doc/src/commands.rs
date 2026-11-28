//! Durable command ledger — port of `packages/session-doc/src/commands.ts`.
//!
//! Rules (verbatim from zend's design):
//! 1. Each device inserts only its own entries; entries are append-only and immutable.
//! 2. The chat's HOST is the sole writer of command outcomes; a composer may only set
//!    `cancelled` on its own still-pending entries.
//! 3. Evaluation (`evaluate_command`, pure): processed-id dedupe → Skip; expired TTL → Expired;
//!    a newer interrupt supersedes an older interrupt; user prompts never supersede each other; an interrupt whose
//!    `based_on.turn_id` is already past → Superseded; otherwise Execute.

use serde::{Deserialize, Serialize};

use zend_proto::{RunRequest, UserInputAnswer};

use crate::constants::COMMAND_DEFAULT_TTL_MS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SessionCommandKind {
    Run,
    Steer,
    Interrupt,
    RespondInput,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SessionCommandStatus {
    Pending,
    Applied,
    Rejected,
    Expired,
    Superseded,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SessionCommandPayload {
    #[serde(rename_all = "camelCase")]
    Run {
        request: RunRequest,
        /// Client-minted message id for the optimistic user entry (dedup key).
        message_id: String,
    },
    #[serde(rename_all = "camelCase")]
    Steer {
        prompt: String,
        message_id: Option<String>,
    },
    Interrupt {
        /// What this Stop stops, as its sender saw the chat: the message ids
        /// of the sends it knew of whose rows it hadn't seen yet, still on
        /// their way ([`sends_on_their_way`]). With it the host decides
        /// causally: a named send is cancelled wherever and whenever it
        /// runs, and the Stop is moot only when a user row after its
        /// `based_on` turn is neither named nor known to the sender. Absent
        /// from a Stop of an engine before 2026-11-17, which keeps the turn
        /// rule (any row after `based_on`) and the ledger order.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cancels: Option<Vec<String>>,
    },
    #[serde(rename_all = "camelCase")]
    RespondInput {
        request_id: String,
        answers: Vec<UserInputAnswer>,
    },
}

impl SessionCommandPayload {
    pub fn kind(&self) -> SessionCommandKind {
        match self {
            SessionCommandPayload::Run { .. } => SessionCommandKind::Run,
            SessionCommandPayload::Steer { .. } => SessionCommandKind::Steer,
            SessionCommandPayload::Interrupt { .. } => SessionCommandKind::Interrupt,
            SessionCommandPayload::RespondInput { .. } => SessionCommandKind::RespondInput,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandBasedOn {
    pub turn_id: Option<String>,
    /// The doc version its sender had as it built the command: the Loro
    /// version vector of its oplog, hex ([`encode_version`]). A Stop
    /// carries it to say what its sender saw (2026-11-17); `None` from
    /// older senders.
    pub frontier: Option<String>,
}

impl CommandBasedOn {
    /// What `frontier` says its sender had; `None` when absent or unreadable.
    pub fn sender_version(&self) -> Option<loro::VersionVector> {
        decode_version(self.frontier.as_deref()?)
    }
}

/// A doc version as [`CommandBasedOn::frontier`] carries it.
pub fn encode_version(version: &loro::VersionVector) -> String {
    use std::fmt::Write;
    version
        .encode()
        .iter()
        .fold(String::new(), |mut out, byte| {
            let _ = write!(out, "{byte:02x}");
            out
        })
}

/// [`encode_version`]'s inverse; `None` for anything it didn't make.
pub fn decode_version(text: &str) -> Option<loro::VersionVector> {
    if text.len() % 2 != 0 || !text.is_ascii() {
        return None;
    }
    let bytes = (0..text.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(&text[at..at + 2], 16).ok())
        .collect::<Option<Vec<u8>>>()?;
    loro::VersionVector::decode(&bytes).ok()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionCommandEntry {
    pub id: String,
    pub payload: SessionCommandPayload,
    pub issued_by: String,
    /// Epoch millis.
    pub issued_at: i64,
    #[serde(default)]
    pub based_on: Option<CommandBasedOn>,
    /// Epoch millis; defaults to issued_at + COMMAND_DEFAULT_TTL_MS when absent.
    #[serde(default)]
    pub expires_at: Option<i64>,
    pub status: SessionCommandStatus,
    #[serde(default)]
    pub resolution: Option<String>,
}

impl SessionCommandEntry {
    pub fn kind(&self) -> SessionCommandKind {
        self.payload.kind()
    }

    pub fn effective_expiry(&self) -> i64 {
        self.expires_at
            .unwrap_or(self.issued_at + COMMAND_DEFAULT_TTL_MS)
    }

    /// The user message a Run or Steer writes (a steer without an id takes
    /// the command's).
    pub fn sent_message_id(&self) -> Option<&str> {
        match &self.payload {
            SessionCommandPayload::Run { message_id, .. } => Some(message_id.as_str()),
            SessionCommandPayload::Steer { message_id, .. } => {
                Some(message_id.as_deref().unwrap_or(self.id.as_str()))
            }
            _ => None,
        }
    }

    /// What a Stop says it stops: `Some` for a Stop that names its sends
    /// (`SessionCommandPayload::Interrupt::cancels`), `None` for any other
    /// command and for a Stop of an older engine.
    pub fn stop_cancels(&self) -> Option<&[String]> {
        match &self.payload {
            SessionCommandPayload::Interrupt { cancels } => cancels.as_deref(),
            _ => None,
        }
    }
}

/// The sends a Stop queued now stops, for its `cancels`: each Run or Steer
/// in the ledger, pending or run, whose message isn't in the sender's
/// transcript (`landed`) yet. A row that has landed is known to the sender
/// through `based_on`, which is the transcript's last row.
pub fn sends_on_their_way(
    commands: &[SessionCommandEntry],
    landed: &dyn Fn(&str) -> bool,
) -> Vec<String> {
    commands
        .iter()
        .filter(|c| {
            matches!(
                c.status,
                SessionCommandStatus::Pending | SessionCommandStatus::Applied
            )
        })
        .filter_map(SessionCommandEntry::sent_message_id)
        .filter(|id| !landed(id))
        .map(str::to_owned)
        .collect()
}

/// Rule 2: only the composer that issued a still-pending command may cancel it.
pub fn can_composer_cancel(entry: &SessionCommandEntry, device_id: &str) -> bool {
    entry.status == SessionCommandStatus::Pending && entry.issued_by == device_id
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandDisposition {
    /// Already in the processed ledger — do nothing (idempotence).
    Skip,
    /// Mark expired.
    Expired,
    /// Mark superseded.
    Superseded,
    /// Mark processed BEFORE executing, then execute.
    Execute,
}

/// Context the host evaluates a pending command against.
pub struct EvaluationContext<'a> {
    /// Processed-command ledger membership test.
    pub is_processed: &'a dyn Fn(&str) -> bool,
    /// Current wall clock, epoch millis.
    pub now_ms: i64,
    /// All command entries in doc order (used to find newer same-kind entries).
    pub entries: &'a [SessionCommandEntry],
    /// The id of the turn currently (or most recently) running, if any.
    pub current_turn_id: Option<&'a str>,
    /// True when the given turn id has already completed.
    pub turn_is_past: &'a dyn Fn(&str) -> bool,
}

/// Rule 3 — pure evaluation of a single pending command.
pub fn evaluate_command(
    entry: &SessionCommandEntry,
    cx: &EvaluationContext<'_>,
) -> CommandDisposition {
    if (cx.is_processed)(&entry.id) {
        return CommandDisposition::Skip;
    }
    if cx.now_ms >= entry.effective_expiry() {
        return CommandDisposition::Expired;
    }
    // Interrupts are redundant control signals; steers are distinct user messages.
    // Coalescing steers loses prompts when remote sync delivers a batch.
    let kind = entry.kind();
    if kind == SessionCommandKind::Interrupt {
        let has_newer_same_kind = cx.entries.iter().any(|other| {
            other.id != entry.id
                && other.kind() == kind
                && other.status == SessionCommandStatus::Pending
                && other.issued_at > entry.issued_at
        });
        if has_newer_same_kind {
            return CommandDisposition::Superseded;
        }
    }
    // An interrupt aimed at a turn that already finished is moot.
    if kind == SessionCommandKind::Interrupt
        && let Some(based_on) = &entry.based_on
        && let Some(turn_id) = &based_on.turn_id
    {
        let is_current = cx.current_turn_id == Some(turn_id.as_str());
        if !is_current && (cx.turn_is_past)(turn_id) {
            return CommandDisposition::Superseded;
        }
    }
    CommandDisposition::Execute
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, payload: SessionCommandPayload, issued_at: i64) -> SessionCommandEntry {
        SessionCommandEntry {
            id: id.into(),
            payload,
            issued_by: "device-a".into(),
            issued_at,
            based_on: None,
            expires_at: None,
            status: SessionCommandStatus::Pending,
            resolution: None,
        }
    }

    fn steer(id: &str, issued_at: i64) -> SessionCommandEntry {
        entry(
            id,
            SessionCommandPayload::Steer {
                prompt: "go".into(),
                message_id: None,
            },
            issued_at,
        )
    }

    fn cx<'a>(
        entries: &'a [SessionCommandEntry],
        processed: &'a dyn Fn(&str) -> bool,
        turn_is_past: &'a dyn Fn(&str) -> bool,
        now_ms: i64,
        current_turn_id: Option<&'a str>,
    ) -> EvaluationContext<'a> {
        EvaluationContext {
            is_processed: processed,
            now_ms,
            entries,
            current_turn_id,
            turn_is_past,
        }
    }

    const NEVER: fn(&str) -> bool = |_| false;

    #[test]
    fn processed_commands_are_skipped() {
        let e = steer("c1", 1_000);
        let entries = vec![e.clone()];
        let processed = |id: &str| id == "c1";
        let cx = cx(&entries, &processed, &NEVER, 2_000, None);
        assert_eq!(evaluate_command(&e, &cx), CommandDisposition::Skip);
    }

    #[test]
    fn expired_commands_are_expired() {
        let e = steer("c1", 0);
        let entries = vec![e.clone()];
        let cx = cx(&entries, &NEVER, &NEVER, COMMAND_DEFAULT_TTL_MS + 1, None);
        assert_eq!(evaluate_command(&e, &cx), CommandDisposition::Expired);
    }

    #[test]
    fn newer_steer_preserves_older_pending_steer() {
        let older = steer("c1", 1_000);
        let newer = steer("c2", 2_000);
        let entries = vec![older.clone(), newer.clone()];
        let cx1 = cx(&entries, &NEVER, &NEVER, 3_000, None);
        assert_eq!(evaluate_command(&older, &cx1), CommandDisposition::Execute);
        assert_eq!(evaluate_command(&newer, &cx1), CommandDisposition::Execute);
    }

    #[test]
    fn batched_remote_steers_all_execute_once_even_with_clock_skew() {
        use std::collections::HashSet;
        for count in [2, 20, 200] {
            let entries: Vec<_> = (0..count)
                .map(|i| {
                    entry(
                        &format!("c{i}"),
                        SessionCommandPayload::Steer {
                            prompt: format!("unique message {i}"),
                            message_id: Some(format!("m{i}")),
                        },
                        1_000 + (i * 17 % 23),
                    )
                })
                .collect();
            let mut processed = HashSet::new();
            for _ in 0..3 {
                for entry in &entries {
                    let expected = if processed.contains(&entry.id) {
                        CommandDisposition::Skip
                    } else {
                        CommandDisposition::Execute
                    };
                    let seen = |id: &str| processed.contains(id);
                    let context = cx(&entries, &seen, &NEVER, 2_000, None);
                    assert_eq!(evaluate_command(entry, &context), expected);
                    processed.insert(entry.id.clone());
                }
            }
            assert_eq!(processed.len(), count as usize);
        }
    }

    #[test]
    fn newer_interrupt_still_supersedes_older_interrupt() {
        let entries = vec![
            entry(
                "i1",
                SessionCommandPayload::Interrupt { cancels: None },
                1_000,
            ),
            entry(
                "i2",
                SessionCommandPayload::Interrupt { cancels: None },
                2_000,
            ),
        ];
        let context = cx(&entries, &NEVER, &NEVER, 3_000, None);
        assert_eq!(
            evaluate_command(&entries[0], &context),
            CommandDisposition::Superseded
        );
        assert_eq!(
            evaluate_command(&entries[1], &context),
            CommandDisposition::Execute
        );
    }

    #[test]
    fn interrupt_for_past_turn_is_superseded() {
        let mut e = entry(
            "c1",
            SessionCommandPayload::Interrupt { cancels: None },
            1_000,
        );
        e.based_on = Some(CommandBasedOn {
            turn_id: Some("turn-1".into()),
            frontier: None,
        });
        let entries = vec![e.clone()];
        let past = |id: &str| id == "turn-1";
        let cx1 = cx(&entries, &NEVER, &past, 2_000, Some("turn-2"));
        assert_eq!(evaluate_command(&e, &cx1), CommandDisposition::Superseded);
        // …but if that turn is still the current one, execute.
        let cx2 = cx(&entries, &NEVER, &past, 2_000, Some("turn-1"));
        assert_eq!(evaluate_command(&e, &cx2), CommandDisposition::Execute);
    }

    #[test]
    fn runs_are_not_superseded_by_newer_runs() {
        // Two queued runs both execute (in order); supersession applies to interrupts only.
        let r1 = entry(
            "r1",
            SessionCommandPayload::Run {
                request: run_request(),
                message_id: "m1".into(),
            },
            1_000,
        );
        let r2 = entry(
            "r2",
            SessionCommandPayload::Run {
                request: run_request(),
                message_id: "m2".into(),
            },
            2_000,
        );
        let entries = vec![r1.clone(), r2.clone()];
        let cx1 = cx(&entries, &NEVER, &NEVER, 3_000, None);
        assert_eq!(evaluate_command(&r1, &cx1), CommandDisposition::Execute);
        assert_eq!(evaluate_command(&r2, &cx1), CommandDisposition::Execute);
    }

    #[test]
    fn composer_cancel_rules() {
        let e = steer("c1", 1_000);
        assert!(can_composer_cancel(&e, "device-a"));
        assert!(!can_composer_cancel(&e, "device-b"));
        let mut applied = e.clone();
        applied.status = SessionCommandStatus::Applied;
        assert!(!can_composer_cancel(&applied, "device-a"));
    }

    fn run_request() -> RunRequest {
        RunRequest {
            mcp: None,
            prompt: "hello".into(),
            harness: None,
            model: None,
            reasoning: None,
            model_options: Default::default(),
            cwd: "/tmp".into(),
            sandbox: zend_proto::SandboxLevel::WorkspaceWrite,
            auto_approve: false,
            access: Default::default(),
            plan: false,
            attachments: Vec::new(),
            worktree: None,
            resume: None,
            env: Default::default(),
            extra_mcp: Vec::new(),
            instructions: Vec::new(),
            fork_from: None,
        }
    }

    #[test]
    fn a_stop_without_cancels_keeps_the_old_wire_shape() {
        let old = SessionCommandPayload::Interrupt { cancels: None };
        assert_eq!(
            serde_json::to_value(&old).unwrap(),
            serde_json::json!({ "kind": "interrupt" })
        );
        let read: SessionCommandPayload =
            serde_json::from_value(serde_json::json!({ "kind": "interrupt" })).unwrap();
        assert_eq!(read, old);
        let named = SessionCommandPayload::Interrupt {
            cancels: Some(vec!["m-1".into()]),
        };
        let json = serde_json::to_value(&named).unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "kind": "interrupt", "cancels": ["m-1"] })
        );
        assert_eq!(
            serde_json::from_value::<SessionCommandPayload>(json).unwrap(),
            named
        );
    }

    #[test]
    fn a_stop_names_the_sends_whose_rows_have_not_landed() {
        let mut ran = entry(
            "c-ran",
            SessionCommandPayload::Steer {
                prompt: "go".into(),
                message_id: Some("m-ran".into()),
            },
            1,
        );
        ran.status = SessionCommandStatus::Applied;
        let mut gone = steer("c-gone", 2);
        gone.status = SessionCommandStatus::Expired;
        let commands = vec![
            ran,
            steer("c-landed", 3),
            steer("c-on-its-way", 4),
            gone,
            entry(
                "c-stop",
                SessionCommandPayload::Interrupt { cancels: None },
                5,
            ),
        ];
        let landed = |id: &str| id == "c-landed";
        assert_eq!(
            sends_on_their_way(&commands, &landed),
            vec!["m-ran".to_string(), "c-on-its-way".to_string()]
        );
    }
}
