//! Serve command and lock file management

use anyhow::Result;
use iroh_base::EndpointId;
use std::net::SocketAddr;
use tokio::fs as afs;

use crate::SERVE_LOCK;

/// Info about a running serve instance
#[derive(Debug, Clone)]
pub struct ServeInfo {
    pub node_id: EndpointId,
    pub addrs: Vec<SocketAddr>,
}

/// Check if serve is running by reading the lock file and verifying the PID
pub async fn get_serve_info() -> Option<ServeInfo> {
    let contents = afs::read_to_string(SERVE_LOCK).await.ok()?;
    let mut lines = contents.lines();
    let node_id_str = lines.next()?;
    let pid_str = lines.next()?;
    let pid: u32 = pid_str.parse().ok()?;

    // Check if process is still alive
    if !is_process_alive(pid) {
        // Stale lock file - remove it
        let _ = afs::remove_file(SERVE_LOCK).await;
        return None;
    }

    let node_id: EndpointId = node_id_str.parse().ok()?;

    // Parse socket addresses (remaining lines)
    let addrs: Vec<SocketAddr> = lines.filter_map(|line| line.parse().ok()).collect();

    Some(ServeInfo { node_id, addrs })
}

/// Check if a process with the given PID is still running
pub fn is_process_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        // kill -0 checks existence without sending a signal
        unsafe { libc::kill(pid as i32, 0) == 0 }
    }
    #[cfg(not(unix))]
    {
        // On non-Unix, just assume it's alive if we have a PID
        let _ = pid;
        true
    }
}

/// Create serve lock file with node ID, PID, and socket addresses
pub async fn create_serve_lock(node_id: &EndpointId, addrs: &[SocketAddr]) -> Result<()> {
    let pid = std::process::id();
    let mut contents = format!("{}\n{}", node_id, pid);
    for addr in addrs {
        contents.push_str(&format!("\n{}", addr));
    }
    afs::write(SERVE_LOCK, contents).await?;
    Ok(())
}

/// Remove serve lock file
pub async fn remove_serve_lock() -> Result<()> {
    let _ = afs::remove_file(SERVE_LOCK).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_process_alive_current_process() {
        let pid = std::process::id();
        assert!(is_process_alive(pid));
    }

    #[test]
    fn test_is_process_alive_nonexistent() {
        // Use a very high PID that's unlikely to exist
        // Note: On non-Unix this always returns true
        #[cfg(unix)]
        {
            assert!(!is_process_alive(999999999));
        }
    }

    #[test]
    fn test_is_process_alive_pid_1() {
        // PID 1 (init) should exist on Unix systems, but may not be visible
        // in containerized environments where the container has its own PID namespace
        #[cfg(unix)]
        {
            // Just check that the function doesn't panic - the result depends on environment
            let _ = is_process_alive(1);
        }
    }

    #[test]
    fn test_serve_info_struct() {
        use iroh_base::SecretKey;
        
        let key = SecretKey::generate(&mut rand::rng());
        let node_id = key.public();
        let addrs = vec![
            "127.0.0.1:8080".parse().unwrap(),
            "[::1]:8080".parse().unwrap(),
        ];
        
        let info = ServeInfo {
            node_id,
            addrs: addrs.clone(),
        };
        
        assert_eq!(info.node_id, node_id);
        assert_eq!(info.addrs.len(), 2);
        assert_eq!(info.addrs[0].to_string(), "127.0.0.1:8080");
    }

    #[test]
    fn test_serve_info_clone() {
        use iroh_base::SecretKey;
        
        let key = SecretKey::generate(&mut rand::rng());
        let node_id = key.public();
        let info = ServeInfo {
            node_id,
            addrs: vec!["127.0.0.1:8080".parse().unwrap()],
        };
        
        let cloned = info.clone();
        assert_eq!(cloned.node_id, info.node_id);
        assert_eq!(cloned.addrs, info.addrs);
    }

    // Integration tests for lock file functions require file system access
    // and are tested via the integration test suite
}
