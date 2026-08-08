//! Slow, bounded resource gauges for operational telemetry.
//!
//! This module deliberately collects no process names, disk names, paths, or
//! other identifiers. It records only numeric capacity and utilization values.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use dystil_telemetry::{ResourceSnapshot, Telemetry};
use sysinfo::{CpuExt, DiskExt, Pid, PidExt, ProcessExt, System, SystemExt};
use tokio::task::JoinHandle;
use tokio::time::MissedTickBehavior;
use tracing::warn;

const SAMPLE_INTERVAL: Duration = Duration::from_secs(10);
const STORAGE_SAMPLE_INTERVAL: Duration = Duration::from_secs(15 * 60);

pub fn start(telemetry: Arc<Telemetry>, data_dir: PathBuf) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(SAMPLE_INTERVAL);
        interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
        let mut system = System::new();
        let mut last_storage_sample = None;
        let mut last_storage_values = (None, None);

        loop {
            interval.tick().await;
            let include_storage = last_storage_sample
                .is_none_or(|last: std::time::Instant| last.elapsed() >= STORAGE_SAMPLE_INTERVAL);
            if include_storage {
                last_storage_sample = Some(std::time::Instant::now());
            }
            let sample_dir = data_dir.clone();
            // `System` holds the previous CPU sample. Move it into the blocking
            // task and always replace the local slot first, so a task failure
            // cannot leave this loop with a moved value.
            let mut sample_system = std::mem::replace(&mut system, System::new());
            match tokio::task::spawn_blocking(move || {
                let snapshot = collect(&mut sample_system, &sample_dir, include_storage);
                (sample_system, snapshot)
            })
            .await
            {
                Ok((next_system, mut snapshot)) => {
                    system = next_system;
                    if include_storage {
                        last_storage_values = (
                            snapshot.storage_data_bytes,
                            snapshot.storage_available_bytes,
                        );
                    } else {
                        snapshot.storage_data_bytes = last_storage_values.0;
                        snapshot.storage_available_bytes = last_storage_values.1;
                    }
                    telemetry.record_resource_snapshot(snapshot);
                }
                Err(error) => warn!(%error, "resource telemetry sampler stopped"),
            }
        }
    })
}

fn collect(system: &mut System, data_dir: &Path, include_storage: bool) -> ResourceSnapshot {
    system.refresh_cpu();
    system.refresh_memory();
    system.refresh_process(Pid::from_u32(std::process::id()));
    if include_storage {
        system.refresh_disks_list();
        system.refresh_disks();
    }

    let process = system.process(Pid::from_u32(std::process::id()));
    ResourceSnapshot {
        process_cpu_percent_x100: process.and_then(|process| percent_x100(process.cpu_usage())),
        process_memory_rss_bytes: process.map(ProcessExt::memory),
        host_cpu_percent_x100: percent_x100(system.global_cpu_info().cpu_usage()),
        host_memory_available_bytes: Some(system.available_memory()),
        storage_data_bytes: include_storage
            .then(|| crate::disk_usage::directory_size(data_dir).ok().flatten())
            .flatten(),
        storage_available_bytes: include_storage
            .then(|| available_space_for(system, data_dir))
            .flatten(),
    }
}

fn percent_x100(percent: f32) -> Option<u32> {
    if !percent.is_finite() || percent.is_sign_negative() {
        return None;
    }
    Some((percent * 100.0).round().min(u32::MAX as f32) as u32)
}

fn available_space_for(system: &System, path: &Path) -> Option<u64> {
    system
        .disks()
        .iter()
        .filter(|disk| path.starts_with(disk.mount_point()))
        .max_by_key(|disk| disk.mount_point().as_os_str().len())
        .map(DiskExt::available_space)
}

#[cfg(test)]
mod tests {
    use super::percent_x100;

    #[test]
    fn cpu_percent_is_bounded_and_finite() {
        assert_eq!(percent_x100(12.345), Some(1_235));
        assert_eq!(percent_x100(-1.0), None);
        assert_eq!(percent_x100(f32::NAN), None);
    }
}
