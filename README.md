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

## Sponsors

Thank you to [The Context Company](https://www.thecontextcompany.com/) for sponsoring Zend. You can help fund Zend's development too by [becoming a sponsor on GitHub](https://github.com/sponsors/sandyhq).

---

Developing or curious how it works? [Ask DeepWiki](https://deepwiki.com/sandyhq/zend) or check out [ARCHITECTURE.md](ARCHITECTURE.md).

Licensed under the [MIT License](LICENSE).
