use std::path::PathBuf;

/// Canonical Dystil data directory. Existing installations keep their
/// `~/.dystil` location; this function does not migrate or delete data.
pub fn data_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".dystil")
}
