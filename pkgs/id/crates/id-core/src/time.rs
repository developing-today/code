//! Small time helpers shared by the sync protocol and the world runtime.
//!
//! Kept in its own ungated module so the crate builds without the `world`
//! feature (`crate::log_sync` and `crate::envelope_net` both need it).

/// Milliseconds since the Unix epoch; 0 if the clock is before it.
pub fn unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as u64)
}
