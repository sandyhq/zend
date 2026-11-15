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
Decided 2026-11-15: upstream changes are taken only when the owner says so, and then by cherry-picking the chosen commits. No merge of upstream's branch, no regular sync.

How, as far as known:
- No upstream remote is configured. Add one for the pick and fetch it.
- Upstream's commits use the old names. A picked commit conflicts wherever it touches a renamed name; run `scripts/zend-rebrand.pl` over the result, or resolve by hand, before committing. Not tried yet since the rebrand.
- The fork's changes sit mostly in `crates/engine/src/plugins`, `crates/mcp`, `crates/harness`, `apps/zgx/src/plugins`, `apps/zgx/ui` and `third_party/zgx`. Picks touching those need the most care.
- `ARCHITECTURE.md` is upstream's and partly out of date for this fork; see [architecture-fork.md](architecture-fork.md).
