//! Versioned names and bounded values accepted by the telemetry pipeline.

pub const TELEMETRY_SCHEMA_VERSION: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstrumentKind {
    Counter,
    Histogram,
    Gauge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MetricSpec {
    pub name: &'static str,
    pub kind: InstrumentKind,
    pub unit: &'static str,
    pub attributes: &'static [&'static str],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpanSpec {
    pub name: &'static str,
    pub attributes: &'static [&'static str],
}

pub mod resource_attribute {
    pub const SERVICE_NAME: &str = "service.name";
    pub const SERVICE_VERSION: &str = "service.version";
    pub const DEPLOYMENT_ENVIRONMENT: &str = "deployment.environment.name";
    pub const BUILD_CHANNEL: &str = "dystil.build_channel";
    pub const OS_TYPE: &str = "os.type";
    pub const HOST_ARCH: &str = "host.arch";
    pub const SERVICE_INSTANCE_ID: &str = "service.instance.id";
    pub const SCHEMA_VERSION: &str = "dystil.telemetry.schema_version";
}

pub const RESOURCE_ATTRIBUTES: &[&str] = &[
    resource_attribute::SERVICE_NAME,
    resource_attribute::SERVICE_VERSION,
    resource_attribute::DEPLOYMENT_ENVIRONMENT,
    resource_attribute::BUILD_CHANNEL,
    resource_attribute::OS_TYPE,
    resource_attribute::HOST_ARCH,
    resource_attribute::SERVICE_INSTANCE_ID,
    resource_attribute::SCHEMA_VERSION,
];

pub mod attribute {
    pub const ACTION: &str = "action";
    pub const CAPTURE_PROVIDER: &str = "capture.provider";
    pub const DELETED_COUNT_BUCKET: &str = "deleted.count.bucket";
    pub const DROP_REASON: &str = "drop.reason";
    pub const ENGINE: &str = "engine";
    pub const ERROR_KIND: &str = "error.kind";
    pub const EVIDENCE_COUNT_BUCKET: &str = "evidence.count.bucket";
    pub const FROM: &str = "from";
    pub const IMAGES_COUNT_BUCKET: &str = "images.count.bucket";
    pub const LANE: &str = "lane";
    pub const OPERATION: &str = "operation";
    pub const OUTCOME: &str = "outcome";
    pub const PAYLOAD_SIZE_BUCKET: &str = "payload_size.bucket";
    pub const POLICY_SOURCE: &str = "policy.source";
    pub const PROVIDER_KIND: &str = "provider.kind";
    pub const PURPOSE_KIND: &str = "purpose.kind";
    pub const REASON_KIND: &str = "reason.kind";
    pub const RECORD_COUNT_BUCKET: &str = "record.count.bucket";
    pub const RESULT_COUNT_BUCKET: &str = "result.count.bucket";
    pub const RUNTIME_KIND: &str = "runtime.kind";
    pub const SEGMENTS_COUNT_BUCKET: &str = "segments.count.bucket";
    pub const SIGNAL_TYPE: &str = "signal.type";
    pub const START_REASON: &str = "start.reason";
    pub const STORAGE_CLASS: &str = "storage.class";
    pub const SYNC_MODE: &str = "sync.mode";
    pub const TO: &str = "to";
    pub const TOKEN_COUNT_BUCKET: &str = "token.count.bucket";
    pub const TRIGGER_KIND: &str = "trigger.kind";
    pub const UPLOADED_COUNT_BUCKET: &str = "uploaded.count.bucket";
}

