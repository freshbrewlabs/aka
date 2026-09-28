#!/bin/bash
set -euo pipefail

# Print one workspace crate's version. The image build scripts use it as the
# second tag (`bin/build.sh` publishes every image as `:latest` *and*
# `:<crate version>`), so a container's image tag, `aka version`, and the
# version the dashboard's healthcheck reports are the same number read from the
# same place — Cargo.toml, through cargo's own metadata.
#
#   ./bin/crate_version.sh aka-cli     # 0.1.0
#   ./bin/crate_version.sh admin-api   # 0.1.0

crate="${1:?usage: $(basename "$0") <crate>}"

cd "$(dirname "$0")/.."

version="$(cargo metadata --format-version 1 --no-deps |
    grep -o "\"name\":\"$crate\",\"version\":\"[^\"]*\"" |
    head -1 |
    sed -e 's/.*version":"//' -e 's/"$//')"

if [ -z "$version" ]; then
    echo "crate_version.sh: '$crate' is not a crate in this workspace" >&2
    echo "  members: $(cargo metadata --format-version 1 --no-deps |
        tr '{' '\n' |
        grep -o '"name":"[a-z0-9-]*","version' |
        sed -e 's/.*"name":"//' -e 's/","version//' |
        tr '\n' ' ')" >&2
    exit 1
fi

echo "$version"
