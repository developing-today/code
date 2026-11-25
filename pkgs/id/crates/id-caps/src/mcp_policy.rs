//! The host-side policy point for the agent-in-worlds spike (design
//! #6300 sections 6 and 7; spike step 4, #7916).
//!
//! The MCP bridge ([`crate::mcp_bridge`]) is a translator and holds no
//! authority: the world host is responsible for credentials, authority,
//! quota, and audit. This module is that host-side policy point. A
//! [`PolicySession`] wraps a [`BridgeSession`] and runs every agent
//! tool call through three steps, in order:
//!
//! 1. **Capability-ceiling check** (section 6.1). The ceiling is set at
//!    role grant and names the tools, the worlds, the object classes,
//!    and the program-invoke allowlist. Anything outside it is refused
//!    and audited, and never reaches the bridge.
//! 2. **Quota charge** (section 6.2). A local, per-session budget over
//!    frames, tool calls, and program invocations — the spike does not
//!    wait for F12.10 (design section 9.3); the audit event shape is
//!    the one F12.10 will consume. Every admitted action is charged
//!    *before* execution; a charge that would exceed the budget
//!    refuses the action and **parks** the agent: parking is returned
//!    to the driver as the first-class world event
//!    [`QUOTA_PARK_EVENT`], never a silent failure.
//! 3. **Audit event** (section 7). Exactly one event per agent action,
//!    including refusals and stale-input rejections. Events are
//!    hash-chained and signed with the host's journal key, so a hosted
//!    operator cannot quietly edit an agent's history; refusals are
//!    never sampled away. The agent is the audit subject, never its
//!    editor: the session exposes the log and the spend read-only.
//!
//! The stale-decision rule (section 4.3, section 6.3) is enforced
//! here, not just plumbed: a frame-carrying call whose
//! `screen_revision` token does not name the latest observed revision
//! is refused as stale input and audited.
//!
//! Per-tool rate limits (section 6.3) are enforced here too: a role
//! may cap admitted calls to a tool inside a sliding window —
//! `chat.post` is the spam-prone surface the design names — and a
//! call past the cap is refused and audited, charged nothing.
//!
//! A park is not a life sentence for the session (#14559): once
//! a replenishment tick has elapsed and the ledger could admit
//! an action's charge again, the park lifts and the action runs
//! through the normal order — the agent does not have to leave
//! and rejoin to spend a replenished budget. A role whose budget
//! never replenishes has no tick to wait for: its park holds
//! until rejoin, as before.
//!
//! This module is the one surviving policy point for the spike.
//! Spike step 4 landed twice (#7916); #13202 reconciled the two
//! implementations into this one, porting the standalone policy
//! point's per-tool rate limits, structured park event, and
//! refusal-code summary breakdown here before deleting it.
//!
//! `world.leave` is the exit, not an in-world action: it is never
//! refused by the ceiling or the quota (an agent must always be able
//! to leave, even parked or out of budget), it is charged nothing, and
//! it is still audited.
//!
//! `program.invoke` (spike step 5, section 4.6) runs through the
//! same order: the allowlist is its ceiling leg, the program axis is
//! charged before execution, and the bridge translates an admitted
//! invoke into the single `input` frame that carries it — one
//! invocation per call, no chaining.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use ed25519_dalek::{Signature, Signer as _, SigningKey, Verifier as _, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::agent_quota_ledger::{AgentQuotaLedger, LedgerCheckedOut, QuotaLedgerStore};
use crate::mcp_bridge::{BridgeError, BridgeSession, ToolAction, ToolDefinition};
use crate::role_config::QuotaClass;

/// The world event text emitted when quota exhaustion parks an agent
/// (design section 6.2, findings Q3b: visible, never silent).
pub const QUOTA_PARK_EVENT: &str = "agent parked, quota exhausted";

/// A per-tool rate limit (design section 6.3), part of the role.
///
/// At most `max_calls` admitted calls to `tool` within any
/// `window_secs` sliding window. Rate limits apply in addition to
/// quota; `chat.post` is the spam-prone surface the design names.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RateLimit {
    /// Tool the limit applies to, e.g. `chat.post`.
    pub tool: String,
    /// Admitted calls allowed inside the window.
    pub max_calls: u64,
    /// Window length, seconds.
    pub window_secs: u64,
}

/// The park world event (design section 6.2).
///
/// Emitted when quota exhaustion parks an agent. "Agent parked,
/// quota exhausted" is a first-class world event the owner sees,
/// never a silent failure; the shape is the section 9.3
/// local-budget event F12.10 will consume.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ParkEvent {
    /// Event kind: always `agent_parked`.
    pub kind: String,
    /// Machine-readable reason: always `quota_exhausted`.
    pub reason: String,
    /// Owner-facing message: [`QUOTA_PARK_EVENT`].
    pub message: String,
    /// The parked agent's public key.
    pub agent_key: [u8; 32],
    /// The world the session is connected to.
    pub world: String,
    /// Host time of the park, unix seconds.
    pub at: u64,
    /// Quota spent when the park happened.
    pub quota_spent: Quota,
    /// The session budget that was exhausted.
    pub quota_budget: Quota,
    /// Unix seconds of the next replenishment, when the role's budget
    /// replenishes; `None` when it never does.
    pub replenishes_at: Option<u64>,
}

/// The capability ceiling set at role grant (design section 6.1).
///
/// The ceiling names the tools, worlds, and object classes the
/// agent may touch, and the programs it may invoke. Anything not
/// named is outside the ceiling. The ceiling can only be changed
/// owner-side, out-of-band — a session holds the grant it was
/// constructed with.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct CapabilityCeiling {
    /// MCP tool names the agent may call.
    pub tools: BTreeSet<String>,
    /// World ids the agent may join.
    pub worlds: BTreeSet<String>,
    /// Object classes the agent may inspect. An inspect whose target
    /// class is not established, or not named here, is refused.
    pub object_classes: BTreeSet<String>,
    /// Program names `program.invoke` may name.
    pub program_allowlist: BTreeSet<String>,
}

/// A quota vector over the expensive axes (design section 6.2). The
/// same shape serves as the per-session budget, the running spend,
/// and the per-action charge recorded in each audit event.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct Quota {
    /// Frames sent to the world host.
    pub frames: u64,
    /// Tool calls admitted by the policy point.
    pub tool_calls: u64,
    /// Program invocations admitted by the policy point.
    pub program_invocations: u64,
}

impl Quota {
    /// The zero vector: no budget, no spend, no charge.
    #[must_use]
    pub const fn zero() -> Self {
        Self {
            frames: 0,
            tool_calls: 0,
            program_invocations: 0,
        }
    }

    pub(crate) const fn add(&mut self, charge: Self) {
        self.frames += charge.frames;
        self.tool_calls += charge.tool_calls;
        self.program_invocations += charge.program_invocations;
    }

    /// The first axis on which `spend + charge` would exceed
    /// `budget`, if any.
    pub(crate) const fn exceeded_axis(
        budget: Self,
        spend: Self,
        charge: Self,
    ) -> Option<&'static str> {
        if spend.frames + charge.frames > budget.frames {
            Some("frames")
        } else if spend.tool_calls + charge.tool_calls > budget.tool_calls {
            Some("tool_calls")
        } else if spend.program_invocations + charge.program_invocations
            > budget.program_invocations
        {
            Some("program_invocations")
        } else {
            None
        }
    }
}

/// The charge one admitted action costs, by tool: every action costs
/// one tool call; the frame-shaped tools (join, subscribe, chat, and
/// the program invocation's `input` frame) cost one frame;
/// `program.invoke` additionally costs one program invocation.
fn charge_for(tool: &str) -> Quota {
    Quota {
        frames: u64::from(matches!(
            tool,
            "world.join" | "screen.subscribe" | "chat.post" | "program.invoke"
        )),
        tool_calls: 1,
        program_invocations: u64::from(tool == "program.invoke"),
    }
}

/// A role grant: the authority a session is constructed with. Set
/// owner-side at grant time; the session never widens it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RoleGrant {
    /// The agent's public key.
    pub agent_key: [u8; 32],
    /// Short human label for the agent ("archivist"), used in the
    /// owner summary and in-world presence.
    pub agent_label: String,
    /// The owner's name; the agent's presence is labelled with it
    /// (design section 6.3).
    pub owner: String,
    /// The world this session connects to.
    pub world: String,
    /// Reference to the capability proof behind this grant (the
    /// delegation / grant record id), stamped on every audit event.
    pub capability_ref: String,
    /// The capability ceiling for this session.
    pub ceiling: CapabilityCeiling,
    /// The role the grant was made under; the quota ledger is keyed
    /// by it for the owner's per-role view.
    pub role: String,
    /// The role's quota class: the budget granted with the role and
    /// its replenishment cadence (#8301).
    pub quota_class: QuotaClass,
    /// Per-tool rate limits for this session (design section 6.3),
    /// part of the role: empty when the role caps no tool.
    pub rate_limits: Vec<RateLimit>,
}

