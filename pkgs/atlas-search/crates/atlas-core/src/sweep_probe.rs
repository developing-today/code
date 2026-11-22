//! Read-only host probes for Atlas sweep guardrail inputs.
//!
//! Disk capacity/free-space is sampled with `statvfs`; I/O wait is the
//! delta in Linux `/proc/stat` counters across an explicit run window.
//! Target row deltas remain caller-supplied because they come from the
//! Turso usage API, not the host.

use std::ffi::CString;
use std::path::Path;

use crate::error::{Error, Result};
use crate::sweep::Measurements;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct CpuTicks {
    total: u64,
    iowait: u64,
}

/// Captured at the beginning of a run; call [`HostProbe::finish`] at its
/// end so I/O wait measures the run window rather than lifetime average.
#[derive(Clone, Copy, Debug)]
pub struct HostProbe {
    cpu: CpuTicks,
}

impl HostProbe {
    /// Begin a read-only measurement window.
    pub fn begin() -> Result<Self> {
        Ok(Self {
            cpu: read_cpu_ticks()?,
        })
    }

    /// Finish the window and sample source/staging volume capacity.
    /// Database usage deltas and projected batch size are supplied by
    /// the caller and preserved in the returned measurement record.
    pub fn finish(
        self,
        source_path: &Path,
        staging_path: &Path,
        projected_batch_bytes: u64,
        tgt_read_rows: u64,
        tgt_write_rows: u64,
    ) -> Result<Measurements> {
        let source = volume(source_path)?;
        let staging = volume(staging_path)?;
        let end = read_cpu_ticks()?;
        let src_io_wait_pct = iowait_pct(self.cpu, end);
        Ok(Measurements {
            src_free_bytes: source.1,
            src_volume_bytes: source.0,
            src_io_wait_pct,
            stg_free_bytes: staging.1,
            projected_batch_bytes,
            tgt_read_rows,
            tgt_write_rows,
            class_median_write_rows: None,
        })
    }
}

fn iowait_pct(start: CpuTicks, end: CpuTicks) -> f64 {
    let total_delta = end.total.saturating_sub(start.total);
    let wait_delta = end.iowait.saturating_sub(start.iowait);
    if total_delta == 0 {
        0.0
    } else {
        (wait_delta as f64 * 100.0 / total_delta as f64).clamp(0.0, 100.0)
    }
}

fn volume(path: &Path) -> Result<(u64, u64)> {
    let path = CString::new(path.as_os_str().as_encoded_bytes())
        .map_err(|e| Error::fatal(format!("sweep probe: invalid path: {e}")))?;
    // SAFETY: `path` is NUL-terminated and `stat` points to initialized
    // writable storage for the duration of this syscall.
    unsafe {
        let mut stat: libc::statvfs = std::mem::zeroed();
        if libc::statvfs(path.as_ptr(), &mut stat) != 0 {
            return Err(Error::backend(format!(
                "sweep probe: statvfs failed for {}: {}",
                path.to_string_lossy(),
                std::io::Error::last_os_error()
            )));
        }
        let block_size = stat.f_frsize as u64;
        let total = (stat.f_blocks as u64).saturating_mul(block_size);
        let available = (stat.f_bavail as u64).saturating_mul(block_size);
        Ok((total, available))
    }
}

fn read_cpu_ticks() -> Result<CpuTicks> {
    let stat = std::fs::read_to_string("/proc/stat")
        .map_err(|e| Error::backend(format!("sweep probe: reading /proc/stat: {e}")))?;
    let line = stat
        .lines()
        .find(|line| line.starts_with("cpu "))
        .ok_or_else(|| Error::backend("sweep probe: /proc/stat has no aggregate cpu row"))?;
    let ticks = line
        .split_ascii_whitespace()
        .skip(1)
        .map(|value| {
            value.parse::<u64>().map_err(|e| {
                Error::backend(format!("sweep probe: parsing /proc/stat cpu ticks: {e}"))
            })
        })
        .collect::<Result<Vec<_>>>()?;
    if ticks.len() < 5 {
        return Err(Error::backend(
            "sweep probe: /proc/stat aggregate cpu row has fewer than five counters",
        ));
    }
    Ok(CpuTicks {
        // Linux appends guest/guest_nice counters, which are already
        // included in user/nice and must not be counted twice.
        total: ticks.iter().take(8).copied().sum(),
        iowait: ticks[4],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Oracle: a host probe against existing fixture directories returns
    /// nonzero volume capacity, bounded I/O percentage, and preserves the
    /// externally measured target counters.
    #[test]
    fn probe_samples_volumes_and_run_window_io_wait() {
        let root = std::env::temp_dir();
        let probe = HostProbe::begin().expect("begin host probe");
        let measurements = probe
            .finish(&root, &root, 4096, 17, 23)
            .expect("finish host probe");
        assert!(measurements.src_volume_bytes > 0);
        assert!(measurements.src_free_bytes > 0);
        assert!(measurements.stg_free_bytes > 0);
        assert!((0.0..=100.0).contains(&measurements.src_io_wait_pct));
        assert_eq!(measurements.projected_batch_bytes, 4096);
        assert_eq!(measurements.tgt_read_rows, 17);
        assert_eq!(measurements.tgt_write_rows, 23);
    }

    /// Oracle: CPU wait percentage uses the iowait delta, not idle+iowait.
    #[test]
    fn run_window_wait_ratio_is_bounded() {
        let start = CpuTicks {
            total: 100,
            iowait: 5,
        };
        let end = CpuTicks {
            total: 200,
            iowait: 25,
        };
        assert_eq!(iowait_pct(start, end), 20.0);
        assert_eq!(iowait_pct(end, end), 0.0);
    }
}
