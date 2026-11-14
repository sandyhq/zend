# Building and testing fast

What makes the Rust build and test loop slow in this repository, what we
measured, and the rules that follow from it. The numbers come from one Linux
machine (Ryzen 7 7800X3D, 16 threads, 30 GB), Rust 1.100.0, November 2026.

## Rules

1. **One integration test binary per crate.** Integration tests live in
   `tests/it/`: `tests/it/main.rs` declares `mod foo;` for each suite in
   `tests/it/foo.rs`. Never add a top-level `tests/foo.rs`: Cargo builds and
   links every top-level file as its own executable against the whole crate.
   Shared helpers (`common/`, `support/`) are declared once in `main.rs` and
   used as `use crate::common;`. Only a suite that needs a process of its own
   stays a top-level `tests/<name>.rs`, and says why in its first lines:
   process-wide state that plain `cargo test` would share between suites (a
   `#[global_allocator]` in `crates/doc/tests/snapshot_memory.rs`, the restart
   hook, the Cloud session, the diagnostic watchdog). nextest gives every test
   its own process anyway; the exception keeps `cargo test` correct too.
   A suite that only runs on Unix starts with `#![cfg(unix)]`: Windows CI
   compiles the whole `it` binary.
2. **Run tests with nextest:** `cargo nextest run`, plus `cargo test --doc`
   for doctests (nextest skips them). Config is in
   [`.config/nextest.toml`](../../.config/nextest.toml), which nextest reads by
   default; CI uses the same file.
3. **Select tests by module, not by binary.** A suite is a module of the `it`
   binary, so `--test foo` is now `--test it foo::`, and nextest filters are
   `binary_id(zend-engine::it) & test(/^foo::/)`.
4. **A test that re-runs itself** (`std::env::current_exe()` with `--exact`)
   passes its full path, `foo::child_name`, or it matches nothing.
5. **While editing, scope to the crate:** `cargo nextest run -p <crate>`. Run
   the whole workspace once before calling work done.

## Measurements

| | before | after |
|---|---|---|
| zend-engine test rebuild after a one-line edit in the crate (3 runs) | 65 / 25 / 24 s (65 binaries) | 8 / 8 / 8 s (1 binary) |
| `cargo nextest run -p zend-engine` (same 1604 tests) | 158 s | 102 s |
| whole workspace, `cargo test` vs `cargo nextest run` | 730 s | 199 s |

- Total test time across the workspace is about 2980 s; 75 tests over 5 s each
  make up 866 s of it (the slowest take 13-52 s, mostly timeouts and sleeps).
  Only parallelism helps those, which is what nextest gives; a compile cache
  does not.
- **Linkers made no real difference.** Rebuild after an edit, three runs:
  default (lld) 67/26/27 s, explicit lld 66/25/25 s, mold 65/22/22 s,
  Wild 0.11 61/21/21 s. Not worth a custom `RUSTFLAGS`, which also misses any
  shared compile cache once.
- **Debug info is already minimal:** `[profile.dev]` uses
  `debug = "line-tables-only"` and dependencies build at `opt-level = 2`
  without debug info. Re-measuring it showed no change.
- The parallel rustc frontend and Cranelift are nightly-only, so they are not
  used.

## Optional local tools

- **[kache](https://github.com/kunobi-ninja/kache):** a compile cache used as
  `rustc-wrapper` (`kache init`). It helps clean builds and switching between
  worktrees, not the rebuild after an edit. Use one wrapper only (not kache
  and sccache together).
- **[blaze](https://github.com/indexable-inc/blaze):** `blaze check -p <crate>`
  took 1.2 s after an edit where `cargo check` took 46 s (zend-engine). Run it
  with `BLAZE_PROC_MACRO_WASM=0` (some proc macros fail in its wasm sandbox).
  It uses its own toolchain, so `cargo check` stays the final word. `blaze
  build` and `blaze test` fail on this workspace (the zend-mobile staticlib
  and a `macro_rules_attribute` proc macro).

## Writing tests that find real problems

- Prefer a few tests at a real boundary (a running engine, the RPC surface, a
  fixture harness) over many unit tests that restate the implementation.
- Don't wait on wall-clock sleeps: wait for the event, with a timeout only as
  a safety net. Most of the 866 s of slow tests is sleeping.
- Tests run in parallel processes under nextest. A test that binds a fixed
  port or mutates shared state needs a test group in `.config/nextest.toml`
  (see `preview-discovery`); `std::env::set_var` is only safe because each
  test has its own process, and is not safe under plain `cargo test`.
- To find tests that don't catch anything, run
  [cargo-mutants](https://mutants.rs) on one crate: a surviving mutant is
  code no test checks. Property tests (`proptest`) and snapshot tests
  (`insta`) cover more input per line than hand-written cases.

## Known failures on `main`

About 10 tests fail on this machine on unmodified `main` under both runners,
e.g. `terminals::link::tests::heavy_output_and_typing_never_desynchronize_the_stream`
(it waits up to 60 s for a flood that never finishes). Compare a change
against that baseline.
