# Zend on Android (sh.zend.zgx)

The desktop app (apps/zgx) as an Android APK: a NativeActivity loading
`libzend_android.so` (this crate), the engine on a thread inside the app
(src/android.rs). No Kotlin, no Gradle.

## Build

Needs ANDROID_HOME (build-tools 36.1.0, platforms;android-36),
ANDROID_NDK_HOME, JAVA_HOME, cargo-ndk and the rustup targets.

    ./build-apk.sh --abi x86_64 --release     # emulator; arm64-v8a for a phone
    adb install -r ~/.cache/android-app/apk/zend.apk
    adb shell am start -n sh.zend.zgx/android.app.NativeActivity

| Build | debuggable | use |
|---|---|---|
| `./build-apk.sh` (debug profile) | yes | rarely: the library is ~900 MB |
| `--release --debuggable` | yes | tests on an emulator: `run-as sh.zend.zgx` reads its files |
| `--release` | no | a phone with real tokens |

## Secrets

- A debuggable build lets anyone with adb read the app's files (`run-as`):
  the edge bearer (`files/.config/zend/edge-token`), the device identity and
  the plugin-signing key (`files/.zend/`). Give a debuggable build fake
  tokens only (a dev bearer like `u-test@invalid` against a test edge),
  never your real edge's.
- Backups and device transfers leave out `files/.zend/` and
  `files/.config/` (res/xml/data_extraction_rules.xml, backup_rules.xml):
  a restored or moved app starts with an identity of its own.
- The engine has no port and no token file: the app reaches it in-process
  over a socket pair (zend_engine::InProcessIpc), with a bearer kept in
  memory. Nothing else on the phone can dial it. Terminals are off on
  Android (their bridge is a process of its own).
- The edge config is written with `zend_cli::store_edge_config`: whole
  or not at all, mode 600.
