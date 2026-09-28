#!/bin/bash
set -euo pipefail

# Build aka's dashboard image — the API plus the dashboard it serves, one
# container — tagged the way src/admin/docker-compose.yml expects it, and
# published to Docker Hub as $HUB_NAMESPACE/aka-admin.
#
# Two tags, like aka's other images: $TAG (`latest`, what compose defaults to)
# and the version of the `admin-api` crate, which is what the healthcheck and
# the dashboard report — so a running container's tag and its advertised version
# are read from the same Cargo.toml line. `bin/version.sh` moves them all.
#
#   ./build.sh            # local tags + hub push
#   ./build.sh --no-push  # local tags only
#
# Local platform only, and the pushed tag therefore names one architecture:
# this is a Rust and wasm compile, not configuration on top of a published base,
# so it is not cross-built the way aka-proxy/aka-dns are (src/aka/build.sh). The
# dashboard stack is a single-machine dev tool — nothing pulls it automatically
# (`aka pull` fetches aka-proxy/aka-dns only), and a machine of another arch
# builds its own tag from source with this script. bin/build.sh runs every
# src/**/build.sh, including this one.

root="$(cd "$(dirname "$0")/../.." && pwd)"

HUB_NAMESPACE="${HUB_NAMESPACE:-dewey4iv}"
TAG="${TAG:-latest}"
PUSH=1

# $root, not a relative path: bin/build.sh runs this file from its own directory
VERSION="$(bash "$root/bin/crate_version.sh" admin-api)"

cd "$root"

for arg in "$@"; do
    case "$arg" in
        --no-push) PUSH=0 ;;
        *) echo "usage: $0 [--no-push]" >&2; exit 2 ;;
    esac
done

# The context is the repo root (see the Dockerfile), not this directory.
docker build -t "$HUB_NAMESPACE/aka-admin:$TAG" \
    -t "$HUB_NAMESPACE/aka-admin:$VERSION" \
    -f devops/docker/aka-admin/Dockerfile .

if [ "$PUSH" = 1 ]; then
    # A plain push of the local tags: single platform, so no buildx split.
    docker push "$HUB_NAMESPACE/aka-admin:$TAG"
    docker push "$HUB_NAMESPACE/aka-admin:$VERSION"
    echo "pushed $HUB_NAMESPACE/aka-admin:$TAG + :$VERSION ($(docker image inspect "$HUB_NAMESPACE/aka-admin:$TAG" --format '{{.Os}}/{{.Architecture}}'))"
fi

echo "tagged $HUB_NAMESPACE/aka-admin:$TAG + :$VERSION locally"
