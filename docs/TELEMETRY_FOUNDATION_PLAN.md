# Telemetry foundation plan

## Status

**Initial implementation in progress.** The desktop has an always-on local aggregate recorder, a five-minute best-effort OTLP/HTTP metrics exporter, and 5% fixed-name lifecycle traces. The API relay, pinned OpenObserve Compose overlay, dashboard specification, and local validation guide are present. Desktop logs remain deliberately absent. A dashboard JSON export from the pinned running instance and full Docker/authenticated runtime validation remain rollout work.

## Goal

Add privacy-preserving operational telemetry to Dystil with no hosted-SaaS cost beyond the team's self-hosted server. Telemetry must never affect capture, local storage, sync, authentication, or normal product operation.

## Initial stack

```text
Dystil desktop app
        |
        | authenticated, sanitized OTLP/HTTP telemetry
        v
existing Nginx edge
        |
        v
existing dystil-api: /v1/telemetry relay
        |
        | server-only OpenObserve ingestion credential
        v
OpenObserve OSS (private network)
  - metrics
  - sampled traces
  - dashboards, trace explorer, search, and alerts
```

- **Backend:** OpenObserve OSS, single-node Docker deployment with persistent storage.
- **Proxy:** Existing Nginx. Do not introduce Caddy for telemetry.
- **Protocol:** OTLP/HTTP with protobuf payloads over HTTPS. OTLP JSON and public OTLP/gRPC are out of scope initially.
- **Signals at launch:** Metrics and sampled traces only.
- **Desktop logs:** Remain on-device; raw log shipping is explicitly out of scope.
- **Authentication:** OpenObserve ingestion token is held only by `dystil-api`. Desktop clients use their existing Dystil device authentication when calling the relay. Anonymous/local-only clients do not export telemetry in the first release.
- **OpenObserve exposure:** Its UI and ingestion port are private behind Nginx/the server network boundary; they are not directly reachable from desktop clients.
- **Versioning:** Pin OpenObserve and all telemetry SDK/container versions (prefer an image digest in production); never deploy `latest`.

## Relay boundary

The OTLP-compatible endpoints `/v1/telemetry/v1/metrics` and `/v1/telemetry/v1/traces` live in the existing API binary initially, but form an isolated best-effort module rather than part of the Dystil data plane. Clients configure `/v1/telemetry` as the OTLP base endpoint.

- It has no database writes and does not use the core database pool.
- It uses its own short-timeout HTTP client and concurrency semaphore. A bounded relay queue and per-device rate limit are deferred until observed load warrants them.
- Nginx must apply a dedicated request-size limit and per-IP rate limit before a public deployment.
- It accepts only `application/x-protobuf`, with no content encoding or `gzip`; unknown encodings are rejected.
- Initial implemented limits: 256 KiB request body, no compression, 2,000 metric points or 200 spans per request, and a configurable eight in-flight requests. Timestamp-window validation remains a rollout hardening item.
- The relay decodes OTLP, validates names/types/counts/timestamps and finite numeric values, constructs a new allowlisted OTLP payload, and only then forwards it. It never blindly proxies client bytes.
- Unknown resource attributes, metric names, span names, span attributes, scope names, and enum values are rejected. String fields have explicit byte-length limits.
- It never returns an OpenObserve response body to the client.
- Accepted payloads receive HTTP `200` with the appropriate empty protobuf `Export*ServiceResponse` after entering the bounded queue. A full/disabled queue receives `429`/`503`; clients use bounded retry with jitter and then drop.
- When its queue is full or OpenObserve is unavailable, telemetry is dropped. Dystil continues normally. There is no durable telemetry queue in the initial design.
- The relay has a server-side kill switch (`TELEMETRY_RELAY_ENABLED=false`) and a health/readiness status that does not make the core API unready.

Split the relay into a separate deployment only if telemetry creates meaningful resource contention, needs independent scaling/deployment, requires durable queuing, or Alloy becomes necessary as a standalone gateway.

## Privacy requirements

