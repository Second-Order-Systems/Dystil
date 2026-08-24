//! Candidate-only Stage 3 scheduling policy.
//!
//! This owns timing only: callers provide normalized activity and perform UIA
//! work only for the returned settled demand.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::{CaptureContext, CaptureTrigger};

pub const CLICK_SETTLE: Duration = Duration::from_millis(500);
pub const SWITCH_SETTLE: Duration = Duration::from_millis(750);
pub const SCROLL_SETTLE: Duration = Duration::from_millis(2_500);
pub const TYPING_PAUSE: Duration = Duration::from_millis(1_500);
pub const CONTINUOUS_CHECKPOINT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ActivityKind {
    Click,
    AppSwitch,
    Scroll,
    Typing,
}

#[derive(Debug, Clone)]
pub struct SettledDemand {
    pub trigger: CaptureTrigger,
    pub context: CaptureContext,
    pub kind: ActivityKind,
    pub correlation_ids: Vec<u64>,
    pub duration_ms: u64,
    pub event_count: u64,
    pub app_sequence: Vec<String>,
    pub scroll_delta_x: i64,
    pub scroll_delta_y: i64,
}

#[derive(Debug, Clone)]
struct Pending {
    kind: ActivityKind,
    context: CaptureContext,
    started_at: Instant,
    deadline: Instant,
    correlation_ids: Vec<u64>,
    event_count: u64,
    app_sequence: Vec<String>,
    scroll_delta_x: i64,
    scroll_delta_y: i64,
}

#[derive(Debug, Default)]
pub struct SettledStatePolicy {
    pending: HashMap<ActivityKind, Pending>,
}

impl SettledStatePolicy {
    pub fn observe(
        &mut self,
        kind: ActivityKind,
        context: CaptureContext,
        correlation_ids: impl IntoIterator<Item = u64>,
        scroll_delta_x: i64,
        scroll_delta_y: i64,
        now: Instant,
    ) {
        let correlation_ids: Vec<_> = correlation_ids.into_iter().collect();
        let deadline = now + settle_for(kind);
        self.pending
            .entry(kind)
            .and_modify(|previous| {
                previous.context = context.clone().with_fallback(&previous.context);
                previous.deadline = deadline.min(previous.started_at + CONTINUOUS_CHECKPOINT);
                previous
                    .correlation_ids
                    .extend(correlation_ids.iter().copied());
                previous.event_count += 1;
                previous.scroll_delta_x += scroll_delta_x;
                previous.scroll_delta_y += scroll_delta_y;
                append_surface(&mut previous.app_sequence, context.application.as_deref());
            })
            .or_insert_with(|| {
                let app_sequence = context.application.iter().cloned().collect();
                Pending {
                    kind,
                    context,
                    started_at: now,
                    deadline,
                    correlation_ids,
                    event_count: 1,
                    app_sequence,
                    scroll_delta_x,
                    scroll_delta_y,
                }
            });
    }

    pub fn take_due(&mut self, now: Instant) -> Vec<SettledDemand> {
        let due: Vec<_> = self
            .pending
            .iter()
            .filter_map(|(kind, pending)| (pending.deadline <= now).then_some(*kind))
            .collect();
        due.into_iter()
            .filter_map(|kind| self.pending.remove(&kind))
            .map(|pending| SettledDemand {
                trigger: trigger_for(pending.kind),
                context: pending.context,
                kind: pending.kind,
                correlation_ids: pending.correlation_ids,
                duration_ms: now.duration_since(pending.started_at).as_millis() as u64,
                event_count: pending.event_count,
                app_sequence: pending.app_sequence,
                scroll_delta_x: pending.scroll_delta_x,
                scroll_delta_y: pending.scroll_delta_y,
            })
            .collect()
    }

