//! The capability-change notice surface — #5959 §3–§4's data side
//! (implementation issue #12921).
//!
//! This module is the pure decision content of the notice, composable
//! with both neighbours and overlapping neither:
//!
//! - [`crate::cap_diff`] computes the trigger (`CapDiff` — the new
//!   classes `N ∖ B`);
//! - [`crate::cap_baseline`] owns the baseline record and its five
//!   re-baselining transitions;
//! - this module turns a [`CapDiff`] into the notice the user sees:
//!   delta-only headline lines in the client's fixed class text,
//!   client-side / server-side groups kept separate, the three choices
//!   with no default, per-class denial composing per #5960 §2, the
//!   grant-on-use honesty line, and the fail-closed `unknown` notice.
//!
//! The live connection hold, the persistence of the store, and the
//! actual `id-web` rendering of both the notice and the grant page
//! are follow-on work — this module returns data the renderers read,
//! never pixels.
//!
//! ```text
//! new_classes (CapDiff) ──▶ NoticeLine per class (client table text)
//!                           ├── client_side (decision controls)
//!                           └── server_side (disclosure only, no controls)
//! ```

use std::collections::BTreeSet;

use crate::cap_baseline::{BaselineHistoryEntry, BaselineStore};
use crate::cap_class::resolve_cap_class;
use crate::cap_diff::{CapDiff, uncomputable_diff};
use crate::world_caps::{ProviderRegistry, ProviderSide};

/// The standing honesty line carried wherever a world's policy is
/// grant-on-use (#5959 §3.3): no reconnect notice can cover grants
/// minted mid-connection, and the notice never claims a guarantee it
/// does not provide.
pub const GRANT_ON_USE_HONESTY_LINE: &str = "This world may ask for more while you are connected; those requests are decided as they happen, not by this notice.";

/// The honesty line for a world's [`crate::world_caps::CapsReport`]
/// policy: present exactly when the policy is grant-on-use.
#[must_use]
pub fn honesty_line_for_policy(policy: &str) -> Option<&'static str> {
    if policy == "grant-on-use" {
        Some(GRANT_ON_USE_HONESTY_LINE)
    } else {
        None
    }
}

/// One delta-only headline line: exactly one new class, in the
/// client's fixed text for that class id (#5959 §3.1). The registry
/// entry supplies the id, never the sentence — a server cannot soften
/// the wording of the warning it triggers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NoticeLine {
    /// The new class this line names.
    pub class: String,
    /// The client's fixed sentence for the class.
    pub message: String,
    /// Whether this line carries decision controls. True only for
    /// client-side lines; server-side lines are disclosure only, and
    /// their absence of buttons is itself the label (#5959 §3.1, §4).
    pub deniable: bool,
    /// Whether the world declares the class load-bearing: denying it
    /// is equivalent to Don't connect for this attempt, and the
    /// notice says so before the user commits (#5959 §4, #5960 §2).
    /// A line whose every requested name is optional degrades
    /// gracefully instead.
    pub load_bearing: bool,
}

/// The names in `declared` behind one new class: the non-optional
/// requested names mapping to it, with `*` expanded exactly as
/// [`crate::cap_diff::message_set`] expands it. The notice groups by
/// whose capability is exercised (#5960 §1), and names — not classes
/// — are what carry a side.
fn names_for_class<'a>(
    registry: &ProviderRegistry,
    declared: &'a BTreeSet<String>,
    optional: &BTreeSet<String>,
    class: &str,
) -> Vec<&'a str> {
    use crate::world_caps::is_sensor_cap;

    let mut names: Vec<&str> = Vec::new();
    if declared.contains("*") {
        for entry in registry.entries() {
            if !is_sensor_cap(entry.name) && !optional.contains(entry.name) && entry.class == class
            {
                names.push(entry.name);
            }
        }
    }
    for name in declared {
        if name != "*"
            && !optional.contains(name.as_str())
            && registry.class_for_name(name).id == class
        {
            names.push(name.as_str());
        }
    }
    names.sort_unstable();
    names.dedup();
    names
}

