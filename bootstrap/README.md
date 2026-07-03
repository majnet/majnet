# Node bootstrap

Phase 0 tooling — idempotent bash bootstrap for the three Debian nodes (evolves v1's `prepare-server`, design doc §18). Re-running is always safe; node recovery = re-run + restic restore + reconciler reconverges from git.

> **Direction (roadmap phase 6):** the manual procedure below is temporary. The end state is Coolify-style auto-provisioning — one-line install on the main node, setup wizard in the dashboard, and node enrollment by handing the control plane SSH access; these scripts then become the payload the brain executes remotely. Keep them standalone-runnable for break-glass and recovery.

## Layout

```
bootstrap.sh          # entry point: runs all steps, or selected by prefix
node.env.example      # per-node config → /etc/majnet/node.env
lib/common.sh         # helpers (idempotent file install, apt, logging)
steps/
├── 10-base.sh        # packages, admin user, SSH hardening, auto security updates
├── 20-wireguard.sh   # cluster mesh: wg0, key generated on-node, 3 static peers
├── 30-docker.sh      # Docker CE; API bound ONLY to the WG IP, mTLS required
├── 40-firewall.sh    # nftables per role; prod: 80/443 from Cloudflare ranges only
└── 50-agents.sh      # Beszel agent (on WG IP); restic installed in 10-base
pki/gen-certs.sh      # CA + per-node server certs + reconciler client cert
```

## Procedure (per node, fresh Debian 12/13, as root)

```sh
# 0. on the operator machine, once:
pki/gen-certs.sh out/                      # keep ca-key.pem offline

# 1. on the node:
mkdir -p /etc/majnet/pki
cp node.env.example /etc/majnet/node.env && $EDITOR /etc/majnet/node.env
scp out/{ca.pem,server-<node>-*.pem} node:/etc/majnet/pki/   # rename to server-{cert,key}.pem
./bootstrap.sh

# 2. after all three nodes ran once: collect the printed WG pubkeys into each
#    node.env (and platform-seed/nodes.yaml), then re-run the mesh step:
./bootstrap.sh 20

# 3. verify from the operator machine / main node:
wg show
curl --cacert ca.pem --cert reconciler-cert.pem --key reconciler-key.pem \
  https://10.88.0.2:2376/_ping        # → OK
```

## Trust model recap (§7)

| Surface | Exposure |
|---|---|
| SSH 22, WireGuard 51820 | public (SSH is key-only, no root) |
| Docker API 2376 | WG interface only, mTLS (client = reconciler) |
| 80/443 | prod node only, Cloudflare ranges only (weekly refresh timer) |
| everything else | dropped |