    /// Return a due demand to the pending set when a candidate-only cadence
    /// guard needs to wait for an earlier expensive UIA walk to cool down.
    /// Nothing is dropped: later activity of the same kind merges into this
    /// demand through `observe`, and the original activity accounting stays
    /// attached to the eventual checkpoint.
    pub fn defer(&mut self, demand: SettledDemand, deadline: Instant, now: Instant) {
        let started_at = now
            .checked_sub(Duration::from_millis(demand.duration_ms))
            .unwrap_or(now);
        self.pending
            .entry(demand.kind)
            .and_modify(|pending| {
                pending.context = demand.context.clone().with_fallback(&pending.context);
                pending.deadline = pending.deadline.min(deadline);
                pending
                    .correlation_ids
                    .extend(demand.correlation_ids.iter().copied());
                pending.event_count += demand.event_count;
                pending.scroll_delta_x += demand.scroll_delta_x;
                pending.scroll_delta_y += demand.scroll_delta_y;
                for app in &demand.app_sequence {
                    append_surface(&mut pending.app_sequence, Some(app));
                }
            })
            .or_insert(Pending {
                kind: demand.kind,
                context: demand.context,
                started_at,
                deadline,
                correlation_ids: demand.correlation_ids,
                event_count: demand.event_count,
                app_sequence: demand.app_sequence,
                scroll_delta_x: demand.scroll_delta_x,
                scroll_delta_y: demand.scroll_delta_y,
            });
    }
}

/// Candidate-only Screenpipe-style cadence control. It changes scheduling,
/// never the size/depth/text/time limits of an actual UIA walk.
#[derive(Debug, Default)]
pub struct AppCadenceGuard {
    entries: HashMap<String, CadenceEntry>,
}

#[derive(Debug, Clone, Copy)]
struct CadenceEntry {
    last_capture: Instant,
    cooldown: Duration,
}

impl AppCadenceGuard {
    /// Typing is a durable text checkpoint and is never deferred. Other due
    /// demands may wait for the same app's immediately preceding costly walk.
    pub fn defer_until(&self, demand: &SettledDemand, now: Instant) -> Option<Instant> {
        if demand.kind == ActivityKind::Typing {
            return None;
        }
        let application = demand.context.application.as_deref()?.trim();
        let entry = self.entries.get(&application.to_ascii_lowercase())?;
        let earliest = entry.last_capture + entry.cooldown;
        (earliest > now).then_some(earliest)
    }

    pub fn record_capture(&mut self, context: &CaptureContext, duration: Duration, now: Instant) {
        let Some(application) = context
            .application
            .as_deref()
            .map(str::trim)
            .filter(|app| !app.is_empty())
        else {
            return;
        };
        // Cost tiers affect only the *next* demand's timing. Every actual
        // capture still uses the fixed 5k/35/50k/250ms UIA limits.
        let cooldown = if duration >= Duration::from_millis(250) {
            Duration::from_secs(5)
        } else if duration >= Duration::from_millis(150) {
            Duration::from_secs(3)
        } else {
            Duration::from_millis(1_500)
        };
        self.entries.insert(
            application.to_ascii_lowercase(),
            CadenceEntry {
                last_capture: now,
                cooldown,
            },
        );
    }
}

fn append_surface(sequence: &mut Vec<String>, application: Option<&str>) {
    let Some(application) = application.filter(|value| !value.is_empty()) else {
        return;
    };
    if sequence.last().is_none_or(|last| last != application) {
        sequence.push(application.to_string());
    }
}

fn settle_for(kind: ActivityKind) -> Duration {
    match kind {
        ActivityKind::Click => CLICK_SETTLE,
        ActivityKind::AppSwitch => SWITCH_SETTLE,
        ActivityKind::Scroll => SCROLL_SETTLE,
        ActivityKind::Typing => TYPING_PAUSE,
    }
}