/// Split one new class's headline line into the client-side and
/// server-side groups (#5959 §3.1, #5960 §1: the two groups are never
/// merged). A class with requested names on both sides appears in
/// both — each aspect discloses its own party.
fn lines_for_class(
    registry: &ProviderRegistry,
    declared: &BTreeSet<String>,
    optional: &BTreeSet<String>,
    load_bearing_classes: &BTreeSet<&str>,
    class: &str,
) -> (Option<NoticeLine>, Option<NoticeLine>) {
    let message = resolve_cap_class(class).message.unwrap_or(class).to_owned();
    let names = names_for_class(registry, declared, optional, class);
    // Whether the world declares the class load-bearing (#5960 §2):
    // a declaration of the world's manifest, supplied by the caller
    // — never inferred from the registry. Only a declared
    // load-bearing class turns a denial into a refusal.
    let load_bearing = load_bearing_classes.contains(class);
    let side_of = |name: &&str| {
        registry
            .get(name)
            .map(|manifest| manifest.side)
            // An unregistered name maps to `unknown` and is
            // fail-closed; its side is unknowable, so it lands with
            // the decision controls rather than in disclosure-only.
            .unwrap_or(ProviderSide::Client)
    };
    let client = names
        .iter()
        .any(|name| side_of(name) == ProviderSide::Client);
    let server = names.iter().any(|name| side_of(name) == ProviderSide::Host);
    let client_line = client.then(|| NoticeLine {
        class: class.to_owned(),
        message: message.clone(),
        deniable: true,
        load_bearing,
    });
    let server_line = server.then(|| NoticeLine {
        class: class.to_owned(),
        message,
        deniable: false,
        load_bearing,
    });
    (client_line, server_line)
}

/// The notice content for one world: the delta-only lines of a
/// [`CapDiff`], grouped client-side / server-side (#5959 §3).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapNotice {
    /// The world the user is connecting to (headline names it).
    pub world_name: String,
    /// New client-side classes: decision controls live here.
    pub client_side: Vec<NoticeLine>,
    /// New server-side classes: disclosure only, decided by the
    /// server's admin (#5959 §6) — never granted or denied here.
    pub server_side: Vec<NoticeLine>,
    /// True where the world's `CapsReport.policy` is grant-on-use:
    /// the notice carries [`GRANT_ON_USE_HONESTY_LINE`] (#5959 §3.3).
    pub grant_on_use: bool,
    /// True for the fail-closed form (#5959 §3.2): the comparison
    /// could not be computed, so the single `unknown` line stands in
    /// for a delta nobody understood.
    pub uncomputable: bool,
}

impl CapNotice {
    /// Whether the notice prompts: exactly the trigger — a new class
    /// on either side (#12921 notice-level oracle: prompt iff
    /// trigger).
    #[must_use]
    pub fn should_prompt(&self) -> bool {
        !self.client_side.is_empty() || !self.server_side.is_empty()
    }

    /// The classes a connecting choice may deny: the new client-side
    /// classes only. Server-side lines are disclosure only and carry
    /// no controls (#5959 §4).
    #[must_use]
    pub fn deniable_classes(&self) -> BTreeSet<&str> {
        self.client_side
            .iter()
            .filter(|line| line.deniable)
            .map(|line| line.class.as_str())
            .collect()
    }
}

/// Build the notice for a computed comparison: one line per class in
/// [`CapDiff::new_classes`], grouped by side, with `policy` the
/// world's `CapsReport.policy` verbatim (`"deny"` or
/// `"grant-on-use"`). `load_bearing_classes` carries the world's
/// manifest load-bearing declarations (#5960 §2) as class ids —
/// empty when the manifest declares none.
#[must_use]
pub fn notice_for_diff(
    world_name: &str,
    registry: &ProviderRegistry,
    declared: &BTreeSet<String>,
    optional: &BTreeSet<String>,
    load_bearing_classes: &BTreeSet<&str>,
    diff: &CapDiff,
    policy: &str,
) -> CapNotice {
    let mut client_side = Vec::new();
    let mut server_side = Vec::new();
    for class in &diff.new_classes {
        let (client, server) =
            lines_for_class(registry, declared, optional, load_bearing_classes, class);
        if let Some(line) = client {
            client_side.push(line);
        }
        if let Some(line) = server {
            server_side.push(line);
        }
    }
    CapNotice {
        world_name: world_name.to_owned(),
        client_side,
        server_side,
        grant_on_use: honesty_line_for_policy(policy).is_some(),
        uncomputable: false,
    }
}