pub mod metric {
    pub const APP_STARTS: &str = "dystil.app.starts";
    pub const CAPTURE_SESSIONS: &str = "dystil.capture.sessions";
    pub const CAPTURE_SESSION_DURATION: &str = "dystil.capture.session.duration";
    pub const CAPTURE_PROVIDER_ERRORS: &str = "dystil.capture.provider.errors";
    pub const CAPTURE_TRIGGERS: &str = "dystil.capture.triggers";
    pub const CAPTURE_IMAGES: &str = "dystil.capture.images";
    pub const AI_OPERATIONS: &str = "dystil.ai.operations";
    pub const CAPTURE_RECORDS: &str = "dystil.capture.records";
    pub const CAPTURE_BATCH_DURATION: &str = "dystil.capture.batch.duration";
    pub const HEALTH_TRANSITIONS: &str = "dystil.health.transitions";
    pub const REDACTION_OPERATIONS: &str = "dystil.redaction.operations";
    pub const REDACTION_DURATION: &str = "dystil.redaction.duration";
    pub const STORAGE_OPERATIONS: &str = "dystil.storage.operations";
    pub const STORAGE_OPERATION_DURATION: &str = "dystil.storage.operation.duration";
    pub const RETENTION_RUNS: &str = "dystil.retention.runs";
    pub const RETENTION_DURATION: &str = "dystil.retention.duration";
    pub const PROCESS_CPU_UTILIZATION: &str = "dystil.process.cpu.utilization";
    pub const PROCESS_MEMORY_RSS: &str = "dystil.process.memory.rss";
    pub const HOST_CPU_UTILIZATION: &str = "dystil.host.cpu.utilization";
    pub const HOST_MEMORY_AVAILABLE: &str = "dystil.host.memory.available";
    pub const STORAGE_DATA_BYTES: &str = "dystil.storage.data.bytes";
    pub const STORAGE_AVAILABLE_BYTES: &str = "dystil.storage.available.bytes";
    pub const PROCESS_CPU_SYNC_AVERAGE: &str = "dystil.process.cpu.sync.average";
    pub const PROCESS_CPU_SYNC_MAX: &str = "dystil.process.cpu.sync.max";
    pub const PROCESS_MEMORY_SYNC_MAX: &str = "dystil.process.memory.sync.max";
    pub const HOST_CPU_SYNC_AVERAGE: &str = "dystil.host.cpu.sync.average";
    pub const HOST_CPU_SYNC_MAX: &str = "dystil.host.cpu.sync.max";
    pub const PROCESS_CPU_BACKGROUND_AVERAGE: &str = "dystil.process.cpu.background.average";
    pub const PROCESS_CPU_BACKGROUND_MAX: &str = "dystil.process.cpu.background.max";
    pub const PROCESS_MEMORY_BACKGROUND_MAX: &str = "dystil.process.memory.background.max";
    pub const HOST_CPU_BACKGROUND_AVERAGE: &str = "dystil.host.cpu.background.average";
    pub const HOST_CPU_BACKGROUND_MAX: &str = "dystil.host.cpu.background.max";
    pub const MODEL_RUNTIME_EVENTS: &str = "dystil.model.runtime.events";
    pub const MODEL_REQUESTS: &str = "dystil.model.requests";
    pub const MODEL_REQUEST_DURATION: &str = "dystil.model.request.duration";
    pub const RETRIEVAL_SEARCHES: &str = "dystil.retrieval.searches";
    pub const RETRIEVAL_SEARCH_DURATION: &str = "dystil.retrieval.search.duration";
    pub const INSIGHTS_BATCHES: &str = "dystil.insights.batches";
    pub const INSIGHTS_BATCH_DURATION: &str = "dystil.insights.batch.duration";
    pub const SYNC_ITERATIONS: &str = "dystil.sync.iterations";
    pub const SYNC_ITERATION_DURATION: &str = "dystil.sync.iteration.duration";
    pub const SYNC_SEGMENT_DURATION: &str = "dystil.sync.segment.duration";
    pub const SYNC_IMAGE_DURATION: &str = "dystil.sync.image.duration";
    pub const SYNC_IMAGE_CANDIDATES_SCANNED: &str = "dystil.sync.image.candidates.scanned";
    pub const SYNC_IMAGE_CANDIDATES_SELECTED: &str = "dystil.sync.image.candidates.selected";
    pub const SYNC_IMAGES_PREPARED: &str = "dystil.sync.images.prepared";
    pub const SYNC_IMAGE_BYTES_PREPARED: &str = "dystil.sync.image.bytes.prepared";
    pub const SYNC_SEMANTIC_SAMPLE_RUNS: &str = "dystil.sync.semantic_sample.runs";
    pub const HTTP_SERVER_REQUEST_DURATION: &str = "http.server.request.duration";
    pub const HTTP_SERVER_REQUESTS: &str = "http.server.requests";
    pub const RELAY_REQUESTS: &str = "dystil.telemetry.relay.requests";
    pub const RELAY_DROPPED: &str = "dystil.telemetry.relay.dropped";
    pub const RELAY_FORWARD_DURATION: &str = "dystil.telemetry.relay.forward.duration";
}

