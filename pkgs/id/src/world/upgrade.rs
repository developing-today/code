//! Program upgrade with state migration and rollback (F12.9, #1066).
//!
//! A world program's state survives a program upgrade through a
//! *versioned snapshot*: the snapshot envelope records its format
//! version, and a guest may provide an optional `migrate` export that
//! converts its state from the old program version's encoding to the
//! new one's. Programs without a `migrate` export keep working under
//! the pre-existing behavior — their state is carried across the
//! upgrade byte-for-byte.
//!
//! An upgrade never mutates the running program in place. It runs on
//! a probe copy of the current state first (the way checkpoints
//! already isolate state), migrates there, re-encodes the migrated
//! state as a current-format snapshot, and reads that snapshot back.
//! The upgrade commits — the running program's version and snapshot
//! are swapped — only if the migrated state round-trips exactly.
//! Any failure (the guest's `migrate` refuses, the migrated state is
//! over the size bound, the round-trip differs) rolls the upgrade
//! back: the old program keeps running with its original version and
//! its original snapshot bytes, untouched.
//!
//! Invariants (the oracle every test in this module names):
//!
//! - A snapshot this module encodes decodes back to the same format
//!   version and the same state bytes.
//! - [`upgrade_program`] either commits a program whose snapshot
//!   decodes to exactly the migrated state, or leaves the program
//!   bit-for-bit unchanged.
//! - A program with no `migrate` export upgrades with its state
//!   carried unchanged; only its version and the snapshot envelope
//!   format may move.

use anyhow::{Context, Result, bail, ensure};

/// Magic opening every program-state snapshot, all format versions.
pub const SNAPSHOT_MAGIC: [u8; 4] = *b"IDWS";

/// The first snapshot format: magic, a one-byte format version, then
/// the raw guest state to the end of the buffer. Written before the
/// envelope grew a length and a checksum; still read so a program
/// snapshotted by an old binary can be upgraded by a new one.
pub const SNAPSHOT_FORMAT_V1: u16 = 1;

/// The current snapshot format: magic, a two-byte little-endian
/// format version, a four-byte little-endian state length, the guest
/// state, and a four-byte FNV-1a checksum of the state. Everything
/// this module encodes uses this format.
pub const SNAPSHOT_FORMAT_CURRENT: u16 = 2;

/// The largest guest program state an upgrade will carry, in bytes.
/// A migration whose output exceeds this bound fails on the probe
/// and rolls back; it never truncates.
pub const MAX_PROGRAM_STATE_BYTES: usize = 4 * 1024 * 1024;

