//! Resolves whether operational telemetry may be collected and exported.
//!
//! `Telemetry` starts in [`ConsentDecision::Unknown`], which records nothing and
//! exports nothing. This module is the only place that moves it out of that
//! state, so there is a single answer to "why is telemetry on?".
//!
//! The decision combines four inputs, in precedence order:
//!
//! 1. `DYSTIL_TELEMETRY=0` — always wins, including for enterprise builds.
//! 2. `enterprise-client` — organization-managed, forced on, no prompt.
//! 3. Onboarding not yet complete — withhold, so no payload leaves the machine
//!    before the user has been shown the disclosure.
//! 4. The user's setting, which defaults to on in community builds.

use std::sync::Arc;

use dystil_telemetry::{ConsentDecision, Telemetry, TELEMETRY_CONSENT_VERSION};
use tauri::AppHandle;
use tracing::{debug, warn};

use crate::store::{OnboardingStore, SettingsStore};

/// Why telemetry is in its current state. Logged so support can answer the
/// question without a debugger.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    /// Collecting and exporting.
    Granted,
    /// Explicitly off: `DYSTIL_TELEMETRY=0` or the user's setting.
    Denied,
    /// Not yet decided — onboarding has not completed. Nothing is recorded.
    WaitingForOnboarding,
}

impl Resolution {
    fn decision(self) -> ConsentDecision {
        match self {
            Self::Granted => ConsentDecision::Granted {
                policy_version: TELEMETRY_CONSENT_VERSION,
            },
            Self::Denied => ConsentDecision::Denied,
            Self::WaitingForOnboarding => ConsentDecision::Unknown,
        }
    }
}

/// Compute the resolution without applying it.
pub fn resolve(settings: &SettingsStore, onboarding_completed: bool) -> Resolution {
    if crate::store::telemetry_disabled_by_env() {
        return Resolution::Denied;
    }
    if cfg!(feature = "enterprise-client") {
        // Organizational consent. Deliberately not gated on onboarding: managed
        // deployments have no per-user disclosure step to wait for.
        return Resolution::Granted;
    }
    if !settings.telemetry_enabled {
        return Resolution::Denied;
    }
    if !onboarding_completed {
        return Resolution::WaitingForOnboarding;
    }
    Resolution::Granted
}

/// Read current state and apply it to `telemetry`.
///
/// Safe to call repeatedly — call it at startup, when the setting changes, and
/// when onboarding completes. Revoking consent clears everything already
/// accumulated, which `Telemetry::set_consent` handles.
pub fn apply(app: &AppHandle, telemetry: &Arc<Telemetry>) -> Resolution {
    let settings = match SettingsStore::get(app) {
        Ok(Some(settings)) => settings,
        Ok(None) => SettingsStore::default(),
        Err(error) => {
            // Fail closed. A settings read failure must not silently enable
            // collection the user may have turned off.
            warn!("telemetry consent: settings unreadable ({error}); withholding consent");
            telemetry.set_consent(ConsentDecision::Denied);
            return Resolution::Denied;
        }
    };

    let onboarding_completed = match OnboardingStore::get(app) {
        Ok(Some(onboarding)) => onboarding.is_completed,
        Ok(None) => false,
        Err(error) => {
            warn!("telemetry consent: onboarding state unreadable ({error}); assuming incomplete");
            false
        }
    };

    let resolution = resolve(&settings, onboarding_completed);
    telemetry.set_consent(resolution.decision());
    debug!("telemetry consent resolved: {resolution:?}");
    resolution
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(enabled: bool) -> SettingsStore {
        SettingsStore {
            telemetry_enabled: enabled,
            ..Default::default()
        }
    }

    #[test]
    fn nothing_is_collected_before_onboarding_completes() {
        assert_eq!(
            resolve(&settings(true), false),
            if cfg!(feature = "enterprise-client") {
                Resolution::Granted
            } else {
                Resolution::WaitingForOnboarding
            }
        );
    }

    #[test]
    fn on_by_default_after_onboarding() {
        assert_eq!(resolve(&settings(true), true), Resolution::Granted);
    }

    #[test]
    fn user_can_disable_in_community_builds() {
        let resolution = resolve(&settings(false), true);
        if cfg!(feature = "enterprise-client") {
            // Organization-managed; the local setting does not apply.
            assert_eq!(resolution, Resolution::Granted);
        } else {
            assert_eq!(resolution, Resolution::Denied);
        }
    }
}
