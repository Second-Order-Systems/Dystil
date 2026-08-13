//! Best-effort OTLP/HTTP metrics exporter. It has no desktop log pipeline.
//!
//! Capture sites can only record bounded enums in `dystil-telemetry`; this is
//! the sole place those aggregates are translated into OTLP protobuf.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use dystil_telemetry::{
    schema, AiOperationPoint, CounterPoint, IntervalSnapshot, ResourceActivitySummary,
    ResourceSnapshot, SignalKind, SyncDiagnostics,
    StartupPoint, StorageOperationPoint, SyncIterationPoint, Telemetry, TelemetryRecorder,
};
use opentelemetry_proto::tonic::collector::metrics::v1::ExportMetricsServiceRequest;
use opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest;
use opentelemetry_proto::tonic::common::v1::{any_value, AnyValue, InstrumentationScope, KeyValue};
use opentelemetry_proto::tonic::metrics::v1::{
    metric, number_data_point, AggregationTemporality, Gauge, Metric, NumberDataPoint,
    ResourceMetrics, ScopeMetrics, Sum,
};
use opentelemetry_proto::tonic::resource::v1::Resource;
use opentelemetry_proto::tonic::trace::v1::{span, ResourceSpans, ScopeSpans, Span};
use prost::Message;
use tokio::task::JoinHandle;
use tokio::time::MissedTickBehavior;
use tracing::{info, warn};

const EXPORT_INTERVAL: Duration = Duration::from_secs(5 * 60);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
const INSTRUMENTATION_SCOPE: &str = "dystil.telemetry";

pub fn start(telemetry: Arc<Telemetry>, instance_id: String) -> Option<JoinHandle<()>> {
    let endpoint = crate::app_config::telemetry_endpoint()?
        .trim_end_matches('/')
        .to_owned();
    Some(tokio::spawn(async move {
        let Ok(client) = reqwest::Client::builder().timeout(REQUEST_TIMEOUT).build() else {
            return;
        };
        let mut interval = tokio::time::interval(EXPORT_INTERVAL);
        interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            let Some(snapshot) = telemetry.drain_interval() else {
                continue;
            };
            // Community builds may export anonymously. Enterprise/cloud builds
            // attach the device credential when one is available; telemetry
            // must not wait for authentication before attempting an export.
            let token = crate::auth::current_device_token().await.ok().flatten();
            if !telemetry.snapshot_is_current(&snapshot) {
                continue;
            }
            let metric_points = snapshot.points.len()
                + snapshot.ai_operations.len()
                + snapshot.app_starts.len()
                + snapshot.storage_operations.len()
                + snapshot.sync_iterations.len()
                + snapshot.sync_diagnostics.as_ref().map(|_| 7).unwrap_or_default()
                + snapshot
                    .resource_activity
                    .as_ref()
                    .map(resource_activity_metric_count)
                    .unwrap_or_default()
                + snapshot
                    .resources
                    .as_ref()
                    .map(resource_metric_count)
                    .unwrap_or_default();
            let trace_count = snapshot.traces.len();
            let trace_body = encode_traces(&snapshot, &instance_id);
            let body = encode_metrics(snapshot, &instance_id);
            // A failed export is intentionally best effort: no local queue and
            // no retry loop that could amplify traffic or retain old data.
            match build_export_request(
                &client,
                format!("{endpoint}/v1/metrics"),
                token.as_deref(),
                body,
            )
            .send()
            .await
            {
                Ok(response) if response.status().is_success() => {
                    info!(metric_points, "telemetry metrics export accepted")
                }
                Ok(response) => {
                    warn!(status = %response.status(), metric_points, "telemetry metrics export rejected")
                }
                Err(error) => warn!(%error, metric_points, "telemetry metrics export failed"),
            }
            if let Some(body) = trace_body {
                match build_export_request(
                    &client,
                    format!("{endpoint}/v1/traces"),
                    token.as_deref(),
                    body,
                )
                .send()
                .await
                {
                    Ok(response) if response.status().is_success() => {
                        info!(trace_count, "telemetry traces export accepted")
                    }
                    Ok(response) => {
                        warn!(status = %response.status(), trace_count, "telemetry traces export rejected")
                    }
                    Err(error) => warn!(%error, trace_count, "telemetry traces export failed"),
                }
            }
        }
    }))
}

