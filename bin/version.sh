#!/bin/bash
set -euo pipefail

# Move every crate in the workspace to one version, in one step. That number is
# the version `aka version` prints, the version the dashboard's healthcheck
# reports, and the tag all three images get next to `latest`
# (src/aka/build.sh, src/admin/build.sh) — so bump it here and everything that
# advertises a version agrees.
#
#   ./bin/version.sh                      # what the crates say now
#   ./bin/version.sh --dry-run 0.2.0      # what would change
#   ./bin/version.sh 0.2.0                # every crate -> 0.2.0
#
# It is a thin, checked wrapper around cargo-edit's `cargo set-version <version>
# --workspace`, which rewrites every member's Cargo.toml and refreshes
# Cargo.lock; the checks here are the ones that tool has been seen to skip.
# One-time install per machine:
#
#   cargo install --no-default-features --features set-version cargo-edit
#
# Not the standalone `cargo-set-version` crate: it takes --workspace/--dry-run,
# ignores them, bumps part of the workspace, and leaves the lock untouched.
#
# cargo-edit will not go backwards, so a downgrade rewrites the package version
# lines directly — same result, same checks afterwards.

cd "$(dirname "$0")/.."

show() { # the crates whose versions become image tags
    for crate in aka-cli aka-kernel admin-api admin-kernel; do
        printf '  %-12s %s\n' "$crate" "$(bash bin/crate_version.sh "$crate")"
    done
}

reject() {
    echo "version.sh: '$1' is not usable as a version and a docker tag" >&2
    echo "  start with a letter or digit, then [A-Za-z0-9._-], no '..' —" >&2
    echo "  e.g. 0.2.0, 1.0.0-rc.1" >&2
    exit 2
}

case "${1:-}" in
    ""|--show)
        show
        exit 0
        ;;
    -h|--help)
        echo "usage: $(basename "$0") [--dry-run] <version>   (no arguments: show)"
        exit 0
        ;;
esac

dry_run=0
if [ "${1:-}" = "--dry-run" ]; then
    dry_run=1
    shift
fi

[ $# -eq 1 ] || {
    echo "usage: $(basename "$0") [--dry-run] <version>   (or --show)" >&2
    exit 2
}

version="$1"

# A version here is also a docker tag, and tags are stricter than semver; cargo
# set-version still has the last word on whether it is semver at all.
case "$version" in
    [0-9A-Za-z]*) ;;
    *) reject "$version" ;;
esac
case "$version" in
    *[!0-9A-Za-z._-]*|*..) reject "$version" ;;
esac

# cargo finds external subcommands on PATH, and cargo-edit's takes only
# versions, so probing it with a flag would fail for the wrong reason.
command -v cargo-set-version >/dev/null 2>&1 || {
    echo "version.sh: \`cargo set-version\` is not installed" >&2
    echo "  cargo install --no-default-features --features set-version cargo-edit" >&2
    exit 1
}

before="$(bash bin/crate_version.sh aka-cli)"

if [ "$dry_run" = 1 ]; then
    cargo set-version "$version" --workspace --dry-run

    # The standalone `cargo-set-version` crate takes --dry-run and writes
    # anyway, so a dry run is only trustworthy if nothing moved.
    after="$(bash bin/crate_version.sh aka-cli)"
    [ "$before" = "$after" ] || {
        echo "version.sh: --dry-run rewrote the manifests ($before -> $after)." >&2
        echo "  That is the incomplete standalone crate, not cargo-edit's:" >&2
        echo "    cargo uninstall cargo-set-version" >&2
        echo "    cargo install --no-default-features --features set-version cargo-edit" >&2
        echo "  Then undo what it wrote (tracked manifests) and bump the untracked" >&2
        echo "  crates back: git checkout -- 'src/*/Cargo.toml' && $(basename "$0") $before" >&2
        exit 1
    }
    exit 0
fi

# Version first: cargo-edit takes flags on either side, and it is also the only
# ordering the standalone crate parses.
if ! out="$(cargo set-version "$version" --workspace 2>&1)"; then
    case "$out" in
        *downgrade*)
            # cargo-edit will not go backwards, and a bump nobody can undo is a
            # trap: only the package version line moves, same as its own rewrite.
            echo "==> cargo-edit will not downgrade; setting the version directly"
            cargo metadata --format-version 1 --no-deps |
                tr '{' '\n' |
                grep -o '"manifest_path":"[^"]*"' |
                sed -e 's/^"manifest_path":"//' -e 's/"$//' |
                while IFS= read -r manifest; do
                    awk -v version="$version" '
                        !moved && /^version = / {
                            print "version = \"" version "\""
                            moved = 1
                            next
                        }
                        { print }
                    ' "$manifest" > "$manifest.tmp" && mv "$manifest.tmp" "$manifest"
                done
            ;;
        *)
            printf '%s\n' "$out" >&2
            echo "version.sh: cargo set-version failed for $version" >&2
            exit 1
            ;;
    esac
fi

# The bump is only real once cargo can resolve the workspace again: a partial
# bump leaves path-dep requirements pointing at versions that do not exist. This
# resolve also refreshes Cargo.lock.
cargo metadata --format-version 1 >/dev/null || {
    echo "version.sh: the workspace does not resolve after the bump to $version" >&2
    echo "  compare the manifests: git diff -- '*Cargo.toml'" >&2
    exit 1
}

echo "==> every workspace crate is now $version (Cargo.lock refreshed)"
show

cat <<EOF

next steps:
  bin/build.sh     # pushes <image>:${TAG:-latest} and <image>:$version
  bin/unit_test.sh # cargo test --workspace
  aka version      # after reinstalling the cli: cargo install --path src/aka/aka-cli --force
EOF
