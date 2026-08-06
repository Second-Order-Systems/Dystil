//! Isolated OTLP relay for Dystil's allowlisted operational telemetry.
//!
//! It intentionally decodes and re-encodes protobuf rather than proxying bytes.
//! Desktop logs are never accepted by this module.

use std::sync::Arc;
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::Response;
use opentelemetry_proto::tonic::collector::{
    metrics::v1::{ExportMetricsServiceRequest, ExportMetricsServiceResponse},
    trace::v1::{ExportTraceServiceRequest, ExportTraceServiceResponse},
};
use opentelemetry_proto::tonic::common::v1::{any_value, AnyValue, InstrumentationScope, KeyValue};
use opentelemetry_proto::tonic::metrics::v1::metric::Data as MetricData;
use opentelemetry_proto::tonic::metrics::v1::{
    number_data_point, AggregationTemporality, Gauge, Metric, NumberDataPoint, ResourceMetrics,
    ScopeMetrics, Sum,
};
use opentelemetry_proto::tonic::resource::v1::Resource;
use opentelemetry_proto::tonic::trace::v1::{span, ResourceSpans, ScopeSpans, Span};
use prost::Message;

use crate::{auth, AppError, AppState};

pub const MAX_REQUEST_BYTES: usize = 256 * 1024;
const MAX_METRIC_POINTS: usize = 2_000;
const MAX_SPANS: usize = 200;

#[derive(Debug, Clone)]
pub(crate) struct TelemetryRelayConfig {
    enabled: bool,
    openobserve_url: Option<String>,
    openobserve_org: String,
    authorization: Option<HeaderValue>,
    in_flight: Arc<tokio::sync::Semaphore>,
}

impl TelemetryRelayConfig {
    pub(crate) fn from_env() -> anyhow::Result<Self> {
        let enabled = std::env::var("TELEMETRY_RELAY_ENABLED")
            .map(|value| value != "false")
            .unwrap_or(false);
        let openobserve_url = std::env::var("TELEMETRY_OPENOBSERVE_URL")
            .ok()
            .map(|value| value.trim_end_matches('/').to_owned());
        let authorization = std::env::var("TELEMETRY_OPENOBSERVE_AUTHORIZATION")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .map(|value| HeaderValue::from_str(&value))
            .transpose()?;
        if enabled && (openobserve_url.is_none() || authorization.is_none()) {
            anyhow::bail!("TELEMETRY_RELAY_ENABLED requires OpenObserve URL and authorization");
        }
        Ok(Self {
            enabled,
            openobserve_url,
            openobserve_org: std::env::var("TELEMETRY_OPENOBSERVE_ORG")
                .unwrap_or_else(|_| "default".to_string()),
            authorization,
            in_flight: Arc::new(tokio::sync::Semaphore::new(
                std::env::var("TELEMETRY_RELAY_MAX_IN_FLIGHT")
                    .ok()
                    .and_then(|value| value.parse::<usize>().ok())
                    .filter(|value| *value > 0)
                    .unwrap_or(8),
            )),
        })
    }

    fn acquire(&self) -> Result<tokio::sync::OwnedSemaphorePermit, AppError> {
        self.in_flight
            .clone()
            .try_acquire_owned()
            .map_err(|_| AppError::TooManyRequests("telemetry relay is busy".into()))
    }

    #[cfg(test)]
    pub(crate) fn disabled_for_test() -> Self {
        Self {
            enabled: false,
            openobserve_url: None,
            openobserve_org: "default".to_string(),
            authorization: None,
            in_flight: Arc::new(tokio::sync::Semaphore::new(1)),
        }
    }
}

