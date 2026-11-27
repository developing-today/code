//! Typed dependency edges — DEP-02 (developing-today/code#9503; slice
//! of #9208, program #9196).
//!
//! The design record is
//! `doc/2026-11-26T03-07-38Z_design_dep02_typed_edge_model_json_error_contract/`.
//! This module is the pure half of the dependency model: how the seven
//! relation spellings become one canonical edge (D1), how the two ends
//! of an edge are named (D2), the ref input forms (D3), the edge record
//! and its before/after revision (D4, D5), how an edge reads from either
//! end (D6), the one error vocabulary (D7), the read-view vocabulary
//! (D8) and the legacy `blocked_deps` mapping (D9).
//!
//! Nothing here does I/O, touches storage or reads a clock. Resolving a
//! ref against GitHub or an authority is the caller's job; this module
//! takes the resolved facts as inputs. DEP-03 (storage, cycle checks,
//! readiness), DEP-04 (GitHub projection, hierarchy) and CLI-13 (the
//! commands) consume it.

use std::fmt;

use serde_json::{Map, Value};

use super::contracts::{
    ContractError, Effect, Outcome, ResultError, WorkRef, is_canonical_decimal,
};

/// Schema id of the edge record (D4).
pub const EDGE_SCHEMA: &str = "fleet.dependency_edge/v1";
/// Schema id of one edge revision (D5).
pub const EDGE_REVISION_SCHEMA: &str = "fleet.dependency_edge_revision/v1";
/// Schema id of the dependency list read view (D8).
pub const DEPENDENCY_LIST_SCHEMA: &str = "fleet.dependency_list/v1";
/// Schema id of the blockers read view (D8).
pub const DEPENDENCY_BLOCKERS_SCHEMA: &str = "fleet.dependency_blockers/v1";
/// The authority whose work items key as `task:<work_id>`, matching
/// the legacy `dep_key` spelling (D2).
pub const LOCAL_AUTHORITY_ID: &str = "local";

// ---------------------------------------------------------------------------
// D7. The error vocabulary
// ---------------------------------------------------------------------------

/// Every refusal of the dependency model, with one stable code each
/// (D7). Codes raised by DEP-03 and DEP-04 are registered here too, so
/// there is a single list. Adding a code is compatible; renaming one
/// is a breaking change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelationError {
    /// The spelling is not one of the seven (D1).
    UnknownRelation {
        /// The spelling that was presented.
        spelling: String,
    },
    /// Unknown field or enum value, or a malformed shape.
    InvalidSchema {
        /// What was wrong, naming the field.
        detail: String,
    },
    /// Both ends of the edge have the same key.
    SelfEdge {
        /// The shared key.
        key: String,
    },
    /// The canonical target is external, or neither end is Fleet work.
    TargetNotFleetWork {
        /// What was refused.
        detail: String,
    },
    /// `hard` was asked on a relation other than `blocks`.
    RelationNotGating {
        /// The relation that was asked to be hard.
        relation: Relation,
    },
    /// A shorthand or `#N` that does not bind to exactly one end.
    AmbiguousRef {
        /// What was ambiguous, naming the qualified form.
        detail: String,
    },
    /// An explicit edge whose external end has no stable provider id.
    UnresolvedRef {
        /// Which end was unresolved.
        detail: String,
    },
    /// A status move outside D5's table, or a revision that changes
    /// the edge's identity.
    InvalidTransition {
        /// What move was refused.
        detail: String,
    },
    /// An active edge with the same relation and keys exists (DEP-03).
    DuplicateEdge {
        /// The existing edge.
        detail: String,
    },
    /// Same `duplicates` pair, different `role` (DEP-04).
    RelationRoleConflict {
        /// The conflicting pair.
        detail: String,
    },
    /// The add would close a `blocks` cycle (DEP-03).
    DependencyCycle {
        /// The cycle.
        detail: String,
    },
    /// The add would make an item its own ancestor (DEP-04).
    HierarchyCycle {
        /// The cycle.
        detail: String,
    },
    /// The add would close a `supersedes` cycle (DEP-04).
    SupersedesCycle {
        /// The cycle.
        detail: String,
    },
    /// The check could not finish within its bound; refused, not
    /// assumed (DEP-03).
    GraphEvaluationIncomplete {
        /// What bound was hit.
        detail: String,
    },
    /// An active cycle was found on read; a repair signal (DEP-03).
    UnexpectedActiveCycle {
        /// The cycle.
        detail: String,
    },
    /// `--expected-version` is stale (DEP-03). Wire code
    /// `VERSION_CONFLICT`.
    VersionConflict {
        /// The expected and current versions.
        detail: String,
    },
}

impl RelationError {
    /// The stable wire code for this refusal.
    pub fn code(&self) -> &'static str {
        match self {
            RelationError::UnknownRelation { .. } => "unknown_relation",
            RelationError::InvalidSchema { .. } => "invalid_schema",
            RelationError::SelfEdge { .. } => "self_edge",
            RelationError::TargetNotFleetWork { .. } => "target_not_fleet_work",
            RelationError::RelationNotGating { .. } => "relation_not_gating",
            RelationError::AmbiguousRef { .. } => "ambiguous_ref",
            RelationError::UnresolvedRef { .. } => "unresolved_ref",
            RelationError::InvalidTransition { .. } => "invalid_transition",
            RelationError::DuplicateEdge { .. } => "duplicate_edge",
            RelationError::RelationRoleConflict { .. } => "relation_role_conflict",
            RelationError::DependencyCycle { .. } => "dependency_cycle",
            RelationError::HierarchyCycle { .. } => "hierarchy_cycle",
            RelationError::SupersedesCycle { .. } => "supersedes_cycle",
            RelationError::GraphEvaluationIncomplete { .. } => "graph_evaluation_incomplete",
            RelationError::UnexpectedActiveCycle { .. } => "unexpected_active_cycle",
            RelationError::VersionConflict { .. } => "VERSION_CONFLICT",
        }
    }

    /// The `fleet.result/v1` outcome of this refusal: `conflict` for a
    /// stale version or an existing duplicate edge, `rejected`
    /// otherwise (D7).
    pub fn outcome(&self) -> Outcome {
        match self {
            RelationError::VersionConflict { .. } | RelationError::DuplicateEdge { .. } => {
                Outcome::Conflict
            }
            _ => Outcome::Rejected,
        }
    }
}

impl fmt::Display for RelationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let code = self.code();
        match self {
            RelationError::UnknownRelation { spelling } => {
                write!(
                    f,
                    "{code}: {spelling:?} is not one of blocks, blocked_by, parent_of, child_of, supersedes, related_to, duplicates"
                )
            }
            RelationError::SelfEdge { key } => write!(f, "{code}: both ends are {key}"),
            RelationError::RelationNotGating { relation } => write!(
                f,
                "{code}: only blocks can be hard; {} is always soft",
                relation.as_str()
            ),
            RelationError::InvalidSchema { detail }
            | RelationError::TargetNotFleetWork { detail }
            | RelationError::AmbiguousRef { detail }
            | RelationError::UnresolvedRef { detail }
            | RelationError::InvalidTransition { detail }
            | RelationError::DuplicateEdge { detail }
            | RelationError::RelationRoleConflict { detail }
            | RelationError::DependencyCycle { detail }
            | RelationError::HierarchyCycle { detail }
            | RelationError::SupersedesCycle { detail }
            | RelationError::GraphEvaluationIncomplete { detail }
            | RelationError::UnexpectedActiveCycle { detail }
            | RelationError::VersionConflict { detail } => write!(f, "{code}: {detail}"),
        }
    }
}

impl std::error::Error for RelationError {}

impl From<&RelationError> for ResultError {
    fn from(error: &RelationError) -> Self {
        ResultError {
            code: error.code().to_string(),
            message: error.to_string(),
        }
    }
}

impl From<ContractError> for RelationError {
    fn from(error: ContractError) -> Self {
        RelationError::InvalidSchema {
            detail: error.to_string(),
        }
    }
}

/// The effects a `conflict` refusal carries, since `fleet.result/v1`
/// has no payload field: the current edge's status and revision as
/// observed values (D7).
pub fn conflict_effects(edge_id: &str, status: EdgeStatus, revision: &str) -> Vec<Effect> {
    vec![
        Effect {
            field: format!("edge:{edge_id}.status"),
            requested: None,
            observed: Some(status.as_str().to_string()),
            verified: None,
        },
        Effect {
            field: format!("edge:{edge_id}.revision"),
            requested: None,
            observed: Some(revision.to_string()),
            verified: None,
        },
    ]
}

fn schema(detail: impl Into<String>) -> RelationError {
    RelationError::InvalidSchema {
        detail: detail.into(),
    }
}

// ---------------------------------------------------------------------------
// Closed enums
// ---------------------------------------------------------------------------

