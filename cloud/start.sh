#!/bin/bash

cleanup() {
  kill $api_pid $auth_pid 2>/dev/null
  wait 2>/dev/null
}
trap cleanup SIGTERM SIGINT

echo "[dystil-api] launching auth service..."
cd /app/cloud/services/auth
bun src/start.ts &
auth_pid=$!

echo "[dystil-api] waiting for auth health..."
auth_healthy=false
for i in $(seq 1 30); do
  if ! kill -0 $auth_pid 2>/dev/null; then
    echo "[dystil-api] auth process died during startup"
    exit 1
  fi
  if curl -sf http://127.0.0.1:3001/health > /dev/null 2>&1; then
    echo "[dystil-api] auth healthy"
    auth_healthy=true
    break
  fi
  sleep 1
done

if [ "$auth_healthy" != true ]; then
  echo "[dystil-api] auth failed to become healthy within 30 seconds"
  kill $auth_pid 2>/dev/null
  exit 1
fi

echo "[dystil-api] launching API server..."
/usr/local/bin/work-insights-api &
api_pid=$!

set +e
wait -n
exit_code=$?
set -e

echo "[dystil-api] process exited (code=$exit_code), shutting down..."
cleanup
exit $exit_code
