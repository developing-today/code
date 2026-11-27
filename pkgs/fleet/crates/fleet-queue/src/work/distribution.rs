//! Cross-target canonical order hash — DST-06
//! (developing-today/code#9456; design parent #9200, program #9196).
//!
//! Pure offline contract slice of the PM orchestration design
//! (`doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/`,
//! `components/9200-priority-distribution.md`, "Cross-target order
//! and reconciliation"): given one complete eligible lane snapshot,
//! compute the deterministic `order_hash` every target must agree
//! on. No I/O, no queue, no broker, no persistence — a partial
//! candidate set cannot claim full-order conformance and is a typed
//! error, never a hash.
//!
//! Framing is normative from the architecture digest/serialization
//! contract and shared with [`crate::work::contracts::digest`]:
//! fixed ASCII purpose (`fleet.selection-order/v1`) + exactly one
//! NUL byte + RFC 8785 canonical UTF-8 of the DTO, SHA-256, then
//! `sha256:` plus 64 lowercase hex characters. The hash covers the
//! snapshot/policy/graph inputs AND every ordered item's comparator
//! keys (authority-qualified identity, effective urgency, exact peer
//! rank, creation time) — never a top-page display subset, and never
//! target-local references. Numeric work-ID ties preserve `9 < 10`
//! despite the string wire encoding
//! ([`cmp_work_id`]/[`sort_by_canonical_identity`]).

use std::cmp::Ordering;

use serde::{Deserialize, Serialize};

use super::contracts::{ContractError, DIGEST_DOMAIN_SELECTION_ORDER, digest};

/// Wire schema identifier of the selection-order object
/// (architecture.md: the order object carries `schema_version`).
pub const SELECTION_ORDER_SCHEMA_VERSION: &str = "fleet.selection-order/v1";

/// Comparator tuple this build implements: PRI-03 peer-order tuple
/// version 1 (contract key PRI-CMP-01). The program index owns the
/// tuple registry; this constant is the one ref this build knows —
/// anything else is `unsupported_version` (DST-06 record §4.3).
pub const ORDER_COMPARATOR_REF: &str = "PRI-CMP-01/tuple-1";

/// One ordered candidate in comparator order (effective urgency
/// ascending, then exact peer rank, creation time, normalized
/// canonical identity, per the #9200 component). The hash is
/// order-sensitive: the same set in a different order is a different
/// observed order and hashes differently.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrderedItem {
    /// Canonical authority scope (bytewise identity).
    pub authority_id: String,
    /// Canonical work ID as a decimal string (never coerced through
    /// float; `9 < 10` numerically despite the string encoding).
    pub work_id: String,
    /// Effective urgency from the pinned policy/snapshot inputs.
    /// Lower claims sooner.
    pub effective_urgency: i64,
    /// Exact peer rank within equal effective urgency.
    pub peer_rank: String,
    /// Creation instant (UTC, pinned string).
    pub created_at: String,
}

/// The snapshot inputs the order was calculated from. Every field is
/// part of the hash: the same items under a different
/// policy/profile/graph snapshot are a different observed order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrderSnapshot {
    /// Priority profile version (e.g. `3`).
    pub profile_version: String,
    /// Priority taxonomy mapping version (e.g. `2`).
    pub taxonomy_version: String,
    /// Effective-policy version the pressure was calculated from.
    pub policy_version: String,
    /// Queue high-water mark identifying the exact candidate set.
    pub queue_rev: String,
    /// Dependency-graph snapshot revision.
    pub graph_rev: String,
    /// Eligible lane this slice was selected for.
    pub lane: String,
    /// Read snapshot / as-of instant (UTC, pinned string).
    pub as_of: String,
}

