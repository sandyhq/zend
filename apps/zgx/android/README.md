# Zend on Android (sh.zend.zgx)

The desktop app (apps/zgx) as an Android APK: a NativeActivity loading
`libzend_android.so` (this crate), the engine on a thread inside the app
(src/android.rs). No Kotlin, no Gradle.

The activity is `sh.zend.zgx.ZendActivity` (java/, compiled into
classes.dex by build-apk.sh with javac and d8): a NativeActivity with what
it lacks for a native feel, all driven from zgx-desktop's `android_ime.rs`:

- the keyboard's text connection (an `InputConnection` over a mirror of
  the focused editor): composing text, suggestions, voice typing, emoji and
  umlauts reach the editor as TSF's do on Windows (`text_store.rs`); a text
  area's Enter is a line break, a field's Enter submits;
- the text selection's handles and floating toolbar (`SelectionUi`: Cut,
  Copy, Paste, Select all);
- the long-press timeout (`ViewConfiguration`) and its haptic tick: a
  row, a tab or anything draggable drags only once held; a swipe scrolls.

## Build

Needs ANDROID_HOME (build-tools 36.1.0, platforms;android-36),
ANDROID_NDK_HOME, JAVA_HOME, cargo-ndk and the rustup targets.

    ./build-apk.sh --abi x86_64 --release     # emulator; arm64-v8a for a phone
    adb install -r ~/.cache/android-app/apk/zend.apk
    adb shell am start -n sh.zend.zgx/.ZendActivity

| Build | debuggable | use |
|---|---|---|
| `./build-apk.sh` (debug profile) | yes | rarely: the library is ~900 MB |
| `--release --debuggable` | yes | tests on an emulator: `run-as sh.zend.zgx` reads its files |
| `--release` | no | a phone with real tokens |

### End-to-end tests

    e2e/run.sh --apk ~/.cache/android-app/apk-x86/zend.apk

Boots the AVD `zgxpoc` headless (unless an emulator is up), installs a
test keyboard (e2e/ime: commits text as a real keyboard does, over the
InputConnection) and the APK (an x86_64 `--release --debuggable` build),
runs a stand-in engine with a mock model, seeds chats, links the app to it
and drives it with adb and the app's devtools: umlauts typed and sent,
Enter as a line break, the keyboard on scroll, in the file viewer and on
Jump to latest, the selection's handles and Copy, held drag versus swipe.
Exits non-zero on a failure; screenshots in ~/.cache/android-app/e2e/out.
About 2½ minutes.

### Thread-locals

On Android std has no ELF TLS: every std `thread_local!` static takes a
pthread key of its own, and bionic aborts past 128 per process ("out of
TLS keys"; the platform's libraries use some too). Write
`zgx::tls::thread_local!` in the app: on Android all of those share one
key. build-apk.sh runs `count-tls-keys.sh` on the built library and fails
above 64 (`TLS_KEY_BUDGET`); `tests/it/android_tls_keys.rs` catches a
std one in the app's sources without an Android build.

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
- A computer's direct code (v=2, its phone link over the tailnet) is kept
  in `files/.config/zend/desktop` with `zend_cli::store_desktop_record` (whole or not at all, mode 600): host,
  port and the link's key. With it kept the app starts no engine and dials
  that computer's (src/engine/rpc.rs); the key opens only the link's
  window-scoped methods, and New key on the computer revokes it.
