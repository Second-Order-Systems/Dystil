use serde::{Deserialize, Serialize};

/// Product capture policy. Full capture acquires AX evidence and a screenshot
/// together for an accepted activity trigger; text-only capture acquires AX
/// evidence only.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureMode {
    /// Accessibility evidence only. This never touches screen-capture APIs.
    TextOnly,
    /// Acquire pixels for every accepted capture trigger.
    #[default]
    FullCapture,
}

impl CaptureMode {
    pub fn captures_for_trigger(self) -> bool {
        matches!(self, Self::FullCapture)
    }
}

/// Stable capture-core configuration.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureConfig {
    pub capture_mode: CaptureMode,
}