/// Input to [`selection_order_hash`]. `complete` must be true: only
/// a full-population eligible slice may claim
/// `fleet.selection-order/v1` conformance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectionOrderInput {
    /// Snapshot/policy inputs shared by every item.
    pub snapshot: OrderSnapshot,
    /// Candidates in comparator order.
    pub items: Vec<OrderedItem>,
    /// True only for a full-population eligible slice. A partial
    /// candidate set (top page, truncated fetch) hashes to a typed
    /// error, never to an order claim.
    pub complete: bool,
    /// Wire schema of the order object. Must be
    /// [`SELECTION_ORDER_SCHEMA_VERSION`]: anything else is
    /// `unsupported_version`, never silent coercion (DST-06
    /// record §4.3).
    pub schema_version: String,
    /// Comparator tuple the order was computed under. Must be
    /// [`ORDER_COMPARATOR_REF`]: a target on another tuple gets
    /// `unsupported_version`, never a hash compared as if current
    /// (DST-06 record §4.3).
    pub comparator_ref: String,
}

/// Why a candidate set cannot claim full-order conformance: the
/// producer holds one page of a paged population, a truncated
/// fetch, or a population whose completeness it cannot state
/// (DST-06 record §3.1). A partial population stays displayable
/// but is never hashable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartialReason {
    /// Only one page of a paged population is held.
    Paginated,
    /// The fetch was cut off before the population was complete.
    Truncated,
    /// The producer cannot state whether the set is complete.
    UnknownCompleteness,
}

impl PartialReason {
    /// Stable machine code for the reason, suitable for logs and
    /// refusal details.
    pub fn code(&self) -> &'static str {
        match self {
            PartialReason::Paginated => "paginated",
            PartialReason::Truncated => "truncated",
            PartialReason::UnknownCompleteness => "unknown_completeness",
        }
    }

    /// One-line human sentence carried in the refusal error.
    pub fn detail(&self) -> String {
        match self {
            PartialReason::Paginated => "population is one page of a paged fetch".to_string(),
            PartialReason::Truncated => "population is a truncated fetch".to_string(),
            PartialReason::UnknownCompleteness => {
                "population completeness cannot be stated".to_string()
            }
        }
    }
}

/// An order population known to be partial: a page, a truncated
/// fetch, or a population of unknown completeness. Carries the
/// same items for display, with the reason stated. Deliberately
/// exposes no hash: there is no function that hashes a
/// `PartialOrder`, so a page or truncated fetch cannot produce a
/// conformant `order_hash` by accident (DST-06 record §3.1, §8).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartialOrder {
    /// Candidates held, in display order (never hashed).
    pub items: Vec<OrderedItem>,
    /// Why the set cannot claim full-order conformance.
    pub reason: PartialReason,
}

impl SelectionOrderInput {
    /// Refuse completion of a partial population: attempting to
    /// complete a `PartialOrder` is a typed `invalid_schema` error
    /// naming the reason, never an order claim. Reuses the shared
    /// `ContractError` so no `contracts.rs` change is needed (the
    /// DST-07 record's G1 landing order leaves that file to DST-04).
    pub fn from_partial(partial: PartialOrder) -> Result<Self, ContractError> {
        Err(ContractError::InvalidSchema {
            detail: format!(
                "partial candidate set cannot claim full-order conformance: [{}] {}",
                partial.reason.code(),
                partial.reason.detail()
            ),
        })
    }
}

/// Compare two canonical work IDs numerically when both are decimal
/// strings (`9 < 10`), falling back to bytewise order otherwise.
/// Integer-like IDs are never coerced through floating point.
pub fn cmp_work_id(a: &str, b: &str) -> Ordering {
    match (parse_decimal_id(a), parse_decimal_id(b)) {
        (Some(x), Some(y)) => x.cmp(&y),
        _ => a.cmp(b),
    }
}

/// Sort items by normalized canonical identity: authority bytewise,
/// then work ID by integer value. This is the final tie-break of
/// the comparator, not a replacement for it.
pub fn sort_by_canonical_identity(items: &mut [OrderedItem]) {
    items.sort_by(|a, b| {
        a.authority_id
            .cmp(&b.authority_id)
            .then_with(|| cmp_work_id(&a.work_id, &b.work_id))
    });
}

