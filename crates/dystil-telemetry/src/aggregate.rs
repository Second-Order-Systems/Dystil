use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Mutex;

use crate::schema::{
    valid_outcome_reason, CaptureProviderKind, CaptureTriggerKind, ErrorKind, Outcome, ReasonKind,
};
use crate::TELEMETRY_SCHEMA_VERSION;

pub const TELEMETRY_CONSENT_VERSION: u16 = 1;

const CONSENT_UNKNOWN: u32 = 0;
const CONSENT_DENIED: u32 = 1;
const CONSENT_GRANTED_OFFSET: u32 = 2;
const CELL_COUNT: usize = CaptureTriggerKind::COUNT * Outcome::COUNT * ReasonKind::COUNT;
const IMAGE_CELL_COUNT: usize = CELL_COUNT * CaptureProviderKind::COUNT;
const TRACE_SAMPLE_DENOMINATOR: u64 = 20;
const MAX_PENDING_TRACES: usize = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsentDecision {
    Unknown,
    Denied,
    Granted { policy_version: u16 },
}

impl ConsentDecision {
    const fn encoded(self) -> u32 {
        match self {
            Self::Unknown => CONSENT_UNKNOWN,
            Self::Denied => CONSENT_DENIED,
            Self::Granted { policy_version } => CONSENT_GRANTED_OFFSET + policy_version as u32,
        }
    }

    pub const fn permits_current_schema(self) -> bool {
        matches!(
            self,
            Self::Granted { policy_version } if policy_version == TELEMETRY_CONSENT_VERSION
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalKind {
    CaptureTrigger,
    ImageCapture,
}

/// A deliberately small set of user-visible AI setup actions. Routine status
/// polling is excluded so telemetry reflects actionable failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AiOperationKind {
    Install,
    SignIn,
    ConnectionTest,
    McpSetup,
    McpConnect,
}

impl AiOperationKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Install => "install",
            Self::SignIn => "sign_in",
            Self::ConnectionTest => "connection_test",
            Self::McpSetup => "mcp_setup",
            Self::McpConnect => "mcp_connect",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AiProviderKind {
    Codex,
    Claude,
}

impl AiProviderKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
        }
    }
}

/// Reviewed failure categories only; never derive these from or export error
/// messages, stderr, paths, account data, or command arguments.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AiErrorKind {
    None,
    SidecarMissing,
    RuntimeMissing,
    Timeout,
    LoginRequired,
    AuthenticationFailed,
    ProcessFailed,
    InvalidOutput,
    Filesystem,
    McpClientUnavailable,
    McpRegistrationFailed,
    Unknown,
}

impl AiErrorKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::SidecarMissing => "sidecar_missing",
            Self::RuntimeMissing => "runtime_missing",
            Self::Timeout => "timeout",
            Self::LoginRequired => "login_required",
            Self::AuthenticationFailed => "authentication_failed",
            Self::ProcessFailed => "process_failed",
            Self::InvalidOutput => "invalid_output",
            Self::Filesystem => "filesystem",
            Self::McpClientUnavailable => "mcp_client_unavailable",
            Self::McpRegistrationFailed => "mcp_registration_failed",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AiOperationPoint {
    pub provider: AiProviderKind,
    pub operation: AiOperationKind,
    pub outcome: Outcome,
    pub error: AiErrorKind,
    pub value: u64,
}

/// Startup reasons deliberately distinguish a normal launch from evidence that
/// the previous local runtime did not reach its orderly shutdown path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AppStartReason {
    Launch,
    CaptureInitialization,
    PreviousUncleanShutdown,
}

impl AppStartReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Launch => "launch",
            Self::CaptureInitialization => "capture_initialization",
            Self::PreviousUncleanShutdown => "previous_unclean_shutdown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StorageOperationKind {
    RetentionCleanup,
    DatabaseCompaction,
    SnapshotCleanup,
}

