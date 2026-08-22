use dystil_capture::CaptureMode;

/// Product-owned capture policy. Organization-enabled screenshots use
/// `FullCapture`; user-choice policies honor `disable_vision`.
pub fn product_capture_mode(disable_vision: bool) -> CaptureMode {
    if matches!(
        crate::app_policy::current().capture.screenshots,
        crate::app_policy::ScreenshotPolicy::UserChoice
    ) && disable_vision
    {
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
        let expected = if matches!(
            crate::app_policy::current().capture.screenshots,
            crate::app_policy::ScreenshotPolicy::OrganizationEnabled
        ) {
            CaptureMode::FullCapture
        } else {
            CaptureMode::TextOnly
        };
        assert_eq!(product_capture_mode(settings.disable_vision), expected);
    }
}
