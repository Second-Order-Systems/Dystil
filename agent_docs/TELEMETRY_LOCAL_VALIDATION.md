---
status: unreviewed
verified_against: e84d34c
verified_on: 2026-08-08
---

> **Migrated, not yet audited.** This document predates the agent_docs split and has not been checked claim-by-claim against the code. Treat its specifics as unverified until someone confirms them and changes this header to `verified`.

# Local telemetry validation

This validates the actual path: desktop → authenticated `dystil-api` relay →
OpenObserve. It intentionally has no desktop log signal.

## 1. Start the server overlay

Copy the existing cloud environment file and add the values from
`.env.telemetry.example`. The OpenObserve authorization value is a Basic value
for the dedicated OpenObserve ingestion credential; it is only visible to
`dystil-api`.

Before exposing the API outside localhost, apply the companion
`cloud/nginx.telemetry.conf.example` to the existing TLS virtual host (with the
real upstream name). It exposes only the API relay, never OpenObserve.

```bash
cd cloud
docker compose -f docker-compose.yml -f docker-compose.telemetry.yml --env-file .env.docker up -d
```

Confirm OpenObserve is healthy from the server host:

```bash
curl -fsS http://127.0.0.1:5080/healthz
docker compose -f docker-compose.yml -f docker-compose.telemetry.yml logs dystil-api
```

The pinned OpenObserve image is distroless, so Docker cannot run a shell-based
container healthcheck inside it. The host `/healthz` request above is the
authoritative readiness check.

Validation note: the pinned `v0.90.3` image was checked locally. Its `/healthz`
endpoint returned `{"status":"ok"}`, and both protobuf OTLP endpoints
`/api/default/v1/metrics` and `/api/default/v1/traces` accepted the configured
`stream-name` headers. Its dashboard API reports export schema version 8.

OpenObserve is deliberately loopback-only. Reach its UI through the operator's
existing private access path, never from a desktop client.

## 2. Build a local cloud-capable desktop

Use your existing local API/auth stack to sign in once; that creates the device
token which the relay authenticates. Debug builds may point both endpoints at
local HTTP only on localhost. Release builds require HTTPS.

```bash
DYSTIL_CLOUD_BASE_URL=http://127.0.0.1:8089 \
DYSTIL_TELEMETRY_ENDPOINT=http://127.0.0.1:8089/v1/telemetry \
cargo run --manifest-path apps/dystil/src-tauri/Cargo.toml --features cloud-sync
```

This localhost HTTP exception is rejected for release builds and for every
non-loopback host.

## 3. Produce a safe test signal

Start and stop capture, then make a few normal capture triggers. Wait up to five
minutes for the exporter tick. The app sends only aggregate metric deltas and,
at a deterministic 5% sample, fixed-name `capture.session.start` / `.stop`
spans. It sends no screenshots, text, titles, URLs, paths, device ID, or logs.

## 4. Verify in OpenObserve

Open **Metrics Explorer**. OpenObserve normalizes dotted OTLP metric names and
stores each one as its own metrics stream. Use the stream picker to confirm:

- `dystil_capture_triggers` and `dystil_capture_images`, grouped by their
  bounded trigger/outcome labels;
- `dystil_process_cpu_utilization`, `dystil_process_memory_rss`,
  `dystil_host_cpu_utilization`, `dystil_host_memory_available`,
  `dystil_storage_data_bytes`, and `dystil_storage_available_bytes`.

Use the picker rather than assuming SQL field names. Select the **Traces**
explorer for sampled lifecycle traces.
The Compose overlay provisions the checked-in operational dashboard rather than
requiring hand-built raw charts. Its one-shot internal service uses the existing
OpenObserve credential over the Docker network; it does not add a privileged
dashboard route to `dystil-api`. It uses friendly legends and makes Dystil
process RSS/CPU primary; host signals are secondary context.

```bash
cd cloud
docker compose \
  -f docker-compose.yml \
  -f docker-compose.telemetry.yml \
  --env-file .env.docker \
  up --build telemetry-dashboard
```

Keep `service.instance.id` out of normal dashboard group-bys. The script does
not overwrite an existing dashboard; delete it deliberately before reprovisioning
an updated dashboard definition.

## 5. Negative checks

Send a request with an unknown metric, arbitrary attribute, JSON content type,
compression, or no `Authorization: Device …`; the relay must reject it. Search
OpenObserve for a known test URL, title, path, or token value; it must return no
rows. The relay does not expose an OTLP logs endpoint.

## Stop

```bash
cd cloud
docker compose -f docker-compose.yml -f docker-compose.telemetry.yml --env-file .env.docker down
```

`down` preserves the named OpenObserve volume. Use an explicit volume removal
only when deliberately discarding local telemetry.
