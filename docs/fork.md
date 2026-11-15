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
| `zend/zengx-dep` | one commit on top of an older `main` (20e2bb51): `apps/zgx` builds against `sandyhq/zengx` instead of the vendored copy in `third_party/zgx`. Not merged. Not decided: whether it goes in |
| `zend/rebrand`, `zend/child-sessions` | merged into `main`; kept on origin |
| `<plan>-G<n>` | one group of a plan, local only, each in a worktree `../zend-<plan>-G<n>`. Safe to delete once merged |

## How work lands
1. A plan (proposal, design, tasks) is agreed with the owner. Plans live in the owner's home repo, not here.
2. Each group is built in a worktree on its own branch, checked, and reported.
3. Groups are merged into the plan's integration branch, built and installed locally ([reference/build-and-deploy.md](reference/build-and-deploy.md)), and tried by the owner.
4. The integration branch is merged into `main` (fast-forward where possible) and pushed, on the owner's word.

## Taking upstream changes
Not decided, and not done since the rebrand. What is known:
- A plain merge from upstream would conflict on every renamed name. The rebrand script would have to be run over upstream's tree first, then merged.
- The fork's changes sit mostly in `crates/engine/src/plugins`, `crates/mcp`, `crates/harness`, `apps/zgx/src/plugins`, `apps/zgx/ui` and `third_party/zgx`. Small, bounded commits there keep a later merge possible.
- `ARCHITECTURE.md` is upstream's and partly out of date for this fork; see [architecture-fork.md](architecture-fork.md).
