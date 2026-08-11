use std::path::PathBuf;

fn data_dir_from_override(value: Option<std::ffi::OsString>) -> Option<PathBuf> {
    value.map(PathBuf::from).filter(|path| path.is_absolute())
}

/// Canonical Dystil data directory. Existing installations keep their
/// `~/.dystil` location; this function does not migrate or delete data.
pub fn data_dir() -> PathBuf {
    if let Some(path) = data_dir_from_override(std::env::var_os("DYSTIL_DATA_DIR")) {
        return path;
    }

    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".dystil")
}

#[cfg(test)]
mod tests {
    use super::data_dir_from_override;

    #[test]
    fn accepts_an_absolute_data_dir_override() {
        assert_eq!(
            data_dir_from_override(Some("/tmp/dystil-fixture".into())),
            Some("/tmp/dystil-fixture".into())
        );
    }

    #[test]
    fn rejects_a_relative_data_dir_override() {
        assert_eq!(data_dir_from_override(Some("fixture".into())), None);
    }
}
