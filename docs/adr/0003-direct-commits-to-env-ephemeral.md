# 0003 — Direct commits to `env/ephemeral` (no render PRs)

**Status:** accepted · **Date:** 2026-07-03

## Context

The design (§9, §13) routes every env-branch change through a render PR,
auto-merged for `stable`/`ephemeral`. For ephemeral that means one
open-PR/force-push/merge API dance per PR event (open, every push, close) —
several GitHub round-trips to produce exactly the same commit the branch
would get anyway, with no human ever in the loop.

## Decision

The bot commits **directly** to `env/ephemeral` (incremental trees: add or
delete just `<app>-pr<N>.yaml` + its secrets file). `stable` keeps its
auto-merged render PRs (they batch whole-tree renders from `main` and keep
the PR timeline as deploy history); `production` keeps its **gated** render
PR — the review gate is untouched.

What the render PR provided is preserved:
- *deploy trigger*: a push to `env/ephemeral` triggers the reconciler the
  same way a render-PR merge does
- *audit trail*: each commit says which app/PR/digest changed and when
- *generated-only manifests*: still true — nothing is hand-written (§8)

## Consequences

- Fewer moving parts and API calls in the hottest path (every PR push).
- `env/ephemeral` history is commits-only; there is no PR timeline for
  previews (acceptable: preview state is also visible on the app PR itself
  via the bot's comment).
- If branch-protection-style controls are ever wanted on ephemeral, revisit.
