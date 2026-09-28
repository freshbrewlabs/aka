#!/bin/bash
set -euo pipefail

# Build aka's managed docker images (tagged to match the defaults in
# aka_kernel::config — override config.proxy.image / dns.image to retarget),
# and publish them to Docker Hub as $HUB_NAMESPACE/<image> for linux/amd64 +
# linux/arm64. Pushing requires `docker login`.
#
# Every image gets two tags: $TAG (`latest`, what the compose files default to)
# and the version of the `aka-cli` crate — so the image a
# container runs from carries the same number `aka version` prints. Both come
# from `bin/version.sh`, which moves every crate at once.
#
#   ./build.sh            # local tags + hub push
#   ./build.sh --no-push  # local tags only

HUB_NAMESPACE="${HUB_NAMESPACE:-dewey4iv}"
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
    if [ "$PUSH" = 1 ]; then
        # buildx --push can't --load, so the hub image is a separate build
        docker buildx build --platform "$PLATFORMS" \
            -t "$HUB_NAMESPACE/$name:$TAG" -t "$HUB_NAMESPACE/$name:$VERSION" \
            --push "$ctx"
    fi
    docker build -t "$HUB_NAMESPACE/$name:$TAG" -t "$HUB_NAMESPACE/$name:$VERSION" "$ctx"
}

build aka-proxy devops/docker/aka-proxy
build aka-dns devops/docker/aka-dns

echo "tagged $HUB_NAMESPACE/aka-proxy and $HUB_NAMESPACE/aka-dns :$TAG + :$VERSION locally"

if [ "$PUSH" = 1 ]; then
    echo "pushed both images for $PLATFORMS"
fi