pub mod span {
    pub const APP_START: &str = "app.start";
    pub const CAPTURE_SESSION_START: &str = "capture.session.start";
    pub const CAPTURE_SESSION_STOP: &str = "capture.session.stop";
    pub const CAPTURE_PROVIDER_INITIALIZE: &str = "capture.provider.initialize";
    pub const CAPTURE_BATCH_PROCESS: &str = "capture.batch.process";
    pub const CAPTURE_HEALTH_TRANSITION: &str = "capture.health.transition";
    pub const REDACTION_BATCH: &str = "redaction.batch";
    pub const STORAGE_OPERATION: &str = "storage.operation";
    pub const RETENTION_CLEANUP: &str = "retention.cleanup";
    pub const MODEL_RUNTIME_START: &str = "model.runtime.start";
    pub const MODEL_STRUCTURED_RUN: &str = "model.structured.run";
    pub const MODEL_AUTOMATION_RUN: &str = "model.automation.run";
    pub const RETRIEVAL_SEARCH: &str = "retrieval.search";
    pub const INSIGHTS_EXPLORER_BATCH: &str = "insights.explorer.batch";
    pub const INSIGHTS_STEWARD_WAKE: &str = "insights.steward.wake";
    pub const SYNC_ONCE: &str = "sync.once";
    pub const SYNC_SEMANTIC_SAMPLE_UPLOAD: &str = "sync.semantic_sample.upload";
    pub const TELEMETRY_RELAY_FORWARD: &str = "telemetry.relay.forward";
    pub const FAILURE_DIAGNOSTIC: &str = "dystil.failure.diagnostic";
}

const OUTCOME_REASON: &[&str] = &[attribute::OUTCOME, attribute::REASON_KIND];
const TRIGGER_OUTCOME_REASON: &[&str] = &[
    attribute::TRIGGER_KIND,
    attribute::OUTCOME,
    attribute::REASON_KIND,
];
const IMAGE_ATTRIBUTES: &[&str] = &[
    attribute::TRIGGER_KIND,
    attribute::OUTCOME,
    attribute::REASON_KIND,
    attribute::CAPTURE_PROVIDER,
];
const ERROR_ATTRIBUTES: &[&str] = &[attribute::OUTCOME, attribute::ERROR_KIND];
const NO_ATTRIBUTES: &[&str] = &[];

