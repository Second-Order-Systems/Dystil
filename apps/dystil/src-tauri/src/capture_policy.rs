use dystil_capture::CaptureMode;

pub const fn enterprise_managed() -> bool {
    cfg!(feature = "enterprise-client")
}

/// Product-owned capture policy. Enterprise builds always use FullCapture;
/// community builds honor the persisted `disable_vision` preference.
pub const fn product_capture_mode(disable_vision: bool) -> CaptureMode {
    if !enterprise_managed() && disable_vision {
        CaptureMode::TextOnly
    } else {
        CaptureMode::FullCapture
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vision_preference_selects_the_only_two_product_modes() {
        assert_eq!(product_capture_mode(false), CaptureMode::FullCapture);
        assert_eq!(product_capture_mode(true), CaptureMode::TextOnly);
    }

    #[test]
    fn first_run_defaults_to_text_only() {
        let settings = crate::recording_settings::RecordingSettings::default();
        assert!(settings.disable_vision);
        let expected = if enterprise_managed() {
            CaptureMode::FullCapture
        } else {
            CaptureMode::TextOnly
        };
        assert_eq!(product_capture_mode(settings.disable_vision), expected);
    }
}