fn trigger_for(kind: ActivityKind) -> CaptureTrigger {
    match kind {
        ActivityKind::Click => CaptureTrigger::Click,
        ActivityKind::AppSwitch => CaptureTrigger::AppSwitch,
        ActivityKind::Scroll => CaptureTrigger::ScrollStop,
        ActivityKind::Typing => CaptureTrigger::TypingPause,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context(application: &str) -> CaptureContext {
        CaptureContext {
            application: Some(application.to_string()),
            ..CaptureContext::default()
        }
    }

    #[test]
    fn click_burst_waits_for_quiet_period_and_keeps_final_target() {
        let start = Instant::now();
        let mut policy = SettledStatePolicy::default();
        let mut first = context("msedge.exe");
        first.target = Some(crate::ScreenPoint { x: 10, y: 10 });
        let mut final_context = context("msedge.exe");
        final_context.target = Some(crate::ScreenPoint { x: 30, y: 40 });
        let expected_target = final_context.target;
        policy.observe(ActivityKind::Click, first, [1], 0, 0, start);
        policy.observe(
            ActivityKind::Click,
            final_context,
            [2],
            0,
            0,
            start + Duration::from_millis(300),
        );

        assert!(policy
            .take_due(start + Duration::from_millis(799))
            .is_empty());
        let due = policy.take_due(start + Duration::from_millis(800));
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].event_count, 2);
        assert_eq!(due[0].correlation_ids, vec![1, 2]);
        assert_eq!(due[0].context.target, expected_target);
    }

    #[test]
    fn rapid_switches_capture_only_final_surface_and_ordered_span() {
        let start = Instant::now();
        let mut policy = SettledStatePolicy::default();
        for (offset, app) in [(0, "msedge.exe"), (100, "explorer.exe"), (200, "Code.exe")] {
            policy.observe(
                ActivityKind::AppSwitch,
                context(app),
                [offset as u64 + 1],
                0,
                0,
                start + Duration::from_millis(offset),
            );
        }
        let due = policy.take_due(start + Duration::from_millis(950));
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].context.application.as_deref(), Some("Code.exe"));
        assert_eq!(
            due[0].app_sequence,
            ["msedge.exe", "explorer.exe", "Code.exe"]
        );
    }

    #[test]
    fn scroll_waits_two_and_a_half_seconds_and_sums_delta() {
        let start = Instant::now();
        let mut policy = SettledStatePolicy::default();
        policy.observe(
            ActivityKind::Scroll,
            context("msedge.exe"),
            [7],
            0,
            -120,
            start,
        );
        policy.observe(
            ActivityKind::Scroll,
            context("msedge.exe"),
            [8],
            0,
            -240,
            start + Duration::from_millis(100),
        );
        assert!(policy
            .take_due(start + Duration::from_millis(2_599))
            .is_empty());
        let due = policy.take_due(start + Duration::from_millis(2_600));
        assert_eq!(due[0].scroll_delta_y, -360);
        assert_eq!(due[0].event_count, 2);
    }

    #[test]
    fn continuous_typing_has_a_thirty_second_checkpoint_but_idle_has_none() {
        let start = Instant::now();
        let mut policy = SettledStatePolicy::default();
        assert!(policy.take_due(start).is_empty());
        for second in 0..30 {
            policy.observe(
                ActivityKind::Typing,
                context("Code.exe"),
                [second],
                0,
                0,
                start + Duration::from_secs(second),
            );
        }
        assert!(policy
            .take_due(start + Duration::from_millis(29_999))
            .is_empty());
        let due = policy.take_due(start + CONTINUOUS_CHECKPOINT);
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].event_count, 30);
    }

    #[test]
    fn cadence_defers_costly_same_app_clicks_but_never_typing() {
        let start = Instant::now();
        let mut guard = AppCadenceGuard::default();
        let edge = context("msedge.exe");
        guard.record_capture(&edge, Duration::from_millis(300), start);
        let click = SettledDemand {
            trigger: CaptureTrigger::Click,
            context: edge.clone(),
            kind: ActivityKind::Click,
            correlation_ids: vec![1],
            duration_ms: 500,
            event_count: 1,
            app_sequence: vec!["msedge.exe".to_string()],
            scroll_delta_x: 0,
            scroll_delta_y: 0,
        };
        assert_eq!(
            guard.defer_until(&click, start + Duration::from_secs(1)),
            Some(start + Duration::from_secs(5))
        );
        let typing = SettledDemand {
            kind: ActivityKind::Typing,
            trigger: CaptureTrigger::TypingPause,
            ..click
        };
        assert_eq!(
            guard.defer_until(&typing, start + Duration::from_secs(1)),
            None
        );
    }

    #[test]
    fn deferred_demand_merges_later_activity_without_losing_accounting() {
        let start = Instant::now();
        let mut policy = SettledStatePolicy::default();
        let deferred = SettledDemand {
            trigger: CaptureTrigger::ScrollStop,
            context: context("msedge.exe"),
            kind: ActivityKind::Scroll,
            correlation_ids: vec![7],
            duration_ms: 1_000,
            event_count: 2,
            app_sequence: vec!["msedge.exe".to_string()],
            scroll_delta_x: 0,
            scroll_delta_y: -240,
        };
        policy.defer(deferred, start + Duration::from_secs(3), start);
        policy.observe(
            ActivityKind::Scroll,
            context("msedge.exe"),
            [8],
            0,
            -120,
            start + Duration::from_secs(1),
        );
        let due = policy.take_due(start + Duration::from_secs(4));
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].correlation_ids, vec![7, 8]);
        assert_eq!(due[0].event_count, 3);
        assert_eq!(due[0].scroll_delta_y, -360);
    }
}
