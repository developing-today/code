//! Step 0 — the TigerBeetle test harness.
//!
//! A throwaway single-replica TigerBeetle dev cluster the PoC test
//! suite can start and stop, sanctioned for the PoC by the cluster
//! topology investigation (`doc/2026-10-29T11-57-00Z_investigation_tigerbeetle_cluster_topology`:
//! "A separate throwaway 3-replica (or single-replica) dev cluster for
//! the PoC is fine"). It holds no real value and is deleted, never
//! migrated.
//!
//! The binary is pinned: [`TIGERBEETLE_PINNED_VERSION`] is verified
//! against `tigerbeetle version` output before a cluster starts, so a
//! developer-machine-only or drifted binary fails loudly instead of
//! silently testing against the wrong release. Acquisition is either
//! the release zip (URL pattern below, the one the TigerBeetle
//! upgrading docs use) or a nix-provided binary — in both cases the
//! path is supplied via `TIGERBEETLE_POC_BINARY` or `PATH`.

use std::fs;
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

/// The pinned TigerBeetle release for the PoC harness (latest release
/// at the time of pinning, verified via the GitHub releases API).
pub const TIGERBEETLE_PINNED_VERSION: &str = "0.17.9";

/// Release-zip URL for the pinned version on x86_64 Linux (asset
/// naming follows the TigerBeetle upgrading documentation).
pub const TIGERBEETLE_RELEASE_ZIP: &str = "https://github.com/tigerbeetle/tigerbeetle/releases/download/0.17.9/tigerbeetle-x86_64-linux.zip";

/// Environment variable naming an explicit binary path.
pub const BINARY_ENV: &str = "TIGERBEETLE_POC_BINARY";

/// Resolve the harness binary: `$TIGERBEETLE_POC_BINARY`, else
/// `tigerbeetle` on `PATH`.
pub fn binary_path() -> PathBuf {
    std::env::var_os(BINARY_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("tigerbeetle"))
}

/// Verify the resolved binary reports the pinned version.
pub fn verify_pinned_version(binary: &PathBuf) -> Result<()> {
    let output = Command::new(binary)
        .arg("version")
        .output()
        .with_context(|| format!("running {} version", binary.display()))?;
    let text = String::from_utf8_lossy(&output.stdout);
    if !output.status.success() || !text.contains(TIGERBEETLE_PINNED_VERSION) {
        bail!(
            "tigerbeetle binary at {} reports {:?}, expected pinned version {} \
             (acquire it from {})",
            binary.display(),
            text.trim(),
            TIGERBEETLE_PINNED_VERSION,
            TIGERBEETLE_RELEASE_ZIP
        );
    }
    Ok(())
}

/// A running throwaway single-replica cluster. Killed and deleted on drop.
pub struct TestCluster {
    dir: PathBuf,
    child: Option<Child>,
    /// The `host:port` address clients should connect to.
    pub address: String,
}

impl TestCluster {
    /// Format a fresh data file and start one replica on a loopback
    /// port. `port` is chosen by the caller (tests use distinct ports).
    pub fn start(port: u16) -> Result<Self> {
        let binary = binary_path();
        verify_pinned_version(&binary)?;
        let dir =
            std::env::temp_dir().join(format!("id-currency-poc-tb-{}-{port}", std::process::id()));
        if dir.exists() {
            fs::remove_dir_all(&dir)?;
        }
        fs::create_dir_all(&dir)?;
        let data_file = dir.join("0_0.tigerbeetle");
        let format = Command::new(&binary)
            .args(["format", "--cluster=0", "--replica=0", "--replica-count=1"])
            .arg(&data_file)
            .output()
            .context("tigerbeetle format")?;
        if !format.status.success() {
            bail!(
                "tigerbeetle format failed: {}",
                String::from_utf8_lossy(&format.stderr)
            );
        }
        let address = format!("127.0.0.1:{port}");
        let child = Command::new(&binary)
            .arg("start")
            .arg(format!("--addresses={address}"))
            .arg(&data_file)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("tigerbeetle start")?;
        let mut cluster = Self {
            dir,
            child: Some(child),
            address,
        };
        cluster.wait_ready(Duration::from_secs(30))?;
        Ok(cluster)
    }

    fn wait_ready(&mut self, timeout: Duration) -> Result<()> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Ok(_stream) = TcpStream::connect(&self.address) {
                // A successful TCP connect is the readiness signal.
                // The replica sends no unsolicited bytes, so any read
                // here would block unboundedly and defeat the deadline
                // this loop is supposed to enforce.
                return Ok(());
            }
            if Instant::now() > deadline {
                bail!(
                    "tigerbeetle replica at {} did not become ready",
                    self.address
                );
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// Stop the replica (the data directory is removed on drop).
    pub fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for TestCluster {
    fn drop(&mut self) {
        self.stop();
        let _ = fs::remove_dir_all(&self.dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Step-0 smoke test: the pinned binary formats and starts a
    /// single-replica cluster and stops cleanly. Ignored by default —
    /// it needs the pinned TigerBeetle binary (see module docs).
    /// Run with: `cargo test -p id currency_poc -- --ignored`
    #[test]
    #[ignore = "requires the pinned tigerbeetle binary (TIGERBEETLE_POC_BINARY or PATH)"]
    fn cluster_starts_and_stops() {
        let mut cluster = TestCluster::start(30011).expect("cluster should start");
        assert!(cluster.address.starts_with("127.0.0.1:"));
        cluster.stop();
    }
}