fn build_export_request(
    client: &reqwest::Client,
    url: String,
    token: Option<&str>,
    body: Vec<u8>,
) -> reqwest::RequestBuilder {
    let request = client
        .post(url)
        .header("content-type", "application/x-protobuf");
    let request = match token {
        Some(token) => request.header("authorization", format!("Device {token}")),
        None => request,
    };
    request.body(body)
}

fn resource_metric_count(resources: &ResourceSnapshot) -> usize {
    [
        resources.process_cpu_percent_x100.is_some(),
        resources.process_memory_rss_bytes.is_some(),
        resources.host_cpu_percent_x100.is_some(),
        resources.host_memory_available_bytes.is_some(),
        resources.storage_data_bytes.is_some(),
        resources.storage_available_bytes.is_some(),
    ]
    .into_iter()
    .filter(|present| *present)
    .count()
}

fn resource_activity_metric_count(activity: &ResourceActivitySummary) -> usize {
    [
        activity.process_cpu_sync_average_x100.is_some(),
        activity.process_cpu_sync_max_x100.is_some(),
        activity.process_memory_sync_max_bytes.is_some(),
        activity.host_cpu_sync_average_x100.is_some(),
        activity.host_cpu_sync_max_x100.is_some(),
        activity.process_cpu_background_average_x100.is_some(),
        activity.process_cpu_background_max_x100.is_some(),
        activity.process_memory_background_max_bytes.is_some(),
        activity.host_cpu_background_average_x100.is_some(),
        activity.host_cpu_background_max_x100.is_some(),
    ]
    .into_iter()
    .filter(|value| *value)
    .count()
}

fn encode_traces(snapshot: &IntervalSnapshot, instance_id: &str) -> Option<Vec<u8>> {
    if snapshot.traces.is_empty() {
        return None;
    }
    let now = unix_nanos();
    let spans = snapshot
        .traces
        .iter()
        .map(|trace| {
            let trace_id = uuid::Uuid::new_v4().into_bytes().to_vec();
            let span_id = uuid::Uuid::new_v4().into_bytes()[..8].to_vec();
            Span {
                trace_id,
                span_id,
                trace_state: String::new(),
                parent_span_id: Vec::new(),
                flags: 1,
                name: trace.kind.as_str().to_string(),
                kind: span::SpanKind::Internal as i32,
                start_time_unix_nano: now,
                end_time_unix_nano: now,
                attributes: Vec::new(),
                dropped_attributes_count: 0,
                events: Vec::new(),
                dropped_events_count: 0,
                links: Vec::new(),
                dropped_links_count: 0,
                status: None,
            }
        })
        .collect();
    Some(
        ExportTraceServiceRequest {
            resource_spans: vec![ResourceSpans {
                resource: Some(Resource {
                    attributes: resource_attributes(snapshot.schema_version, instance_id),
                    dropped_attributes_count: 0,
                    entity_refs: Vec::new(),
                }),
                scope_spans: vec![ScopeSpans {
                    scope: Some(InstrumentationScope {
                        name: INSTRUMENTATION_SCOPE.to_string(),
                        version: env!("CARGO_PKG_VERSION").to_string(),
                        attributes: Vec::new(),
                        dropped_attributes_count: 0,
                    }),
                    spans,
                    schema_url: String::new(),
                }],
                schema_url: String::new(),
            }],
        }
        .encode_to_vec(),
    )
}

fn encode_metrics(snapshot: IntervalSnapshot, instance_id: &str) -> Vec<u8> {
    let now = unix_nanos();
    let mut metrics = snapshot
        .points
        .into_iter()
        .map(|point| counter_metric(point, now))
        .collect::<Vec<_>>();
    metrics.extend(
        snapshot
            .ai_operations
            .into_iter()
            .map(|point| ai_operation_metric(point, now)),
    );
    if let Some(diagnostics) = snapshot.sync_diagnostics {
        metrics.extend(sync_diagnostic_metrics(diagnostics, now));
    }
    metrics.extend(snapshot.app_starts.into_iter().map(|point| app_start_metric(point, now)));
    metrics.extend(
        snapshot
            .storage_operations
            .into_iter()
            .map(|point| storage_operation_metric(point, now)),
    );
    metrics.extend(
        snapshot
            .sync_iterations
            .into_iter()
            .map(|point| sync_iteration_metric(point, now)),
    );
    if let Some(resources) = snapshot.resources {
        metrics.extend(resource_metrics(resources, now));
    }
    if let Some(activity) = snapshot.resource_activity {
        metrics.extend(resource_activity_metrics(activity, now));
    }
    ExportMetricsServiceRequest {
        resource_metrics: vec![ResourceMetrics {
            resource: Some(Resource {
                attributes: resource_attributes(snapshot.schema_version, instance_id),
                dropped_attributes_count: 0,
                entity_refs: Vec::new(),
            }),
            scope_metrics: vec![ScopeMetrics {
                scope: Some(InstrumentationScope {
                    name: INSTRUMENTATION_SCOPE.to_string(),
                    version: env!("CARGO_PKG_VERSION").to_string(),
                    attributes: Vec::new(),
                    dropped_attributes_count: 0,
                }),
                metrics,
                schema_url: String::new(),
            }],
            schema_url: String::new(),
        }],
    }
    .encode_to_vec()
}

