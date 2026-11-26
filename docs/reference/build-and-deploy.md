# Building and installing this fork locally

How a branch becomes the Zend that runs on the owner's Linux machine, and how to go back. This is the path used on 2026-11-15; there is no release pipeline for the fork.

## Where the installed Zend lives
| Path | What |
|---|---|
| `~/.local/opt/zend/` | the running build: `zend-zgx` (window), `zend-engine`, `ui/`, `plugins-builtin/` |
| `~/.local/opt/zend-next/` | a built, not yet installed build |
| `~/.local/opt/zend.bak-<date>-<time>/` | the build before a switch |
| `~/.local/bin/zend` | the CLI's launcher |
| `~/.zend/` | data: chats, plugins, logs, docs. A switch never touches it |

## 1. Build and stage
On the build server, in the build container ([infra/dev](../../infra/dev/README.md)): release builds of the engine and the window (stable, thin LTO off), staged with `ui/` and `plugins-builtin/` in the checkout's `target/zend-next`. In the background; it takes minutes.

```sh
ssh AI 'cd ~/zend-build/<checkout> && nohup infra/dev/stage.sh > ~/zend-build/stage.log 2>&1 &'
ssh AI 'tail -1 ~/zend-build/stage.log'      # BUILD_OK <path> (<commit>)
```

Then bring it to the machine that runs Zend, as `~/.local/opt/zend-next`:

```sh
rm -rf ~/.local/opt/zend-next
ssh AI 'tar c -C ~/zend-build/<checkout>/target zend-next' | tar x -C ~/.local/opt
cat ~/.local/opt/zend-next/BUILT_FROM
```

The container is Ubuntu 24.04 (glibc 2.39); the binaries run on a host with that glibc or newer.

## 2. Check before installing
- What is the installed build made from? If it holds commits the new one lacks, installing takes them away. Compare first.
- Smoke the staged build as a stand-in ([test-instance.md](test-instance.md)), with the binaries from `zend-next`: the engine starts, `zend status` answers, the window loads offscreen.

## 3. Switch: the owner's step
The switch stops the engine, and with it every chat it hosts, including an agent's own. So an agent prepares it and the owner runs it from a plain terminal, not from a Zend chat.

```sh
#!/bin/sh
set -eu
OPT="$HOME/.local/opt"
[ -x "$OPT/zend-next/zend-zgx" ] || { echo "no new build in $OPT/zend-next"; exit 1; }
pkill -f "$OPT/zend/zend-zgx" 2>/dev/null || true
pkill -f "$OPT/zend/zend-engine" 2>/dev/null || true
sleep 3
pgrep -f "$OPT/zend/zend-" >/dev/null && { echo "zend still runs; close it and retry"; exit 1; }
B="$OPT/zend.bak-$(date +%Y%m%d-%H%M)"
mv "$OPT/zend" "$B"; mv "$OPT/zend-next" "$OPT/zend"
echo "installed. Old build kept in $B. Start zend."
```

Then start Zend. Chats continue where they were.

## 4. Verify live
In the new Zend: `whoami` answers, one changed feature works through its real entry point (for the plugin dev loop: `plugin_status` with `snapshot`, `check_plugin`, `plugin_logs`). Say what was seen and what was not tried.

## Going back
Close Zend, then:
```sh
mv ~/.local/opt/zend ~/.local/opt/zend.failed && mv ~/.local/opt/zend.bak-<date>-<time> ~/.local/opt/zend
```
Data written by the newer build stays in `~/.zend`. Whether an older build reads all of it is untested.

## Development mode with no edge
A development install (\`~/.config/zend/edge-token\`, a bearer \`u-<id>@<org>\`) turns the edge on whenever the token is non-empty, and shows "Reconnecting…" when that edge is gone. The token also names the profile (data folder \`~/.zend/orgs/<org>/u-<id>\`), so deleting it moves the data. To keep the identity and drop the edge, make \`edge-url\` say \`none\` (or set \`ZEND_EDGE_URL=none\`):

\`\`\`sh
echo none > ~/.config/zend/edge-url   # keep edge-token as it is; restart Zend (engine and window)
\`\`\`

The engine then opens no room, relay or pull/push, connectivity is \`Disabled\` (no pill) and the registry counts as synced, as for a local profile. Release checks still go to the default edge address. A token with no \`edge-url\` is not read at all; an edge that is set behaves as before. To go back, write the edge's URL into \`edge-url\`. Production (WorkOS) installs ignore both files.

## Not covered
- The AI VM's engine is built and installed separately (owner's home repo, `infra/ai-vm`) and was not updated with the 2026-11-15 build.
- macOS and Windows: untested for the fork.
- The app's self-update points at releases; whether it would replace a locally installed build is unchecked.
