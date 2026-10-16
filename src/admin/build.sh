#!/bin/bash
set -euo pipefail

# Build aka's dashboard image — the API plus the dashboard it serves, one
# container — tagged the way src/admin/docker-compose.yml expects it, and
# published to the FBL registry ($REGISTRY_NAMESPACE).
#
# Two tags, like aka's other images: $TAG (`latest`, what compose defaults to)
# and the version of the `admin-api` crate, which is what the healthcheck and
# the dashboard report — so a running container's tag and its advertised version
# are read from the same Cargo.toml line. `bin/version.sh` moves them all.
#
#   ./build.sh            # local tags + push to both names
#   ./build.sh --no-push  # local tags only
#
# Published for linux/amd64 + linux/arm64 ($PLATFORMS), like aka-proxy and
# aka-dns: the Rust stages in the Dockerfile run on the builder's own
# architecture and cross-link the API, so the extra platform costs no emulated
# compile and no arm64 runner. The local tags are still the builder's own arch
# (buildx `--push` can't also `--load`), which is what `aka pull` and
# src/admin/docker-compose.yml run; bin/build.sh runs every src/**/build.sh,
# including this one.

root="$(cd "$(dirname "$0")/../.." && pwd)"

# The FBL registry is the only default target, like src/aka/build.sh; set
# HUB_NAMESPACE to co-push the public mirror (Docker Hub) as well.
REGISTRY_NAMESPACE="${REGISTRY_NAMESPACE:-registry.freshbrewlabs.cloud/aka}"
NAMES=("$REGISTRY_NAMESPACE")
if [ -n "${HUB_NAMESPACE:-}" ]; then NAMES+=("$HUB_NAMESPACE"); fi
TAG="${TAG:-latest}"
PLATFORMS="${PLATFORMS:-linux/amd64,linux/arm64}"
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

# One or two names, two tags each.
tag_args=()
refs=()
for ns in "${NAMES[@]}"; do
    tag_args+=(-t "$ns/aka-admin:$TAG" -t "$ns/aka-admin:$VERSION")
    refs+=("$ns/aka-admin:$TAG" "$ns/aka-admin:$VERSION")
done

# The context is the repo root (see the Dockerfile), not this directory.
if [ "$PUSH" = 1 ]; then
    # buildx --push can't --load, so the local tags are a separate build
    docker buildx build --platform "$PLATFORMS" "${tag_args[@]}" --push \
        -f devops/docker/aka-admin/Dockerfile .
fi
docker build "${tag_args[@]}" -f devops/docker/aka-admin/Dockerfile .

if [ "$PUSH" = 1 ]; then
    echo "pushed ${refs[*]} for $PLATFORMS"
fi

echo "tagged ${refs[*]} locally ($(docker image inspect "$REGISTRY_NAMESPACE/aka-admin:$TAG" --format '{{.Os}}/{{.Architecture}}'))"