fn parse_decimal_id(value: &str) -> Option<u128> {
    if value.is_empty() {
        return None;
    }
    if value.as_bytes()[0] == b'0' && value.len() > 1 {
        return None;
    }
    if !value.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    value.parse::<u128>().ok()
}

/// Build the canonical DTO hashed by [`selection_order_hash`].
/// Exposed so cross-target fixtures and the framing oracle can
/// reconstruct and independently verify the exact bytes.
pub fn selection_order_dto(input: &SelectionOrderInput) -> serde_json::Value {
    serde_json::json!({
        "purpose": DIGEST_DOMAIN_SELECTION_ORDER,
        "schema_version": input.schema_version,
        "comparator_ref": input.comparator_ref,
        "snapshot": {
            "profile_version": input.snapshot.profile_version,
            "taxonomy_version": input.snapshot.taxonomy_version,
            "policy_version": input.snapshot.policy_version,
            "queue_rev": input.snapshot.queue_rev,
            "graph_rev": input.snapshot.graph_rev,
            "lane": input.snapshot.lane,
            "as_of": input.snapshot.as_of,
        },
        "items": input.items.iter().map(|item| serde_json::json!({
            "authority_id": item.authority_id,
            "work_id": item.work_id,
            "effective_urgency": item.effective_urgency,
            "peer_rank": item.peer_rank,
            "created_at": item.created_at,
        })).collect::<Vec<_>>(),
    })
}

/// Compute the canonical cross-target order hash for one complete
/// eligible lane snapshot. Deterministic: the same
/// authority/profile/snapshot/items yield byte-identical output on
/// every target. Any included-input change (snapshot field, item
/// field, item order) changes the hash. Version/target divergence
/// is observed as hash inequality, never coerced.
pub fn selection_order_hash(input: &SelectionOrderInput) -> Result<String, ContractError> {
    if !input.complete {
        return Err(ContractError::InvalidSchema {
            detail: "partial candidate set cannot claim full-order conformance".to_string(),
        });
    }
    if input.schema_version != SELECTION_ORDER_SCHEMA_VERSION {
        return Err(ContractError::UnsupportedVersion {
            schema_version: input.schema_version.clone(),
        });
    }
    if input.comparator_ref != ORDER_COMPARATOR_REF {
        return Err(ContractError::UnsupportedVersion {
            schema_version: format!("comparator:{}", input.comparator_ref),
        });
    }
    digest(DIGEST_DOMAIN_SELECTION_ORDER, &selection_order_dto(input))
}

/// How two order observations relate, checked in this order
/// (DST-06 record §4.1). The classes mean different things, so the
/// report stops at the first class that applies: different inputs
/// are expected during normal operation (re-run on a pinned
/// snapshot), while a different sequence over the same inputs,
/// items and keys is always a comparator defect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrderComparison {
    /// Same inputs, same population, same keys, same sequence: the
    /// two targets computed the same order. A fingerprint only — it
    /// grants nothing (SEL-02).
    Identical,
    /// The two targets ordered different snapshots (different graph
    /// revision, as-of, policy ...). Expected during normal
    /// operation; re-run on a pinned snapshot. Order beneath it is
    /// not inspected.
    InputsDiffer {
        /// Snapshot keys that differ, in canonical snapshot order.
        keys: Vec<&'static str>,
    },
    /// Same pinned inputs, different eligible set: an eligibility
    /// disagreement (predicates or hard gates).
    PopulationDiffers {
        /// Canonical identities only in `a`, sorted.
        only_in_a: Vec<String>,
        /// Canonical identities only in `b`, sorted.
        only_in_b: Vec<String>,
    },
    /// Same population, an item's comparator keys differ (e.g. one
    /// target normalised `peer_rank` differently).
    ItemKeysDiffer {
        /// `work_id` of the first item (in `a`'s order) whose
        /// comparator keys differ.
        work_id: String,
        /// Comparator keys that differ, in comparator order.
        keys: Vec<&'static str>,
    },
    /// Same inputs, same items, same keys, different sequence: a
    /// comparator bug. The only class that is always a defect.
    OrderDiffers {
        /// First index where the two sequences disagree.
        first_index: usize,
        /// The item `a` holds at that index.
        a: OrderedItem,
        /// The item `b` holds at that index.
        b: OrderedItem,
    },
    /// The pair cannot be compared: at least one side is a partial
    /// candidate set, which claims no order (`partial_candidate_set`
    /// reason), or the pair runs unknown or unequal versions, which
    /// must never be compared as if current (`version_skew` reason,
    /// DST-06 record §4.3).
    NotComparable {
        /// Machine-readable reason (`partial_candidate_set` or
        /// `version_skew`).
        reason: &'static str,
    },
}

