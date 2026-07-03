# Runbooks

Operational procedures, driven by the failure modes in the design doc (§17). Target: node recovery < 1 h, runbook-driven, zero improvisation.

Planned (Phase 5):

| Runbook | Covers |
|---|---|
| `node-recovery.md` | Any node down: bootstrap script → restic restore → reconverge from git |
| `bad-deploy.md` | Health-gate failures, `git revert` on `main` + re-render, `git log env/production` forensics |
| `db-break-glass.md` | Emergency DB access: SSH over WG + `docker exec` (DBs have no VPN listener by design) |
| `secret-rotation.md` | SOPS/age key rotation: edit → commit → blue-green roll; platform class key rotation |
| `restore-test.md` | Weekly restic restore verification |
| `github-outage.md` | Pipeline paused; workloads unaffected; what still works and what doesn't |