/// The owner authority one action ran under (design #6301 section
/// 2.5, #8302): the review request the owner approved, and who
/// approved it. Stamped on exactly the one audit event that request
/// executed; it is never a delegation and never edits the role.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct OwnerApproval {
    /// The review request this approval answered.
    pub request_id: u64,
    /// The owner who approved it.
    pub approver: String,
}

/// Which leg of the capability ceiling refused an action. Its
/// [`std::fmt::Display`] is the `detail` the audit log records for a
/// [`AuditOutcome::RefusedCeiling`], and [`CeilingLeg::from_detail`]
/// reads it back, so the owner review tooling can name the role edit
/// that would fix a refusal from the log alone (#8302).
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CeilingLeg {
    /// The tool is not in the ceiling's tool set.
    Tool(String),
    /// The world is not in the ceiling's world set.
    World(String),
    /// The inspect target's object class is not in the ceiling.
    ObjectClass(String),
    /// The inspect target's object class is not established.
    ClassNotEstablished,
    /// The program is not on the program-invoke allowlist.
    Program(String),
    /// `program.invoke` named no program.
    NoProgram,
}

const OUTSIDE_CEILING: &str = " is outside the capability ceiling";
const OUTSIDE_ALLOWLIST: &str = " is outside the program-invoke allowlist";
const CLASS_NOT_ESTABLISHED: &str = "the object class of the inspect target is not established";
const NO_PROGRAM: &str = "program.invoke names no program";

impl std::fmt::Display for CeilingLeg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Tool(tool) => write!(f, "tool {tool}{OUTSIDE_CEILING}"),
            Self::World(world) => write!(f, "world {world}{OUTSIDE_CEILING}"),
            Self::ObjectClass(class) => write!(f, "object class {class}{OUTSIDE_CEILING}"),
            Self::ClassNotEstablished => f.write_str(CLASS_NOT_ESTABLISHED),
            Self::Program(program) => write!(f, "program {program}{OUTSIDE_ALLOWLIST}"),
            Self::NoProgram => f.write_str(NO_PROGRAM),
        }
    }
}

impl CeilingLeg {
    /// Read a ceiling refusal's audit `detail` back into its leg;
    /// `None` when the detail is not one this policy point writes.
    #[must_use]
    pub fn from_detail(detail: &str) -> Option<Self> {
        if detail == CLASS_NOT_ESTABLISHED {
            return Some(Self::ClassNotEstablished);
        }
        if detail == NO_PROGRAM {
            return Some(Self::NoProgram);
        }
        if let Some(program) = detail
            .strip_suffix(OUTSIDE_ALLOWLIST)
            .and_then(|rest| rest.strip_prefix("program "))
        {
            return Some(Self::Program(program.to_owned()));
        }
        let subject = detail.strip_suffix(OUTSIDE_CEILING)?;
        if let Some(class) = subject.strip_prefix("object class ") {
            Some(Self::ObjectClass(class.to_owned()))
        } else if let Some(tool) = subject.strip_prefix("tool ") {
            Some(Self::Tool(tool.to_owned()))
        } else {
            subject
                .strip_prefix("world ")
                .map(|world| Self::World(world.to_owned()))
        }
    }
}

/// The outcome of one agent action, as recorded in the audit log
/// (design section 7: one event per action, refusals included).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum AuditOutcome {
    /// The action was admitted and executed (or answered locally).
    Executed,
    /// The action was outside the capability ceiling; never executed.
    RefusedCeiling {
        /// What was outside the ceiling.
        detail: String,
    },
    /// The action's charge would have exceeded the session budget;
    /// the agent parked. Never executed.
    RefusedQuota {
        /// The budget axis the charge would have exceeded.
        axis: String,
    },
    /// The agent was already parked; the action was refused without
    /// a further charge. Never executed.
    RefusedParked,
    /// The action carried a stale-decision token naming a revision
    /// other than the latest observed one. Never executed.
    RefusedStaleInput {
        /// The revision the agent acted on.
        token: u64,
        /// The latest revision the host has observed.
        current: u64,
    },
    /// The action exceeded the role's per-tool rate limit (design
    /// section 6.3). Never executed, charged nothing.
    RefusedRateLimit {
        /// The tool whose limit was hit.
        tool: String,
        /// Admitted calls allowed inside the window.
        limit: u64,
        /// Window length, seconds.
        window_secs: u64,
    },
    /// The policy point admitted the action and the bridge refused
    /// it (unknown tool, invalid arguments, no frame mapping, ...).
    /// The charge stands: the attempt consumed the host's work.
    RefusedBridge {
        /// The bridge refusal, debug-formatted.
        detail: String,
    },
}

impl AuditOutcome {
    /// The stable machine-readable refusal code for this outcome,
    /// or `None` for an executed action. The owner summary's
    /// refusal breakdown is keyed by these codes.
    #[must_use]
    pub const fn refusal_code(&self) -> Option<&'static str> {
        match self {
            Self::Executed => None,
            Self::RefusedCeiling { .. } => Some("ceiling"),
            Self::RefusedQuota { .. } => Some("quota"),
            Self::RefusedParked => Some("parked"),
            Self::RefusedStaleInput { .. } => Some("stale_input"),
            Self::RefusedRateLimit { .. } => Some("rate_limited"),
            Self::RefusedBridge { .. } => Some("bridge"),
        }
    }
}

/// One signed, hash-chained audit event (design section 7).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AuditEvent {
    /// Position in the chain, from 0.
    pub seq: u64,
    /// The agent the event is about.
    pub agent_key: [u8; 32],
    /// The agent's owner.
    pub owner: String,
    /// The world the session is connected to.
    pub world: String,
    /// The MCP tool the agent called.
    pub tool: String,
    /// The object the action named, where it named one (the inspect
    /// target, the invoked program).
    pub object: Option<String>,
    /// The capability proof reference from the role grant.
    pub capability_ref: String,
    /// The role the grant was made under (#8302): the owner review
    /// view and refusal digests read the role from the log, which
    /// stays the record of truth. Empty only on events written
    /// before the field existed; an empty role is left out of the
    /// canonical bytes, so those events still verify.
    #[serde(default)]
    pub role: String,
    /// The owner approval this action ran under, when it is the one
    /// action of a one-shot review approval (#8302); `None` for
    /// every action the agent's own authority covered. Left out of
    /// the canonical bytes when `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approved_by: Option<OwnerApproval>,
    /// The quota this action was charged (zero for refusals that
    /// were never admitted).
    pub quota_charged: Quota,
    /// What the policy point did with the action.
    pub outcome: AuditOutcome,
    /// Hash of the previous event; all-zero for the first.
    pub prev_hash: [u8; 32],
    /// SHA-256 of this event's canonical bytes.
    pub hash: [u8; 32],
    /// Host signature over the canonical bytes.
    #[serde(with = "signature_serde")]
    pub signature: [u8; 64],
}

/// Serde for a 64-byte signature: serde's built-in array impls stop
/// at 32 elements, so the signature round-trips as a byte sequence.
mod signature_serde {
    use serde::{
        Deserializer, Serializer,
        de::{Error as _, SeqAccess, Visitor},
    };

    pub fn serialize<S>(signature: &[u8; 64], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_seq(signature.iter())
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<[u8; 64], D::Error>
    where
        D: Deserializer<'de>,
    {
        struct SignatureVisitor;

        impl<'de> Visitor<'de> for SignatureVisitor {
            type Value = [u8; 64];

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a sequence of 64 bytes")
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let mut bytes = [0u8; 64];
                for (index, slot) in bytes.iter_mut().enumerate() {
                    *slot = seq
                        .next_element()?
                        .ok_or_else(|| A::Error::invalid_length(index, &self))?;
                }
                Ok(bytes)
            }
        }

        deserializer.deserialize_seq(SignatureVisitor)
    }
}

impl AuditEvent {
    /// The canonical bytes the hash and the signature cover: every
    /// field except `hash` and `signature`, in declaration order.
    /// `role` and `approved_by` are left out when empty, so events
    /// written before #8302 keep their bytes and still verify.
    fn canonical_bytes(&self) -> Vec<u8> {
        #[derive(Serialize)]
        struct Canonical<'a> {
            seq: u64,
            agent_key: &'a [u8; 32],
            owner: &'a str,
            world: &'a str,
            tool: &'a str,
            object: Option<&'a str>,
            capability_ref: &'a str,
            #[serde(skip_serializing_if = "str::is_empty")]
            role: &'a str,
            #[serde(skip_serializing_if = "Option::is_none")]
            approved_by: Option<&'a OwnerApproval>,
            quota_charged: &'a Quota,
            outcome: &'a AuditOutcome,
            prev_hash: &'a [u8; 32],
        }
        serde_json::to_vec(&Canonical {
            seq: self.seq,
            agent_key: &self.agent_key,
            owner: &self.owner,
            world: &self.world,
            tool: &self.tool,
            object: self.object.as_deref(),
            capability_ref: &self.capability_ref,
            role: &self.role,
            approved_by: self.approved_by.as_ref(),
            quota_charged: &self.quota_charged,
            outcome: &self.outcome,
            prev_hash: &self.prev_hash,
        })
        .unwrap_or_default()
    }
}

