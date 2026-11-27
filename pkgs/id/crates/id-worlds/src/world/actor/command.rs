//! The world actor's command vocabulary: the commands a session or
//! the host can send the actor, and each command's stable name.
//! Extracted from `actor.rs` for the production-line guard (#14547):
//! moves only, no behaviour change.

use super::*;

use anyhow::Result;
use std::collections::BTreeMap;
use tokio::sync::{broadcast, oneshot};

use crate::world_caps::CapsReport;

impl WorldCommand {
    pub(crate) fn name(&self) -> &'static str {
        match self {
            Self::Issue { .. } => "Issue",
            Self::Attenuate { .. } => "Attenuate",
            Self::DelegateToFriend { .. } => "DelegateToFriend",
            Self::Directory { .. } => "Directory",
            Self::ApplyDirectory { .. } => "ApplyDirectory",
            Self::DirectoryAccount { .. } => "DirectoryAccount",
            Self::Revoke { .. } => "Revoke",
            Self::RevokeAuthorized { .. } => "RevokeAuthorized",
            Self::ParticipantId { .. } => "ParticipantId",
            Self::AuthorizeScope { .. } => "AuthorizeScope",
            Self::BindAuthorKey { .. } => "BindAuthorKey",
            Self::Chat { .. } => "Chat",
            Self::Input { .. } => "Input",
            Self::EntityInput { .. } => "EntityInput",
            Self::ExportEntity { .. } => "ExportEntity",
            Self::ImportEntity { .. } => "ImportEntity",
            Self::HandoffWriteLease { .. } => "HandoffWriteLease",
            Self::RecoverExpiredLease { .. } => "RecoverExpiredLease",
            Self::FailoverWriteLease { .. } => "FailoverWriteLease",
            Self::DispatchSpillCopy { .. } => "DispatchSpillCopy",
            Self::Snapshot { .. } => "Snapshot",
            Self::HostView { .. } => "HostView",
            Self::EventsAfter { .. } => "EventsAfter",
            Self::ProjectEvent { .. } => "ProjectEvent",
            Self::View { .. } => "View",
            Self::ViewAt { .. } => "ViewAt",
            Self::InstallProgram { .. } => "InstallProgram",
            Self::InstallProgramAuthorized { .. } => "InstallProgramAuthorized",
            Self::Presence { .. } => "Presence",
            Self::SubmitFormResult { .. } => "SubmitFormResult",
            Self::UpdateCaps { .. } => "UpdateCaps",
            Self::SetCapsCeiling { .. } => "SetCapsCeiling",
            Self::SetExecutionProfile { .. } => "SetExecutionProfile",
            Self::ExecutionProfileInfo { .. } => "ExecutionProfileInfo",
            Self::SetPresentationPolicy { .. } => "SetPresentationPolicy",
            Self::PresentationPolicyInfo { .. } => "PresentationPolicyInfo",
            Self::ManifestState { .. } => "ManifestState",
            Self::AdminScheduleAt { .. } => "AdminScheduleAt",
            Self::AdminCancelSchedule { .. } => "AdminCancelSchedule",
            Self::AdminSetScheduleGrant { .. } => "AdminSetScheduleGrant",
            Self::DeliverSchedule { .. } => "DeliverSchedule",
            Self::SetScheduleDelivery { .. } => "SetScheduleDelivery",
            Self::AdminSetPortalGrant { .. } => "AdminSetPortalGrant",
            Self::PortalGrantsInfo { .. } => "PortalGrantsInfo",
            Self::PortalSendPrecheck { .. } => "PortalSendPrecheck",
            Self::PortalSendAdmit { .. } => "PortalSendAdmit",
            Self::PortalSendRecord { .. } => "PortalSendRecord",
            Self::DeliverPortal { .. } => "DeliverPortal",
            Self::CapsInfo { .. } => "CapsInfo",
            Self::SetIsolation { .. } => "SetIsolation",
            Self::AuthorizeLifecycle { .. } => "AuthorizeLifecycle",
            Self::IsolationInfo { .. } => "IsolationInfo",
            Self::SetCrossWorldWrite { .. } => "SetCrossWorldWrite",
            Self::ForceCrossWorldWrite { .. } => "ForceCrossWorldWrite",
            Self::SetFileScope { .. } => "SetFileScope",
            Self::PolicyInfo { .. } => "PolicyInfo",
            Self::Metrics { .. } => "Metrics",
            Self::ProgramHealth { .. } => "ProgramHealth",
            Self::AdminEventHistory { .. } => "AdminEventHistory",
            Self::ReplicationFeed { .. } => "ReplicationFeed",
            Self::ReplicaSidecars { .. } => "ReplicaSidecars",
            Self::ReplicationCompact { .. } => "ReplicationCompact",
            Self::Shutdown { .. } => "Shutdown",
        }
    }
}

