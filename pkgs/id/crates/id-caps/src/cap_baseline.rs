//! The accepted capability baseline — #5959 §1's client-side
//! record and §5's re-baselining rule (implementation issue #12921).
//!
//! The baseline `B` of #5999 §6 is a per-(user, world) record stored
//! with the user's other ID state — the same store #5960 §5.1 puts
//! remembered grants in, per user identity, following the user
//! across devices, never per device fingerprint. It is client-side
//! by design: a server that stores the baseline can also rewrite it,
//! and the notice would then be something the server decides to
//! show. This module is the record and its transitions; the
//! persistence backend and the connect-pipeline stage that calls
//! [`BaselineStore::evaluate`] are #12921's follow-on slices.
//!
//! `B` changes only through the five paths of #5959 §5, each written
//! to the record's history and inspectable on the world's grant
//! page:
//!
//! 1. an explicit remember-choice on a notice ([`CapBaseline::accept`],
//!    composing with per-class denial via
//!    [`CapBaseline::accept_minus_denied`]);
//! 2. first-connect decisions ([`CapBaseline::establish`]);
//! 3. an invite pin ([`CapBaseline::establish`] with
//!    [`BaselineHistoryKind::Pin`]);
//! 4. automatic narrowing on a covered, strictly smaller reconnect
//!    ([`CapBaseline::narrow`]) — a decrease is never an
//!    interruption, and the narrowing is load-bearing: a class the
//!    world drops and later re-adds prompts as the expansion it is;
//! 5. grant-page edits ([`CapBaseline::revoke_class`]).
//!
//! And never through connecting by itself, a server or admin action,
//! a world update by itself, or the passage of time. While YOLO is
//! on, the trigger is suppressed but the bookkeeping still happens
//! ([`CapBaseline::yolo_auto_accept`]), so turning YOLO off never
//! resurrects a stale baseline (#5959 §5).
//!
//! [`CapBaseline::evaluate`] is the pure comparison of
//! [`crate::cap_diff`], not the §2 step-3 short-circuit: for a
//! baseline recorded from the same release and table the two agree,
//! and after a grant-page edit the pure comparison is the reading
//! §5 path 5 requires (a revoked class the world still requests is
//! new, and prompts).

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::cap_class::CAP_CLASS_TABLE_VERSION;
use crate::cap_diff::{CapDiff, compare, uncomputable_diff};

/// How a baseline came to hold the message set it holds — the
/// `kind` of a history entry (#5959 §1, §5). Every transition that
/// changes `B` records exactly one of these; there is no sixth path.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BaselineHistoryKind {
    /// An explicit remember-choice on a notice, or the decisions of
    /// a first connect (#5959 §4, §5 path 1 and 2).
    Accept,
    /// Automatic narrowing on a covered, strictly smaller reconnect
    /// (#5959 §5 path 4).
    Narrow,
    /// A baseline seeded by an invite's capability pin (#5959 §7,
    /// §5 path 3).
    Pin,
    /// A grant-page edit: the user revoked a remembered grant
    /// (#5959 §5 path 5).
    GrantPageEdit,
    /// YOLO auto-accept: the comparison ran, the trigger was
    /// suppressed by YOLO, and the bookkeeping still happened
    /// (#5959 §5).
    YoloAutoAccept,
}

/// One entry in a baseline's history (#5959 §1).
///
/// When a transition happened, which of the five paths it was, and
/// the message set it left behind. `at` is the caller's ISO-8601
/// timestamp — this module keeps no clock, so the record stays a
/// pure function of the transitions applied to it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BaselineHistoryEntry {
    /// When the transition happened (ISO-8601, caller-supplied).
    pub at: String,
    /// Which re-baselining path the transition was.
    pub kind: BaselineHistoryKind,
    /// The message set the transition left behind.
    pub message_set: Vec<String>,
}

/// The accepted baseline for one world (#5959 §1): the message set
/// the user last explicitly approved, keyed by `baseline_key` over
/// the release identity it was accepted under.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapBaseline {
    /// The world instance id.
    pub world_id: String,
    /// The #5958 release identity (content-addressed: platform tree
    /// hash + source closure hash + DLL closure hash + compiler
    /// digest + target) the baseline was recorded against.
    pub release_identity: String,
    /// The accepted message set `B`: class ids, sorted, maximal
    /// under the subsumption order.
    pub message_set: Vec<String>,
    /// The class-table version the message set was computed under
    /// (#5999 §2.4).
    pub table_version: u32,
    /// `H(world_id ‖ release_identity ‖ message_set ‖ table_version)`
    /// — the key a comparison is recorded under (#5959 §1). Derived
    /// from the other fields on every transition; never edited
    /// directly.
    pub baseline_key: String,
    /// Every transition that produced this record, oldest first.
    pub history: Vec<BaselineHistoryEntry>,
}