- Desktop operational telemetry is product-default on and has no settings UI in this phase. It remains strictly metadata-only and is limited to the reviewed schema below.
- An internal recorder gate remains available for an emergency product or policy disable. If disabled, it clears pending aggregates; a future exporter must also clear its in-memory queue.
- Startup measurements may be accumulated in primitive local variables and emitted through the same allowlisted schema.
- Server-side API operational telemetry is independent of desktop telemetry, but remains content-free and follows the same attribute allowlist.
- Telemetry is metadata-only and allowlisted at creation and at the relay.
- Never export screenshots, accessibility text, window titles, application names, URLs, paths, prompts, completions, work cards, evidence, raw errors, user email/name, auth credentials, or local database contents.
- Each app process generates a random, non-persisted `service.instance.id`. It is required to keep concurrent metric streams distinct, changes on every process launch, and must never be joined to an account/device ID or used for user tracking.
- Persistent device/user identifiers remain prohibited in all telemetry. Dashboards aggregate away `service.instance.id` unless investigating a single live session.
- Existing local rotating logs and crash files remain local support artifacts.
- Never use bare `#[instrument]` on functions that accept captured/user data. Instrumented Rust functions use `#[instrument(skip_all, fields(...))]` or manually created spans with allowlisted fields.
- Never call automatic error-recording helpers with arbitrary error/display strings. Map errors to the bounded `error.kind` taxonomy first.
- Existing PostHog calls must be disabled/removed from the initial telemetry path; do not identify users by email/name or run two diagnostic systems implicitly. Any future product analytics requires a separate consent and review.
- The existing `analyticsEnabled` value (currently defaulting true) does not control this operational telemetry and must not initialize a second diagnostic pipeline.

## Sampling and retention

- Capture-trigger and image metrics are aggregated locally. A click, app switch, or other trigger increments an in-memory counter; it never produces an individual network event or span.
- Export aggregate metric deltas every 5 minutes. Delta temporality prevents independently reporting desktops from colliding as cumulative counters and makes process restarts unambiguous. A dropped batch may lose its interval; this is acceptable for best-effort diagnostics.
- Resource gauges carry the per-process random `service.instance.id` and retain their slower cadence defined below.
- Keep the full local count of failures in aggregate metrics, but create a separate sanitized failure-diagnostic span at most once per `operation` + `error.kind` per desktop per 5 minutes. This is explicit application rate limiting, not outcome-based tail sampling.
- Trace slow capture batches and selected workflow boundaries, never individual clicks, app switches, frames, or images.
- Head-sample up to 10% of ordinary non-capture workflow attempts initially. Without a collector/tail sampler, the normal head-sampled span cannot be promoted after its outcome is known; failures use the separately rate-limited diagnostic span above.
- Retain metrics for 30 days and traces for 7 days initially.
- Do not export desktop logs at launch.
- Apply hard backend retention policies, not dashboard-time filters. Telemetry is disposable and is not included in content/database backups; back up only deployment configuration and dashboards.

### Initial volume guardrails

At 100 users, telemetry volume must be determined by export cadence and fixed metric cardinality, not by capture-event volume.

- Target roughly 50–100 active metric series per running desktop process, including its ephemeral `service.instance.id`.
- At a five-minute interval, this is roughly 1–3 million metric points per day across 100 users.
- Treat 0.25–1 GB/day as a planning envelope, not a promise; verify actual wire bytes, stored bytes, index overhead, and compaction behavior with a 5–10 user pilot.
- Provision 30–50 GB persistent storage for the pilot, alert at 70% usage, stop nonessential ingestion at 85%, and preserve service stability over retention at 90%. Resize and set final retention from measurements.

No dynamic metric labels are permitted other than the explicitly ephemeral `service.instance.id`. In particular, app/window names, URLs, persistent identifiers, error text, and arbitrary trigger values must never create new time series. The schema registry defines every allowed attribute key, enum value, and maximum cardinality; code review must reject ad hoc attributes.

## Deferred components

- Grafana Alloy / OpenTelemetry Collector gateway
- Grafana, Tempo, Loki, and Prometheus
- PostHog product analytics
- Sentry or another hosted/self-hosted crash tracker
- Desktop log shipping

Alloy is a future option when central filtering, tail sampling, buffering, multi-backend routing, or a backend migration warrants an additional gateway. It is not required for the first deployment.

