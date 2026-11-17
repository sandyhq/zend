# ADR 0007: A Stop names the sends it stops

- Status: Accepted
- Date: 2026-11-17

## Context

A Stop (an `interrupt` command) and the sends around it reach the chat's host by different roads: the doc's sync, the peer relay, in either order. The host had to tell two kinds of send apart. One kind was sent before the Stop: it is part of what the Stop stops, and must not run after it. The other kind is the person's next message: the Stop must leave it alone, and a late Stop aimed at a finished turn is moot. Every rule the host could compute on its own failed for some arrival order. Ledger order failed because a relayed command isn't in the ledger yet. Wall-clock send times failed on the same millisecond and on skewed clocks. A second review reproduced each failure in an engine run. The owner chose to fix it properly across devices.

## Decision

The Stop carries what it stops. `SessionCommandPayload::Interrupt` gets an optional `cancels`: the message ids of the sends its sender knew of whose rows weren't in its transcript yet (`zend_doc::sends_on_their_way`). Its `based_on.turn_id` (the sender's last row) already says which rows the sender knew. The engine (`queue_command_with_transfers`) and the window's client (`queue_command_as`) fill `cancels` in when they queue a Stop.

On the host:
- **Cancelling a send.** A send is cancelled if and only if a Stop claims it: the Stop names it, or its sender had its row (one up to the Stop's `based_on` turn) while it hadn't reached an agent yet. That holds even when the Stop itself is moot or gives way to a newer Stop: the person stopped that send either way. The host writes what it claims to its processed ledger on disk as it claims the Stop, before it acts: one key per name, and one for the turn it was aimed at, read by position (a row at or before that turn is claimed), and also checks the Stops resolved in the doc's ledger. So the order the two arrive in doesn't matter, nor a restart in between. The claims also go to the sessions engine. There one function answers "may this message reach the agent" (`StopGate` in sessions.rs, reading disk and memory), and every road a message takes to an agent asks it as its very last step: under the run lock as its runtime is registered (a fresh send, a retry, a revival after a restart), and under the runtime's ledger lock as it is handed to a live one (a routed send, a steer, a queue row sent now, a steer redelivered after a turn stop). A claimed send therefore never runs after its Stop unless it reached the agent before the Stop's kill, and then the kill stops it.
- **When the Stop is moot.** It is moot only when a user row after its `based_on` turn is one it doesn't name, or when that turn is gone (a rewind removed it). Such a row is a message its sender didn't know of. No clock is compared.
- **What the Stop kills.** The Stop waits for no send. It kills the live runtime (one registered for a turn that hasn't opened its stream counts too) only if every message that runtime was handed is one the Stop may stop: its names, the rows up to its `based_on` turn, and what continues them. The runtime keeps every message it was handed for its whole life; no count bounds it, so no count decides what a Stop kills. A runtime that was handed anything else is running a turn the sender never saw, and is left alone (the Stop resolves `superseded`). The decision and the retiring of the runtime happen in one step under the runtime's ledger lock, where sends are handed to it, so no turn reaches the runtime between them; a send after that step starts a fresh runtime, which the Stop doesn't touch. The queue's lock is tried only for a bounded time, so a wedged queue send can't hold the Stop up. (Attempts before this one made the Stop wait for its named sends and for prompts starting; each wait had a bound, and a dispatch slower than the bound broke the rule.)

A Stop without the field (an engine from before this change) keeps the old rules. It is moot when any row comes after its `based_on`, and it cancels the sends before it in the ledger.

Rejected:
- Ledger position. The ledger doesn't hold relayed commands yet.
- Comparing `issued_at` times. Clocks skew, and two commands can share a millisecond.
- A vector clock in `based_on.frontier`. It is more than needed: the sender's last row plus the sends it named is enough.

## Consequences

- The wire shape of an old Stop is unchanged: `{"kind":"interrupt"}`. Older engines ignore `cancels` and judge a new Stop by the old rules.
- A send from a third device that the Stop's sender hadn't seen is not stopped by it, even if it was sent a moment earlier by wall time. That is the causal answer: the person couldn't have meant it.
- Sessions keeps the claims in memory too, per chat, for the engine's life; neither set is pruned yet (a follow-up).
- "The rows up to the `based_on` turn" is read in the host's doc order, the same order the kill rule reads. A row the host doesn't have yet when the Stop's anchor is missing is claimed only by name.
- The host keeps every name a Stop gave, one processed-ledger key per name (`stop-cancel:<chat>:<message>`), and one per turn a Stop was aimed at (`stop-seen:<chat>:<message>`), across restarts. (Before 2026-11-17's second review it kept 256 per chat in memory, and lost them on a restart, when the Stop came by relay and its row hadn't synced.)
