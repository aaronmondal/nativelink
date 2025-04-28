#!/usr/bin/env bash
set -euo pipefail

### ─── Configuration ────────────────────────────────────────────────
# List each port‑forward as "<resource> <localPort>:<remotePort>"
# e.g. "svc/my-service1 8080:80" or "pod/my-pod 9090:90"
forwards=(
  "svc/vlogs-vlogs 9428"
  "svc/vmsingle-vmsingle 8429"
  "svc/tempo-tempo-jaegerui 16686"
)

echo "Running under: $SHELL ($BASH_VERSION)"

### ─── Internal State ───────────────────────────────────────────────
i=0
pids=()    # to capture background PIDs
labels=()  # to tag each line of output

### ─── Cleanup Handler ──────────────────────────────────────────────
cleanup() {
  echo
  echo "🛑 Shutting down port‑forwards..."
  for pid in "${pids[@]}"; do
    kill "$pid" 2>/dev/null || true
  done
  # wait for them to exit
  wait
  exit
}
trap cleanup SIGINT SIGTERM

### ─── Start Forwards ───────────────────────────────────────────────
for entry in "${forwards[@]}"; do
  i=$(( i + 1 ))
  # split into resource and ports
  read -r resource ports <<<"$entry"
  label="fwd#$i"
  labels+=("$label")
  echo "▶️ [$label] kubectl port‑forward $resource $ports"
  # launch and prefix each line with its label
  kubectl port-forward $resource $ports \
    2>&1 | sed "s/^/[$label] /" &
  pids+=($!)
done

### ─── Wait ────────────────────────────────────────────────────────
# now sit and let them run until signal
wait