## Visualization

OpenObserve is the initial visualization and alerting surface:

- metrics explorer and dashboards for rates, error ratios, and latency percentiles;
- trace explorer/timelines for slow and failed workflows;
- search for the allowed structured trace attributes;
- alerts for error-rate, latency, and relay-ingestion failures.

## Initial instrumentation catalog

Instrument workflow boundaries, not every Tauri command, UI event, frame, database query, or health-check tick. The app currently has many command entry points; recording a span for every one would add overhead and unhelpful noise.

### Shared attributes

All signals may contain only the following common resource attributes:

- `service.name` (`dystil-desktop` or `dystil-api`)
- `service.version`
- `deployment.environment.name`
- `dystil.build_channel`
- `os.type`
- `host.arch`
- `service.instance.id` (random per process; never persisted)

Operation-specific attributes must use bounded enumerations such as `component`, `operation`, `outcome`, `error.kind`, `provider.kind`, `capture.provider`, and `sync.mode`. User/device IDs, user-entered values, names, paths, URLs, and arbitrary error strings are prohibited.

Metric instrument names follow OpenTelemetry naming rules: lower-case dot-separated namespaces, no `_total`/`.total` suffix in the SDK, and a declared UCUM unit (`{event}`, `s`, `By`, or `1`). Prometheus-compatible backends may add `_total` when rendering monotonic counters.

### Desktop lifecycle and capture

| Boundary | Metrics | Trace/span | Safe attributes |
| --- | --- | --- | --- |
| Application start / recovery | `dystil.app.starts` | `app.start` | `start.reason`, `outcome` |
| Capture start, stop, pause, resume | `dystil.capture.sessions`, `dystil.capture.session.duration` | `capture.session.start`, `capture.session.stop` | `action`, `outcome`, `error.kind` |
| Capture provider lifecycle | `dystil.capture.provider.errors` | `capture.provider.initialize` | `capture.provider`, `outcome`, `error.kind` |
| Capture trigger aggregate | `dystil.capture.triggers` | none per trigger | `trigger.kind`, `outcome`, `reason.kind` |
| Image capture aggregate | `dystil.capture.images` | none per image | `trigger.kind`, `outcome`, `reason.kind`, `capture.provider` |
| Capture pipeline batch/flush | `dystil.capture.records`, `dystil.capture.batch.duration` | `capture.batch.process` | `lane`, `outcome`, `record.count.bucket` |
| Health status transition only | `dystil.health.transitions` | `capture.health.transition` | `from`, `to`, `reason.kind` |

Do not create a signal for every captured frame, UI event, permission poll, application, or window.

### Local data and privacy pipeline

| Boundary | Metrics | Trace/span | Safe attributes |
| --- | --- | --- | --- |
| Redaction batch | `dystil.redaction.operations`, `dystil.redaction.duration` | `redaction.batch` | `engine` (`deterministic`/`onnx`), `outcome`, `record.count.bucket` |
| SQLite operation boundary | `dystil.storage.operations`, `dystil.storage.operation.duration` | `storage.operation` | `operation`, `outcome`, `error.kind` |
| Retention/cleanup run | `dystil.retention.runs`, `dystil.retention.duration` | `retention.cleanup` | `outcome`, `deleted.count.bucket`, `error.kind` |

No SQL, paths, row contents, detector matches, or redaction inputs/outputs are exported.

### Desktop resource usage

| Boundary | Metrics | Cadence | Safe attributes |
| --- | --- | --- | --- |
| Dystil process resources | `dystil.process.cpu.utilization`, `dystil.process.memory.rss` | Every 15 minutes | none beyond shared resource attributes |
| Host resource headroom | `dystil.host.cpu.utilization`, `dystil.host.memory.available` | Every 15 minutes | none beyond shared resource attributes |
| Dystil data storage | `dystil.storage.data.bytes`, `dystil.storage.available.bytes` | Every 15 minutes | none beyond shared resource attributes |

Do not export host names, process lists, mount/volume names, filesystem paths, or other applications' usage. Storage values are charted as time series, so OpenObserve shows growth and cleanup over time without frequent reporting.

