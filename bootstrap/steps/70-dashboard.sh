# 70-dashboard — the admin dashboard behind Tailscale (§16), main node only.
# The dashboard runs as a compose service from the CI-built image (ADR 0008,
# deploy/compose.yaml); this step only wires Tailscale — its identity trust
# anchor (Tailscale-User-Login headers) — and `tailscale serve`. Joining the
# tailnet is an interactive login, so it no-ops with a hint until `tailscale up`
# has run. After that: bootstrap.sh 70.

if [[ $NODE_ROLE != main ]]; then
  return 0
fi

COMPOSE=/opt/majnet/deploy/compose.yaml
if [[ ! -f $COMPOSE ]]; then
  warn "no deploy/compose.yaml ($COMPOSE) — run install.sh first; skipping"
  return 0
fi

if ! command -v tailscale &>/dev/null; then
  log "installing Tailscale"
  # shellcheck source=/dev/null  # /etc/os-release exists only on the node
  codename=$(. /etc/os-release && echo "$VERSION_CODENAME")
  curl -fsSL "https://pkgs.tailscale.com/stable/debian/$codename.noarmor.gpg" \
    -o /usr/share/keyrings/tailscale-archive-keyring.gpg
  curl -fsSL "https://pkgs.tailscale.com/stable/debian/$codename.tailscale-keyring.list" \
    -o /etc/apt/sources.list.d/tailscale.list
  apt-get update -q
fi
apt_ensure tailscale docker-compose-plugin
systemctl enable --now tailscaled

if ! tailscale status &>/dev/null; then
  warn "not logged into the tailnet — run 'tailscale up', then: bootstrap.sh 70"
  return 0
fi

log "ensuring the dashboard is up + tailscale serve"
# Idempotent; the image ref is auto-loaded from deploy/.env (written by install
# / majnet-update). nginx binds 127.0.0.1:8090 (host networking).
docker compose -f "$COMPOSE" up -d dashboard
tailscale serve --bg --http 80 http://127.0.0.1:8090
# Also serve HTTPS so the dashboard is installable as a PWA: browsers gate
# service-worker registration (and the install prompt) on a secure context, and
# `http://majksa` is not one. Keep :80 as well — the CLI's saved context and any
# bookmark point at http://, and dropping it would break them.
#
# Needs HTTPS certificates enabled for the tailnet (admin console → DNS). Not
# fatal if they are not: the dashboard stays reachable over :80, it just cannot
# be installed.
tailscale serve --bg --https 443 http://127.0.0.1:8090 ||
  warn "tailscale serve :443 failed — enable HTTPS certificates for the tailnet if you want the dashboard installable as a PWA"
