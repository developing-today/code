//! Versioned configuration contract of the `id` crate: the canonical V2
//! `ConfigDocument`, its hard invariants, and the deterministic resolver.
//!
//! Slice 9111-01 (#10178) of the ID/Atlas V2 program. Design basis:
//! `doc/2026-11-13T03-40-00Z_design_id-atlas-summit/crosscut/versioned-design.md`,
//! section "Configuration data model and precedence" (statements S7.1,
//! S7.10–S7.13). The JSON Schema form of this contract lives at
//! `pkgs/id/config/id-config-v1.schema.json`; the two are kept in lockstep by
//! `pkgs/id/tests/config_invariants.rs`.
//!
//! The contract in one paragraph: mechanism is primitive, behavior is
//! config. The fixed admission gates (signature verification, validation,
//! replay protection, authorization, tenant scope) are mechanism — a config
//! document that tries to disable one is rejected, never honored. Resource
//! and authority ceilings are hard: scopes may narrow them, never widen
//! them, and any behavioral value that would exceed an applicable ceiling
//! rejects the document. Behavioral values (mode defaults, document values,
//! scoped overrides) may replace each other in either direction within the
//! ceilings. Validation is fail-closed: an unknown field, an unknown schema
//! version, or an unnamespaced extension key rejects the document;
//! namespaced extensions are retained in the resolved output but are never
//! executed by this crate. Resolution is deterministic: the same document
//! always resolves to the same [`ResolvedConfig`], regardless of the order
//! overrides or allowlist entries were written in.

use std::collections::BTreeMap;
use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

/// The only `ConfigDocument` schema version this crate accepts. The schema
/// file is `pkgs/id/config/id-config-v1.schema.json`; a document naming any
/// other version is rejected ([`ConfigError::UnsupportedSchemaVersion`])
/// rather than silently reinterpreted.
pub const CONFIG_SCHEMA_VERSION: u32 = 1;

/// The fixed admission gates, by canonical ID. These are mechanism, not
/// behavior: they are compiled into the admission pipeline (design decision
/// summary, item 3) and no configuration plane — document, mode, tenant,
/// peer, link, storage, or operation scope — may disable them. Consumed by
/// #9141's peer-trust contract: the signed-data acceptance gate is not
/// configurable.
pub const FIXED_GATE_IDS: [&str; 5] = [
    "signature-verification",
    "validation",
    "replay-protection",
    "authorization",
    "tenant-scope",
];

/// Whether a configuration field is mechanism (a primitive invariant that
/// configuration may not change) or behavior (a value configuration planes
/// may replace within the ceilings). The mechanism-versus-behavior
/// classification is part of the 9111-01 contract: a field classified
/// [`FieldClass::Mechanism`] must never become settable behavior without a
/// new major schema version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FieldClass {
    /// Primitive invariant: not replaceable by any configuration plane.
    Mechanism,
    /// Behavioral value: replaceable by later authorized scopes, within
    /// every applicable ceiling.
    Behavior,
}

/// Classify a dotted `ConfigDocument` field path. Returns `None` for a path
/// this schema does not define — which is also how an unknown field is
/// treated at parse time (rejected).
#[must_use]
pub fn field_class(path: &str) -> Option<FieldClass> {
    let head = path.split('.').next().unwrap_or(path);
    match head {
        // Identity, versioning, the protocol allowlist, the hard ceilings
        // and the fixed gates are the invariant skeleton of the document.
        "schema_version" | "config_id" | "revision" | "protocol_versions" | "ceilings"
        | "gates" => Some(FieldClass::Mechanism),
        // Mode bundles, behavioral values, scoped overrides and retained
        // extensions are the configurable surface.
        "mode" | "behavior" | "overrides" | "extensions" => Some(FieldClass::Behavior),
        _ => None,
    }
}

/// A named mode: a coherent behavior bundle (design: "Mode is a coherent
/// behavior bundle … and never a code fork. A mode cannot confer identity
/// or privilege."). The mode contributes behavioral defaults only; it
/// cannot touch ceilings or gates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Mode {
    /// One node, no federation: minimal fan-out by default.
    SingleNode,
    /// Hosted multi-tenant deployment.
    HostedTenant,
    /// Federation relay between peers.
    FederatedRelay,
}

