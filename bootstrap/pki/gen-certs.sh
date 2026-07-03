#!/usr/bin/env bash
# PKI for the Docker APIs — one CA, a server cert per node (SAN = its WG IP),
# one client cert for the reconciler. Run on the operator machine; distribute
# server material to /etc/majnet/pki/ on each node, keep the client material
# for the reconciler, and keep ca-key.pem offline.
#
# Usage: ./gen-certs.sh <outdir> [main_wg_ip prod_wg_ip private_wg_ip]

set -euo pipefail

OUT=${1:?usage: gen-certs.sh <outdir> [main prod private WG IPs]}
MAIN_IP=${2:-10.88.0.1} PROD_IP=${3:-10.88.0.2} PRIV_IP=${4:-10.88.0.3}
DAYS=3650

mkdir -p "$OUT" && cd "$OUT"

if [[ ! -f ca.pem ]]; then
  echo "→ CA"
  openssl genpkey -algorithm ed25519 -out ca-key.pem
  openssl req -new -x509 -key ca-key.pem -days $DAYS -subj "/CN=majnet-docker-ca" -out ca.pem
fi

issue() { # issue <name> <extfile-content>
  local name=$1 ext=$2
  [[ -f $name-cert.pem ]] && { echo "→ $name (exists, skipping)"; return; }
  echo "→ $name"
  openssl genpkey -algorithm ed25519 -out "$name-key.pem"
  openssl req -new -key "$name-key.pem" -subj "/CN=majnet-$name" -out "$name.csr"
  openssl x509 -req -in "$name.csr" -CA ca.pem -CAkey ca-key.pem -CAcreateserial \
    -days $DAYS -extfile <(printf '%s' "$ext") -out "$name-cert.pem"
  rm -f "$name.csr"
  chmod 0400 "$name-key.pem"
}

for node_ip in "main:$MAIN_IP" "prod:$PROD_IP" "private:$PRIV_IP"; do
  node=${node_ip%%:*} ip=${node_ip##*:}
  issue "server-$node" "extendedKeyUsage=serverAuth
subjectAltName=IP:$ip,IP:127.0.0.1"
done

issue reconciler "extendedKeyUsage=clientAuth"

echo
echo "Distribute per node (rename to server-{cert,key}.pem):"
echo "  scp ca.pem server-<node>-cert.pem server-<node>-key.pem <node>:/etc/majnet/pki/"
echo "Reconciler gets: ca.pem reconciler-cert.pem reconciler-key.pem"
echo "Keep ca-key.pem OFFLINE."