/// Pure connect-stage outcome for a freshly resolved release.
///
/// This decides whether the caller may proceed, needs the first-connect
/// disclosure flow, or must hold for a capability-change notice. It does
/// not resolve manifests, persist a narrowed baseline, or control a live
/// connection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConnectVerdict {
    /// There is no accepted baseline; disclose this complete message set
    /// and let the first-connect flow record the user's decisions.
    FirstConnect { message_set: Vec<String> },
    /// The baseline already describes this release under the current
    /// class table, so no comparison or notice is needed.
    SameReleaseAndTable,
    /// The computed set is covered by the baseline. `narrowed` tells the
    /// caller whether automatic narrowing bookkeeping is applicable.
    Proceed {
        message_set: Vec<String>,
        narrowed: bool,
    },
    /// The connection must be held; the diff names exactly the classes
    /// the notice needs to disclose.
    Hold(CapDiff),
}

/// Decide the connect-stage capability outcome from already resolved
/// inputs. `message_set` is `None` when release/manifest/class comparison
/// could not be computed; that path fails closed with the `unknown` class.
///
/// A baseline for the same release and shipped class-table version
/// short-circuits before requiring a computed set. Other outcomes use the
/// same table version recorded by the shipped vocabulary, rather than a
/// caller-supplied version that could disagree with it.
#[must_use]
pub fn evaluate_connect(
    baseline: Option<&CapBaseline>,
    release_identity: &str,
    message_set: Option<&BTreeSet<&str>>,
) -> ConnectVerdict {
    let Some(baseline) = baseline else {
        return match message_set {
            Some(message_set) => ConnectVerdict::FirstConnect {
                message_set: message_set
                    .iter()
                    .map(|class| (*class).to_owned())
                    .collect(),
            },
            None => ConnectVerdict::Hold(uncomputable_diff()),
        };
    };

    if baseline.release_identity == release_identity
        && baseline.table_version == CAP_CLASS_TABLE_VERSION
    {
        return ConnectVerdict::SameReleaseAndTable;
    }

    let Some(message_set) = message_set else {
        return ConnectVerdict::Hold(uncomputable_diff());
    };
    let diff = baseline.evaluate(message_set);
    if diff.triggers() {
        return ConnectVerdict::Hold(diff);
    }
    ConnectVerdict::Proceed {
        narrowed: *message_set != baseline.message_set(),
        message_set: message_set
            .iter()
            .map(|class| (*class).to_owned())
            .collect(),
    }
}

/// Lowercase hex digits, for the baseline key's encoding.
const HEX: &[u8; 16] = b"0123456789abcdef";

/// `H(world_id ‖ release_identity ‖ message_set ‖ table_version)`
/// (#5959 §1, #5999 §4.2), SHA-256 over a length-framed encoding so
/// no two distinct field tuples share a preimage by concatenation.
fn compute_baseline_key(
    world_id: &str,
    release_identity: &str,
    message_set: &[String],
    table_version: u32,
) -> String {
    let mut classes: Vec<&str> = message_set.iter().map(String::as_str).collect();
    classes.sort_unstable();
    classes.dedup();
    let mut hasher = Sha256::new();
    for field in [world_id, release_identity] {
        hasher.update((field.len() as u64).to_le_bytes());
        hasher.update(field.as_bytes());
    }
    hasher.update((classes.len() as u64).to_le_bytes());
    for class in classes {
        hasher.update((class.len() as u64).to_le_bytes());
        hasher.update(class.as_bytes());
    }
    hasher.update(table_version.to_le_bytes());
    let digest = hasher.finalize();
    let mut key = String::with_capacity(digest.len() * 2);
    for byte in digest {
        key.push(HEX[(byte >> 4) as usize] as char);
        key.push(HEX[(byte & 0x0f) as usize] as char);
    }
    key
}