macro_rules! closed_enum {
    (
        $(#[$meta:meta])*
        $name:ident {
            $( $(#[$vmeta:meta])* $variant:ident => $wire:literal ),+ $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum $name {
            $( $(#[$vmeta])* $variant ),+
        }

        impl $name {
            /// Every value, in declaration order.
            pub const ALL: &'static [$name] = &[$($name::$variant),+];

            /// The wire spelling.
            pub fn as_str(&self) -> &'static str {
                match self {
                    $( $name::$variant => $wire ),+
                }
            }

            /// Parse the wire spelling of `field`. An unknown value is
            /// `invalid_schema` naming the field; it is never mapped to
            /// a near value.
            pub fn parse_field(field: &str, value: &str) -> Result<$name, RelationError> {
                match value {
                    $( $wire => Ok($name::$variant), )+
                    other => Err(schema(format!(
                        "{field}: {other:?} is not a {} value",
                        stringify!($name)
                    ))),
                }
            }
        }
    };
}

closed_enum! {
    /// The seven input spellings of a relationship (D1).
    RelationSpelling {
        /// `A blocks B`.
        Blocks => "blocks",
        /// `A blocked_by B`.
        BlockedBy => "blocked_by",
        /// `A parent_of B`.
        ParentOf => "parent_of",
        /// `A child_of B`.
        ChildOf => "child_of",
        /// `A supersedes B`.
        Supersedes => "supersedes",
        /// `A related_to B`.
        RelatedTo => "related_to",
        /// `A duplicates B`: A is the copy.
        Duplicates => "duplicates",
    }
}

impl RelationSpelling {
    /// Parse an input spelling. Anything but the seven is
    /// `unknown_relation`; nothing is aliased by guess.
    pub fn parse(value: &str) -> Result<RelationSpelling, RelationError> {
        RelationSpelling::ALL
            .iter()
            .copied()
            .find(|spelling| spelling.as_str() == value)
            .ok_or_else(|| RelationError::UnknownRelation {
                spelling: value.to_string(),
            })
    }

    /// The spelling that states the same fact with subject and object
    /// swapped, where one exists among the seven. `supersedes` has no
    /// input inverse (`superseded_by` is display-only), and swapping a
    /// `duplicates` statement changes which end is the copy.
    pub fn inverse(&self) -> Option<RelationSpelling> {
        match self {
            RelationSpelling::Blocks => Some(RelationSpelling::BlockedBy),
            RelationSpelling::BlockedBy => Some(RelationSpelling::Blocks),
            RelationSpelling::ParentOf => Some(RelationSpelling::ChildOf),
            RelationSpelling::ChildOf => Some(RelationSpelling::ParentOf),
            RelationSpelling::RelatedTo => Some(RelationSpelling::RelatedTo),
            RelationSpelling::Supersedes | RelationSpelling::Duplicates => None,
        }
    }
}

closed_enum! {
    /// The five canonical relations (D1).
    Relation {
        /// Source blocks target; the only relation that can be hard.
        Blocks => "blocks",
        /// An advisory link with no direction.
        RelatedTo => "related_to",
        /// Source is the parent of target.
        Hierarchy => "hierarchy",
        /// The two ends are the same work; `role` names the copy.
        Duplicates => "duplicates",
        /// Source supersedes target.
        Supersedes => "supersedes",
    }
}

closed_enum! {
    /// Which end of a `duplicates` edge is the copy (D1).
    DuplicateRole {
        /// The source is the copy.
        SourceIsDuplicate => "source_is_duplicate",
        /// The target is the copy.
        TargetIsDuplicate => "target_is_duplicate",
    }
}

closed_enum! {
    /// The provider of an external end (D2).
    Provider {
        /// GitHub.
        Github => "github",
    }
}

closed_enum! {
    /// The kind of provider resource an external end names (D2).
    ResourceKind {
        /// An issue.
        Issue => "issue",
        /// A pull request.
        PullRequest => "pull_request",
        /// An Actions workflow run.
        ActionsRun => "actions_run",
    }
}

impl ResourceKind {
    /// The key prefix, matching the legacy `dep_key` spelling.
    pub fn key_prefix(&self) -> &'static str {
        match self {
            ResourceKind::Issue => "issue",
            ResourceKind::PullRequest => "pr",
            ResourceKind::ActionsRun => "ci",
        }
    }
}

closed_enum! {
    /// Whether an edge gates readiness (D4).
    Hardness {
        /// Gates readiness; `blocks` only.
        Hard => "hard",
        /// Advisory.
        Soft => "soft",
    }
}

closed_enum! {
    /// An edge's lifecycle status (D4, D5).
    EdgeStatus {
        /// In force.
        Active => "active",
        /// Satisfied, automatically, with evidence, or by override.
        Resolved => "resolved",
        /// Removed with a reason; final.
        Removed => "removed",
        /// Held aside by import or repair.
        Quarantined => "quarantined",
    }
}

closed_enum! {
    /// Where an edge came from (the wire field `reason_code`, D4).
    /// Distinct from `contracts::ReasonCode`, which is about commands.
    EdgeOrigin {
        /// Stated by a caller.
        Explicit => "explicit",
        /// Brought in by an importer.
        Imported => "imported",
        /// Carried over by a migration.
        Migrated => "migrated",
        /// Inferred; waits for review.
        InferredPendingReview => "inferred_pending_review",
    }
}

closed_enum! {
    /// How sure the edge's ends are (D4).
    Certainty {
        /// Every external end carries a provider id.
        Verified => "verified",
        /// Stated, not checked.
        Asserted => "asserted",
        /// An end may not be what it names.
        Uncertain => "uncertain",
    }
}

closed_enum! {
    /// How an edge reads from one of its ends (D6). `superseded_by` and
    /// `duplicated_by` are display words only, never input spellings.
    PerspectiveSpelling {
        /// This end blocks the other.
        Blocks => "blocks",
        /// This end is blocked by the other.
        BlockedBy => "blocked_by",
        /// This end is the other's parent.
        ParentOf => "parent_of",
        /// This end is the other's child.
        ChildOf => "child_of",
        /// This end supersedes the other.
        Supersedes => "supersedes",
        /// This end is superseded by the other.
        SupersededBy => "superseded_by",
        /// The two ends are related.
        RelatedTo => "related_to",
        /// This end duplicates the other.
        Duplicates => "duplicates",
        /// This end is duplicated by the other.
        DuplicatedBy => "duplicated_by",
    }
}

closed_enum! {
    /// Why a blocker holds, in the blockers read view (D8).
    BlockerState {
        /// The blocker is not satisfied.
        Unsatisfied => "unsatisfied",
        /// The external end could not be resolved; it blocks by default.
        UnresolvedExternal => "unresolved_external",
        /// The edge is quarantined.
        Quarantined => "quarantined",
        /// An override is in force.
        OverrideActive => "override_active",
    }
}

// ---------------------------------------------------------------------------
// D2. Edge ends
// ---------------------------------------------------------------------------

/// A provider resource at one end of an edge (D2).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ExternalRef {
    /// The provider.
    pub provider: Provider,
    /// The provider's canonical `owner/repo`, as the caller resolved it.
    pub repository: String,
    /// What kind of resource it is.
    pub resource_kind: ResourceKind,
    /// The issue, pull request or run number, as a canonical decimal
    /// string.
    pub number: String,
    /// The provider's stable id. The repository name is the label;
    /// this is the identity.
    pub node_id: Option<String>,
}

impl ExternalRef {
    /// The identity key, byte-identical to the legacy `dep_key`
    /// spelling: `issue:owner/repo#N`, `pr:owner/repo#N`,
    /// `ci:owner/repo#RUN`.
    pub fn key(&self) -> String {
        format!(
            "{}:{}#{}",
            self.resource_kind.key_prefix(),
            self.repository,
            self.number
        )
    }

    /// Validate the field shapes.
    pub fn validate(&self) -> Result<(), RelationError> {
        if !is_repository(&self.repository) {
            return Err(schema(format!(
                "repository: {:?} is not owner/repo",
                self.repository
            )));
        }
        if !is_canonical_decimal(&self.number) {
            return Err(schema(format!(
                "number: {:?} is not a canonical decimal string",
                self.number
            )));
        }
        if matches!(&self.node_id, Some(id) if id.is_empty()) {
            return Err(schema("node_id: must be null or non-empty"));
        }
        Ok(())
    }
}

fn is_repository(value: &str) -> bool {
    let mut parts = value.split('/');
    let valid_part = |part: Option<&str>| {
        part.is_some_and(|p| {
            !p.is_empty()
                && p.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
        })
    };
    valid_part(parts.next()) && valid_part(parts.next()) && parts.next().is_none()
}

/// One end of an edge (D2).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum EdgeEnd {
    /// A Fleet work item.
    FleetWork(WorkRef),
    /// A provider resource.
    External(ExternalRef),
}

impl EdgeEnd {
    /// The identity key DEP-03's `source_key` stores: `task:<work_id>`
    /// for local Fleet work, `work:<authority_id>/<work_id>` for Fleet
    /// work in another authority, and the [`ExternalRef::key`]
    /// spelling for external ends. Never a number alone.
    pub fn key(&self) -> String {
        match self {
            EdgeEnd::FleetWork(work) if work.authority_id == LOCAL_AUTHORITY_ID => {
                format!("task:{}", work.work_id)
            }
            EdgeEnd::FleetWork(work) => format!("work:{}/{}", work.authority_id, work.work_id),
            EdgeEnd::External(external) => external.key(),
        }
    }

    /// Whether this end is Fleet work.
    pub fn is_fleet_work(&self) -> bool {
        matches!(self, EdgeEnd::FleetWork(_))
    }

    /// Whether this is an external end without a provider id.
    fn missing_node_id(&self) -> bool {
        matches!(self, EdgeEnd::External(ExternalRef { node_id: None, .. }))
    }

    /// Validate the field shapes.
    pub fn validate(&self) -> Result<(), RelationError> {
        match self {
            EdgeEnd::FleetWork(work) => {
                if work.authority_id.contains('/') {
                    return Err(schema(format!(
                        "authority_id: {:?} must not contain '/'",
                        work.authority_id
                    )));
                }
                work.validate().map_err(RelationError::from)
            }
            EdgeEnd::External(external) => external.validate(),
        }
    }

    /// The wire object for this end.
    pub fn to_json_value(&self) -> Value {
        let mut map = Map::new();
        match self {
            EdgeEnd::FleetWork(work) => {
                map.insert("kind".into(), Value::from("fleet_work"));
                let mut inner = Map::new();
                inner.insert(
                    "authority_id".into(),
                    Value::from(work.authority_id.clone()),
                );
                inner.insert("work_id".into(), Value::from(work.work_id.clone()));
                map.insert("work".into(), Value::Object(inner));
            }
            EdgeEnd::External(external) => {
                map.insert("kind".into(), Value::from("external"));
                map.insert("provider".into(), Value::from(external.provider.as_str()));
                map.insert(
                    "repository".into(),
                    Value::from(external.repository.clone()),
                );
                map.insert(
                    "resource_kind".into(),
                    Value::from(external.resource_kind.as_str()),
                );
                map.insert("number".into(), Value::from(external.number.clone()));
                map.insert("node_id".into(), nullable(&external.node_id));
            }
        }
        Value::Object(map)
    }

    /// Parse the wire object of `field`. Unknown fields are refused.
    pub fn from_json_value(field: &str, value: &Value) -> Result<EdgeEnd, RelationError> {
        let kind = value
            .as_object()
            .and_then(|map| map.get("kind"))
            .and_then(Value::as_str)
            .ok_or_else(|| schema(format!("{field}: must be an object with a string kind")))?;
        let end = match kind {
            "fleet_work" => {
                let map = object(field, value, &["kind", "work"])?;
                let work_field = format!("{field}.work");
                let work = object(&work_field, &map["work"], &["authority_id", "work_id"])?;
                let work = work.clone();
                EdgeEnd::FleetWork(WorkRef {
                    authority_id: string(&work_field, &work, "authority_id")?,
                    work_id: string(&work_field, &work, "work_id")?,
                })
            }
            "external" => {
                let map = object(
                    field,
                    value,
                    &[
                        "kind",
                        "provider",
                        "repository",
                        "resource_kind",
                        "number",
                        "node_id",
                    ],
                )?;
                EdgeEnd::External(ExternalRef {
                    provider: Provider::parse_field(
                        &format!("{field}.provider"),
                        &string(field, map, "provider")?,
                    )?,
                    repository: string(field, map, "repository")?,
                    resource_kind: ResourceKind::parse_field(
                        &format!("{field}.resource_kind"),
                        &string(field, map, "resource_kind")?,
                    )?,
                    number: string(field, map, "number")?,
                    node_id: nullable_string(field, map, "node_id")?,
                })
            }
            other => {
                return Err(schema(format!(
                    "{field}.kind: {other:?} is not fleet_work or external"
                )));
            }
        };
        end.validate()?;
        Ok(end)
    }
}

// ---------------------------------------------------------------------------
// D1. Normalization
// ---------------------------------------------------------------------------

/// One relationship in its canonical direction (D1).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CanonicalEdge {
    /// The canonical relation.
    pub relation: Relation,
    /// The source end.
    pub source: EdgeEnd,
    /// The target end; always Fleet work.
    pub target: EdgeEnd,
    /// Which end is the copy; `duplicates` only.
    pub role: Option<DuplicateRole>,
}

/// Turn `subject spelling object` into its one canonical edge (D1).
///
/// `blocked_by` and `child_of` swap the ends; `related_to` and
/// `duplicates` use pair order (Fleet work is the target; between two
/// work items the smaller `WorkRef` is the source). The canonical
/// target must be Fleet work, and the two ends must differ by key.
pub fn normalize(
    subject: &EdgeEnd,
    spelling: RelationSpelling,
    object: &EdgeEnd,
) -> Result<CanonicalEdge, RelationError> {
    subject.validate()?;
    object.validate()?;
    let subject_key = subject.key();
    if subject_key == object.key() {
        return Err(RelationError::SelfEdge { key: subject_key });
    }
    let (relation, source, target, role) = match spelling {
        RelationSpelling::Blocks => (Relation::Blocks, subject, object, None),
        RelationSpelling::BlockedBy => (Relation::Blocks, object, subject, None),
        RelationSpelling::ParentOf => (Relation::Hierarchy, subject, object, None),
        RelationSpelling::ChildOf => (Relation::Hierarchy, object, subject, None),
        RelationSpelling::Supersedes => (Relation::Supersedes, subject, object, None),
        RelationSpelling::RelatedTo => {
            let (source, target, _) = pair_order(subject, object)?;
            (Relation::RelatedTo, source, target, None)
        }
        RelationSpelling::Duplicates => {
            let (source, target, swapped) = pair_order(subject, object)?;
            // "A duplicates B": A is the copy, wherever ordering put it.
            let role = if swapped {
                DuplicateRole::TargetIsDuplicate
            } else {
                DuplicateRole::SourceIsDuplicate
            };
            (Relation::Duplicates, source, target, Some(role))
        }
    };
    if !target.is_fleet_work() {
        return Err(RelationError::TargetNotFleetWork {
            detail: format!(
                "{} {} has an external target {}; the target is always Fleet work",
                relation.as_str(),
                source.key(),
                target.key()
            ),
        });
    }
    Ok(CanonicalEdge {
        relation,
        source: source.clone(),
        target: target.clone(),
        role,
    })
}

/// Pair order for the undirected relations: `(source, target,
/// swapped)`, where `swapped` says the subject became the target.
fn pair_order<'a>(
    subject: &'a EdgeEnd,
    object: &'a EdgeEnd,
) -> Result<(&'a EdgeEnd, &'a EdgeEnd, bool), RelationError> {
    match (subject, object) {
        (EdgeEnd::FleetWork(a), EdgeEnd::FleetWork(b)) => {
            if a < b {
                Ok((subject, object, false))
            } else {
                Ok((object, subject, true))
            }
        }
        (EdgeEnd::External(_), EdgeEnd::FleetWork(_)) => Ok((subject, object, false)),
        (EdgeEnd::FleetWork(_), EdgeEnd::External(_)) => Ok((object, subject, true)),
        (EdgeEnd::External(_), EdgeEnd::External(_)) => {
            let (mut a, mut b) = (subject.key(), object.key());
            if b < a {
                std::mem::swap(&mut a, &mut b);
            }
            Err(RelationError::TargetNotFleetWork {
                detail: format!("neither {a} nor {b} is Fleet work"),
            })
        }
    }
}