pub(crate) enum WorldCommand {
    Issue {
        name: String,
        scopes: WorldScopes,
        bounds: CapabilityBounds,
        reply: oneshot::Sender<Result<(Participant, JoinCapability)>>,
    },
    Attenuate {
        token: JoinCapability,
        name: String,
        scopes: WorldScopes,
        bounds: CapabilityBounds,
        reply: oneshot::Sender<Result<(Participant, JoinCapability)>>,
    },
    DelegateToFriend {
        token: JoinCapability,
        friend: String,
        name: String,
        scopes: WorldScopes,
        bounds: CapabilityBounds,
        reply: oneshot::Sender<Result<(Participant, JoinCapability)>>,
    },
    Directory {
        caller: crate::directory_auth::Caller,
        action: crate::directory_view::DirectoryAction,
        /// Whether this server has an outbox to queue what the action sends.
        outbox: bool,
        reply: oneshot::Sender<Result<crate::directory_view::DirectoryOutcome>>,
    },
    ApplyDirectory {
        entry: crate::directory::DirectoryEntry,
        reply: oneshot::Sender<Result<()>>,
    },
    /// Whether the world's directory holds `account`, and if so
    /// whether it is verified (#8538). Refused on a replica, like
    /// `ApplyDirectory`: the answer exists so a caller can apply an
    /// entry next, and a replica never applies one.
    DirectoryAccount {
        account: String,
        reply: oneshot::Sender<Result<Option<bool>>>,
    },
    Revoke {
        participant_id: u64,
        /// The moderator participant who ordered the revoke, for the audit
        /// journal. `None` means the admin revoked.
        by: Option<u64>,
        reply: oneshot::Sender<bool>,
    },
    /// Revoke ordered by a joined moderator (design #835 phase 2): the
    /// actor authorizes the live capability itself and derives the audit
    /// actor from it; no caller-supplied actor ID is trusted. The reply is
    /// a `Result` so a journal failure is surfaced instead of being
    /// reported as an audited success; the in-memory revocation stands
    /// either way (fail-closed).
    RevokeAuthorized {
        token: JoinCapability,
        participant_id: u64,
        reply: oneshot::Sender<Result<bool>>,
    },
    ParticipantId {
        token: JoinCapability,
        reply: oneshot::Sender<Result<u64>>,
    },
    /// Authorize a live capability for a scope set without performing
    /// the operation (design #835 phase 3): session paths check this
    /// before receiving upload bytes, while the operation commands
    /// (`InstallProgramAuthorized`, `RevokeAuthorized`) still re-check
    /// at the effect boundary. Replies with the derived participant ID.
    AuthorizeScope {
        token: JoinCapability,
        required: WorldScopes,
        reply: oneshot::Sender<Result<u64>>,
    },
    Chat {
        token: JoinCapability,
        text: String,
        reply: oneshot::Sender<Result<WorldEvent>>,
    },
    Input {
        token: JoinCapability,
        input: Vec<u8>,
        session_id: Option<SessionId>,
        caller_account: Option<String>,
        device_input: Option<serde_json::Value>,
        /// The player-signed move event bytes for this input (#2814), when
        /// the participant plays signed. `None` is the unsigned path,
        /// which refuses participants with a bound author key.
        signed_move: Option<Vec<u8>>,
        reply: oneshot::Sender<Result<WorldEvent>>,
    },
    /// Bind the calling participant's author key for signed play (#2814).
    BindAuthorKey {
        token: JoinCapability,
        author_key: [u8; 32],
        reply: oneshot::Sender<Result<u64>>,
    },
    EntityInput {
        token: JoinCapability,
        entity_ref: String,
        input: Vec<u8>,
        reply: oneshot::Sender<Result<WorldEvent>>,
    },
    ExportEntity {
        handoff_id: String,
        entity_ref: String,
        source_region: String,
        target_region: String,
        prior_owner_epoch: u64,
        reply: oneshot::Sender<Result<HandoffTransferState>>,
    },
    ImportEntity {
        handoff_id: String,
        entity_ref: String,
        source_region: String,
        target_region: String,
        prior_owner_epoch: u64,
        program_module_hash: String,
        payload_schema: String,
        payload: Vec<u8>,
        payload_digest: String,
        reply: oneshot::Sender<Result<HandoffTransferState>>,
    },
    /// DO-authority control request. The actor derives the source holder
    /// from its journaled state and serializes the durable boundary with all
    /// world commands.
    HandoffWriteLease {
        to: String,
        epoch: u64,
        expires_at_ms: Option<u64>,
        reply: oneshot::Sender<Result<u64>>,
    },
    /// DO-authority control request. Recover an expired native lease by
    /// journaling revoke then grant back to the DO (#13938 slice 1).
    /// The live lease tracks the journaled prefix after each entry, so
    /// a failed grant append between them leaves the world holder-less
    /// (writers refused `holder-lost`), never split-brained.
    RecoverExpiredLease {
        reply: oneshot::Sender<Result<u64>>,
    },
    /// DO-authority control request. The actor plans a spill v2
    /// failover (design #5787 §1) from its journaled lease and the
    /// supplied grantor-side pool, then journals the plan's
    /// revoke/grant entries in order.
    FailoverWriteLease {
        pool: spill_pool::SpillPool,
        journal_tip: u64,
        max_catch_up_lag: u64,
        term_ms: u64,
        reply: oneshot::Sender<Result<spill_pool::FailoverExecution>>,
    },
    /// Release copied journal bytes only after the current lease and its
    /// grant replay barrier pass on the serialized authority actor.
    DispatchSpillCopy {
        holder: String,
        epoch: u64,
        replayed_position: u64,
        journal_copy: Vec<u8>,
        reply: oneshot::Sender<Result<Vec<u8>>>,
    },
    Snapshot {
        token: JoinCapability,
        reply: oneshot::Sender<Result<WorldSnapshot>>,
    },
    /// Admin-authorized web rendering; the caller must validate the admin
    /// secret before requesting this view. The synthetic viewer is not a
    /// registered participant and never receives presence or event rights.
    HostView {
        reply: oneshot::Sender<Result<(WorldSnapshot, Option<String>)>>,
    },
    EventsAfter {
        token: JoinCapability,
        sequence: u64,
        reply: oneshot::Sender<Result<EventPage>>,
    },
    ProjectEvent {
        token: JoinCapability,
        event: WorldEvent,
        reply: oneshot::Sender<Result<Option<WorldEvent>>>,
    },
    View {
        token: JoinCapability,
        caller_account: Option<String>,
        reply: oneshot::Sender<Result<Option<String>>>,
    },
    ViewAt {
        token: JoinCapability,
        width: u16,
        height: u16,
        caller_account: Option<String>,
        reply: oneshot::Sender<Result<Option<String>>>,
    },
    InstallProgram {
        module_hash: String,
        seed: u64,
        provenance: Option<ModuleProvenance>,
        /// The builder participant who ordered the install, for the audit
        /// journal. `None` means the admin installed.
        installed_by: Option<u64>,
        /// The trust decision that admitted the module, for the audit
        /// journal (#1069). `None` records no decision.
        trust: Option<crate::world::TrustDecision>,
        program: Box<dyn WorldProgram>,
        reply: oneshot::Sender<Result<WorldEvent>>,
    },
    /// Install ordered by a joined builder (design #835 phase 2): the
    /// actor authorizes the live capability itself at the mutation
    /// boundary and derives `installed_by` from it.
    InstallProgramAuthorized {
        token: JoinCapability,
        module_hash: String,
        seed: u64,
        provenance: Option<ModuleProvenance>,
        trust: Option<crate::world::TrustDecision>,
        program: Box<dyn WorldProgram>,
        reply: oneshot::Sender<Result<WorldEvent>>,
    },
    /// A session opened or closed for this participant.
    Presence {
        participant: u64,
        joined: bool,
    },
    /// A trusted host submits a deferred answer from its local form presenter.
    SubmitFormResult {
        /// When present, the joined session that submitted the answer.
        token: Option<JoinCapability>,
        /// Connection context for attribution of resulting program requests.
        session_id: Option<SessionId>,
        result: id_core::world_presentation::FormResult,
        reply: oneshot::Sender<Result<FormSubmissionStatus>>,
    },
    /// Admin: change the granted capability set.
    UpdateCaps {
        grant: Vec<String>,
        revoke: Vec<String>,
        reply: oneshot::Sender<Result<CapsReport>>,
    },
    /// Admin: replace the world's capability ceiling. It applies immediately.
    SetCapsCeiling {
        ceiling: Vec<String>,
        reply: oneshot::Sender<Result<CapsReport>>,
    },
    /// Admin: durably replace this world's requested runtime profile.
    SetExecutionProfile {
        profile: crate::world_limits::WorldExecutionProfile,
        reply: oneshot::Sender<Result<ExecutionProfileUpdate>>,
    },
    /// Read this world's requested runtime profile.
    ExecutionProfileInfo {
        reply: oneshot::Sender<crate::world_limits::WorldExecutionProfile>,
    },
    /// Admin: durably replace this world's presentation policy (#14731
    /// slice 2b). It applies to the next presentation decision.
    SetPresentationPolicy {
        policy: id_core::presentation_policy::PresentationPolicy,
        reply: oneshot::Sender<Result<id_core::presentation_policy::PresentationPolicy>>,
    },
    /// Read this world's presentation policy. It is public: a world
    /// publishes it so a user can see it before joining (#5970 §5).
    PresentationPolicyInfo {
        reply: oneshot::Sender<id_core::presentation_policy::PresentationPolicy>,
    },
    /// Admin: read the manifest-reconciliation state — the active install
    /// seed and the pending schedules. The seed stays inside the actor and
    /// the admin layer; it is never part of a participant projection (#240).
    ManifestState {
        reply: oneshot::Sender<(u64, BTreeMap<String, u64>)>,
    },
    /// Admin: pend one schedule, replacing any pending schedule with the
    /// same id. Replies with the full pending map after the change.
    AdminScheduleAt {
        id: String,
        at_ms: u64,
        reply: oneshot::Sender<Result<BTreeMap<String, u64>>>,
    },
    /// Admin: cancel a pending schedule by id (a no-op when absent).
    /// Replies with the full pending map after the change.
    AdminCancelSchedule {
        id: String,
        reply: oneshot::Sender<Result<BTreeMap<String, u64>>>,
    },
    /// Admin: set this world's schedule-delivery consent for one peer
    /// (#1355 part 2). Replies with the full grant map after the change.
    AdminSetScheduleGrant {
        peer: String,
        send: bool,
        receive: bool,
        reply: oneshot::Sender<Result<BTreeMap<String, ScheduleGrant>>>,
    },
    /// A cross-world schedule fire arriving from `source` (#1355 part 2).
    /// Replies true only when this world durably accepted the fire.
    DeliverSchedule {
        source: String,
        id: String,
        at_ms: u64,
        late: bool,
        reply: oneshot::Sender<bool>,
    },
    /// Install the cross-world schedule delivery sink (composition layer).
    SetScheduleDelivery {
        delivery: std::sync::Arc<dyn ScheduleDelivery>,
        reply: oneshot::Sender<()>,
    },
    /// Admin: set this world's F12.14 portal grant for one peer
    /// (code#1068 wiring slice): `granted: true` grants the portal,
    /// `granted: false` revokes a live grant. Replies with the full
    /// grant book after the change.
    AdminSetPortalGrant {
        peer: String,
        granted: bool,
        reply: oneshot::Sender<Result<id_caps::portal::PortalGrants>>,
    },
    /// Read this world's current portal grant book (the transport
    /// reads the peer's book through this before a send).
    PortalGrantsInfo {
        reply: oneshot::Sender<id_caps::portal::PortalGrants>,
    },
    /// Phase 1 of a portal send (code#1068 wiring slice): the
    /// actor-local half of the pre-transport check — this world must
    /// be writable and must itself grant the target. Runs no
    /// cross-world await, so the command loop never blocks on a peer.
    PortalSendPrecheck {
        target: String,
        bytes: usize,
        reply: oneshot::Sender<Result<(), PortalSendError>>,
    },
    /// Phase 3 of a portal send: the full mutual check against the
    /// peer book the handle fetched through the transport (phase 2,
    /// outside the actor), the delivered-envelope bound, and the
    /// per-pair rate admission. Replies with this world's id (the
    /// source name the transport delivers under).
    PortalSendAdmit {
        target: String,
        peer_grants: id_caps::portal::PortalGrants,
        message: String,
        reply: oneshot::Sender<Result<String, PortalSendError>>,
    },
    /// Phase 5 of a portal send: record the send-side audit shape
    /// for the delivery outcome the handle observed (phase 4) — an
    /// accepted shape for `refusal: None`, the refusal shape (and a
    /// `Refused` reply) for `Some`.
    PortalSendRecord {
        target: String,
        bytes: usize,
        refusal: Option<id_caps::portal::PortalRefusal>,
        reply: oneshot::Sender<Result<(), PortalSendError>>,
    },
    /// A portal message arriving from `source` (code#1068 wiring
    /// slice). Replies `Ok` only when this world accepted it (its
    /// own grant re-checked, message delivered to the program); a
    /// policy refusal is recorded in this world's audit log.
    DeliverPortal {
        source: String,
        message: String,
        reply: oneshot::Sender<Result<(), PortalSendError>>,
    },
    /// Admin: read the capability report.
    CapsInfo {
        reply: oneshot::Sender<Result<CapsReport>>,
    },
    /// Owner or operator: may this authority change the world's
    /// lifecycle (wake, pause, archive; #10694)? Changes nothing.
    AuthorizeLifecycle {
        authority: Authority,
        act: &'static str,
        reply: oneshot::Sender<Result<()>>,
    },
    /// Admin or moderator: change the world's isolation.
    SetIsolation {
        authority: Authority,
        isolation: Isolation,
        reply: oneshot::Sender<Result<Isolation>>,
    },
    IsolationInfo {
        reply: oneshot::Sender<Isolation>,
    },
    /// Moderator or admin: let other unisolated worlds write here.
    SetCrossWorldWrite {
        authority: Authority,
        enabled: bool,
        reply: oneshot::Sender<Result<WorldPolicy>>,
    },
    /// Admin: force cross-world writes on.
    ForceCrossWorldWrite {
        authority: Authority,
        reply: oneshot::Sender<Result<WorldPolicy>>,
    },
    /// Moderator or admin: set the file scope; the admin alone sets the override.
    SetFileScope {
        authority: Authority,
        scope: FileScope,
        write_override: Option<bool>,
        reply: oneshot::Sender<Result<WorldPolicy>>,
    },
    PolicyInfo {
        reply: oneshot::Sender<WorldPolicy>,
    },
    /// Read this world's host metrics (F11.7): counters and histograms
    /// recorded by the actor, the journal path, and the session layer.
    Metrics {
        reply: oneshot::Sender<MetricsSnapshot>,
    },
    /// Whether the program is still healthy: false once the actor has
    /// latched a guest failure (an input, records, or wants call that
    /// trapped). Answered in command order, and the run loop awaits
    /// `after_change` after each command before receiving the next, so
    /// a health query sent after an input observes that input's
    /// post-input `after_change` outcome.
    ProgramHealth {
        reply: oneshot::Sender<bool>,
    },
    /// Admin-only consistent snapshot for replayable fixture export.
    AdminEventHistory {
        reply: oneshot::Sender<Result<AdminEventHistory>>,
    },
    /// Atomically capture the journal and subscribe to subsequent durable changes.
    /// When the captured journal is large enough to hold an over-bound entry,
    /// a second source is returned as a probe: the caller scans it OUTSIDE
    /// the actor (#8095), because awaiting the scan here would suspend all
    /// command processing for the duration of a full journal read.
    ReplicationFeed {
        reply: oneshot::Sender<
            Result<(
                String,
                JournalSnapshotSource,
                Option<JournalSnapshotSource>,
                broadcast::Receiver<ReplicationUpdate>,
            )>,
        >,
    },
    /// The authenticated sidecars for one replication snapshot
    /// (F12.13 phase 5): the directory facts for `subjects` (the
    /// accounts in the snapshot journal's capability chains) as
    /// JSONL entry bytes, plus the active module hash whose bytes
    /// the feed serves from the module store. The feed captures the
    /// sidecars from the actor immediately after the snapshot image
    /// in the same session, and the signed batch binds them to that
    /// snapshot's cursor; facts are derived inside the actor (the
    /// only place directory changes are applied), never read from a
    /// file the actor is concurrently appending to.
    ReplicaSidecars {
        subjects: std::collections::BTreeSet<String>,
        reply: oneshot::Sender<Result<(Vec<u8>, Option<String>)>>,
    },
    /// Serialized forced compaction after an out-of-actor probe found an
    /// oversized legacy entry (#8095). Compaction itself stays an actor
    /// operation; only the scan moved out.
    ReplicationCompact {
        ordinal: usize,
        encoded_size: usize,
        reply: oneshot::Sender<Result<()>>,
    },
    Shutdown {
        reply: oneshot::Sender<()>,
    },
}
