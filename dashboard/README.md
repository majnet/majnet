# Dashboard

Web UI on the main node, reachable via Tailscale (admin + per-project ACLs).

A static page (`index.html`) + nginx proxy (`compose.yaml`, `nginx.conf`): event log, manifest editing, member management, TTL extension, promote/rollback (→ bot → commits) and restart (→ reconciler, the imperative exception), role-gated via `people.yaml`.

**Deploying:** on the main node, `tailscale up` (interactive login — the one manual step), then `bootstrap.sh 70` (`steps/70-dashboard.sh`, also suggested by `install.sh`): installs Tailscale + the compose plugin, runs `docker compose up -d`, and fronts it with `tailscale serve --bg --http 80 http://127.0.0.1:8090`. Manual equivalent: those last two commands.

- **Reads** come from the reconciler's state API: per-project deploys, env inventory, health, events, diffs.
- **Writes go through git, never around it** — every mutating action is sent to the bot's write API, which turns it into a validated commit or PR on the relevant `ops` repo with the acting user attributed (`Co-authored-by`). The dashboard holds no GitHub credentials.
- **One imperative escape hatch:** restart / redeploy-same-digest, via a narrow audit-logged reconciler endpoint.

Authorization: Tailscale identity headers → `people.yaml` → project role (`admin` | `developer`). Production actions are additionally protected downstream by branch protection on `env/production` — a compromised dashboard still can't skip the review.

See design doc §16 for the full UI-action → git-effect table.
