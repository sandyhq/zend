# ADR 0007: A Stop names the sends it stops

- Status: Accepted
- Date: 2026-11-17

## Context

A Stop (an `interrupt` command) and the sends around it reach the chat's host by different roads: the doc's sync, the peer relay, in either order. The host had to tell two kinds of send apart. One kind was sent before the Stop: it is part of what the Stop stops, and must not run after it. The other kind is the person's next message: the Stop must leave it alone, and a late Stop aimed at a finished turn is moot. Every rule the host could compute on its own failed for some arrival order. Ledger order failed because a relayed command isn't in the ledger yet. Wall-clock send times failed on the same millisecond and on skewed clocks. A second review reproduced each failure in an engine run. The owner chose to fix it properly across devices.

## Decision

The Stop carries what it stops. `SessionCommandPayload::Interrupt` gets an optional `cancels`: the message ids of the sends its sender knew of whose rows weren't in its transcript yet (`zend_doc::sends_on_their_way`). Its `based_on.turn_id` (the sender's last row) already says which rows the sender knew. The engine (`queue_command_with_transfers`) and the window's client (`queue_command_as`) fill `cancels` in when they queue a Stop.

On the host:
- **Cancelling a send.** A send is cancelled if and only if a Stop names it. That holds even when the Stop itself is moot or gives way to a newer Stop: the person stopped that send either way. The host writes the names to its processed ledger on disk as it claims the Stop, before it acts, and also checks the Stops resolved in the doc's ledger. So the order the two arrive in doesn't matter, nor a restart in between.
- **When the Stop is moot.** It is moot only when a user row after its `based_on` turn is one it doesn't name, or when that turn is gone (a rewind removed it). Such a row is a message its sender didn't know of. No clock is compared. The Stop waits for the named sends still being dispatched, and is judged again after the wait: a send it doesn't name may have started a turn meanwhile.

A Stop without the field (an engine from before this change) keeps the old rules. It is moot when any row comes after its `based_on`, and it cancels the sends before it in the ledger.

Rejected:
- Ledger position. The ledger doesn't hold relayed commands yet.
- Comparing `issued_at` times. Clocks skew, and two commands can share a millisecond.
- A vector clock in `based_on.frontier`. It is more than needed: the sender's last row plus the sends it named is enough.

## Consequences

- The wire shape of an old Stop is unchanged: `{"kind":"interrupt"}`. Older engines ignore `cancels` and judge a new Stop by the old rules.
- A send from a third device that the Stop's sender hadn't seen is not stopped by it, even if it was sent a moment earlier by wall time. That is the causal answer: the person couldn't have meant it.
- The host keeps every name a Stop gave, one processed-ledger key per name (`stop-cancel:<chat>:<message>`), across restarts. (Before 2026-11-17's second review it kept 256 per chat in memory, and lost them on a restart, when the Stop came by relay and its row hadn't synced.)
