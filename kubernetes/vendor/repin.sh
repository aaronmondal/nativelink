#!/usr/bin/env bash

# Tekton Operator
wget \
    https://storage.googleapis.com/tekton-releases/operator/previous/v0.75.0/release.yaml \
    -O tekton-operator/tekton-operator.yaml


# Operator Lifecycle Manager
wget \
    https://github.com/operator-framework/operator-controller/releases/download/v1.2.0/operator-controller.yaml \
    -O olm/controller.yaml

wget \
    https://github.com/operator-framework/operator-controller/releases/download/v1.2.0/default-catalogs.yaml \
    -O olm-catalog/default-catalogs.yaml
