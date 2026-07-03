# Node bootstrap

Phase 0 tooling — evolves MajNet v1's `prepare-server` (see design doc §18) into an idempotent bootstrap for the three nodes.

Per node (`main` / `prod` / `private`), bootstrap must:

1. Install Docker; bind the Docker API **only to the node's WireGuard IP**, mTLS required (client certs held by the reconciler).
2. Configure the WireGuard mesh (three static peers, from `nodes.yaml`).
3. Apply role-specific firewalling:
   - **prod**: 80/443 open to Cloudflare IP ranges only; everything else WG-only
   - **main** / **private**: no public listeners; access via Tailscale/WG only
4. Install the restic backup agent + Beszel agent.

Node recovery = run bootstrap → restic restore → reconciler reconverges from git.
