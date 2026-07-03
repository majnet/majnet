# Runbooks

Operational procedures, driven by the failure modes in the design doc (§17). Target: node recovery < 1 h, runbook-driven, zero improvisation.

| Runbook | Covers |
|---|---|
| [node-recovery.md](node-recovery.md) | Any node down: bootstrap → restic restore → reconverge from git |
| [bad-deploy.md](bad-deploy.md) | Health-gate failures, rollback via revert, stuck-container restart |
| [db-break-glass.md](db-break-glass.md) | Emergency DB access: SSH + `docker exec`; deriving lost credentials |
| [secret-rotation.md](secret-rotation.md) | App secrets, personal keys, platform class keys, DB master key |
| [restore-test.md](restore-test.md) | Weekly restic restore verification |
| [github-outage.md](github-outage.md) | Pipeline paused; workloads unaffected; what not to do |
