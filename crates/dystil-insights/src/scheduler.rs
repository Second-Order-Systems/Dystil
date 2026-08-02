use chrono::{DateTime, NaiveDate, Utc};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WakeReason {
    ObservationThreshold,
    ThresholdCrossing,
    ActivePeriodEnded,
    ExplicitRequest,
    EndOfDay,
    Recovery,
}

#[derive(Debug, Clone)]
pub struct WakeState {
    pub local_day: NaiveDate,
    pub normal_wakes_started: u8,
    pub pending_observations: usize,
    pub job_running: bool,
    pub threshold_crossing: bool,
    pub active_period_ended: bool,
    pub explicit_request: bool,
    pub end_of_active_day: bool,
    pub recovery_pending: bool,
    pub provider_ready: bool,
    pub resource_permitted: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct WakePolicy {
    pub observation_threshold: usize,
    pub max_normal_wakes_per_day: u8,
}

impl Default for WakePolicy {
    fn default() -> Self {
        Self {
            observation_threshold: 15,
            max_normal_wakes_per_day: 4,
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
        if state.pending_observations == 0 && !state.explicit_request {
            return None;
        }
        if state.normal_wakes_started >= self.max_normal_wakes_per_day {
            return None;
        }
        if state.explicit_request {
            return Some(WakeReason::ExplicitRequest);
        }
        if state.threshold_crossing {
            return Some(WakeReason::ThresholdCrossing);
        }
        if state.pending_observations >= self.observation_threshold {
            return Some(WakeReason::ObservationThreshold);
        }
        if state.active_period_ended {
            return Some(WakeReason::ActivePeriodEnded);
        }
        if state.end_of_active_day && state.normal_wakes_started == 0 {
            return Some(WakeReason::EndOfDay);
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
            local_day: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            normal_wakes_started: 0,
            pending_observations: 15,
            job_running: false,
            threshold_crossing: false,
            active_period_ended: false,
            explicit_request: false,
            end_of_active_day: false,
            recovery_pending: false,
            provider_ready: true,
            resource_permitted: true,
        }
    }

    #[test]
    fn coalesces_while_a_job_is_running_and_caps_normal_wakes() {
        let policy = WakePolicy::default();
        let mut value = state();
        value.job_running = true;
        assert_eq!(policy.decide(&value), None);
        value.job_running = false;
        value.normal_wakes_started = 4;
        assert_eq!(policy.decide(&value), None);
    }

    #[test]
    fn recovery_does_not_consume_a_normal_wake() {
        let policy = WakePolicy::default();
        let mut value = state();
        value.normal_wakes_started = 4;
        value.recovery_pending = true;
        assert_eq!(policy.decide(&value), Some(WakeReason::Recovery));
    }

    #[test]
    fn provider_pressure_postpones_without_consuming_pending_work() {
        let policy = WakePolicy::default();
        let mut value = state();
        value.provider_ready = false;
        assert_eq!(policy.decide(&value), None);
        assert_eq!(value.pending_observations, 15);
    }
}
