# majnet-reconciler

The orchestrator (design doc §12). Phase-2 MVP: full converge loop — snapshots from the bot, SOPS→tmpfs secrets, per-project networks, migrations, blue-green deploys, removed-app GC.

## Configuration (env)

| Variable | Required | Default | Purpose |
|---|---|---|---|
| `MAJNET_BOT_URL` | ✱ | — | bot's WG-internal API, e.g. `http://10.88.0.1:8081` |
| `MAJNET_LISTEN` | | `127.0.0.1:9090` | notify + state API — **bind to the WG IP** in production |
| `MAJNET_ROOT_ORG` | | `majksa-platform` | root platform org |
| `MAJNET_AGE_KEY_DIR` | | `/etc/majnet/age` | `age-stable.key`, `age-production.key` |
| `MAJNET_DOCKER_CERT_DIR` | | `/etc/majnet/pki` | `ca.pem`, `reconciler-cert.pem`, `reconciler-key.pem` |
| `MAJNET_DATA_DIR` | | `/var/lib/majnet-reconciler` | SQLite event log |
| `MAJNET_DB_ROOT_DIR` | | `/etc/majnet/db-root` | host dir holding each DB engine's root-secret file, bind-mounted into the engine container (the smoke test points it at a temp dir) |
| `MAJNET_POLL_INTERVAL_SECS` | | `300` | drift poll (§12.1) |
| `MAJNET_DRY_RUN` | | off | `1`/`true`: log planned actions, touch nothing |

Runtime dependencies on the main node: the `sops` binary (secret decryption shells out — ADR-worthy if it ever hurts) and network reachability of each node's Docker API over WireGuard.

## Endpoints

| | |
|---|---|
| `POST /notify` | bot's deploy nudge (payload informational — every cycle reconciles everything) |
| `GET /api/events?limit=N` | recent events `{at, commit, project, node, action, result}` |
| `POST /api/exec/{project}/{class}/{app}` | run one command in the app's container → `{exit_code, stdout, stderr, truncated}` (ADR 0029) |
| `GET /api/db/{project}/{class}/{app}` | which engine + database the app uses, and where it runs |
| `POST /api/sql/{project}/{class}/{app}` | run one statement against it; `?write=true` to allow writes, `?meta=tables\|columns` for the catalog |
| `GET /healthz` | liveness |

`exec` and `sql` are gated like `/api/logs` — production needs a project admin, other classes a developer, and a SQL write needs an admin in every class — and both write an audit event naming the caller. `sql` connects as the **app's own** derived DB role, never the superuser; without `write=true` the connection is read-only, which is a seatbelt against a mistyped `UPDATE` rather than a sandbox (see the module docs in `src/run.rs`).

## Invariants enforced here

- Re-validates every manifest defensively; failed validation/decrypt aborts that app **loudly**, never partially.
- Deletions only when config is gone from git (`deploy::gc_removed_apps`); a failed deploy keeps the app in the keep-list so the old container survives.
- Secrets: tmpfs only (`/run/majnet/secrets/...`), delivered via a short-lived helper container, mounted read-only — never env vars, never disk.
- Blue-green health gate: new container's HEALTHCHECK must pass before old containers stop; Traefik only routes healthy containers (ADR 0002).
- Private GHCR pulls need node-level pull auth (bootstrap concern, not reconciler credentials) — see roadmap open questions.
