# ADR 0004: A plugin preview is a second Zend, then the person's window

- Status: Accepted
- Date: 2026-11-15

## Context

An agent changing a plugin needs to see and try the result before the person does, without breaking the plugin the person is using and without taking over their screen. A broken version loaded into the real window replaces a working one.

## Decision

Two stages.

1. **Apart.** `preview_plugin { path }` runs a copy of the folder in a Zend of its own: an engine on its own port with its own data folder (`plugin-preview/<id>/`), and a window that is drawn but never shown. The copy gets the permissions the person allowed the plugin and a copy of what it keeps. `plugin_status`, `plugin_logs` and `plugin_input` with `preview = true` look and act there. Input is by selector (`role:button "Usage"`), the same selectors `zend.place` takes. It stops after 30 minutes without a call and with the engine that started it.
2. **In the person's window.** `preview_plugin { path, in_window = true }` runs the changed copy in place of the kept version, with a bar offering Revert and Keep. Only the bar's Keep keeps it; agents and other devices are refused.

Rejected: a sandbox inside the one window (a failure there is a failure in the person's window). Rejected: screenshots of the desktop (shows the person's screen, needs a visible workspace, one compositor only). Rejected: input by pixel coordinates alone (breaks on every layout change).

## Consequences

- An agent can iterate (edit, check, preview, picture, click) with no person present, and what breaks stays in the preview.
- The preview's plugin runs with real permissions on the real machine: programs it runs (`commands`) run as the person. Only Zend's data folder is separate.
- Keeping is the person's decision, as allowing a plugin is.
- The offscreen window needs changes in `third_party/zgx` (`ZGX_OFFSCREEN`), which makes merging upstream harder.
