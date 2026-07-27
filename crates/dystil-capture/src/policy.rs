//! Easily tunable policy for macOS settled-activity visual capture.
//!
//! This module deliberately contains no Dystil or OS API types. The
//! scheduler adapter supplies normalized Dystil triggers and a clock value;
//! changing the debounce or trigger classification cannot disturb AX capture,
//! persistence, or native stream lifecycle code.

use std::time::{Duration, Instant};

use crate::{CaptureContext, CaptureTrigger};

pub const DEFAULT_VISUAL_SETTLE_DELAY: Duration = Duration::from_millis(1_500);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingVisualDemand {
    pub latest_context: CaptureContext,
}

#[derive(Debug, Clone)]
pub struct SettledVisualPolicy {
    settle_delay: Duration,
    pending: Option<PendingVisualDemand>,
    deadline: Option<Instant>,
}

impl Default for SettledVisualPolicy {
    fn default() -> Self {
        Self::new(DEFAULT_VISUAL_SETTLE_DELAY)
    }
}

impl SettledVisualPolicy {
    pub fn new(settle_delay: Duration) -> Self {
        Self {
            settle_delay,
            pending: None,
            deadline: None,
        }
    }

    pub fn settle_delay(&self) -> Duration {
        self.settle_delay
    }

    pub fn has_pending(&self) -> bool {
        self.pending.is_some()
    }

    pub fn deadline(&self) -> Option<Instant> {
        self.deadline
    }

    /// Observe one normalized activity event.
    ///
    /// Eligible triggers create (or update) demand. Key presses only extend an
    /// already-pending demand so typing cannot cause a capture by itself. Idle,
    /// visual-change, settled, and internal manual events do not participate in
    /// automatic scheduling.
    pub fn observe(&mut self, trigger: &CaptureTrigger, context: CaptureContext, now: Instant) {
        if creates_visual_demand(trigger) {
            let latest_context = match self.pending.take() {
                Some(previous) => context.with_fallback(&previous.latest_context),
                None => context,
            };
            self.pending = Some(PendingVisualDemand { latest_context });
            self.deadline = Some(now + self.settle_delay);
        } else if resets_pending_deadline(trigger) && self.pending.is_some() {
            self.deadline = Some(now + self.settle_delay);
        }
    }

    pub fn take_due(&mut self, now: Instant) -> Option<PendingVisualDemand> {
        if self.deadline.is_some_and(|deadline| deadline <= now) {
            self.deadline = None;
            self.pending.take()
        } else {
            None
        }
    }

    pub fn clear(&mut self) -> Option<PendingVisualDemand> {
        self.deadline = None;
        self.pending.take()
    }
}

pub fn creates_visual_demand(trigger: &CaptureTrigger) -> bool {
    matches!(
        trigger,
        CaptureTrigger::AppSwitch
            | CaptureTrigger::WindowFocus
            | CaptureTrigger::Click
            | CaptureTrigger::TypingPause
            | CaptureTrigger::ScrollStop
            | CaptureTrigger::Clipboard
    )
}

pub fn resets_pending_deadline(trigger: &CaptureTrigger) -> bool {
    creates_visual_demand(trigger) || matches!(trigger, CaptureTrigger::KeyPress)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ScreenPoint;

    fn context(app: &str, point: Option<(i32, i32)>) -> CaptureContext {
        CaptureContext {
            application: Some(app.to_string()),
            target: point.map(|(x, y)| ScreenPoint { x, y }),
            ..CaptureContext::default()
        }
    }

    #[test]
    fn one_click_fires_after_trailing_delay() {
        let start = Instant::now();
        let mut policy = SettledVisualPolicy::new(Duration::from_millis(1_500));
        policy.observe(
            &CaptureTrigger::Click,
            context("Browser", Some((5, 7))),
            start,
        );

        assert!(policy
            .take_due(start + Duration::from_millis(1_499))
            .is_none());
        let due = policy
            .take_due(start + Duration::from_millis(1_500))
            .expect("click should become due");
        assert_eq!(due.latest_context.application.as_deref(), Some("Browser"));
        assert_eq!(due.latest_context.target, Some(ScreenPoint { x: 5, y: 7 }));
    }

    #[test]
    fn rapid_app_switches_capture_only_latest_stable_context() {
        let start = Instant::now();
        let delay = Duration::from_millis(1_500);
        let mut policy = SettledVisualPolicy::new(delay);
        policy.observe(&CaptureTrigger::AppSwitch, context("A", None), start);
        policy.observe(
            &CaptureTrigger::AppSwitch,
            context("B", None),
            start + Duration::from_millis(700),
        );
        policy.observe(
            &CaptureTrigger::AppSwitch,
            context("C", None),
            start + Duration::from_millis(1_100),
        );

        assert!(policy
            .take_due(start + Duration::from_millis(2_599))
            .is_none());
        let due = policy
            .take_due(start + Duration::from_millis(2_600))
            .expect("final app should become due");
        assert_eq!(due.latest_context.application.as_deref(), Some("C"));
    }

    #[test]
    fn keypress_does_not_create_demand_but_extends_existing_demand() {
        let start = Instant::now();
        let mut policy = SettledVisualPolicy::new(Duration::from_secs(1));
        policy.observe(&CaptureTrigger::KeyPress, CaptureContext::default(), start);
        assert!(!policy.has_pending());

        policy.observe(&CaptureTrigger::Click, CaptureContext::default(), start);
        policy.observe(
            &CaptureTrigger::KeyPress,
            CaptureContext::default(),
            start + Duration::from_millis(900),
        );
        assert!(policy
            .take_due(start + Duration::from_millis(1_000))
            .is_none());
        assert!(policy
            .take_due(start + Duration::from_millis(1_900))
            .is_some());
    }

    #[test]
    fn uninterrupted_activity_has_no_forced_max_wait() {
        let start = Instant::now();
        let mut policy = SettledVisualPolicy::new(Duration::from_secs(2));
        policy.observe(&CaptureTrigger::Click, CaptureContext::default(), start);

        for second in 1..=30 {
            let now = start + Duration::from_secs(second);
            policy.observe(&CaptureTrigger::KeyPress, CaptureContext::default(), now);
            assert!(policy.take_due(now).is_none());
        }

        assert!(policy.take_due(start + Duration::from_secs(32)).is_some());
    }

    #[test]
    fn idle_visual_change_and_manual_do_not_schedule_automatic_images() {
        let start = Instant::now();
        let mut policy = SettledVisualPolicy::default();
        for trigger in [
            CaptureTrigger::Idle,
            CaptureTrigger::VisualChange,
            CaptureTrigger::Manual,
            CaptureTrigger::ActivitySettled,
        ] {
            policy.observe(&trigger, CaptureContext::default(), start);
        }
        assert!(!policy.has_pending());
    }
}