/// Why an audit chain failed verification.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AuditError {
    /// An event's `prev_hash` does not name its predecessor's hash.
    BrokenLink {
        /// The event whose link is broken.
        seq: u64,
    },
    /// An event's stored hash does not match its contents.
    HashMismatch {
        /// The event whose hash does not match.
        seq: u64,
    },
    /// An event's signature does not verify under the host key.
    BadSignature {
        /// The event whose signature does not verify.
        seq: u64,
    },
}

/// Verify a whole audit chain end-to-end: sequence, links, hashes,
/// and host signatures. This is the check behind acceptance
/// criterion "the audit chain verifies end-to-end for a spike
/// session".
///
/// # Errors
/// Returns the first [`AuditError`] found, naming the event at
/// fault.
pub fn verify_audit_chain(
    events: &[AuditEvent],
    host_key: &VerifyingKey,
) -> Result<(), AuditError> {
    let mut prev_hash = [0u8; 32];
    for (index, event) in events.iter().enumerate() {
        let seq = index as u64;
        if event.seq != seq || event.prev_hash != prev_hash {
            return Err(AuditError::BrokenLink { seq: event.seq });
        }
        let digest: [u8; 32] = Sha256::digest(event.canonical_bytes()).into();
        if digest != event.hash {
            return Err(AuditError::HashMismatch { seq: event.seq });
        }
        host_key
            .verify(
                &event.canonical_bytes(),
                &Signature::from_bytes(&event.signature),
            )
            .map_err(|_error| AuditError::BadSignature { seq: event.seq })?;
        prev_hash = event.hash;
    }
    Ok(())
}

/// The append-only audit log. Only the policy point appends; the
/// agent's side of the API is the read-only [`AuditLog::events`]
/// slice and [`AuditLog::verify`].
#[derive(Debug)]
pub struct AuditLog {
    host_key: SigningKey,
    events: Vec<AuditEvent>,
}

impl AuditLog {
    /// Start an empty log signed with the host's journal key.
    #[must_use]
    pub const fn new(host_key: SigningKey) -> Self {
        Self {
            host_key,
            events: Vec::new(),
        }
    }

    /// The host's verifying key, for chain verification by owners
    /// and auditors.
    #[must_use]
    pub fn host_verifying_key(&self) -> VerifyingKey {
        self.host_key.verifying_key()
    }

    /// The events so far, in chain order. Read-only: there is no
    /// mutable accessor — the agent cannot edit its own history.
    #[must_use]
    pub fn events(&self) -> &[AuditEvent] {
        &self.events
    }

    /// Verify the whole chain under the host key.
    ///
    /// # Errors
    /// Returns the first [`AuditError`] found.
    pub fn verify(&self) -> Result<(), AuditError> {
        verify_audit_chain(&self.events, &self.host_key.verifying_key())
    }

    fn append(
        &mut self,
        approved_by: Option<OwnerApproval>,
        grant: &RoleGrant,
        tool: &str,
        object: Option<String>,
        quota_charged: Quota,
        outcome: AuditOutcome,
    ) -> &AuditEvent {
        let prev_hash = self.events.last().map_or([0u8; 32], |event| event.hash);
        let mut event = AuditEvent {
            seq: self.events.len() as u64,
            agent_key: grant.agent_key,
            owner: grant.owner.clone(),
            world: grant.world.clone(),
            tool: tool.to_owned(),
            object,
            capability_ref: grant.capability_ref.clone(),
            role: grant.role.clone(),
            approved_by,
            quota_charged,
            outcome,
            prev_hash,
            hash: [0u8; 32],
            signature: [0u8; 64],
        };
        let canonical = event.canonical_bytes();
        event.hash = Sha256::digest(&canonical).into();
        event.signature = self.host_key.sign(&canonical).to_bytes();
        self.events.push(event);
        let index = self.events.len() - 1;
        &self.events[index]
    }
}

/// The owner-readable periodic summary (design section 7): the raw
/// log is for machines; this is the review surface an owner reads.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OwnerSummary {
    /// The agent's human label from the grant.
    pub agent_label: String,
    /// The owner the summary is rendered for.
    pub owner: String,
    /// The world the session is connected to.
    pub world: String,
    /// Total actions (executed plus refused).
    pub actions: u64,
    /// Actions admitted and executed.
    pub executed: u64,
    /// Actions refused, for any reason. Refusals are the
    /// highest-signal events and are never sampled away.
    pub refusals: u64,
    /// Refusals broken down by stable refusal code
    /// ([`AuditOutcome::refusal_code`]).
    pub refusals_by_code: BTreeMap<String, u64>,
    /// Total quota spent across the session.
    pub spend: Quota,
    /// Whether the agent is currently parked.
    pub parked: bool,
}

impl OwnerSummary {
    /// Render the one-paragraph summary in the design's shape:
    /// actions, spend, refusals.
    #[must_use]
    pub fn render(&self) -> String {
        let parked = if self.parked {
            " It is parked, quota exhausted."
        } else {
            ""
        };
        format!(
            "your {} agent performed {} actions in world {} ({} executed), spent {} frames / {} tool calls / {} program invocations, and was refused {} times.{}",
            self.agent_label,
            self.actions,
            self.world,
            self.executed,
            self.spend.frames,
            self.spend.tool_calls,
            self.spend.program_invocations,
            self.refusals,
            parked,
        )
    }
}

/// Why the policy point refused an action.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RefusalKind {
    /// Outside the capability ceiling.
    Ceiling,
    /// The charge would have exceeded the budget; the agent parked.
    Quota,
    /// The agent was already parked.
    Parked,
    /// The stale-decision token did not name the current revision.
    StaleInput,
    /// The action exceeded the role's per-tool rate limit.
    RateLimited,
    /// Admitted by the policy point, refused by the bridge.
    Bridge,
}

/// A policy-point refusal: the action was not executed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PolicyRefusal {
    /// Which check refused the action.
    pub kind: RefusalKind,
    /// Human-readable detail for the caller and the audit record.
    pub detail: String,
    /// The world event the driver must surface, when this refusal
    /// parked the agent; `None` otherwise.
    pub park_event: Option<ParkEvent>,
}

/// What the bridge driver must do with one policy-checked tool call.
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, Debug, PartialEq)]
pub enum PolicyOutcome {
    /// The action was admitted and executed; carry out the bridge
    /// action.
    Executed(ToolAction),
    /// The action was refused and not executed.
    Refused(PolicyRefusal),
}

/// A bridge session behind the policy point: one role grant, one
/// bridge session, one audit log, one quota budget.
#[derive(Debug)]
pub struct PolicySession {
    grant: RoleGrant,
    bridge: BridgeSession,
    audit: AuditLog,
    ledger: AgentQuotaLedger,
    parked: bool,
    /// When the current park's budget next replenishes, as named
    /// by the exhaustion that parked the session (#14559); `None`
    /// when the session is not parked, or when the budget never
    /// replenishes and the park holds until rejoin.
    parked_replenishes_at: Option<u64>,
    object_classes: BTreeMap<String, String>,
    admitted_at: BTreeMap<String, VecDeque<u64>>,
    /// The owner approval in force for the one action
    /// [`Self::act_owner_approved`] is running; `None` otherwise.
    approval: Option<OwnerApproval>,
}

impl PolicySession {
    /// Open a policy session for one role grant. The bridge session
    /// is started (and its definitions pinned) by the driver, as in
    /// [`BridgeSession::start`]; `host_key` signs the audit chain.
    /// The session's quota is the grant's per-(agent, world) ledger
    /// (#8301), opened at `now`; a rejoin reopens the persisted ledger
    /// instead.
    #[must_use]
    pub fn new(grant: RoleGrant, bridge: BridgeSession, host_key: SigningKey, now: u64) -> Self {
        let ledger = AgentQuotaLedger::open(
            grant.agent_key,
            grant.world.clone(),
            grant.role.clone(),
            grant.quota_class,
            now,
        );
        Self {
            grant,
            bridge,
            audit: AuditLog::new(host_key),
            ledger,
            parked: false,
            parked_replenishes_at: None,
            object_classes: BTreeMap::new(),
            admitted_at: BTreeMap::new(),
            approval: None,
        }
    }

