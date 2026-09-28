#!/bin/bash
set -euo pipefail

# Build aka's managed docker images (tagged to match the defaults in
# aka_kernel::config — override config.proxy.image / dns.image to retarget).

cd "$(dirname "$0")/../.."

docker build -t aka-proxy:local devops/docker/aka-proxy
docker build -t aka-dns:local devops/docker/aka-dns

echo "built aka-proxy:local and aka-dns:local"
