use serde::{Deserialize, Serialize};
use specta::Type;
use sysinfo::SystemExt;

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct HardwareCapability {
    pub has_gpu: bool,
    pub cpu_cores: usize,
    pub total_memory_gb: f64,
    pub recommended_engine: String,
    pub reason: String,
}

pub fn detect_hardware_capability() -> HardwareCapability {
    // Dystil currently ships the CPU ONNX redactor only. Keep this explicit
    // until a platform-specific execution provider is actually supported.
    let has_gpu = false;

    // Only refresh CPU + memory — avoid new_all() which enumerates all
    // processes/disks/networks and can take hundreds of ms.
    let mut sys = sysinfo::System::new();
    sys.refresh_cpu();
    sys.refresh_memory();
    let cpu_cores = sys.cpus().len();
    let total_memory_gb = sys.total_memory() as f64 / (1024.0 * 1024.0 * 1024.0);

    let reason = format!(
        "Screen capture and local PII redaction ({} cores, {:.1} GB RAM)",
        cpu_cores, total_memory_gb
    );

    HardwareCapability {
        has_gpu,
        cpu_cores,
        total_memory_gb,
        recommended_engine: "not-applicable".to_string(),
        reason,
    }
}

#[tauri::command]
#[specta::specta]
pub fn get_hardware_capability() -> HardwareCapability {
    detect_hardware_capability()
}
