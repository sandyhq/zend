# Packaging

## Linux (implemented)

```sh
scripts/package-linux.sh            # release build (thin LTO, stripped)
PROFILE=debug scripts/package-linux.sh   # fast smoke package
```

Produces `target/package/zend-<version>-linux-<arch>.tar.gz` containing:

- `zend` — the binary (headed by default; `zend headless` runs the engine alone)
- `zend.desktop` — XDG desktop entry template (`Exec=zend` for packagers;
  the installers rewrite `Exec`, `TryExec`, and `Icon` to absolute paths under
  `~/.zend/app/current`, since `~/.local/bin` is often not on a desktop
  session's `PATH`)
- `zend.png` — 1024×1024 Zend app icon (`scripts/make-app-icons.py`)
- `install.sh` — installs into `~/.zend/app/<version>` behind a `current`
  symlink (the curl installer's layout, which the in-app updater manages),
  links `~/.local/bin/zend` to it, and writes the desktop entry and icon under
  `$XDG_DATA_HOME` (default `~/.local/share`). The curl installer does the same
  from the extracted tarball; `scripts/test-linux-desktop-entry.sh` checks both

The release profile in the root `Cargo.toml` sets `lto = "thin"` and
`strip = "symbols"` for distribution builds.

## macOS

```sh
scripts/package-macos.sh    # → target/package/zend-<version>-macos-<arch>.dmg
```

Builds the release binary, assembles `Zend.app` (Info.plist + icns), ad-hoc
signs it (set `CODESIGN_IDENTITY` for a real Developer ID), and wraps it in a
dmg. The auto-update tarball retains an internal `Zend.app` path so older
installed builds can update into Zend. CI runs this on tags
(`.github/workflows/release.yml`). The manual steps it automates, for reference
(run on a macOS host — gpui needs Metal; no cross-build from Linux):

1. Build the universal (or per-arch) binary:
   ```sh
   cargo build --release -p zend --target aarch64-apple-darwin
   cargo build --release -p zend --target x86_64-apple-darwin
   lipo -create -output zend \
     target/aarch64-apple-darwin/release/zend \
     target/x86_64-apple-darwin/release/zend
   ```
2. Assemble the bundle:
   ```sh
   mkdir -p Zend.app/Contents/{MacOS,Resources}
   cp zend Zend.app/Contents/MacOS/zend
   sed "s/__VERSION__/$(grep -m1 '^version' Cargo.toml | sed 's/.*"\(.*\)".*/\1/')/" \
     dist/macos/Info.plist > Zend.app/Contents/Info.plist
   ```
3. Icon: `scripts/macos-app-icon.sh Zend.app/Contents/Resources` writes
   `Assets.car` from `dist/macos/Zend.icon` (light, dark, clear and tinted, on
   macOS 26 and later; compiling it takes Xcode 26 or later) and `zend.icns`
   from `dist/macos/icon-1024.png` (earlier macOS and the dmg). All the app's
   icons are made from the art in `dist/icon` by `scripts/make-app-icons.py`.
4. Sign + notarize (required for distribution):
   ```sh
   codesign --deep --force --options runtime --sign "Developer ID Application: …" Zend.app
   xcrun notarytool submit Zend.zip --keychain-profile … --wait
   xcrun stapler staple Zend.app
   ```
5. Ship as a `.dmg` (`hdiutil create -volname Zend -srcfolder Zend.app -ov -format UDZO Zend.dmg`).

## Windows

```powershell
./scripts/package-windows.ps1 -ReleasesUrl https://github.com/sandyhq/zend/releases/latest/download
```

Produces, under `target/package/`:

- `zend-<version>-windows-<arch>-setup.exe` — the per-user installer built
  from `dist/windows/zend.iss` with Inno Setup 6
- `zend-<version>-windows-<arch>.zip` — the portable package
- `zend-<version>-windows-<arch>.exe` — the bare executable the in-app
  updater downloads

The installer and the zip both carry `zend-update.json`, the marker that lets
the app update itself in place. CI runs `scripts/test-windows-installer.ps1`
against the setup on every Windows build.