/// Render the authority-qualified identity of one item for
/// population reports: `authority_id/work_id`.
fn canonical_identity(item: &OrderedItem) -> String {
    format!("{}/{}", item.authority_id, item.work_id)
}

/// Snapshot keys in canonical order, for [`OrderComparison`]
/// reports.
fn snapshot_diff_keys(a: &OrderSnapshot, b: &OrderSnapshot) -> Vec<&'static str> {
    let mut keys = Vec::new();
    if a.profile_version != b.profile_version {
        keys.push("profile_version");
    }
    if a.taxonomy_version != b.taxonomy_version {
        keys.push("taxonomy_version");
    }
    if a.policy_version != b.policy_version {
        keys.push("policy_version");
    }
    if a.queue_rev != b.queue_rev {
        keys.push("queue_rev");
    }
    if a.graph_rev != b.graph_rev {
        keys.push("graph_rev");
    }
    if a.lane != b.lane {
        keys.push("lane");
    }
    if a.as_of != b.as_of {
        keys.push("as_of");
    }
    keys
}

/// Sorted canonical identities of one population, multiset: the same
/// identity appears once per occurrence, so a duplicated identity is
/// a population difference, not silent agreement. (Duplicates
/// themselves are a builder refusal — oracle test 5.)
fn sorted_identities(items: &[OrderedItem]) -> Vec<String> {
    let mut ids: Vec<String> = items.iter().map(canonical_identity).collect();
    ids.sort();
    ids
}

/// Whether one input runs versions this build knows: the wire
/// schema and the one comparator tuple it implements. Anything
/// else is `unsupported_version` at build time and `version_skew`
/// at compare time — never silent coercion (DST-06 record §4.3).
fn is_known_version(input: &SelectionOrderInput) -> bool {
    input.schema_version == SELECTION_ORDER_SCHEMA_VERSION
        && input.comparator_ref == ORDER_COMPARATOR_REF
}

/// Whether two known-version inputs run unequal versions. Version
/// skew is checked before snapshot inputs: different versions may
/// legitimately order differently, so the pair is refused, not
/// diagnosed as an input difference (DST-06 record §4.1 vs §4.3).
fn version_skewed(a: &SelectionOrderInput, b: &SelectionOrderInput) -> bool {
    a.schema_version != b.schema_version || a.comparator_ref != b.comparator_ref
}

