# When something is wrong

Cases that actually happened, with what was seen and what fixed it. Add a case when you solve one.

## Plugins

**A plugin shows "pending" and does nothing.**
It was installed, by you or an agent, and is not allowed yet. Allow it in Zend's window (Settings → Plugins). An agent cannot allow it; `zend plugin sign <id>` from a terminal answers "Allowing a plugin needs Zend's own consent window".

**A plugin shows "broken", or a toast shows one line of an error.**
Read its log: `zend plugin logs <id>`, or ask an agent (`plugin_logs`). It holds the whole error with file and line. Then `zend plugin check <id>` for syntax and type errors.

**A plugin crashes right after it was changed, with "attempt to index nil".**
The window can still hold data in the shape the old version kept. A plugin must cope with what an older version of itself stored.

**The check says "Types not checked: luau-lsp isn't on this machine".**
Only syntax was checked. With a network connection the first check fetches luau-lsp 1.70.1 into `~/.zend/tools/`; offline, install it yourself or set `LUAU_LSP`.

**`plugin_status` with `snapshot` answers "The window isn't showing".**
An old build (before 2026-11-15 the picture worked on macOS only). Install a current build.

**A preview shows nothing.**
A preview gets the permissions you allowed the plugin. A plugin you never allowed is allowed nothing there and stays empty. Allow it in your Zend first.

**The preview bar (Keep / Revert) does not appear.**
Seen once on the owner's machine: the engine reported the preview as running, no bar was visible. Cause not found. Stop it with `zend plugin preview <id> --in-window --stop`.

**A click in a preview fails with "matches 2 nodes".**
The selector is ambiguous. Narrow it: a role (`role:button "Value"`), a parent (`#sidebar >> "Value"`), or an index (`[1]`, `@649`).

**A plugin's tool fails with "service.luau:<n>: attempt to call a nil value".**
Seen with a plugin written against an older SDK. Cause not looked into; update the plugin's copy of `zend.luau` and check it.

**A plugin's program is not found.**
`zend.run` without a `cwd` starts in the plugin's own folder (since 2026-11-15). Older builds started elsewhere; give `cwd` or an absolute path.

## Agents and instructions

**Agents ignore a plugin's instructions.**
Check, in this order: the plugin is on and allowed; its Settings page shows the text; the chat runs on Pi (other harnesses get no instructions yet); the text was actually set (a plugin may set it only on an explicit Apply); it is under 8 KB.

**The same rules appear twice in an agent's prompt.**
Two plugins set the same text. Turn one off.

**A lead agent seems stuck after starting children.**
It ended its turn on purpose and continues when a child reports. Look at the children: one waiting for input holds the work until it is answered.

## Usage readings

**A limit shows a lower percentage than the provider's own page, and no longer moves.**
The provider's sign-in expired (seen with Claude Code signed out): reading fails and the last value stays. Sign in to that provider again; the next reading is current.

## The app and the CLI

**`zend …` answers "zend-engine has no window: open the Zend app".**
The command needs the window (anything that draws: pictures, previews in the window). Open Zend.

**After installing a new build, something is worse.**
Go back to the previous build: [reference/build-and-deploy.md](reference/build-and-deploy.md), "Going back".

**A test opened a window on my screen.**
A stand-in instance was started without `ZGX_OFFSCREEN=1`: [reference/test-instance.md](reference/test-instance.md).
