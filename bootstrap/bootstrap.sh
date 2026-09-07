#!/usr/bin/env bash
# MajNet node bootstrap — idempotent, safe to re-run (design doc §4, §19 phase 0).
#
# Usage (on a fresh Debian 12/13 minimal install, as root):
#   1. mkdir -p /etc/majnet && cp node.env.example /etc/majnet/node.env && $EDITOR /etc/majnet/node.env
#   2. copy PKI material (from pki/gen-certs.sh) to /etc/majnet/pki/
#   3. ./bootstrap.sh            # runs all steps
#      ./bootstrap.sh 20 30     # or just selected steps by prefix
#
# Node recovery = re-run this + restic restore + reconciler reconverges from git.

set -euo pipefail
cd "$(dirname "$0")"
# shellcheck source-path=SCRIPTDIR
source lib/common.sh

require_root
load_config

log "bootstrapping node '$NODE_NAME' (role: $NODE_ROLE)"

# Which payload is this? `bootstrap.sh` sources whatever is in `steps/`, so a
# stale tree re-applies the old config, exits 0, and prints "done." — a merged
# fix can be "applied" several times without ever landing, and the only way to
# tell is to inspect whatever the step was supposed to write.
#
# Two provenance sources, because a node has neither reliably:
#   - `main` is a git checkout (majnet-update fetches the pinned ref there)
#   - enrolled nodes get a tarball, so no .git — `enroll.rs` writes
#     `.payload-ref` instead
# Neither present means nobody knows what this tree is, which is worth saying
# out loud rather than leaving to be discovered.
payload_ref() {
  local rev
  if rev=$(git -C . rev-parse --short HEAD 2>/dev/null) && [[ -n $rev ]]; then
    printf 'git %s' "$rev"
    git -C . diff --quiet 2>/dev/null || printf ' (dirty)'
    return
  fi
  if [[ -f .payload-ref ]]; then
    printf 'pushed %s' "$(cut -c1-12 < .payload-ref)"
    return
  fi
  return 1
}
if ref=$(payload_ref); then
  log "payload: $ref"
else
  warn "payload provenance UNKNOWN — no .git and no .payload-ref."
  warn "  This tree may predate a fix you believe is applied; bootstrap.sh"
  warn "  cannot tell. Verify whatever the step writes, not just its exit code."
fi

steps=(steps/*.sh)
if (($#)); then
  selected=()
  for prefix in "$@"; do
    for s in "${steps[@]}"; do
      [[ $(basename "$s") == "$prefix"* ]] && selected+=("$s")
    done
  done
  steps=("${selected[@]}")
fi

for step in "${steps[@]}"; do
  log "── $(basename "$step") ──────────────────────"
  # shellcheck source=/dev/null
  source "$step"
done

log "done. If this was the first run: share the WireGuard pubkey above with"
log "the other nodes' node.env, re-run step 20 everywhere, then verify:"
log "  wg show && curl --cacert /etc/majnet/pki/ca.pem https://\$WG_IP:\$DOCKER_API_PORT/_ping"
