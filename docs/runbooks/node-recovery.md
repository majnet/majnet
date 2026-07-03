# Node recovery

**Target: < 1 h** (design §17). Applies to any of the three nodes; prod node down = prod down (deliberate, no failover).

1. **Provision replacement** — fresh Debian 12/13, same public IP if possible (else update WG `Endpoint` in the other two nodes' `/etc/majnet/node.env` + `platform-seed`→platform repo `nodes.yaml`).
2. **Bootstrap** — copy `node.env` (from your ops records or reconstruct from `nodes.yaml`) + Docker PKI (`gen-certs.sh` output; reissue the node's server cert if lost) to `/etc/majnet/`, run `bootstrap/bootstrap.sh`.
   - New WG key was generated → paste the printed pubkey into the peers' `node.env` and the platform repo, re-run `bootstrap.sh 20` on the other nodes.
3. **Restore data** — `source /etc/majnet/restic.env && restic restore latest --target / --include /var/backups/majnet`, then load dumps into the (freshly started) engine containers:
   `zcat /var/backups/majnet/postgres.sql.gz | docker exec -i majnet-postgres psql -U postgres`
4. **Platform services** (prod: `edge-main`; main: bot/reconciler/dashboard) — `docker compose up -d` from the platform repo manifests.
5. **Reconverge** — the reconciler's next cycle recreates every project network, ingress, DB user and container from git. Force it: `curl -X POST http://10.88.0.1:9090/notify -d '{}' -H 'content-type: application/json'`.
6. **Verify** — `wg show` (handshakes), `curl …/_ping` (Docker mTLS), dashboard events show converges, hello-world/apps respond.

DB passwords are HMAC-derived from `/etc/majnet/age/db-master.key` — restoring that file restores every credential. It's in no backup by default: keep it with the age keys in your offline secrets store.
