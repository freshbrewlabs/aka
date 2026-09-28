#!/bin/bash
set -euo pipefail

# Install aka from this checkout — run directly, or let the repo-root
# install.sh one-liner clone the repo and call this. Installs: the `aka`
# binary into ~/.cargo/bin and a commented ~/.aka.toml (an existing config is
# never overwritten). Safe to re-run; nothing here needs sudo, and **no docker
# images are built** — `aka pull` fetches the published aka-proxy/aka-dns
# images, and `bash src/aka/build.sh --no-push` builds them locally if you're
# hacking on them. Running `aka install-sudo-rule` once is what makes
# day-to-day `aka up`/`aka down` sudo-free (without it they prompt for a
# password).

cd "$(dirname "$0")/.."

export PATH="$HOME/.cargo/bin:$PATH"  # a just-installed rustup isn't on PATH yet

command -v docker >/dev/null || { echo "docker is required" >&2; exit 1; }
docker info >/dev/null 2>&1 || { echo "docker daemon is not running" >&2; exit 1; }

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
  aka pull                # first time: fetches aka-proxy / aka-dns from the hub
  aka up                  # starts dns + proxy + proxyd, writes /etc/resolver/*
  aka status
  aka down

Without the rule, aka up/down fall back to prompting for your password.

EOF