impl StorageOperationKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RetentionCleanup => "retention_cleanup",
            Self::DatabaseCompaction => "database_compaction",
            Self::SnapshotCleanup => "snapshot_cleanup",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StartupPoint {
    pub reason: AppStartReason,
    pub outcome: Outcome,
    pub value: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StorageOperationPoint {
    pub operation: StorageOperationKind,
    pub outcome: Outcome,
    pub error: Option<ErrorKind>,
    pub value: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SyncIterationPoint {
    pub outcome: Outcome,
    pub error: Option<ErrorKind>,
    pub value: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CounterPoint {
    pub signal: SignalKind,
    pub trigger: CaptureTriggerKind,
    pub outcome: Outcome,
    pub reason: ReasonKind,
    pub provider: Option<CaptureProviderKind>,
    pub value: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntervalSnapshot {
    pub schema_version: u16,
    pub consent_version: u16,
    pub points: Vec<CounterPoint>,
    pub ai_operations: Vec<AiOperationPoint>,
    pub app_starts: Vec<StartupPoint>,
    pub storage_operations: Vec<StorageOperationPoint>,
    pub sync_iterations: Vec<SyncIterationPoint>,
    /// Latest slow-cadence process, host, and storage measurements. These are
    /// gauges, so only the most recent value is retained for an interval.
    pub resources: Option<ResourceSnapshot>,
    /// A bounded 5% deterministic sample of safe, fixed-name lifecycle spans.
    pub traces: Vec<TracePoint>,
    consent_generation: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceKind {
    CaptureSessionStart,
    CaptureSessionStop,
}

impl TraceKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CaptureSessionStart => "capture.session.start",
            Self::CaptureSessionStop => "capture.session.stop",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TracePoint {
    pub kind: TraceKind,
}

/// Typed resource gauges ready for a future exporter. CPU values are percent
/// multiplied by 100, avoiding floating point and invalid numeric values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceSnapshot {
    pub process_cpu_percent_x100: Option<u32>,
    pub process_memory_rss_bytes: Option<u64>,
    pub host_cpu_percent_x100: Option<u32>,
    pub host_memory_available_bytes: Option<u64>,
    pub storage_data_bytes: Option<u64>,
    pub storage_available_bytes: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordStatus {
    Recorded,
    Disabled,
    InvalidCombination,
}

pub trait TelemetryRecorder: Send + Sync {
    fn record_capture_trigger(
        &self,
        trigger: CaptureTriggerKind,
        outcome: Outcome,
        reason: ReasonKind,
    ) -> RecordStatus;

    fn record_image_capture(
        &self,
        trigger: CaptureTriggerKind,
        provider: CaptureProviderKind,
        outcome: Outcome,
        reason: ReasonKind,
    ) -> RecordStatus;
}

#[derive(Debug, Default)]
pub struct NoopRecorder;

impl TelemetryRecorder for NoopRecorder {
    fn record_capture_trigger(
        &self,
        _trigger: CaptureTriggerKind,
        _outcome: Outcome,
        _reason: ReasonKind,
    ) -> RecordStatus {
        RecordStatus::Disabled
    }

    fn record_image_capture(
        &self,
        _trigger: CaptureTriggerKind,
        _provider: CaptureProviderKind,
        _outcome: Outcome,
        _reason: ReasonKind,
    ) -> RecordStatus {
        RecordStatus::Disabled
    }
}

#[derive(Debug)]
struct CounterBank {
    cells: Box<[AtomicU64]>,
}

#[derive(Debug)]
struct ImageCounterBank {
    cells: Box<[AtomicU64]>,
}

impl ImageCounterBank {
    fn new() -> Self {
        Self {
            cells: (0..IMAGE_CELL_COUNT)
                .map(|_| AtomicU64::new(0))
                .collect::<Vec<_>>()
                .into_boxed_slice(),
        }
    }

    fn index(
        trigger: CaptureTriggerKind,
        provider: CaptureProviderKind,
        outcome: Outcome,
        reason: ReasonKind,
    ) -> usize {
        CounterBank::index(trigger, outcome, reason) * CaptureProviderKind::COUNT
            + provider as usize
    }

    fn increment(
        &self,
        trigger: CaptureTriggerKind,
        provider: CaptureProviderKind,
        outcome: Outcome,
        reason: ReasonKind,
    ) {
        self.cells[Self::index(trigger, provider, outcome, reason)].fetch_add(1, Ordering::Relaxed);
    }

    fn drain(&self, output: &mut Vec<CounterPoint>) {
        for trigger in CaptureTriggerKind::ALL {
            for outcome in Outcome::ALL {
                for reason in ReasonKind::ALL {
                    for provider in CaptureProviderKind::ALL {
                        let value = self.cells[Self::index(trigger, provider, outcome, reason)]
                            .swap(0, Ordering::AcqRel);
                        if value > 0 {
                            output.push(CounterPoint {
                                signal: SignalKind::ImageCapture,
                                trigger,
                                outcome,
                                reason,
                                provider: Some(provider),
                                value,
                            });
                        }
                    }
                }
            }
        }
    }

    fn clear(&self) {
        for cell in &self.cells {
            cell.store(0, Ordering::Release);
        }
    }
}

impl CounterBank {
    fn new() -> Self {
        Self {
            cells: (0..CELL_COUNT)
                .map(|_| AtomicU64::new(0))
                .collect::<Vec<_>>()
                .into_boxed_slice(),
        }
    }

    fn index(trigger: CaptureTriggerKind, outcome: Outcome, reason: ReasonKind) -> usize {
        ((trigger as usize * Outcome::COUNT) + outcome as usize) * ReasonKind::COUNT
            + reason as usize
    }

    fn increment(&self, trigger: CaptureTriggerKind, outcome: Outcome, reason: ReasonKind) {
        self.cells[Self::index(trigger, outcome, reason)].fetch_add(1, Ordering::Relaxed);
    }

    fn drain(&self, signal: SignalKind, output: &mut Vec<CounterPoint>) {
        for trigger in CaptureTriggerKind::ALL {
            for outcome in Outcome::ALL {
                for reason in ReasonKind::ALL {
                    let value =
                        self.cells[Self::index(trigger, outcome, reason)].swap(0, Ordering::AcqRel);
                    if value > 0 {
                        output.push(CounterPoint {
                            signal,
                            trigger,
                            outcome,
                            reason,
                            provider: None,
                            value,
                        });
                    }
                }
            }
        }
    }

    fn clear(&self) {
        for cell in &self.cells {
            cell.store(0, Ordering::Release);
        }
    }
}

impl Default for CounterBank {
    fn default() -> Self {
        Self::new()
    }
}

/// Always-on, allocation-free-on-record aggregation.
///
/// Draining allocates a compact vector once per export interval. No captured
/// strings or identifiers can enter this type because its recording API only
/// accepts bounded enums.
#[derive(Debug)]
pub struct Telemetry {
    consent: AtomicU32,
    consent_generation: AtomicU64,
    capture: CounterBank,
    image: ImageCounterBank,
    ai_operations: Mutex<HashMap<(AiProviderKind, AiOperationKind, Outcome, AiErrorKind), u64>>,
    app_starts: Mutex<HashMap<(AppStartReason, Outcome), u64>>,
    storage_operations: Mutex<HashMap<(StorageOperationKind, Outcome, Option<ErrorKind>), u64>>,
    sync_iterations: Mutex<HashMap<(Outcome, Option<ErrorKind>), u64>>,
    resources: Mutex<Option<ResourceSnapshot>>,
    traces: Mutex<Vec<TracePoint>>,
    trace_sample_counter: AtomicU64,
}

impl Default for Telemetry {
    fn default() -> Self {
        Self::new()
    }
}

impl Telemetry {
    pub fn new() -> Self {
        Self {
            // Desktop operational telemetry is currently product-default on.
            // The retained gate is reserved for an emergency local disable or
            // a future policy change; it is not backed by a user setting.
            consent: AtomicU32::new(
                ConsentDecision::Granted {
                    policy_version: TELEMETRY_CONSENT_VERSION,
                }
                .encoded(),
            ),
            consent_generation: AtomicU64::new(0),
            capture: CounterBank::new(),
            image: ImageCounterBank::new(),
            ai_operations: Mutex::new(HashMap::new()),
            app_starts: Mutex::new(HashMap::new()),
            storage_operations: Mutex::new(HashMap::new()),
            sync_iterations: Mutex::new(HashMap::new()),
            resources: Mutex::new(None),
            traces: Mutex::new(Vec::new()),
            trace_sample_counter: AtomicU64::new(0),
        }
    }

    pub fn set_consent(&self, decision: ConsentDecision) {
        // Move through disabled before clearing so hot-path recorders stop
        // accepting data during a consent transition. Enabling clears again,
        // preventing a racing pre-revocation increment from leaking forward.
        self.consent.store(CONSENT_DENIED, Ordering::Release);
        self.consent_generation.fetch_add(1, Ordering::AcqRel);
        self.clear();
        self.consent.store(decision.encoded(), Ordering::Release);
        self.consent_generation.fetch_add(1, Ordering::AcqRel);
    }

    pub fn is_enabled(&self) -> bool {
        self.consent.load(Ordering::Acquire)
            == ConsentDecision::Granted {
                policy_version: TELEMETRY_CONSENT_VERSION,
            }
            .encoded()
    }

    pub fn drain_interval(&self) -> Option<IntervalSnapshot> {
        if !self.is_enabled() {
            return None;
        }
        let generation = self.consent_generation.load(Ordering::Acquire);
        let mut points = Vec::new();
        self.capture.drain(SignalKind::CaptureTrigger, &mut points);
        self.image.drain(&mut points);
        let ai_operations =
            std::mem::take(&mut *self.ai_operations.lock().expect("telemetry mutex poisoned"))
                .into_iter()
                .map(
                    |((provider, operation, outcome, error), value)| AiOperationPoint {
                        provider,
                        operation,
                        outcome,
                        error,
                        value,
                    },
                )
                .collect();
        let app_starts =
            std::mem::take(&mut *self.app_starts.lock().expect("telemetry mutex poisoned"))
                .into_iter()
                .map(|((reason, outcome), value)| StartupPoint {
                    reason,
                    outcome,
                    value,
                })
                .collect();
        let storage_operations = std::mem::take(
            &mut *self
                .storage_operations
                .lock()
                .expect("telemetry mutex poisoned"),
        )
        .into_iter()
        .map(
            |((operation, outcome, error), value)| StorageOperationPoint {
                operation,
                outcome,
                error,
                value,
            },
        )
        .collect();
        let sync_iterations = std::mem::take(
            &mut *self
                .sync_iterations
                .lock()
                .expect("telemetry mutex poisoned"),
        )
        .into_iter()
        .map(|((outcome, error), value)| SyncIterationPoint {
            outcome,
            error,
            value,
        })
        .collect();
        let resources = self
            .resources
            .lock()
            .expect("telemetry mutex poisoned")
            .take();
        let traces = std::mem::take(&mut *self.traces.lock().expect("telemetry mutex poisoned"));

        if !self.is_enabled() || generation != self.consent_generation.load(Ordering::Acquire) {
            return None;
        }

        Some(IntervalSnapshot {
            schema_version: TELEMETRY_SCHEMA_VERSION,
            consent_version: TELEMETRY_CONSENT_VERSION,
            points,
            ai_operations,
            app_starts,
            storage_operations,
            sync_iterations,
            resources,
            traces,
            consent_generation: generation,
        })
    }

    /// Exporters must check this immediately before enqueue/send so consent
    /// revocation invalidates a snapshot drained concurrently.
    pub fn snapshot_is_current(&self, snapshot: &IntervalSnapshot) -> bool {
        self.is_enabled()
            && snapshot.consent_generation == self.consent_generation.load(Ordering::Acquire)
    }

    fn clear(&self) {
        self.capture.clear();
        self.image.clear();
        *self.resources.lock().expect("telemetry mutex poisoned") = None;
        self.ai_operations
            .lock()
            .expect("telemetry mutex poisoned")
            .clear();
        self.app_starts
            .lock()
            .expect("telemetry mutex poisoned")
            .clear();
        self.storage_operations
            .lock()
            .expect("telemetry mutex poisoned")
            .clear();
        self.sync_iterations
            .lock()
            .expect("telemetry mutex poisoned")
            .clear();
        self.traces
            .lock()
            .expect("telemetry mutex poisoned")
            .clear();
    }

    /// Replace the latest slow-cadence gauge values. The API cannot accept
    /// strings, paths, identifiers, or arbitrary attributes.
    pub fn record_resource_snapshot(&self, resources: ResourceSnapshot) -> RecordStatus {
        if !self.is_enabled() {
            return RecordStatus::Disabled;
        }
        *self.resources.lock().expect("telemetry mutex poisoned") = Some(resources);
        RecordStatus::Recorded
    }

    pub fn record_ai_operation(
        &self,
        provider: AiProviderKind,
        operation: AiOperationKind,
        outcome: Outcome,
        error: AiErrorKind,
    ) -> RecordStatus {
        if !self.is_enabled() || !matches!(outcome, Outcome::Succeeded | Outcome::Failed) {
            return RecordStatus::Disabled;
        }
        if (matches!(outcome, Outcome::Succeeded) && !matches!(error, AiErrorKind::None))
            || (matches!(outcome, Outcome::Failed) && matches!(error, AiErrorKind::None))
        {
            return RecordStatus::InvalidCombination;
        }
        let mut operations = self.ai_operations.lock().expect("telemetry mutex poisoned");
        *operations
            .entry((provider, operation, outcome, error))
            .or_default() += 1;
        RecordStatus::Recorded
    }

    pub fn record_app_start(&self, reason: AppStartReason, outcome: Outcome) -> RecordStatus {
        if !self.is_enabled() || !matches!(outcome, Outcome::Succeeded | Outcome::Failed) {
            return RecordStatus::Disabled;
        }
        *self
            .app_starts
            .lock()
            .expect("telemetry mutex poisoned")
            .entry((reason, outcome))
            .or_default() += 1;
        RecordStatus::Recorded
    }

    pub fn record_storage_operation(
        &self,
        operation: StorageOperationKind,
        outcome: Outcome,
        error: Option<ErrorKind>,
    ) -> RecordStatus {
        self.record_operational(
            &self.storage_operations,
            (operation, outcome, error),
            outcome,
            error,
        )
    }

    pub fn record_sync_iteration(
        &self,
        outcome: Outcome,
        error: Option<ErrorKind>,
    ) -> RecordStatus {
        self.record_operational(&self.sync_iterations, (outcome, error), outcome, error)
    }

    fn record_operational<K: std::cmp::Eq + std::hash::Hash>(
        &self,
        bank: &Mutex<HashMap<K, u64>>,
        key: K,
        outcome: Outcome,
        error: Option<ErrorKind>,
    ) -> RecordStatus {
        if !self.is_enabled() || !matches!(outcome, Outcome::Succeeded | Outcome::Failed) {
            return RecordStatus::Disabled;
        }
        if (matches!(outcome, Outcome::Succeeded) && error.is_some())
            || (matches!(outcome, Outcome::Failed) && error.is_none())
        {
            return RecordStatus::InvalidCombination;
        }
        *bank
            .lock()
            .expect("telemetry mutex poisoned")
            .entry(key)
            .or_default() += 1;
        RecordStatus::Recorded
    }

    /// Record a fixed-name operational lifecycle span at a deterministic 5%
    /// sample rate. There are no attributes, events, links, or payload fields.
    pub fn record_sampled_trace(&self, kind: TraceKind) -> RecordStatus {
        if !self.is_enabled() {
            return RecordStatus::Disabled;
        }
        if self.trace_sample_counter.fetch_add(1, Ordering::Relaxed) % TRACE_SAMPLE_DENOMINATOR != 0
        {
            return RecordStatus::Recorded;
        }
        let mut traces = self.traces.lock().expect("telemetry mutex poisoned");
        if traces.len() < MAX_PENDING_TRACES {
            traces.push(TracePoint { kind });
        }
        RecordStatus::Recorded
    }

    fn record(
        &self,
        bank: &CounterBank,
        trigger: CaptureTriggerKind,
        outcome: Outcome,
        reason: ReasonKind,
    ) -> RecordStatus {
        if !self.is_enabled() {
            return RecordStatus::Disabled;
        }
        if !valid_outcome_reason(outcome, reason) {
            return RecordStatus::InvalidCombination;
        }
        bank.increment(trigger, outcome, reason);
        RecordStatus::Recorded
    }
}

impl TelemetryRecorder for Telemetry {
    fn record_capture_trigger(
        &self,
        trigger: CaptureTriggerKind,
        outcome: Outcome,
        reason: ReasonKind,
    ) -> RecordStatus {
        self.record(&self.capture, trigger, outcome, reason)
    }

    fn record_image_capture(
        &self,
        trigger: CaptureTriggerKind,
        provider: CaptureProviderKind,
        outcome: Outcome,
        reason: ReasonKind,
    ) -> RecordStatus {
        if !self.is_enabled() {
            return RecordStatus::Disabled;
        }
        if !valid_outcome_reason(outcome, reason) {
            return RecordStatus::InvalidCombination;
        }
        self.image.increment(trigger, provider, outcome, reason);
        RecordStatus::Recorded
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    fn grant(telemetry: &Telemetry) {
        telemetry.set_consent(ConsentDecision::Granted {
            policy_version: TELEMETRY_CONSENT_VERSION,
        });
    }

    #[test]
    fn collection_is_on_by_default() {
        let telemetry = Telemetry::new();
        assert_eq!(
            telemetry.record_capture_trigger(
                CaptureTriggerKind::Click,
                Outcome::Succeeded,
                ReasonKind::None,
            ),
            RecordStatus::Recorded
        );
        assert!(telemetry.is_enabled());
        telemetry.set_consent(ConsentDecision::Granted {
            policy_version: TELEMETRY_CONSENT_VERSION - 1,
        });
        assert!(!telemetry.is_enabled());
        assert!(telemetry.drain_interval().is_none());
    }

    #[test]
    fn drain_returns_only_typed_nonzero_deltas_and_resets_them() {
        let telemetry = Telemetry::new();
        grant(&telemetry);
        for _ in 0..3 {
            assert_eq!(
                telemetry.record_capture_trigger(
                    CaptureTriggerKind::Click,
                    Outcome::Succeeded,
                    ReasonKind::None,
                ),
                RecordStatus::Recorded
            );
        }
        telemetry.record_image_capture(
            CaptureTriggerKind::AppSwitch,
            CaptureProviderKind::None,
            Outcome::Skipped,
            ReasonKind::PolicyDisabled,
        );

        let snapshot = telemetry.drain_interval().expect("enabled snapshot");
        assert_eq!(snapshot.schema_version, TELEMETRY_SCHEMA_VERSION);
        assert_eq!(snapshot.points.len(), 2);
        assert!(snapshot.points.contains(&CounterPoint {
            signal: SignalKind::CaptureTrigger,
            trigger: CaptureTriggerKind::Click,
            outcome: Outcome::Succeeded,
            reason: ReasonKind::None,
            provider: None,
            value: 3,
        }));
        assert!(snapshot.points.contains(&CounterPoint {
            signal: SignalKind::ImageCapture,
            trigger: CaptureTriggerKind::AppSwitch,
            outcome: Outcome::Skipped,
            reason: ReasonKind::PolicyDisabled,
            provider: Some(CaptureProviderKind::None),
            value: 1,
        }));
        assert!(telemetry.drain_interval().unwrap().points.is_empty());
    }

    #[test]
    fn latest_resource_snapshot_is_drained_once() {
        let telemetry = Telemetry::new();
        let resources = ResourceSnapshot {
            process_cpu_percent_x100: Some(123),
            process_memory_rss_bytes: Some(456),
            host_cpu_percent_x100: Some(789),
            host_memory_available_bytes: Some(101_112),
            storage_data_bytes: Some(131_415),
            storage_available_bytes: Some(161_718),
        };
        assert_eq!(
            telemetry.record_resource_snapshot(resources),
            RecordStatus::Recorded
        );
        assert_eq!(
            telemetry.drain_interval().unwrap().resources,
            Some(resources)
        );
        assert_eq!(telemetry.drain_interval().unwrap().resources, None);
    }

    #[test]
    fn invalid_outcome_reason_is_never_counted() {
        let telemetry = Telemetry::new();
        grant(&telemetry);
        assert_eq!(
            telemetry.record_capture_trigger(
                CaptureTriggerKind::Click,
                Outcome::Succeeded,
                ReasonKind::Internal,
            ),
            RecordStatus::InvalidCombination
        );
        assert_eq!(
            telemetry.record_capture_trigger(
                CaptureTriggerKind::Click,
                Outcome::Failed,
                ReasonKind::None,
            ),
            RecordStatus::InvalidCombination
        );
        assert!(telemetry.drain_interval().unwrap().points.is_empty());
    }

    #[test]
    fn revocation_clears_counts_and_invalidates_drained_snapshots() {
        let telemetry = Telemetry::new();
        grant(&telemetry);
        telemetry.record_capture_trigger(
            CaptureTriggerKind::WindowFocus,
            Outcome::Failed,
            ReasonKind::ProviderUnavailable,
        );
        let snapshot = telemetry.drain_interval().unwrap();
        assert!(telemetry.snapshot_is_current(&snapshot));

        telemetry.set_consent(ConsentDecision::Denied);
        assert!(!telemetry.snapshot_is_current(&snapshot));
        assert!(telemetry.drain_interval().is_none());

        grant(&telemetry);
        assert!(telemetry.drain_interval().unwrap().points.is_empty());
    }

    #[test]
    fn noop_recorder_never_collects() {
        let recorder = NoopRecorder;
        assert_eq!(
            recorder.record_image_capture(
                CaptureTriggerKind::Manual,
                CaptureProviderKind::Unknown,
                Outcome::Failed,
                ReasonKind::Internal,
            ),
            RecordStatus::Disabled
        );
    }

    #[test]
    fn concurrent_hot_path_updates_are_not_lost() {
        const WORKERS: usize = 8;
        const RECORDS_PER_WORKER: usize = 10_000;

        let telemetry = Arc::new(Telemetry::new());
        grant(&telemetry);
        let workers = (0..WORKERS)
            .map(|_| {
                let telemetry = telemetry.clone();
                std::thread::spawn(move || {
                    for _ in 0..RECORDS_PER_WORKER {
                        telemetry.record_capture_trigger(
                            CaptureTriggerKind::Click,
                            Outcome::Succeeded,
                            ReasonKind::None,
                        );
                    }
                })
            })
            .collect::<Vec<_>>();
        for worker in workers {
            worker.join().unwrap();
        }

        let snapshot = telemetry.drain_interval().unwrap();
        assert_eq!(snapshot.points.len(), 1);
        assert_eq!(
            snapshot.points[0].value,
            (WORKERS * RECORDS_PER_WORKER) as u64
        );
    }
}