/// FNV-1a 32-bit checksum, the same hash the world metrics module
/// uses for region names. It guards the snapshot against torn or
/// truncated writes; it is not a signature.
fn state_checksum(state: &[u8]) -> u32 {
    let mut hash: u32 = 0x811c_9dc5;
    for &byte in state {
        hash ^= u32::from(byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash
}

/// A decoded program-state snapshot: the envelope's format version
/// and the guest state it carried.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProgramSnapshot {
    /// The snapshot envelope format the state was read from (or will
    /// be written as, for a freshly encoded snapshot).
    pub format_version: u16,
    /// The guest program's opaque state bytes.
    pub state: Vec<u8>,
}

impl ProgramSnapshot {
    /// Wrap guest `state` in a current-format snapshot.
    pub fn new(state: Vec<u8>) -> Self {
        Self {
            format_version: SNAPSHOT_FORMAT_CURRENT,
            state,
        }
    }

    /// Encode the snapshot to its wire bytes, at its recorded format
    /// version. Encoding a legacy (v1) snapshot re-emits the v1
    /// layout, so a decode/encode pair is lossless for both formats.
    pub fn encode(&self) -> Result<Vec<u8>> {
        ensure!(
            self.state.len() <= MAX_PROGRAM_STATE_BYTES,
            "program state of {} bytes exceeds the {MAX_PROGRAM_STATE_BYTES}-byte bound",
            self.state.len()
        );
        match self.format_version {
            SNAPSHOT_FORMAT_V1 => {
                let mut out = Vec::with_capacity(5 + self.state.len());
                out.extend_from_slice(&SNAPSHOT_MAGIC);
                out.push(1);
                out.extend_from_slice(&self.state);
                Ok(out)
            }
            SNAPSHOT_FORMAT_CURRENT => {
                let mut out = Vec::with_capacity(11 + self.state.len());
                out.extend_from_slice(&SNAPSHOT_MAGIC);
                out.extend_from_slice(&SNAPSHOT_FORMAT_CURRENT.to_le_bytes());
                let len = u32::try_from(self.state.len())
                    .context("program state length does not fit the snapshot header")?;
                out.extend_from_slice(&len.to_le_bytes());
                out.extend_from_slice(&self.state);
                out.extend_from_slice(&state_checksum(&self.state).to_le_bytes());
                Ok(out)
            }
            other => bail!("unsupported snapshot format version {other}"),
        }
    }

    /// Decode snapshot wire bytes of any supported format version.
    /// The returned snapshot records the format it was read from, so
    /// a caller can tell a legacy snapshot apart from a current one.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        ensure!(
            bytes.len() >= 5 && bytes[..4] == SNAPSHOT_MAGIC,
            "not a program-state snapshot (bad magic)"
        );
        // The v1 header is magic + one version byte; the v2 header is
        // magic + a two-byte version whose high byte is 0, so byte 4
        // alone distinguishes the layouts that exist today.
        if bytes[4] == 1 {
            let state = bytes[5..].to_vec();
            ensure!(
                state.len() <= MAX_PROGRAM_STATE_BYTES,
                "program state of {} bytes exceeds the {MAX_PROGRAM_STATE_BYTES}-byte bound",
                state.len()
            );
            return Ok(Self {
                format_version: SNAPSHOT_FORMAT_V1,
                state,
            });
        }
        ensure!(bytes.len() >= 11, "truncated current-format snapshot");
        let format_version = u16::from_le_bytes([bytes[4], bytes[5]]);
        ensure!(
            format_version == SNAPSHOT_FORMAT_CURRENT,
            "unsupported snapshot format version {format_version}"
        );
        let len = u32::from_le_bytes([bytes[6], bytes[7], bytes[8], bytes[9]]) as usize;
        ensure!(
            len <= MAX_PROGRAM_STATE_BYTES,
            "program state of {len} bytes exceeds the {MAX_PROGRAM_STATE_BYTES}-byte bound"
        );
        ensure!(
            bytes.len() == 10 + len + 4,
            "snapshot length header ({len} state bytes) does not match the buffer ({} bytes)",
            bytes.len()
        );
        let state = bytes[10..10 + len].to_vec();
        let recorded = u32::from_le_bytes([
            bytes[10 + len],
            bytes[11 + len],
            bytes[12 + len],
            bytes[13 + len],
        ]);
        ensure!(
            recorded == state_checksum(&state),
            "snapshot state checksum mismatch"
        );
        Ok(Self {
            format_version,
            state,
        })
    }
}

/// A guest program's optional `migrate` export, as the host calls it:
/// given the state bytes written by program version `from_version`,
/// return the state bytes for program version `to_version`. A guest
/// that does not export `migrate` is represented by `None`, never by
/// a stub that errors.
pub type MigrateExport<'a> = &'a dyn Fn(&[u8], u32, u32) -> Result<Vec<u8>>;

/// One installed world program, as the upgrade runner sees it: its
/// program version and the snapshot of its state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunningProgram {
    /// The installed program's version.
    pub version: u32,
    /// Wire bytes of the program's state snapshot (any supported
    /// snapshot format).
    pub snapshot: Vec<u8>,
}