pub(crate) async fn post_metrics(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, AppError> {
    require_protobuf(&headers)?;
    let _permit = state.config.telemetry.acquire()?;
    auth::authenticate_device(&state, &headers).await?;
    let request = ExportMetricsServiceRequest::decode(body)
        .map_err(|_| AppError::BadRequest("invalid OTLP metrics protobuf".into()))?;
    let accepted = validate_metrics(request)?;
    forward(&state, "metrics", accepted.encode_to_vec()).await?;
    protobuf_response(ExportMetricsServiceResponse {
        partial_success: None,
    })
}

pub(crate) async fn post_traces(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, AppError> {
    require_protobuf(&headers)?;
    let _permit = state.config.telemetry.acquire()?;
    auth::authenticate_device(&state, &headers).await?;
    let request = ExportTraceServiceRequest::decode(body)
        .map_err(|_| AppError::BadRequest("invalid OTLP traces protobuf".into()))?;
    let accepted = validate_traces(request)?;
    forward(&state, "traces", accepted.encode_to_vec()).await?;
    protobuf_response(ExportTraceServiceResponse {
        partial_success: None,
    })
}

fn require_protobuf(headers: &HeaderMap) -> Result<(), AppError> {
    if headers.get(header::CONTENT_ENCODING).is_some() {
        return Err(AppError::BadRequest(
            "compressed telemetry is not accepted".into(),
        ));
    }
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok());
    if content_type != Some("application/x-protobuf") {
        return Err(AppError::BadRequest(
            "telemetry requires application/x-protobuf".into(),
        ));
    }
    Ok(())
}

fn validate_metrics(
    request: ExportMetricsServiceRequest,
) -> Result<ExportMetricsServiceRequest, AppError> {
    let mut points = 0usize;
    let resource_metrics = request
        .resource_metrics
        .into_iter()
        .map(|resource| {
            let attributes = validate_resource(resource.resource.as_ref())?;
            let scopes = resource
                .scope_metrics
                .into_iter()
                .map(|scope| {
                    validate_scope(scope.scope.as_ref())?;
                    let metrics = scope
                        .metrics
                        .into_iter()
                        .map(|metric| {
                            let allowed_attributes = metric_attributes(&metric.name)?;
                            if !metric.description.is_empty()
                                || !metric.metadata.is_empty()
                                || metric.unit != metric_unit(&metric.name)?
                            {
                                return Err(AppError::BadRequest(
                                    "telemetry metric metadata is not allowlisted".into(),
                                ));
                            }
                            let data = match metric.data {
                                Some(MetricData::Gauge(gauge)) if is_gauge(&metric.name) => {
                                    points += gauge.data_points.len();
                                    MetricData::Gauge(Gauge {
                                        data_points: gauge
                                            .data_points
                                            .into_iter()
                                            .map(|point| sanitize_point(point, allowed_attributes))
                                            .collect::<Result<_, _>>()?,
                                    })
                                }
                                Some(MetricData::Sum(sum))
                                    if !is_gauge(&metric.name)
                                        && sum.is_monotonic
                                        && sum.aggregation_temporality
                                            == AggregationTemporality::Delta as i32 =>
                                {
                                    points += sum.data_points.len();
                                    MetricData::Sum(Sum {
                                        data_points: sum
                                            .data_points
                                            .into_iter()
                                            .map(|point| sanitize_point(point, allowed_attributes))
                                            .collect::<Result<_, _>>()?,
                                        aggregation_temporality: AggregationTemporality::Delta
                                            as i32,
                                        is_monotonic: true,
                                    })
                                }
                                _ => {
                                    return Err(AppError::BadRequest(
                                        "telemetry metric type is not allowlisted".into(),
                                    ))
                                }
                            };
                            Ok(Metric {
                                name: metric.name,
                                description: String::new(),
                                unit: metric.unit,
                                metadata: Vec::new(),
                                data: Some(data),
                            })
                        })
                        .collect::<Result<Vec<_>, AppError>>()?;
                    Ok(ScopeMetrics {
                        scope: Some(InstrumentationScope {
                            name: "dystil.telemetry".into(),
                            version: scope
                                .scope
                                .and_then(|value| {
                                    if value.version.len() <= 64 {
                                        Some(value.version)
                                    } else {
                                        None
                                    }
                                })
                                .ok_or_else(|| {
                                    AppError::BadRequest("invalid telemetry scope".into())
                                })?,
                            attributes: Vec::new(),
                            dropped_attributes_count: 0,
                        }),
                        metrics,
                        schema_url: String::new(),
                    })
                })
                .collect::<Result<Vec<_>, AppError>>()?;
            Ok(ResourceMetrics {
                resource: Some(Resource {
                    attributes,
                    dropped_attributes_count: 0,
                    entity_refs: Vec::new(),
                }),
                scope_metrics: scopes,
                schema_url: String::new(),
            })
        })
        .collect::<Result<Vec<_>, AppError>>()?;
    if points > MAX_METRIC_POINTS {
        return Err(AppError::TooManyRequests(
            "too many telemetry metric points".into(),
        ));
    }
    Ok(ExportMetricsServiceRequest { resource_metrics })
}

fn validate_resource(resource: Option<&Resource>) -> Result<Vec<KeyValue>, AppError> {
    let resource =
        resource.ok_or_else(|| AppError::BadRequest("telemetry resource is required".into()))?;
    if resource.dropped_attributes_count != 0 || !resource.entity_refs.is_empty() {
        return Err(AppError::BadRequest(
            "telemetry resource metadata is not allowlisted".into(),
        ));
    }
    let mut output = Vec::with_capacity(resource.attributes.len());
    for attribute in &resource.attributes {
        let value = string_value(attribute)?;
        if !valid_resource_value(&attribute.key, value)
            || output
                .iter()
                .any(|existing: &KeyValue| existing.key == attribute.key)
        {
            return Err(AppError::BadRequest(
                "telemetry resource attribute is not allowlisted".into(),
            ));
        }
        output.push(string_attribute(&attribute.key, value));
    }
    if output
        .iter()
        .find(|value| value.key == "service.name")
        .is_none()
    {
        return Err(AppError::BadRequest(
            "telemetry service.name is required".into(),
        ));
    }
    Ok(output)
}

fn validate_scope(scope: Option<&InstrumentationScope>) -> Result<(), AppError> {
    let scope = scope.ok_or_else(|| AppError::BadRequest("telemetry scope is required".into()))?;
    if scope.name != "dystil.telemetry"
        || !scope.attributes.is_empty()
        || scope.dropped_attributes_count != 0
        || scope.version.is_empty()
        || scope.version.len() > 64
    {
        return Err(AppError::BadRequest(
            "telemetry scope is not allowlisted".into(),
        ));
    }
    Ok(())
}

fn sanitize_point(point: NumberDataPoint, allowed: &[&str]) -> Result<NumberDataPoint, AppError> {
    if point.exemplars.len() != 0 || point.flags != 0 || point.time_unix_nano == 0 {
        return Err(AppError::BadRequest(
            "telemetry point metadata is not allowlisted".into(),
        ));
    }
    let value = match point.value {
        Some(number_data_point::Value::AsInt(value)) if value >= 0 => value,
        _ => {
            return Err(AppError::BadRequest(
                "telemetry point value is invalid".into(),
            ))
        }
    };
    let mut attributes = Vec::with_capacity(point.attributes.len());
    for attribute in &point.attributes {
        let value = string_value(attribute)?;
        if !allowed.contains(&attribute.key.as_str())
            || !valid_metric_attribute(&attribute.key, value)
            || attributes
                .iter()
                .any(|existing: &KeyValue| existing.key == attribute.key)
        {
            return Err(AppError::BadRequest(
                "telemetry point attribute is not allowlisted".into(),
            ));
        }
        attributes.push(string_attribute(&attribute.key, value));
    }
    if attributes.len() != allowed.len() {
        return Err(AppError::BadRequest(
            "telemetry point attributes are incomplete".into(),
        ));
    }
    Ok(NumberDataPoint {
        attributes,
        start_time_unix_nano: 0,
        time_unix_nano: point.time_unix_nano,
        exemplars: Vec::new(),
        flags: 0,
        value: Some(number_data_point::Value::AsInt(value)),
    })
}

fn string_value(attribute: &KeyValue) -> Result<&str, AppError> {
    if attribute.key_strindex != 0 {
        return Err(AppError::BadRequest(
            "telemetry attribute key is invalid".into(),
        ));
    }
    match attribute
        .value
        .as_ref()
        .and_then(|value| value.value.as_ref())
    {
        Some(any_value::Value::StringValue(value)) if value.len() <= 128 => Ok(value),
        _ => Err(AppError::BadRequest(
            "telemetry attribute value is invalid".into(),
        )),
    }
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

fn metric_attributes(name: &str) -> Result<&'static [&'static str], AppError> {
    match name {
        "dystil.capture.triggers" => Ok(&["trigger.kind", "outcome", "reason.kind"]),
        "dystil.capture.images" => {
            Ok(&["trigger.kind", "outcome", "reason.kind", "capture.provider"])
        }
        "dystil.ai.operations" => Ok(&["provider.kind", "operation", "outcome", "error.kind"]),
        "dystil.app.starts" => Ok(&["start.reason", "outcome"]),
        "dystil.storage.operations" => Ok(&["operation", "outcome", "error.kind"]),
        "dystil.sync.iterations" => Ok(&["outcome", "policy.source", "error.kind"]),
        name if is_gauge(name) => Ok(&[]),
        _ => Err(AppError::BadRequest(
            "telemetry metric is not allowlisted".into(),
        )),
    }
}
fn metric_unit(name: &str) -> Result<&'static str, AppError> {
    match name {
        "dystil.capture.triggers" => Ok("{trigger}"),
        "dystil.capture.images" => Ok("{image}"),
        "dystil.ai.operations" => Ok("{operation}"),
        "dystil.app.starts" => Ok("{start}"),
        "dystil.storage.operations" => Ok("{operation}"),
        "dystil.sync.iterations" => Ok("{iteration}"),
        "dystil.process.cpu.utilization" | "dystil.host.cpu.utilization" => Ok("1"),
        name if is_gauge(name) => Ok("By"),
        _ => Err(AppError::BadRequest(
            "telemetry metric is not allowlisted".into(),
        )),
    }
}
fn is_gauge(name: &str) -> bool {
    matches!(
        name,
        "dystil.process.cpu.utilization"
            | "dystil.process.memory.rss"
            | "dystil.host.cpu.utilization"
            | "dystil.host.memory.available"
            | "dystil.storage.data.bytes"
            | "dystil.storage.available.bytes"
    )
}
fn valid_resource_value(key: &str, value: &str) -> bool {
    match key {
        "service.name" => value == "dystil-app",
        "service.version" => !value.is_empty() && value.len() <= 64,
        "deployment.environment.name" | "dystil.build_channel" => {
            matches!(value, "local" | "beta" | "prod")
        }
        "os.type" => matches!(value, "linux" | "macos" | "windows"),
        "host.arch" => matches!(value, "x86" | "x86_64" | "aarch64" | "arm"),
        "service.instance.id" => uuid::Uuid::parse_str(value).is_ok(),
        "dystil.telemetry.schema_version" => value == "1",
        _ => false,
    }
}
fn valid_metric_attribute(key: &str, value: &str) -> bool {
    match key {
        "trigger.kind" => matches!(
            value,
            "app_switch"
                | "window_focus"
                | "click"
                | "typing_pause"
                | "scroll_stop"
                | "key_press"
                | "clipboard"
                | "visual_change"
                | "idle"
                | "manual"
                | "activity_settled"
        ),
        "outcome" => matches!(value, "succeeded" | "failed" | "skipped"),
        "start.reason" => matches!(
            value,
            "launch" | "capture_initialization" | "previous_unclean_shutdown"
        ),
        "policy.source" => value == "background",
        "reason.kind" => matches!(
            value,
            "none"
                | "permission_denied"
                | "policy_disabled"
                | "provider_unavailable"
                | "timeout"
                | "rate_limited"
                | "unchanged"
                | "deduplicated"
                | "coalesced"
                | "no_evidence"
                | "queue_full"
                | "shutdown"
                | "storage"
                | "internal"
        ),
        "capture.provider" => matches!(
            value,
            "none"
                | "screen_capture_kit"
                | "windows_graphics_capture"
                | "xcap"
                | "wayshot"
                | "unknown"
        ),
        "provider.kind" => matches!(value, "codex" | "claude"),
        "operation" => matches!(
            value,
            "install"
                | "sign_in"
                | "connection_test"
                | "mcp_setup"
                | "mcp_connect"
                | "retention_cleanup"
                | "database_compaction"
                | "snapshot_cleanup"
        ),
        "error.kind" => matches!(
            value,
            "none"
                | "sidecar_missing"
                | "runtime_missing"
                | "timeout"
                | "login_required"
                | "authentication_failed"
                | "process_failed"
                | "invalid_output"
                | "filesystem"
                | "mcp_client_unavailable"
                | "mcp_registration_failed"
                | "authentication"
                | "permission_denied"
                | "policy_disabled"
                | "provider_unavailable"
                | "rate_limited"
                | "queue_full"
                | "database_busy"
                | "database"
                | "network"
                | "invalid_request"
                | "storage_full"
                | "storage"
                | "process_exit"
                | "cancelled"
                | "internal"
                | "unknown"
        ),
        _ => false,
    }
}

