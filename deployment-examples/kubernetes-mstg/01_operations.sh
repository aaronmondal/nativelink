# This script configures a cluster with a few standard deployments.

# TODO(aaronmondal): Add Grafana, OpenTelemetry and the various other standard
#                    deployments one would expect in a cluster.

set -xeuo pipefail

SRC_ROOT=$(git rev-parse --show-toplevel)

kubectl apply -f ${SRC_ROOT}/deployment-examples/kubernetes-mstg/gateway.yaml

# The image for the scheduler and CAS.
nix run .#image.copyTo \
    docker://localhost:5001/nativelink:local \
    -- \
    --dest-tls-verify=false

# Wrap it with nativelink to turn it into a worker.
nix run .#nativelink-worker-mstg-rbe.copyTo \
    docker://localhost:5001/nativelink-worker-mstg-rbe:local \
    -- \
    --dest-tls-verify=false