impl Mode {
    /// The behavioral defaults this mode bundle contributes, before the
    /// document's own behavioral values and scoped overrides replace them.
    #[must_use]
    pub const fn defaults(self) -> ResolvedBehavior {
        match self {
            Self::SingleNode => ResolvedBehavior {
                max_hops: 1,
                sync_scope: SyncScope::Metadata,
                tier: Tier::Cold,
            },
            Self::HostedTenant => ResolvedBehavior {
                max_hops: 3,
                sync_scope: SyncScope::Full,
                tier: Tier::Warm,
            },
            Self::FederatedRelay => ResolvedBehavior {
                max_hops: 5,
                sync_scope: SyncScope::Full,
                tier: Tier::Hot,
            },
        }
    }

    /// The canonical string form, used in provenance records.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SingleNode => "single-node",
            Self::HostedTenant => "hosted-tenant",
            Self::FederatedRelay => "federated-relay",
        }
    }
}

/// How much data a sync relationship carries. A behavioral value: it may be
/// replaced in either direction by later scopes, subject to the tenant's
/// authority ceiling (which lives outside this minimal schema, with its
/// domain owner).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SyncScope {
    /// Metadata only.
    Metadata,
    /// Full content.
    Full,
}

/// Placement/retention tier label. A behavioral value; a tier label never
/// authorizes access (design: "Tier labels do not authorize access").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Tier {
    /// Cold storage tier.
    Cold,
    /// Warm storage tier.
    Warm,
    /// Hot storage tier.
    Hot,
}

/// Hard resource ceilings. Ceilings are mechanism: the values in the
/// document are the widest any scope may ever resolve to, and a scoped
/// override may only narrow them (see [`CeilingOverride`]). A ceiling of
/// zero is rejected — a ceiling that forbids the operation outright is a
/// policy decision for the owning plane, not a config value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ceilings {
    /// Maximum relay hops for any operation.
    pub max_hops: u32,
    /// Maximum bytes for any single operation payload.
    pub max_bytes: u64,
    /// Maximum wall-clock milliseconds for any single operation.
    pub max_time_ms: u64,
    /// Maximum fan-out (delivery targets) for any single operation.
    pub max_fan_out: u32,
}

/// A scoped narrowing of the hard ceilings. Every field is optional; a
/// present field must be less than or equal to the document ceiling for
/// that field — an attempt to widen is [`ConfigError::CeilingWidened`],
/// never a silent clamp.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CeilingOverride {
    /// Narrowed maximum relay hops, if this scope narrows it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_hops: Option<u32>,
    /// Narrowed maximum payload bytes, if this scope narrows it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_bytes: Option<u64>,
    /// Narrowed maximum operation milliseconds, if this scope narrows it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_time_ms: Option<u64>,
    /// Narrowed maximum fan-out, if this scope narrows it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_fan_out: Option<u32>,
}

const fn narrow_u32(current: u32, narrowing: Option<u32>) -> u32 {
    match narrowing {
        Some(value) if value < current => value,
        _ => current,
    }
}

const fn narrow_u64(current: u64, narrowing: Option<u64>) -> u64 {
    match narrowing {
        Some(value) if value < current => value,
        _ => current,
    }
}

impl Ceilings {
    /// Apply one scope's narrowing. Validation has already established the
    /// override does not widen, so this is a plain per-field minimum.
    #[must_use]
    pub const fn narrowed_by(self, narrowing: &CeilingOverride) -> Self {
        Self {
            max_hops: narrow_u32(self.max_hops, narrowing.max_hops),
            max_bytes: narrow_u64(self.max_bytes, narrowing.max_bytes),
            max_time_ms: narrow_u64(self.max_time_ms, narrowing.max_time_ms),
            max_fan_out: narrow_u32(self.max_fan_out, narrowing.max_fan_out),
        }
    }
}

const fn default_true() -> bool {
    true
}

