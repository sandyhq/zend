# third_party/zgx: what the fork changes

A vendored copy of [zeronsh/zgx](https://github.com/zeronsh/zgx), built by path from `apps/zgx`.

- **Upstream commit:** `1accd1b75` (zgx main, "A list scrolled by the view around it places its rows only as its window moves"). Before: `0ca78b2cb` (branch `zeron/pin-0.3.3`).
- **Vendored set:** `crates/`, `vendor/`, the root `Cargo.toml`, `Cargo.lock`, `LICENSE` and `README.md`, and `docs/`, `scripts/` and `comparisons/` without new subfolders, plus `scripts/perf/fingerprint` (a dev-dependency of six crates; without it the vendored workspace doesn't load). Left out: dotfiles, `AGENTS.md`, `CHANGELOG.md`, `CLAUDE.md`, `changelog.d/`, `fuzz/`, `site/`, `website/` and `docs/results/`.

## The fork's edits

Each one is a commit on top of upstream, replayed at every update.

| Edit | Files | Why |
|---|---|---|
| Workspace members as vendored: no `comparisons/*`, no `scripts/perf/fingerprint` | `Cargo.toml` | those aren't vendored |
| `workspace = "../.."` in every crate's manifest | `crates/*/Cargo.toml` | cargo finds no workspace root for a path dependency otherwise |
| Rebrand (`scripts/zend-rebrand.pl`): comments, example titles, `ZEND_SEND`/`ZEND_MODEL` test hooks | ~55 files | the fork's naming |
| `ScriptHost::on_print` | `crates/zgx-script/src/lib.rs`, `node.rs` | plugin logs: a plugin's prints and errors reach the engine |
| `Ui::on_draw` / `Ui::draw`; `draw_alone` and `draw_context` in `application.rs` | `crates/zgx/src/widgets.rs`, `crates/zgx-desktop/src/application.rs` | plugin_status snapshots and page pictures |
| A window drawn on the software raster pictures nodes on a headless GPU context of its own | `crates/zgx-desktop/src/application.rs` | snapshots where the only adapter is a software one (`ZGX_RENDERER=auto` picks the raster there) |
| `ZGX_OFFSCREEN` / `application::offscreen()`: laid out and drawn, never shown | `crates/zgx-desktop/src/application.rs` | plugin previews and stand-in instances |

None of these is upstream as of `1accd1b75`.

## Updating

1. In a clone of zeronsh/zgx, check out the vendored commit. Add one commit that copies the vendored root `Cargo.toml`, then the fork's commits to `third_party/zgx` since the last update:
   `git -C <fork> format-patch --relative=third_party/zgx <range> -- third_party/zgx | git am`.
   Check: the tree matches `third_party/zgx` file for file.
2. `git rebase <new upstream>`. Resolve, and add `workspace = "../.."` to crates that are new.
3. Copy `crates/` and `vendor/` whole into `third_party/zgx`, plus the vendored set above. Run `scripts/zend-rebrand.pl --filter` over the files that say "zeron" (it keeps zgx's own URLs).
4. Run `cargo check --all-targets` in `apps/zgx` (it updates `apps/zgx/Cargo.lock`), then the checks in `docs/reference/testing.md`.
5. Land it as one commit that names both upstream commits, with this file updated.
