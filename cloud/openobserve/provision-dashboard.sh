#!/usr/bin/env bash
# Create the checked-in Dystil dashboard once in a local/self-hosted OpenObserve.
# This is executed by the private `telemetry-dashboard` Compose service. It
# never prints credentials. Re-running is safe: an existing dashboard is kept.
set -euo pipefail

base_url="${DYSTIL_OPENOBSERVE_URL:-http://127.0.0.1:5080}"
base_url="${base_url%/}"
org="${DYSTIL_OPENOBSERVE_ORG:-default}"
authorization="${DYSTIL_OPENOBSERVE_AUTHORIZATION:-${TELEMETRY_OPENOBSERVE_AUTHORIZATION:-}}"
dashboard_file="${DYSTIL_DASHBOARD_FILE:-$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)/dashboards/dystil-desktop-operational.json}"
dashboard_title="Dystil Desktop — Operational"

if [[ -z "$authorization" ]]; then
  echo "Set DYSTIL_OPENOBSERVE_AUTHORIZATION (or TELEMETRY_OPENOBSERVE_AUTHORIZATION) to a Basic authorization value." >&2
  exit 2
fi

for _attempt in $(seq 1 60); do
  if curl --fail --silent --show-error "$base_url/healthz" >/dev/null; then
    break
  fi
  sleep 1
done

if ! curl --fail --silent --show-error "$base_url/healthz" >/dev/null; then
  echo "OpenObserve did not become ready at $base_url within 60 seconds." >&2
  exit 1
fi

existing="$(curl --fail --silent --show-error \
  -H "Authorization: $authorization" \
  "$base_url/api/$org/dashboards")"

if jq -e --arg title "$dashboard_title" '.dashboards[]? | select(.title == $title)' \
  >/dev/null <<<"$existing"; then
  echo "Dashboard already exists: $dashboard_title"
  exit 0
fi

curl --fail-with-body --silent --show-error \
  -X POST \
  -H "Authorization: $authorization" \
  -H "Content-Type: application/json" \
  --data-binary "@$dashboard_file" \
  "$base_url/api/$org/dashboards" \
  | jq -r '"Created dashboard: " + .v8.title + " (id " + .v8.dashboardId + ")'