// ---------------------------------------------------------------------------
// D6. Perspective
// ---------------------------------------------------------------------------

/// How an edge reads from one of its ends (D6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Perspective {
    /// The relation as stated from this end.
    pub spelling: PerspectiveSpelling,
    /// The other end.
    pub other_end: EdgeEnd,
}

impl Perspective {
    /// The wire object for this perspective.
    pub fn to_json_value(&self) -> Value {
        let mut map = Map::new();
        map.insert("spelling".into(), Value::from(self.spelling.as_str()));
        map.insert("other_end".into(), self.other_end.to_json_value());
        Value::Object(map)
    }
}

/// Read `edge` from the end `from` (matched by key). `None` when
/// `from` is neither end.
pub fn describe(edge: &CanonicalEdge, from: &EdgeEnd) -> Option<Perspective> {
    let key = from.key();
    let from_source = key == edge.source.key();
    let from_target = key == edge.target.key();
    if !from_source && !from_target {
        return None;
    }
    let spelling = match (edge.relation, from_source) {
        (Relation::Blocks, true) => PerspectiveSpelling::Blocks,
        (Relation::Blocks, false) => PerspectiveSpelling::BlockedBy,
        (Relation::Hierarchy, true) => PerspectiveSpelling::ParentOf,
        (Relation::Hierarchy, false) => PerspectiveSpelling::ChildOf,
        (Relation::Supersedes, true) => PerspectiveSpelling::Supersedes,
        (Relation::Supersedes, false) => PerspectiveSpelling::SupersededBy,
        (Relation::RelatedTo, _) => PerspectiveSpelling::RelatedTo,
        (Relation::Duplicates, from_source) => match edge.role {
            None => PerspectiveSpelling::Duplicates,
            Some(DuplicateRole::SourceIsDuplicate) if from_source => {
                PerspectiveSpelling::Duplicates
            }
            Some(DuplicateRole::TargetIsDuplicate) if !from_source => {
                PerspectiveSpelling::Duplicates
            }
            Some(_) => PerspectiveSpelling::DuplicatedBy,
        },
    };
    let other_end = if from_source {
        edge.target.clone()
    } else {
        edge.source.clone()
    };
    Some(Perspective {
        spelling,
        other_end,
    })
}

// ---------------------------------------------------------------------------
// D3. Ref input forms
// ---------------------------------------------------------------------------

/// The syntax of a ref, before binding (D3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefInput {
    /// `fleet://<authority_id>/work/<work_id>`.
    Qualified(WorkRef),
    /// `fleet://work/<work_id>` or `fleet:<work_id>`; needs exactly one
    /// authority in context.
    LocalShorthand {
        /// The work id.
        work_id: String,
    },
    /// `owner/repo#N` (kind from the lookup) or `issue:`, `pr:`, `ci:`
    /// with the kind given.
    External {
        /// The kind, when the text gave one.
        resource_kind: Option<ResourceKind>,
        /// `owner/repo`.
        repository: String,
        /// The number.
        number: String,
    },
    /// `#N`; needs exactly one linked repository.
    LinkedRepository {
        /// The number.
        number: String,
    },
}

/// Parse ref text (D3). `None` when the text matches no form,
/// including a bare number; the caller names that refusal
/// (`malformed_value` or `ambiguous_target_form` from the CLI parser,
/// `invalid_schema` from JSON — see [`parse_ref_field`]).
pub fn parse_ref(text: &str) -> Option<RefInput> {
    if let Some(rest) = text.strip_prefix("fleet://") {
        if let Some(work_id) = rest.strip_prefix("work/") {
            if is_canonical_decimal(work_id) {
                return Some(RefInput::LocalShorthand {
                    work_id: work_id.to_string(),
                });
            }
        }
        let (authority_id, work_id) = rest.split_once("/work/")?;
        if authority_id.is_empty() || authority_id.contains('/') || !is_canonical_decimal(work_id) {
            return None;
        }
        return Some(RefInput::Qualified(WorkRef {
            authority_id: authority_id.to_string(),
            work_id: work_id.to_string(),
        }));
    }
    if let Some(work_id) = text.strip_prefix("fleet:") {
        return is_canonical_decimal(work_id).then(|| RefInput::LocalShorthand {
            work_id: work_id.to_string(),
        });
    }
    for (prefix, kind) in [
        ("issue:", ResourceKind::Issue),
        ("pr:", ResourceKind::PullRequest),
        ("ci:", ResourceKind::ActionsRun),
    ] {
        if let Some(rest) = text.strip_prefix(prefix) {
            let (repository, number) = repository_number(rest)?;
            return Some(RefInput::External {
                resource_kind: Some(kind),
                repository,
                number,
            });
        }
    }
    if let Some(number) = text.strip_prefix('#') {
        return is_canonical_decimal(number).then(|| RefInput::LinkedRepository {
            number: number.to_string(),
        });
    }
    let (repository, number) = repository_number(text)?;
    Some(RefInput::External {
        resource_kind: None,
        repository,
        number,
    })
}

/// [`parse_ref`] for a value that arrived in JSON: text matching no
/// form is `invalid_schema` naming `field`.
pub fn parse_ref_field(field: &str, text: &str) -> Result<RefInput, RelationError> {
    parse_ref(text).ok_or_else(|| schema(format!("{field}: {text:?} is not a ref")))
}

fn repository_number(text: &str) -> Option<(String, String)> {
    let (repository, number) = text.split_once('#')?;
    if is_repository(repository) && is_canonical_decimal(number) {
        Some((repository.to_string(), number.to_string()))
    } else {
        None
    }
}

/// The facts a caller resolved before binding a ref (D3).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BindContext {
    /// The authenticated authorities a shorthand could mean.
    pub authorities: Vec<String>,
    /// The work item's linked repositories, for `#N`.
    pub linked_repositories: Vec<String>,
    /// The provider's answers for external lookups.
    pub resolved_externals: Vec<ExternalRef>,
}

/// Bind parsed ref syntax to an edge end, given the resolved facts.
///
/// A shorthand under zero or several authorities, or `#N` under zero
/// or several linked repositories, is `ambiguous_ref`. An untyped
/// `owner/repo#N` the lookup did not answer is `unresolved_ref`. A
/// typed external ref the lookup did not answer binds without a
/// `node_id`; [`Edge::validate`] decides whether that is allowed.
pub fn bind_ref(input: &RefInput, context: &BindContext) -> Result<EdgeEnd, RelationError> {
    let end = match input {
        RefInput::Qualified(work) => EdgeEnd::FleetWork(work.clone()),
        RefInput::LocalShorthand { work_id } => match context.authorities.as_slice() {
            [authority] => EdgeEnd::FleetWork(WorkRef {
                authority_id: authority.clone(),
                work_id: work_id.clone(),
            }),
            others => {
                return Err(RelationError::AmbiguousRef {
                    detail: format!(
                        "fleet:{work_id} needs exactly one authority, found {}; write fleet://<authority_id>/work/{work_id}",
                        others.len()
                    ),
                });
            }
        },
        RefInput::LinkedRepository { number } => match context.linked_repositories.as_slice() {
            [repository] => bind_external(None, repository, number, context)?,
            others => {
                return Err(RelationError::AmbiguousRef {
                    detail: format!(
                        "#{number} needs exactly one linked repository, found {}; write owner/repo#{number}",
                        others.len()
                    ),
                });
            }
        },
        RefInput::External {
            resource_kind,
            repository,
            number,
        } => bind_external(*resource_kind, repository, number, context)?,
    };
    end.validate()?;
    Ok(end)
}

fn bind_external(
    resource_kind: Option<ResourceKind>,
    repository: &str,
    number: &str,
    context: &BindContext,
) -> Result<EdgeEnd, RelationError> {
    let matches: Vec<&ExternalRef> = context
        .resolved_externals
        .iter()
        .filter(|candidate| {
            candidate.provider == Provider::Github
                && candidate.repository == repository
                && candidate.number == number
                && resource_kind.is_none_or(|kind| candidate.resource_kind == kind)
        })
        .collect();
    match (matches.as_slice(), resource_kind) {
        ([only], _) => Ok(EdgeEnd::External((*only).clone())),
        ([], Some(kind)) => Ok(EdgeEnd::External(ExternalRef {
            provider: Provider::Github,
            repository: repository.to_string(),
            resource_kind: kind,
            number: number.to_string(),
            node_id: None,
        })),
        ([], None) => Err(RelationError::UnresolvedRef {
            detail: format!(
                "{repository}#{number}: the provider lookup did not say whether it is an issue or a pull request"
            ),
        }),
        (several, _) => Err(RelationError::AmbiguousRef {
            detail: format!(
                "{repository}#{number} matches {} resources; write issue:, pr: or ci: with it",
                several.len()
            ),
        }),
    }
}

// ---------------------------------------------------------------------------
// D4. The edge record
// ---------------------------------------------------------------------------

/// Who and what produced an edge or revision (D4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Provenance {
    /// The authenticated principal the caller passes in; never read
    /// from a client flag.
    pub principal_ref: String,
    /// The acting agent or person, for audit only.
    pub actor_ref: Option<String>,
    /// The session, for audit only.
    pub session_ref: Option<String>,
    /// The operation that wrote the edge.
    pub operation_id: Option<String>,
    /// The source event, when the edge derives from one.
    pub source_ref: Option<String>,
}

const PROVENANCE_FIELDS: &[&str] = &[
    "principal_ref",
    "actor_ref",
    "session_ref",
    "operation_id",
    "source_ref",
];

impl Provenance {
    fn to_json_value(&self) -> Value {
        let mut map = Map::new();
        map.insert(
            "principal_ref".into(),
            Value::from(self.principal_ref.clone()),
        );
        map.insert("actor_ref".into(), nullable(&self.actor_ref));
        map.insert("session_ref".into(), nullable(&self.session_ref));
        map.insert("operation_id".into(), nullable(&self.operation_id));
        map.insert("source_ref".into(), nullable(&self.source_ref));
        Value::Object(map)
    }

    fn from_json_value(field: &str, value: &Value) -> Result<Provenance, RelationError> {
        let map = object(field, value, PROVENANCE_FIELDS)?;
        Ok(Provenance {
            principal_ref: string(field, map, "principal_ref")?,
            actor_ref: nullable_string(field, map, "actor_ref")?,
            session_ref: nullable_string(field, map, "session_ref")?,
            operation_id: nullable_string(field, map, "operation_id")?,
            source_ref: nullable_string(field, map, "source_ref")?,
        })
    }
}

/// How a non-active edge ended (D4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolution {
    /// Why, e.g. `blocker_satisfied`.
    pub reason: String,
    /// The evidence, when there is any.
    pub evidence_ref: Option<String>,
    /// Whether the resolution is an override; `resolved` only.
    pub override_: bool,
    /// When an override expires.
    pub expires_at: Option<String>,
    /// The edge that replaced this one, as a canonical decimal string.
    pub successor_edge_id: Option<String>,
}

const RESOLUTION_FIELDS: &[&str] = &[
    "reason",
    "evidence_ref",
    "override",
    "expires_at",
    "successor_edge_id",
];

impl Resolution {
    fn to_json_value(&self) -> Value {
        let mut map = Map::new();
        map.insert("reason".into(), Value::from(self.reason.clone()));
        map.insert("evidence_ref".into(), nullable(&self.evidence_ref));
        map.insert("override".into(), Value::Bool(self.override_));
        map.insert("expires_at".into(), nullable(&self.expires_at));
        map.insert(
            "successor_edge_id".into(),
            nullable(&self.successor_edge_id),
        );
        Value::Object(map)
    }

    fn from_json_value(field: &str, value: &Value) -> Result<Option<Resolution>, RelationError> {
        if value.is_null() {
            return Ok(None);
        }
        let map = object(field, value, RESOLUTION_FIELDS)?;
        let override_ = match map.get("override") {
            Some(Value::Bool(b)) => *b,
            _ => return Err(schema(format!("{field}.override must be a boolean"))),
        };
        Ok(Some(Resolution {
            reason: string(field, map, "reason")?,
            evidence_ref: nullable_string(field, map, "evidence_ref")?,
            override_,
            expires_at: nullable_string(field, map, "expires_at")?,
            successor_edge_id: nullable_string(field, map, "successor_edge_id")?,
        }))
    }
}

