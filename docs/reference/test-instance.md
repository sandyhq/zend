# A stand-in instance

How to run a second Zend beside the installed one, to try a change for real without touching the person's Zend, their window or their data. Worked out during the plugin dev loop (November 2026, Linux, Wayland); the same recipe ran for every live check since.

## What it is
An engine of its own (own data folder, own port) and, when you need the interface, a window of its own that is drawn but never shown. The installed Zend keeps running and is not asked anything.

## Build
From your worktree, debug builds are enough:

```sh
export CARGO_TARGET_DIR=$HOME/.cache/zend-target-<name>
cargo build -p zend --bin zend-engine --bin zend      # engine and CLI
(cd apps/zgx && cargo build --target-dir $HOME/.cache/zend-target-<name>-zgx)   # the window, zend-zgx
```

## Start
Pick a folder `R` outside the repo and a free port. Start with a clean environment (`env -i`) so nothing of the installed Zend leaks in.

```sh
R=$HOME/.cache/<name>/run; PORT=27941
T=$HOME/.cache/zend-target-<name>/debug
Z=$HOME/.cache/zend-target-<name>-zgx/debug/zend-zgx
BASE="PATH=$PATH XDG_RUNTIME_DIR=$XDG_RUNTIME_DIR WAYLAND_DISPLAY=$WAYLAND_DISPLAY LANG=en_US.UTF-8 USER=$USER LOGNAME=$USER SHELL=/bin/bash"
mkdir -p $R/home $R/apphome $R/engine

# engine
env -i $BASE HOME=$R/home ZEND_DATA_DIR=$R/engine ZEND_IPC_PORT=$PORT \
  setsid $T/zend-engine headless > $R/engine.log 2>&1 < /dev/null &
echo $! > $R/engine.pid

# window, drawn offscreen
env -i $BASE HOME=$R/apphome ZGX_CHECK=0 ZGX_OFFSCREEN=1 ZGX_POPUP_WINDOWS=0 \
  ZEND_ZGX_ENGINE_SPAWN=0 ZEND_ZGX_ENGINE_PORT=$PORT ZEND_ZGX_ENGINE_DATA=$R/engine ZEND_ZGX_PLUGIN_LOG=1 \
  setsid $Z main > $R/app.log 2>&1 < /dev/null &
echo $! > $R/app.pid

# CLI against it
env -i $BASE HOME=$R/home ZEND_DATA_DIR=$R/engine ZEND_IPC_PORT=$PORT $T/zend status
```

| Variable | What it does |
|---|---|
| `ZEND_DATA_DIR`, `ZEND_IPC_PORT` | the engine's data folder and port; the CLI and `zend mcp` find the engine by them |
| `ZGX_OFFSCREEN=1` | the window is drawn but never mapped: nothing appears on any screen and nothing takes focus |
| `ZGX_POPUP_WINDOWS=0` | no extra windows (consent sheets, popups) |
| `ZEND_ZGX_ENGINE_SPAWN=0`, `ZEND_ZGX_ENGINE_PORT`, `ZEND_ZGX_ENGINE_DATA` | the window uses your engine instead of starting one |
| `ZEND_ZGX_PLUGIN_LOG=1` | plugin prints also go to the window's stderr (`app.log`) |
| `ZEND_IDEAS_TEST_MIC=<file.wav>` | on the window: Ideas & Intents records from this WAV, played in as if spoken in real time, instead of the microphone (a stand-in has none). Only for testing; dictation in the composer is unaffected |
| `ZEND_PLUGIN_PREVIEW_APP`, `ZEND_PLUGIN_PREVIEW_HOME` | on the engine: which window binary and home a plugin preview uses (only for testing previews) |

Without `ZGX_OFFSCREEN=1` the window opens on the person's current workspace. That happened once (a consent window, about three seconds). Don't.

## Use the real MCP server
Check through the same door agents use, not through internal calls. `zend mcp` speaks JSON-RPC on stdio:

```sh
env -i PATH=$PATH HOME=$R/home ZEND_DATA_DIR=$R/engine ZEND_IPC_PORT=$PORT $T/zend mcp
```

Send `initialize`, then `notifications/initialized`, then `tools/call { name, arguments }`, one JSON object per line. Set `ZEND_CHAT_ID` to a chat's id to get the tools only a chat sees (`show_visual`, `show_image`).

## Look and act
- `plugin_status { id, snapshot: true }` returns a PNG path; with `page` it pictures one plugin page. Read the file.
- `plugin_input { id, preview: false, action: "click", at: "role:button \"Usage\"" }` acts on the stand-in's window. It is accepted there only because the window runs offscreen.
- Logs: `$R/engine.log`, `$R/app.log`, and `plugin_logs`.

## A chat without spending usage
Use a mock model: a Pi extension that registers a provider answering from a script (the session-context check used one named `zend-probe/mock`, which called `create_chats` when its system prompt held the plugin's section). Name the harness and model in every `create_chat` the mock makes: a request without a harness falls to the default, which is real Claude Code on the person's sign-in.

## Limits
- A plugin cannot be signed or allowed in a stand-in: allowing needs Zend's consent window (`zend plugin sign` answers "needs Zend's own consent window"). Plugins there run only if the data folder already holds the grant, as a plugin preview arranges for its copy.
- Type checks need luau-lsp; a clean environment has none on the PATH, so the first check fetches it or checks syntax only when offline.
- Proven with debug builds. An installed release window as the stand-in's window is untested.

## Stop
```sh
kill -TERM -$(cat $R/app.pid) -$(cat $R/engine.pid)    # process groups, hence the minus
```
