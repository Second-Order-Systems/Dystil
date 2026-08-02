use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

#[derive(Debug, Clone, Copy)]
pub struct DiagnosticRetention {
    pub successful_ttl: Duration,
    pub rejected_ttl: Duration,
    pub metadata_ttl: Duration,
    pub max_bytes: u64,
}

impl Default for DiagnosticRetention {
    fn default() -> Self {
        Self {
            successful_ttl: Duration::from_secs(24 * 60 * 60),
            rejected_ttl: Duration::from_secs(7 * 24 * 60 * 60),
            metadata_ttl: Duration::from_secs(30 * 24 * 60 * 60),
            max_bytes: 50 * 1024 * 1024,
        }
    }
}

impl DiagnosticRetention {
    pub fn enhanced() -> Self {
        Self {
            successful_ttl: Duration::from_secs(14 * 24 * 60 * 60),
            rejected_ttl: Duration::from_secs(14 * 24 * 60 * 60),
            metadata_ttl: Duration::from_secs(30 * 24 * 60 * 60),
            max_bytes: 250 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct CleanupResult {
    pub removed_files: usize,
    pub removed_bytes: u64,
    pub retained_bytes: u64,
}

fn class_ttl(path: &Path, policy: DiagnosticRetention) -> Duration {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    if name.contains("rejected") || name.contains("stderr") {
        policy.rejected_ttl
    } else if name.contains("metadata") || name.contains("usage") {
        policy.metadata_ttl
    } else {
        policy.successful_ttl
    }
}

pub fn cleanup_diagnostics(
    root: &Path,
    now: SystemTime,
    policy: DiagnosticRetention,
) -> std::io::Result<CleanupResult> {
    if !root.exists() {
        return Ok(CleanupResult::default());
    }
    let mut files: Vec<(PathBuf, SystemTime, u64)> = fs::read_dir(root)?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let metadata = entry.metadata().ok()?;
            if !metadata.is_file() {
                return None;
            }
            Some((entry.path(), metadata.modified().ok()?, metadata.len()))
        })
        .collect();
    let mut result = CleanupResult::default();
    files.retain(|(path, modified, bytes)| {
        let expired = now.duration_since(*modified).unwrap_or_default() > class_ttl(path, policy);
        if expired && fs::remove_file(path).is_ok() {
            result.removed_files += 1;
            result.removed_bytes += *bytes;
            false
        } else {
            true
        }
    });
    files.sort_by_key(|(_, modified, _)| *modified);
    let mut total = files.iter().map(|(_, _, bytes)| *bytes).sum::<u64>();
    for (path, _, bytes) in files {
        if total <= policy.max_bytes {
            break;
        }
        if fs::remove_file(path).is_ok() {
            total = total.saturating_sub(bytes);
            result.removed_files += 1;
            result.removed_bytes += bytes;
        }
    }
    result.retained_bytes = total;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use std::{fs::File, io::Write};

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn size_cleanup_removes_oldest_without_touching_other_directories() {
        let directory = tempdir().unwrap();
        for name in ["first.success", "second.success", "usage.metadata"] {
            let mut file = File::create(directory.path().join(name)).unwrap();
            file.write_all(&[0; 8]).unwrap();
            std::thread::sleep(Duration::from_millis(5));
        }
        let result = cleanup_diagnostics(
            directory.path(),
            SystemTime::now(),
            DiagnosticRetention {
                max_bytes: 10,
                ..DiagnosticRetention::default()
            },
        )
        .unwrap();
        assert_eq!(result.retained_bytes, 8);
        assert_eq!(result.removed_files, 2);
    }

    #[test]
    fn ttl_cleanup_uses_artifact_class_limits() {
        let directory = tempdir().unwrap();
        for name in ["accepted.success", "bad.rejected", "totals.usage"] {
            File::create(directory.path().join(name)).unwrap();
        }
        std::thread::sleep(Duration::from_millis(2));
        let result = cleanup_diagnostics(
            directory.path(),
            SystemTime::now(),
            DiagnosticRetention {
                successful_ttl: Duration::ZERO,
                rejected_ttl: Duration::from_secs(60),
                metadata_ttl: Duration::from_secs(60),
                max_bytes: u64::MAX,
            },
        )
        .unwrap();
        assert_eq!(result.removed_files, 1);
        assert!(!directory.path().join("accepted.success").exists());
        assert!(directory.path().join("bad.rejected").exists());
        assert!(directory.path().join("totals.usage").exists());
    }
}
