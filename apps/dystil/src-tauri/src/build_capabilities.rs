use serde::Serialize;

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct BuildCapabilities {
    pub cloud_available: bool,
    pub auth_mode: AuthMode,
    pub cloud_base_url: Option<String>,
    pub official_build: bool,
    pub enterprise_managed: bool,
}

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum AuthMode {
    Individual,
    Workspace,
}

pub fn current() -> BuildCapabilities {
    BuildCapabilities {
        cloud_available: cfg!(feature = "cloud-sync"),
        auth_mode: if cfg!(feature = "workspace-auth") {
            AuthMode::Workspace
        } else {
            AuthMode::Individual
        },
        cloud_base_url: crate::app_config::cloud_base_url().map(str::to_owned),
        official_build: cfg!(feature = "official-build"),
        enterprise_managed: cfg!(feature = "enterprise-client"),
    }
}

#[tauri::command]
#[specta::specta]
pub fn get_build_capabilities() -> BuildCapabilities {
    current()
}

#[cfg(test)]
mod tests {
    use super::{current, AuthMode};

    #[test]
    fn capabilities_match_compiled_features() {
        let capabilities = current();
        assert_eq!(capabilities.cloud_available, cfg!(feature = "cloud-sync"));
        assert_eq!(
            capabilities.official_build,
            cfg!(feature = "official-build")
        );
        assert_eq!(
            capabilities.enterprise_managed,
            cfg!(feature = "enterprise-client")
        );
        assert!(matches!(
            capabilities.auth_mode,
            AuthMode::Individual | AuthMode::Workspace
        ));
    }
}