impl CapBaseline {
    /// Establish a baseline where none exists: a first connect's
    /// decisions (#5959 §5 path 2, `kind` =
    /// [`BaselineHistoryKind::Accept`]) or an invite pin (§7, `kind`
    /// = [`BaselineHistoryKind::Pin`], the pinned set narrowed to
    /// the set actually joined). The establishing act is the first
    /// history entry.
    #[must_use]
    pub fn establish(
        world_id: &str,
        release_identity: &str,
        message_set: BTreeSet<String>,
        table_version: u32,
        kind: BaselineHistoryKind,
        at: &str,
    ) -> Self {
        let message_set: Vec<String> = message_set.into_iter().collect();
        let mut baseline = Self {
            baseline_key: compute_baseline_key(
                world_id,
                release_identity,
                &message_set,
                table_version,
            ),
            world_id: world_id.to_owned(),
            release_identity: release_identity.to_owned(),
            message_set,
            table_version,
            history: Vec::new(),
        };
        baseline.record(kind, at);
        baseline
    }

    /// The accepted message set as a set of class ids.
    #[must_use]
    pub fn message_set(&self) -> BTreeSet<&str> {
        self.message_set.iter().map(String::as_str).collect()
    }

    /// Compare a freshly computed message set `new` against this
    /// baseline (#5999 §6): covered ⇒ the connection proceeds (and
    /// [`CapBaseline::narrow`] applies the bookkeeping); not covered
    /// ⇒ the connection is held and the notice names
    /// [`CapDiff::new_classes`].
    #[must_use]
    pub fn evaluate(&self, new: &BTreeSet<&str>) -> CapDiff {
        compare(&self.message_set(), new)
    }

    /// Explicit remember (#5959 §4, §5 path 1): `B := N`, the only
    /// notice path that raises the baseline, always an explicit act.
    pub fn accept(
        &mut self,
        release_identity: &str,
        new: BTreeSet<String>,
        table_version: u32,
        at: &str,
    ) {
        self.rebase(release_identity, new, table_version);
        self.record(BaselineHistoryKind::Accept, at);
    }

    /// Remember with per-class denial composed in (#5959 §4):
    /// `B := N ∖ {denied classes}`. A denied class was never
    /// accepted, so if the world still requests it, it prompts
    /// again next time — the correct consequence of the denial, not
    /// a bug.
    pub fn accept_minus_denied(
        &mut self,
        release_identity: &str,
        new: BTreeSet<String>,
        denied: &BTreeSet<&str>,
        table_version: u32,
        at: &str,
    ) {
        let kept: BTreeSet<String> = new
            .into_iter()
            .filter(|c| !denied.contains(c.as_str()))
            .collect();
        self.accept(release_identity, kept, table_version, at);
    }

    /// YOLO auto-accept (#5959 §5): the comparison ran and the
    /// trigger was suppressed; the bookkeeping still happens, so
    /// the baseline is never stale when YOLO turns off. `B := N`.
    pub fn yolo_auto_accept(
        &mut self,
        release_identity: &str,
        new: BTreeSet<String>,
        table_version: u32,
        at: &str,
    ) {
        self.rebase(release_identity, new, table_version);
        self.record(BaselineHistoryKind::YoloAutoAccept, at);
    }

    /// Automatic narrowing (#5959 §5 path 4): on a covered reconnect
    /// where `new` is strictly narrower than the accepted set,
    /// `B := N`, written to history. Returns whether the baseline
    /// changed — a covered reconnect with the same message set, and
    /// an uncovered one (which must prompt, never narrow), both
    /// leave the record untouched.
    pub fn narrow(
        &mut self,
        release_identity: &str,
        new: BTreeSet<String>,
        table_version: u32,
        at: &str,
    ) -> bool {
        let new_refs: BTreeSet<&str> = new.iter().map(String::as_str).collect();
        if self.evaluate(&new_refs).triggers() || new_refs == self.message_set() {
            return false;
        }
        self.rebase(release_identity, new, table_version);
        self.record(BaselineHistoryKind::Narrow, at);
        true
    }

    /// Grant-page edit (#5959 §5 path 5): revoking a remembered
    /// grant removes the class from `B`; if the world later requests
    /// it again, it is new and prompts. Returns whether the class
    /// was in the baseline.
    pub fn revoke_class(&mut self, class: &str, at: &str) -> bool {
        if !self.message_set.iter().any(|c| c == class) {
            return false;
        }
        self.message_set.retain(|c| c != class);
        self.refresh_key();
        self.record(BaselineHistoryKind::GrantPageEdit, at);
        true
    }

    /// Whether `baseline_key` is the key the record's current fields
    /// compute to — the check a store runs after loading a record it
    /// did not write.
    #[must_use]
    pub fn key_is_current(&self) -> bool {
        self.baseline_key
            == compute_baseline_key(
                &self.world_id,
                &self.release_identity,
                &self.message_set,
                self.table_version,
            )
    }

