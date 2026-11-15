# Roadmap

State of the work in this fork. The plans themselves (proposal, design, tasks, reports with check output) live outside this repo, in the owner's home repo under `plans/<id>/`; the id is given with each entry. Keep this file to one line per item and move items between sections as they change.

Updated: 2026-11-15

## Done
- Child sessions and child events (plan `child-sessions`, PR #1).
- Plugin dev loop: logs, check, pictures without a shown window, preview apart, preview in the window, `zend.run` folder (plan `plugin-dev-loop`, G1-G6).
- Instructions from plugins, Pi only (plan `session-context`, G1-G2).

## In progress
- Live visuals (plan `live-visuals`, branch `live-visuals`). First milestone: input nodes in visuals with entries kept by Zend, an answer button that appends the entries to the composer, `update_visual` and `read_visual`, open visuals pinned above the composer, and the input kinds of the component kit (`ui.kit`).

## Planned
- Live visuals, rest: example collection and gallery, the tool `visual_examples`, the remaining kit kinds (the catalogue of ComicBit/bb-plugin-genui), agent instructions for visuals.
- Instructions from plugins for Claude Code, Codex and ACP harnesses.

## Open points on what is done
- Preview in the window: the Keep/Revert bar was proven in a test instance only; the owner has not seen it in his window. An error after loading (not while loading) is untested there.
- The whole test suite was run per branch, not on the merged `main` (fc28f468). Baseline on the dev machine: about 11 failing tests in `apps/zgx`.
- Signing or allowing a plugin needs Zend's consent window, so a headless test instance cannot allow one.
- Not decided: Zend signing in to Claude Code again by itself when its token has expired (usage readings stop until then).