fn validate_traces(
    request: ExportTraceServiceRequest,
) -> Result<ExportTraceServiceRequest, AppError> {
    let mut span_count = 0usize;
    let resource_spans = request
        .resource_spans
        .into_iter()
        .map(|resource| {
            let attributes = validate_resource(resource.resource.as_ref())?;
            let scope_spans = resource
                .scope_spans
                .into_iter()
                .map(|scope| {
                    validate_scope(scope.scope.as_ref())?;
                    let spans = scope
                        .spans
                        .into_iter()
                        .map(|span| {
                            span_count += 1;
                            sanitize_span(span)
                        })
                        .collect::<Result<Vec<_>, AppError>>()?;
                    Ok(ScopeSpans {
                        scope: Some(InstrumentationScope {
                            name: "dystil.telemetry".into(),
                            version: scope
                                .scope
                                .and_then(|value| {
                                    if value.version.len() <= 64 {
                                        Some(value.version)
                                    } else {
                                        None
                                    }
                                })
                                .ok_or_else(|| {
                                    AppError::BadRequest("invalid telemetry scope".into())
                                })?,
                            attributes: Vec::new(),
                            dropped_attributes_count: 0,
                        }),
                        spans,
                        schema_url: String::new(),
                    })
                })
                .collect::<Result<Vec<_>, AppError>>()?;
            Ok(ResourceSpans {
                resource: Some(Resource {
                    attributes,
                    dropped_attributes_count: 0,
                    entity_refs: Vec::new(),
                }),
                scope_spans,
                schema_url: String::new(),
            })
        })
        .collect::<Result<Vec<_>, AppError>>()?;
    if span_count > MAX_SPANS {
        return Err(AppError::TooManyRequests("too many telemetry spans".into()));
    }
    Ok(ExportTraceServiceRequest { resource_spans })
}