    fn rebase(&mut self, release_identity: &str, new: BTreeSet<String>, table_version: u32) {
        release_identity.clone_into(&mut self.release_identity);
        self.message_set = new.into_iter().collect();
        self.table_version = table_version;
        self.refresh_key();
    }

    fn refresh_key(&mut self) {
        self.baseline_key = compute_baseline_key(
            &self.world_id,
            &self.release_identity,
            &self.message_set,
            self.table_version,
        );
    }

    fn record(&mut self, kind: BaselineHistoryKind, at: &str) {
        self.history.push(BaselineHistoryEntry {
            at: at.to_owned(),
            kind,
            message_set: self.message_set.clone(),
        });
    }
}

/// The per-user baseline store: one [`CapBaseline`] per world.
///
/// Held with the user's other ID state (#5959 §1). The store is the
/// user's — two users' stores never share a record, and the
/// strictest device's record is the one that prompts (#5999 §4.2).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BaselineStore {
    baselines: BTreeMap<String, CapBaseline>,
}

impl BaselineStore {
    /// An empty store — a user who has never connected anywhere.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The baseline for a world, if the user has one. `None` is a
    /// first connect: it follows #5960 §1.2's full-disclosure flow
    /// and its decisions establish the baseline (#5959 §2 step 5).
    #[must_use]
    pub fn get(&self, world_id: &str) -> Option<&CapBaseline> {
        self.baselines.get(world_id)
    }

    /// Mutable access for the re-baselining transitions.
    pub fn get_mut(&mut self, world_id: &str) -> Option<&mut CapBaseline> {
        self.baselines.get_mut(world_id)
    }

    /// Record a baseline (established by a first connect or an
    /// invite pin), replacing any record for the same world.
    pub fn insert(&mut self, baseline: CapBaseline) {
        self.baselines.insert(baseline.world_id.clone(), baseline);
    }

    /// Every baseline, in world-id order — the grant page's listing
    /// surface (#5959 §1, #5960 §5.1).
    pub fn baselines(&self) -> impl Iterator<Item = &CapBaseline> {
        self.baselines.values()
    }

