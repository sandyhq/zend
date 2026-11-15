# Zend

Control your coding agents (Claude Code, Codex, Cursor, Devin, Grok, Hermes, Pi, Antigravity) locally by default, with optional multi-device sync.

*English | [简体中文](README.zh-CN.md) | [한국어](README.ko.md) | [日本語](README.ja.md) | [Español](README.es.md)*

![Zend desktop app](docs/media/readme/app-screenshot.jpg)

## Desktop app

Download the latest release for your platform from [GitHub Releases](https://github.com/sandyhq/zend/releases/latest):

- **macOS** — `zend-<version>-macos-arm64.dmg`
- **Windows** — `zend-<version>-windows-x86_64-setup.exe`
- **Linux** — `zend-<version>-linux-<arch>.tar.gz`, then run its `install.sh`

No account or network connection is needed; sessions stay on your device. The app updates itself.

## Headless (CLI)

For servers and other machines without a display, such as a VPS that keeps agents running after you close your laptop. Linux only:

```bash
curl -fsSL https://zend.invalid/install.sh | sh
zend status
```

The installer starts the engine as a background service that survives reboots.

```bash
zend status      # local/synced mode and engine status
zend update      # update to the latest release
zend daemon start|stop|restart|status
```

## Multi-device sync (optional)

Sign in to start an agent on one device and follow or drive it from another:

```bash
zend daemon stop
zend login        # or: zend logout to return to local-only
zend daemon start
```

Devices signed in to the same account can read and write each other's workspace files, so only sign in devices you trust. Existing local sessions are never uploaded.

## Documentation

- **Using it:** [working with agents](docs/working-with-agents.md), [using plugins](docs/using-plugins.md), [when something is wrong](docs/troubleshooting.md).
- **What this fork adds:** [features](docs/features.md), [roadmap](docs/roadmap.md), [changelog](CHANGELOG.md).
- **Building on it:** [plugin guide](apps/zgx/docs/plugins.md), [visuals](docs/visuals.md), [MCP tools](docs/mcp.md).
- **Changing Zend itself:** [AGENTS.md](AGENTS.md), [architecture of the fork](docs/architecture-fork.md), [fork and upstream](docs/fork.md), [testing](docs/reference/testing.md), [stand-in instance](docs/reference/test-instance.md), [build and install](docs/reference/build-and-deploy.md), [decisions](docs/adr/), [words](CONTEXT.md).

## Sponsors

Thank you to [The Context Company](https://www.thecontextcompany.com/) for sponsoring Zend. You can help fund Zend's development too by [becoming a sponsor on GitHub](https://github.com/sponsors/sandyhq).

---

Developing or curious how it works? [Ask DeepWiki](https://deepwiki.com/sandyhq/zend) or check out [ARCHITECTURE.md](ARCHITECTURE.md).

Licensed under the [MIT License](LICENSE).