Resource collection must stay cheap: reuse the existing process/disk inspection paths, scan only Dystil-owned directories, do not follow symlinks outside those roots, cap scan time, and skip a sample rather than contending with capture or storage work.

### Local AI, retrieval, and insights

| Boundary | Metrics | Trace/span | Safe attributes |
| --- | --- | --- | --- |
| Model runtime lifecycle/health | `dystil.model.runtime.events` | `model.runtime.start` | `runtime.kind`, `outcome`, `error.kind` |
| Structured/automation model run | `dystil.model.requests`, `dystil.model.request.duration` | `model.structured.run`, `model.automation.run` | `runtime.kind`, `purpose.kind`, `outcome`, `error.kind`, `token.count.bucket` |
| Retrieval search | `dystil.retrieval.searches`, `dystil.retrieval.search.duration` | `retrieval.search` | `outcome`, `result.count.bucket`, `error.kind` |
| Explorer/steward batch | `dystil.insights.batches`, `dystil.insights.batch.duration` | `insights.explorer.batch`, `insights.steward.wake` | `outcome`, `evidence.count.bucket`, `error.kind` |

No prompt, completion, model endpoint, query text, result text, work-card data, evidence ID, or file content is exported.

### Optional sync and cloud API

| Boundary | Metrics | Trace/span | Safe attributes |
| --- | --- | --- | --- |
| Sync iteration | `dystil.sync.iterations`, `dystil.sync.iteration.duration` | `sync.once` | `outcome`, `policy.source`, `segments.count.bucket`, `images.count.bucket`, `error.kind` |
| Semantic sample pass | `dystil.sync.semantic_sample.runs` | `sync.semantic_sample.upload` | `outcome`, `uploaded.count.bucket`, `error.kind` |
| Existing API routes | `http.server.request.duration`, `http.server.requests` | HTTP server span | route template, method, status code, error kind |
| Telemetry relay | `dystil.telemetry.relay.requests`, `dystil.telemetry.relay.dropped`, `dystil.telemetry.relay.forward.duration` | `telemetry.relay.forward` | signal type, outcome, drop reason, payload-size bucket |

HTTP telemetry must use route templates only, never full URLs, query strings, authorization headers, request bodies, or device IDs.

## Schema and compatibility contract

- A versioned telemetry schema in code is the source of truth for metric/span names, instrument types, units, attribute keys, enum values, count buckets, and error taxonomy.
- Every payload includes `dystil.telemetry.schema_version`; the relay supports an explicit bounded set of versions during desktop upgrade windows and rejects unsupported versions.
- New optional fields are additive. Renames, instrument-type changes, unit changes, and enum removals require a schema-version change and a migration window.
- Numeric values must be finite and within per-instrument bounds. Histograms use explicit, reviewed bucket boundaries so SDK/backend defaults cannot create unexpected cardinality or incompatible dashboards.
- The relay does not trust client-provided `service.name`, deployment environment, or server-owned attributes; it overwrites them from the authenticated route and server configuration.
- Trace/span IDs are random correlation identifiers only. Baggage is disabled, and arbitrary trace-state values are not forwarded.

## Deployment hardening

- OpenObserve binds only to the private Docker/server network. Nginx exposes the authenticated Dystil relay, not OpenObserve ingestion.
- OpenObserve administrative/query UI is restricted to operators through the existing private access mechanism (VPN/SSH tunnel/access proxy); it is not public merely because ingestion uses Nginx.
- Store the OpenObserve ingestion token in the API deployment secret environment, never in Compose files, images, logs, desktop configuration, or frontend bundles. Rotate it without a desktop release.
- Use a dedicated OpenObserve organization/streams for Dystil telemetry and a credential that can ingest only. The relay never uses an administrative credential.
- Enable TLS at Nginx, request timeouts, request-body buffering limits, and access-log redaction for authorization headers and query strings.
- Apply container CPU/memory/PID limits, a health check, restart policy, and persistent-volume ownership/permissions. OpenObserve exhaustion must not starve `dystil-api` or Postgres.
- Encrypt the host volume at rest when the server platform supports it. Telemetry backups are disabled initially; configuration/dashboard backups contain no telemetry data or credentials.
- Pin a tested OpenObserve version and document upgrade/rollback steps before production deployment.