/// The fail-closed notice for a comparison that cannot be computed
/// (#5959 §3.2, #5999 §7 invariant 10): missing manifest data, an
/// unparseable class, version skew. Silence requires understanding;
/// an unparseable world is a prompting world. The single `unknown`
/// line carries decision controls — declining it leaves the user
/// exactly where they were.
#[must_use]
pub fn uncomputable_notice(world_name: &str, policy: &str) -> CapNotice {
    let diff = uncomputable_diff();
    let message = resolve_cap_class(&diff.new_classes[0])
        .message
        .unwrap_or("unknown")
        .to_owned();
    CapNotice {
        world_name: world_name.to_owned(),
        client_side: vec![NoticeLine {
            class: diff.new_classes[0].clone(),
            message,
            deniable: true,
            load_bearing: true,
        }],
        server_side: Vec::new(),
        grant_on_use: honesty_line_for_policy(policy).is_some(),
        uncomputable: true,
    }
}

/// Exactly the three choices of #5959 §4 — and no fourth, and no
/// default. There is deliberately **no `Default` impl** on this enum:
/// a default on a warning is an implicit-accept path, and implicit
/// accept is the silent upgrade the requirement forbids. Focus lands
/// on the headline, not on a control; the caller must name a choice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoticeChoice {
    /// Connect holding `N`; the baseline is unchanged, so the same
    /// change prompts again next time (Android "Only this time").
    ConnectOnce,
    /// Connect holding `N` and re-baseline to what was accepted:
    /// `B := N ∖ {denied}` — the only notice path that raises the
    /// baseline, always an explicit act.
    ConnectAndRemember,
    /// Do not connect. The world stays installed, the baseline and
    /// prior grants are untouched.
    DontConnect,
}

/// What applying a [`NoticeChoice`] decides.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppliedChoice {
    /// Whether the connection proceeds.
    pub connects: bool,
    /// The message set to remember, if the choice re-baselines. `None`
    /// for choices that leave `B` untouched.
    pub remembered: Option<BTreeSet<String>>,
    /// True when a denied class the world declares load-bearing
    /// refused the attempt: equivalent to [`NoticeChoice::DontConnect`]
    /// for this attempt, and the notice says so (#5959 §4).
    pub load_bearing_refusal: bool,
}

/// Apply a choice to a notice: per-class denial composes with the two
/// connecting choices (#5959 §4, #5960 §2).
///
/// `new` is the message set `N` the connection would hold;
/// `denied_classes` names the new client-side classes the user
/// denied. Denying a class excludes exactly that class from the
/// remembered set — a denied class was never accepted, so if the
/// world still requests it, it prompts again next time. Denials of
/// classes outside [`CapNotice::deniable_classes`] are ignored:
/// server-side lines carry no controls.
#[must_use]
pub fn apply_choice(
    choice: NoticeChoice,
    notice: &CapNotice,
    new: &BTreeSet<String>,
    denied_classes: &BTreeSet<&str>,
) -> AppliedChoice {
    match choice {
        NoticeChoice::DontConnect => AppliedChoice {
            connects: false,
            remembered: None,
            load_bearing_refusal: false,
        },
        NoticeChoice::ConnectOnce => AppliedChoice {
            connects: true,
            remembered: None,
            load_bearing_refusal: false,
        },
        NoticeChoice::ConnectAndRemember => {
            let deniable = notice.deniable_classes();
            let denied: BTreeSet<&str> = denied_classes
                .iter()
                .copied()
                .filter(|class| deniable.contains(class))
                .collect();
            let load_bearing_refusal = notice
                .client_side
                .iter()
                .any(|line| line.load_bearing && denied.contains(line.class.as_str()));
            if load_bearing_refusal {
                return AppliedChoice {
                    connects: false,
                    remembered: None,
                    load_bearing_refusal: true,
                };
            }
            let remembered: BTreeSet<String> = new
                .iter()
                .filter(|class| !denied.contains(class.as_str()))
                .cloned()
                .collect();
            AppliedChoice {
                connects: true,
                remembered: Some(remembered),
                load_bearing_refusal: false,
            }
        }
    }
}

