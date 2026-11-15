# ADR 0006: What a person enters in a visual is kept by Zend, not by the script

- Status: Accepted
- Date: 2026-11-15

## Context

An agent that leads child sessions asks the person questions in a visual. Children settle while the person is still answering, and the lead's next turn runs on. If it shows a new visual, or replaces the old one, what the person typed is gone; if Zend holds back the child's event until the person answers, the work stalls. The owner chose: no holding back; the one visual is updated in place, entries are never overwritten, the agent can read them, and they bind only once the person sends them.

## Decision

Each input in a visual has a fixed `name`. Zend keeps the entries per visual id and input name, outside the script. `update_visual` swaps the script of the same visual: a kept entry wins over a value the new script gives, the input that has focus is not touched, inputs that are new are marked, and an input the script drops keeps its entry. `read_visual` returns the entries with whether they were sent. An answer button appends the entries as text to the chat's composer draft; the person edits and sends. Until then the entries are the person's draft: the agent may read them, not act on them.

Rejected: entries in the script's own state, copied by the agent into the next script (the agent would overwrite what was typed in the meantime; only Zend can merge without a race). Rejected: holding child events while a visual is open.

Status of the build: being built (plan `live-visuals`); this record is updated when it lands.

## Consequences

- The agent keeps working while the person answers, and nothing typed is lost.
- "Don't decide the open questions yourself" stays a request to the model; Zend enforces only that the visual stays visible, the entries survive, and a second visual of the same title is refused in favour of an update.
- A script that renames an input hides its entry (kept, shown nowhere); `update_visual` reports such names.