/// Status and resolution must agree: no resolution while `active`, a
/// resolution with a reason otherwise, and an override only on
/// `resolved`.
fn validate_resolution(
    status: EdgeStatus,
    resolution: &Option<Resolution>,
) -> Result<(), RelationError> {
    match (status, resolution) {
        (EdgeStatus::Active, None) => Ok(()),
        (EdgeStatus::Active, Some(_)) => Err(schema("resolution: must be null while active")),
        (_, None) => Err(schema(format!(
            "resolution: required when status is {}",
            status.as_str()
        ))),
        (_, Some(resolution)) => {
            if resolution.reason.is_empty() {
                return Err(schema("resolution.reason: must not be empty"));
            }
            if resolution.override_ && status != EdgeStatus::Resolved {
                return Err(schema(format!(
                    "resolution.override: allowed only with status resolved, not {}",
                    status.as_str()
                )));
            }
            if let Some(successor) = &resolution.successor_edge_id {
                if !is_canonical_decimal(successor) {
                    return Err(schema(
                        "resolution.successor_edge_id: not a canonical decimal string",
                    ));
                }
            }
            Ok(())
        }
    }
}

/// One dependency edge, field for field DEP-03's `dep_edges` row (D4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edge {
    /// The edge id, as a canonical decimal string.
    pub edge_id: String,
    /// The revision, as a canonical decimal string; `1` at add.
    pub revision: String,
    /// The canonical relation.
    pub relation: Relation,
    /// Which end is the copy; `duplicates` only.
    pub role: Option<DuplicateRole>,
    /// The source end.
    pub source: EdgeEnd,
    /// The target end; always Fleet work.
    pub target: EdgeEnd,
    /// `source.key()`, stored for lookups.
    pub source_key: String,
    /// Whether the edge gates readiness.
    pub hardness: Hardness,
    /// The lifecycle status.
    pub status: EdgeStatus,
    /// Where the edge came from (wire `reason_code`).
    pub reason_code: EdgeOrigin,
    /// How sure the ends are.
    pub certainty: Certainty,
    /// Who and what wrote it.
    pub provenance: Provenance,
    /// When it was added (RFC 3339, from the caller).
    pub created_at: String,
    /// When it last changed (RFC 3339, from the caller).
    pub updated_at: String,
    /// How it ended; null while active.
    pub resolution: Option<Resolution>,
}

const EDGE_FIELDS: &[&str] = &[
    "schema",
    "edge_id",
    "revision",
    "relation",
    "role",
    "source",
    "target",
    "source_key",
    "hardness",
    "status",
    "reason_code",
    "certainty",
    "provenance",
    "created_at",
    "updated_at",
    "resolution",
];

impl Edge {
    /// The edge's canonical relationship.
    pub fn canonical(&self) -> CanonicalEdge {
        CanonicalEdge {
            relation: self.relation,
            source: self.source.clone(),
            target: self.target.clone(),
            role: self.role,
        }
    }

    /// The mutable part of the edge, as a revision records it.
    pub fn state(&self) -> EdgeState {
        EdgeState {
            status: self.status,
            hardness: self.hardness,
            certainty: self.certainty,
            role: self.role,
            resolution: self.resolution.clone(),
        }
    }

    /// Check every D4 field rule.
    pub fn validate(&self) -> Result<(), RelationError> {
        require_decimal("edge_id", &self.edge_id)?;
        require_revision("revision", &self.revision)?;
        self.source.validate()?;
        self.target.validate()?;
        if self.source_key != self.source.key() {
            return Err(schema(format!(
                "source_key: {:?} does not match the source's key {:?}",
                self.source_key,
                self.source.key()
            )));
        }
        if !self.target.is_fleet_work() {
            return Err(RelationError::TargetNotFleetWork {
                detail: format!("target {} is external", self.target.key()),
            });
        }
        if self.source.key() == self.target.key() {
            return Err(RelationError::SelfEdge {
                key: self.source_key.clone(),
            });
        }
        if self.hardness == Hardness::Hard && self.relation != Relation::Blocks {
            return Err(RelationError::RelationNotGating {
                relation: self.relation,
            });
        }
        if matches!(self.relation, Relation::RelatedTo | Relation::Duplicates) {
            let (_, _, swapped) = pair_order(&self.source, &self.target)?;
            if swapped {
                // Refuse rather than silently swapping: on duplicates, role
                // names an end and would change meaning if left untouched.
                return Err(schema("source and target: non-canonical pair order"));
            }
        }
        if self.role.is_some() && self.relation != Relation::Duplicates {
            return Err(schema(format!(
                "role: present only on duplicates, not {}",
                self.relation.as_str()
            )));
        }
        for end in [&self.source, &self.target] {
            if end.missing_node_id() {
                let imported = matches!(
                    self.reason_code,
                    EdgeOrigin::Imported | EdgeOrigin::Migrated
                );
                if !imported || self.certainty != Certainty::Uncertain {
                    return Err(RelationError::UnresolvedRef {
                        detail: format!(
                            "{} has no provider id; only an imported or migrated edge with certainty uncertain may keep it, not {} with {}",
                            end.key(),
                            self.reason_code.as_str(),
                            self.certainty.as_str()
                        ),
                    });
                }
            }
        }
        if self.provenance.principal_ref.is_empty() {
            return Err(schema("provenance.principal_ref: must not be empty"));
        }
        if self.created_at.is_empty() || self.updated_at.is_empty() {
            return Err(schema("created_at and updated_at: must not be empty"));
        }
        validate_resolution(self.status, &self.resolution)
    }

    /// The `fleet.dependency_edge/v1` wire object.
    pub fn to_json_value(&self) -> Value {
        let mut map = Map::new();
        map.insert("schema".into(), Value::from(EDGE_SCHEMA));
        map.insert("edge_id".into(), Value::from(self.edge_id.clone()));
        map.insert("revision".into(), Value::from(self.revision.clone()));
        map.insert("relation".into(), Value::from(self.relation.as_str()));
        map.insert("role".into(), role_value(self.role));
        map.insert("source".into(), self.source.to_json_value());
        map.insert("target".into(), self.target.to_json_value());
        map.insert("source_key".into(), Value::from(self.source_key.clone()));
        map.insert("hardness".into(), Value::from(self.hardness.as_str()));
        map.insert("status".into(), Value::from(self.status.as_str()));
        map.insert("reason_code".into(), Value::from(self.reason_code.as_str()));
        map.insert("certainty".into(), Value::from(self.certainty.as_str()));
        map.insert("provenance".into(), self.provenance.to_json_value());
        map.insert("created_at".into(), Value::from(self.created_at.clone()));
        map.insert("updated_at".into(), Value::from(self.updated_at.clone()));
        map.insert(
            "resolution".into(),
            self.resolution
                .as_ref()
                .map_or(Value::Null, Resolution::to_json_value),
        );
        Value::Object(map)
    }

    /// Compact JSON text of [`Edge::to_json_value`].
    pub fn to_json(&self) -> String {
        self.to_json_value().to_string()
    }

    /// Parse and validate a wire edge. Unknown fields and enum values
    /// are refused.
    pub fn from_json(text: &str) -> Result<Edge, RelationError> {
        let value: Value =
            serde_json::from_str(text).map_err(|e| schema(format!("edge: not JSON: {e}")))?;
        let edge = Edge::from_json_value(&value)?;
        edge.validate()?;
        Ok(edge)
    }

    /// Parse a wire edge's shape, without the D4 rule checks.
    pub fn from_json_value(value: &Value) -> Result<Edge, RelationError> {
        let map = object("edge", value, EDGE_FIELDS)?;
        let schema_id = string("edge", map, "schema")?;
        if schema_id != EDGE_SCHEMA {
            return Err(schema(format!(
                "schema: {schema_id:?} is not {EDGE_SCHEMA}"
            )));
        }
        Ok(Edge {
            edge_id: string("edge", map, "edge_id")?,
            revision: string("edge", map, "revision")?,
            relation: Relation::parse_field("relation", &string("edge", map, "relation")?)?,
            role: role_field("role", &map["role"])?,
            source: EdgeEnd::from_json_value("source", &map["source"])?,
            target: EdgeEnd::from_json_value("target", &map["target"])?,
            source_key: string("edge", map, "source_key")?,
            hardness: Hardness::parse_field("hardness", &string("edge", map, "hardness")?)?,
            status: EdgeStatus::parse_field("status", &string("edge", map, "status")?)?,
            reason_code: EdgeOrigin::parse_field(
                "reason_code",
                &string("edge", map, "reason_code")?,
            )?,
            certainty: Certainty::parse_field("certainty", &string("edge", map, "certainty")?)?,
            provenance: Provenance::from_json_value("provenance", &map["provenance"])?,
            created_at: string("edge", map, "created_at")?,
            updated_at: string("edge", map, "updated_at")?,
            resolution: Resolution::from_json_value("resolution", &map["resolution"])?,
        })
    }
}

// ---------------------------------------------------------------------------
// D5. Transitions and revisions
// ---------------------------------------------------------------------------

/// The eight allowed status moves (D5). `None` is "no edge yet".
pub const ALLOWED_TRANSITIONS: &[(Option<EdgeStatus>, EdgeStatus)] = &[
    (None, EdgeStatus::Active),
    (None, EdgeStatus::Quarantined),
    (Some(EdgeStatus::Active), EdgeStatus::Resolved),
    (Some(EdgeStatus::Active), EdgeStatus::Removed),
    (Some(EdgeStatus::Active), EdgeStatus::Quarantined),
    (Some(EdgeStatus::Quarantined), EdgeStatus::Active),
    (Some(EdgeStatus::Quarantined), EdgeStatus::Removed),
    (Some(EdgeStatus::Resolved), EdgeStatus::Active),
];

/// Check one status move against D5's closed table. `removed` is
/// final; bringing a removed fact back is a new add.
pub fn transition(from: Option<EdgeStatus>, to: EdgeStatus) -> Result<(), RelationError> {
    if ALLOWED_TRANSITIONS.contains(&(from, to)) {
        Ok(())
    } else {
        Err(RelationError::InvalidTransition {
            detail: format!(
                "{} -> {} is not an allowed status move",
                from.map_or("(none)", |status| status.as_str()),
                to.as_str()
            ),
        })
    }
}

/// The part of an edge a revision records before and after (D5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EdgeState {
    /// The status.
    pub status: EdgeStatus,
    /// The hardness.
    pub hardness: Hardness,
    /// The certainty.
    pub certainty: Certainty,
    /// The duplicate role; never changes across revisions.
    pub role: Option<DuplicateRole>,
    /// The resolution; null while active.
    pub resolution: Option<Resolution>,
}

const STATE_FIELDS: &[&str] = &["status", "hardness", "certainty", "role", "resolution"];

impl EdgeState {
    fn to_json_value(&self) -> Value {
        let mut map = Map::new();
        map.insert("status".into(), Value::from(self.status.as_str()));
        map.insert("hardness".into(), Value::from(self.hardness.as_str()));
        map.insert("certainty".into(), Value::from(self.certainty.as_str()));
        map.insert("role".into(), role_value(self.role));
        map.insert(
            "resolution".into(),
            self.resolution
                .as_ref()
                .map_or(Value::Null, Resolution::to_json_value),
        );
        Value::Object(map)
    }

    fn from_json_value(field: &str, value: &Value) -> Result<EdgeState, RelationError> {
        let map = object(field, value, STATE_FIELDS)?;
        Ok(EdgeState {
            status: EdgeStatus::parse_field(
                &format!("{field}.status"),
                &string(field, map, "status")?,
            )?,
            hardness: Hardness::parse_field(
                &format!("{field}.hardness"),
                &string(field, map, "hardness")?,
            )?,
            certainty: Certainty::parse_field(
                &format!("{field}.certainty"),
                &string(field, map, "certainty")?,
            )?,
            role: role_field(&format!("{field}.role"), &map["role"])?,
            resolution: Resolution::from_json_value(
                &format!("{field}.resolution"),
                &map["resolution"],
            )?,
        })
    }
}

/// One change to an edge, keeping both its before and after (D5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EdgeRevision {
    /// The edge id.
    pub edge_id: String,
    /// This revision; exactly one more than the previous.
    pub revision: String,
    /// The operation that made the change.
    pub operation_id: String,
    /// The state before; null only at revision `1`.
    pub before: Option<EdgeState>,
    /// The state after.
    pub after: EdgeState,
}

const REVISION_FIELDS: &[&str] = &[
    "schema",
    "edge_id",
    "revision",
    "operation_id",
    "before",
    "after",
];

