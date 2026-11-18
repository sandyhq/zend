# This fork and upstream

Written 2026-11-15 from the state of the repository. Lines marked "not decided" are open.

## What it is
`github.com/sandyhq/zend` is a fork of the Zeron app, renamed throughout to Zend by `scripts/zend-rebrand.pl` (commit 19527cf4: names, paths, environment variables, the SDK; domains to `zend.invalid`; GitHub links to `sandyhq`). Upstream's own history is in `main` up to its commits of 2026-11-10 (the pull requests numbered 12xx and 13xx). There is one remote, `origin`; no upstream remote is configured.

The fork is where the owner's changes to Zend's core are made. They are not sent upstream.

## Branches
| Branch | What |
|---|---|
| `main` | the fork's trunk, pushed. Holds child sessions (PR #1), the plugin dev loop and plugin instructions |
| `live-visuals` | integration branch for the plan of that name; groups are built on `live-visuals-G<n>` and merged into it |
| `zend/zengx-dep` | one commit on top of an older `main` (20e2bb51), the first try at building `apps/zgx` against `sandyhq/zengx`. Superseded by `zengx-switch` (2026-11-17), which took its CI and docs changes; safe to delete |
| `zend/rebrand`, `zend/child-sessions` | merged into `main`; kept on origin |
| `<plan>-G<n>` | one group of a plan, local only, each in a worktree `../zend-<plan>-G<n>`. Safe to delete once merged |

## How work lands
1. A plan (proposal, design, tasks) is agreed with the owner. Plans live in the owner's home repo, not here.
2. Each group is built in a worktree on its own branch, checked, and reported.
3. Groups are merged into the plan's integration branch, built and installed locally ([reference/build-and-deploy.md](reference/build-and-deploy.md)), and tried by the owner.
4. The integration branch is merged into `main` (fast-forward where possible) and pushed, on the owner's word.

## Taking upstream changes
Decided 2026-11-15: upstream changes are taken only when the owner says so, and then by cherry-picking the chosen commits. No merge of upstream's branch, no regular sync.

How, as far as known:
- No upstream remote is configured. Add one for the pick and fetch it.
- Upstream's commits use the old names. Rebrand them as they come in:
  - One commit made after the fork's merge-base with upstream: `git format-patch -1 <sha> --stdout | scripts/zend-rebrand.pl --filter | git am -3`.
  - A commit or a range that upstream branched before the merge-base: `git am -3` fails outright, because the rebranded patch names blobs the fork never had. Use `scripts/zend-pick-range.sh <base> <target> [paths]` instead. It merges the range's net change file by file, 3-way, with the range's start and end rebranded and the fork's file as the other side. Then resolve the conflicts it names and commit once, naming the upstream commits. Used for upstream's composer refactor (cd769402..7a188dfe); two later fixes of it were then picked one by one: 6df5169d (a sent pick cleared by its fields) and 82d410d7 (the series alone took `split_tests::a_dragged_sidebar_edge_builds_no_list` over its limit).
  - Upstream's pins of zgx (`Cargo.toml`, `Cargo.lock` and `git = …zgx` revs) are dropped: the fork pins its own zgx (below). A template's `.base.json` is re-blessed, not merged.
- The fork's changes sit mostly in `crates/engine/src/plugins`, `crates/mcp`, `crates/harness`, `apps/zgx/src/plugins` and `apps/zgx/ui`. Picks touching those need the most care.
- `ARCHITECTURE.md` is upstream's and partly out of date for this fork; see [architecture-fork.md](architecture-fork.md).

## zgx
Decided 2026-11-17: `apps/zgx` builds against [sandyhq/zengx](https://github.com/sandyhq/zengx), pinned at one commit (every `rev` in `apps/zgx/Cargo.toml`). zengx's `main` is zeronsh/zgx's `main` with the fork's zgx edits as commits on top; its `FORK.md` lists them. Until then zgx was vendored in the repository.

- **Taking upstream zgx:** the whole of zeronsh/zgx's `main`, merged into zengx's `main` (no rebase, no force-push: this repository pins its commits), tested against plain upstream and pushed as a fast-forward. Then the pin here moves. Both steps: zengx's `FORK.md` ("Taking upstream") and `apps/zgx/README.md` ("zgx").
- **A zgx edit the fork needs** is a commit on zengx's `main` with its row in zengx's `FORK.md`, then a pin move here.
- **Zend's rebrand stays out of zgx.** zgx's examples, tests and comments keep upstream's Zeron names: Zend builds none of them, and carrying the rename would conflict with every upstream merge.
- **The engine's copy of zgx's types** (`crates/engine/src/plugins/zgx.d.luau`, for `zend plugin check`) moves with the pin; `the_engine_s_zgx_types_are_the_window_s` in apps/zgx fails until it is recorded again.
