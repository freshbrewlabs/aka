#!/bin/bash
set -euo pipefail

# Build aka's managed docker images (tagged to match the defaults in
# aka_kernel::config — override config.proxy.image / dns.image to retarget),
# and publish them to the FBL registry ($REGISTRY_NAMESPACE) for
# linux/amd64 + linux/arm64. Pushing requires `docker login` to every name
# published.
#
# Every image gets two tags: $TAG (`latest`, what the compose files default to)
# and the version of the `aka-cli` crate — so the image a
# container runs from carries the same number `aka version` prints. Both come
# from `bin/version.sh`, which moves every crate at once.
#
#   ./build.sh            # local tags + registry push
#   ./build.sh --no-push  # local tags only

# The FBL registry is the only default target: it's what src/aka/docker-compose.yml
# and src/admin/docker-compose.yml name. Docker Hub is opt-in — set
# HUB_NAMESPACE to co-push the public mirror (anonymous `aka pull`).
# Releases reach the hub through CI instead (the hub-mirror crane step in
# devops/woodpecker/workflows/), so local pushes rarely need HUB_NAMESPACE.
REGISTRY_NAMESPACE="${REGISTRY_NAMESPACE:-registry.freshbrewlabs.cloud/aka}"
NAMES=("$REGISTRY_NAMESPACE")
if [ -n "${HUB_NAMESPACE:-}" ]; then NAMES+=("$HUB_NAMESPACE"); fi
TAG="${TAG:-latest}"
PLATFORMS="${PLATFORMS:-linux/amd64,linux/arm64}"
PUSH=1

root="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$root"

for arg in "$@"; do
    case "$arg" in
        --no-push) PUSH=0 ;;
        *) echo "usage: $0 [--no-push]" >&2; exit 2 ;;
    esac
done

# The second tag: the version aka reports (`aka version`, healthcheck payloads)
# is the version its images carry. bin/version.sh moves every crate at once.
# $root, not a relative path: bin/build.sh runs this file from its own directory
VERSION="$(bash "$root/bin/crate_version.sh" aka-cli)"

build() { # <name> <context>
    local name=$1 ctx=$2
    local tags=() ns
    for ns in "${NAMES[@]}"; do
        tags+=(-t "$ns/$name:$TAG" -t "$ns/$name:$VERSION")
    done
    if [ "$PUSH" = 1 ]; then
        # buildx --push can't --load, so the local tags are a separate build
        docker buildx build --platform "$PLATFORMS" "${tags[@]}" --push "$ctx"
    fi
    docker build "${tags[@]}" "$ctx"
}

build aka-proxy devops/docker/aka-proxy
build aka-dns devops/docker/aka-dns

echo "tagged aka-proxy and aka-dns :$TAG + :$VERSION locally in ${NAMES[*]}"

if [ "$PUSH" = 1 ]; then
    echo "pushed both images for $PLATFORMS to ${NAMES[*]}"
fi
