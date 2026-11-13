# Zend

Controla tus agentes de programación (Claude Code, Codex, Cursor, Devin, Grok, Hermes, Pi, Antigravity) de forma local por defecto, con sincronización opcional entre varios dispositivos.

*[English](README.md) | [简体中文](README.zh-CN.md) | [한국어](README.ko.md) | [日本語](README.ja.md) | Español*

![App de escritorio de Zend](docs/media/readme/app-screenshot.jpg)

## App de escritorio

Descarga la última versión para tu plataforma desde [GitHub Releases](https://github.com/sandyhq/zend/releases/latest):

- **macOS** — `zend-<version>-macos-arm64.dmg`
- **Windows** — `zend-<version>-windows-x86_64-setup.exe`
- **Linux** — `zend-<version>-linux-<arch>.tar.gz` y luego ejecuta el `install.sh` que incluye

No necesitas cuenta ni conexión a internet; las sesiones se quedan en tu dispositivo. La app se actualiza sola.

## Modo headless (CLI)

Para servidores y otras máquinas sin pantalla, como un VPS que mantiene los agentes en ejecución después de cerrar tu portátil. Solo Linux:

```bash
curl -fsSL https://zend.invalid/install.sh | sh
zend status
```

El instalador arranca el motor como un servicio en segundo plano que sobrevive a los reinicios.

```bash
zend status      # modo local/sincronizado y estado del motor
zend update      # actualizar a la última versión
zend daemon start|stop|restart|status
```

## Sincronización entre dispositivos (opcional)

Inicia sesión para lanzar un agente en un dispositivo y seguirlo o controlarlo desde otro:

```bash
zend daemon stop
zend login        # o: zend logout para volver al modo solo local
zend daemon start
```

Los dispositivos con sesión iniciada en la misma cuenta pueden leer y escribir los archivos del espacio de trabajo de los demás, así que inicia sesión solo en dispositivos de confianza. Las sesiones locales existentes nunca se suben.

## Patrocinadores

Gracias a [The Context Company](https://www.thecontextcompany.com/) por patrocinar Zend. Tú también puedes ayudar a financiar el desarrollo de Zend [convirtiéndote en patrocinador en GitHub](https://github.com/sponsors/sandyhq).

---

¿Quieres contribuir o te interesa saber cómo funciona? [Pregúntale a DeepWiki](https://deepwiki.com/sandyhq/zend) o consulta [ARCHITECTURE.md](ARCHITECTURE.md).

Publicado bajo la [licencia MIT](LICENSE).
