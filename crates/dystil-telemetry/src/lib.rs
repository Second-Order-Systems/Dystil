//! Privacy-safe telemetry primitives for Dystil.
//!
//! This crate deliberately has no exporter or OpenTelemetry SDK dependency.
//! Product crates record typed, bounded values here; a binary may later drain
//! interval snapshots into an exporter.

mod aggregate;
pub mod schema;

pub use aggregate::{
    AiErrorKind, AiOperationKind, AiOperationPoint, AiProviderKind, AppStartReason,
    ConsentDecision, CounterPoint, IntervalSnapshot, NoopRecorder, RecordStatus, ResourceSnapshot,
    SignalKind, StartupPoint, StorageOperationKind, StorageOperationPoint, SyncIterationPoint,
    Telemetry, TelemetryRecorder, TraceKind, TracePoint, TELEMETRY_CONSENT_VERSION,
};
pub use schema::{
    CaptureProviderKind, CaptureTriggerKind, ErrorKind, Outcome, ReasonKind,
    TELEMETRY_SCHEMA_VERSION,
};
