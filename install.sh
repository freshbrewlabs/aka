#!/usr/bin/env bash
set -euo pipefail

# One-liner installer for aka:
#
#   curl -fsSL https://raw.githubusercontent.com/freshbrewlabs/aka/main/install.sh | bash
#
# Checks prerequisites, clones the repo into a temp dir, and hands off to the
# repo's real installer (bin/install.sh): cargo installs the `aka` binary into
# ~/.cargo/bin and writes a commented ~/.aka.toml if you don't have one (never
# overwrites). Nothing here needs sudo, and no docker images are built — run
# `aka pull` once afterwards to fetch the published aka-proxy/aka-dns images.
# First run compiles aka from source, so give it a few minutes.
#
# Pick the git ref (branch or tag) to install:
#
#   curl -fsSL <url> | bash                       # main
#   curl -fsSL <url> | AKA_REF=v1.2.3 bash        # or: bash -s -- v1.2.3

AKA_REPO="${AKA_REPO:-https://github.com/freshbrewlabs/aka.git}"
AKA_REF="${1:-${AKA_REF:-main}}"

fail() { # <message> <hint...> — one actionable error, then out
    printf 'install.sh: %s\n' "$1" >&2
    shift
    printf '  %s\n' "$@" >&2
    exit 1
}

command -v git >/dev/null 2>&1 || fail "git is required" \
    "macOS: xcode-select --install    Debian/Ubuntu: sudo apt install git"

command -v cargo >/dev/null 2>&1 || fail "cargo (Rust) is required" \
    "install Rust, then re-run this:  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"

command -v docker >/dev/null 2>&1 || fail "docker is required" \
    "install Docker Desktop (or the docker engine), start it, and re-run this"

docker info >/dev/null 2>&1 || fail "the docker daemon is not reachable" \
    "start Docker (Desktop) and re-run this"

tmp="$(mktemp -d "${TMPDIR:-/tmp}/aka-install.XXXXXX")"
trap 'rm -rf "$tmp"' EXIT

echo "==> cloning $AKA_REPO ($AKA_REF)"
git clone --quiet --depth 1 --branch "$AKA_REF" "$AKA_REPO" "$tmp/aka"

bash "$tmp/aka/bin/install.sh"
