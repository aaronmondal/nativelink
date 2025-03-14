#!/usr/bin/env bash

# Operator lifecycle manager
wget \
    https://github.com/operator-framework/operator-lifecycle-manager/releases/download/v0.31.0/crds.yaml \
    -O olm/crds.yaml
wget \
    https://github.com/operator-framework/operator-lifecycle-manager/releases/download/v0.31.0/olm.yaml \
    -O olm/olm.yaml

# Operators

# OpenTelemetry
wget \
    https://operatorhub.io/install/opentelemetry-operator.yaml \
    -O operators/opentelemetry-operator.yaml

# Grafana operator
wget \
    https://operatorhub.io/install/grafana-operator.yaml \
    -O operators/grafana-operator.yaml

# VictoriaMetrics and VictoriaLogs
wget \
    https://operatorhub.io/install/victoriametrics-operator.yaml \
    -O operators/victoriametrics-operator.yaml

# Tempo
wget \
    https://operatorhub.io/install/tempo-operator.yaml \
    -O operators/tempo-operator.yaml

# # Loki
# # TODO(aaronmondal): Probably just temporary until resolution of:
# # https://github.com/VictoriaMetrics/victorialogs-datasource/issues/265
# wget \
#     https://operatorhub.io/install/loki-operator.yaml \
#     -O operators/loki-operator.yaml

# Tekton Operator
# TODO(aaronmondal): Migrate to OLM once 0.75 releases
wget \
    https://storage.googleapis.com/tekton-releases/operator/previous/v0.75.0/release.yaml \
    -O tekton-operator/tekton-operator.yaml

pre-commit run -a
