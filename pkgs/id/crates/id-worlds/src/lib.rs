//! Worlds layer of the `id` crate.
//!
//! Step 5 of the pkgs/id crate split (#3447 umbrella, #4408): the
//! world family lives here, over `id-core`, `id-caps`, `id-store`,
//! `id-events`, `id-groups` and `id-sync`.
//!
//! Contents:
//!
//! - The runtime: [`world`] (core, actor, replica, replication,
//!   handle), [`world_session`], [`world_hub`], [`world_net`],
//!   [`world_store`], [`world_records`], [`world_compile`],
//!   [`world_native`], [`world_fixture`], [`world_cache`] and, under
//!   the `sandbox` feature, [`sandbox`] — the wasmtime guest runner
//!   whose fuel budgets are the split's determinism canary.
//! - [`program_bundle`]: the signed single-file program bundle (#8280,
//!   design D1-D3): export, verify and fork, fully offline.
//! - [`shell_contract`]: the F13.1 shell contract the native shell
//!   speaks.
//! - [`groups::worlds`]: seam 6 of the design — the groups→worlds
//!   bridge is the consumer side of the relationship, so it lives in
//!   the worlds crate, over the group model in `id-events`.
//! - [`blob_gate`]: the blob gate's folded-worlds adapter, which
//!   folds through the bridge above (step 4 parked it in the facade
//!   to move together with the bridge; it cannot live in `id-groups`,
//!   which is below this crate and must not name world types).
//! - The directory cluster — [`directory`], [`directory_auth`],
//!   [`directory_view`], [`directory_mail`] — the world runtime's own
//!   state: `WorldCore` holds the directory, the world actor executes
//!   directory actions against it, and `world_store` persists it
//!   beside the journal, so the cluster moves as the consumer side
//!   (the seam-6 pattern). Seam 10's remaining half is unchanged:
//!   step 6's `id-directory` extraction must still invert or re-home
//!   `WorldCore`'s hold on the directory, exactly as it would have
//!   had to from the facade. The facade re-exports these modules at
//!   their historical `id::` paths until then.
//!
//! The facade re-exports each module at its historical `id::` path,
//! so no call site outside the moved modules changed. The module
//! aliases below (`frames`, `world_caps`, …) exist so the moved
//! modules' `crate::` paths resolve exactly as they did in the
//! facade; they name the lower crates that own those modules.

/// The port contract (`world-store` port kind) this crate's adapters
/// advertise against, owned by `id-core` (slice 9110-02, #10160).
pub use id_core::port;

pub mod account_charge;
pub mod blob_gate;
pub mod chat_anchor_render;
pub mod groups;
pub mod guest_burn;
pub mod mcp_bridge;
pub mod program_bundle;
pub mod program_gallery;
pub mod program_gallery_admission;
pub mod program_gallery_operator;
pub mod program_registry;
pub mod program_registry_net;
#[cfg(feature = "sandbox")]
pub mod sandbox;
mod session_host;
pub mod shell_contract;
pub mod world;
pub mod world_cache;
pub mod world_compile;
#[cfg(feature = "sandbox")]
pub mod world_dev;
pub mod world_doctor;
pub mod world_fixture;
pub mod world_hub;
pub mod world_line;
pub mod world_native;
pub mod world_net;
pub mod world_owner_report;
pub mod world_records;
pub mod world_replication_net;
pub mod world_session;
#[cfg(feature = "ssh")]
pub mod world_ssh;
pub mod world_storage;
pub mod world_store;
pub mod world_terminal;

pub use id_caps::world_caps;
pub use id_caps::world_limits;
pub use id_core::abuse;
pub use id_core::connectivity;
pub use id_core::frames;
pub(crate) use id_core::is_format_char;
pub use id_core::petname;
pub(crate) use id_core::text;
pub(crate) use id_core::time;
pub use id_core::world_presentation;
pub use id_directory::directory;
pub use id_directory::directory_auth;
pub use id_directory::directory_mail;
pub use id_directory::directory_view;
pub use id_events::event_store;
pub use id_store::chain;
pub use id_store::store;
pub use id_store::tags;
pub use id_sync::artifact;
pub use id_sync::envelope_outbox;

#[cfg(test)]
pub(crate) use id_core::test_forgeries;

/// Venue capability probe for the QUIC (UDP) test suites (#12488).
///
/// The `world_net` / `world_records` / `world_replication_net` tests dial
/// iroh endpoints over loopback UDP. Some venues forbid UDP outright —
/// in the Muse sandbox `sendto` fails with EPERM even on loopback — so
/// there a handshake can never complete, no matter how generous the
/// deadline, and every dial test burns its full connect timeout before
/// failing. A test that needs the transport calls
/// [`test_transport::skip_if_no_udp_loopback`] first: where UDP loopback
/// works nothing changes; where it cannot, the test skips with a printed
/// line naming the test and the venue defect instead of a bare timeout.
#[cfg(test)]
pub(crate) mod test_transport {
    use std::net::UdpSocket;
    use std::sync::OnceLock;
    use std::time::Duration;

    /// `Some(reason)` when this venue cannot move a UDP datagram over
    /// loopback, with the OS error that proved it; `None` when it can.
    /// Probed once per test process.
    fn udp_loopback_unusable_reason() -> Option<&'static str> {
        static PROBE: OnceLock<Option<String>> = OnceLock::new();
        PROBE
            .get_or_init(|| {
                let probe = (|| {
                    let socket = UdpSocket::bind("127.0.0.1:0")?;
                    socket.set_read_timeout(Some(Duration::from_secs(2)))?;
                    let addr = socket.local_addr()?;
                    socket.send_to(b"udp-loopback-probe", addr)?;
                    let mut buf = [0_u8; 32];
                    let (n, _) = socket.recv_from(&mut buf)?;
                    if buf[..n] == *b"udp-loopback-probe" {
                        Ok(())
                    } else {
                        Err(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            "probe payload mismatch",
                        ))
                    }
                })();
                match probe {
                    Ok(()) => None,
                    Err(err) => Some(err.to_string()),
                }
            })
            .as_deref()
    }

    /// Guard for a test that dials or syncs over iroh/QUIC. Returns
    /// `true` — after printing the skip line naming `test` — when the
    /// venue has no UDP loopback and the test must skip; `false` when
    /// the test should run normally.
    pub(crate) fn skip_if_no_udp_loopback(test: &str) -> bool {
        match udp_loopback_unusable_reason() {
            Some(reason) => {
                eprintln!(
                    "skipping {test}: UDP loopback transport unavailable in this venue \
                     ({reason}); QUIC/iroh tests need UDP — see #12488"
                );
                true
            }
            None => false,
        }
    }
}