/// The fixed admission gates. Every field defaults to `true` and validation
/// rejects a document that sets any of them to `false`
/// ([`ConfigError::FixedGateDisabled`]): the fields exist in the schema so
/// that a downgrade attempt is *visible and rejected*, not silently
/// ignored. The JSON Schema states the same rule as `"const": true`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FixedGates {
    /// Verify signature and signer binding before admission.
    #[serde(default = "default_true")]
    pub signature_verification: bool,
    /// Validate object semantics before admission.
    #[serde(default = "default_true")]
    pub validation: bool,
    /// Check replay/idempotency before admission.
    #[serde(default = "default_true")]
    pub replay_protection: bool,
    /// Authorize signer/action before admission.
    #[serde(default = "default_true")]
    pub authorization: bool,
    /// Check tenant/scope before admission.
    #[serde(default = "default_true")]
    pub tenant_scope: bool,
}

impl Default for FixedGates {
    fn default() -> Self {
        Self {
            signature_verification: true,
            validation: true,
            replay_protection: true,
            authorization: true,
            tenant_scope: true,
        }
    }
}

impl FixedGates {
    /// The name of the first disabled gate, if any.
    fn first_disabled(&self) -> Option<&'static str> {
        let gates = [
            ("signature-verification", self.signature_verification),
            ("validation", self.validation),
            ("replay-protection", self.replay_protection),
            ("authorization", self.authorization),
            ("tenant-scope", self.tenant_scope),
        ];
        gates
            .into_iter()
            .find_map(|(id, enabled)| if enabled { None } else { Some(id) })
    }
}

/// Behavioral values, in partial (override) form: a `None` field leaves the
/// value from the previous resolution plane in place, a `Some` field
/// replaces it. Replacement may go in either direction; the resolved result
/// must still fit within every applicable ceiling.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Behavior {
    /// Behavioral maximum relay hops (bounded by the `max_hops` ceiling).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_hops: Option<u32>,
    /// Behavioral sync scope.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sync_scope: Option<SyncScope>,
    /// Behavioral placement tier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tier: Option<Tier>,
}

/// Behavioral values in fully resolved form: every field is decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedBehavior {
    /// Resolved maximum relay hops.
    pub max_hops: u32,
    /// Resolved sync scope.
    pub sync_scope: SyncScope,
    /// Resolved placement tier.
    pub tier: Tier,
}

impl ResolvedBehavior {
    fn apply(&mut self, behavior: &Behavior) {
        if let Some(max_hops) = behavior.max_hops {
            self.max_hops = max_hops;
        }
        if let Some(sync_scope) = behavior.sync_scope {
            self.sync_scope = sync_scope;
        }
        if let Some(tier) = behavior.tier {
            self.tier = tier;
        }
    }
}

/// One scoped override: behavioral replacements and/or a ceiling narrowing
/// that apply to a named scope (a tenant, peer, link, or storage binding —
/// the axes stay distinct in their owning planes; this minimal schema only
/// names the scope). Two overrides for the same scope in one document are
/// rejected ([`ConfigError::DuplicateScope`]) so resolution never depends
/// on which one a reader happens to apply last.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScopedOverride {
    /// The scope this override applies to. Must be non-empty.
    pub scope: String,
    /// Ceiling narrowing for this scope (never a widening).
    #[serde(default)]
    pub ceilings: CeilingOverride,
    /// Behavioral replacements for this scope.
    #[serde(default)]
    pub behavior: Behavior,
}

/// The canonical versioned configuration document, schema version 1.
/// Unknown fields are rejected at parse time (`deny_unknown_fields` on
/// every struct in this crate): per the design, "unknown mandatory
/// field/version rejects the document; namespaced optional extensions may
/// be retained but are not executed unless understood and authorized."
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigDocument {
    /// Schema version. Must equal [`CONFIG_SCHEMA_VERSION`].
    pub schema_version: u32,
    /// Stable configuration identity. Must be non-empty.
    pub config_id: String,
    /// Monotonic revision of this configuration.
    pub revision: u64,
    /// Named mode whose behavioral defaults seed resolution.
    pub mode: Mode,
    /// Protocol-version allowlist. Must be non-empty and duplicate-free;
    /// the resolved form is sorted, so authoring order carries no meaning.
    pub protocol_versions: Vec<String>,
    /// Hard, non-overridable resource ceilings.
    pub ceilings: Ceilings,
    /// The fixed admission gates. Any attempt to disable one rejects the
    /// document; see [`FixedGates`].
    #[serde(default)]
    pub gates: FixedGates,
    /// The document's own behavioral values, replacing mode defaults.
    #[serde(default)]
    pub behavior: Behavior,
    /// Scoped overrides, applied in scope-name order during resolution.
    #[serde(default)]
    pub overrides: Vec<ScopedOverride>,
    /// Namespaced optional extensions. Keys must be valid namespaces per
    /// the canonical schema rule: at least two non-empty, non-whitespace
    /// segments joined by single `.` or `/` separators (e.g.
    /// `example.com/experiment`); values are retained verbatim in the
    /// resolved output and never executed here.
    #[serde(default)]
    pub extensions: BTreeMap<String, serde_json::Value>,
}