pub const METRICS: &[MetricSpec] = &[
    MetricSpec {
        name: metric::APP_STARTS,
        kind: InstrumentKind::Counter,
        unit: "{start}",
        attributes: &[attribute::START_REASON, attribute::OUTCOME],
    },
    MetricSpec {
        name: metric::CAPTURE_SESSIONS,
        kind: InstrumentKind::Counter,
        unit: "{session}",
        attributes: &[attribute::ACTION, attribute::OUTCOME, attribute::ERROR_KIND],
    },
    MetricSpec {
        name: metric::CAPTURE_SESSION_DURATION,
        kind: InstrumentKind::Histogram,
        unit: "s",
        attributes: &[attribute::ACTION, attribute::OUTCOME],
    },
    MetricSpec {
        name: metric::CAPTURE_PROVIDER_ERRORS,
        kind: InstrumentKind::Counter,
        unit: "{error}",
        attributes: &[attribute::CAPTURE_PROVIDER, attribute::ERROR_KIND],
    },
    MetricSpec {
        name: metric::CAPTURE_TRIGGERS,
        kind: InstrumentKind::Counter,
        unit: "{trigger}",
        attributes: TRIGGER_OUTCOME_REASON,
    },
    MetricSpec {
        name: metric::CAPTURE_IMAGES,
        kind: InstrumentKind::Counter,
        unit: "{image}",
        attributes: IMAGE_ATTRIBUTES,
    },
    MetricSpec {
        name: metric::CAPTURE_RECORDS,
        kind: InstrumentKind::Counter,
        unit: "{record}",
        attributes: &[attribute::LANE, attribute::OUTCOME],
    },
    MetricSpec {
        name: metric::CAPTURE_BATCH_DURATION,
        kind: InstrumentKind::Histogram,
        unit: "s",
        attributes: &[attribute::LANE, attribute::OUTCOME],
    },
    MetricSpec {
        name: metric::HEALTH_TRANSITIONS,
        kind: InstrumentKind::Counter,
        unit: "{transition}",
        attributes: &[attribute::FROM, attribute::TO, attribute::REASON_KIND],
    },
    MetricSpec {
        name: metric::REDACTION_OPERATIONS,
        kind: InstrumentKind::Counter,
        unit: "{operation}",
        attributes: &[attribute::ENGINE, attribute::OUTCOME],
    },
    MetricSpec {
        name: metric::REDACTION_DURATION,
        kind: InstrumentKind::Histogram,
        unit: "s",
        attributes: &[attribute::ENGINE, attribute::OUTCOME],
    },
    MetricSpec {
        name: metric::STORAGE_OPERATIONS,
        kind: InstrumentKind::Counter,
        unit: "{operation}",
        attributes: &[
            attribute::OPERATION,
            attribute::OUTCOME,
            attribute::ERROR_KIND,
        ],
    },
    MetricSpec {
        name: metric::STORAGE_OPERATION_DURATION,
        kind: InstrumentKind::Histogram,
        unit: "s",
        attributes: &[attribute::OPERATION, attribute::OUTCOME],
    },
    MetricSpec {
        name: metric::RETENTION_RUNS,
        kind: InstrumentKind::Counter,
        unit: "{run}",
        attributes: ERROR_ATTRIBUTES,
    },
    MetricSpec {
        name: metric::RETENTION_DURATION,
        kind: InstrumentKind::Histogram,
        unit: "s",
        attributes: OUTCOME_REASON,
    },
    MetricSpec {
        name: metric::PROCESS_CPU_UTILIZATION,
        kind: InstrumentKind::Gauge,
        unit: "1",
        attributes: NO_ATTRIBUTES,
    },
    MetricSpec {
        name: metric::PROCESS_MEMORY_RSS,
        kind: InstrumentKind::Gauge,
        unit: "By",
        attributes: NO_ATTRIBUTES,
    },
    MetricSpec {
        name: metric::HOST_CPU_UTILIZATION,
        kind: InstrumentKind::Gauge,
        unit: "1",
        attributes: NO_ATTRIBUTES,
    },
    MetricSpec {
        name: metric::HOST_MEMORY_AVAILABLE,
        kind: InstrumentKind::Gauge,
        unit: "By",
        attributes: NO_ATTRIBUTES,
    },
    MetricSpec {
        name: metric::STORAGE_DATA_BYTES,
        kind: InstrumentKind::Gauge,
        unit: "By",
        attributes: &[attribute::STORAGE_CLASS],
    },
    MetricSpec {
        name: metric::STORAGE_AVAILABLE_BYTES,
        kind: InstrumentKind::Gauge,
        unit: "By",
        attributes: NO_ATTRIBUTES,
    },
    MetricSpec { name: metric::PROCESS_CPU_SYNC_AVERAGE, kind: InstrumentKind::Gauge, unit: "1", attributes: NO_ATTRIBUTES },
    MetricSpec { name: metric::PROCESS_CPU_SYNC_MAX, kind: InstrumentKind::Gauge, unit: "1", attributes: NO_ATTRIBUTES },
    MetricSpec { name: metric::PROCESS_MEMORY_SYNC_MAX, kind: InstrumentKind::Gauge, unit: "By", attributes: NO_ATTRIBUTES },
    MetricSpec { name: metric::HOST_CPU_SYNC_AVERAGE, kind: InstrumentKind::Gauge, unit: "1", attributes: NO_ATTRIBUTES },
    MetricSpec { name: metric::HOST_CPU_SYNC_MAX, kind: InstrumentKind::Gauge, unit: "1", attributes: NO_ATTRIBUTES },
    MetricSpec { name: metric::PROCESS_CPU_BACKGROUND_AVERAGE, kind: InstrumentKind::Gauge, unit: "1", attributes: NO_ATTRIBUTES },
    MetricSpec { name: metric::PROCESS_CPU_BACKGROUND_MAX, kind: InstrumentKind::Gauge, unit: "1", attributes: NO_ATTRIBUTES },
    MetricSpec { name: metric::PROCESS_MEMORY_BACKGROUND_MAX, kind: InstrumentKind::Gauge, unit: "By", attributes: NO_ATTRIBUTES },
    MetricSpec { name: metric::HOST_CPU_BACKGROUND_AVERAGE, kind: InstrumentKind::Gauge, unit: "1", attributes: NO_ATTRIBUTES },
    MetricSpec { name: metric::HOST_CPU_BACKGROUND_MAX, kind: InstrumentKind::Gauge, unit: "1", attributes: NO_ATTRIBUTES },
    MetricSpec {
        name: metric::MODEL_RUNTIME_EVENTS,
        kind: InstrumentKind::Counter,
        unit: "{event}",
        attributes: &[
            attribute::RUNTIME_KIND,
            attribute::OUTCOME,
            attribute::ERROR_KIND,
        ],
    },
    MetricSpec {
        name: metric::MODEL_REQUESTS,
        kind: InstrumentKind::Counter,
        unit: "{request}",
        attributes: &[
            attribute::RUNTIME_KIND,
            attribute::PURPOSE_KIND,
            attribute::OUTCOME,
            attribute::ERROR_KIND,
        ],
    },
    MetricSpec {
        name: metric::MODEL_REQUEST_DURATION,
        kind: InstrumentKind::Histogram,
        unit: "s",
        attributes: &[
            attribute::RUNTIME_KIND,
            attribute::PURPOSE_KIND,
            attribute::OUTCOME,
        ],
    },
    MetricSpec {
        name: metric::RETRIEVAL_SEARCHES,
        kind: InstrumentKind::Counter,
        unit: "{search}",
        attributes: ERROR_ATTRIBUTES,
    },
    MetricSpec {
        name: metric::RETRIEVAL_SEARCH_DURATION,
        kind: InstrumentKind::Histogram,
        unit: "s",
        attributes: &[attribute::OUTCOME],
    },
    MetricSpec {
        name: metric::INSIGHTS_BATCHES,
        kind: InstrumentKind::Counter,
        unit: "{batch}",
        attributes: &[
            attribute::OPERATION,
            attribute::OUTCOME,
            attribute::ERROR_KIND,
        ],
    },
    MetricSpec {
        name: metric::INSIGHTS_BATCH_DURATION,
        kind: InstrumentKind::Histogram,
        unit: "s",
        attributes: &[attribute::OPERATION, attribute::OUTCOME],
    },
    MetricSpec {
        name: metric::SYNC_ITERATIONS,
        kind: InstrumentKind::Counter,
        unit: "{iteration}",
        attributes: &[
            attribute::OUTCOME,
            attribute::POLICY_SOURCE,
            attribute::ERROR_KIND,
        ],
    },
    MetricSpec {
        name: metric::SYNC_ITERATION_DURATION,
        kind: InstrumentKind::Histogram,
        unit: "ms",
        attributes: &[attribute::OUTCOME, attribute::POLICY_SOURCE],
    },
    MetricSpec { name: metric::SYNC_SEGMENT_DURATION, kind: InstrumentKind::Counter, unit: "ms", attributes: NO_ATTRIBUTES },
    MetricSpec { name: metric::SYNC_IMAGE_DURATION, kind: InstrumentKind::Counter, unit: "ms", attributes: NO_ATTRIBUTES },
    MetricSpec { name: metric::SYNC_IMAGE_CANDIDATES_SCANNED, kind: InstrumentKind::Counter, unit: "{candidate}", attributes: NO_ATTRIBUTES },
    MetricSpec { name: metric::SYNC_IMAGE_CANDIDATES_SELECTED, kind: InstrumentKind::Counter, unit: "{candidate}", attributes: NO_ATTRIBUTES },
    MetricSpec { name: metric::SYNC_IMAGES_PREPARED, kind: InstrumentKind::Counter, unit: "{image}", attributes: NO_ATTRIBUTES },
    MetricSpec { name: metric::SYNC_IMAGE_BYTES_PREPARED, kind: InstrumentKind::Counter, unit: "By", attributes: NO_ATTRIBUTES },
    MetricSpec {
        name: metric::SYNC_SEMANTIC_SAMPLE_RUNS,
        kind: InstrumentKind::Counter,
        unit: "{run}",
        attributes: ERROR_ATTRIBUTES,
    },
    MetricSpec {
        name: metric::HTTP_SERVER_REQUEST_DURATION,
        kind: InstrumentKind::Histogram,
        unit: "s",
        attributes: &[
            "http.request.method",
            "http.route",
            "http.response.status_code",
            attribute::ERROR_KIND,
        ],
    },
    MetricSpec {
        name: metric::HTTP_SERVER_REQUESTS,
        kind: InstrumentKind::Counter,
        unit: "{request}",
        attributes: &[
            "http.request.method",
            "http.route",
            "http.response.status_code",
            attribute::ERROR_KIND,
        ],
    },
    MetricSpec {
        name: metric::RELAY_REQUESTS,
        kind: InstrumentKind::Counter,
        unit: "{request}",
        attributes: &[attribute::SIGNAL_TYPE, attribute::OUTCOME],
    },
    MetricSpec {
        name: metric::RELAY_DROPPED,
        kind: InstrumentKind::Counter,
        unit: "{request}",
        attributes: &[attribute::SIGNAL_TYPE, attribute::DROP_REASON],
    },
    MetricSpec {
        name: metric::RELAY_FORWARD_DURATION,
        kind: InstrumentKind::Histogram,
        unit: "s",
        attributes: &[
            attribute::SIGNAL_TYPE,
            attribute::OUTCOME,
            attribute::PAYLOAD_SIZE_BUCKET,
        ],
    },
];

