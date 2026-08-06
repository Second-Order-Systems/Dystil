# Dystil desktop operational dashboard

`dystil-desktop-operational.json` is the checked-in OpenObserve v8 dashboard.
The Compose overlay provisions it automatically through a one-shot, private
`telemetry-dashboard` service. It communicates directly with OpenObserve over
the Docker network; it does not expose a dashboard-management route through
`dystil-api`.

To run or retry only that service after starting the overlay:

```bash
cd cloud
docker compose \
  -f docker-compose.yml \
  -f docker-compose.telemetry.yml \
  --env-file .env.docker \
  up --build telemetry-dashboard
```

Provisioning is idempotent: it leaves an existing dashboard unchanged. To
intentionally replace it, delete the old dashboard in OpenObserve and rerun
the command. Keep the dashboard JSON under version control whenever panels
change.

Use the **Metrics Explorer** and its metric picker. OpenObserve stores each
normalized metric name as its own stream. Do not put `service.instance.id` in a
dashboard group-by or variable.

| Panel | Visualization | Metric / grouping | Purpose |
| --- | --- | --- | --- |
| Capture outcomes | Time series, stacked | `dystil_capture_triggers`; `trigger_kind`, `outcome` | Capture activity and failure/skip mix without per-click events. |
| Image outcomes | Time series, stacked | `dystil_capture_images`; `outcome`, `capture_provider` | Image acquisition outcome and provider health. |
| Process CPU | Time series | `dystil_process_cpu_utilization` | Per-process resource use. |
| Dystil process RSS | Time series | `dystil_process_memory_rss` | Memory used by the Dystil process only. |
| Dystil process CPU | Time series | `dystil_process_cpu_utilization` | CPU used by the Dystil process only. |
| Dystil storage | Time series | `dystil_storage_data_bytes`, `dystil_storage_available_bytes` | Local Dystil data growth and available capacity. |
| Host context | Time series | `dystil_host_cpu_utilization` | Secondary machine-level context only. |

The dashboard uses friendly PromQL legends (`{trigger_kind} · {outcome}` and
`{outcome} · {capture_provider}`), and hides legends where one Dystil resource
series is sufficient. Its default time range is 24 hours.
Use the trace explorer separately on `dystil_traces`, filtering only fixed span
names `capture.session.start` and `capture.session.stop`. There must be no log
dashboard: the desktop does not ship logs.
