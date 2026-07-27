use crate::{CaptureContext, CaptureTrigger, ScreenPoint};

/// Dystil's recorder-to-capture message. Correlation IDs are only present for
/// persisted UI rows that can later be linked to a captured frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureTriggerMessage {
    pub trigger: CaptureTrigger,
    pub context: CaptureContext,
    pub correlation_id: Option<u64>,
}

impl CaptureTriggerMessage {
    pub fn new(trigger: CaptureTrigger, context: CaptureContext) -> Self {
        Self {
            trigger,
            context,
            correlation_id: None,
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