impl EdgeRevision {
    /// Check the revision's own rules: decimal versions, `before` null
    /// exactly at revision `1`, a D5 status move, an unchanged role,
    /// and status/resolution agreement on both sides.
    pub fn validate(&self) -> Result<(), RelationError> {
        require_decimal("edge_id", &self.edge_id)?;
        require_revision("revision", &self.revision)?;
        if self.operation_id.is_empty() {
            return Err(schema("operation_id: must not be empty"));
        }
        validate_resolution(self.after.status, &self.after.resolution)?;
        match (&self.before, self.revision == "1") {
            (None, true) => transition(None, self.after.status),
            (None, false) => Err(schema("before: null only at revision 1")),
            (Some(_), true) => Err(schema("before: must be null at revision 1")),
            (Some(before), false) => {
                validate_resolution(before.status, &before.resolution)?;
                if before.role != self.after.role {
                    return Err(identity_changed());
                }
                if before.status == EdgeStatus::Removed {
                    return Err(RelationError::InvalidTransition {
                        detail: "removed is final; add a new edge instead".to_string(),
                    });
                }
                if before.status != self.after.status {
                    transition(Some(before.status), self.after.status)?;
                }
                Ok(())
            }
        }
    }

    /// The `fleet.dependency_edge_revision/v1` wire object.
    pub fn to_json_value(&self) -> Value {
        let mut map = Map::new();
        map.insert("schema".into(), Value::from(EDGE_REVISION_SCHEMA));
        map.insert("edge_id".into(), Value::from(self.edge_id.clone()));
        map.insert("revision".into(), Value::from(self.revision.clone()));
        map.insert(
            "operation_id".into(),
            Value::from(self.operation_id.clone()),
        );
        map.insert(
            "before".into(),
            self.before
                .as_ref()
                .map_or(Value::Null, EdgeState::to_json_value),
        );
        map.insert("after".into(), self.after.to_json_value());
        Value::Object(map)
    }

    /// Compact JSON text of [`EdgeRevision::to_json_value`].
    pub fn to_json(&self) -> String {
        self.to_json_value().to_string()
    }

    /// Parse and validate a wire revision. Unknown fields, enum values
    /// and numeric versions are refused.
    pub fn from_json(text: &str) -> Result<EdgeRevision, RelationError> {
        let value: Value = serde_json::from_str(text)
            .map_err(|e| schema(format!("edge_revision: not JSON: {e}")))?;
        let map = object("edge_revision", &value, REVISION_FIELDS)?;
        let schema_id = string("edge_revision", map, "schema")?;
        if schema_id != EDGE_REVISION_SCHEMA {
            return Err(schema(format!(
                "schema: {schema_id:?} is not {EDGE_REVISION_SCHEMA}"
            )));
        }
        let before = match &map["before"] {
            Value::Null => None,
            value => Some(EdgeState::from_json_value("before", value)?),
        };
        let revision = EdgeRevision {
            edge_id: string("edge_revision", map, "edge_id")?,
            revision: string("edge_revision", map, "revision")?,
            operation_id: string("edge_revision", map, "operation_id")?,
            before,
            after: EdgeState::from_json_value("after", &map["after"])?,
        };
        revision.validate()?;
        Ok(revision)
    }
}

fn identity_changed() -> RelationError {
    RelationError::InvalidTransition {
        detail: "relation, ends and role never change across revisions; remove and add instead"
            .to_string(),
    }
}

/// Whether two ends name the same thing: equal keys, and a provider id
/// that is only ever filled in, never changed.
fn same_end(before: &EdgeEnd, after: &EdgeEnd) -> bool {
    if before.key() != after.key() {
        return false;
    }
    match (before, after) {
        (EdgeEnd::External(a), EdgeEnd::External(b)) => match (&a.node_id, &b.node_id) {
            (Some(x), Some(y)) => x == y,
            (Some(_), None) => false,
            (None, _) => true,
        },
        _ => true,
    }
}

/// The revision that adds `edge` (revision `1`, no `before`).
pub fn first_revision(edge: &Edge, operation_id: &str) -> Result<EdgeRevision, RelationError> {
    edge.validate()?;
    let revision = EdgeRevision {
        edge_id: edge.edge_id.clone(),
        revision: edge.revision.clone(),
        operation_id: operation_id.to_string(),
        before: None,
        after: edge.state(),
    };
    revision.validate()?;
    Ok(revision)
}

/// The revision that moves `previous` to `next`. Refuses a change of
/// relation, ends or role (that is remove plus add), a revision that
/// is not exactly one more, and a status move outside D5.
pub fn revise(
    previous: &Edge,
    next: &Edge,
    operation_id: &str,
) -> Result<EdgeRevision, RelationError> {
    previous.validate()?;
    next.validate()?;
    if previous.edge_id != next.edge_id
        || previous.relation != next.relation
        || previous.role != next.role
        || !same_end(&previous.source, &next.source)
        || !same_end(&previous.target, &next.target)
    {
        return Err(identity_changed());
    }
    if next.revision != increment_decimal(&previous.revision) {
        return Err(RelationError::InvalidTransition {
            detail: format!(
                "revision {} does not follow {}",
                next.revision, previous.revision
            ),
        });
    }
    let revision = EdgeRevision {
        edge_id: next.edge_id.clone(),
        revision: next.revision.clone(),
        operation_id: operation_id.to_string(),
        before: Some(previous.state()),
        after: next.state(),
    };
    revision.validate()?;
    Ok(revision)
}

/// `value + 1` for a canonical decimal string, at any length.
pub fn increment_decimal(value: &str) -> String {
    let mut digits: Vec<u8> = value.bytes().collect();
    let mut index = digits.len();
    loop {
        if index == 0 {
            digits.insert(0, b'1');
            break;
        }
        index -= 1;
        if digits[index] == b'9' {
            digits[index] = b'0';
        } else {
            digits[index] += 1;
            break;
        }
    }
    String::from_utf8(digits).expect("ASCII digits")
}

// ---------------------------------------------------------------------------
// D8. Read views
// ---------------------------------------------------------------------------

/// The blockers view's `ready`: true only when the evaluation is
/// complete and no active hard `blocks` edge targets the work. An
/// incomplete evaluation is never ready (D8).
pub fn blockers_ready(complete: bool, active_hard_blockers: usize) -> bool {
    complete && active_hard_blockers == 0
}

// ---------------------------------------------------------------------------
// D9. Legacy blocked_deps
// ---------------------------------------------------------------------------

/// Map one legacy `blocked_deps` entry (`task`, `issue`, `pr`, `ci`) to
/// an end whose [`EdgeEnd::key`] equals `dep_key` for the same value.
/// The end carries no `node_id`, so an edge built from it is
/// `imported` and `uncertain` until DEP-03's importer resolves it.
pub fn from_legacy_dep(dep: &Value) -> Result<EdgeEnd, RelationError> {
    let map = dep
        .as_object()
        .ok_or_else(|| schema("legacy dep: must be an object"))?;
    let kind = map
        .get("kind")
        .and_then(Value::as_str)
        .ok_or_else(|| schema("legacy dep: kind must be a string"))?;
    let integer = |key: &str| -> Result<String, RelationError> {
        map.get(key)
            .and_then(Value::as_i64)
            .filter(|n| *n >= 0)
            .map(|n| n.to_string())
            .ok_or_else(|| {
                schema(format!(
                    "legacy {kind} dep: {key} must be a non-negative integer"
                ))
            })
    };
    let repository = || -> Result<String, RelationError> {
        map.get("repo")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| schema(format!("legacy {kind} dep: repo must be a string")))
    };
    let external =
        |resource_kind: ResourceKind, number_key: &str| -> Result<EdgeEnd, RelationError> {
            Ok(EdgeEnd::External(ExternalRef {
                provider: Provider::Github,
                repository: repository()?,
                resource_kind,
                number: integer(number_key)?,
                node_id: None,
            }))
        };
    let end = match kind {
        "task" => EdgeEnd::FleetWork(WorkRef {
            authority_id: LOCAL_AUTHORITY_ID.to_string(),
            work_id: integer("task_id")?,
        }),
        "issue" => external(ResourceKind::Issue, "number")?,
        "pr" => external(ResourceKind::PullRequest, "number")?,
        "ci" => external(ResourceKind::ActionsRun, "run_id")?,
        other => {
            return Err(schema(format!(
                "legacy dep: kind {other:?} is not task, issue, pr or ci"
            )));
        }
    };
    end.validate()?;
    Ok(end)
}

// ---------------------------------------------------------------------------
// Wire helpers
// ---------------------------------------------------------------------------

/// The object at `field`, refusing unknown and missing keys.
fn object<'a>(
    field: &str,
    value: &'a Value,
    allowed: &[&str],
) -> Result<&'a Map<String, Value>, RelationError> {
    let map = value
        .as_object()
        .ok_or_else(|| schema(format!("{field}: must be an object")))?;
    if let Some(unknown) = map.keys().find(|key| !allowed.contains(&key.as_str())) {
        return Err(schema(format!("{field}: unknown field {unknown:?}")));
    }
    if let Some(missing) = allowed.iter().find(|key| !map.contains_key(**key)) {
        return Err(schema(format!("{field}: missing field {missing:?}")));
    }
    Ok(map)
}

fn string(field: &str, map: &Map<String, Value>, key: &str) -> Result<String, RelationError> {
    match map.get(key) {
        Some(Value::String(text)) => Ok(text.clone()),
        _ => Err(schema(format!("{field}.{key}: must be a string"))),
    }
}

fn nullable_string(
    field: &str,
    map: &Map<String, Value>,
    key: &str,
) -> Result<Option<String>, RelationError> {
    match map.get(key) {
        Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        _ => Err(schema(format!("{field}.{key}: must be a string or null"))),
    }
}

fn nullable(value: &Option<String>) -> Value {
    value
        .as_ref()
        .map_or(Value::Null, |text| Value::from(text.clone()))
}

fn role_value(role: Option<DuplicateRole>) -> Value {
    role.map_or(Value::Null, |role| Value::from(role.as_str()))
}

fn role_field(field: &str, value: &Value) -> Result<Option<DuplicateRole>, RelationError> {
    match value {
        Value::Null => Ok(None),
        Value::String(text) => DuplicateRole::parse_field(field, text).map(Some),
        _ => Err(schema(format!("{field}: must be a string or null"))),
    }
}

fn require_decimal(field: &str, value: &str) -> Result<(), RelationError> {
    if is_canonical_decimal(value) {
        Ok(())
    } else {
        Err(schema(format!(
            "{field}: {value:?} is not a canonical decimal string"
        )))
    }
}

