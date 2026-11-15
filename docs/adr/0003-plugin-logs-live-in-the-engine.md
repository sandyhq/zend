# ADR 0003: Plugin logs live in the engine

- Status: Accepted
- Date: 2026-11-15

## Context

A plugin runs in two places: its window part (`init.luau`) in the window, its service (`service.luau`) in the engine. Errors went to the window's stderr and a toast showing only the first line, so neither an agent nor the person could read why a plugin failed. Agents reach Zend only through the MCP server, which talks to the engine.

## Decision

The engine keeps one log per plugin: the last 500 lines in memory, appended to `logs/plugins/<id>.log` in Zend's data folder (rotated past 1 MB, a line cut at 16 KB). The window reports its plugins' prints and errors, with the whole traceback, to the engine; the service's go there directly. `plugin_logs` and `zend plugin logs` read it, and `plugin_status` carries the last 20 lines.

Rejected: a log file written by the window itself. Service errors arise in the engine, so there would be two places and two truths, and the MCP server could read neither without the window.

## Consequences

- One place answers "what did this plugin say and why did it fail", for the window part, the service and the check after a save.
- A log can hold chat content if a plugin prints it. It stays on the device: never synced, and only Zend's own window may write to it. Removing a plugin removes its log.