fn sanitize_span(span: Span) -> Result<Span, AppError> {
    if !SPAN_NAMES.contains(&span.name.as_str())
        || span.trace_id.len() != 16
        || span.trace_id.iter().all(|byte| *byte == 0)
        || span.span_id.len() != 8
        || span.span_id.iter().all(|byte| *byte == 0)
        || !span.parent_span_id.is_empty()
        || !span.trace_state.is_empty()
        || span.kind != span::SpanKind::Internal as i32
        || span.flags != 1
        || span.start_time_unix_nano == 0
        || span.end_time_unix_nano < span.start_time_unix_nano
        || !span.attributes.is_empty()
        || span.dropped_attributes_count != 0
        || !span.events.is_empty()
        || span.dropped_events_count != 0
        || !span.links.is_empty()
        || span.dropped_links_count != 0
        || span.status.is_some()
    {
        return Err(AppError::BadRequest(
            "telemetry span is not allowlisted".into(),
        ));
    }
    Ok(Span {
        trace_id: span.trace_id,
        span_id: span.span_id,
        trace_state: String::new(),
        parent_span_id: Vec::new(),
        flags: 1,
        name: span.name,
        kind: span::SpanKind::Internal as i32,
        start_time_unix_nano: span.start_time_unix_nano,
        end_time_unix_nano: span.end_time_unix_nano,
        attributes: Vec::new(),
        dropped_attributes_count: 0,
        events: Vec::new(),
        dropped_events_count: 0,
        links: Vec::new(),
        dropped_links_count: 0,
        status: None,
    })
}

