# 10-base — packages, admin user, SSH hardening, unattended upgrades.

apt-get update -q
apt_ensure ca-certificates curl gnupg jq nftables wireguard-tools \
  unattended-upgrades openssh-server sudo restic

# Admin user with SSH keys; root login gets disabled below.
if ! id "$ADMIN_USER" &>/dev/null; then
  log "creating admin user $ADMIN_USER"
  useradd -m -s /bin/bash -G sudo "$ADMIN_USER"
fi
install -d -m 0700 -o "$ADMIN_USER" -g "$ADMIN_USER" "/home/$ADMIN_USER/.ssh"
install_stdin "/home/$ADMIN_USER/.ssh/authorized_keys" 0600 \
  < <(printf '%s\n' "$ADMIN_SSH_KEYS" | grep -v '^[[:space:]]*$' || true)
chown "$ADMIN_USER:$ADMIN_USER" "/home/$ADMIN_USER/.ssh/authorized_keys"
echo "$ADMIN_USER ALL=(ALL) NOPASSWD:ALL" | install_stdin /etc/sudoers.d/majnet 0440

# SSH: keys only, no root.
install_stdin /etc/ssh/sshd_config.d/majnet.conf 0644 <<'EOF'
PasswordAuthentication no
KbdInteractiveAuthentication no
PermitRootLogin no
X11Forwarding no
EOF
systemctl reload ssh || systemctl reload sshd

# Security updates on autopilot.
install_stdin /etc/apt/apt.conf.d/20auto-upgrades 0644 <<'EOF'
APT::Periodic::Update-Package-Lists "1";
APT::Periodic::Unattended-Upgrade "1";
EOF
