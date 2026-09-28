#!/bin/bash
set -euo pipefail

# Install aka locally: the two managed docker images, the `aka` binary into
# ~/.cargo/bin, and a commented ~/.aka.toml (an existing config is never
# overwritten). Safe to re-run; nothing here needs sudo — `aka up` prompts
# for sudo itself when it writes /etc/resolver files.

cd "$(dirname "$0")/.."

command -v docker >/dev/null || { echo "docker is required" >&2; exit 1; }
docker info >/dev/null 2>&1 || { echo "docker daemon is not running" >&2; exit 1; }

echo "==> building aka images (aka-proxy:local, aka-dns:local)"
bash src/aka/build.sh

echo "==> installing aka (cargo install --path src/aka/aka-cli)"
cargo install --path src/aka/aka-cli --locked --force

echo "==> $(aka version)  at $(command -v aka)"

if [ ! -f "$HOME/.aka.toml" ]; then
    echo "==> writing commented default ~/.aka.toml"
    aka config-file
else
    echo "==> keeping existing ~/.aka.toml"
fi

cat <<'EOF'

next steps:
  sudo aka up     # starts dns + proxy + proxyd, writes /etc/resolver/*
  aka status
  sudo aka down

EOF
