//! Local hardware detection (RAM, CPU, unified memory). No network access.

use serde::Serialize;

/// 2.5 GiB reserved for the OS and other apps on unified-memory machines.
const UNIFIED_RESERVE_BYTES: u64 = 5 * 1024 * 1024 * 1024 / 2;

/// What the fit estimator needs to know about this machine.
#[derive(Serialize, Clone, Debug)]
pub struct HardwareInfo {
    pub os: String,
    pub arch: String,
    pub total_ram_bytes: u64,
    pub available_ram_bytes: u64,
    pub cpu_cores: usize,
    /// Apple Silicon (CPU and GPU share memory).
    pub unified_memory: bool,
    /// Memory available to the GPU. Apple Silicon: the unified budget
    /// `(total − 2.5 GiB) × 0.9`. Elsewhere `None` (unknown until an engine
    /// reports its devices).
    pub gpu_budget_bytes: Option<u64>,
}

/// Unified-memory GPU budget: `(total − 2.5 GiB) × 0.9`, never negative.
pub fn unified_budget(total_ram_bytes: u64) -> u64 {
    let usable = total_ram_bytes.saturating_sub(UNIFIED_RESERVE_BYTES);
    (usable as f64 * 0.9) as u64
}

/// Detect the current machine.
pub fn detect() -> HardwareInfo {
    let mut sys = sysinfo::System::new();
    sys.refresh_memory();
    let total = sys.total_memory();
    let available = sys.available_memory();
    let cpu_cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    let unified_memory = cfg!(all(target_os = "macos", target_arch = "aarch64"));
    HardwareInfo {
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        total_ram_bytes: total,
        available_ram_bytes: available,
        cpu_cores,
        unified_memory,
        gpu_budget_bytes: unified_memory.then(|| unified_budget(total)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GIB: u64 = 1024 * 1024 * 1024;

    #[test]
    fn budget_formula() {
        assert_eq!(unified_budget(0), 0);
        assert_eq!(unified_budget(2 * GIB), 0);
        let b = unified_budget(16 * GIB);
        let expected = (13.5 * 0.9 * GIB as f64) as u64;
        assert!(b.abs_diff(expected) < 1024);
    }

    #[test]
    fn detect_is_sane() {
        let hw = detect();
        assert!(hw.cpu_cores >= 1);
        assert!(hw.total_ram_bytes > 0);
        assert!(hw.available_ram_bytes <= hw.total_ram_bytes);
        assert_eq!(hw.unified_memory, hw.gpu_budget_bytes.is_some());
        assert!(!hw.os.is_empty());
    }
}
