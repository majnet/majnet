# 30-docker — Docker CE with the API bound ONLY to the WireGuard IP, mTLS
# required (client certs held by the reconciler). Design doc §5, §7.

if ! command -v docker &>/dev/null; then
  log "installing Docker CE"
  install -m 0755 -d /etc/apt/keyrings
  curl -fsSL https://download.docker.com/linux/debian/gpg -o /etc/apt/keyrings/docker.asc
  chmod a+r /etc/apt/keyrings/docker.asc
  # shellcheck source=/dev/null  # /etc/os-release exists only on the node
  echo "deb [arch=$(dpkg --print-architecture) signed-by=/etc/apt/keyrings/docker.asc] \
https://download.docker.com/linux/debian $(. /etc/os-release && echo "$VERSION_CODENAME") stable" \
    > /etc/apt/sources.list.d/docker.list
  apt-get update -q
fi
apt_ensure docker-ce docker-ce-cli containerd.io

WG_IP=${WG_ADDRESS%/*}
PKI=/etc/majnet/pki
for f in ca.pem server-cert.pem server-key.pem; do
  [[ -f $PKI/$f ]] || die "missing $PKI/$f — generate with bootstrap/pki/gen-certs.sh and copy over"
done

install_stdin /etc/docker/daemon.json 0644 <<EOF
{
  "hosts": ["unix:///var/run/docker.sock", "tcp://$WG_IP:${DOCKER_API_PORT:-2376}"],
  "tls": true,
  "tlsverify": true,
  "tlscacert": "$PKI/ca.pem",
  "tlscert": "$PKI/server-cert.pem",
  "tlskey": "$PKI/server-key.pem",
  "live-restore": true,
  "log-driver": "local",
  "log-opts": { "max-size": "20m", "max-file": "3" },
  "builder": {
    "gc": {
      "enabled": true,
      "defaultKeepStorage": "20GB",
      "policy": [
        { "keepStorage": "10GB", "filter": ["unused-for=168h"] },
        { "keepStorage": "20GB", "all": true }
      ]
    }
  }
}
EOF

# daemon.json "hosts" conflicts with the packaged unit's -H flag — clear it.
install_stdin /etc/systemd/system/docker.service.d/majnet.conf 0644 <<'EOF'
[Unit]
# Docker must not start before wg0 exists, or the tcp bind fails.
After=wg-quick@wg0.service
Requires=wg-quick@wg0.service

[Service]
# Docker 29's daemon refuses API < 1.40. The per-project ingress Traefik's
# docker provider hard-pins API 1.24 (ignores DOCKER_API_VERSION), so without
# this the provider can't reach the daemon → no routers → VPN hosts 404. Lower
# the accepted minimum so old clients (Traefik) work.
Environment=DOCKER_MIN_API_VERSION=1.24
ExecStart=
ExecStart=/usr/bin/dockerd --containerd=/run/containerd/containerd.sock
EOF

# Reclaim disk that nothing references. Without this a node fills up and stops
# being able to converge: containers cannot write, so apps die and every
# subsequent deploy fails. Observed on a node at 155.9G/156.2G — 100%, zero
# bytes free — where two apps were already dead and every converge was failing.
# Nothing in MajNet reclaimed images or build cache; `daemon.json` capped
# container logs only, and the reconciler's `app_info_prune` is database rows.
#
# `until=168h` keeps a week, so `deploy rollback` to a recent version still has
# its image locally. A bare `image prune -af` would delete every image not
# currently running and turn a rollback into a registry pull — or a failure, if
# the registry is unreachable at the moment you need it most.
#
# Deliberately NOT `docker system prune --volumes`: these nodes host the
# managed databases, and that flag deletes volumes no *running* container
# claims. A stopped Postgres during a deploy would qualify. It is the one flag
# that turns a cleanup into data loss.
install_stdin /etc/systemd/system/majnet-docker-prune.service 0644 <<'EOF'
[Unit]
Description=MajNet Docker reclaim (images and stopped containers older than a week)
[Service]
Type=oneshot
ExecStart=/usr/bin/docker image prune -af --filter until=168h
ExecStart=/usr/bin/docker container prune -f --filter until=168h
EOF

install_stdin /etc/systemd/system/majnet-docker-prune.timer 0644 <<'EOF'
[Unit]
Description=MajNet Docker reclaim
[Timer]
OnCalendar=*-*-* 04:30:00
RandomizedDelaySec=30m
Persistent=true
[Install]
WantedBy=timers.target
EOF

systemctl daemon-reload
systemctl enable --now docker
systemctl enable --now majnet-docker-prune.timer
changed && systemctl restart docker
reset_changed
