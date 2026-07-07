# 70-dashboard — the admin dashboard behind Tailscale (§16), main node only.
# Tailscale is the dashboard's identity trust anchor (Tailscale-User-Login
# headers), and joining the tailnet is an interactive login — so this step
# installs everything, then no-ops with a hint until `tailscale up` has run.
# After that: bootstrap.sh 70 brings the dashboard up.

if [[ $NODE_ROLE != main ]]; then
  return 0
fi

# install.sh clones the full checkout to /opt/majnet; manual runs from a
# checkout land in <repo>/bootstrap, so the parent works too. Enrolled
# workers only receive the bootstrap/ payload — but they never get here.
DASH_DIR=/opt/majnet/dashboard
[[ -d $DASH_DIR ]] || DASH_DIR=$(cd .. && pwd)/dashboard
if [[ ! -d $DASH_DIR ]]; then
  warn "no dashboard/ next to this payload — skipping"
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

log "starting the dashboard + tailscale serve"
docker compose -f "$DASH_DIR/compose.yaml" up -d
tailscale serve --bg --http 80 http://127.0.0.1:8090
