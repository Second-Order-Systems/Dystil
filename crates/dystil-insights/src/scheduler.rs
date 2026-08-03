use chrono::{DateTime, NaiveDate, Utc};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WakeReason {
    ObservationVolume,
    ObservationBurst,
    PendingDeadline,
    ExplicitRequest,
    EndOfDay,
    Recovery,
}

#[derive(Debug, Clone)]
pub struct WakeState {
    pub pending_observations: usize,
    pub pending_episode_groups: usize,
    pub minutes_since_last_successful_wake: i64,
    pub oldest_pending_minutes: i64,
    pub job_running: bool,
    pub explicit_request: bool,
    pub end_of_active_day: bool,
    pub recovery_pending: bool,
    pub provider_ready: bool,
    pub resource_permitted: bool,
}

/// Adaptive Steward batching policy.
///
/// This deliberately uses evidence volume plus elapsed queue time instead of a
/// fixed daily wake allowance. A normal wake waits for a useful batch; a burst
/// drains pressure sooner; and the deadline prevents sparse work from waiting
/// indefinitely. Explicit requests, one end-of-day flush, and crash recovery
/// bypass batching thresholds. Provider cache state is never part of the
/// decision: durable observations and memory remain the source of truth.
#[derive(Debug, Clone, Copy)]
pub struct WakePolicy {
    pub observation_threshold: usize,
    pub episode_group_threshold: usize,
    pub normal_interval_minutes: i64,
    pub burst_observation_threshold: usize,
    pub burst_interval_minutes: i64,
    pub deadline_observation_threshold: usize,
    pub max_pending_minutes: i64,
}

impl Default for WakePolicy {
    fn default() -> Self {
        Self {
            observation_threshold: 12,
            episode_group_threshold: 2,
            normal_interval_minutes: 30,
            burst_observation_threshold: 40,
            burst_interval_minutes: 10,
            deadline_observation_threshold: 3,
            max_pending_minutes: 90,
        }
    }
}

impl WakePolicy {
    pub fn decide(&self, state: &WakeState) -> Option<WakeReason> {
        if state.job_running {
            return None;
        }
        if state.recovery_pending {
            return Some(WakeReason::Recovery);
        }
        if !state.provider_ready || !state.resource_permitted {
            return None;
        }
        if state.pending_observations == 0 {
            return None;
        }
        if state.explicit_request {
            return Some(WakeReason::ExplicitRequest);
        }
        if state.end_of_active_day {
            return Some(WakeReason::EndOfDay);
        }
        if state.pending_observations >= self.burst_observation_threshold
            && state.minutes_since_last_successful_wake >= self.burst_interval_minutes
        {
            return Some(WakeReason::ObservationBurst);
        }
        if state.pending_observations >= self.deadline_observation_threshold
            && state.oldest_pending_minutes >= self.max_pending_minutes
        {
            return Some(WakeReason::PendingDeadline);
        }
        if state.pending_observations >= self.observation_threshold
            && state.pending_episode_groups >= self.episode_group_threshold
            && state.minutes_since_last_successful_wake >= self.normal_interval_minutes
        {
            return Some(WakeReason::ObservationVolume);
        }
        None
    }
}

pub fn local_day(now: DateTime<Utc>, timezone_offset_minutes: i32) -> NaiveDate {
    (now + chrono::Duration::minutes(timezone_offset_minutes as i64)).date_naive()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> WakeState {
        WakeState {
            pending_observations: 12,
            pending_episode_groups: 2,
            minutes_since_last_successful_wake: 30,
            oldest_pending_minutes: 30,
            job_running: false,
            explicit_request: false,
            end_of_active_day: false,
            recovery_pending: false,
            provider_ready: true,
            resource_permitted: true,
        }
    }

    #[test]
    fn coalesces_until_volume_context_and_interval_are_ready() {
        let policy = WakePolicy::default();
        let mut value = state();
        assert_eq!(policy.decide(&value), Some(WakeReason::ObservationVolume));
        value.pending_observations = 11;
        assert_eq!(policy.decide(&value), None);
        value.pending_observations = 12;
        value.pending_episode_groups = 1;
        assert_eq!(policy.decide(&value), None);
        value.pending_episode_groups = 2;
        value.minutes_since_last_successful_wake = 29;
        assert_eq!(policy.decide(&value), None);
    }

    #[test]
    fn burst_and_deadline_bound_latency_without_a_daily_cap() {
        let policy = WakePolicy::default();
        let mut value = state();
        value.pending_observations = 40;
        value.pending_episode_groups = 1;
        value.minutes_since_last_successful_wake = 10;
        assert_eq!(policy.decide(&value), Some(WakeReason::ObservationBurst));

        value.pending_observations = 3;
        value.minutes_since_last_successful_wake = 5;
        value.oldest_pending_minutes = 90;
        assert_eq!(policy.decide(&value), Some(WakeReason::PendingDeadline));
    }

    #[test]
    fn explicit_end_of_day_and_recovery_bypass_batching_thresholds() {
        let policy = WakePolicy::default();
        let mut value = state();
        value.pending_observations = 1;
        value.pending_episode_groups = 1;
        value.minutes_since_last_successful_wake = 0;

        value.explicit_request = true;
        assert_eq!(policy.decide(&value), Some(WakeReason::ExplicitRequest));
        value.explicit_request = false;
        value.end_of_active_day = true;
        assert_eq!(policy.decide(&value), Some(WakeReason::EndOfDay));
        value.end_of_active_day = false;
        value.recovery_pending = true;
        value.provider_ready = false;
        assert_eq!(policy.decide(&value), Some(WakeReason::Recovery));
    }

    #[test]
    fn provider_pressure_and_in_flight_work_preserve_the_queue() {
        let policy = WakePolicy::default();
        let mut value = state();
        value.provider_ready = false;
        assert_eq!(policy.decide(&value), None);
        assert_eq!(value.pending_observations, 12);
        value.provider_ready = true;
        value.job_running = true;
        assert_eq!(policy.decide(&value), None);
    }
}