/// The deterministic result of resolving a valid [`ConfigDocument`]: the
/// canonical effective configuration. Provenance is recorded separately
/// from the values, per the design ("Canonical effective config is
/// deterministic; provenance is recorded separately").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedConfig {
    /// Schema version the document was resolved under.
    pub schema_version: u32,
    /// Configuration identity, carried from the document.
    pub config_id: String,
    /// Configuration revision, carried from the document.
    pub revision: u64,
    /// Mode the document named.
    pub mode: Mode,
    /// Protocol-version allowlist, sorted into canonical order.
    pub protocol_versions: Vec<String>,
    /// Effective ceilings: the document ceilings narrowed by every scope.
    pub ceilings: Ceilings,
    /// The fixed gates, always fully enabled in a resolved config.
    pub gates: FixedGates,
    /// Fully resolved behavioral values.
    pub behavior: ResolvedBehavior,
    /// Which plane supplied each resolved behavioral field: `mode:<mode>`,
    /// `document`, or `override:<scope>`.
    pub provenance: BTreeMap<String, String>,
    /// Namespaced extensions, retained verbatim, never executed.
    pub extensions: BTreeMap<String, serde_json::Value>,
}

impl ResolvedConfig {
    /// The canonical JSON encoding of this resolved config. Deterministic:
    /// struct fields serialize in declaration order, maps are `BTreeMap`s,
    /// and the allowlist is sorted during resolution, so equal resolved
    /// configs always produce byte-identical output.
    ///
    /// # Errors
    /// Returns [`ConfigError::Parse`] if serialization fails (not expected
    /// for these types, but never panics).
    pub fn canonical_json(&self) -> Result<String, ConfigError> {
        Ok(serde_json::to_string(self)?)
    }
}

/// Everything that can make a config document unacceptable. Validation is
/// fail-closed: each variant is a rejection, never a warning.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// The document did not parse (malformed JSON, a wrong-typed field, or
    /// an unknown field — `serde`'s `deny_unknown_fields` message names it).
    #[error("config document does not parse: {0}")]
    Parse(#[from] serde_json::Error),
    /// The document names a schema version this crate does not accept.
    #[error("unsupported config schema version {found}; this build accepts {supported}")]
    UnsupportedSchemaVersion {
        /// The version found in the document.
        found: u32,
        /// The version this crate accepts.
        supported: u32,
    },
    /// The config ID is empty.
    #[error("config_id must not be empty")]
    EmptyConfigId,
    /// The protocol-version allowlist is empty.
    #[error("protocol_versions allowlist must not be empty")]
    EmptyProtocolAllowlist,
    /// The protocol-version allowlist names a version twice (or names an
    /// empty version).
    #[error("protocol_versions contains a duplicate or empty entry: {0:?}")]
    DuplicateProtocolVersion(String),
    /// A ceiling field is zero.
    #[error("ceiling {0} must be at least 1")]
    ZeroCeiling(&'static str),
    /// The document tried to disable a fixed admission gate.
    #[error("fixed gate {0} cannot be disabled by configuration")]
    FixedGateDisabled(&'static str),
    /// A scoped override tried to raise a hard ceiling.
    #[error(
        "scope {scope} attempts to widen ceiling {field} to {attempted}, above the hard ceiling {ceiling}"
    )]
    CeilingWidened {
        /// The scope that attempted the widening.
        scope: String,
        /// The ceiling field it attempted to widen.
        field: &'static str,
        /// The attempted (wider) value.
        attempted: u64,
        /// The hard ceiling it exceeds.
        ceiling: u64,
    },
    /// A behavioral value exceeds the ceiling that applies to it.
    #[error("behavioral {field} value {value} exceeds the applicable ceiling {ceiling}")]
    BehaviorExceedsCeiling {
        /// The behavioral field.
        field: &'static str,
        /// The offending value.
        value: u64,
        /// The ceiling it exceeds.
        ceiling: u64,
    },
    /// Two overrides name the same scope.
    #[error("duplicate override for scope {0:?}")]
    DuplicateScope(String),
    /// An override names an empty scope.
    #[error("override scope must not be empty")]
    EmptyScope,
    /// An extension key is not a valid namespace.
    #[error(
        "extension key {0:?} is not a valid namespace (expected non-empty, non-whitespace segments joined by '.' or '/' separators, e.g. 'example.com/experiment')"
    )]
    UnnamespacedExtension(String),
}

