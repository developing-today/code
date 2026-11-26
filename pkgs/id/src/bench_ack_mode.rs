//! Benchmark-only journal ack mode selector (#557).
//!
//! The `many_worlds` bench alone opts into the relaxed [`JournalAckMode`]s
//! via `ID_BENCH_MW_JOURNAL_ACK_MODE` (`sync` default, `batched`, `async`).
//! Nothing outside the bench calls this module: the production default
//! stays [`JournalAckMode::Sync`] and no durability contract changes.

pub use id_worlds::world::JournalAckMode;

/// Read the bench-only ack mode override, falling back to the production
/// default [`JournalAckMode::Sync`] when the variable is unset or
/// unparseable (the same fallback rule as the bench's `mw_env` helper).
pub fn mw_journal_ack_mode() -> JournalAckMode {
    parse_journal_ack_mode(&std::env::var("ID_BENCH_MW_JOURNAL_ACK_MODE").unwrap_or_default())
}

/// Map one raw selector value to its mode. Anything but an exact
/// (case- and whitespace-tolerant) `batched` / `async` resolves to
/// [`JournalAckMode::Sync`].
pub fn parse_journal_ack_mode(value: &str) -> JournalAckMode {
    match value.trim().to_ascii_lowercase().as_str() {
        "batched" => JournalAckMode::Batched,
        "async" => JournalAckMode::Async,
        _ => JournalAckMode::Sync,
    }
}

/// Machine-readable name of a [`JournalAckMode`] for the
/// `bench many_worlds` result line (`journal_ack_mode=<name>`).
pub fn journal_ack_mode_name(mode: JournalAckMode) -> &'static str {
    match mode {
        JournalAckMode::Sync => "sync",
        JournalAckMode::Batched => "batched",
        JournalAckMode::Async => "async",
    }
}

#[cfg(test)]
mod tests {
    use super::{JournalAckMode, journal_ack_mode_name, parse_journal_ack_mode};

    #[test]
    fn accepted_values_map_to_modes() {
        assert_eq!(parse_journal_ack_mode("sync"), JournalAckMode::Sync);
        assert_eq!(parse_journal_ack_mode("batched"), JournalAckMode::Batched);
        assert_eq!(parse_journal_ack_mode("async"), JournalAckMode::Async);
    }

    #[test]
    fn matching_is_case_and_whitespace_tolerant() {
        assert_eq!(
            parse_journal_ack_mode("  Batched "),
            JournalAckMode::Batched
        );
        assert_eq!(parse_journal_ack_mode("ASYNC"), JournalAckMode::Async);
    }

    #[test]
    fn unset_or_garbage_falls_back_to_sync_default() {
        assert_eq!(parse_journal_ack_mode(""), JournalAckMode::Sync);
        assert_eq!(
            parse_journal_ack_mode("fsync-everything"),
            JournalAckMode::Sync
        );
    }

    #[test]
    fn machine_readable_names_cover_every_mode() {
        assert_eq!(journal_ack_mode_name(JournalAckMode::Sync), "sync");
        assert_eq!(journal_ack_mode_name(JournalAckMode::Batched), "batched");
        assert_eq!(journal_ack_mode_name(JournalAckMode::Async), "async");
    }
}
