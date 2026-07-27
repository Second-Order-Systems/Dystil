use dystil_capture::CaptureMode;

/// Product-owned capture policy. The persisted `disable_vision` preference is
/// the only selector: it makes the permission and privacy behavior explicit
/// without build flags or environment-only modes.
pub const fn product_capture_mode(disable_vision: bool) -> CaptureMode {
    if disable_vision {
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
        assert_eq!(
            product_capture_mode(settings.disable_vision),
            CaptureMode::TextOnly
        );
    }
}
