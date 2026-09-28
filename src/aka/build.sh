#!/bin/bash
set -euo pipefail

# Build aka's managed docker images (tagged to match the defaults in
# aka_kernel::config — override config.proxy.image / dns.image to retarget),
# and publish them to Docker Hub as $HUB_NAMESPACE/<image>:$TAG for
# linux/amd64 + linux/arm64. Pushing requires `docker login`.
#
#   ./build.sh            # local tags + hub push
#   ./build.sh --no-push  # local tags only (bin/install.sh uses this)

HUB_NAMESPACE="${HUB_NAMESPACE:-dewey4iv}"
TAG="${TAG:-latest}"
PLATFORMS="${PLATFORMS:-linux/amd64,linux/arm64}"
PUSH=1

cd "$(dirname "$0")/../.."

for arg in "$@"; do
    case "$arg" in
        --no-push) PUSH=0 ;;
        *) echo "usage: $0 [--no-push]" >&2; exit 2 ;;
    esac
done

build() { # <name> <context>
    local name=$1 ctx=$2
    if [ "$PUSH" = 1 ]; then
        # buildx --push can't --load, so the hub image is a separate build
        docker buildx build --platform "$PLATFORMS" \
            -t "$HUB_NAMESPACE/$name:$TAG" --push "$ctx"
    fi
    docker build -t "$HUB_NAMESPACE/$name:$TAG" "$ctx"
}

build aka-proxy devops/docker/aka-proxy
build aka-dns devops/docker/aka-dns

if [ "$PUSH" = 1 ]; then
    echo "pushed $HUB_NAMESPACE/aka-proxy:$TAG and $HUB_NAMESPACE/aka-dns:$TAG ($PLATFORMS)"
fi
echo "tagged $HUB_NAMESPACE/aka-proxy:$TAG and $HUB_NAMESPACE/aka-dns:$TAG locally"
