//! Build-time cloud configuration. Auth mode is selected exclusively by Cargo
//! features; the endpoint is only injected into cloud-capable release builds.

pub fn cloud_base_url() -> Option<&'static str> {
    option_env!("DYSTIL_CLOUD_BASE_URL")
}

#[cfg(test)]
mod tests {
    use super::cloud_base_url;

    #[test]
    fn community_build_has_no_cloud_url() {
        #[cfg(not(feature = "cloud-sync"))]
        assert!(cloud_base_url().is_none());
    }
}