pub const SPANS: &[SpanSpec] = &[
    SpanSpec {
        name: span::APP_START,
        attributes: &[attribute::START_REASON, attribute::OUTCOME],
    },
    SpanSpec {
        name: span::CAPTURE_SESSION_START,
        attributes: ERROR_ATTRIBUTES,
    },
    SpanSpec {
        name: span::CAPTURE_SESSION_STOP,
        attributes: ERROR_ATTRIBUTES,
    },
    SpanSpec {
        name: span::CAPTURE_PROVIDER_INITIALIZE,
        attributes: &[
            attribute::CAPTURE_PROVIDER,
            attribute::OUTCOME,
            attribute::ERROR_KIND,
        ],
    },
    SpanSpec {
        name: span::CAPTURE_BATCH_PROCESS,
        attributes: &[
            attribute::LANE,
            attribute::OUTCOME,
            attribute::RECORD_COUNT_BUCKET,
            attribute::ERROR_KIND,
        ],
    },
    SpanSpec {
        name: span::CAPTURE_HEALTH_TRANSITION,
        attributes: &[attribute::FROM, attribute::TO, attribute::REASON_KIND],
    },
    SpanSpec {
        name: span::REDACTION_BATCH,
        attributes: &[
            attribute::ENGINE,
            attribute::OUTCOME,
            attribute::RECORD_COUNT_BUCKET,
            attribute::ERROR_KIND,
        ],
    },
    SpanSpec {
        name: span::STORAGE_OPERATION,
        attributes: &[
            attribute::OPERATION,
            attribute::OUTCOME,
            attribute::ERROR_KIND,
        ],
    },
    SpanSpec {
        name: span::RETENTION_CLEANUP,
        attributes: &[
            attribute::OUTCOME,
            attribute::DELETED_COUNT_BUCKET,
            attribute::ERROR_KIND,
        ],
    },
    SpanSpec {
        name: span::MODEL_RUNTIME_START,
        attributes: &[
            attribute::RUNTIME_KIND,
            attribute::OUTCOME,
            attribute::ERROR_KIND,
        ],
    },
    SpanSpec {
        name: span::MODEL_STRUCTURED_RUN,
        attributes: &[
            attribute::RUNTIME_KIND,
            attribute::PURPOSE_KIND,
            attribute::OUTCOME,
            attribute::ERROR_KIND,
            attribute::TOKEN_COUNT_BUCKET,
        ],
    },
    SpanSpec {
        name: span::MODEL_AUTOMATION_RUN,
        attributes: &[
            attribute::RUNTIME_KIND,
            attribute::PURPOSE_KIND,
            attribute::OUTCOME,
            attribute::ERROR_KIND,
            attribute::TOKEN_COUNT_BUCKET,
        ],
    },
    SpanSpec {
        name: span::RETRIEVAL_SEARCH,
        attributes: &[
            attribute::OUTCOME,
            attribute::RESULT_COUNT_BUCKET,
            attribute::ERROR_KIND,
        ],
    },
    SpanSpec {
        name: span::INSIGHTS_EXPLORER_BATCH,
        attributes: &[
            attribute::OUTCOME,
            attribute::EVIDENCE_COUNT_BUCKET,
            attribute::ERROR_KIND,
        ],
    },
    SpanSpec {
        name: span::INSIGHTS_STEWARD_WAKE,
        attributes: &[
            attribute::OUTCOME,
            attribute::EVIDENCE_COUNT_BUCKET,
            attribute::ERROR_KIND,
        ],
    },
    SpanSpec {
        name: span::SYNC_ONCE,
        attributes: &[
            attribute::OUTCOME,
            attribute::POLICY_SOURCE,
            attribute::SEGMENTS_COUNT_BUCKET,
            attribute::IMAGES_COUNT_BUCKET,
            attribute::ERROR_KIND,
        ],
    },
    SpanSpec {
        name: span::SYNC_SEMANTIC_SAMPLE_UPLOAD,
        attributes: &[
            attribute::OUTCOME,
            attribute::UPLOADED_COUNT_BUCKET,
            attribute::ERROR_KIND,
        ],
    },
    SpanSpec {
        name: span::TELEMETRY_RELAY_FORWARD,
        attributes: &[
            attribute::SIGNAL_TYPE,
            attribute::OUTCOME,
            attribute::PAYLOAD_SIZE_BUCKET,
        ],
    },
    SpanSpec {
        name: span::FAILURE_DIAGNOSTIC,
        attributes: &[attribute::OPERATION, attribute::ERROR_KIND],
    },
];

