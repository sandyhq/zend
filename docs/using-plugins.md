# Using plugins

For people who use Zend. Writing a plugin is in the [plugin guide](../apps/zgx/docs/plugins.md).

## What a plugin is
A small addition to Zend: a page in the sidebar, a badge on a chat, a tool your agents can call, or instructions your agents follow. It is a signed folder under `~/.zend/plugins/<id>/`. It runs in a sandbox and can do only what you allowed.

## Getting one
- **Settings → Plugins → Install…** takes a `.zend-plugin` file or a link (to such a file, a GitHub release, or a git repository).
- **An agent can make or install one for you** (`create_plugin`, `install_plugin`). It then waits for you.

Zend checks the signature over every file first. An unsigned or changed plugin is refused.

## Allowing it
A plugin that asks for more than the harmless basics is **pending** until you allow it, in Zend's window: the sheet shows who made it and each thing it asks for. Only you can allow a plugin. No agent, and no command line, can do it for you.

Read the list. Most permissions are narrow (add a page, read chat titles). A few give real power:

| Permission | Means |
|---|---|
| `commands`, `native`, `service` | it runs programs as you. It can do anything you can. Only for a plugin you trust |
| `messages.send` | it can send messages and start chats as you |
| `session.instructions` | it writes into your agents' system prompt: they follow its text in every chat |
| `accounts.switch`, `accounts.pin`, `models.set` | it changes which account or model your chats use |

These are asked every time they are new, even for plugins you made yourself. The full list: plugin guide, "Permissions".

## Who made it
Zend tells you from the signature, never from what the plugin claims: "by Zend", "Made by you", "Made on one of your devices", "by @username", or "Unverified". "Changed since you allowed it" means its files changed in a way its author's signature does not cover; it does not run until you remove or reinstall it.

## Living with it
- **Settings → Plugins** lists every plugin with its state: on, off, pending, tampered, broken (with the reason).
- A plugin with settings has a page of its own there. If it gives your agents instructions, that page shows the text whole under "What it tells your agents", read only.
- **Off** stops it and keeps what it stored. **Remove** takes away its folder, what it stored, its pages, its log and its instructions.
- **Updates** are yours to install (Settings → Plugins). An update that asks for a new permission, or is signed by a new key, waits for you again.

## When an agent changes a plugin of yours
An agent can try a change without touching your window (a preview apart). To show you a change it can run it in your window as a preview: a bar in the window says "Previewing <name>" with **Revert** and **Keep**. Nothing is kept until you press Keep; Revert brings back your version, its permissions and what it stored. If the changed version fails while loading, your version keeps running and the bar says why.

## If it doesn't work
[troubleshooting.md](troubleshooting.md), "Plugins".