fn app_start_metric(point: StartupPoint, now: u64) -> Metric {
    sum_metric(
        schema::metric::APP_STARTS,
        "{start}",
        vec![
            string_attribute(schema::attribute::START_REASON, point.reason.as_str()),
            string_attribute(schema::attribute::OUTCOME, point.outcome.as_str()),
        ],
        point.value,
        now,
    )
}

fn storage_operation_metric(point: StorageOperationPoint, now: u64) -> Metric {
    sum_metric(
        schema::metric::STORAGE_OPERATIONS,
        "{operation}",
        vec![
            string_attribute(schema::attribute::OPERATION, point.operation.as_str()),
            string_attribute(schema::attribute::OUTCOME, point.outcome.as_str()),
            string_attribute(
                schema::attribute::ERROR_KIND,
                point.error.map_or("none", |error| error.as_str()),
            ),
        ],
        point.value,
        now,
    )
}

fn sync_iteration_metric(point: SyncIterationPoint, now: u64) -> Metric {
    sum_metric(
        schema::metric::SYNC_ITERATIONS,
        "{iteration}",
        vec![
            string_attribute(schema::attribute::OUTCOME, point.outcome.as_str()),
            string_attribute(schema::attribute::POLICY_SOURCE, "background"),
            string_attribute(
                schema::attribute::ERROR_KIND,
                point.error.map_or("none", |error| error.as_str()),
            ),
        ],
        point.value,
        now,
    )
}

fn sync_diagnostic_metrics(point: SyncDiagnostics, now: u64) -> Vec<Metric> {
    [
        (schema::metric::SYNC_ITERATION_DURATION, "ms", point.iteration_duration_ms),
        (schema::metric::SYNC_SEGMENT_DURATION, "ms", point.segment_duration_ms),
        (schema::metric::SYNC_IMAGE_DURATION, "ms", point.image_duration_ms),
        (schema::metric::SYNC_IMAGE_CANDIDATES_SCANNED, "{candidate}", point.image_candidates_scanned),
        (schema::metric::SYNC_IMAGE_CANDIDATES_SELECTED, "{candidate}", point.image_candidates_selected),
        (schema::metric::SYNC_IMAGES_PREPARED, "{image}", point.images_prepared),
        (schema::metric::SYNC_IMAGE_BYTES_PREPARED, "By", point.image_bytes_prepared),
    ]
    .into_iter()
    .map(|(name, unit, value)| sum_metric(name, unit, Vec::new(), value, now))
    .collect()
}

fn sum_metric(name: &str, unit: &str, attributes: Vec<KeyValue>, value: u64, now: u64) -> Metric {
    Metric {
        name: name.to_string(),
        description: String::new(),
        unit: unit.to_string(),
        metadata: Vec::new(),
        data: Some(metric::Data::Sum(Sum {
            data_points: vec![number_point(
                attributes,
                value.min(i64::MAX as u64) as i64,
                now,
            )],
            aggregation_temporality: AggregationTemporality::Delta as i32,
            is_monotonic: true,
        })),
    }
}

fn ai_operation_metric(point: AiOperationPoint, now: u64) -> Metric {
    Metric {
        name: schema::metric::AI_OPERATIONS.to_string(),
        description: String::new(),
        unit: "{operation}".to_string(),
        metadata: Vec::new(),
        data: Some(metric::Data::Sum(Sum {
            data_points: vec![number_point(
                vec![
                    string_attribute("provider.kind", point.provider.as_str()),
                    string_attribute("operation", point.operation.as_str()),
                    string_attribute("outcome", point.outcome.as_str()),
                    string_attribute("error.kind", point.error.as_str()),
                ],
                point.value.min(i64::MAX as u64) as i64,
                now,
            )],
            aggregation_temporality: AggregationTemporality::Delta as i32,
            is_monotonic: true,
        })),
    }
}

