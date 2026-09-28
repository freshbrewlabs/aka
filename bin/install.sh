#!/bin/bash
set -euo pipefail

# Install aka from this checkout — run directly, or let the repo-root
# install.sh one-liner clone the repo and call this. Installs: the two
# managed docker images, the `aka` binary into ~/.cargo/bin, and a commented
# ~/.aka.toml (an existing config is never overwritten). Safe to re-run;
# nothing here needs sudo. Running `aka install-sudo-rule` once is what
# makes day-to-day `aka up`/`aka down` sudo-free (without it they prompt
# for a password).

cd "$(dirname "$0")/.."

export PATH="$HOME/.cargo/bin:$PATH"  # a just-installed rustup isn't on PATH yet

command -v docker >/dev/null || { echo "docker is required" >&2; exit 1; }
docker info >/dev/null 2>&1 || { echo "docker daemon is not running" >&2; exit 1; }

echo "==> building aka images (tagged ${HUB_NAMESPACE:-dewey4iv}/aka-*:${TAG:-latest})"
bash src/aka/build.sh --no-push

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
  aka install-sudo-rule   # one-time: makes up/down permanently sudo-free
  aka up                  # starts dns + proxy + proxyd, writes /etc/resolver/*
  aka status
  aka down

Without the rule, aka up/down fall back to prompting for your password.

EOF
