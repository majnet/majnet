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
# HTTPS only. Browsers gate service-worker registration — and so the PWA install
# prompt — on a secure context, which plain `http://majksa` is not.
#
# Turning :80 off does NOT strand anything: with an HTTPS serve in place,
# Tailscale answers port 80 itself with a 308 to the https:// URL. Saved CLI
# contexts and old bookmarks keep working (reqwest follows redirects), and
# browsers upgrade silently — so this enforces HTTPS without a coordinated
# re-login.
#
# Needs HTTPS certificates enabled for the tailnet (admin console → DNS). If
# they are not, fall back to plain HTTP so the dashboard is still reachable —
# it just cannot host the PWA.
if tailscale serve --bg --https 443 http://127.0.0.1:8090; then
  tailscale serve --http=80 off 2>/dev/null || true
else
  warn "tailscale serve :443 failed — enable HTTPS certificates for the tailnet to serve the dashboard over HTTPS (and to install it as a PWA)"
  tailscale serve --bg --http 80 http://127.0.0.1:8090
fi