/// Compare two order observations and name their divergence class.
///
/// Both sides must be complete: a partial candidate set claims no
/// order, so a pair holding one is [`OrderComparison::NotComparable`],
/// never a comparison. Both sides must also run known, equal
/// versions: an unknown schema or comparator on either side — or a
/// skew between them — is [`OrderComparison::NotComparable`] with
/// reason `version_skew`, never a comparison as if current (DST-06
/// record §4.3). Otherwise the report stops at the first class that
/// applies: inputs, then population, then item keys, then sequence
/// (DST-06 record §4.1).
pub fn compare(a: &SelectionOrderInput, b: &SelectionOrderInput) -> OrderComparison {
    if !a.complete || !b.complete {
        return OrderComparison::NotComparable {
            reason: "partial_candidate_set",
        };
    }
    if !is_known_version(a) || !is_known_version(b) || version_skewed(a, b) {
        return OrderComparison::NotComparable {
            reason: "version_skew",
        };
    }
    let keys = snapshot_diff_keys(&a.snapshot, &b.snapshot);
    if !keys.is_empty() {
        return OrderComparison::InputsDiffer { keys };
    }
    let ids_a = sorted_identities(&a.items);
    let ids_b = sorted_identities(&b.items);
    if ids_a != ids_b {
        let only_in_a: Vec<String> = ids_a
            .iter()
            .filter(|id| !ids_b.contains(id))
            .cloned()
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        let only_in_b: Vec<String> = ids_b
            .iter()
            .filter(|id| !ids_a.contains(id))
            .cloned()
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        return OrderComparison::PopulationDiffers {
            only_in_a,
            only_in_b,
        };
    }
    for item_a in &a.items {
        if let Some(item_b) = b
            .items
            .iter()
            .find(|item| item.authority_id == item_a.authority_id && item.work_id == item_a.work_id)
        {
            let mut keys = Vec::new();
            if item_a.effective_urgency != item_b.effective_urgency {
                keys.push("effective_urgency");
            }
            if item_a.peer_rank != item_b.peer_rank {
                keys.push("peer_rank");
            }
            if item_a.created_at != item_b.created_at {
                keys.push("created_at");
            }
            if !keys.is_empty() {
                return OrderComparison::ItemKeysDiffer {
                    work_id: item_a.work_id.clone(),
                    keys,
                };
            }
        }
    }
    for (index, (item_a, item_b)) in a.items.iter().zip(b.items.iter()).enumerate() {
        if item_a.authority_id != item_b.authority_id || item_a.work_id != item_b.work_id {
            return OrderComparison::OrderDiffers {
                first_index: index,
                a: item_a.clone(),
                b: item_b.clone(),
            };
        }
    }
    OrderComparison::Identical
}

#[cfg(test)]
mod tests {
    use super::super::contracts::is_digest_string;
    use super::*;

    fn fixture_snapshot() -> OrderSnapshot {
        OrderSnapshot {
            profile_version: "3".to_string(),
            taxonomy_version: "2".to_string(),
            policy_version: "policy:v7".to_string(),
            queue_rev: "queue:rev:118".to_string(),
            graph_rev: "graph:rev:57".to_string(),
            lane: "fleet-default".to_string(),
            as_of: "2026-11-13T03:30:00Z".to_string(),
        }
    }

    fn fixture_items() -> Vec<OrderedItem> {
        vec![
            OrderedItem {
                authority_id: "fleet-local".to_string(),
                work_id: "842".to_string(),
                effective_urgency: 15,
                peer_rank: "a".to_string(),
                created_at: "2026-11-12T10:00:00Z".to_string(),
            },
            OrderedItem {
                authority_id: "fleet-local".to_string(),
                work_id: "843".to_string(),
                effective_urgency: 20,
                peer_rank: "a".to_string(),
                created_at: "2026-11-12T10:01:00Z".to_string(),
            },
        ]
    }

    fn fixture_input() -> SelectionOrderInput {
        SelectionOrderInput {
            snapshot: fixture_snapshot(),
            items: fixture_items(),
            complete: true,
            schema_version: SELECTION_ORDER_SCHEMA_VERSION.to_string(),
            comparator_ref: ORDER_COMPARATOR_REF.to_string(),
        }
    }

