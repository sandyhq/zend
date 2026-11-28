Pi RPC compatibility contract

The native driver targets Pi >= 0.85.1. Requests and events are JSONL, never
JSON-RPC. Responses and lifecycle events must retain stdout order.

`prompt` success acknowledges preflight. Normal runs finish at `agent_settled`,
not `agent_end`; a final error message determines the terminal status after
retries. Extension commands and input handlers may acknowledge without a run.

Legacy ACK barrier (verified against installed 0.85.1 and inspected 0.87.1):
after prompt acceptance request `get_state`. In these versions AgentSession's
`isStreaming` is `_isAgentRunActive`, covering retries/post-run continuations.
The synchronous path sets it immediately after the preflight callback, before
another stdin command can execute. A false isStreaming AND false isCompacting
at this ordered barrier closes a handled prompt (or a run already settled).
This is NOT polling for idle or a quiet timeout; the one re-poll is an idle
barrier reporting pendingMessageCount > 0 (input Pi left queued, with no
settle event coming), bounded before the turn settles as it stands. Process
all preceding events before the response, and scope barriers to the submitted
execution. A future `data.disposition` is additive; the state barrier also
catches extension-started work, which can accompany a handled disposition.

Independent work started later by an extension is a new run (`agent_start`).
Discovery/startup timeouts diagnose startup only; no active model run is timed
out for lack of output. Interruption clears queues before aborting.

Steering uses `prompt` with `streamingBehavior: "steer"`. `set_steering_mode`
persists globally, so Pi's native `all` mode is selected only while neither
global nor project settings configure a mode.
Preflight/state barriers remain serialized, but consumption does not gate the
next submission after `queue_update` confirms an appended steering entry.
Keep one FIFO delivery record per original input, including duplicate text.
`message_start` for each consumed user input emits its own `Steered`; neither
the prompt ACK nor `queue_update` is a consumption receipt. Extension commands
wait for earlier deliveries, and unqueued/handled inputs block pipelining until
consumed or settled. An idle barrier must not confirm queued inputs that never
produced their consumption events.

Sources inspected: Pi dist/core/agent-session.js and dist/modes/rpc/rpc-mode.js
0.85.1, and upstream commit f07218c4d4bbc12bef056a7058c3dd49dfe41abe.

Session identity is refreshed from every accepted state barrier, including
extension-driven new/switch/fork operations. Pi defers file creation until an
assistant message. Only a host-owned empty-session record, checked with
get_entries (model/thinking changes only) and a second idle get_state, permits
recreation using --session-id in the same cwd. Any submitted input revokes that
proof before the write to stdin. Missing history/custom entries never authorize
recreating the same UUID; an unrestorable session starts a new one with a
visible "without the previous context" notice, like the other harnesses.

Native fork (pi-native-fork, verified against 1.5.2 dist): a fork's first turn
(`RunRequest::fork_from`: source session id, `leaf`, fallback prompt) runs a
short Pi `--mode rpc --fork <snapshot> --session-dir <staging>` before the main
one. The source is the session this host's Pi keeps under that id (the index,
Pi's own folders, its global `sessionDir`; never a project's settings, never
the path the doc names), read once through one handle (no symlink) into
`<staging>/source.jsonl`: Pi never opens the source (it appends a missing last
newline to a file it only reads, session-manager.js:367-368). `<staging>` is
a new private folder `.zend-fork-<uuid>` inside the folder the fork is kept
in (a sandboxed chat's `state/sessions`, else the source's own folder), and
goes whole at the end, whatever Pi left there.
`--fork` writes the copy at once, into `--session-dir` when given (main.js
600-603, 342-347; SessionManager.forkFrom session-manager.js:1397-1431), with
`parentSession` = the source. `get_state` names the copy, `get_entries` lists it
(`{entries, leafId}`, rpc-mode.js:525-535). From `leaf` the driver walks the
only line of children past entries beside the conversation (model_change,
thinking_level_change, custom, label, session_info): no child copies the whole
session; the first user message is cut with RPC `fork {entryId}` (rpc-mode.js
503-509, position "before": the new file holds what came before that entry,
agent-session-runtime.js:174-235); anything else refuses. RPC `fork` needs no
prompt first. Its file names the intermediate copy as `parentSession`
(session-manager.js:1260). Pi must name files right in `<staging>`. The
driver reads the result (one handle), keeps it only as one line of entries
from the first to `leaf` (then only entries beside the conversation): `pi
--fork` copies every branch and reopens at the file's last entry, so a source
with another branch is refused. The header is rewritten to name the source
and the file is linked, never over another, into the fork's folder before
the main Pi opens it with `--session`; its first `get_state` must name that
file and session, else it is stopped and the file removed. The fork runs
before `set_model` (configure appends `model_change`). Any failure emits
`NativeForkFallback` at the start of the turn and runs a fresh session with
the fallback prompt. At every turn's end the driver emits `NativeTurn`
(session id, file, the id of the file's last line: Pi appends every entry)
before `Done`; `leaf` is none when the turn took a Pi command (`/session`,
answered by RPC, kept nowhere) or a prompt Pi refused, and the engine marks
no fork point there.