    /// Open a policy session for one role grant, drawing its quota
    /// ledger from the host's [`QuotaLedgerStore`]. The first open
    /// for an agent grants the role's budget; a rejoin reopens the
    /// persisted ledger with its spend intact, so rejoining never
    /// resets a budget (#8301).
    ///
    /// # Errors
    ///
    /// Returns [`LedgerCheckedOut`] when the agent's ledger for
    /// the grant's world is already checked out by a live session
    /// (#14526): one ledger backs at most one live session, so a
    /// duplicate open is refused instead of doubling the budget.
    pub fn open_stored(
        grant: RoleGrant,
        bridge: BridgeSession,
        host_key: SigningKey,
        store: &mut QuotaLedgerStore,
        now: u64,
    ) -> Result<Self, LedgerCheckedOut> {
        let ledger = store.open(
            grant.agent_key,
            grant.world.clone(),
            grant.role.clone(),
            grant.quota_class,
            now,
        )?;
        Ok(Self {
            grant,
            bridge,
            audit: AuditLog::new(host_key),
            ledger,
            parked: false,
            parked_replenishes_at: None,
            object_classes: BTreeMap::new(),
            admitted_at: BTreeMap::new(),
            approval: None,
        })
    }

    /// End the session, handing its quota ledger back to the host's
    /// [`QuotaLedgerStore`]. The driver calls this on leave (or on
    /// host shutdown) so the next join reopens the same spend.
    pub fn close(self, store: &mut QuotaLedgerStore) {
        store.close(self.ledger);
    }

    /// The role grant this session was opened with.
    #[must_use]
    pub const fn grant(&self) -> &RoleGrant {
        &self.grant
    }

    /// Owner-side ceiling refresh (v2 agents-as-citizens step 1,
    /// #8299 §2.1): replace the ceiling in force for this live
    /// session. A lowered ceiling takes effect at the next
    /// [`Self::act`] frame check — not the next join — so a
    /// citizen's rights track the current role configuration; the
    /// role edit itself is audited by the role registry that issued
    /// the change. Nothing here widens: it is the same owner-side
    /// act as an out-of-band ceiling edit in v1, now reachable
    /// through world configuration.
    ///
    /// # Errors
    ///
    /// Returns a [`RefusalKind::Ceiling`] refusal if the new ceiling
    /// does not cover the session's world: a session never runs
    /// under a ceiling that does not name its world.
    //
    // PolicyRefusal carries the full park event for the driver; a
    // ceiling refresh runs at owner-operation cadence, so the large
    // Err payload never sits in a hot path.
    #[allow(clippy::result_large_err)]
    pub fn refresh_ceiling(&mut self, ceiling: CapabilityCeiling) -> Result<(), PolicyRefusal> {
        if !ceiling.worlds.contains(&self.grant.world) {
            return Err(PolicyRefusal {
                kind: RefusalKind::Ceiling,
                detail: format!(
                    "world {} is outside the refreshed capability ceiling",
                    self.grant.world
                ),
                park_event: None,
            });
        }
        self.grant.ceiling = ceiling;
        Ok(())
    }

    /// The wrapped bridge session, read-only.
    #[must_use]
    pub const fn bridge(&self) -> &BridgeSession {
        &self.bridge
    }

    /// Whether quota exhaustion has parked this agent.
    #[must_use]
    pub const fn is_parked(&self) -> bool {
        self.parked
    }

    /// The quota spent so far as of `now`, after replenishment. A
    /// copy: the agent cannot reach the session's own ledger through it.
    #[must_use]
    pub fn spend(&self, now: u64) -> Quota {
        self.ledger.spend_at(now)
    }

    /// The audit events so far, read-only.
    #[must_use]
    pub fn audit_events(&self) -> &[AuditEvent] {
        self.audit.events()
    }

    /// The host's verifying key for the audit chain.
    #[must_use]
    pub fn host_verifying_key(&self) -> VerifyingKey {
        self.audit.host_verifying_key()
    }

    /// Verify the audit chain end-to-end under the host key.
    ///
    /// # Errors
    /// Returns the first [`AuditError`] found.
    pub fn verify_audit(&self) -> Result<(), AuditError> {
        self.audit.verify()
    }

    /// The owner summary over the whole session so far, spend read
    /// as of `now`.
    #[must_use]
    pub fn owner_summary(&self, now: u64) -> OwnerSummary {
        let mut executed = 0u64;
        let mut refusals = 0u64;
        let mut refusals_by_code: BTreeMap<String, u64> = BTreeMap::new();
        for event in self.audit.events() {
            match event.outcome {
                AuditOutcome::Executed => executed += 1,
                _ => refusals += 1,
            }
            if let Some(code) = event.outcome.refusal_code() {
                *refusals_by_code.entry(code.to_owned()).or_default() += 1;
            }
        }
        OwnerSummary {
            agent_label: self.grant.agent_label.clone(),
            owner: self.grant.owner.clone(),
            world: self.grant.world.clone(),
            actions: self.audit.events().len() as u64,
            executed,
            refusals,
            refusals_by_code,
            spend: self.spend(now),
            parked: self.parked,
        }
    }

    /// Check the bridge's pinned tool definitions (passthrough to
    /// [`BridgeSession::verify_definitions`]). A mismatch terminates
    /// the bridge session; later actions are refused by the bridge
    /// and audited as such.
    ///
    /// # Errors
    /// Returns the bridge's [`BridgeError`] unchanged.
    pub fn verify_definitions(
        &mut self,
        definitions: &[ToolDefinition],
    ) -> Result<(), BridgeError> {
        self.bridge.verify_definitions(definitions)
    }

    /// Observe one server frame: the bridge updates its screen
    /// state, and the policy point notes the object classes the
    /// frame establishes (the ceiling check for `object.inspect`
    /// consults them).
    pub fn observe_frame(&mut self, frame: &serde_json::Value) {
        if frame.get("type").and_then(serde_json::Value::as_str) == Some("semantic_diff")
            && let Some(changes) = frame.get("changes").and_then(serde_json::Value::as_array)
        {
            for change in changes {
                if change.get("kind").and_then(serde_json::Value::as_str) == Some("object_changed")
                    && let (Some(entity_ref), Some(class)) = (
                        change.get("entity_ref"),
                        change
                            .get("object_class")
                            .and_then(serde_json::Value::as_str),
                    )
                {
                    self.object_classes
                        .insert(entity_key(entity_ref), class.to_owned());
                }
            }
        }
        self.bridge.observe_frame(frame);
    }