    /// DST-06 oracle (#9456): same authority/profile/snapshot yields
    /// byte-identical canonical IDs and hash on every target;
    /// version/target divergence fails (hash inequality); framing
    /// matches the normative domain/NUL/RFC-8785/SHA-256 bytes;
    /// numeric work-ID ties preserve 9 < 10; a partial set is a
    /// typed error, never an order claim. Oracle source: the #9200
    /// component's "Cross-target order and reconciliation" section
    /// and the architecture digest/serialization contract — the
    /// independent framing recomputation below copies those
    /// documents, not this module's implementation.
    #[test]
    fn pmo_distribution_host_order_hash() {
        // Same logical input built twice on "two targets" hashes
        // byte-identically.
        let first = selection_order_hash(&fixture_input()).unwrap();
        let second = selection_order_hash(&fixture_input()).unwrap();
        assert_eq!(first, second);
        assert!(is_digest_string(&first));

        // Framing vector: independently recompute the exact wire
        // bytes — ASCII domain, one NUL, canonical JSON — and hash
        // with SHA-256 directly, without touching this module's
        // digest path.
        let dto = selection_order_dto(&fixture_input());
        let canonical = super::super::contracts::canonical_json(&dto).unwrap();
        let mut hasher = sha2::Sha256::new();
        use sha2::Digest as _;
        hasher.update(DIGEST_DOMAIN_SELECTION_ORDER.as_bytes());
        hasher.update([0u8]);
        hasher.update(canonical.as_bytes());
        let bytes: [u8; 32] = hasher.finalize().into();
        let mut expected = String::from("sha256:");
        for byte in bytes {
            expected.push_str(&format!("{byte:02x}"));
        }
        assert_eq!(first, expected);

        // Included-input changes diverge: policy version bump,
        // item-field change, and item reorder each change the hash.
        let mut bumped = fixture_input();
        bumped.snapshot.policy_version = "policy:v8".to_string();
        assert_ne!(selection_order_hash(&bumped).unwrap(), first);

        let mut rescored = fixture_input();
        rescored.items[0].effective_urgency = 16;
        assert_ne!(selection_order_hash(&rescored).unwrap(), first);

        let mut reordered = fixture_input();
        reordered.items.reverse();
        assert_ne!(selection_order_hash(&reordered).unwrap(), first);

        // Target divergence on taxonomy version fails equality too.
        let mut retaxed = fixture_input();
        retaxed.snapshot.taxonomy_version = "3".to_string();
        assert_ne!(selection_order_hash(&retaxed).unwrap(), first);

        // Numeric work-ID ties: 9 < 10 despite string encoding, and
        // the canonical sort applies it after authority order.
        assert_eq!(cmp_work_id("9", "10"), Ordering::Less);
        assert_eq!(cmp_work_id("10", "9"), Ordering::Greater);
        assert_eq!(cmp_work_id("9", "9"), Ordering::Equal);
        let mut ties = vec![
            OrderedItem {
                authority_id: "fleet-local".to_string(),
                work_id: "10".to_string(),
                effective_urgency: 15,
                peer_rank: "a".to_string(),
                created_at: "2026-11-12T10:00:00Z".to_string(),
            },
            OrderedItem {
                authority_id: "fleet-local".to_string(),
                work_id: "9".to_string(),
                effective_urgency: 15,
                peer_rank: "a".to_string(),
                created_at: "2026-11-12T10:00:00Z".to_string(),
            },
        ];
        sort_by_canonical_identity(&mut ties);
        assert_eq!(
            ties.iter()
                .map(|item| item.work_id.as_str())
                .collect::<Vec<_>>(),
            vec!["9", "10"]
        );

        // A partial candidate set cannot claim full-order
        // conformance: typed invalid_schema, no hash.
        let mut partial = fixture_input();
        partial.complete = false;
        let err = selection_order_hash(&partial).unwrap_err();
        assert_eq!(err.code(), "invalid_schema");
        assert!(format!("{err:?}").contains("full-order conformance"));
    }

    /// DST-06 oracle test 2 (#9456): a producer holding only a
    /// page, a truncated fetch, or a population of unknown
    /// completeness builds a `PartialOrder` stating the reason.
    /// `PartialOrder` exposes no hash function by construction, and
    /// completing one is a typed `invalid_schema` refusal naming the
    /// reason — never an order claim. Oracle source: the DST-06
    /// design record §3.1 ("only one can be hashed") and §8
    /// ("Producer has only a page: PartialOrder, no hash").
    #[test]
    fn pmo_distribution_partial_order_has_no_hash() {
        // Reason codes are stable: logs and refusal details can
        // match on them.
        assert_eq!(PartialReason::Paginated.code(), "paginated");
        assert_eq!(PartialReason::Truncated.code(), "truncated");
        assert_eq!(
            PartialReason::UnknownCompleteness.code(),
            "unknown_completeness"
        );

        // Every stated reason refuses completion with its name in
        // the typed error, and yields no input that could hash.
        for reason in [
            PartialReason::Paginated,
            PartialReason::Truncated,
            PartialReason::UnknownCompleteness,
        ] {
            let partial = PartialOrder {
                items: fixture_items(),
                reason,
            };
            let err = SelectionOrderInput::from_partial(partial).unwrap_err();
            assert_eq!(err.code(), "invalid_schema");
            let rendered = format!("{err:?}");
            assert!(
                rendered.contains(reason.code()),
                "refusal names the reason: {rendered}"
            );
            assert!(
                rendered.contains("full-order conformance"),
                "refusal states the rule: {rendered}"
            );
        }
    }