pub fn metric_spec(name: &str) -> Option<&'static MetricSpec> {
    METRICS.iter().find(|spec| spec.name == name)
}

pub fn span_spec(name: &str) -> Option<&'static SpanSpec> {
    SPANS.iter().find(|spec| spec.name == name)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(usize)]
pub enum CaptureTriggerKind {
    AppSwitch,
    WindowFocus,
    Click,
    TypingPause,
    ScrollStop,
    KeyPress,
    Clipboard,
    VisualChange,
    Idle,
    Manual,
    ActivitySettled,
}

impl CaptureTriggerKind {
    pub const COUNT: usize = 11;
    pub const ALL: [Self; Self::COUNT] = [
        Self::AppSwitch,
        Self::WindowFocus,
        Self::Click,
        Self::TypingPause,
        Self::ScrollStop,
        Self::KeyPress,
        Self::Clipboard,
        Self::VisualChange,
        Self::Idle,
        Self::Manual,
        Self::ActivitySettled,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AppSwitch => "app_switch",
            Self::WindowFocus => "window_focus",
            Self::Click => "click",
            Self::TypingPause => "typing_pause",
            Self::ScrollStop => "scroll_stop",
            Self::KeyPress => "key_press",
            Self::Clipboard => "clipboard",
            Self::VisualChange => "visual_change",
            Self::Idle => "idle",
            Self::Manual => "manual",
            Self::ActivitySettled => "activity_settled",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(usize)]
pub enum Outcome {
    Succeeded,
    Failed,
    Skipped,
}

impl Outcome {
    pub const COUNT: usize = 3;
    pub const ALL: [Self; Self::COUNT] = [Self::Succeeded, Self::Failed, Self::Skipped];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Skipped => "skipped",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(usize)]
pub enum ReasonKind {
    None,
    PermissionDenied,
    PolicyDisabled,
    ProviderUnavailable,
    Timeout,
    RateLimited,
    Unchanged,
    Deduplicated,
    Coalesced,
    NoEvidence,
    QueueFull,
    Shutdown,
    Storage,
    Internal,
}

impl ReasonKind {
    pub const COUNT: usize = 14;
    pub const ALL: [Self; Self::COUNT] = [
        Self::None,
        Self::PermissionDenied,
        Self::PolicyDisabled,
        Self::ProviderUnavailable,
        Self::Timeout,
        Self::RateLimited,
        Self::Unchanged,
        Self::Deduplicated,
        Self::Coalesced,
        Self::NoEvidence,
        Self::QueueFull,
        Self::Shutdown,
        Self::Storage,
        Self::Internal,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::PermissionDenied => "permission_denied",
            Self::PolicyDisabled => "policy_disabled",
            Self::ProviderUnavailable => "provider_unavailable",
            Self::Timeout => "timeout",
            Self::RateLimited => "rate_limited",
            Self::Unchanged => "unchanged",
            Self::Deduplicated => "deduplicated",
            Self::Coalesced => "coalesced",
            Self::NoEvidence => "no_evidence",
            Self::QueueFull => "queue_full",
            Self::Shutdown => "shutdown",
            Self::Storage => "storage",
            Self::Internal => "internal",
        }
    }
}

