# Roadmap

Phased plan from the design doc (§19), tracked here as the implementation progresses.

## Phase 0 — Foundations 🚧 (current)

Tooling ✅ / infra provisioning ⏳:

- [x] Node bootstrap tooling: WireGuard mesh, Docker APIs bound to WG IPs + mTLS, node roles, PKI (`bootstrap/`)
- [x] Firewall tooling: nftables per role, prod 80/443 from Cloudflare ranges w/ weekly refresh (`bootstrap/steps/40`)
- [x] `edge-main` Traefik + hello-world manifests (`platform-seed/platform/`)
- [x] Platform repo seed: nodes.yaml, people.yaml, projects.yaml, ACL template (`platform-seed/`)
- [ ] Provision the 3 Debian nodes + run bootstrap (needs servers, WG pubkey exchange, Docker PKI distribution)
- [ ] Tailscale org + paste rendered base ACL
- [ ] Create root org `majksa-platform` on GitHub + push `platform-seed/` as the `platform` repo
- [ ] Cloudflare: origin cert on prod node, proxied DNS record → hello-world reachable publicly

## Phase 1 — Bot MVP 🚧

Code ✅ / live wiring ⏳:

- [x] GitHub App: JWT auth, per-org installation token cache (`bot/src/github.rs`)
- [x] Webhook server: HMAC verification, delivery dedup, event dispatch (`bot/src/webhooks.rs`)
- [x] Digest bumps: GHCR `registry_package` event → App-signed commit to `apps/<app>/stable.yaml` on ops `main` (ADR 0001, `bot/src/digest.rs`)
- [x] Repo access proxy: `GET /api/snapshot/{org}/{repo}/{branch}` — SHA-cached tarballs on the WG-internal listener (`bot/src/proxy.rs`)
- [x] Reconciler notify on `env/*` + platform pushes (best-effort; drift poll backs it up)
- [x] GHA workflow templates: `rust-service`, `web-app` (test → GHCR by digest)
- [ ] Register the GitHub App (key, webhook secret, events per `crates/bot/README.md`) and deploy the bot to the main node
- [ ] Verify the `registry_package` payload digest path against a real delivery (ADR 0001 caveat)

## Phase 2 — Reconciler MVP 🚧

Code ✅ / live verification ⏳:

- [x] Manifest schema v1 + strict validation + base ⊕ overlay merge (`common/src/{manifest,merge}.rs`)
- [x] Rendering: ops `main` push → full-tree render PRs onto `env/*`; stable auto-merges, production waits for admin review (`bot/src/render.rs`)
- [x] Convergence loop: platform + env snapshots → per-project networks → validate → decrypt → diff → deploy; ~5 min drift poll + `/notify` nudge (`reconciler/src/{converge,main}.rs`)
- [x] Blue-green: migrations → health-gated rollout, old container survives failed deploys (ADR 0002, `reconciler/src/deploy.rs`)
- [x] SOPS decrypt (sops subprocess + class age key) → tmpfs delivery via helper container, ro-mounted at `/run/secrets` (`reconciler/src/secrets.rs`)
- [x] Removed-app GC (deletions only when config gone from git) + SQLite event log tagged with causing commit
- [ ] End-to-end verification against a real node (needs phase 0 infra): render PR → merge → converge → hello-world serving
- [ ] Private GHCR pull auth on nodes (bootstrap-level `docker login`; reconciler stays credential-free)

## Phase 3 — Org management 🚧

Code ✅ / live wiring ⏳:

- [x] Registry-gated discovery: App installed ∧ listed in `projects.yaml`; listed-but-uninstalled logs "pending" (`bot/src/org_sync.rs`)
- [x] Org reconciliation loop (hourly + on config pushes): ops repo + scaffold, app repos from `repo-templates/` with `{{app}}`/`{{org}}` placeholders, archive-on-removal, branch protection (`env/production` review gate, app `main` build check), `admins`/`developers` teams + membership
- [x] Tailscale sync: ACL policy rendered from people.yaml + project members, pushed via API; one-shot tagged auth keys minted for ingresses over the WG-internal API (`bot/src/tailscale.rs`)
- [x] Per-project ingress: Traefik + tailscale sidecar (shared netns, state volume, docker-provider constraint on `majnet.project`) ensured by the reconciler on the private node (`reconciler/src/ingress.rs`)
- [ ] Split DNS for `*.<project>.majksa.net` on the tailnet (Tailscale admin: DNS → split DNS pointing at the project ingress IPs; automate later)
- [ ] Live verification: real org onboarding end-to-end (create org → install App → registry line → repos/teams/ACLs appear)

