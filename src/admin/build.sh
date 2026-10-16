#!/bin/bash
set -euo pipefail

# Build aka's dashboard image — the API plus the dashboard it serves, one
# container — tagged the way src/admin/docker-compose.yml expects it, and
# published under two names — Docker Hub ($HUB_NAMESPACE) and the FBL registry
# ($REGISTRY_NAMESPACE).
#
# Two tags, like aka's other images: $TAG (`latest`, what compose defaults to)
# and the version of the `admin-api` crate, which is what the healthcheck and
# the dashboard report — so a running container's tag and its advertised version
# are read from the same Cargo.toml line. `bin/version.sh` moves them all.
#
#   ./build.sh            # local tags + push to both names
#   ./build.sh --no-push  # local tags only
#
# Local platform only, and the pushed tag therefore names one architecture:
# this is a Rust and wasm compile, not configuration on top of a published base,
# so it is not cross-built the way aka-proxy/aka-dns are (src/aka/build.sh). The
# dashboard stack is a single-machine dev tool — `aka pull` fetches it with the
# other images, and the pushed :latest names one arch, so a machine of another
# arch rebuilds the tag from source with this script. bin/build.sh runs every
# src/**/build.sh, including this one.

root="$(cd "$(dirname "$0")/../.." && pwd)"

# Both names, like src/aka/build.sh: the hub repo is public, the FBL registry
# is what src/admin/docker-compose.yml names.
HUB_NAMESPACE="${HUB_NAMESPACE:-dewey4iv}"
REGISTRY_NAMESPACE="${REGISTRY_NAMESPACE:-registry.freshbrewlabs.cloud/aka}"
NAMES=("$HUB_NAMESPACE" "$REGISTRY_NAMESPACE")
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

# Both names, two tags each.
tag_args=()
refs=()
for ns in "${NAMES[@]}"; do
    tag_args+=(-t "$ns/aka-admin:$TAG" -t "$ns/aka-admin:$VERSION")
    refs+=("$ns/aka-admin:$TAG" "$ns/aka-admin:$VERSION")
done

# The context is the repo root (see the Dockerfile), not this directory.
docker build "${tag_args[@]}" -f devops/docker/aka-admin/Dockerfile .

if [ "$PUSH" = 1 ]; then
    # A plain push of the local tags: single platform, so no buildx split.
    for ref in "${refs[@]}"; do
        docker push "$ref"
    done
    echo "pushed ${refs[*]} ($(docker image inspect "$HUB_NAMESPACE/aka-admin:$TAG" --format '{{.Os}}/{{.Architecture}}'))"
fi

echo "tagged ${refs[*]} locally"