pub const fn valid_outcome_reason(outcome: Outcome, reason: ReasonKind) -> bool {
    match outcome {
        Outcome::Succeeded => matches!(reason, ReasonKind::None),
        Outcome::Failed | Outcome::Skipped => !matches!(reason, ReasonKind::None),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(usize)]
pub enum CaptureProviderKind {
    None,
    ScreenCaptureKit,
    WindowsGraphicsCapture,
    Xcap,
    Wayshot,
    Unknown,
}

impl CaptureProviderKind {
    pub const COUNT: usize = 6;
    pub const ALL: [Self; Self::COUNT] = [
        Self::None,
        Self::ScreenCaptureKit,
        Self::WindowsGraphicsCapture,
        Self::Xcap,
        Self::Wayshot,
        Self::Unknown,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::ScreenCaptureKit => "screen_capture_kit",
            Self::WindowsGraphicsCapture => "windows_graphics_capture",
            Self::Xcap => "xcap",
            Self::Wayshot => "wayshot",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorKind {
    Authentication,
    PermissionDenied,
    PolicyDisabled,
    ProviderUnavailable,
    Timeout,
    RateLimited,
    QueueFull,
    DatabaseBusy,
    Database,
    Network,
    InvalidRequest,
    InvalidOutput,
    StorageFull,
    Storage,
    ProcessExit,
    Cancelled,
    Internal,
}

impl ErrorKind {
    pub const ALL: [Self; 17] = [
        Self::Authentication,
        Self::PermissionDenied,
        Self::PolicyDisabled,
        Self::ProviderUnavailable,
        Self::Timeout,
        Self::RateLimited,
        Self::QueueFull,
        Self::DatabaseBusy,
        Self::Database,
        Self::Network,
        Self::InvalidRequest,
        Self::InvalidOutput,
        Self::StorageFull,
        Self::Storage,
        Self::ProcessExit,
        Self::Cancelled,
        Self::Internal,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Authentication => "authentication",
            Self::PermissionDenied => "permission_denied",
            Self::PolicyDisabled => "policy_disabled",
            Self::ProviderUnavailable => "provider_unavailable",
            Self::Timeout => "timeout",
            Self::RateLimited => "rate_limited",
            Self::QueueFull => "queue_full",
            Self::DatabaseBusy => "database_busy",
            Self::Database => "database",
            Self::Network => "network",
            Self::InvalidRequest => "invalid_request",
            Self::InvalidOutput => "invalid_output",
            Self::StorageFull => "storage_full",
            Self::Storage => "storage",
            Self::ProcessExit => "process_exit",
            Self::Cancelled => "cancelled",
            Self::Internal => "internal",
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    fn valid_instrument_name(name: &str) -> bool {
        !name.is_empty()
            && !name.ends_with(".total")
            && name.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'_')
            })
    }

    #[test]
    fn registry_names_are_unique_and_canonical() {
        let mut names = HashSet::new();
        for spec in METRICS {
            assert!(
                valid_instrument_name(spec.name),
                "invalid metric {}",
                spec.name
            );
            assert!(names.insert(spec.name), "duplicate metric {}", spec.name);
            assert!(!spec.unit.is_empty(), "metric {} has no unit", spec.name);
        }

        names.clear();
        for spec in SPANS {
            assert!(!spec.name.is_empty());
            assert!(names.insert(spec.name), "duplicate span {}", spec.name);
        }
    }

    #[test]
    fn registry_has_no_known_sensitive_attribute_keys() {
        let prohibited = [
            "app.name",
            "application.name",
            "browser.url",
            "db.statement",
            "device.id",
            "email",
            "exception.message",
            "file.path",
            "http.request.body",
            "http.url",
            "prompt",
            "user.id",
            "window.title",
        ];
        for key in RESOURCE_ATTRIBUTES
            .iter()
            .copied()
            .chain(
                METRICS
                    .iter()
                    .flat_map(|spec| spec.attributes.iter().copied()),
            )
            .chain(
                SPANS
                    .iter()
                    .flat_map(|spec| spec.attributes.iter().copied()),
            )
        {
            assert!(!prohibited.contains(&key), "prohibited attribute {key}");
        }
    }

    #[test]
    fn bounded_enums_are_unique() {
        for values in [
            CaptureTriggerKind::ALL
                .map(CaptureTriggerKind::as_str)
                .as_slice(),
            Outcome::ALL.map(Outcome::as_str).as_slice(),
            ReasonKind::ALL.map(ReasonKind::as_str).as_slice(),
            CaptureProviderKind::ALL
                .map(CaptureProviderKind::as_str)
                .as_slice(),
            ErrorKind::ALL.map(ErrorKind::as_str).as_slice(),
        ] {
            let unique = values.iter().copied().collect::<HashSet<_>>();
            assert_eq!(unique.len(), values.len());
        }
    }

    #[test]
    fn outcome_reason_contract_is_strict() {
        assert!(valid_outcome_reason(Outcome::Succeeded, ReasonKind::None));
        assert!(!valid_outcome_reason(
            Outcome::Succeeded,
            ReasonKind::Internal
        ));
        assert!(!valid_outcome_reason(Outcome::Failed, ReasonKind::None));
        assert!(valid_outcome_reason(Outcome::Failed, ReasonKind::Internal));
        assert!(valid_outcome_reason(
            Outcome::Skipped,
            ReasonKind::Coalesced
        ));
    }
}
