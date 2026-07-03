# GitHub outage

**Workloads are unaffected** — apps, DBs, ingresses and `edge-main` keep serving; nothing at runtime depends on GitHub (§17).

What stops: deploys, render PRs, digest bumps, org reconciliation, ephemeral previews, dashboard *writes*. Dashboard reads (events, health) keep working — they come from the reconciler.

During the outage:
- The reconciler's drift poll will log snapshot failures — noisy but harmless; it never deletes anything just because a snapshot failed (deletions require config *observed gone*, not *unavailable*).
- Do **not** hand-edit containers to "deploy anyway"; the reconciler reverts manual drift on the next successful cycle. If something must ship mid-outage, that's an incident decision: stop the reconciler first (`systemctl stop majnet-reconciler` on main), act, restart it after GitHub returns and git reflects reality.
- Restart escape hatch still works (it's git-free).

After recovery: nothing to do. Webhooks GitHub queued may replay (delivery dedup absorbs duplicates); the next poll/nudge converges any merged-during-outage state.