    /// DST-06 oracle test 6 (#9456): `compare` returns each
    /// divergence class for a constructed pair, in the §4.1
    /// precedence — inputs, then population, then item keys, then
    /// sequence — and refuses a pair holding a partial candidate
    /// set. Oracle source: the DST-06 design record §4
    /// ("Comparing two targets"), which this test copies, not this
    /// module's implementation.
    #[test]
    fn pmo_distribution_divergence_classes() {
        // Identical observations compare identical, and their hashes
        // agree: same inputs hashed twice on "two targets".
        let first = fixture_input();
        let second = fixture_input();
        assert_eq!(compare(&first, &second), OrderComparison::Identical);
        assert_eq!(
            selection_order_hash(&first).unwrap(),
            selection_order_hash(&second).unwrap()
        );

        // One snapshot field differs: inputs differ, naming the key
        // in canonical snapshot order. Order beneath it is not
        // inspected, so the population may differ too.
        let mut retaxed = fixture_input();
        retaxed.snapshot.policy_version = "policy:v8".to_string();
        retaxed.items.pop();
        assert_eq!(
            compare(&first, &retaxed),
            OrderComparison::InputsDiffer {
                keys: vec!["policy_version"],
            }
        );
        let mut multi = fixture_input();
        multi.snapshot.policy_version = "policy:v8".to_string();
        multi.snapshot.as_of = "2026-11-13T04:30:00Z".to_string();
        assert_eq!(
            compare(&first, &multi),
            OrderComparison::InputsDiffer {
                keys: vec!["policy_version", "as_of"],
            }
        );

        // Same pinned inputs, different eligible set: the missing
        // and extra identities are named, sorted.
        let mut smaller = fixture_input();
        smaller.items.pop();
        assert_eq!(
            compare(&first, &smaller),
            OrderComparison::PopulationDiffers {
                only_in_a: vec!["fleet-local/843".to_string()],
                only_in_b: vec![],
            }
        );
        let mut swapped_set = fixture_input();
        swapped_set.items[1].work_id = "844".to_string();
        assert_eq!(
            compare(&first, &swapped_set),
            OrderComparison::PopulationDiffers {
                only_in_a: vec!["fleet-local/843".to_string()],
                only_in_b: vec!["fleet-local/844".to_string()],
            }
        );

        // Same population, an item's comparator keys differ: the
        // work_id and the differing keys, in comparator order.
        let mut reranked = fixture_input();
        reranked.items[1].peer_rank = "b".to_string();
        assert_eq!(
            compare(&first, &reranked),
            OrderComparison::ItemKeysDiffer {
                work_id: "843".to_string(),
                keys: vec!["peer_rank"],
            }
        );

        // Same inputs, same items, same keys, different sequence: a
        // comparator bug, with the first disagreeing index. The
        // hashes differ too: order is part of the claim.
        let mut reordered = fixture_input();
        reordered.items.reverse();
        assert_eq!(
            compare(&first, &reordered),
            OrderComparison::OrderDiffers {
                first_index: 0,
                a: first.items[0].clone(),
                b: first.items[1].clone(),
            }
        );
        assert_ne!(
            selection_order_hash(&first).unwrap(),
            selection_order_hash(&reordered).unwrap()
        );

        // Precedence: a population difference does not mask an input
        // difference, and a sequence difference does not mask a
        // population difference.
        assert!(matches!(
            compare(&first, &retaxed),
            OrderComparison::InputsDiffer { .. }
        ));
        let mut both = fixture_input();
        both.items[1].work_id = "844".to_string();
        both.items.reverse();
        assert!(matches!(
            compare(&first, &both),
            OrderComparison::PopulationDiffers { .. }
        ));

        // A pair holding a partial candidate set claims no order, so
        // there is nothing to compare: refused, never classified.
        let mut partial = fixture_input();
        partial.complete = false;
        assert_eq!(
            compare(&first, &partial),
            OrderComparison::NotComparable {
                reason: "partial_candidate_set",
            }
        );
        assert_eq!(
            compare(&partial, &first),
            OrderComparison::NotComparable {
                reason: "partial_candidate_set",
            }
        );
    }