fn counter_metric(point: CounterPoint, now: u64) -> Metric {
    let name = match point.signal {
        SignalKind::CaptureTrigger => schema::metric::CAPTURE_TRIGGERS,
        SignalKind::ImageCapture => schema::metric::CAPTURE_IMAGES,
    };
    let mut attributes = vec![
        string_attribute(schema::attribute::TRIGGER_KIND, point.trigger.as_str()),
        string_attribute(schema::attribute::OUTCOME, point.outcome.as_str()),
        string_attribute(schema::attribute::REASON_KIND, point.reason.as_str()),
    ];
    if let Some(provider) = point.provider {
        attributes.push(string_attribute(
            schema::attribute::CAPTURE_PROVIDER,
            provider.as_str(),
        ));
    }
    Metric {
        name: name.to_string(),
        description: String::new(),
        unit: if matches!(point.signal, SignalKind::CaptureTrigger) {
            "{trigger}"
        } else {
            "{image}"
        }
        .to_string(),
        metadata: Vec::new(),
        data: Some(metric::Data::Sum(Sum {
            data_points: vec![number_point(attributes, point.value as i64, now)],
            aggregation_temporality: AggregationTemporality::Delta as i32,
            is_monotonic: true,
        })),
    }
}

fn resource_metrics(resources: ResourceSnapshot, now: u64) -> Vec<Metric> {
    [
        (
            schema::metric::PROCESS_CPU_UTILIZATION,
            "1",
            resources.process_cpu_percent_x100.map(|v| v as u64),
        ),
        (
            schema::metric::PROCESS_MEMORY_RSS,
            "By",
            resources.process_memory_rss_bytes,
        ),
        (
            schema::metric::HOST_CPU_UTILIZATION,
            "1",
            resources.host_cpu_percent_x100.map(|v| v as u64),
        ),
        (
            schema::metric::HOST_MEMORY_AVAILABLE,
            "By",
            resources.host_memory_available_bytes,
        ),
        (
            schema::metric::STORAGE_DATA_BYTES,
            "By",
            resources.storage_data_bytes,
        ),
        (
            schema::metric::STORAGE_AVAILABLE_BYTES,
            "By",
            resources.storage_available_bytes,
        ),
    ]
    .into_iter()
    .filter_map(|(name, unit, value)| value.map(|value| gauge_metric(name, unit, value, now)))
    .collect()
}

fn resource_activity_metrics(activity: ResourceActivitySummary, now: u64) -> Vec<Metric> {
    [
        (schema::metric::PROCESS_CPU_SYNC_AVERAGE, "1", activity.process_cpu_sync_average_x100.map(u64::from)),
        (schema::metric::PROCESS_CPU_SYNC_MAX, "1", activity.process_cpu_sync_max_x100.map(u64::from)),
        (schema::metric::PROCESS_MEMORY_SYNC_MAX, "By", activity.process_memory_sync_max_bytes),
        (schema::metric::HOST_CPU_SYNC_AVERAGE, "1", activity.host_cpu_sync_average_x100.map(u64::from)),
        (schema::metric::HOST_CPU_SYNC_MAX, "1", activity.host_cpu_sync_max_x100.map(u64::from)),
        (schema::metric::PROCESS_CPU_BACKGROUND_AVERAGE, "1", activity.process_cpu_background_average_x100.map(u64::from)),
        (schema::metric::PROCESS_CPU_BACKGROUND_MAX, "1", activity.process_cpu_background_max_x100.map(u64::from)),
        (schema::metric::PROCESS_MEMORY_BACKGROUND_MAX, "By", activity.process_memory_background_max_bytes),
        (schema::metric::HOST_CPU_BACKGROUND_AVERAGE, "1", activity.host_cpu_background_average_x100.map(u64::from)),
        (schema::metric::HOST_CPU_BACKGROUND_MAX, "1", activity.host_cpu_background_max_x100.map(u64::from)),
    ]
    .into_iter()
    .filter_map(|(name, unit, value)| value.map(|value| gauge_metric(name, unit, value, now)))
    .collect()
}

fn gauge_metric(name: &str, unit: &str, value: u64, now: u64) -> Metric {
    Metric {
        name: name.to_string(),
        description: String::new(),
        unit: unit.to_string(),
        metadata: Vec::new(),
        data: Some(metric::Data::Gauge(Gauge {
            data_points: vec![number_point(
                Vec::new(),
                value.min(i64::MAX as u64) as i64,
                now,
            )],
        })),
    }
}