    /// Run one agent tool call through the policy point, in the
    /// design's order: ceiling, stale-decision, rate limit, quota,
    /// execution — with exactly one audit event whatever the
    /// verdict. `now` is the host time, unix seconds; it drives the
    /// rate-limit windows and stamps the park event.
    pub fn act(&mut self, name: &str, arguments: &serde_json::Value, now: u64) -> PolicyOutcome {
        let object = audit_object(name, arguments);
        // The exit is never refused: not by the ceiling, not by the
        // quota, not by parking. It is charged nothing and audited.
        if name == "world.leave" {
            return self.execute(name, arguments, object, Quota::zero());
        }
        if self.parked {
            // Auto-lift (#14559): the park recorded when the ledger
            // next replenishes. Once that tick has elapsed and the
            // ledger could admit this action's charge, the park
            // lifts and the action continues through the normal
            // ceiling/rate/quota order below — no leave and rejoin
            // needed. A park whose budget never replenishes has no
            // tick to wait for and holds until rejoin.
            let lift = match self.parked_replenishes_at {
                Some(at) => now >= at && self.ledger.can_admit(now, charge_for(name)),
                None => false,
            };
            if !lift {
                self.audit.append(
                    self.approval.clone(),
                    &self.grant,
                    name,
                    object,
                    Quota::zero(),
                    AuditOutcome::RefusedParked,
                );
                return PolicyOutcome::Refused(PolicyRefusal {
                    kind: RefusalKind::Parked,
                    detail: QUOTA_PARK_EVENT.to_owned(),
                    park_event: None,
                });
            }
            self.parked = false;
            self.parked_replenishes_at = None;
        }
        if self.approval.is_none()
            && let Some(leg) = self.ceiling_refusal(name, arguments)
        {
            let detail = leg.to_string();
            self.audit.append(
                self.approval.clone(),
                &self.grant,
                name,
                object,
                Quota::zero(),
                AuditOutcome::RefusedCeiling {
                    detail: detail.clone(),
                },
            );
            return PolicyOutcome::Refused(PolicyRefusal {
                kind: RefusalKind::Ceiling,
                detail,
                park_event: None,
            });
        }
        if let Some(token) = arguments
            .get("screen_revision")
            .and_then(serde_json::Value::as_u64)
            && token != self.bridge.view_revision()
        {
            let current = self.bridge.view_revision();
            self.audit.append(
                self.approval.clone(),
                &self.grant,
                name,
                object,
                Quota::zero(),
                AuditOutcome::RefusedStaleInput { token, current },
            );
            return PolicyOutcome::Refused(PolicyRefusal {
                kind: RefusalKind::StaleInput,
                detail: format!(
                    "stale-decision token names revision {token}, current revision is {current}"
                ),
                park_event: None,
            });
        }
        if let Some(limit) = self.rate_limit_hit(name, now) {
            self.audit.append(
                self.approval.clone(),
                &self.grant,
                name,
                object,
                Quota::zero(),
                AuditOutcome::RefusedRateLimit {
                    tool: name.to_owned(),
                    limit: limit.max_calls,
                    window_secs: limit.window_secs,
                },
            );
            return PolicyOutcome::Refused(PolicyRefusal {
                kind: RefusalKind::RateLimited,
                detail: format!(
                    "rate limit for {name}: {} admitted calls per {}s window",
                    limit.max_calls, limit.window_secs
                ),
                park_event: None,
            });
        }
        let charge = charge_for(name);
        if let Err(exhaustion) = self.ledger.charge(now, charge) {
            self.parked = true;
            let axis = exhaustion.axis;
            let replenishes_at = exhaustion.replenishes_at;
            self.parked_replenishes_at = replenishes_at;
            self.audit.append(
                self.approval.clone(),
                &self.grant,
                name,
                object,
                Quota::zero(),
                AuditOutcome::RefusedQuota { axis: axis.clone() },
            );
            let detail = match replenishes_at {
                Some(at) => format!("quota exhausted on the {axis} axis; replenishes at {at}"),
                None => format!("quota exhausted on the {axis} axis; the budget never replenishes"),
            };
            return PolicyOutcome::Refused(PolicyRefusal {
                kind: RefusalKind::Quota,
                detail,
                park_event: Some(ParkEvent {
                    kind: "agent_parked".to_owned(),
                    reason: "quota_exhausted".to_owned(),
                    message: QUOTA_PARK_EVENT.to_owned(),
                    agent_key: self.grant.agent_key,
                    world: self.grant.world.clone(),
                    at: now,
                    quota_spent: self.ledger.spend_at(now),
                    quota_budget: self.ledger.budget(),
                    replenishes_at,
                }),
            });
        }
        self.admitted_at
            .entry(name.to_owned())
            .or_default()
            .push_back(now);
        self.execute(name, arguments, object, charge)
    }

    /// Run one action under a one-shot owner approval (design #6301
    /// section 2.5, #8302). The approval lifts the capability
    /// ceiling for exactly this call and nothing else: the exit,
    /// parking, the stale-decision rule, the rate limit and the
    /// quota charge all run as for [`Self::act`], and the one audit
    /// event the call writes carries `approval` as its
    /// [`AuditEvent::approved_by`]. The grant — and so the role — is
    /// never touched: the next identical call under the agent's own
    /// authority is refused by the ceiling again.
    ///
    /// Only the owner review queue reaches this
    /// ([`crate::owner_review::ReviewQueue::approve`]); no tool call
    /// does.
    pub(crate) fn act_owner_approved(
        &mut self,
        approval: OwnerApproval,
        name: &str,
        arguments: &serde_json::Value,
        now: u64,
    ) -> PolicyOutcome {
        self.approval = Some(approval);
        let outcome = self.act(name, arguments, now);
        self.approval = None;
        outcome
    }

    /// The rate limit this call hits at `now`, if any (design
    /// section 6.3): the role's limit for the tool, once the
    /// admitted calls inside the trailing window reach its cap.
    /// Expired admissions slide out of the window as a side effect.
    fn rate_limit_hit(&mut self, name: &str, now: u64) -> Option<RateLimit> {
        let limit = self
            .grant
            .rate_limits
            .iter()
            .find(|limit| limit.tool == name)?
            .clone();
        let window = self.admitted_at.entry(name.to_owned()).or_default();
        while window
            .front()
            .is_some_and(|at| now.saturating_sub(*at) >= limit.window_secs)
        {
            window.pop_front();
        }
        (window.len() as u64 >= limit.max_calls).then_some(limit)
    }

    /// The ceiling check (step 1). Returns the refusing leg when the
    /// action is outside the ceiling.
    fn ceiling_refusal(&self, name: &str, arguments: &serde_json::Value) -> Option<CeilingLeg> {
        if !self.grant.ceiling.tools.contains(name) {
            return Some(CeilingLeg::Tool(name.to_owned()));
        }
        match name {
            "world.join" if !self.grant.ceiling.worlds.contains(&self.grant.world) => {
                Some(CeilingLeg::World(self.grant.world.clone()))
            }
            "object.inspect" => self.inspect_class_refusal(arguments),
            "program.invoke" => {
                match arguments.get("program").and_then(serde_json::Value::as_str) {
                    Some(program) if self.grant.ceiling.program_allowlist.contains(program) => None,
                    Some(program) => Some(CeilingLeg::Program(program.to_owned())),
                    None => Some(CeilingLeg::NoProgram),
                }
            }
            _ => None,
        }
    }

    /// The object-class leg of the ceiling check: the target's class
    /// must be established (observed on a frame, or carried by a
    /// `class:id` entity reference) and named in the ceiling.
    fn inspect_class_refusal(&self, arguments: &serde_json::Value) -> Option<CeilingLeg> {
        let entity_ref = arguments.get("entity_ref")?;
        let class = self
            .object_classes
            .get(&entity_key(entity_ref))
            .cloned()
            .or_else(|| {
                entity_ref
                    .as_str()
                    .and_then(|text| text.split_once(':').map(|(class, _)| class.to_owned()))
            });
        match class {
            Some(class) if self.grant.ceiling.object_classes.contains(&class) => None,
            Some(class) => Some(CeilingLeg::ObjectClass(class)),
            None => Some(CeilingLeg::ClassNotEstablished),
        }
    }

    /// Execute an admitted action against the bridge and audit the
    /// verdict either way.
    fn execute(
        &mut self,
        name: &str,
        arguments: &serde_json::Value,
        object: Option<String>,
        charge: Quota,
    ) -> PolicyOutcome {
        match self.bridge.tool_action(name, arguments) {
            Ok(action) => {
                self.audit.append(
                    self.approval.clone(),
                    &self.grant,
                    name,
                    object,
                    charge,
                    AuditOutcome::Executed,
                );
                PolicyOutcome::Executed(action)
            }
            Err(error) => {
                let detail = format!("{error:?}");
                self.audit.append(
                    self.approval.clone(),
                    &self.grant,
                    name,
                    object,
                    charge,
                    AuditOutcome::RefusedBridge {
                        detail: detail.clone(),
                    },
                );
                PolicyOutcome::Refused(PolicyRefusal {
                    kind: RefusalKind::Bridge,
                    detail,
                    park_event: None,
                })
            }
        }
    }
}

/// The map key for an entity reference: the string itself when the
/// reference is a string, its JSON rendering otherwise (the same
/// keying the bridge's observed-object map uses).
fn entity_key(entity_ref: &serde_json::Value) -> String {
    entity_ref
        .as_str()
        .map_or_else(|| entity_ref.to_string(), ToOwned::to_owned)
}