/// One world's row on the grant page: its baseline plus the full
/// history in display order (oldest first — the order the record
/// stores, suitable for rendering as a timeline). The actual `id-web`
/// rendering is follow-on work; this is the data it reads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GrantPageWorld {
    /// The world instance id.
    pub world_id: String,
    /// The release identity the baseline was recorded against.
    pub release_identity: String,
    /// The class-table version the message set was computed under.
    pub table_version: u32,
    /// The baseline key over the record's current fields.
    pub baseline_key: String,
    /// The accepted message set, one line per class in the client's
    /// fixed text.
    pub current: Vec<NoticeLine>,
    /// Every transition that produced this record, oldest first.
    pub history: Vec<BaselineHistoryEntry>,
}

/// List a store's baselines with their full histories in display
/// order: worlds in world-id order, each world's history oldest
/// first (#5959 §1, #5960 §5.1).
#[must_use]
pub fn grant_page_listing(store: &BaselineStore) -> Vec<GrantPageWorld> {
    store
        .baselines()
        .map(|baseline| {
            let current = baseline
                .message_set
                .iter()
                .map(|class| {
                    let message = resolve_cap_class(class).message.unwrap_or(class).to_owned();
                    NoticeLine {
                        class: class.clone(),
                        message,
                        deniable: true,
                        load_bearing: false,
                    }
                })
                .collect();
            GrantPageWorld {
                world_id: baseline.world_id.clone(),
                release_identity: baseline.release_identity.clone(),
                table_version: baseline.table_version,
                baseline_key: baseline.baseline_key.clone(),
                current,
                history: baseline.history.clone(),
            }
        })
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::cap_baseline::{BaselineHistoryKind, CapBaseline};
    use crate::cap_class::{
        CAP_CLASS_TABLE_VERSION, CLASS_CHAT_READ, CLASS_CHAT_WRITE, CLASS_DEVICE_LOCATION,
        CLASS_NET_OUT, CLASS_UNKNOWN,
    };
    use crate::world_caps::PROVIDER_REGISTRY;

    const AT: &str = "2026-11-23T06:00:00Z";

    fn names(list: &[&str]) -> BTreeSet<String> {
        list.iter().copied().map(ToOwned::to_owned).collect()
    }

    fn refs<'a>(list: &[&'a str]) -> BTreeSet<&'a str> {
        list.iter().copied().collect()
    }

    fn baseline(classes: &[&str]) -> CapBaseline {
        CapBaseline::establish(
            "world-1",
            "release-a",
            classes.iter().copied().map(ToOwned::to_owned).collect(),
            CAP_CLASS_TABLE_VERSION,
            BaselineHistoryKind::Accept,
            AT,
        )
    }

    #[test]
    fn notice_names_only_the_delta_grouped_by_side() {
        // Baseline covers chat.read; the world now also reaches
        // net.out (host side, via http.fetch) and unknown (client
        // side, via client.file.read). chat.read itself is silent.
        let accepted = baseline(&[CLASS_CHAT_READ]);
        let declared = names(&["chat.history", "http.fetch", "client.file.read"]);
        let new = crate::cap_diff::message_set(&PROVIDER_REGISTRY, &declared, &BTreeSet::new());
        let diff = accepted.evaluate(&new);
        assert!(diff.triggers());
        let notice = notice_for_diff(
            "world-1",
            &PROVIDER_REGISTRY,
            &declared,
            &BTreeSet::new(),
            &BTreeSet::new(),
            &diff,
            "deny",
        );
        assert!(notice.should_prompt());
        assert!(!notice.uncomputable);
        assert!(!notice.grant_on_use);
        assert_eq!(
            notice
                .server_side
                .iter()
                .map(|line| line.class.as_str())
                .collect::<Vec<_>>(),
            [CLASS_NET_OUT]
        );
        assert_eq!(
            notice
                .client_side
                .iter()
                .map(|line| line.class.as_str())
                .collect::<Vec<_>>(),
            [CLASS_UNKNOWN]
        );
        // Server-side lines are disclosure only; client-side lines
        // carry the controls.
        assert!(notice.server_side.iter().all(|line| !line.deniable));
        assert!(notice.client_side.iter().all(|line| line.deniable));
        // Lines speak the client table's sentences, never hashes or
        // raw names.
        for line in notice.client_side.iter().chain(&notice.server_side) {
            assert_eq!(
                line.message,
                resolve_cap_class(&line.class).message.unwrap()
            );
        }
    }

    #[test]
    fn silent_reconnect_builds_a_notice_that_does_not_prompt() {
        // #12921 notice-level oracle, silent direction: no new class,
        // no prompt.
        let accepted = baseline(&[CLASS_CHAT_READ, CLASS_CHAT_WRITE]);
        let declared = names(&["chat.history", "chat.say"]);
        let new = crate::cap_diff::message_set(&PROVIDER_REGISTRY, &declared, &BTreeSet::new());
        let diff = accepted.evaluate(&new);
        assert!(!diff.triggers());
        let notice = notice_for_diff(
            "world-1",
            &PROVIDER_REGISTRY,
            &declared,
            &BTreeSet::new(),
            &BTreeSet::new(),
            &diff,
            "deny",
        );
        assert!(!notice.should_prompt());
    }

    #[test]
    fn uncomputable_notice_prompts_with_the_unknown_line() {
        // #12921 notice-level oracle, fail-closed: an uncomputable
        // comparison is a prompt carrying `unknown` (#5959 §3.2).
        let notice = uncomputable_notice("world-1", "deny");
        assert!(notice.should_prompt());
        assert!(notice.uncomputable);
        assert_eq!(notice.client_side.len(), 1);
        assert_eq!(notice.server_side, [] as [cap_notice::NoticeLine; 0]);
        assert_eq!(notice.client_side[0].class, CLASS_UNKNOWN);
        assert_eq!(
            notice.client_side[0].message,
            resolve_cap_class(CLASS_UNKNOWN).message.unwrap()
        );
    }

    #[test]
    fn per_class_denial_excludes_exactly_that_class_from_the_remembered_set() {
        // #12921 notice-level oracle (#5959 §4, #5960 §2): "Connect
        // and remember" after denying the new client-side class
        // remembers N minus exactly that class.
        let accepted = baseline(&[CLASS_CHAT_READ]);
        let declared = names(&["chat.history", "client.file.read", "http.fetch"]);
        let new_names =
            crate::cap_diff::message_set(&PROVIDER_REGISTRY, &declared, &BTreeSet::new());
        let diff = accepted.evaluate(&new_names);
        let notice = notice_for_diff(
            "world-1",
            &PROVIDER_REGISTRY,
            &declared,
            &BTreeSet::new(),
            &BTreeSet::new(),
            &diff,
            "deny",
        );
        assert!(notice.deniable_classes().contains(CLASS_UNKNOWN));
        assert!(!notice.deniable_classes().contains(CLASS_NET_OUT));
        let new: BTreeSet<String> = new_names.iter().map(ToString::to_string).collect();
        let applied = apply_choice(
            NoticeChoice::ConnectAndRemember,
            &notice,
            &new,
            &refs(&[CLASS_UNKNOWN]),
        );
        assert!(applied.connects);
        assert!(!applied.load_bearing_refusal);
        let remembered = applied.remembered.expect("remember choice re-baselines");
        assert!(!remembered.iter().any(|c| c == CLASS_UNKNOWN));
        assert!(remembered.iter().any(|c| c == CLASS_NET_OUT));
        assert!(remembered.iter().any(|c| c == CLASS_CHAT_READ));
        // The denied class prompts again next time: it is new against
        // the remembered set.
        let remembered_refs: BTreeSet<&str> = remembered.iter().map(String::as_str).collect();
        let again = crate::cap_diff::compare(&remembered_refs, &new_names);
        assert!(again.triggers());
        assert_eq!(again.new_classes, [CLASS_UNKNOWN]);
        // Denying a server-side class is ignored: those lines carry
        // no controls.
        let applied = apply_choice(
            NoticeChoice::ConnectAndRemember,
            &notice,
            &new,
            &refs(&[CLASS_NET_OUT]),
        );
        let remembered = applied.remembered.expect("remember choice re-baselines");
        assert!(remembered.iter().any(|c| c == CLASS_NET_OUT));
    }

    #[test]
    fn denying_a_load_bearing_class_refuses_the_attempt() {
        // #5960 §2: denying load-bearing is equivalent to Don't
        // connect for this attempt. The world's manifest declares the
        // unknown class load-bearing here.
        let accepted = baseline(&[CLASS_CHAT_READ]);
        let declared = names(&["chat.history", "client.file.read"]);
        let new_names =
            crate::cap_diff::message_set(&PROVIDER_REGISTRY, &declared, &BTreeSet::new());
        let diff = accepted.evaluate(&new_names);
        let notice = notice_for_diff(
            "world-1",
            &PROVIDER_REGISTRY,
            &declared,
            &BTreeSet::new(),
            &refs(&[CLASS_UNKNOWN]),
            &diff,
            "deny",
        );
        assert!(
            notice
                .client_side
                .iter()
                .any(|line| line.class == CLASS_UNKNOWN && line.load_bearing)
        );
        let new: BTreeSet<String> = new_names.iter().map(ToString::to_string).collect();
        let applied = apply_choice(
            NoticeChoice::ConnectAndRemember,
            &notice,
            &new,
            &refs(&[CLASS_UNKNOWN]),
        );
        assert!(!applied.connects);
        assert!(applied.load_bearing_refusal);
        assert_eq!(applied.remembered, None);
    }

    #[test]
    fn connect_once_and_dont_connect_leave_the_baseline_untouched() {
        // #5959 §4: refusal costs nothing; connecting once remembers
        // nothing.
        let accepted = baseline(&[CLASS_CHAT_READ]);
        let declared = names(&["chat.history", "chat.say"]);
        let new_names =
            crate::cap_diff::message_set(&PROVIDER_REGISTRY, &declared, &BTreeSet::new());
        let diff = accepted.evaluate(&new_names);
        let notice = notice_for_diff(
            "world-1",
            &PROVIDER_REGISTRY,
            &declared,
            &BTreeSet::new(),
            &BTreeSet::new(),
            &diff,
            "deny",
        );
        let new: BTreeSet<String> = new_names.iter().map(ToString::to_string).collect();
        for choice in [NoticeChoice::ConnectOnce, NoticeChoice::DontConnect] {
            let applied = apply_choice(choice, &notice, &new, &BTreeSet::new());
            assert_eq!(applied.connects, choice == NoticeChoice::ConnectOnce);
            assert_eq!(applied.remembered, None);
            assert!(!applied.load_bearing_refusal);
        }
    }

    #[test]
    fn honesty_line_rides_only_on_grant_on_use() {
        assert_eq!(
            honesty_line_for_policy("grant-on-use"),
            Some(GRANT_ON_USE_HONESTY_LINE)
        );
        assert_eq!(honesty_line_for_policy("deny"), None);
        let accepted = baseline(&[CLASS_CHAT_READ]);
        let declared = names(&["chat.history", "chat.say"]);
        let new = crate::cap_diff::message_set(&PROVIDER_REGISTRY, &declared, &BTreeSet::new());
        let diff = accepted.evaluate(&new);
        assert!(
            notice_for_diff(
                "world-1",
                &PROVIDER_REGISTRY,
                &declared,
                &BTreeSet::new(),
                &BTreeSet::new(),
                &diff,
                "grant-on-use",
            )
            .grant_on_use
        );
        assert!(
            !notice_for_diff(
                "world-1",
                &PROVIDER_REGISTRY,
                &declared,
                &BTreeSet::new(),
                &BTreeSet::new(),
                &diff,
                "deny",
            )
            .grant_on_use
        );
    }

    #[test]
    fn narrowing_then_readdition_prompts_through_the_notice() {
        // #12921 notice-level oracle at the notice layer: a covered
        // shrink narrows silently, then the dropped class re-added
        // prompts as the expansion it is (#5959 §5).
        let mut accepted = baseline(&[CLASS_CHAT_READ, CLASS_DEVICE_LOCATION]);
        assert!(
            accepted.narrow(
                "release-b",
                [CLASS_CHAT_READ]
                    .into_iter()
                    .map(ToOwned::to_owned)
                    .collect(),
                CAP_CLASS_TABLE_VERSION,
                AT,
            )
        );
        let declared = names(&["chat.history", "loc.position"]);
        let new = crate::cap_diff::message_set(&PROVIDER_REGISTRY, &declared, &BTreeSet::new());
        let diff = accepted.evaluate(&new);
        assert!(diff.triggers());
        let notice = notice_for_diff(
            "world-1",
            &PROVIDER_REGISTRY,
            &declared,
            &BTreeSet::new(),
            &BTreeSet::new(),
            &diff,
            "deny",
        );
        assert!(notice.should_prompt());
        assert!(
            notice
                .client_side
                .iter()
                .chain(&notice.server_side)
                .any(|line| line.class == CLASS_DEVICE_LOCATION)
        );
    }

    #[test]
    fn yolo_auto_accept_leaves_no_stale_baseline_behind() {
        // #12921 notice-level oracle (#5959 §5): under YOLO the
        // trigger is suppressed but the bookkeeping still happens, so
        // the notice for the same set afterwards is silent.
        let mut accepted = baseline(&[CLASS_CHAT_READ]);
        accepted.yolo_auto_accept(
            "release-b",
            [CLASS_CHAT_READ, CLASS_CHAT_WRITE]
                .into_iter()
                .map(ToOwned::to_owned)
                .collect(),
            CAP_CLASS_TABLE_VERSION,
            AT,
        );
        let declared = names(&["chat.history", "chat.say"]);
        let new = crate::cap_diff::message_set(&PROVIDER_REGISTRY, &declared, &BTreeSet::new());
        let notice = notice_for_diff(
            "world-1",
            &PROVIDER_REGISTRY,
            &declared,
            &BTreeSet::new(),
            &BTreeSet::new(),
            &accepted.evaluate(&new),
            "deny",
        );
        assert!(!notice.should_prompt());
    }

    #[test]
    fn grant_page_listing_orders_worlds_and_histories() {
        let mut store = BaselineStore::new();
        let mut second = CapBaseline::establish(
            "world-2",
            "release-a",
            [CLASS_CHAT_READ]
                .into_iter()
                .map(ToOwned::to_owned)
                .collect(),
            CAP_CLASS_TABLE_VERSION,
            BaselineHistoryKind::Accept,
            AT,
        );
        second.narrow(
            "release-b",
            [CLASS_CHAT_READ]
                .into_iter()
                .map(ToOwned::to_owned)
                .collect(),
            CAP_CLASS_TABLE_VERSION,
            AT,
        );
        let mut first = CapBaseline::establish(
            "world-1",
            "release-a",
            [CLASS_CHAT_READ, CLASS_CHAT_WRITE]
                .into_iter()
                .map(ToOwned::to_owned)
                .collect(),
            CAP_CLASS_TABLE_VERSION,
            BaselineHistoryKind::Accept,
            AT,
        );
        first.revoke_class(CLASS_CHAT_WRITE, AT);
        // Insert out of order: the listing still reads world-id
        // order, each history oldest first.
        store.insert(second);
        store.insert(first);
        let listing = grant_page_listing(&store);
        assert_eq!(listing.len(), 2);
        assert_eq!(listing[0].world_id, "world-1");
        assert_eq!(listing[1].world_id, "world-2");
        assert_eq!(
            listing[0].baseline_key,
            store.get("world-1").unwrap().baseline_key
        );
        assert_eq!(
            listing[0]
                .history
                .iter()
                .map(|entry| entry.kind)
                .collect::<Vec<_>>(),
            [
                BaselineHistoryKind::Accept,
                BaselineHistoryKind::GrantPageEdit
            ]
        );
        assert_eq!(
            listing[1]
                .history
                .iter()
                .map(|entry| entry.kind)
                .collect::<Vec<_>>(),
            [BaselineHistoryKind::Accept]
        );
        // Current lines carry the client table's sentences.
        assert_eq!(
            listing[0]
                .current
                .iter()
                .map(|line| line.class.as_str())
                .collect::<Vec<_>>(),
            [CLASS_CHAT_READ]
        );
        assert_eq!(
            listing[0].current[0].message,
            resolve_cap_class(CLASS_CHAT_READ).message.unwrap()
        );
    }
}