async fn forward(state: &AppState, signal: &str, body: Vec<u8>) -> Result<(), AppError> {
    let config = &state.config.telemetry;
    if !config.enabled {
        return Err(AppError::ServiceUnavailable(
            "telemetry relay is disabled".into(),
        ));
    }
    let base = config.openobserve_url.as_ref().expect("validated config");
    let authorization = config.authorization.as_ref().expect("validated config");
    let response = state
        .http
        .post(format!("{base}/api/{}/v1/{signal}", config.openobserve_org))
        .timeout(Duration::from_secs(5))
        .header(header::AUTHORIZATION, authorization)
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .header("stream-name", format!("dystil_{signal}"))
        .body(body)
        .send()
        .await
        .map_err(|_| AppError::BadGateway("telemetry upstream unavailable".into()))?;
    if !response.status().is_success() {
        return Err(AppError::BadGateway(
            "telemetry upstream rejected request".into(),
        ));
    }
    Ok(())
}

fn protobuf_response(message: impl Message) -> Result<Response, AppError> {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/x-protobuf")
        .body(axum::body::Body::from(message.encode_to_vec()))
        .map_err(|_| AppError::Internal("failed to build telemetry response".into()))
}

const SPAN_NAMES: &[&str] = &[
    "capture.session.start",
    "capture.session.stop",
    "dystil.failure.diagnostic",
];

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_metrics() -> ExportMetricsServiceRequest {
        ExportMetricsServiceRequest {
            resource_metrics: vec![ResourceMetrics {
                resource: Some(Resource {
                    attributes: vec![string_attribute("service.name", "dystil-app")],
                    dropped_attributes_count: 0,
                    entity_refs: Vec::new(),
                }),
                scope_metrics: vec![ScopeMetrics {
                    scope: Some(InstrumentationScope {
                        name: "dystil.telemetry".into(),
                        version: "0.0.7".into(),
                        attributes: Vec::new(),
                        dropped_attributes_count: 0,
                    }),
                    metrics: vec![Metric {
                        name: "dystil.capture.triggers".into(),
                        description: String::new(),
                        unit: "{trigger}".into(),
                        metadata: Vec::new(),
                        data: Some(MetricData::Sum(Sum {
                            data_points: vec![NumberDataPoint {
                                attributes: vec![
                                    string_attribute("trigger.kind", "click"),
                                    string_attribute("outcome", "succeeded"),
                                    string_attribute("reason.kind", "none"),
                                ],
                                start_time_unix_nano: 0,
                                time_unix_nano: 1,
                                exemplars: Vec::new(),
                                flags: 0,
                                value: Some(number_data_point::Value::AsInt(1)),
                            }],
                            aggregation_temporality: AggregationTemporality::Delta as i32,
                            is_monotonic: true,
                        })),
                    }],
                    schema_url: String::new(),
                }],
                schema_url: String::new(),
            }],
        }
    }

    #[test]
    fn reconstructs_a_valid_allowlisted_metric() {
        let accepted = validate_metrics(valid_metrics()).unwrap();
        let metric = &accepted.resource_metrics[0].scope_metrics[0].metrics[0];
        assert_eq!(metric.name, "dystil.capture.triggers");
        assert!(metric.metadata.is_empty());
    }

    #[test]
    fn rejects_an_arbitrary_attribute_before_forwarding() {
        let mut request = valid_metrics();
        request.resource_metrics[0].scope_metrics[0].metrics[0]
            .data
            .as_mut()
            .and_then(|data| match data {
                MetricData::Sum(sum) => Some(&mut sum.data_points[0]),
                _ => None,
            })
            .unwrap()
            .attributes
            .push(string_attribute("window.title", "private value"));
        assert!(validate_metrics(request).is_err());
    }
}
