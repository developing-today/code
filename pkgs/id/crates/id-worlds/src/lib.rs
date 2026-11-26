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
//!   [`world_functions`], [`world_native`], [`world_fixture`],
//!   [`world_cache`] and, under the `sandbox` feature, [`sandbox`] —
//!   the wasmtime guest runner whose fuel budgets are the split's
//!   determinism canary.
//! - [`host`]: host mode's supervisor state for #10693 (packet P1) — the
//!   lifecycle host file, transitions, rebuild, per-state census, and
//!   the replaying supervisor over one data root.
//! - [`host_drill`]: one-shot restore drills of a hosted world (#13744):
//!   export, restore into scratch, replay-equality check, and the
//!   per-attempt drill record.
//! - [`creator_announcements`]: scheduled creator announcements — the
//!   durable local schedule and its evaluator, which fires late after
//!   downtime and records the lateness (#13743).
//! - [`creator_index`]: the author-scoped local index over the author's
//!   own worlds and programs, rebuilt from journals and records, with
//!   build time and staleness (#13741).
//! - [`creator_org_views`]: read-only organization views (text and JSON)
//!   over the creator index, publish states and announcement schedule
//!   (#13745).
//! - [`program_bundle`]: the signed single-file program bundle (#8280,
//!   design D1-D3): export, verify and fork, fully offline.
//! - [`program_update`]: update discovery and the update envelope
//!   decision for installed program bundles (#8546, design v2 §§4–5).
//! - [`program_publish_state`]: draft / published journal facts per
//!   program version (#13742), keyed to the #6278 bundle record.
//! - [`safety_appeal`]: appeals against logged moderation actions,
//!   linked by action hash and decided through the review queue (#14467).
//! - [`safety_report`]: public-world safety reports — the signed
//!   report fact and its operator-scoped intake (#14464).
//! - [`safety_review_queue`]: the moderator review queue over report
//!   facts, with signed closures (#14465).
//! - [`safety_transparency`]: the read-only, replayable transparency
//!   log view over the moderation record (#14466).
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
//!   [`directory_view`], [`directory_mail`] — is owned by `id-directory`.
//!   This crate re-exports those modules for compatibility; `WorldCore`
//!   persists directory state as part of the world runtime.
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
pub mod attachment_dispatch;
pub mod blob_gate;
pub mod chat_anchor_render;
pub mod creator_announcements;
pub mod creator_index;
pub mod creator_org_views;
pub mod groups;
pub mod guest_burn;
pub mod host;
pub mod host_drill;
#[cfg(unix)]
pub mod host_control;
pub mod input_framing;
pub mod kitty_graphics;
pub mod kitty_still;
#[cfg(unix)]
pub mod node_control;
pub mod program_bundle;
pub mod program_gallery;
pub mod program_gallery_admission;
pub mod program_gallery_operator;
pub mod program_publish_state;
pub mod program_registry;
pub mod program_registry_net;
pub mod program_registry_pkarr;
pub mod program_update;
pub mod safety_appeal;
pub mod safety_report;
pub mod safety_review_queue;
pub mod safety_transparency;
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
pub mod world_export;
pub mod world_fixture;
pub mod world_functions;
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
pub mod world_template;
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

#[cfg(test)]
pub(crate) mod test_tracing {
    /// Two-step defence for the audit-capture tests (#13200).
    ///
    /// The whole audit stream is emitted from one callsite
    /// (`world::actor::emit_audit_record`). A callsite's interest is
    /// computed once, when the callsite first registers, from
    /// whichever dispatcher is current on the *registering thread*.
    /// Any test that drives an actor without a capture dispatcher can
    /// therefore register the audit callsite with `Interest::never`
    /// (its thread's default is empty), and from then on the `info!`
    /// macro skips the event before any dispatcher is consulted: a
    /// capture test running in parallel silently loses audit records
    /// — a few, or its whole capture. The flakes clustered at the
    /// start of a capture (seed 1) and vanished standalone.
    ///
    /// Step 1, `register_audit_callsite`: before installing its
    /// capture dispatcher, the test emits one warm-up record under a
    /// throwaway dispatcher on its own thread. That registers the
    /// audit callsite (registration happens exactly once per process)
    /// under a real subscriber, so no foreign thread's first emission
    /// can register it under an empty default later, mid-capture.
    ///
    /// Step 2, `rebuild_interest_cache`: if some earlier foreign
    /// emission already registered the callsite with `never`, the
    /// warm-up changes nothing — so immediately after `set_default`
    /// installs the capture dispatcher, the test rebuilds the
    /// interest cache on its own thread, recomputing the audit
    /// callsite's interest from a dispatcher that wants it. After
    /// that, the interest can only be recomputed by a full rebuild,
    /// and every full rebuild (triggered by any new `Dispatch`)
    /// includes this test's live dispatcher, so the capture stays
    /// live for the test's duration.
    pub(crate) fn register_audit_callsite() {
        let _guard = tracing::subscriber::set_default(tracing_subscriber::registry());
        crate::world::emit_audit_record(&crate::world::AuditRecord {
            schema_version: 2,
            timestamp_ms: 0,
            world_id: "capture-warmup".to_owned(),
            actor: "test".to_owned(),
            action: "capture_warmup".to_owned(),
            parameters: serde_json::json!({}),
            outcome: "warmup".to_owned(),
            target: "capture-warmup".to_owned(),
            correlation_id: "audit:0".to_owned(),
        });
    }

    /// Step 2 of the audit-capture defence: see the module note above.
    pub(crate) fn rebuild_interest_cache() {
        tracing::callsite::rebuild_interest_cache();
    }
}