/// The object an action names, for the audit record: the inspect
/// target or the invoked program; nothing for the other tools.
pub(crate) fn audit_object(name: &str, arguments: &serde_json::Value) -> Option<String> {
    match name {
        "object.inspect" => arguments.get("entity_ref").map(entity_key),
        "program.invoke" => arguments
            .get("program")
            .and_then(serde_json::Value::as_str)
            .map(ToOwned::to_owned),
        _ => None,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::mcp_bridge::v1_tool_definitions;
    use serde_json::json;

    fn host_key() -> SigningKey {
        SigningKey::from_bytes(&[7; 32])
    }

    const TEST_NOW: u64 = 1_700_000_000;

    fn grant() -> RoleGrant {
        let mut tools: BTreeSet<String> = v1_tool_definitions()
            .iter()
            .map(|definition| definition.name.clone())
            .collect();
        tools.insert("program.invoke".to_owned());
        RoleGrant {
            agent_key: [3; 32],
            agent_label: "archivist".to_owned(),
            owner: "drew".to_owned(),
            world: "tic-tac-toe".to_owned(),
            capability_ref: "grant-1".to_owned(),
            ceiling: CapabilityCeiling {
                tools,
                worlds: BTreeSet::from(["tic-tac-toe".to_owned()]),
                object_classes: BTreeSet::from(["cell".to_owned()]),
                program_allowlist: BTreeSet::from(["scorekeeper".to_owned()]),
            },
            role: "archivist".to_owned(),
            quota_class: QuotaClass {
                budget: Quota {
                    frames: 8,
                    tool_calls: 32,
                    program_invocations: 1,
                },
                replenishment: None,
            },
            rate_limits: Vec::new(),
        }
    }

    fn session(grant: RoleGrant) -> PolicySession {
        PolicySession::new(
            grant,
            BridgeSession::start(&v1_tool_definitions()).unwrap(),
            host_key(),
            TEST_NOW,
        )
    }

    fn joined(grant: RoleGrant) -> PolicySession {
        let mut session = session(grant);
        let outcome = session.act("world.join", &json!({ "capability": "cap-1" }), TEST_NOW);
        assert!(
            matches!(outcome, PolicyOutcome::Executed(ToolAction::SendFrame(_))),
            "{outcome:?}"
        );
        session
    }

    fn last_outcome(session: &PolicySession) -> &AuditOutcome {
        &session.audit_events().last().unwrap().outcome
    }

    #[test]
    fn ceiling_refusal_is_audited_and_never_executed() {
        // A tool outside the ceiling: refused before the bridge, so
        // no frame exists and the session state does not move.
        {
            let mut scoped_grant = grant();
            scoped_grant.ceiling.tools.remove("chat.post");
            let mut scoped_session = joined(scoped_grant);
            let outcome = scoped_session.act("chat.post", &json!({ "text": "hello" }), TEST_NOW);
            assert!(matches!(
                outcome,
                PolicyOutcome::Refused(PolicyRefusal {
                    kind: RefusalKind::Ceiling,
                    ..
                })
            ));
            assert!(matches!(
                last_outcome(&scoped_session),
                AuditOutcome::RefusedCeiling { .. }
            ));
            assert_eq!(
                scoped_session.audit_events().last().unwrap().quota_charged,
                Quota::zero()
            );
        }

        // A world outside the ceiling: the join is refused and the
        // bridge never joins.
        let mut elsewhere_grant = grant();
        elsewhere_grant.world = "elsewhere".to_owned();
        let mut elsewhere_session = session(elsewhere_grant);
        let outcome =
            elsewhere_session.act("world.join", &json!({ "capability": "cap-1" }), TEST_NOW);
        assert!(matches!(
            outcome,
            PolicyOutcome::Refused(PolicyRefusal {
                kind: RefusalKind::Ceiling,
                ..
            })
        ));
        assert!(!elsewhere_session.bridge().is_joined());
    }

    #[test]
    fn replenishing_quota_parks_naming_the_next_tick() {
        let mut grant = grant();
        grant.quota_class = QuotaClass {
            budget: Quota {
                frames: 8,
                tool_calls: 2,
                program_invocations: 1,
            },
            replenishment: Some(crate::role_config::Replenishment {
                every_secs: 60,
                amount: Quota {
                    frames: 0,
                    tool_calls: 1,
                    program_invocations: 0,
                },
            }),
        };
        let mut session = joined(grant);
        session.act("screen.read", &json!({}), TEST_NOW);
        let outcome = session.act("screen.read", &json!({}), TEST_NOW + 10);
        let PolicyOutcome::Refused(refusal) = outcome else {
            panic!("quota refusal expected, got {outcome:?}");
        };
        let park = refusal.park_event.expect("park event present");
        assert_eq!(park.replenishes_at, Some(TEST_NOW + 60));
        assert_eq!(park.quota_budget.tool_calls, 2);
        assert!(refusal.detail.contains(&(TEST_NOW + 60).to_string()));
        assert_eq!(session.spend(TEST_NOW + 60).tool_calls, 1);
    }

    #[test]
    fn park_lifts_once_a_replenishment_tick_lets_the_ledger_admit() {
        let mut grant = grant();
        grant.quota_class = QuotaClass {
            budget: Quota {
                frames: 8,
                tool_calls: 2,
                program_invocations: 1,
            },
            replenishment: Some(crate::role_config::Replenishment {
                every_secs: 60,
                amount: Quota {
                    frames: 0,
                    tool_calls: 1,
                    program_invocations: 0,
                },
            }),
        };
        let mut session = joined(grant);
        session.act("screen.read", &json!({}), TEST_NOW);
        // The budget is spent; the next action parks the agent,
        // naming the tick at TEST_NOW + 60.
        let outcome = session.act("screen.read", &json!({}), TEST_NOW + 10);
        assert!(matches!(
            outcome,
            PolicyOutcome::Refused(PolicyRefusal {
                kind: RefusalKind::Quota,
                ..
            })
        ));
        assert!(session.is_parked());
        // Before the tick the park holds, charged nothing.
        let spend_before = session.spend(TEST_NOW + 30);
        let outcome = session.act("screen.read", &json!({}), TEST_NOW + 30);
        assert!(matches!(
            outcome,
            PolicyOutcome::Refused(PolicyRefusal {
                kind: RefusalKind::Parked,
                ..
            })
        ));
        assert!(session.is_parked());
        assert_eq!(session.spend(TEST_NOW + 30), spend_before);
        // At the tick the ledger can admit again: the same session
        // acts without leaving and rejoining, charged through the
        // normal order.
        let outcome = session.act("screen.read", &json!({}), TEST_NOW + 60);
        assert!(matches!(outcome, PolicyOutcome::Executed(_)), "{outcome:?}");
        assert!(!session.is_parked());
        assert!(matches!(last_outcome(&session), AuditOutcome::Executed));
        assert_eq!(session.spend(TEST_NOW + 60).tool_calls, 2);
    }

    #[test]
    fn never_replenishing_park_holds_until_rejoin() {
        let mut grant = grant();
        grant.quota_class.budget.tool_calls = 2; // replenishment stays None
        let mut session = joined(grant);
        session.act("screen.read", &json!({}), TEST_NOW);
        let outcome = session.act("screen.read", &json!({}), TEST_NOW);
        let PolicyOutcome::Refused(refusal) = outcome else {
            panic!("quota refusal expected, got {outcome:?}");
        };
        assert_eq!(
            refusal
                .park_event
                .expect("park event present")
                .replenishes_at,
            None
        );
        // No tick exists to wait for: a day later the park still
        // holds, and only the exit remains.
        let outcome = session.act("screen.read", &json!({}), TEST_NOW + 86_400);
        assert!(matches!(
            outcome,
            PolicyOutcome::Refused(PolicyRefusal {
                kind: RefusalKind::Parked,
                ..
            })
        ));
        assert!(session.is_parked());
        assert_eq!(
            session.act("world.leave", &json!({}), TEST_NOW + 86_400),
            PolicyOutcome::Executed(ToolAction::Close)
        );
    }

    #[test]
    fn park_holds_when_elapsed_ticks_do_not_regain_the_exhausted_axis() {
        let mut grant = grant();
        grant.quota_class = QuotaClass {
            budget: Quota {
                frames: 1,
                tool_calls: 32,
                program_invocations: 1,
            },
            replenishment: Some(crate::role_config::Replenishment {
                every_secs: 60,
                amount: Quota {
                    frames: 0,
                    tool_calls: 1,
                    program_invocations: 0,
                },
            }),
        };
        let mut session = joined(grant); // the join spends the only frame
        let outcome = session.act("chat.post", &json!({ "text": "hello" }), TEST_NOW);
        let PolicyOutcome::Refused(refusal) = outcome else {
            panic!("quota refusal expected, got {outcome:?}");
        };
        assert_eq!(refusal.kind, RefusalKind::Quota);
        assert_eq!(
            refusal
                .park_event
                .expect("park event present")
                .replenishes_at,
            Some(TEST_NOW + 60)
        );
        // Ticks elapse and tool calls replenish, but the frames
        // axis never regains budget: the park holds.
        let outcome = session.act("chat.post", &json!({ "text": "hello" }), TEST_NOW + 600);
        assert!(matches!(
            outcome,
            PolicyOutcome::Refused(PolicyRefusal {
                kind: RefusalKind::Parked,
                ..
            })
        ));
        assert!(session.is_parked());
    }

    #[test]
    fn quota_exhaustion_parks_the_agent_visibly() {
        let mut grant = grant();
        grant.quota_class.budget.frames = 1; // the join spends the only frame
        let mut session = joined(grant);
        let outcome = session.act("chat.post", &json!({ "text": "hello" }), TEST_NOW);
        let PolicyOutcome::Refused(refusal) = outcome else {
            panic!("quota refusal expected, got {outcome:?}");
        };
        assert_eq!(refusal.kind, RefusalKind::Quota);
        let park = refusal.park_event.as_ref().expect("park event present");
        assert_eq!(park.kind, "agent_parked");
        assert_eq!(park.reason, "quota_exhausted");
        assert_eq!(park.message, QUOTA_PARK_EVENT);
        assert_eq!(park.world, "tic-tac-toe");
        assert_eq!(park.quota_budget.frames, 1);
        assert_eq!(park.quota_spent.frames, 1);
        assert!(session.is_parked());
        assert!(matches!(
            last_outcome(&session),
            AuditOutcome::RefusedQuota { .. }
        ));
        // Parked actions are refused without a charge...
        let spend_before = session.spend(TEST_NOW);
        let outcome = session.act("screen.read", &json!({}), TEST_NOW);
        assert!(matches!(
            outcome,
            PolicyOutcome::Refused(PolicyRefusal {
                kind: RefusalKind::Parked,
                ..
            })
        ));
        assert_eq!(session.spend(TEST_NOW), spend_before);
        // ...but the agent can always leave.
        assert_eq!(
            session.act("world.leave", &json!({}), TEST_NOW),
            PolicyOutcome::Executed(ToolAction::Close)
        );
        let summary = session.owner_summary(TEST_NOW);
        assert!(summary.parked);
        assert!(summary.render().contains("parked, quota exhausted"));
    }

    #[test]
    fn audit_chain_verifies_end_to_end_and_tampering_fails() {
        let mut session = joined(grant());
        session.act(
            "screen.subscribe",
            &json!({ "event_classes": ["chat_posted"] }),
            TEST_NOW,
        );
        session.act("chat.post", &json!({ "text": "hello" }), TEST_NOW);
        session.act("host.exec", &json!({}), TEST_NOW); // ceiling refusal
        session.verify_audit().unwrap();

        let key = session.host_verifying_key();
        let mut tampered = session.audit_events().to_vec();
        tampered[1].tool = "host.exec".to_owned();
        assert_eq!(
            verify_audit_chain(&tampered, &key),
            Err(AuditError::HashMismatch { seq: 1 })
        );

        let mut resigned = session.audit_events().to_vec();
        resigned[0].signature[0] ^= 0xff;
        assert_eq!(
            verify_audit_chain(&resigned, &key),
            Err(AuditError::BadSignature { seq: 0 })
        );

        let mut relinked = session.audit_events().to_vec();
        relinked.remove(1);
        assert!(matches!(
            verify_audit_chain(&relinked, &key),
            Err(AuditError::BrokenLink { .. })
        ));
    }

    #[test]
    fn audit_events_round_trip_through_serde_and_still_verify() {
        let mut session = joined(grant());
        session.act("chat.post", &json!({ "text": "hello" }), TEST_NOW);
        let bytes = serde_json::to_vec(session.audit_events()).unwrap();
        let restored: Vec<AuditEvent> = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(restored, session.audit_events());
        verify_audit_chain(&restored, &session.host_verifying_key()).unwrap();
    }

    #[test]
    fn owner_summary_counts_actions_spend_and_refusals() {
        let mut session = joined(grant()); // 1 frame + 1 tool call
        session.act(
            "screen.subscribe",
            &json!({ "event_classes": ["chat_posted"] }),
            TEST_NOW,
        ); // 1 frame + 1 tool call
        session.act("screen.read", &json!({}), TEST_NOW); // 1 tool call
        session.act("chat.post", &json!({ "text": "hi" }), TEST_NOW); // 1 frame + 1 tool call
        session.act("host.exec", &json!({}), TEST_NOW); // ceiling refusal, no charge
        let summary = session.owner_summary(TEST_NOW);
        assert_eq!(summary.actions, 5);
        assert_eq!(summary.executed, 4);
        assert_eq!(summary.refusals, 1);
        assert_eq!(
            summary.spend,
            Quota {
                frames: 3,
                tool_calls: 4,
                program_invocations: 0,
            }
        );
        let rendered = summary.render();
        assert!(rendered.contains("archivist"), "{rendered}");
        assert!(rendered.contains("was refused 1 times"), "{rendered}");
        // The summary is the audit log, counted: nothing is sampled.
        assert_eq!(summary.actions, session.audit_events().len() as u64);
    }

    #[test]
    fn stale_input_rejection_is_audited_and_never_executed() {
        let mut session = joined(grant());
        session.observe_frame(&json!({
            "type": "view",
            "sequence": 1,
            "data": "board",
            "view_revision": 3,
        }));
        let outcome = session.act(
            "chat.post",
            &json!({ "text": "move", "screen_revision": 2 }),
            TEST_NOW,
        );
        assert!(matches!(
            outcome,
            PolicyOutcome::Refused(PolicyRefusal {
                kind: RefusalKind::StaleInput,
                ..
            })
        ));
        assert_eq!(
            last_outcome(&session),
            &AuditOutcome::RefusedStaleInput {
                token: 2,
                current: 3,
            }
        );
        // The same call on the current revision is admitted.
        let outcome = session.act(
            "chat.post",
            &json!({ "text": "move", "screen_revision": 3 }),
            TEST_NOW,
        );
        assert!(matches!(
            outcome,
            PolicyOutcome::Executed(ToolAction::SendFrame(_))
        ));
    }

    #[test]
    fn inspect_checks_the_object_class_ceiling() {
        let mut session = joined(grant());
        session.observe_frame(&json!({
            "type": "semantic_diff",
            "sequence": 2,
            "view_revision": 1,
            "changes": [
                { "kind": "object_changed", "entity_ref": "cell-4", "object_class": "cell" },
                { "kind": "object_changed", "entity_ref": "safe-1", "object_class": "vault" },
            ],
        }));
        // An observed class inside the ceiling: admitted.
        assert!(matches!(
            session.act(
                "object.inspect",
                &json!({ "entity_ref": "cell-4" }),
                TEST_NOW
            ),
            PolicyOutcome::Executed(ToolAction::Local(_))
        ));
        // An observed class outside the ceiling: refused.
        assert!(matches!(
            session.act(
                "object.inspect",
                &json!({ "entity_ref": "safe-1" }),
                TEST_NOW
            ),
            PolicyOutcome::Refused(PolicyRefusal {
                kind: RefusalKind::Ceiling,
                ..
            })
        ));
        // A class:id reference carries its class without a frame.
        assert!(matches!(
            session.act(
                "object.inspect",
                &json!({ "entity_ref": "vault:9" }),
                TEST_NOW
            ),
            PolicyOutcome::Refused(PolicyRefusal {
                kind: RefusalKind::Ceiling,
                ..
            })
        ));
    }

    #[test]
    fn program_invoke_checks_the_allowlist_and_charges_the_program_axis() {
        let mut session = joined(grant());
        // Outside the allowlist: ceiling refusal, no program charge.
        assert!(matches!(
            session.act("program.invoke", &json!({ "program": "wipe" }), TEST_NOW),
            PolicyOutcome::Refused(PolicyRefusal {
                kind: RefusalKind::Ceiling,
                ..
            })
        ));
        assert_eq!(session.spend(TEST_NOW).program_invocations, 0);
        // Inside the allowlist: admitted, charged on the program
        // axis, and executed — the bridge translates the invoke into
        // the single `input` frame that carries it (spike step 5).
        assert_eq!(
            session.act(
                "program.invoke",
                &json!({ "program": "scorekeeper", "object": "board", "arguments": "4" }),
                TEST_NOW
            ),
            PolicyOutcome::Executed(ToolAction::SendFrame(
                json!({ "type": "input", "data_hex": "34" })
            ))
        );
        assert_eq!(session.spend(TEST_NOW).program_invocations, 1);
        assert!(matches!(last_outcome(&session), AuditOutcome::Executed));
        assert_eq!(
            session.audit_events().last().unwrap().object.as_deref(),
            Some("scorekeeper")
        );
    }

    #[test]
    fn the_agent_cannot_mutate_the_log_or_its_quota() {
        let mut session = joined(grant());
        session.act("chat.post", &json!({ "text": "hello" }), TEST_NOW);
        // The spend comes out by value: scribbling on the copy does
        // not reach the session's ledger.
        let mut copy = session.spend(TEST_NOW);
        copy.frames = 0;
        copy.tool_calls = 0;
        assert_eq!(copy.tool_calls, 0); // the copy really was scribbled on
        assert_eq!(session.spend(TEST_NOW).tool_calls, 2);
        // The log comes out as a shared slice; the chain still
        // verifies after every read.
        assert_eq!(session.audit_events().len(), 2);
        session.verify_audit().unwrap();
    }

    #[test]
    fn rate_limit_refuses_beyond_the_role_limit_and_the_window_slides() {
        let mut grant = grant();
        grant.rate_limits = vec![RateLimit {
            tool: "chat.post".to_owned(),
            max_calls: 2,
            window_secs: 60,
        }];
        let mut session = joined(grant);
        // Two admitted posts inside the window. (The join is a
        // different tool and never counts against this limit.)
        assert!(matches!(
            session.act("chat.post", &json!({ "text": "one" }), TEST_NOW),
            PolicyOutcome::Executed(_)
        ));
        assert!(matches!(
            session.act("chat.post", &json!({ "text": "two" }), TEST_NOW + 1),
            PolicyOutcome::Executed(_)
        ));
        // The third is refused by the rate limit, audited, and
        // charged nothing.
        let spend_before = session.spend(TEST_NOW);
        let outcome = session.act("chat.post", &json!({ "text": "three" }), TEST_NOW + 2);
        assert!(matches!(
            outcome,
            PolicyOutcome::Refused(PolicyRefusal {
                kind: RefusalKind::RateLimited,
                ..
            })
        ));
        assert!(matches!(
            last_outcome(&session),
            AuditOutcome::RefusedRateLimit { .. }
        ));
        assert_eq!(session.spend(TEST_NOW), spend_before);
        // Once the window slides past the admitted calls, the tool
        // admits again.
        assert!(matches!(
            session.act("chat.post", &json!({ "text": "later" }), TEST_NOW + 62),
            PolicyOutcome::Executed(_)
        ));
    }

    #[test]
    fn owner_summary_breaks_refusals_down_by_code() {
        let mut session = joined(grant());
        session.act("host.exec", &json!({}), TEST_NOW); // ceiling refusal
        session.act("host.exec", &json!({}), TEST_NOW); // ceiling refusal again
        let summary = session.owner_summary(TEST_NOW);
        assert_eq!(summary.refusals, 2);
        assert_eq!(summary.refusals_by_code.get("ceiling"), Some(&2));
        assert_eq!(summary.refusals_by_code.len(), 1);
    }

    #[test]
    fn stored_session_survives_leave_and_rejoin_without_resetting_spend() {
        let mut store = QuotaLedgerStore::new();
        let first_grant = grant();
        let agent = first_grant.agent_key;
        let world = first_grant.world.clone();

        let mut first = PolicySession::open_stored(
            first_grant,
            BridgeSession::start(&v1_tool_definitions()).unwrap(),
            host_key(),
            &mut store,
            TEST_NOW,
        )
        .unwrap();
        assert!(matches!(
            first.act("world.join", &json!({ "capability": "cap-1" }), TEST_NOW),
            PolicyOutcome::Executed(_)
        ));
        first.act("screen.read", &json!({}), TEST_NOW);
        let spent = first.spend(TEST_NOW);
        // The leave is never refused and is charged nothing; the
        // driver then hands the ledger back to the store.
        first.act("world.leave", &json!({}), TEST_NOW);
        let spent_after_leave = first.spend(TEST_NOW);
        assert_eq!(spent_after_leave, spent);
        first.close(&mut store);

        // The owner sees the spend while the agent is away.
        assert_eq!(
            store.spend_for_agent(&agent, &world, TEST_NOW),
            Some(spent_after_leave)
        );

        // Rejoining reopens the same ledger: no fresh budget.
        let mut second = PolicySession::open_stored(
            grant(),
            BridgeSession::start(&v1_tool_definitions()).unwrap(),
            host_key(),
            &mut store,
            TEST_NOW + 1,
        )
        .unwrap();
        assert_eq!(second.spend(TEST_NOW + 1), spent_after_leave);
        second.act(
            "world.join",
            &json!({ "capability": "cap-1" }),
            TEST_NOW + 1,
        );
        second.act("screen.read", &json!({}), TEST_NOW + 1);
        assert!(second.spend(TEST_NOW + 1).tool_calls > spent_after_leave.tool_calls);
    }

    #[test]
    fn stored_session_refuses_a_second_live_open_for_the_same_pair() {
        let mut store = QuotaLedgerStore::new();
        let first = PolicySession::open_stored(
            grant(),
            BridgeSession::start(&v1_tool_definitions()).unwrap(),
            host_key(),
            &mut store,
            TEST_NOW,
        )
        .unwrap();

        // A duplicate grant opening a second live session for the
        // same (agent, world) pair is refused (#14526): two live
        // sessions would each hold a clone of one ledger and
        // double its budget.
        let refusal = PolicySession::open_stored(
            grant(),
            BridgeSession::start(&v1_tool_definitions()).unwrap(),
            host_key(),
            &mut store,
            TEST_NOW,
        )
        .err()
        .expect("second live open must be refused");
        assert_eq!(refusal.agent_key, grant().agent_key);
        assert_eq!(refusal.world, grant().world);

        // Once the first session is handed back, a rejoin opens.
        first.close(&mut store);
        let rejoined = PolicySession::open_stored(
            grant(),
            BridgeSession::start(&v1_tool_definitions()).unwrap(),
            host_key(),
            &mut store,
            TEST_NOW + 1,
        );
        assert!(rejoined.is_ok());
    }

    /// The #8301 acceptance against the #1067 (F12.10) consumed
    /// shape: v1 §9.3 keeps the bridge and audit shapes fixed while
    /// the quota system lands, so the per-agent ledger must emit
    /// exactly the events F12.10 consumes — per-action
    /// `quota_charged` on every audit event, a `RefusedQuota` naming
    /// the axis plus a visible park event on exhaustion — with the
    /// v1 field sets intact. This test pins those shapes: any
    /// additive field (like `replenishes_at`) stays additive, and
    /// any rename or removal fails here first.
    #[test]
    fn ledger_events_keep_the_f12_10_consumed_shape() {
        let mut grant = grant();
        grant.quota_class = QuotaClass {
            budget: Quota {
                frames: 8,
                tool_calls: 3,
                program_invocations: 0,
            },
            replenishment: Some(crate::role_config::Replenishment {
                every_secs: 60,
                amount: Quota {
                    frames: 0,
                    tool_calls: 1,
                    program_invocations: 0,
                },
            }),
        };
        let budget = grant.quota_class.budget;
        let mut session = joined(grant);

        // Every admitted action charges exactly the v1 per-action
        // charge (charging parity with v1, #8301 acceptance).
        session.act("screen.read", &json!({}), TEST_NOW);
        session.act("chat.post", &json!({ "text": "hi" }), TEST_NOW);
        for event in session.audit_events() {
            assert_eq!(event.quota_charged, charge_for(&event.tool));
            assert!(matches!(event.outcome, AuditOutcome::Executed));
            assert_eq!(event.world, "tic-tac-toe");
        }

        // Exhaustion refuses on the axis, parks visibly, and names
        // the next replenishment tick.
        let outcome = session.act("chat.post", &json!({ "text": "again" }), TEST_NOW);
        let PolicyOutcome::Refused(refusal) = outcome else {
            panic!("quota refusal expected, got {outcome:?}");
        };
        assert_eq!(refusal.kind, RefusalKind::Quota);
        assert!(matches!(
            last_outcome(&session),
            AuditOutcome::RefusedQuota { axis } if axis == "tool_calls"
        ));
        let park = refusal.park_event.expect("park event present");
        assert_eq!(park.reason, "quota_exhausted");
        assert_eq!(park.quota_budget, budget);
        assert_eq!(park.quota_spent, session.spend(TEST_NOW));
        assert_eq!(park.replenishes_at, Some(TEST_NOW + 60));

        // Shape pins: the exact field sets F12.10 consumes. Quota
        // stays the three-axis vector; the audit event keeps its v1
        // fields; the park event keeps its v1 fields plus the
        // additive replenishment time.
        let quota_keys: BTreeSet<String> = serde_json::to_value(Quota {
            frames: 1,
            tool_calls: 2,
            program_invocations: 3,
        })
        .unwrap()
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect();
        assert_eq!(
            quota_keys,
            BTreeSet::from([
                "frames".to_owned(),
                "tool_calls".to_owned(),
                "program_invocations".to_owned(),
            ])
        );

        let audit_keys: BTreeSet<String> =
            serde_json::to_value(session.audit_events().first().unwrap())
                .unwrap()
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect();
        assert_eq!(
            audit_keys,
            BTreeSet::from([
                "agent_key".to_owned(),
                "capability_ref".to_owned(),
                "hash".to_owned(),
                "object".to_owned(),
                "outcome".to_owned(),
                "owner".to_owned(),
                "prev_hash".to_owned(),
                "quota_charged".to_owned(),
                "seq".to_owned(),
                "signature".to_owned(),
                "tool".to_owned(),
                "world".to_owned(),
            ])
        );

        let park_keys: BTreeSet<String> = serde_json::to_value(&park)
            .unwrap()
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect();
        assert_eq!(
            park_keys,
            BTreeSet::from([
                "agent_key".to_owned(),
                "at".to_owned(),
                "kind".to_owned(),
                "message".to_owned(),
                "quota_budget".to_owned(),
                "quota_spent".to_owned(),
                "reason".to_owned(),
                "replenishes_at".to_owned(),
                "world".to_owned(),
            ])
        );
    }
}