/// How a committed upgrade changed the program.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpgradeOutcome {
    /// The version moved and the state was migrated by the guest's
    /// `migrate` export.
    Migrated,
    /// The version moved (or the snapshot format was refreshed) with
    /// the state carried byte-for-byte: the guest exports no
    /// `migrate`, so its state encoding did not change.
    CarriedUnchanged,
}

/// Run an upgrade of `program` to `target_version` on a probe copy
/// of its state, committing only if the migrated state round-trips
/// through the current snapshot format.
///
/// Probe-then-commit is the whole contract: every fallible step —
/// decoding the current snapshot, the guest's `migrate`, encoding
/// and re-decoding the migrated state — happens against the probe.
/// `program` is mutated only after all of them succeed, so any
/// error return guarantees the old program is still running with
/// its original version and snapshot bytes (the rollback).
pub fn upgrade_program(
    program: &mut RunningProgram,
    target_version: u32,
    migrate: Option<MigrateExport<'_>>,
) -> Result<UpgradeOutcome> {
    // Probe: a decoded copy of the live state. The live snapshot is
    // not touched while the probe is worked on.
    let probe = ProgramSnapshot::decode(&program.snapshot)
        .context("current program snapshot does not decode; upgrade not attempted")?;

    let (migrated, outcome) = if target_version == program.version {
        (probe.state.clone(), UpgradeOutcome::CarriedUnchanged)
    } else if let Some(migrate) = migrate {
        (
            migrate(&probe.state, program.version, target_version)
                .context("guest migrate export failed; upgrade rolled back")?,
            UpgradeOutcome::Migrated,
        )
    } else {
        (probe.state.clone(), UpgradeOutcome::CarriedUnchanged)
    };

    // Re-encode at the current snapshot format and read it back: the
    // upgrade commits only if the migrated state round-trips exactly.
    let committed = ProgramSnapshot::new(migrated);
    let wire = committed
        .encode()
        .context("migrated state does not encode; upgrade rolled back")?;
    let read_back = ProgramSnapshot::decode(&wire)
        .context("migrated snapshot does not decode; upgrade rolled back")?;
    ensure!(
        read_back == committed,
        "migrated state does not round-trip; upgrade rolled back"
    );

    // Commit: the single point where the running program changes.
    program.version = target_version;
    program.snapshot = wire;
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Oracle: a snapshot encodes and decodes to itself, with the
    /// format version recorded in the decoded snapshot.
    #[test]
    fn current_snapshot_round_trips_and_records_its_format() {
        let snapshot = ProgramSnapshot::new(b"counter=41".to_vec());
        let wire = snapshot.encode().expect("encode");
        let decoded = ProgramSnapshot::decode(&wire).expect("decode");
        assert_eq!(decoded, snapshot);
        assert_eq!(decoded.format_version, SNAPSHOT_FORMAT_CURRENT);
    }

    /// Oracle: a legacy v1 snapshot (magic + one version byte + raw
    /// state) still decodes, and reports the v1 format.
    #[test]
    fn legacy_v1_snapshot_still_decodes() {
        let mut wire = Vec::new();
        wire.extend_from_slice(&SNAPSHOT_MAGIC);
        wire.push(1);
        wire.extend_from_slice(b"legacy-state");
        let decoded = ProgramSnapshot::decode(&wire).expect("decode v1");
        assert_eq!(decoded.format_version, SNAPSHOT_FORMAT_V1);
        assert_eq!(decoded.state, b"legacy-state");
    }

    /// Oracle: a corrupted current-format snapshot is refused, not
    /// silently read.
    #[test]
    fn corrupted_snapshot_is_refused() {
        let mut wire = ProgramSnapshot::new(b"state".to_vec())
            .encode()
            .expect("encode");
        let last = wire.len() - 1;
        wire[last] ^= 0xff;
        assert!(ProgramSnapshot::decode(&wire).is_err());
    }

    /// The plan's first exit test: a counter upgrades across a
    /// snapshot format change. The v1 program stored its counter as
    /// ASCII decimal in a v1 snapshot; the v2 program's `migrate`
    /// export converts it to an eight-byte little-endian counter.
    /// Oracle: after the upgrade the program is at version 2, its
    /// snapshot is current-format, and the state decodes to the same
    /// counter value in the new encoding.
    #[test]
    fn counter_upgrades_across_a_snapshot_format_change() {
        let mut legacy_wire = Vec::new();
        legacy_wire.extend_from_slice(&SNAPSHOT_MAGIC);
        legacy_wire.push(1);
        legacy_wire.extend_from_slice(b"41");
        let mut program = RunningProgram {
            version: 1,
            snapshot: legacy_wire,
        };

        let migrate = |state: &[u8], from: u32, to: u32| -> Result<Vec<u8>> {
            assert_eq!((from, to), (1, 2));
            let text = std::str::from_utf8(state).context("counter state is not text")?;
            let value: u64 = text.parse().context("counter state is not a number")?;
            Ok(value.to_le_bytes().to_vec())
        };
        let outcome = upgrade_program(&mut program, 2, Some(&migrate)).expect("upgrade");

        assert_eq!(outcome, UpgradeOutcome::Migrated);
        assert_eq!(program.version, 2);
        let decoded = ProgramSnapshot::decode(&program.snapshot).expect("decode upgraded");
        assert_eq!(decoded.format_version, SNAPSHOT_FORMAT_CURRENT);
        assert_eq!(
            u64::from_le_bytes(decoded.state.try_into().expect("8 bytes")),
            41
        );
    }

    /// The plan's second exit test: a deliberately failing upgrade
    /// leaves the old program running. Oracle: the program's version
    /// and snapshot bytes are bit-for-bit what they were before the
    /// attempt.
    #[test]
    fn failing_upgrade_leaves_the_old_program_running() {
        let mut program = RunningProgram {
            version: 1,
            snapshot: ProgramSnapshot::new(b"41".to_vec())
                .encode()
                .expect("encode"),
        };
        let before = program.clone();

        let migrate = |_: &[u8], _: u32, _: u32| -> Result<Vec<u8>> {
            bail!("guest migrate export refuses this upgrade")
        };
        let result = upgrade_program(&mut program, 2, Some(&migrate));

        assert!(result.is_err());
        assert_eq!(program, before);
    }

    /// Oracle: a program with no `migrate` export upgrades with its
    /// state carried byte-for-byte (the pre-existing behavior), while
    /// its version moves and its snapshot is refreshed to the
    /// current format.
    #[test]
    fn program_without_migrate_export_carries_its_state() {
        let mut legacy_wire = Vec::new();
        legacy_wire.extend_from_slice(&SNAPSHOT_MAGIC);
        legacy_wire.push(1);
        legacy_wire.extend_from_slice(b"opaque-state");
        let mut program = RunningProgram {
            version: 3,
            snapshot: legacy_wire,
        };

        let outcome = upgrade_program(&mut program, 4, None).expect("upgrade");

        assert_eq!(outcome, UpgradeOutcome::CarriedUnchanged);
        assert_eq!(program.version, 4);
        let decoded = ProgramSnapshot::decode(&program.snapshot).expect("decode");
        assert_eq!(decoded.format_version, SNAPSHOT_FORMAT_CURRENT);
        assert_eq!(decoded.state, b"opaque-state");
    }

    /// Oracle: a migration whose output exceeds the state bound
    /// rolls back instead of committing a truncated state.
    #[test]
    fn oversized_migrated_state_rolls_back() {
        let mut program = RunningProgram {
            version: 1,
            snapshot: ProgramSnapshot::new(b"small".to_vec())
                .encode()
                .expect("encode"),
        };
        let before = program.clone();

        let migrate = |_: &[u8], _: u32, _: u32| -> Result<Vec<u8>> {
            Ok(vec![0u8; MAX_PROGRAM_STATE_BYTES + 1])
        };
        let result = upgrade_program(&mut program, 2, Some(&migrate));

        assert!(result.is_err());
        assert_eq!(program, before);
    }
}