fn require_revision(field: &str, value: &str) -> Result<(), RelationError> {
    require_decimal(field, value)?;
    if value == "0" {
        return Err(schema(format!("{field}: revisions start at 1")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn work(id: &str) -> EdgeEnd {
        EdgeEnd::FleetWork(WorkRef::new(LOCAL_AUTHORITY_ID, id).unwrap())
    }

    fn remote_work(authority: &str, id: &str) -> EdgeEnd {
        EdgeEnd::FleetWork(WorkRef::new(authority, id).unwrap())
    }

    fn issue(repository: &str, number: &str, node_id: Option<&str>) -> EdgeEnd {
        EdgeEnd::External(ExternalRef {
            provider: Provider::Github,
            repository: repository.to_string(),
            resource_kind: ResourceKind::Issue,
            number: number.to_string(),
            node_id: node_id.map(str::to_string),
        })
    }

    fn edge(relation: Relation, source: EdgeEnd, target: EdgeEnd) -> Edge {
        let source_key = source.key();
        Edge {
            edge_id: "7".to_string(),
            revision: "1".to_string(),
            relation,
            role: None,
            source,
            target,
            source_key,
            hardness: Hardness::Soft,
            status: EdgeStatus::Active,
            reason_code: EdgeOrigin::Explicit,
            certainty: Certainty::Asserted,
            provenance: Provenance {
                principal_ref: "principal:test".to_string(),
                actor_ref: Some("agent:test".to_string()),
                session_ref: None,
                operation_id: Some("op-1".to_string()),
                source_ref: None,
            },
            created_at: "2026-11-26T03:00:00Z".to_string(),
            updated_at: "2026-11-26T03:00:00Z".to_string(),
            resolution: None,
        }
    }

    fn resolved(reason: &str) -> Option<Resolution> {
        Some(Resolution {
            reason: reason.to_string(),
            evidence_ref: None,
            override_: false,
            expires_at: None,
            successor_edge_id: None,
        })
    }

    #[test]
    fn pmo_dep_wire_rejects_noncanonical_pair_order() {
        // Oracle: DEP-02 D1 pair order; numeric WorkRef ordering, not text ordering.
        // Both JSON ingestion and direct validation must enforce the same invariant.
        for relation in [Relation::RelatedTo, Relation::Duplicates] {
            let reversed = edge(relation, work("10"), work("2"));
            assert_eq!(reversed.validate().unwrap_err().code(), "invalid_schema");
            assert_eq!(
                Edge::from_json(&reversed.to_json()).unwrap_err().code(),
                "invalid_schema"
            );
            let canonical = edge(relation, work("2"), work("10"));
            assert_eq!(Edge::from_json(&canonical.to_json()).unwrap(), canonical);
            // Authority sorts before numeric work id.
            assert!(
                edge(relation, remote_work("a", "10"), remote_work("z", "2"))
                    .validate()
                    .is_ok()
            );
            assert!(
                edge(relation, remote_work("z", "2"), remote_work("a", "10"))
                    .validate()
                    .is_err()
            );
            // An external end belongs in source, regardless of lexical key.
            assert!(
                edge(relation, issue("z/repo", "2", Some("I_2")), work("10"))
                    .validate()
                    .is_ok()
            );
        }
        // Directed relations must retain their meaning, not adopt pair ordering.
        for relation in [Relation::Blocks, Relation::Hierarchy, Relation::Supersedes] {
            assert!(edge(relation, work("10"), work("2")).validate().is_ok());
        }
    }

    proptest! {
        // Oracle: every distinct pair has exactly one valid orientation under
        // WorkRef::Ord; reversing it must fail without silently swapping roles.
        #[test]
        fn pmo_dep_wire_pair_order_is_canonical(a in 1u64..u64::MAX, b in 1u64..u64::MAX) {
            prop_assume!(a != b);
            let (low, high) = (a.min(b).to_string(), a.max(b).to_string());
            for relation in [Relation::RelatedTo, Relation::Duplicates] {
                let canonical = edge(relation, work(&low), work(&high));
                prop_assert_eq!(Edge::from_json(&canonical.to_json()).unwrap(), canonical);
                let mut reversed = edge(relation, work(&high), work(&low));
                reversed.role = (relation == Relation::Duplicates).then_some(DuplicateRole::SourceIsDuplicate);
                prop_assert_eq!(Edge::from_json(&reversed.to_json()).unwrap_err().code(), "invalid_schema");
            }
        }
    }

    #[test]
    fn pmo_dep_aliases_normalize_to_one_canonical_edge() {
        let a = work("761");
        let b = work("842");
        let ext = issue("developing-today/code", "761", Some("I_1"));
        for spelling in RelationSpelling::ALL {
            let Some(inverse) = spelling.inverse() else {
                continue;
            };
            assert_eq!(
                normalize(&a, *spelling, &b).unwrap(),
                normalize(&b, inverse, &a).unwrap(),
                "{} and its inverse {}",
                spelling.as_str(),
                inverse.as_str()
            );
        }
        // The slice's negative: B blocked_by A is the same fact as A
        // blocks B, so the store's unique key sees one edge.
        let blocks = normalize(&ext, RelationSpelling::Blocks, &b).unwrap();
        let blocked_by = normalize(&b, RelationSpelling::BlockedBy, &ext).unwrap();
        assert_eq!(blocks, blocked_by);
        assert_eq!(blocks.relation, Relation::Blocks);
        assert_eq!(blocks.source, ext);
        assert_eq!(blocks.target, b);

        let child_of = normalize(&b, RelationSpelling::ChildOf, &a).unwrap();
        assert_eq!(child_of.relation, Relation::Hierarchy);
        assert_eq!((child_of.source, child_of.target), (a.clone(), b.clone()));

        let supersedes = normalize(&b, RelationSpelling::Supersedes, &a).unwrap();
        assert_eq!(supersedes.relation, Relation::Supersedes);
        assert_eq!(
            (supersedes.source, supersedes.target),
            (b.clone(), a.clone())
        );

        // Pair order: the smaller WorkRef is the source; Fleet work is
        // the target when only one end is Fleet work.
        let related = normalize(&b, RelationSpelling::RelatedTo, &a).unwrap();
        assert_eq!((related.source, related.target), (a.clone(), b.clone()));
        assert_eq!(related.role, None);
        let related_ext = normalize(&b, RelationSpelling::RelatedTo, &ext).unwrap();
        assert_eq!(
            (related_ext.source, related_ext.target),
            (ext.clone(), b.clone())
        );
        // Numeric, not lexical, WorkRef order: 9 < 10.
        let nine_ten = normalize(&work("10"), RelationSpelling::RelatedTo, &work("9")).unwrap();
        assert_eq!(nine_ten.source, work("9"));

        // duplicates keeps the stated copy in role, whichever end
        // ordering made the source.
        let a_copy = normalize(&a, RelationSpelling::Duplicates, &b).unwrap();
        let b_copy = normalize(&b, RelationSpelling::Duplicates, &a).unwrap();
        assert_eq!((&a_copy.source, &a_copy.target), (&a, &b));
        assert_eq!((&b_copy.source, &b_copy.target), (&a, &b));
        assert_eq!(a_copy.role, Some(DuplicateRole::SourceIsDuplicate));
        assert_eq!(b_copy.role, Some(DuplicateRole::TargetIsDuplicate));
        for spelling in RelationSpelling::ALL {
            if *spelling != RelationSpelling::Duplicates {
                assert_eq!(normalize(&a, *spelling, &b).unwrap().role, None);
            }
        }

        assert_eq!(
            normalize(&a, RelationSpelling::Blocks, &work("761"))
                .unwrap_err()
                .code(),
            "self_edge"
        );
    }

    const REVISION_WIRE: &str = r#"{"schema":"fleet.dependency_edge_revision/v1","edge_id":"7","revision":"3","operation_id":"op-3","before":{"status":"active","hardness":"hard","certainty":"verified","role":null,"resolution":null},"after":{"status":"resolved","hardness":"hard","certainty":"verified","role":null,"resolution":{"reason":"blocker_satisfied","evidence_ref":"evidence:761-closed","override":false,"expires_at":null,"successor_edge_id":null}}}"#;

    const EDGE_WIRE: &str = r#"{"schema":"fleet.dependency_edge/v1","edge_id":"7","revision":"3","relation":"blocks","role":null,"source":{"kind":"external","provider":"github","repository":"developing-today/code","resource_kind":"issue","number":"761","node_id":"I_kwDOexample"},"target":{"kind":"fleet_work","work":{"authority_id":"local","work_id":"842"}},"source_key":"issue:developing-today/code#761","hardness":"hard","status":"resolved","reason_code":"explicit","certainty":"verified","provenance":{"principal_ref":"principal:owner","actor_ref":"agent:lane","session_ref":"session:1","operation_id":"op-3","source_ref":null},"created_at":"2026-11-26T03:00:00Z","updated_at":"2026-11-26T04:00:00Z","resolution":{"reason":"blocker_satisfied","evidence_ref":"evidence:761-closed","override":false,"expires_at":null,"successor_edge_id":null}}"#;

    #[test]
    fn pmo_dep_edge_wire_preserves_before_after() {
        let revision = EdgeRevision::from_json(REVISION_WIRE).unwrap();
        assert_eq!(revision.to_json(), REVISION_WIRE);
        let before = revision.before.as_ref().unwrap();
        assert_eq!(before.status, EdgeStatus::Active);
        assert_eq!(before.resolution, None);
        assert_eq!(revision.after.status, EdgeStatus::Resolved);
        assert_eq!(
            revision.after.resolution.as_ref().unwrap().reason,
            "blocker_satisfied"
        );
        assert_eq!(revision.revision, "3");

        let edge = Edge::from_json(EDGE_WIRE).unwrap();
        assert_eq!(edge.to_json(), EDGE_WIRE);
        assert_eq!(edge.edge_id, "7");

        // Versions are strings on the wire, never numbers.
        let numeric = REVISION_WIRE.replace(r#""revision":"3""#, r#""revision":3"#);
        assert_eq!(
            EdgeRevision::from_json(&numeric).unwrap_err().code(),
            "invalid_schema"
        );
        let numeric_edge = EDGE_WIRE.replace(r#""edge_id":"7""#, r#""edge_id":7"#);
        assert_eq!(
            Edge::from_json(&numeric_edge).unwrap_err().code(),
            "invalid_schema"
        );
        let padded = REVISION_WIRE.replace(r#""revision":"3""#, r#""revision":"03""#);
        assert_eq!(
            EdgeRevision::from_json(&padded).unwrap_err().code(),
            "invalid_schema"
        );

        // Unknown fields are refused at every level.
        for (text, needle) in [
            (REVISION_WIRE, r#""operation_id":"op-3","#),
            (REVISION_WIRE, r#""before":{"#),
            (EDGE_WIRE, r#""edge_id":"7","#),
            (EDGE_WIRE, r#""provenance":{"#),
            (EDGE_WIRE, r#""target":{"#),
        ] {
            let injected = text.replacen(needle, &format!("{needle}\"extra\":1,"), 1);
            assert_ne!(injected, text);
            let error = if text == REVISION_WIRE {
                EdgeRevision::from_json(&injected).unwrap_err()
            } else {
                Edge::from_json(&injected).unwrap_err()
            };
            assert_eq!(error.code(), "invalid_schema", "{injected}");
            assert!(error.to_string().contains("extra"), "{error}");
        }

        // before is null exactly at revision 1.
        let first_with_before = REVISION_WIRE.replace(r#""revision":"3""#, r#""revision":"1""#);
        assert!(EdgeRevision::from_json(&first_with_before).is_err());
        let later_without_before = REVISION_WIRE.replace(
            r#""before":{"status":"active","hardness":"hard","certainty":"verified","role":null,"resolution":null}"#,
            r#""before":null"#,
        );
        assert!(EdgeRevision::from_json(&later_without_before).is_err());

        // revise builds the same before/after from two edge records.
        let mut active = edge.clone();
        active.revision = "2".to_string();
        active.status = EdgeStatus::Active;
        active.resolution = None;
        let built = revise(&active, &edge, "op-3").unwrap();
        assert_eq!(built.before.as_ref().unwrap().status, EdgeStatus::Active);
        assert_eq!(built.after, edge.state());
        assert_eq!(EdgeRevision::from_json(&built.to_json()).unwrap(), built);
    }

    #[test]
    fn pmo_dep_same_number_across_repos_is_two_ends() {
        let x = issue("a/x", "42", Some("I_x"));
        let y = issue("a/y", "42", Some("I_y"));
        assert_eq!(x.key(), "issue:a/x#42");
        assert_eq!(y.key(), "issue:a/y#42");
        assert_ne!(x.key(), y.key());
        assert_ne!(x, y);
        let target = work("842");
        let ex = normalize(&x, RelationSpelling::Blocks, &target).unwrap();
        let ey = normalize(&y, RelationSpelling::Blocks, &target).unwrap();
        assert_ne!(ex, ey);
        // A local work item numbered 42 is a third end.
        assert_eq!(work("42").key(), "task:42");
        assert_ne!(work("42").key(), x.key());
        // So is the same work id in another authority.
        assert_eq!(remote_work("peer", "42").key(), "work:peer/42");
        // An issue and a pull request with the same number differ too.
        let pr = EdgeEnd::External(ExternalRef {
            provider: Provider::Github,
            repository: "a/x".to_string(),
            resource_kind: ResourceKind::PullRequest,
            number: "42".to_string(),
            node_id: Some("PR_x".to_string()),
        });
        assert_eq!(pr.key(), "pr:a/x#42");
        assert_ne!(pr.key(), x.key());
    }

    #[test]
    fn pmo_dep_unknown_enum_is_refused_not_aliased() {
        for near_miss in [
            "superseded_by",
            "duplicated_by",
            "depends_on",
            "blocking",
            "Blocks",
            "BLOCKED_BY",
            "blocked-by",
            "hierarchy",
            "",
        ] {
            let error = RelationSpelling::parse(near_miss).unwrap_err();
            assert_eq!(error.code(), "unknown_relation", "{near_miss:?}");
        }
        for spelling in RelationSpelling::ALL {
            assert_eq!(
                RelationSpelling::parse(spelling.as_str()).unwrap(),
                *spelling
            );
        }

        // Every other closed enum is invalid_schema naming the field.
        let cases: [(&str, &str, &str); 6] = [
            (
                r#""relation":"blocks""#,
                r#""relation":"blocked_by""#,
                "relation",
            ),
            (r#""hardness":"hard""#, r#""hardness":"Hard""#, "hardness"),
            (r#""status":"resolved""#, r#""status":"closed""#, "status"),
            (
                r#""reason_code":"explicit""#,
                r#""reason_code":"manual""#,
                "reason_code",
            ),
            (
                r#""certainty":"verified""#,
                r#""certainty":"certain""#,
                "certainty",
            ),
            (
                r#""resource_kind":"issue""#,
                r#""resource_kind":"pr""#,
                "resource_kind",
            ),
        ];
        for (from, to, field) in cases {
            let text = EDGE_WIRE.replace(from, to);
            assert_ne!(text, EDGE_WIRE, "{from}");
            let error = Edge::from_json(&text).unwrap_err();
            assert_eq!(error.code(), "invalid_schema", "{to}");
            assert!(error.to_string().contains(field), "{error}");
        }
        let role = EDGE_WIRE.replace(r#""role":null"#, r#""role":"duplicate""#);
        let error = Edge::from_json(&role).unwrap_err();
        assert_eq!(error.code(), "invalid_schema");
        assert!(error.to_string().contains("role"), "{error}");
        let provider = EDGE_WIRE.replace(r#""provider":"github""#, r#""provider":"GitHub""#);
        let error = Edge::from_json(&provider).unwrap_err();
        assert_eq!(error.code(), "invalid_schema");
        assert!(error.to_string().contains("provider"), "{error}");
        let kind = EDGE_WIRE.replace(r#""kind":"fleet_work""#, r#""kind":"work""#);
        assert_eq!(Edge::from_json(&kind).unwrap_err().code(), "invalid_schema");
        let state = REVISION_WIRE.replace(r#""status":"resolved""#, r#""status":"done""#);
        let error = EdgeRevision::from_json(&state).unwrap_err();
        assert_eq!(error.code(), "invalid_schema");
        assert!(error.to_string().contains("after.status"), "{error}");
    }

    #[test]
    fn pmo_dep_shorthand_needs_one_authority() {
        for text in ["fleet:842", "fleet://work/842"] {
            let input = parse_ref(text).unwrap();
            assert_eq!(
                input,
                RefInput::LocalShorthand {
                    work_id: "842".to_string()
                }
            );
            let one = BindContext {
                authorities: vec!["local".to_string()],
                ..BindContext::default()
            };
            assert_eq!(bind_ref(&input, &one).unwrap(), work("842"));
            let none = BindContext::default();
            let error = bind_ref(&input, &none).unwrap_err();
            assert_eq!(error.code(), "ambiguous_ref");
            assert!(
                error
                    .to_string()
                    .contains("fleet://<authority_id>/work/842")
            );
            let two = BindContext {
                authorities: vec!["local".to_string(), "peer".to_string()],
                ..BindContext::default()
            };
            assert_eq!(bind_ref(&input, &two).unwrap_err().code(), "ambiguous_ref");
        }
        // The qualified form always binds, whatever the context.
        let qualified = parse_ref("fleet://peer/work/842").unwrap();
        for context in [
            BindContext::default(),
            BindContext {
                authorities: vec!["local".to_string(), "peer".to_string()],
                ..BindContext::default()
            },
        ] {
            assert_eq!(
                bind_ref(&qualified, &context).unwrap(),
                remote_work("peer", "842")
            );
        }
        // #N needs exactly one linked repository.
        let hash = parse_ref("#761").unwrap();
        let linked = BindContext {
            linked_repositories: vec!["developing-today/code".to_string()],
            resolved_externals: vec![ExternalRef {
                provider: Provider::Github,
                repository: "developing-today/code".to_string(),
                resource_kind: ResourceKind::Issue,
                number: "761".to_string(),
                node_id: Some("I_761".to_string()),
            }],
            ..BindContext::default()
        };
        assert_eq!(
            bind_ref(&hash, &linked).unwrap(),
            issue("developing-today/code", "761", Some("I_761"))
        );
        let two_repos = BindContext {
            linked_repositories: vec!["a/x".to_string(), "a/y".to_string()],
            ..BindContext::default()
        };
        assert_eq!(
            bind_ref(&hash, &two_repos).unwrap_err().code(),
            "ambiguous_ref"
        );
        assert_eq!(
            bind_ref(&hash, &BindContext::default()).unwrap_err().code(),
            "ambiguous_ref"
        );
        // Typed and untyped external forms.
        assert_eq!(
            parse_ref("issue:a/x#42"),
            Some(RefInput::External {
                resource_kind: Some(ResourceKind::Issue),
                repository: "a/x".to_string(),
                number: "42".to_string()
            })
        );
        assert_eq!(
            parse_ref("ci:a/x#12345"),
            Some(RefInput::External {
                resource_kind: Some(ResourceKind::ActionsRun),
                repository: "a/x".to_string(),
                number: "12345".to_string()
            })
        );
        let untyped = parse_ref("a/x#42").unwrap();
        assert_eq!(
            bind_ref(&untyped, &BindContext::default())
                .unwrap_err()
                .code(),
            "unresolved_ref"
        );
        // A bare number and malformed text match no form; JSON callers
        // name that invalid_schema.
        for text in [
            "842",
            "fleet:0842",
            "fleet://",
            "#",
            "a#1",
            "x/y/z#1",
            "issue:a/x#-1",
        ] {
            assert_eq!(parse_ref(text), None, "{text:?}");
        }
        assert_eq!(
            parse_ref_field("blocked_by", "842").unwrap_err().code(),
            "invalid_schema"
        );
    }

    #[test]
    fn pmo_dep_unresolved_external_ref_is_never_explicit() {
        let unresolved = issue("a/x", "42", None);
        let mut explicit = edge(Relation::Blocks, unresolved.clone(), work("842"));
        for certainty in Certainty::ALL {
            explicit.certainty = *certainty;
            assert_eq!(
                explicit.validate().unwrap_err().code(),
                "unresolved_ref",
                "{}",
                certainty.as_str()
            );
        }
        for origin in [EdgeOrigin::Imported, EdgeOrigin::Migrated] {
            let mut imported = edge(Relation::Blocks, unresolved.clone(), work("842"));
            imported.reason_code = origin;
            imported.certainty = Certainty::Uncertain;
            imported.validate().unwrap();
            for certainty in [Certainty::Verified, Certainty::Asserted] {
                imported.certainty = certainty;
                assert_eq!(imported.validate().unwrap_err().code(), "unresolved_ref");
            }
        }
        let mut inferred = edge(Relation::Blocks, unresolved, work("842"));
        inferred.reason_code = EdgeOrigin::InferredPendingReview;
        inferred.certainty = Certainty::Uncertain;
        assert_eq!(inferred.validate().unwrap_err().code(), "unresolved_ref");

        // With a node id, an explicit edge validates, verified included.
        let mut resolved_end = edge(
            Relation::Blocks,
            issue("a/x", "42", Some("I_x")),
            work("842"),
        );
        resolved_end.certainty = Certainty::Verified;
        resolved_end.validate().unwrap();

        // A typed ref the lookup did not answer binds without a node id,
        // and an explicit edge built from it is refused.
        let bound = bind_ref(&parse_ref("issue:a/x#42").unwrap(), &BindContext::default()).unwrap();
        let explicit = edge(Relation::Blocks, bound, work("842"));
        assert_eq!(explicit.validate().unwrap_err().code(), "unresolved_ref");
    }

    #[test]
    fn pmo_dep_only_blocks_can_be_hard() {
        let mut hard_blocks = edge(Relation::Blocks, work("1"), work("2"));
        hard_blocks.hardness = Hardness::Hard;
        hard_blocks.validate().unwrap();
        for relation in [
            Relation::RelatedTo,
            Relation::Hierarchy,
            Relation::Duplicates,
            Relation::Supersedes,
        ] {
            let mut advisory = edge(relation, work("1"), work("2"));
            advisory.validate().unwrap();
            advisory.hardness = Hardness::Hard;
            let error = advisory.validate().unwrap_err();
            assert_eq!(error.code(), "relation_not_gating", "{}", relation.as_str());
            assert_eq!(error, RelationError::RelationNotGating { relation });
        }
    }

    #[test]
    fn pmo_dep_target_is_always_fleet_work() {
        let ext = issue("a/x", "1", Some("I_1"));
        let other = issue("a/y", "2", Some("I_2"));
        let item = work("842");
        for spelling in [
            RelationSpelling::Blocks,
            RelationSpelling::ParentOf,
            RelationSpelling::Supersedes,
        ] {
            assert_eq!(
                normalize(&item, spelling, &ext).unwrap_err().code(),
                "target_not_fleet_work",
                "{}",
                spelling.as_str()
            );
        }
        for spelling in [RelationSpelling::BlockedBy, RelationSpelling::ChildOf] {
            assert_eq!(
                normalize(&ext, spelling, &item).unwrap_err().code(),
                "target_not_fleet_work"
            );
        }
        for spelling in RelationSpelling::ALL {
            assert_eq!(
                normalize(&ext, *spelling, &other).unwrap_err().code(),
                "target_not_fleet_work",
                "external to external, {}",
                spelling.as_str()
            );
        }
        // Undirected relations put the Fleet work end at the target.
        for spelling in [RelationSpelling::RelatedTo, RelationSpelling::Duplicates] {
            let canonical = normalize(&item, spelling, &ext).unwrap();
            assert_eq!(canonical.target, item);
        }
        // And a stored edge with an external target is refused.
        let mut stored = edge(Relation::Blocks, item.clone(), ext.clone());
        stored.certainty = Certainty::Verified;
        assert_eq!(
            stored.validate().unwrap_err().code(),
            "target_not_fleet_work"
        );
    }

    #[test]
    fn pmo_dep_transitions_are_closed() {
        let mut from_states: Vec<Option<EdgeStatus>> = vec![None];
        from_states.extend(EdgeStatus::ALL.iter().copied().map(Some));
        let mut accepted = 0;
        for from in &from_states {
            for to in EdgeStatus::ALL {
                let allowed = ALLOWED_TRANSITIONS.contains(&(*from, *to));
                match transition(*from, *to) {
                    Ok(()) => {
                        assert!(allowed);
                        accepted += 1;
                    }
                    Err(error) => {
                        assert!(!allowed);
                        assert_eq!(error.code(), "invalid_transition");
                    }
                }
            }
        }
        assert_eq!(accepted, 8);
        assert_eq!(ALLOWED_TRANSITIONS.len(), 8);
        // removed is final, whatever comes next.
        for to in EdgeStatus::ALL {
            assert!(transition(Some(EdgeStatus::Removed), *to).is_err());
        }
        assert!(transition(None, EdgeStatus::Resolved).is_err());
        assert!(transition(Some(EdgeStatus::Resolved), EdgeStatus::Removed).is_err());

        // revise applies the table.
        let active = edge(Relation::Blocks, work("1"), work("2"));
        let mut removed = active.clone();
        removed.revision = "2".to_string();
        removed.status = EdgeStatus::Removed;
        removed.resolution = resolved("dropped");
        let step = revise(&active, &removed, "op-2").unwrap();
        assert_eq!(step.after.status, EdgeStatus::Removed);
        let mut back = removed.clone();
        back.revision = "3".to_string();
        back.status = EdgeStatus::Active;
        back.resolution = None;
        assert_eq!(
            revise(&removed, &back, "op-3").unwrap_err().code(),
            "invalid_transition"
        );
        // An override is resolved-only.
        let mut overridden = removed.clone();
        overridden.resolution.as_mut().unwrap().override_ = true;
        assert_eq!(overridden.validate().unwrap_err().code(), "invalid_schema");
        // The first revision is an add: active or quarantined only.
        first_revision(&active, "op-1").unwrap();
        let mut first_resolved = active.clone();
        first_resolved.status = EdgeStatus::Resolved;
        first_resolved.resolution = resolved("blocker_satisfied");
        assert_eq!(
            first_revision(&first_resolved, "op-1").unwrap_err().code(),
            "invalid_transition"
        );
        // Revisions count up by exactly one, at any length.
        assert_eq!(increment_decimal("1"), "2");
        assert_eq!(increment_decimal("9"), "10");
        assert_eq!(increment_decimal("199"), "200");
        assert_eq!(
            increment_decimal("99999999999999999999"),
            "100000000000000000000"
        );
        let mut skipped = removed.clone();
        skipped.revision = "3".to_string();
        assert_eq!(
            revise(&active, &skipped, "op-3").unwrap_err().code(),
            "invalid_transition"
        );
    }

    #[test]
    fn pmo_dep_identity_never_changes_across_revisions() {
        let mut base = edge(Relation::Duplicates, work("1"), work("2"));
        base.role = Some(DuplicateRole::SourceIsDuplicate);
        base.validate().unwrap();
        let next = |change: &dyn Fn(&mut Edge)| {
            let mut edge = base.clone();
            edge.revision = "2".to_string();
            change(&mut edge);
            edge
        };
        // A change of certainty alone is a valid revision.
        let certain = next(&|e: &mut Edge| e.certainty = Certainty::Verified);
        revise(&base, &certain, "op-2").unwrap();

        let changes: Vec<Box<dyn Fn(&mut Edge)>> = vec![
            Box::new(|e: &mut Edge| {
                e.relation = Relation::RelatedTo;
                e.role = None;
            }),
            Box::new(|e: &mut Edge| e.role = Some(DuplicateRole::TargetIsDuplicate)),
            Box::new(|e: &mut Edge| e.role = None),
            Box::new(|e: &mut Edge| {
                // Keep pair order valid so this isolates identity mutation,
                // rather than failing the independent schema invariant first.
                e.source = remote_work("alpha", "3");
                e.source_key = e.source.key();
            }),
            Box::new(|e: &mut Edge| e.target = work("4")),
            Box::new(|e: &mut Edge| e.edge_id = "8".to_string()),
        ];
        for change in &changes {
            let changed = next(&**change);
            changed
                .validate()
                .expect("identity fixture must remain a valid edge");
            assert_eq!(
                revise(&base, &changed, "op-2").unwrap_err().code(),
                "invalid_transition",
                "{changed:?}"
            );
        }
        // A role change inside a revision record is refused too.
        let text = REVISION_WIRE.replacen(r#""role":null"#, r#""role":"source_is_duplicate""#, 1);
        assert_eq!(
            EdgeRevision::from_json(&text).unwrap_err().code(),
            "invalid_transition"
        );

        // An external end's provider id may be filled in, never changed.
        let mut imported = edge(Relation::Blocks, issue("a/x", "5", None), work("9"));
        imported.reason_code = EdgeOrigin::Imported;
        imported.certainty = Certainty::Uncertain;
        let mut filled = imported.clone();
        filled.revision = "2".to_string();
        filled.source = issue("a/x", "5", Some("I_5"));
        revise(&imported, &filled, "op-2").unwrap();
        let mut renamed = filled.clone();
        renamed.revision = "3".to_string();
        renamed.source = issue("a/x", "5", Some("I_other"));
        assert_eq!(
            revise(&filled, &renamed, "op-3").unwrap_err().code(),
            "invalid_transition"
        );
    }

    #[test]
    fn pmo_dep_perspective_preserves_meaning() {
        let a = work("1");
        let b = work("2");
        let ext = issue("a/x", "7", Some("I_7"));
        let read = |edge: &CanonicalEdge, from: &EdgeEnd| {
            let view = describe(edge, from).unwrap();
            (view.spelling, view.other_end)
        };
        let blocks = normalize(&ext, RelationSpelling::Blocks, &a).unwrap();
        assert_eq!(
            read(&blocks, &a),
            (PerspectiveSpelling::BlockedBy, ext.clone())
        );
        assert_eq!(
            read(&blocks, &ext),
            (PerspectiveSpelling::Blocks, a.clone())
        );
        let hierarchy = normalize(&a, RelationSpelling::ParentOf, &b).unwrap();
        assert_eq!(
            read(&hierarchy, &a),
            (PerspectiveSpelling::ParentOf, b.clone())
        );
        assert_eq!(
            read(&hierarchy, &b),
            (PerspectiveSpelling::ChildOf, a.clone())
        );
        let supersedes = normalize(&b, RelationSpelling::Supersedes, &a).unwrap();
        assert_eq!(
            read(&supersedes, &b),
            (PerspectiveSpelling::Supersedes, a.clone())
        );
        assert_eq!(
            read(&supersedes, &a),
            (PerspectiveSpelling::SupersededBy, b.clone())
        );
        let related = normalize(&b, RelationSpelling::RelatedTo, &a).unwrap();
        assert_eq!(read(&related, &a).0, PerspectiveSpelling::RelatedTo);
        assert_eq!(read(&related, &b).0, PerspectiveSpelling::RelatedTo);
        let duplicates = normalize(&b, RelationSpelling::Duplicates, &a).unwrap();
        assert_eq!(
            read(&duplicates, &b),
            (PerspectiveSpelling::Duplicates, a.clone())
        );
        assert_eq!(
            read(&duplicates, &a),
            (PerspectiveSpelling::DuplicatedBy, b.clone())
        );
        let unknown_copy = CanonicalEdge {
            role: None,
            ..duplicates.clone()
        };
        assert_eq!(read(&unknown_copy, &a).0, PerspectiveSpelling::Duplicates);
        assert_eq!(read(&unknown_copy, &b).0, PerspectiveSpelling::Duplicates);
        // A stranger reads nothing.
        assert_eq!(describe(&blocks, &work("99")), None);
        // Display-only words are never input spellings.
        assert!(RelationSpelling::parse(PerspectiveSpelling::SupersededBy.as_str()).is_err());
        assert!(RelationSpelling::parse(PerspectiveSpelling::DuplicatedBy.as_str()).is_err());
        let wire = describe(&blocks, &a).unwrap().to_json_value();
        assert_eq!(wire["spelling"], "blocked_by");
        assert_eq!(wire["other_end"]["kind"], "external");
        // Every stated spelling reads back from its subject.
        for spelling in RelationSpelling::ALL {
            let canonical = normalize(&b, *spelling, &a).unwrap();
            let view = describe(&canonical, &b).unwrap();
            assert_eq!(view.spelling.as_str(), spelling.as_str());
            assert_eq!(view.other_end, a);
        }
    }

    #[test]
    fn pmo_dep_legacy_keys_match_dep_key() {
        let deps = [
            (
                serde_json::json!({"kind": "pr", "repo": "developing-today/code", "number": 3931}),
                "pr:developing-today/code#3931",
            ),
            (
                serde_json::json!({"kind": "issue", "repo": "developing-today/code", "number": 3901}),
                "issue:developing-today/code#3901",
            ),
            (
                serde_json::json!({"kind": "task", "task_id": 5865}),
                "task:5865",
            ),
            (
                serde_json::json!({"kind": "ci", "repo": "developing-today/code", "run_id": 12345}),
                "ci:developing-today/code#12345",
            ),
        ];
        for (dep, expected) in &deps {
            let end = from_legacy_dep(dep).unwrap();
            assert_eq!(end.key(), *expected);
            #[cfg(feature = "queue-broker")]
            assert_eq!(end.key(), crate::turso_queue::dep_key(dep));
            // Legacy ends carry no provider id.
            assert!(!matches!(
                end,
                EdgeEnd::External(ExternalRef {
                    node_id: Some(_),
                    ..
                })
            ));
        }
        // An edge from a legacy end is imported and uncertain.
        let legacy = from_legacy_dep(&deps[1].0).unwrap();
        let mut imported = edge(Relation::Blocks, legacy, work("842"));
        imported.reason_code = EdgeOrigin::Imported;
        imported.certainty = Certainty::Uncertain;
        imported.validate().unwrap();
        for broken in [
            serde_json::json!({"kind": "label", "name": "x"}),
            serde_json::json!({"kind": "issue", "number": 1}),
            serde_json::json!({"kind": "task"}),
            serde_json::json!({"kind": "pr", "repo": "a/x", "number": -1}),
            serde_json::json!("issue:a/x#1"),
        ] {
            assert_eq!(
                from_legacy_dep(&broken).unwrap_err().code(),
                "invalid_schema",
                "{broken}"
            );
        }
    }

    #[test]
    fn pmo_dep_error_codes_are_stable() {
        let detail = || "x".to_string();
        let table: Vec<(RelationError, &str, &str)> = vec![
            (
                RelationError::UnknownRelation { spelling: detail() },
                "unknown_relation",
                "rejected",
            ),
            (
                RelationError::InvalidSchema { detail: detail() },
                "invalid_schema",
                "rejected",
            ),
            (
                RelationError::SelfEdge { key: detail() },
                "self_edge",
                "rejected",
            ),
            (
                RelationError::TargetNotFleetWork { detail: detail() },
                "target_not_fleet_work",
                "rejected",
            ),
            (
                RelationError::RelationNotGating {
                    relation: Relation::RelatedTo,
                },
                "relation_not_gating",
                "rejected",
            ),
            (
                RelationError::AmbiguousRef { detail: detail() },
                "ambiguous_ref",
                "rejected",
            ),
            (
                RelationError::UnresolvedRef { detail: detail() },
                "unresolved_ref",
                "rejected",
            ),
            (
                RelationError::InvalidTransition { detail: detail() },
                "invalid_transition",
                "rejected",
            ),
            (
                RelationError::DuplicateEdge { detail: detail() },
                "duplicate_edge",
                "conflict",
            ),
            (
                RelationError::RelationRoleConflict { detail: detail() },
                "relation_role_conflict",
                "rejected",
            ),
            (
                RelationError::DependencyCycle { detail: detail() },
                "dependency_cycle",
                "rejected",
            ),
            (
                RelationError::HierarchyCycle { detail: detail() },
                "hierarchy_cycle",
                "rejected",
            ),
            (
                RelationError::SupersedesCycle { detail: detail() },
                "supersedes_cycle",
                "rejected",
            ),
            (
                RelationError::GraphEvaluationIncomplete { detail: detail() },
                "graph_evaluation_incomplete",
                "rejected",
            ),
            (
                RelationError::UnexpectedActiveCycle { detail: detail() },
                "unexpected_active_cycle",
                "rejected",
            ),
            (
                RelationError::VersionConflict { detail: detail() },
                "VERSION_CONFLICT",
                "conflict",
            ),
        ];
        assert_eq!(table.len(), 16);
        let mut seen = std::collections::BTreeSet::new();
        for (error, code, outcome) in &table {
            assert_eq!(error.code(), *code);
            assert_eq!(error.outcome().as_str(), *outcome);
            assert!(seen.insert(*code), "{code} listed twice");
            let wire = ResultError::from(error);
            assert_eq!(wire.code, *code);
            assert!(wire.message.starts_with(code), "{}", wire.message);
            if *code != "VERSION_CONFLICT" {
                assert_eq!(*code, code.to_lowercase(), "codes are snake_case");
            }
        }
        // Conflicts carry the current status and revision as effects.
        let effects = conflict_effects("7", EdgeStatus::Active, "3");
        assert_eq!(effects[0].field, "edge:7.status");
        assert_eq!(effects[0].observed.as_deref(), Some("active"));
        assert_eq!(effects[1].field, "edge:7.revision");
        assert_eq!(effects[1].observed.as_deref(), Some("3"));
    }

    #[test]
    fn pmo_dep_ready_needs_complete_evaluation() {
        assert!(blockers_ready(true, 0));
        assert!(!blockers_ready(true, 1));
        assert!(!blockers_ready(false, 0));
        assert!(!blockers_ready(false, 2));
    }

    fn end_strategy() -> impl Strategy<Value = EdgeEnd> {
        prop_oneof![
            (
                prop::sample::select(vec!["local", "alpha", "beta"]),
                1u32..40
            )
                .prop_map(|(authority, id)| EdgeEnd::FleetWork(
                    WorkRef::new(authority, id.to_string()).unwrap()
                )),
            (
                prop::sample::select(vec!["a/x", "a/y"]),
                prop::sample::select(ResourceKind::ALL.to_vec()),
                1u32..40,
            )
                .prop_map(|(repository, kind, number)| EdgeEnd::External(
                    ExternalRef {
                        provider: Provider::Github,
                        repository: repository.to_string(),
                        resource_kind: kind,
                        number: number.to_string(),
                        node_id: Some(format!("N_{repository}_{number}")),
                    }
                )),
        ]
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(512))]

        /// A statement and its inverse normalize to the same edge, and
        /// the edge reads back from the subject as the stated spelling.
        #[test]
        fn pmo_dep_normalize_and_describe_round_trip(
            subject in end_strategy(),
            spelling in prop::sample::select(RelationSpelling::ALL.to_vec()),
            object in end_strategy(),
        ) {
            prop_assume!(subject.key() != object.key());
            let stated = normalize(&subject, spelling, &object);
            if let Some(inverse) = spelling.inverse() {
                let inverted = normalize(&object, inverse, &subject);
                prop_assert_eq!(
                    stated.as_ref().map_err(RelationError::code),
                    inverted.as_ref().map_err(RelationError::code)
                );
            }
            if let Ok(canonical) = stated {
                prop_assert!(canonical.target.is_fleet_work());
                let view = describe(&canonical, &subject).unwrap();
                prop_assert_eq!(view.spelling.as_str(), spelling.as_str());
                prop_assert_eq!(view.other_end.key(), object.key());
            }
        }
    }
}
