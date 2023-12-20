#!/usr/bin/env bash
# This script configures a cluster with a few standard deployments.

# TODO(aaronmondal): Add Grafana, OpenTelemetry and the various other standard
#                    deployments one would expect in a cluster.

set -xeuo pipefail

SRC_ROOT=$(git rev-parse --show-toplevel)

kubectl apply -f "$SRC_ROOT"/deployment-examples/kubernetes/gateway.yaml

IMAGE_TAG=$(nix eval .#image.imageTag --raw)

nix build .#image --print-build-logs --verbose \
    && ./result \
    | skopeo \
      copy \
      --dest-tls-verify=false \
      docker-archive:/dev/stdin \
      docker://localhost:5001/nativelink:local

IMAGE_TAG=$(nix eval .#lre.imageTag --raw)

echo "$IMAGE_TAG"

nix build .#lre --print-build-logs --verbose \
    && ./result \
    | skopeo \
      copy \
      --dest-tls-verify=false \
      docker-archive:/dev/stdin \
      docker://localhost:5001/nativelink-toolchain:local
