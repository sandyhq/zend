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
Release builds of the engine and the window, with the flags the VM build uses (`infra/ai-vm/build/build-zend.sh` in the owner's home repo). In the background; it takes minutes.

```sh
#!/bin/sh
set -e
SRC=$HOME/Projects/zend-integration          # the worktree to build
T=$HOME/.cache/zend-target-deploy
export CARGO_PROFILE_RELEASE_LTO=off LUAU_CXXFLAGS=-DNDEBUG
cd $SRC
cargo build --release -p zend --bin zend-engine --target-dir $T/engine
cd apps/zgx && cargo build --release --locked --target-dir $T/zgx
S=$HOME/.local/opt/zend-next; rm -rf $S; mkdir -p $S
cp $T/zgx/release/zend-zgx $T/engine/release/zend-engine $S/
cp -r ui $S/ui; rm -rf $S/ui/.zgx $S/ui/tests
cp -r plugins-builtin $S/plugins-builtin; rm -rf $S/plugins-builtin/*/.zgx
for d in $S/plugins-builtin/*/; do cp plugins-sdk/zend.luau $d/zend.luau; done
echo BUILD_OK
```

Run it as `nohup sh build.sh > build.log 2>&1 &` and wait for `BUILD_OK`.

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

## Not covered
- The AI VM's engine is built and installed separately (owner's home repo, `infra/ai-vm`) and was not updated with the 2026-11-15 build.
- macOS and Windows: untested for the fork.
- The app's self-update points at releases; whether it would replace a locally installed build is unchecked.
