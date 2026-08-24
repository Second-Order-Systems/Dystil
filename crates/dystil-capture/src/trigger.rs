use crate::{CaptureContext, CaptureTrigger, ScreenPoint};

/// Dystil's recorder-to-capture message. Correlation IDs are only present for
/// persisted UI rows that can later be linked to a captured frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureTriggerMessage {
    pub trigger: CaptureTrigger,
    pub context: CaptureContext,
    pub correlation_id: Option<u64>,
    /// Additional persisted UI rows represented by this one aggregate trigger.
    pub additional_correlation_ids: Vec<u64>,
    /// Candidate-only compact activity span to link to the settled frame.
    pub activity_span_id: Option<i64>,
    /// Aggregate wheel movement carried from the recorder into the candidate
    /// settled-state policy. Other triggers leave these at zero.
    pub activity_delta_x: i64,
    pub activity_delta_y: i64,
}

impl CaptureTriggerMessage {
    pub fn new(trigger: CaptureTrigger, context: CaptureContext) -> Self {
        Self {
            trigger,
            context,
            correlation_id: None,
            additional_correlation_ids: Vec::new(),
            activity_span_id: None,
            activity_delta_x: 0,
            activity_delta_y: 0,
        }
    }

    pub fn with_correlation(
        trigger: CaptureTrigger,
        context: CaptureContext,
        correlation_id: u64,
    ) -> Self {
        Self {
            trigger,
            context,
            correlation_id: Some(correlation_id),
            additional_correlation_ids: Vec::new(),
            activity_span_id: None,
            activity_delta_x: 0,
            activity_delta_y: 0,
        }
    }

    pub fn target(x: i32, y: i32) -> CaptureContext {
        CaptureContext {
            target: Some(ScreenPoint { x, y }),
            ..CaptureContext::default()
        }
    }
}

pub const TRIGGER_CHANNEL_BUFFER: usize = 1024;