/// Whether an extension key is a valid namespace per the canonical schema
/// rule (`^[^./\\s]+([./][^./\\s]+)+$` in id-config-v1.schema.json): at
/// least two segments of non-separator, non-whitespace characters joined by
/// single `.` or `/` separators. Separator-only keys (`.`), empty segments
/// (`a..b`), leading or trailing separators, and whitespace anywhere in the
/// key are all malformed and reject.
fn is_namespaced_extension_key(key: &str) -> bool {
    let mut segments = 0usize;
    for segment in key.split(['.', '/']) {
        if segment.is_empty() || segment.chars().any(char::is_whitespace) {
            return false;
        }
        segments += 1;
    }
    segments >= 2
}

fn check_ceiling_widening(
    scope: &str,
    field: &'static str,
    attempted: Option<u64>,
    ceiling: u64,
) -> Result<(), ConfigError> {
    if let Some(value) = attempted {
        if value > ceiling {
            return Err(ConfigError::CeilingWidened {
                scope: scope.to_owned(),
                field,
                attempted: value,
                ceiling,
            });
        }
    }
    Ok(())
}

impl ConfigDocument {
    /// Parse a config document from JSON. Unknown fields, wrong types, and
    /// malformed JSON are rejected here; the hard invariants are checked by
    /// [`ConfigDocument::validate`] (which [`ConfigDocument::resolve`]
    /// always runs first).
    ///
    /// # Errors
    /// Returns [`ConfigError::Parse`] for any parse failure.
    pub fn parse(input: &str) -> Result<Self, ConfigError> {
        Ok(serde_json::from_str(input)?)
    }

