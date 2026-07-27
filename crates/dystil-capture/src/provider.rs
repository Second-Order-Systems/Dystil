use async_trait::async_trait;

use crate::{
    AccessibilitySnapshot, CaptureContext, CaptureError, CaptureObservation, CaptureTrigger,
    DisplayGeometry, StoredCapture, VisualDemand, VisualSnapshot,
};

#[async_trait]
pub trait AccessibilityProvider: Send + Sync {
    async fn capture(
        &self,
        trigger: &CaptureTrigger,
    ) -> Result<Option<AccessibilitySnapshot>, CaptureError>;
}

#[derive(Debug, Clone)]
pub struct VisualRequest {
    pub trigger: CaptureTrigger,
    pub context: CaptureContext,
    pub demand: Option<VisualDemand>,
}

#[async_trait]
pub trait VisualProvider: Send + Sync {
    /// Acquire the visual snapshots relevant to one trigger.
    ///
    /// A provider may return one snapshot when the active display is known, or
    /// one snapshot per connected display when the platform cannot map the
    /// trigger/focus to a display reliably. One-shot implementations must
    /// release every native capture session before this future returns,
    /// including on timeout and error. Encoding and persistence happen after
    /// this boundary.
    async fn capture_all(
        &self,
        request: &VisualRequest,
    ) -> Result<Vec<VisualSnapshot>, CaptureError>;

    /// Release all persistent visual acquisition resources. Implementations
    /// must make this safe to call repeatedly.
    async fn stop(&self) -> Result<(), CaptureError>;
}

#[async_trait]
pub trait DisplayGeometryProvider: Send + Sync {
    async fn displays(&self) -> Result<Vec<DisplayGeometry>, CaptureError>;
}

#[async_trait]
pub trait CaptureStore: Send + Sync {
    async fn persist(&self, observation: CaptureObservation)
        -> Result<StoredCapture, CaptureError>;
}
