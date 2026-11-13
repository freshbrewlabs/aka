#!/bin/bash
set -euo pipefail

# Ship the public landing page (src/public) to the freshbrewlabs k3s cluster.
#
#   bin/deploy.sh              # deploy the image tag the crates carry now
#   bin/deploy.sh 0.4.3        # deploy an explicit image tag
#   bin/deploy.sh --dry-run    # render check; nothing is applied
#
# The image must exist in Harbor first: CI publishes it on a v* tag
# (aka-public-build.yml), tagged with the aka-cli version this script defaults
# to. Deploys stay manual by policy (devops/woodpecker/README.md): no workflow
# touches a cluster.
#
# Needs: helm, kubectl (context fbl-k3s), and the decrypted production overlay
# src/public/helm/values-production.yaml — `devops/woodpecker/bin/decrypt`
# regenerates it from the committed .enc (keybase, team freshbrewlabs).

cd "$(dirname "$0")/.."

chart=src/public/helm
overlay=$chart/values-production.yaml

usage() {
    echo "usage: $(basename "$0") [--dry-run] [version]   (no version: the aka-cli crate version)"
}

dry_run=0
version=""
for arg in "$@"; do
    case "$arg" in
        --dry-run) dry_run=1 ;;
        -h|--help) usage; exit 0 ;;
        -*) echo "deploy.sh: unknown flag '$arg'" >&2; usage >&2; exit 2 ;;
        *)
            [ -z "$version" ] || {
                echo "deploy.sh: a single version argument, got '$version' and '$arg'" >&2
                exit 2
            }
            version="$arg"
            ;;
    esac
done

command -v helm >/dev/null || { echo "deploy.sh: helm is not installed" >&2; exit 1; }
command -v kubectl >/dev/null || { echo "deploy.sh: kubectl is not installed" >&2; exit 1; }

[ -f "$overlay" ] || {
    echo "deploy.sh: $overlay is missing — decrypt it first:" >&2
    echo "  devops/woodpecker/bin/decrypt   # keybase, team freshbrewlabs" >&2
    exit 1
}

# Single source of truth for the target namespace is the overlay.
namespace=$(awk '/^namespace:/ {print $2; exit}' "$overlay")
[ -n "$namespace" ] || {
    echo "deploy.sh: no 'namespace:' in $overlay" >&2
    exit 1
}

if [ -z "$version" ]; then
    version=$(bash bin/crate_version.sh aka-cli)
    echo "==> no version given; deploying the crates' version: $version"
fi

release=aka-public
args=(upgrade --install "$release" "$chart"
    --namespace "$namespace" --create-namespace
    -f "$overlay"
    --set image.tag="$version")

if [ "$dry_run" = 1 ]; then
    echo "==> dry run (client render, nothing applied)"
    helm "${args[@]}" --dry-run=client >/dev/null
    echo "==> render ok: helm, kubectl and $overlay all check out"
    exit 0
fi

echo "==> helm upgrade --install $release -> namespace $namespace, image tag $version"
helm "${args[@]}"

echo "==> waiting for the rollout"
if ! kubectl -n "$namespace" rollout status deployment/"$release" --timeout=180s; then
    echo "deploy.sh: rollout did not come up. Usual suspects:" >&2
    echo "  - image not published: registry.freshbrewlabs.cloud/aka/aka-public:$version" >&2
    echo "    (CI pushes it on a v* tag; bin/version.sh sets the number)" >&2
    echo "  - pull secret stale: re-run devops/woodpecker/bin/encrypt + decrypt" >&2
    echo "  - kubectl -n $namespace describe pod -l app=$release" >&2
    exit 1
fi

cat <<EOF

==> $release is up in $namespace (tag $version).

still manual, outside this repo:
  - the angie vhost for akacli.io (fbl-lab-devops, just apply rouge) — until it
    is applied the site answers only inside the cluster
  - smoke test: curl -fsS https://akacli.io | grep '<title>'
EOF