    /// Compare a freshly computed message set against the stored
    /// baseline for a world. `None` when there is no baseline —
    /// a first connect, which never reaches the notice (#5959 §2
    /// step 5).
    #[must_use]
    pub fn evaluate(&self, world_id: &str, new: &BTreeSet<&str>) -> Option<CapDiff> {
        self.baselines
            .get(world_id)
            .map(|baseline| baseline.evaluate(new))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::cap_class::{
        CLASS_CHAT_READ, CLASS_CHAT_WRITE, CLASS_DEVICE_LOCATION, CLASS_UNKNOWN,
    };

    const AT: &str = "2026-11-23T06:00:00Z";

    fn set(classes: &[&str]) -> BTreeSet<String> {
        classes.iter().copied().map(ToOwned::to_owned).collect()
    }

    fn refs<'a>(classes: &[&'a str]) -> BTreeSet<&'a str> {
        classes.iter().copied().collect()
    }

    fn baseline() -> CapBaseline {
        CapBaseline::establish(
            "world-1",
            "release-a",
            set(&[CLASS_CHAT_READ, CLASS_CHAT_WRITE]),
            1,
            BaselineHistoryKind::Accept,
            AT,
        )
    }

    #[test]
    fn establish_records_the_first_decision_and_a_current_key() {
        let baseline = baseline();
        assert!(baseline.key_is_current());
        assert_eq!(baseline.history.len(), 1);
        assert_eq!(baseline.history[0].kind, BaselineHistoryKind::Accept);
        assert_eq!(baseline.history[0].message_set, baseline.message_set);
        // The accepted set compares silent against itself (prompt
        // iff trigger, #12921 notice-level oracle).
        assert!(
            !baseline
                .evaluate(&refs(&[CLASS_CHAT_READ, CLASS_CHAT_WRITE]))
                .triggers()
        );
        // And an expansion names exactly the new class.
        let diff = baseline.evaluate(&refs(&[
            CLASS_CHAT_READ,
            CLASS_CHAT_WRITE,
            CLASS_DEVICE_LOCATION,
        ]));
        assert!(diff.triggers());
        assert_eq!(diff.new_classes, [CLASS_DEVICE_LOCATION]);
    }

    #[test]
    fn baseline_key_changes_with_every_keyed_field() {
        let base = baseline();
        let other_world = CapBaseline::establish(
            "world-2",
            "release-a",
            set(&[CLASS_CHAT_READ, CLASS_CHAT_WRITE]),
            1,
            BaselineHistoryKind::Accept,
            AT,
        );
        let other_release = CapBaseline::establish(
            "world-1",
            "release-b",
            set(&[CLASS_CHAT_READ, CLASS_CHAT_WRITE]),
            1,
            BaselineHistoryKind::Accept,
            AT,
        );
        let other_set = CapBaseline::establish(
            "world-1",
            "release-a",
            set(&[CLASS_CHAT_READ]),
            1,
            BaselineHistoryKind::Accept,
            AT,
        );
        let other_table = CapBaseline::establish(
            "world-1",
            "release-a",
            set(&[CLASS_CHAT_READ, CLASS_CHAT_WRITE]),
            2,
            BaselineHistoryKind::Accept,
            AT,
        );
        for other in [other_world, other_release, other_set, other_table] {
            assert_ne!(base.baseline_key, other.baseline_key);
            assert!(other.key_is_current());
        }
    }

    #[test]
    fn per_class_denial_excludes_exactly_that_class_from_the_remembered_set() {
        // #12921 notice-level oracle: "Connect and remember" after
        // denying chat.write remembers N ∖ {chat.write} — and the
        // denied class prompts again next time (#5959 §4).
        let mut baseline = baseline();
        baseline.accept_minus_denied(
            "release-b",
            set(&[CLASS_CHAT_READ, CLASS_CHAT_WRITE, CLASS_DEVICE_LOCATION]),
            &refs(&[CLASS_CHAT_WRITE]),
            1,
            AT,
        );
        assert_eq!(
            baseline.message_set(),
            refs(&[CLASS_CHAT_READ, CLASS_DEVICE_LOCATION])
        );
        let diff = baseline.evaluate(&refs(&[
            CLASS_CHAT_READ,
            CLASS_CHAT_WRITE,
            CLASS_DEVICE_LOCATION,
        ]));
        assert!(diff.triggers());
        assert_eq!(diff.new_classes, [CLASS_CHAT_WRITE]);
        assert!(baseline.key_is_current());
    }

    #[test]
    fn narrowing_on_shrink_and_reprompt_on_readdition() {
        // #12921 notice-level oracle: a covered shrink narrows the
        // baseline silently; the dropped class, re-added later, is
        // an expansion and prompts (#5959 §5).
        let mut baseline = baseline();
        assert!(baseline.narrow("release-b", set(&[CLASS_CHAT_READ]), 1, AT));
        assert_eq!(baseline.message_set(), refs(&[CLASS_CHAT_READ]));
        assert_eq!(
            baseline.history.last().map(|entry| entry.kind),
            Some(BaselineHistoryKind::Narrow)
        );
        let diff = baseline.evaluate(&refs(&[CLASS_CHAT_READ, CLASS_CHAT_WRITE]));
        assert!(diff.triggers());
        assert_eq!(diff.new_classes, [CLASS_CHAT_WRITE]);
        // An uncovered set never narrows.
        assert!(!baseline.narrow(
            "release-c",
            set(&[CLASS_CHAT_READ, CLASS_DEVICE_LOCATION]),
            1,
            AT
        ));
        assert_eq!(baseline.message_set(), refs(&[CLASS_CHAT_READ]));
        // A covered reconnect with the same set writes nothing.
        assert!(!baseline.narrow("release-c", set(&[CLASS_CHAT_READ]), 1, AT));
        assert_eq!(baseline.history.len(), 2);
    }

    #[test]
    fn yolo_auto_accept_records_history_and_never_leaves_a_stale_baseline() {
        // #12921 notice-level oracle (#5959 §5): under YOLO the
        // bookkeeping still happens, so the new set compares silent
        // afterwards and the history shows the auto-accept.
        let mut baseline = baseline();
        baseline.yolo_auto_accept(
            "release-b",
            set(&[CLASS_CHAT_READ, CLASS_DEVICE_LOCATION]),
            1,
            AT,
        );
        assert_eq!(
            baseline.history.last().map(|entry| entry.kind),
            Some(BaselineHistoryKind::YoloAutoAccept)
        );
        assert!(
            !baseline
                .evaluate(&refs(&[CLASS_CHAT_READ, CLASS_DEVICE_LOCATION]))
                .triggers()
        );
        assert!(baseline.key_is_current());
    }

    #[test]
    fn grant_page_revoke_removes_the_class_and_reprompts_on_return() {
        let mut baseline = baseline();
        assert!(baseline.revoke_class(CLASS_CHAT_WRITE, AT));
        assert_eq!(
            baseline.history.last().map(|entry| entry.kind),
            Some(BaselineHistoryKind::GrantPageEdit)
        );
        let diff = baseline.evaluate(&refs(&[CLASS_CHAT_READ, CLASS_CHAT_WRITE]));
        assert!(diff.triggers());
        assert_eq!(diff.new_classes, [CLASS_CHAT_WRITE]);
        // Revoking what is not there changes nothing.
        assert!(!baseline.revoke_class(CLASS_DEVICE_LOCATION, AT));
        assert!(baseline.key_is_current());
    }

    #[test]
    fn invite_pin_establishes_a_baseline_that_covers_the_pinned_set() {
        // #5959 §7: a pin seeds B at or below the set actually
        // joined; the covered first join needs no blocking prompt.
        let mut store = BaselineStore::new();
        assert!(
            store
                .evaluate("world-1", &refs(&[CLASS_CHAT_READ]))
                .is_none()
        );
        store.insert(CapBaseline::establish(
            "world-1",
            "release-a",
            set(&[CLASS_CHAT_READ]),
            1,
            BaselineHistoryKind::Pin,
            AT,
        ));
        let diff = store
            .evaluate("world-1", &refs(&[CLASS_CHAT_READ]))
            .expect("baseline recorded");
        assert!(!diff.triggers());
        assert_eq!(
            store.get("world-1").map(|b| b.history[0].kind),
            Some(BaselineHistoryKind::Pin)
        );
    }

    #[test]
    fn the_record_serializes_in_the_designs_shape() {
        // #5959 §1's record, field for field, with the history
        // kinds spelled as the design spells them.
        let baseline = baseline();
        let value = serde_json::to_value(&baseline).expect("serializes");
        assert_eq!(value["world_id"], "world-1");
        assert_eq!(value["release_identity"], "release-a");
        assert_eq!(value["table_version"], 1);
        assert_eq!(
            value["baseline_key"],
            serde_json::Value::String(baseline.baseline_key.clone())
        );
        assert_eq!(value["history"][0]["kind"], "accept");
        let round_tripped: CapBaseline = serde_json::from_value(value).expect("round-trips");
        assert_eq!(round_tripped, baseline);
        assert!(round_tripped.key_is_current());
    }

    #[test]
    fn connect_verdict_covers_first_connect_short_circuit_narrowing_and_hold() {
        // Oracle: #5999 §4/#5959 §2 — no baseline enters first-connect
        // disclosure; an identical release/table skips recomputation.
        assert_eq!(
            evaluate_connect(None, "release-a", Some(&refs(&[CLASS_CHAT_READ]))),
            ConnectVerdict::FirstConnect {
                message_set: vec![CLASS_CHAT_READ.to_owned()],
            }
        );
        assert_eq!(
            evaluate_connect(Some(&baseline()), "release-a", None),
            ConnectVerdict::SameReleaseAndTable
        );

        // Oracle: #5999 §4 rule 1/#5959 §5 path 4 — covered sets
        // proceed, and a strict shrink is marked for narrowing.
        assert_eq!(
            evaluate_connect(
                Some(&baseline()),
                "release-b",
                Some(&refs(&[CLASS_CHAT_READ])),
            ),
            ConnectVerdict::Proceed {
                message_set: vec![CLASS_CHAT_READ.to_owned()],
                narrowed: true,
            }
        );

        // Oracle: #5999 §6 — an uncovered reconnect holds and names
        // exactly the newly uncovered class.
        assert_eq!(
            evaluate_connect(
                Some(&baseline()),
                "release-b",
                Some(&refs(&[CLASS_CHAT_READ, CLASS_DEVICE_LOCATION])),
            ),
            ConnectVerdict::Hold(CapDiff {
                new_classes: vec![CLASS_DEVICE_LOCATION.to_owned()],
            })
        );
    }

    #[test]
    fn connect_verdict_fails_closed_when_computation_is_unavailable() {
        // Oracle: #5999 §7 invariant 10 — missing comparison input is a
        // hold whose sole disclosed class is unknown, even on first connect.
        let accepted = baseline();
        for baseline in [None, Some(&accepted)] {
            assert_eq!(
                evaluate_connect(baseline, "release-b", None),
                ConnectVerdict::Hold(CapDiff {
                    new_classes: vec![CLASS_UNKNOWN.to_owned()],
                })
            );
        }
    }
}
