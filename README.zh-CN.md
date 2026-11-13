# Zend

在本地管理你的编码 agent（Claude Code、Codex、Cursor、Devin、Grok、Hermes、Pi、Antigravity），也可以打开多设备同步。

*[English](README.md) | 简体中文 | [한국어](README.ko.md) | [日本語](README.ja.md) | [Español](README.es.md)*

![Zend 桌面应用](docs/media/readme/app-screenshot.jpg)

## 桌面应用

从 [GitHub Releases](https://github.com/sandyhq/zend/releases/latest) 下载对应平台的最新版本：

- **macOS** — `zend-<version>-macos-arm64.dmg`
- **Windows** — `zend-<version>-windows-x86_64-setup.exe`
- **Linux** — `zend-<version>-linux-<arch>.tar.gz`，解压后运行里面的 `install.sh`

不用账号，也不用联网，会话就存在这台设备上。应用会自动更新。

## 无界面运行（CLI）

适用于服务器等没有显示器的机器，比如在你合上笔记本之后继续跑 agent 的 VPS。仅支持 Linux：

```bash
curl -fsSL https://zend.invalid/install.sh | sh
zend status
```

安装脚本会把引擎作为后台服务拉起来，重启之后也会自己回来。

```bash
zend status      # 查看本地/同步模式和引擎状态
zend update      # 更新到最新版本
zend daemon start|stop|restart|status
```

## 多设备同步（可选）

登录后，可以在一台设备上起 agent，换另一台设备接着看、接着操作：

```bash
zend daemon stop
zend login        # 或者 zend logout 切回纯本地模式
zend daemon start
```

登录同一账号的设备可以读写彼此工作区里的文件，所以只登录你信任的设备。已有的本地会话不会被上传。

## 赞助

感谢 [The Context Company](https://www.thecontextcompany.com/) 对 Zend 的赞助。你也可以[通过 GitHub 成为赞助者](https://github.com/sponsors/sandyhq)，资助 Zend 的开发。

---

想参与开发，或者好奇它怎么跑起来的？[Ask DeepWiki](https://deepwiki.com/sandyhq/zend)，也可以看 [ARCHITECTURE.md](ARCHITECTURE.md)。

采用 [MIT License](LICENSE)。