    /// DST-06 oracle test 7 (#9456): an unknown schema or comparator
    /// is `unsupported_version` for build and compare (DST-06 record
    /// §4.3). A target on another tuple version never gets a hash,
    /// and a skewed pair is refused — never compared as if current.
    #[test]
    fn pmo_distribution_unknown_version_not_compared() {
        // Build: unknown schema version is unsupported_version, not
        // a hash and not silent coercion to the known schema.
        let mut future_schema = fixture_input();
        future_schema.schema_version = "fleet.selection-order/v2".to_string();
        let err = selection_order_hash(&future_schema).unwrap_err();
        assert_eq!(err.code(), "unsupported_version");
        assert!(format!("{err:?}").contains("fleet.selection-order/v2"));

        // Build: unknown comparator tuple is unsupported_version —
        // an old target cannot silently hash under the old tuple.
        let mut old_tuple = fixture_input();
        old_tuple.comparator_ref = "PRI-CMP-01/tuple-0".to_string();
        let err = selection_order_hash(&old_tuple).unwrap_err();
        assert_eq!(err.code(), "unsupported_version");

        // Build: a missing comparator ref is unknown, not a default:
        // empty never means "the current tuple".
        let mut unversioned = fixture_input();
        unversioned.comparator_ref = String::new();
        assert_eq!(
            selection_order_hash(&unversioned).unwrap_err().code(),
            "unsupported_version"
        );

        // Compare: the same unknown schema on both sides is still
        // refused — an unknown version is never comparable, even
        // with itself.
        let also_future = future_schema.clone();
        assert_eq!(
            compare(&future_schema, &also_future),
            OrderComparison::NotComparable {
                reason: "version_skew",
            }
        );

        // Compare: skew between known schema and unknown schema is
        // refused, and it preempts input diagnosis — the report is
        // version_skew, not InputsDiffer, even when the snapshots
        // also differ.
        let first = fixture_input();
        let mut skewed_inputs = future_schema.clone();
        skewed_inputs.snapshot.policy_version = "policy:v8".to_string();
        assert_eq!(
            compare(&first, &skewed_inputs),
            OrderComparison::NotComparable {
                reason: "version_skew",
            }
        );

        // Compare: same known schema, different comparator tuples —
        // one target moved tuples — is refused, never classified.
        assert_eq!(
            compare(&first, &old_tuple),
            OrderComparison::NotComparable {
                reason: "version_skew",
            }
        );
        assert_eq!(
            compare(&old_tuple, &first),
            OrderComparison::NotComparable {
                reason: "version_skew",
            }
        );

        // Compare: equal known versions still compare normally —
        // the refusal only fires on unknown or unequal versions.
        let second = fixture_input();
        assert_eq!(compare(&first, &second), OrderComparison::Identical);

        // Known-version constants are the wire values: schema is the
        // digest domain's version, comparator is PRI-03 tuple 1.
        assert_eq!(SELECTION_ORDER_SCHEMA_VERSION, "fleet.selection-order/v1");
        assert_eq!(ORDER_COMPARATOR_REF, "PRI-CMP-01/tuple-1");
    }
}
