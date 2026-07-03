# Roadmap

Phased plan from the design doc (§19), tracked here as the implementation progresses.

## Phase 0 — Foundations 🚧 (current)

- [ ] Node bootstrap: WireGuard mesh, Docker APIs bound to WG IPs + mTLS, node roles (`bootstrap/`)
- [ ] Firewalls (public 80/443 from Cloudflare ranges only on prod; everything else WG/Tailscale)
- [ ] Tailscale org + base ACLs
- [ ] Root org `majksa-platform` + `platform` repo (nodes.yaml, people.yaml, projects.yaml, ACL template)
- [ ] `edge-main` Traefik on prod node
- [ ] Hello-world public service

## Phase 1 — Bot MVP

- [ ] GitHub App: JWT auth, per-org installation tokens
- [ ] Webhook server + signature verification
- [ ] Digest bumps: signed commits to a project `ops` repo
- [ ] Repo access proxy (cached snapshots served to the reconciler over WG)
- [ ] GHA workflow template (`templates/repo-templates/`)

## Phase 2 — Reconciler MVP

- [ ] Manifest schema v1 (`crates/common`)
- [ ] Rendering: base ⊕ overlay → render PRs (bot side)
- [ ] Single-app convergence to private node (bollard over WG)
- [ ] Blue-green: start new → health check → flip Traefik label → stop old
- [ ] SOPS decrypt → tmpfs-mounted secret files

## Phase 3 — Org management

- [ ] Registry-gated discovery (App installed ∧ listed in `projects.yaml`)
- [ ] Org reconciliation loop: repo creation from templates, settings, branch protection, teams, membership, archive-on-removal
- [ ] Tailscale sync: groups, ACLs, per-project ingress auth keys
- [ ] Per-project ingress (Traefik + tailscale sidecar) + Docker networks
- [ ] Split DNS for `*.{project}.majksa.net` on the tailnet

## Phase 4 — Environment classes

- [ ] Production class: promote PRs, `env/production` render-PR review gate, `age-production` key
- [ ] Ephemeral lifecycle: PR-scoped deploys, preview-URL comments, 48 h grace / 7 d hard TTL GC

## Phase 5 — Data & polish

- [ ] DB provisioning (per-project logical DBs/users) + migrations as one-shot containers
- [ ] restic backups + weekly restore tests
- [ ] Dashboard (reads: reconciler state API; writes: bot → commits/PRs)
- [ ] Runbooks (`docs/runbooks/`)
- [ ] Self-update story

## Open questions (design doc §20)

1. Backup target: Backblaze B2 vs Hetzner Storage Box
2. Per-project ingress footprint if projects multiply (full Traefik vs lighter proxy)
3. Reconciler self-update via ops repo vs manual bump
4. Whether `people.yaml` drives Tailscale user invitations or only ACLs
5. GHCR scope: per-org packages (default) vs central registry org
