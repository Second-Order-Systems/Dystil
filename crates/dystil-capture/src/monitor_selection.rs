use crate::screen::monitor::SafeMonitor;

use crate::CaptureContext;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MonitorSelectionReason {
    ContextMonitor,
    TriggerPoint,
    NativeFocus,
    AllMonitorsFallback,
}

pub(super) struct MonitorSelection {
    pub monitors: Vec<SafeMonitor>,
    pub reason: MonitorSelectionReason,
}

#[derive(Debug, Clone, Copy)]
struct MonitorGeometry {
    id: u32,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

/// Resolve the monitor set for one FullCapture trigger.
///
/// This deliberately mirrors Dystil's old per-monitor routing behavior:
/// a point that maps into the capture API's display geometry targets one
/// monitor; an absent or un-mappable point fans out to all monitors. Native
/// focus is an additional safe one-monitor hint on platforms that implement
/// it. Linux currently reports no native focus, so it retains Dystil's
/// all-monitor fallback instead of guessing.
pub(super) fn select_monitors(
    mut monitors: Vec<SafeMonitor>,
    context: &CaptureContext,
    focused_id: Option<u32>,
) -> MonitorSelection {
    monitors.sort_by_key(|monitor| (monitor.x(), monitor.y(), monitor.id()));
    let geometry = monitors
        .iter()
        .map(|monitor| MonitorGeometry {
            id: monitor.id(),
            x: monitor.x(),
            y: monitor.y(),
            width: monitor.width(),
            height: monitor.height(),
        })
        .collect::<Vec<_>>();
    let (selected_ids, reason) = select_monitor_ids(&geometry, context, focused_id);
    let selected = monitors
        .into_iter()
        .filter(|monitor| selected_ids.contains(&monitor.id()))
        .collect();
    MonitorSelection {
        monitors: selected,
        reason,
    }
}

fn select_monitor_ids(
    monitors: &[MonitorGeometry],
    context: &CaptureContext,
    focused_id: Option<u32>,
) -> (Vec<u32>, MonitorSelectionReason) {
    if let Some(monitor_id) = context.monitor_id {
        if monitors.iter().any(|monitor| monitor.id == monitor_id) {
            return (vec![monitor_id], MonitorSelectionReason::ContextMonitor);
        }
    }

    if let Some(target) = context.target {
        if let Some(monitor) = monitors
            .iter()
            .find(|monitor| point_is_on_monitor(target.x, target.y, monitor))
        {
            return (vec![monitor.id], MonitorSelectionReason::TriggerPoint);
        }
    }

    if let Some(focused_id) = focused_id {
        if monitors.iter().any(|monitor| monitor.id == focused_id) {
            return (vec![focused_id], MonitorSelectionReason::NativeFocus);
        }
    }

    (
        monitors.iter().map(|monitor| monitor.id).collect(),
        MonitorSelectionReason::AllMonitorsFallback,
    )
}

fn point_is_on_monitor(x: i32, y: i32, monitor: &MonitorGeometry) -> bool {
    let x = i64::from(x);
    let y = i64::from(y);
    let left = i64::from(monitor.x);
    let top = i64::from(monitor.y);
    let right = left + i64::from(monitor.width);
    let bottom = top + i64::from(monitor.height);
    x >= left && x < right && y >= top && y < bottom
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ScreenPoint;

    fn geometry() -> Vec<MonitorGeometry> {
        vec![
            MonitorGeometry {
                id: 1082,
                x: 0,
                y: 0,
                width: 1600,
                height: 900,
            },
            MonitorGeometry {
                id: 1085,
                x: 1600,
                y: 0,
                width: 1600,
                height: 900,
            },
        ]
    }

    #[test]
    fn mapped_trigger_targets_only_its_monitor() {
        let context = CaptureContext {
            target: Some(ScreenPoint { x: 1700, y: 400 }),
            ..CaptureContext::default()
        };
        let (ids, reason) = select_monitor_ids(&geometry(), &context, None);
        assert_eq!(ids, vec![1085]);
        assert_eq!(reason, MonitorSelectionReason::TriggerPoint);
    }

    #[test]
    fn coordinate_space_mismatch_fans_out_to_every_monitor() {
        let context = CaptureContext {
            // Mirrors the observed Linux mismatch: the input source places
            // HDMI left of the laptop while xcap reports it to the right.
            target: Some(ScreenPoint { x: -604, y: 47 }),
            ..CaptureContext::default()
        };
        let (ids, reason) = select_monitor_ids(&geometry(), &context, None);
        assert_eq!(ids, vec![1082, 1085]);
        assert_eq!(reason, MonitorSelectionReason::AllMonitorsFallback);
    }

    #[test]
    fn missing_routing_metadata_fans_out_when_native_focus_is_unknown() {
        let (ids, reason) = select_monitor_ids(&geometry(), &CaptureContext::default(), None);
        assert_eq!(ids, vec![1082, 1085]);
        assert_eq!(reason, MonitorSelectionReason::AllMonitorsFallback);
    }

    #[test]
    fn native_focus_targets_one_monitor_without_a_trigger_point() {
        let (ids, reason) = select_monitor_ids(&geometry(), &CaptureContext::default(), Some(1085));
        assert_eq!(ids, vec![1085]);
        assert_eq!(reason, MonitorSelectionReason::NativeFocus);
    }
}
