# platform-seed

Initial content for the **`majksa-platform/platform`** repo (design doc §10) — the root of all platform config. Copy this directory's contents into that repo once the root org exists; from then on *that repo* is the source of truth and this seed is only a reference.

```
nodes.yaml          # the three nodes: WG IPs, roles, Docker API endpoints
people.yaml         # GitHub ↔ Tailscale identity map, admin group
projects.yaml       # project registry — the discovery gate (§2)
tailscale-acl.tmpl  # ACL policy template, rendered + pushed by the bot
platform/           # manifests for platform services on the nodes
├── edge-main/      # public Traefik on the prod node
└── hello-world/    # phase-0 smoke test behind edge-main
```

Bootstrapping order (phase 0):

1. Create the `majksa-platform` org on GitHub (manual — §2), create the `platform` repo, push this seed.
2. Bootstrap the three nodes (`bootstrap/`), fill real WG pubkeys + endpoints into `nodes.yaml`.
3. On the prod node, bring up `edge-main` + `hello-world` by hand (`docker compose up -d`) — from phase 2 on, the reconciler owns platform manifests and manual composes are retired.
4. Point a test DNS record (Cloudflare, proxied) at the prod node → the hello-world page proves the CF → edge-main → container path.
