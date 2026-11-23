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
//! `world.leave` is the exit, not an in-world action: it is never
//! refused by the ceiling or the quota (an agent must always be able
//! to leave, even parked or out of budget), it is charged nothing, and
//! it is still audited.
//!
//! `program.invoke` arrives with spike step 5. The policy point
//! already enforces its allowlist and charges its quota axis; until
//! the bridge grows the tool, an admitted invoke ends as an audited
//! bridge refusal (`UnknownTool`), charged like any admitted attempt.

use std::collections::{BTreeMap, BTreeSet};

use ed25519_dalek::{Signature, Signer as _, SigningKey, Verifier as _, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::mcp_bridge::{BridgeError, BridgeSession, ToolAction, ToolDefinition};

/// The world event text emitted when quota exhaustion parks an agent
/// (design section 6.2, findings Q3b: visible, never silent).
pub const QUOTA_PARK_EVENT: &str = "agent parked, quota exhausted";

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

    const fn add(&mut self, charge: Self) {
        self.frames += charge.frames;
        self.tool_calls += charge.tool_calls;
        self.program_invocations += charge.program_invocations;
    }

    /// The first axis on which `spend + charge` would exceed
    /// `budget`, if any.
    const fn exceeded_axis(budget: Self, spend: Self, charge: Self) -> Option<&'static str> {
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
/// one tool call; the frame-shaped tools cost one frame;
/// `program.invoke` costs one program invocation.
fn charge_for(tool: &str) -> Quota {
    Quota {
        frames: u64::from(matches!(
            tool,
            "world.join" | "screen.subscribe" | "chat.post"
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
    /// The per-session quota budget for this session.
    pub quota: Quota,
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
    /// The policy point admitted the action and the bridge refused
    /// it (unknown tool, invalid arguments, no frame mapping, ...).
    /// The charge stands: the attempt consumed the host's work.
    RefusedBridge {
        /// The bridge refusal, debug-formatted.
        detail: String,
    },
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
    /// parked the agent ([`QUOTA_PARK_EVENT`]); `None` otherwise.
    pub park_event: Option<String>,
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
    spend: Quota,
    parked: bool,
    object_classes: BTreeMap<String, String>,
}

impl PolicySession {
    /// Open a policy session for one role grant. The bridge session
    /// is started (and its definitions pinned) by the driver, as in
    /// [`BridgeSession::start`]; `host_key` signs the audit chain.
    #[must_use]
    pub const fn new(grant: RoleGrant, bridge: BridgeSession, host_key: SigningKey) -> Self {
        Self {
            grant,
            bridge,
            audit: AuditLog::new(host_key),
            spend: Quota::zero(),
            parked: false,
            object_classes: BTreeMap::new(),
        }
    }

    /// The role grant this session was opened with.
    #[must_use]
    pub const fn grant(&self) -> &RoleGrant {
        &self.grant
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

    /// The quota spent so far. A copy: the agent cannot reach the
    /// session's own ledger through it.
    #[must_use]
    pub const fn spend(&self) -> Quota {
        self.spend
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

    /// The owner summary over the whole session so far.
    #[must_use]
    pub fn owner_summary(&self) -> OwnerSummary {
        let mut executed = 0u64;
        let mut refusals = 0u64;
        for event in self.audit.events() {
            match event.outcome {
                AuditOutcome::Executed => executed += 1,
                _ => refusals += 1,
            }
        }
        OwnerSummary {
            agent_label: self.grant.agent_label.clone(),
            owner: self.grant.owner.clone(),
            world: self.grant.world.clone(),
            actions: self.audit.events().len() as u64,
            executed,
            refusals,
            spend: self.spend,
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
    /// design's order: ceiling, quota, execution — with exactly one
    /// audit event whatever the verdict.
    pub fn act(&mut self, name: &str, arguments: &serde_json::Value) -> PolicyOutcome {
        let object = audit_object(name, arguments);
        // The exit is never refused: not by the ceiling, not by the
        // quota, not by parking. It is charged nothing and audited.
        if name == "world.leave" {
            return self.execute(name, arguments, object, Quota::zero());
        }
        if self.parked {
            self.audit.append(
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
        if let Some(detail) = self.ceiling_refusal(name, arguments) {
            self.audit.append(
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
        let charge = charge_for(name);
        if let Some(axis) = Quota::exceeded_axis(self.grant.quota, self.spend, charge) {
            self.parked = true;
            self.audit.append(
                &self.grant,
                name,
                object,
                Quota::zero(),
                AuditOutcome::RefusedQuota {
                    axis: axis.to_owned(),
                },
            );
            return PolicyOutcome::Refused(PolicyRefusal {
                kind: RefusalKind::Quota,
                detail: format!("quota exhausted on the {axis} axis"),
                park_event: Some(QUOTA_PARK_EVENT.to_owned()),
            });
        }
        self.spend.add(charge);
        self.execute(name, arguments, object, charge)
    }

    /// The ceiling check (step 1). Returns the refusal detail when
    /// the action is outside the ceiling.
    fn ceiling_refusal(&self, name: &str, arguments: &serde_json::Value) -> Option<String> {
        if !self.grant.ceiling.tools.contains(name) {
            return Some(format!("tool {name} is outside the capability ceiling"));
        }
        match name {
            "world.join" if !self.grant.ceiling.worlds.contains(&self.grant.world) => {
                Some(format!(
                    "world {} is outside the capability ceiling",
                    self.grant.world
                ))
            }
            "object.inspect" => self.inspect_class_refusal(arguments),
            "program.invoke" => {
                match arguments.get("program").and_then(serde_json::Value::as_str) {
                    Some(program) if self.grant.ceiling.program_allowlist.contains(program) => None,
                    Some(program) => Some(format!(
                        "program {program} is outside the program-invoke allowlist"
                    )),
                    None => Some("program.invoke names no program".to_owned()),
                }
            }
            _ => None,
        }
    }

    /// The object-class leg of the ceiling check: the target's class
    /// must be established (observed on a frame, or carried by a
    /// `class:id` entity reference) and named in the ceiling.
    fn inspect_class_refusal(&self, arguments: &serde_json::Value) -> Option<String> {
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
            Some(class) => Some(format!(
                "object class {class} is outside the capability ceiling"
            )),
            None => Some("the object class of the inspect target is not established".to_owned()),
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
                self.audit
                    .append(&self.grant, name, object, charge, AuditOutcome::Executed);
                PolicyOutcome::Executed(action)
            }
            Err(error) => {
                let detail = format!("{error:?}");
                self.audit.append(
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
fn audit_object(name: &str, arguments: &serde_json::Value) -> Option<String> {
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
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::mcp_bridge::v1_tool_definitions;
    use serde_json::json;

    fn host_key() -> SigningKey {
        SigningKey::from_bytes(&[7; 32])
    }

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
            quota: Quota {
                frames: 8,
                tool_calls: 32,
                program_invocations: 1,
            },
        }
    }

    fn session(grant: RoleGrant) -> PolicySession {
        PolicySession::new(
            grant,
            BridgeSession::start(&v1_tool_definitions()).unwrap(),
            host_key(),
        )
    }

    fn joined(grant: RoleGrant) -> PolicySession {
        let mut session = session(grant);
        let outcome = session.act("world.join", &json!({ "capability": "cap-1" }));
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
            let outcome = scoped_session.act("chat.post", &json!({ "text": "hello" }));
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
        let outcome = elsewhere_session.act("world.join", &json!({ "capability": "cap-1" }));
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
    fn quota_exhaustion_parks_the_agent_visibly() {
        let mut grant = grant();
        grant.quota.frames = 1; // the join spends the only frame
        let mut session = joined(grant);
        let outcome = session.act("chat.post", &json!({ "text": "hello" }));
        let PolicyOutcome::Refused(refusal) = outcome else {
            panic!("quota refusal expected, got {outcome:?}");
        };
        assert_eq!(refusal.kind, RefusalKind::Quota);
        assert_eq!(refusal.park_event.as_deref(), Some(QUOTA_PARK_EVENT));
        assert!(session.is_parked());
        assert!(matches!(
            last_outcome(&session),
            AuditOutcome::RefusedQuota { .. }
        ));
        // Parked actions are refused without a charge...
        let spend_before = session.spend();
        let outcome = session.act("screen.read", &json!({}));
        assert!(matches!(
            outcome,
            PolicyOutcome::Refused(PolicyRefusal {
                kind: RefusalKind::Parked,
                ..
            })
        ));
        assert_eq!(session.spend(), spend_before);
        // ...but the agent can always leave.
        assert_eq!(
            session.act("world.leave", &json!({})),
            PolicyOutcome::Executed(ToolAction::Close)
        );
        let summary = session.owner_summary();
        assert!(summary.parked);
        assert!(summary.render().contains("parked, quota exhausted"));
    }

    #[test]
    fn audit_chain_verifies_end_to_end_and_tampering_fails() {
        let mut session = joined(grant());
        session.act(
            "screen.subscribe",
            &json!({ "event_classes": ["chat_posted"] }),
        );
        session.act("chat.post", &json!({ "text": "hello" }));
        session.act("host.exec", &json!({})); // ceiling refusal
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
        session.act("chat.post", &json!({ "text": "hello" }));
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
        ); // 1 frame + 1 tool call
        session.act("screen.read", &json!({})); // 1 tool call
        session.act("chat.post", &json!({ "text": "hi" })); // 1 frame + 1 tool call
        session.act("host.exec", &json!({})); // ceiling refusal, no charge
        let summary = session.owner_summary();
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
            session.act("object.inspect", &json!({ "entity_ref": "cell-4" })),
            PolicyOutcome::Executed(ToolAction::Local(_))
        ));
        // An observed class outside the ceiling: refused.
        assert!(matches!(
            session.act("object.inspect", &json!({ "entity_ref": "safe-1" })),
            PolicyOutcome::Refused(PolicyRefusal {
                kind: RefusalKind::Ceiling,
                ..
            })
        ));
        // A class:id reference carries its class without a frame.
        assert!(matches!(
            session.act("object.inspect", &json!({ "entity_ref": "vault:9" })),
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
            session.act("program.invoke", &json!({ "program": "wipe" })),
            PolicyOutcome::Refused(PolicyRefusal {
                kind: RefusalKind::Ceiling,
                ..
            })
        ));
        assert_eq!(session.spend().program_invocations, 0);
        // Inside the allowlist: admitted and charged; the v1 bridge
        // has no such tool yet (spike step 5), so the attempt ends
        // as an audited bridge refusal with the charge standing.
        assert!(matches!(
            session.act("program.invoke", &json!({ "program": "scorekeeper" })),
            PolicyOutcome::Refused(PolicyRefusal {
                kind: RefusalKind::Bridge,
                ..
            })
        ));
        assert_eq!(session.spend().program_invocations, 1);
        assert!(matches!(
            last_outcome(&session),
            AuditOutcome::RefusedBridge { .. }
        ));
        assert_eq!(
            session.audit_events().last().unwrap().object.as_deref(),
            Some("scorekeeper")
        );
    }

    #[test]
    fn the_agent_cannot_mutate_the_log_or_its_quota() {
        let mut session = joined(grant());
        session.act("chat.post", &json!({ "text": "hello" }));
        // The spend comes out by value: scribbling on the copy does
        // not reach the session's ledger.
        let mut copy = session.spend();
        copy.frames = 0;
        copy.tool_calls = 0;
        assert_eq!(copy.tool_calls, 0); // the copy really was scribbled on
        assert_eq!(session.spend().tool_calls, 2);
        // The log comes out as a shared slice; the chain still
        // verifies after every read.
        assert_eq!(session.audit_events().len(), 2);
        session.verify_audit().unwrap();
    }
}