fn number_point(attributes: Vec<KeyValue>, value: i64, now: u64) -> NumberDataPoint {
    NumberDataPoint {
        attributes,
        start_time_unix_nano: 0,
        time_unix_nano: now,
        exemplars: Vec::new(),
        flags: 0,
        value: Some(number_data_point::Value::AsInt(value)),
    }
}

fn resource_attributes(schema_version: u16, instance_id: &str) -> Vec<KeyValue> {
    vec![
        string_attribute(schema::resource_attribute::SERVICE_NAME, "dystil-app"),
        string_attribute(
            schema::resource_attribute::SERVICE_VERSION,
            env!("CARGO_PKG_VERSION"),
        ),
        string_attribute(
            schema::resource_attribute::DEPLOYMENT_ENVIRONMENT,
            if cfg!(debug_assertions) {
                "local"
            } else {
                "prod"
            },
        ),
        string_attribute(
            schema::resource_attribute::BUILD_CHANNEL,
            option_env!("DYSTIL_BUILD_CHANNEL").unwrap_or("local"),
        ),
        string_attribute(
            schema::resource_attribute::EDITION,
            if cfg!(feature = "enterprise-client") {
                "enterprise"
            } else {
                "community"
            },
        ),
        string_attribute(schema::resource_attribute::OS_TYPE, std::env::consts::OS),
        string_attribute(
            schema::resource_attribute::HOST_ARCH,
            std::env::consts::ARCH,
        ),
        string_attribute(schema::resource_attribute::SERVICE_INSTANCE_ID, instance_id),
        string_attribute(
            schema::resource_attribute::SCHEMA_VERSION,
            &schema_version.to_string(),
        ),
    ]
}

fn string_attribute(key: &str, value: &str) -> KeyValue {
    KeyValue {
        key: key.to_string(),
        value: Some(AnyValue {
            value: Some(any_value::Value::StringValue(value.to_string())),
        }),
        key_strindex: 0,
    }
}

fn unix_nanos() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .min(u64::MAX as u128) as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use dystil_telemetry::{
        CaptureTriggerKind, ConsentDecision, Outcome, ReasonKind, TraceKind,
        TELEMETRY_CONSENT_VERSION,
    };

    #[test]
    fn anonymous_export_has_no_authorization_header() {
        let client = reqwest::Client::new();
        let request = build_export_request(
            &client,
            "https://telemetry.invalid/v1/metrics".to_string(),
            None,
            vec![1, 2, 3],
        )
        .build()
        .unwrap();

        assert_eq!(
            request.headers().get("content-type").unwrap(),
            "application/x-protobuf"
        );
        assert!(request.headers().get("authorization").is_none());
    }

    #[test]
    fn authenticated_export_has_device_authorization_header() {
        let client = reqwest::Client::new();
        let request = build_export_request(
            &client,
            "https://telemetry.invalid/v1/metrics".to_string(),
            Some("device-token"),
            vec![1, 2, 3],
        )
        .build()
        .unwrap();

        assert_eq!(
            request.headers().get("authorization").unwrap(),
            "Device device-token"
        );
    }

    #[test]
    fn encodes_only_aggregate_metrics_and_allowlisted_resource_attributes() {
        let telemetry = Telemetry::new();
        telemetry.set_consent(ConsentDecision::Granted {
            policy_version: TELEMETRY_CONSENT_VERSION,
        });
        telemetry.record_capture_trigger(
            CaptureTriggerKind::Click,
            Outcome::Succeeded,
            ReasonKind::None,
        );
        telemetry.record_resource_snapshot(ResourceSnapshot {
            process_cpu_percent_x100: Some(120),
            process_memory_rss_bytes: None,
            host_cpu_percent_x100: None,
            host_memory_available_bytes: None,
            storage_data_bytes: None,
            storage_available_bytes: None,
        });
        telemetry.record_sampled_trace(TraceKind::CaptureSessionStart);
        let snapshot = telemetry.drain_interval().unwrap();
        let encoded = encode_metrics(snapshot.clone(), "ephemeral-instance");
        let decoded = ExportMetricsServiceRequest::decode(encoded.as_slice()).unwrap();
        let resource = &decoded.resource_metrics[0];
        assert_eq!(resource.scope_metrics[0].metrics.len(), 4);
        assert!(resource
            .resource
            .as_ref()
            .unwrap()
            .attributes
            .iter()
            .any(|attribute| attribute.key == schema::resource_attribute::SERVICE_INSTANCE_ID));
        assert!(encode_traces(&snapshot, "ephemeral-instance").is_some());
    }
}
