# Dashboard

Phase 5 — web UI on the main node, reachable via Tailscale (admin + per-project ACLs).

- **Reads** come from the reconciler's state API: per-project deploys, env inventory, health, events, diffs.
- **Writes go through git, never around it** — every mutating action is sent to the bot's write API, which turns it into a validated commit or PR on the relevant `ops` repo with the acting user attributed (`Co-authored-by`). The dashboard holds no GitHub credentials.
- **One imperative escape hatch:** restart / redeploy-same-digest, via a narrow audit-logged reconciler endpoint.

Authorization: Tailscale identity headers → `people.yaml` → project role (`admin` | `developer`). Production actions are additionally protected downstream by branch protection on `env/production` — a compromised dashboard still can't skip the review.

See design doc §16 for the full UI-action → git-effect table.
