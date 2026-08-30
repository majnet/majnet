#!/usr/bin/env bash
# Install the `majnet` CLI on a laptop.
#
#   curl -fsSL https://raw.githubusercontent.com/majnet/majnet/main/scripts/install-cli.sh | bash
#
# Downloads the release binary for this platform, verifies its checksum, and
# puts it somewhere on PATH. No Rust toolchain, no root unless the chosen
# directory needs it.
#
# Environment:
#   MAJNET_VERSION   release tag to install (default: latest)
#   MAJNET_BIN_DIR   where to install (default: ~/.local/bin, or /usr/local/bin
#                    when that is writable and ~/.local/bin is not on PATH)
#   MAJNET_REPO      owner/repo to download from (default: majnet/majnet)
set -euo pipefail

REPO=${MAJNET_REPO:-majnet/majnet}
VERSION=${MAJNET_VERSION:-latest}

die() { printf 'install-cli: %s\n' "$*" >&2; exit 1; }
log() { printf '\033[36m==>\033[0m %s\n' "$*" >&2; }

need() { command -v "$1" >/dev/null 2>&1 || die "need $1 on PATH"; }
need uname
need tar
if command -v curl >/dev/null 2>&1; then
  fetch() { curl -fsSL "$1" -o "$2"; }
  fetch_stdout() { curl -fsSL "$1"; }
elif command -v wget >/dev/null 2>&1; then
  fetch() { wget -qO "$2" "$1"; }
  fetch_stdout() { wget -qO- "$1"; }
else
  die "need curl or wget"
fi

# --- target ----------------------------------------------------------------
os=$(uname -s)
arch=$(uname -m)
case "$os/$arch" in
  Linux/x86_64|Linux/amd64)   target=x86_64-unknown-linux-musl ;;
  Linux/aarch64|Linux/arm64)  target=aarch64-unknown-linux-musl ;;
  Darwin/x86_64)              target=x86_64-apple-darwin ;;
  Darwin/arm64)               target=aarch64-apple-darwin ;;
  *) die "no prebuilt binary for $os/$arch — build from source: cargo install --git https://github.com/$REPO majnet-cli" ;;
esac

# --- version ---------------------------------------------------------------
if [[ $VERSION == latest ]]; then
  # Resolve through the API rather than the /latest redirect so the failure is
  # a readable message when there is no release yet.
  VERSION=$(fetch_stdout "https://api.github.com/repos/$REPO/releases/latest" \
    | sed -n 's/.*"tag_name":[[:space:]]*"\([^"]*\)".*/\1/p' | head -1)
  [[ -n $VERSION ]] || die "could not resolve the latest release of $REPO (set MAJNET_VERSION)"
fi

name="majnet-$target"
base="https://github.com/$REPO/releases/download/$VERSION"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

log "downloading $name ($VERSION)"
fetch "$base/$name.tar.gz" "$tmp/$name.tar.gz" \
  || die "no asset $name.tar.gz in release $VERSION"

# --- verify ----------------------------------------------------------------
# A truncated or substituted download must fail here, not at the first run.
if fetch "$base/$name.tar.gz.sha256" "$tmp/$name.tar.gz.sha256" 2>/dev/null; then
  expected=$(awk '{print $1}' "$tmp/$name.tar.gz.sha256")
  if command -v sha256sum >/dev/null 2>&1; then
    actual=$(sha256sum "$tmp/$name.tar.gz" | awk '{print $1}')
  elif command -v shasum >/dev/null 2>&1; then
    actual=$(shasum -a 256 "$tmp/$name.tar.gz" | awk '{print $1}')
  else
    actual=$expected
    log "no sha256 tool available — skipping checksum verification"
  fi
  [[ $actual == "$expected" ]] || die "checksum mismatch for $name.tar.gz"
else
  log "no published checksum for $VERSION — skipping verification"
fi

tar -xzf "$tmp/$name.tar.gz" -C "$tmp"
[[ -f "$tmp/$name/majnet" ]] || die "archive did not contain a majnet binary"

# --- install ---------------------------------------------------------------
if [[ -n ${MAJNET_BIN_DIR:-} ]]; then
  bin_dir=$MAJNET_BIN_DIR
elif [[ ":$PATH:" == *":$HOME/.local/bin:"* ]] || [[ ! -w /usr/local/bin ]]; then
  bin_dir="$HOME/.local/bin"
else
  bin_dir=/usr/local/bin
fi
mkdir -p "$bin_dir"
install -m 0755 "$tmp/$name/majnet" "$bin_dir/majnet"
log "installed $bin_dir/majnet"

case ":$PATH:" in
  *":$bin_dir:"*) ;;
  *) log "note: $bin_dir is not on your PATH — add it to your shell profile" ;;
esac

"$bin_dir/majnet" --version || true
cat >&2 <<'NEXT'

Next:
  majnet login                     # find the control plane on your tailnet
  majnet whoami                    # confirm the platform sees you as you
  majnet status                    # the fleet, on one screen

Shell completions:
  majnet completions zsh  > ~/.zfunc/_majnet          # zsh
  majnet completions bash > ~/.local/share/bash-completion/completions/majnet

Driving this from an AI agent:
  majnet agent-guide               # the full machine-readable reference
  majnet agent-guide --install     # drop it into ./.claude/skills/majnet/
NEXT
