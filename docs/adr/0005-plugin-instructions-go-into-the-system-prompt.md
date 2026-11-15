# ADR 0005: Plugin instructions go into the agent's system prompt

- Status: Accepted
- Date: 2026-11-15

## Context

The owner wants plugins to tell agents how to work (how to delegate, when to show a visual), for every chat, child sessions and subagents included. The existing ways were weak: an MCP server's `instructions` and tool descriptions may be shortened by a harness, are fixed per server start and cannot differ per chat; a normal message lands in the transcript as a user line, counts as a turn and is lost at compaction.

## Decision

A new permission, `session.instructions`, always asked, even for the person's own plugins. A plugin sets entries with `zend.set_instructions { text, key?, chat?, project? }` or by answering `run.preparing` with `{ instructions }`. The engine keeps the entries per plugin and gives each chat's agent the plugin's text, whole, as a section of its own (`plugin-<id>`) in the system prompt, renewed at the start of every turn. Of one key the nearest stands: a chat's over its project's over the device's. A child session hears its parent's entries too. The limit is 8 KB a chat for each plugin.

Built for Pi first; other harnesses run without it until their adapters follow (Claude Code, then Codex, then ACP).

Rejected: MCP instructions only (no engine change, but static, per plugin a program of its own, and how each harness passes them on was unverified). Rejected: instructions as a message.

## Consequences

- The model reads every word on every turn, and the text survives compaction.
- A plugin writes into the system prompt: that is prompt injection by design. Hence the permission is always asked, the size is capped, every change is recorded as the plugin's, and Settings shows the text whole, read only.
- Changing an entry costs running chats their prompt cache once. Plugins should set text on an explicit action, not while the person types.
- Chats on harnesses other than Pi get nothing yet, silently.
