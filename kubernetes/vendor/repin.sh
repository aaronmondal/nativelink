#!/usr/bin/env bash

# Tekton Operator
wget \
    https://storage.googleapis.com/tekton-releases/operator/previous/v0.75.0/release.yaml \
    -O tekton-operator/tekton-operator.yaml

# OpenTelemetry Operator
wget \
    https://github.com/open-telemetry/opentelemetry-operator/releases/download/v0.120.0/opentelemetry-operator.yaml \
    -O opentelemetry-operator/opentelemetry-operator.yaml

# Tempo Operator
wget \
    https://github.com/grafana/tempo-operator/releases/download/v0.15.3/tempo-operator.yaml \
    -O tempo-operator/tempo-operator.yaml