    /// Check every hard invariant of this document without resolving it.
    ///
    /// # Errors
    /// Returns the first [`ConfigError`] an invariant rejects with.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.schema_version != CONFIG_SCHEMA_VERSION {
            return Err(ConfigError::UnsupportedSchemaVersion {
                found: self.schema_version,
                supported: CONFIG_SCHEMA_VERSION,
            });
        }
        if self.config_id.trim().is_empty() {
            return Err(ConfigError::EmptyConfigId);
        }
        if self.protocol_versions.is_empty() {
            return Err(ConfigError::EmptyProtocolAllowlist);
        }
        let mut seen = BTreeSet::new();
        for version in &self.protocol_versions {
            if version.trim().is_empty() || !seen.insert(version) {
                return Err(ConfigError::DuplicateProtocolVersion(version.clone()));
            }
        }
        let ceilings = [
            ("max_hops", u64::from(self.ceilings.max_hops)),
            ("max_bytes", self.ceilings.max_bytes),
            ("max_time_ms", self.ceilings.max_time_ms),
            ("max_fan_out", u64::from(self.ceilings.max_fan_out)),
        ];
        for (field, value) in ceilings {
            if value == 0 {
                return Err(ConfigError::ZeroCeiling(field));
            }
        }
        if let Some(gate) = self.gates.first_disabled() {
            return Err(ConfigError::FixedGateDisabled(gate));
        }
        for key in self.extensions.keys() {
            if !is_namespaced_extension_key(key) {
                return Err(ConfigError::UnnamespacedExtension(key.clone()));
            }
        }
        if let Some(max_hops) = self.behavior.max_hops {
            if max_hops > self.ceilings.max_hops {
                return Err(ConfigError::BehaviorExceedsCeiling {
                    field: "max_hops",
                    value: u64::from(max_hops),
                    ceiling: u64::from(self.ceilings.max_hops),
                });
            }
        }
        let mut scopes = BTreeSet::new();
        for scoped in &self.overrides {
            if scoped.scope.trim().is_empty() {
                return Err(ConfigError::EmptyScope);
            }
            if !scopes.insert(&scoped.scope) {
                return Err(ConfigError::DuplicateScope(scoped.scope.clone()));
            }
            check_ceiling_widening(
                &scoped.scope,
                "max_hops",
                scoped.ceilings.max_hops.map(u64::from),
                u64::from(self.ceilings.max_hops),
            )?;
            check_ceiling_widening(
                &scoped.scope,
                "max_bytes",
                scoped.ceilings.max_bytes,
                self.ceilings.max_bytes,
            )?;
            check_ceiling_widening(
                &scoped.scope,
                "max_time_ms",
                scoped.ceilings.max_time_ms,
                self.ceilings.max_time_ms,
            )?;
            check_ceiling_widening(
                &scoped.scope,
                "max_fan_out",
                scoped.ceilings.max_fan_out.map(u64::from),
                u64::from(self.ceilings.max_fan_out),
            )?;
            // A scope's behavioral max_hops must fit within the ceiling as
            // narrowed by that same scope — the tightest applicable bound.
            let narrowed = self.ceilings.narrowed_by(&scoped.ceilings);
            if let Some(max_hops) = scoped.behavior.max_hops {
                if max_hops > narrowed.max_hops {
                    return Err(ConfigError::BehaviorExceedsCeiling {
                        field: "max_hops",
                        value: u64::from(max_hops),
                        ceiling: u64::from(narrowed.max_hops),
                    });
                }
            }
        }
        Ok(())
    }

    /// Validate and resolve this document into the canonical effective
    /// configuration. Resolution order (design precedence): mode defaults,
    /// then the document's behavioral values, then scoped overrides in
    /// scope-name order; ceilings narrow monotonically across all scopes.
    /// Deterministic: authoring order of `overrides` and
    /// `protocol_versions` does not affect the result.
    ///
    /// # Errors
    /// Returns the [`ConfigError`] validation rejects with, if any.
    pub fn resolve(&self) -> Result<ResolvedConfig, ConfigError> {
        self.validate()?;

        let mut ordered: Vec<&ScopedOverride> = self.overrides.iter().collect();
        ordered.sort_by(|a, b| a.scope.cmp(&b.scope));

        let mut ceilings = self.ceilings;
        for scoped in &ordered {
            ceilings = ceilings.narrowed_by(&scoped.ceilings);
        }

        let mut behavior = self.mode.defaults();
        let mut provenance: BTreeMap<String, String> = BTreeMap::new();
        for field in ["max_hops", "sync_scope", "tier"] {
            provenance.insert(field.to_owned(), format!("mode:{}", self.mode.as_str()));
        }
        apply_tracked(&mut behavior, &self.behavior, "document", &mut provenance);
        for scoped in &ordered {
            apply_tracked(
                &mut behavior,
                &scoped.behavior,
                &format!("override:{}", scoped.scope),
                &mut provenance,
            );
        }

        if behavior.max_hops > ceilings.max_hops {
            return Err(ConfigError::BehaviorExceedsCeiling {
                field: "max_hops",
                value: u64::from(behavior.max_hops),
                ceiling: u64::from(ceilings.max_hops),
            });
        }

        let mut protocol_versions = self.protocol_versions.clone();
        protocol_versions.sort();

        Ok(ResolvedConfig {
            schema_version: self.schema_version,
            config_id: self.config_id.clone(),
            revision: self.revision,
            mode: self.mode,
            protocol_versions,
            ceilings,
            gates: self.gates,
            behavior,
            provenance,
            extensions: self.extensions.clone(),
        })
    }
}

fn apply_tracked(
    behavior: &mut ResolvedBehavior,
    patch: &Behavior,
    source: &str,
    provenance: &mut BTreeMap<String, String>,
) {
    if patch.max_hops.is_some() {
        provenance.insert("max_hops".to_owned(), source.to_owned());
    }
    if patch.sync_scope.is_some() {
        provenance.insert("sync_scope".to_owned(), source.to_owned());
    }
    if patch.tier.is_some() {
        provenance.insert("tier".to_owned(), source.to_owned());
    }
    behavior.apply(patch);
}