## Failure semantics and observability of telemetry

- Telemetry is never part of a product transaction and is never awaited on capture/UI critical paths.
- Desktop queues default to at most 1 MiB / three export batches and a 15-minute maximum age. Shutdown gets at most 500 ms to flush. Crash/offline loss is acceptable.
- The relay queue defaults to at most 16 MiB, forwarding concurrency 4, a 2-second connect timeout, and a 5-second total upstream timeout. These are configuration limits with conservative maximums, not unbounded tuning knobs.
- The relay queue exposes local counters for accepted, rejected, dropped, forwarded, and failed payloads plus queue bytes/age. These counters must also be visible in local service logs/health because OpenObserve cannot report its own outage reliably.
- Alerting inside OpenObserve can detect client/relay degradation only while OpenObserve is reachable. Host disk/container health needs the team's existing external host monitoring or a manual operational check in the initial deployment.
- Client and relay retries use capped exponential backoff with jitter and a maximum attempt/age budget. A persistent outage must not create a retry storm after recovery.
- Telemetry payloads and OpenObserve error bodies are never written to normal logs.

## Validation and rollout gates

Implementation is not production-ready until all of the following pass:

1. Unit tests prove the schema rejects every prohibited key category, unknown enums, non-finite/out-of-range numbers, oversized strings, bad timestamps, and unsupported schema versions.
2. Golden OTLP protobuf tests cover metrics/traces decode, validation, reconstruction, and OpenObserve-compatible forwarding.
3. Abuse tests cover compressed-size/decompressed-size limits, point/span count limits, invalid protobuf, unauthenticated clients, per-device/IP rate limits, and queue exhaustion.
4. Failure injection proves OpenObserve DNS failure, timeout, 4xx/5xx, full disk, relay disablement, and a full queue do not degrade core API health or desktop capture.
5. Performance tests show telemetry adds less than 1% sustained CPU, less than 25 MiB steady-state desktop memory, and no measurable capture-loop latency regression under the agreed workload.
6. A privacy fixture containing URLs, paths, prompts, window titles, emails, tokens, and raw errors produces none of those values in reconstructed OTLP payloads or OpenObserve.
7. Run a 5–10 opted-in-user pilot for at least 7 days. Measure real wire/stored volume, active series, trace count, relay drop rate, desktop overhead, query speed, and disk growth.
8. Production rollout is staged by build channel until the privacy review and pilot budgets pass. Both desktop export and relay ingestion have independently testable kill switches.

Initial promotion targets: relay-induced core API errors = 0; prohibited-field findings = 0; telemetry-related capture failures = 0; relay drop rate under healthy conditions <1%; measured storage fits the configured retention with at least 30% headroom.

## Implementation sequence

1. Create the versioned schema registry, safe enums/error mapper, no-op facade, and privacy/cardinality tests. **Implemented:** `crates/dystil-telemetry` provides the schema registry, bounded capture/provider/outcome/reason/error enums, default-on local aggregation, no-op recorder, concurrent delta aggregation, and privacy/cardinality tests. `dystil-capture` maps terminal capture/image outcomes into it; providers report `capture.provider=unknown` until each platform adapter is explicitly classified and reviewed.
2. Deploy pinned OpenObserve privately and provision retention, restricted ingestion credentials, operator access, disk safeguards, and baseline dashboards.
3. Implement and abuse-test the authenticated API relay and Nginx route while desktop export remains disabled.
4. Add the desktop OTLP exporter, ephemeral instance ID, and kill switch. **Implemented:** the exporter sends delta aggregate metrics every five minutes when a cloud-sync build embeds `DYSTIL_TELEMETRY_ENDPOINT`; it retains no durable queue. The resource sampler is implemented.
5. Instrument capture/image aggregate counters first, then resource gauges and the remaining workflow boundaries in small reviewed batches. **Implemented so far:** capture/image counters and 15-minute process CPU/RSS, host CPU/available memory, data-directory size, and free-storage gauges.
6. Run the pilot and promote only after every rollout gate passes.
