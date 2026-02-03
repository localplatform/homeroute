//! System metrics collection (CPU, memory) for LXC containers.
//! Uses cgroups v2 for accurate container-level metrics.

use std::sync::RwLock;
use std::time::Instant;

use anyhow::Result;
use tokio::fs;
use tracing::warn;

/// Previous CPU stats for calculating percentage.
#[derive(Debug, Clone)]
struct CpuSnapshot {
    /// CPU usage in microseconds from cgroup.
    usage_usec: u64,
    /// Timestamp when this snapshot was taken.
    timestamp: Instant,
}

/// Collects system metrics from cgroups v2 (container-aware).
#[derive(Debug)]
pub struct MetricsCollector {
    prev_cpu: RwLock<Option<CpuSnapshot>>,
    /// Number of CPUs (for percentage calculation).
    num_cpus: u32,
}

impl MetricsCollector {
    /// Create a new metrics collector.
    pub fn new() -> Self {
        // Get number of CPUs
        let num_cpus = std::thread::available_parallelism()
            .map(|p| p.get() as u32)
            .unwrap_or(1);

        Self {
            prev_cpu: RwLock::new(None),
            num_cpus,
        }
    }

    /// Get current memory usage in bytes (from cgroup).
    pub async fn memory_bytes(&self) -> u64 {
        // Try cgroup v2 first
        if let Ok(mem) = self.read_cgroup_memory().await {
            return mem;
        }

        // Fallback to /proc/meminfo
        match self.read_proc_memory().await {
            Ok(mem) => mem,
            Err(e) => {
                warn!("Failed to read memory info: {e}");
                0
            }
        }
    }

    /// Read memory usage from cgroup v2.
    async fn read_cgroup_memory(&self) -> Result<u64> {
        // cgroups v2: /sys/fs/cgroup/memory.current
        let content = fs::read_to_string("/sys/fs/cgroup/memory.current").await?;
        Ok(content.trim().parse()?)
    }

    /// Read memory usage from /proc/meminfo (fallback).
    async fn read_proc_memory(&self) -> Result<u64> {
        let content = fs::read_to_string("/proc/meminfo").await?;

        let mut mem_total: u64 = 0;
        let mut mem_available: u64 = 0;

        for line in content.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                match parts[0] {
                    "MemTotal:" => mem_total = parts[1].parse().unwrap_or(0),
                    "MemAvailable:" => mem_available = parts[1].parse().unwrap_or(0),
                    _ => {}
                }
            }
        }

        // meminfo reports in KB
        let used_kb = mem_total.saturating_sub(mem_available);
        Ok(used_kb * 1024)
    }

    /// Get current CPU usage percentage (0.0 - 100.0) for the container.
    pub async fn cpu_percent(&self) -> f32 {
        // Try cgroup v2 first
        if let Ok(percent) = self.read_cgroup_cpu().await {
            return percent;
        }

        // Fallback to /proc/stat (less accurate for containers)
        match self.read_proc_cpu().await {
            Ok(percent) => percent,
            Err(e) => {
                warn!("Failed to read CPU info: {e}");
                0.0
            }
        }
    }

    /// Read CPU usage from cgroup v2.
    async fn read_cgroup_cpu(&self) -> Result<f32> {
        // cgroups v2: /sys/fs/cgroup/cpu.stat
        let content = fs::read_to_string("/sys/fs/cgroup/cpu.stat").await?;

        let mut usage_usec: u64 = 0;
        for line in content.lines() {
            if line.starts_with("usage_usec ") {
                usage_usec = line.split_whitespace().nth(1)
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
                break;
            }
        }

        let current = CpuSnapshot {
            usage_usec,
            timestamp: Instant::now(),
        };

        let percent = {
            let prev = self.prev_cpu.read().unwrap();
            if let Some(prev) = prev.as_ref() {
                self.calculate_cgroup_cpu_percent(prev, &current)
            } else {
                0.0
            }
        };

        // Update previous stats
        {
            let mut prev = self.prev_cpu.write().unwrap();
            *prev = Some(current);
        }

        Ok(percent)
    }

    /// Calculate CPU percentage from cgroup snapshots.
    fn calculate_cgroup_cpu_percent(&self, prev: &CpuSnapshot, current: &CpuSnapshot) -> f32 {
        let usage_delta = current.usage_usec.saturating_sub(prev.usage_usec);
        let time_delta = current.timestamp.duration_since(prev.timestamp);
        let time_delta_usec = time_delta.as_micros() as u64;

        if time_delta_usec == 0 {
            return 0.0;
        }

        // CPU usage percentage = (usage_delta / (time_delta * num_cpus)) * 100
        // usage_delta is total CPU time used across all CPUs
        let max_usec = time_delta_usec * self.num_cpus as u64;
        let percent = (usage_delta as f64 / max_usec as f64) * 100.0;

        // Clamp to 0-100 range
        percent.clamp(0.0, 100.0) as f32
    }

    /// Fallback: Read CPU stats from /proc/stat (host-level, less accurate for containers).
    async fn read_proc_cpu(&self) -> Result<f32> {
        let content = fs::read_to_string("/proc/stat").await?;

        let mut user: u64 = 0;
        let mut nice: u64 = 0;
        let mut system: u64 = 0;
        let mut idle: u64 = 0;
        let mut iowait: u64 = 0;

        for line in content.lines() {
            if line.starts_with("cpu ") {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 6 {
                    user = parts[1].parse().unwrap_or(0);
                    nice = parts[2].parse().unwrap_or(0);
                    system = parts[3].parse().unwrap_or(0);
                    idle = parts[4].parse().unwrap_or(0);
                    iowait = parts[5].parse().unwrap_or(0);
                }
                break;
            }
        }

        let current = CpuSnapshot {
            usage_usec: user + nice + system,
            timestamp: Instant::now(),
        };

        let prev_idle = {
            let prev = self.prev_cpu.read().unwrap();
            prev.as_ref().map(|p| p.usage_usec).unwrap_or(0)
        };

        let total_current = user + nice + system + idle + iowait;
        let total_prev = prev_idle + idle + iowait; // Approximate

        // Update previous
        {
            let mut prev = self.prev_cpu.write().unwrap();
            *prev = Some(current);
        }

        // This fallback is less accurate
        if total_current > total_prev && total_current > 0 {
            let busy = (user + nice + system) as f32;
            let total = total_current as f32;
            Ok((busy / total) * 100.0)
        } else {
            Ok(0.0)
        }
    }
}

impl Default for MetricsCollector {
    fn default() -> Self {
        Self::new()
    }
}