## Phase 4 — Environment classes 🚧

Code ✅ / live wiring ⏳:

- [x] Promote flow: `POST /api/promote/{org}/{app}` copies the stable digest into the production overlay on ops `main`; the gated `env/production` render PR follows automatically (`bot/src/promote.rs`)
- [x] Ephemeral lifecycle: `pr-N` GHCR build → generated manifest (base ⊕ ephemeral overlay ⊕ PR patch) committed directly onto `env/ephemeral` (ADR 0003) → preview-URL PR comment (updated in place); PR close removes the manifest (`bot/src/ephemeral.rs`)
- [x] Ephemeral GC: 48 h grace after manifest removal, 7 d hard TTL enforced even while a manifest lingers; SQLite tracking (`reconciler/src/gc.rs`)
- [x] Reconciler converges all three classes; `age-production`/`age-stable` class keys already wired (§14)
- [ ] Generate the two class age keys + distribute (`age-keygen`; reconciler `MAJNET_AGE_KEY_DIR`)
- [ ] Live verification: PR → preview URL → close → grace GC observed end-to-end

## Phase 5 — Data & polish 🚧

Code ✅ / remaining ⏳:

- [x] DB provisioning: `database: {engine}` in manifests → logical DB + user on the zone's engine container, deterministic HMAC-derived passwords (no state), engine attached to project network, `DATABASE_URL` injected (`reconciler/src/db.rs`; postgres + mariadb — valkey/mongodb TODO)
- [x] Engine platform manifests (`platform-seed/platform/databases/`)
- [x] Backups: nightly dumps → restic → offsite + retention, systemd timer (`bootstrap/steps/60-backups.sh`)
- [x] Restart escape hatch: `POST /api/restart/{project}/{class}/{app}`, audit-logged with Tailscale identity (§16)
- [x] Rollback: `POST /api/rollback/{org}` — revert of ops `main` head, propagates via render PRs
- [x] Dashboard MVP: events + promote/rollback/restart (`dashboard/`)
- [x] Runbooks: node-recovery, bad-deploy, db-break-glass, secret-rotation, restore-test, github-outage
- [ ] Full dashboard: manifest editing, member management, TTL extension, role-based authorization from `people.yaml`
- [ ] Valkey + MongoDB provisioning
- [ ] Self-update story (open question §20.3)
- [ ] First weekly restore test actually performed

## Phase 6 — One-line auto-provisioning (Coolify-style install)

The end-state onboarding must be fully automatic — no manual per-node SSH steps:

- [ ] **One-line install on the main node** (`curl … | bash` style): installs the control plane (bot, reconciler, dashboard) and self-configures everything it can locally
- [ ] **Web-based setup continues from there**: first-run wizard on the dashboard (GitHub App creation via the [App manifest flow](https://docs.github.com/en/apps/creating-github-apps/registering-a-github-app/registering-a-github-app-from-a-manifest), Tailscale key, Cloudflare token, root org)
- [ ] **Node enrollment through the brain**: give the control plane SSH credentials for a fresh server + pick its role → it runs the bootstrap remotely (WG keys, Docker + mTLS PKI, firewall, agents) and registers the node in `nodes.yaml` itself
- [ ] The manual `bootstrap/` scripts remain the underlying payload — the brain executes the same steps over SSH; keep them runnable standalone for break-glass/recovery

> Origin: requirement added 2026-07-03 — the whole setup must be auto-provisioned like Coolify: one command on the master, continue in the web UI, add nodes by handing the brain SSH access.

## Open questions (design doc §20)

1. Backup target: Backblaze B2 vs Hetzner Storage Box
2. Per-project ingress footprint if projects multiply (full Traefik vs lighter proxy)
3. Reconciler self-update via ops repo vs manual bump
4. Whether `people.yaml` drives Tailscale user invitations or only ACLs
5. GHCR scope: per-org packages (default) vs central registry org
