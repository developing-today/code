//! Completion freshness policy — REL-09
//! (developing-today/code#9519; design parent #9209, program #9196).
//!
//! A completion claim is read, merged and reported against activity
//! that can move in between. This module owns the pure policy for
//! that boundary:
//!
//! * **Invalidating activity classes.** Only activity that can
//!   change the completion decision (reviews, review comments,
//!   explicit approval/block, close/reopen) invalidates a claim.
//!   Ordinary comments, label bookkeeping and pushes do not.
//! * **Watermark.** A completion read pins the highest activity
//!   revision it observed. The pinned value travels with the
//!   receipt, so a later reader can see exactly what the decision
//!   was based on.
//! * **Refresh → merge residual race.** If invalidating activity
//!   lands after the pinned read but before the merge is reported,
//!   the receipt must expose that freshness limit instead of
//!   claiming clean reconciliation. The merge is held for a fresh
//!   read rather than silently accepted.
//! * **Duplicate delivery.** Activity revisions are monotonic per
//!   work item; a redelivered event (same revision) is idempotent
//!   and never adds a new late class.
//! * **Post-terminal events.** Events after the completion boundary
//!   are reported as ignored; they never reopen an already-reported
//!   receipt.
//!
//! There is no global lock here: the watermark is a value, the
//! policy is a pure function of it plus the observed activity, and
//! nothing in this module reads, writes or waits.

/// Classes of activity that can arrive around a completion claim.
///
/// The set is deliberately small: it covers the events the design
/// names for REL-09 and leaves routing/dedupe to #9203 and cursor
/// reads to #9207.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ActivityClass {
    /// An ordinary issue/PR comment.
    Comment,
    /// A submitted review (approve, request changes or comment).
    Review,
    /// A line-level review comment.
    ReviewComment,
    /// An explicit approval outside a review body.
    ExplicitApproval,
    /// An explicit block outside a review body.
    ExplicitBlock,
    /// A code push to the reviewed branch.
    Push,
    /// A label add/remove that is not itself a decision.
    LabelChange,
    /// The work item was closed.
    Close,
    /// The work item was reopened.
    Reopen,
    /// An event type no mapping names yet. It holds a completion
    /// rather than slipping through as harmless; the receipt names
    /// it so a mapping can be added (REL-09 design record §3).
    Unclassified,
}

impl ActivityClass {
    /// Policy: does activity of this class invalidate a completion
    /// claim? Conservative hold only for classes that can change
    /// the decision; comments, pushes and label bookkeeping do not.
    pub fn invalidates_completion(self) -> bool {
        matches!(
            self,
            ActivityClass::Review
                | ActivityClass::ReviewComment
                | ActivityClass::ExplicitApproval
                | ActivityClass::ExplicitBlock
                | ActivityClass::Close
                | ActivityClass::Reopen
                | ActivityClass::Unclassified
        )
    }

    /// Policy: is this class terminal for the work item? A close or
    /// reopen ends the completion window.
    pub fn is_terminal(self) -> bool {
        matches!(self, ActivityClass::Close | ActivityClass::Reopen)
    }
}

/// Monotonic per-work activity revision. The numbered value is the
/// watermark; revisions are assigned by the event owner (#9203) and
/// only compared here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct ActivityRevision(pub u64);

/// One observed activity event, as read at the completion boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Activity {
    /// Monotonic revision of this event within the work item.
    pub revision: ActivityRevision,
    /// What kind of activity it is.
    pub class: ActivityClass,
    /// Human-readable summary carried through to the receipt.
    pub summary: String,
}

impl Activity {
    /// Convenience constructor for fixtures.
    pub fn new(revision: u64, class: ActivityClass, summary: &str) -> Self {
        Self {
            revision: ActivityRevision(revision),
            class,
            summary: summary.to_string(),
        }
    }
}

/// The pin a completion read takes before merging.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActivityWatermark {
    /// Highest activity revision the read observed.
    pub observed: ActivityRevision,
}

impl ActivityWatermark {
    /// Pin a read at `observed`.
    pub fn at(observed: u64) -> Self {
        Self {
            observed: ActivityRevision(observed),
        }
    }
}

/// Freshness of a completion receipt relative to its watermark.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Freshness {
    /// Nothing that can change the decision arrived after the pin.
    Current,
    /// Invalidating activity arrived after the pin and before the
    /// merge was reported; the receipt exposes the limit and the
    /// classes responsible.
    Limited {
        /// Revision the completion read was pinned to.
        observed: ActivityRevision,
        /// Highest revision seen at the completion boundary.
        current: ActivityRevision,
        /// Late invalidating classes, de-duplicated and ordered.
        late: Vec<ActivityClass>,
    },
}

/// The decision a completion boundary can report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionReceipt {
    /// Revision the completion read was pinned to.
    pub observed: ActivityRevision,
    /// Highest revision seen when the boundary was evaluated.
    pub current: ActivityRevision,
    /// Freshness verdict.
    pub freshness: Freshness,
    /// Late activity that was considered, de-duplicated by revision
    /// and ordered by revision. Non-invalidating late activity is
    /// carried for audit but does not limit freshness.
    pub late_activity: Vec<Activity>,
    /// True when the policy allows the merge/report to proceed
    /// without a fresh read.
    pub merge_allowed: bool,
}

impl CompletionReceipt {
    /// True when the receipt exposes a freshness limit.
    pub fn is_freshness_limited(&self) -> bool {
        matches!(self.freshness, Freshness::Limited { .. })
    }

    /// Post-terminal policy: an event observed after this receipt
    /// was reported is ignored. It never mutates or reopens the
    /// receipt; the caller records it on the work item instead.
    pub fn note_post_terminal(&self, activity: &Activity) -> PostTerminalOutcome {
        if activity.revision <= self.current {
            PostTerminalOutcome::AlreadyCovered
        } else {
            PostTerminalOutcome::IgnoredAfterCompletion
        }
    }
}

/// What an event observed after a reported completion does to the
/// work item (REL-09 design record §6). None of these reopens the
/// receipt, re-runs an effect or re-queues the work: re-queuing is a
/// lifecycle decision made through its own verb.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PostCompletionEffect {
    /// At or below the receipt's boundary: already covered, nothing
    /// to record.
    Covered,
    /// Recorded as an internal journal entry only.
    JournalOnly,
    /// Recorded as an internal journal entry plus an advisory triage
    /// hint on the work item; no base field changes.
    JournalAndTriageHint,
}

impl CompletionReceipt {
    /// Post-completion policy: what a later event does to the work
    /// item. Decisions people make after completion (reviews,
    /// blocks, a reopen, or anything unmapped) raise a triage hint;
    /// bookkeeping is journalled only.
    pub fn post_completion_effect(&self, activity: &Activity) -> PostCompletionEffect {
        if self.note_post_terminal(activity) == PostTerminalOutcome::AlreadyCovered {
            return PostCompletionEffect::Covered;
        }
        match activity.class {
            ActivityClass::Comment
            | ActivityClass::Push
            | ActivityClass::LabelChange
            | ActivityClass::ExplicitApproval
            | ActivityClass::Close => PostCompletionEffect::JournalOnly,
            ActivityClass::Review
            | ActivityClass::ReviewComment
            | ActivityClass::ExplicitBlock
            | ActivityClass::Reopen
            | ActivityClass::Unclassified => PostCompletionEffect::JournalAndTriageHint,
        }
    }
}

/// What the post-terminal policy did with an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PostTerminalOutcome {
    /// The event's revision is at or below the receipt's boundary;
    /// it was already covered (and de-duplicated) by the receipt.
    AlreadyCovered,
    /// The event arrived after the receipt; it is recorded but does
    /// not reopen the completion.
    IgnoredAfterCompletion,
}

/// Evaluate a completion boundary.
///
/// `watermark` is the pin taken before the merge; `activities` is
/// everything observed at the boundary, typically the read result
/// plus a refresh. Activity at or below the watermark is not late:
/// it was part of the decision's input.
///
/// The policy:
/// * de-duplicate by revision (a redelivered webhook is idempotent);
/// * treat only invalidating classes above the watermark as a
///   freshness limit;
/// * hold the merge (`merge_allowed = false`) when limited.
pub fn evaluate_completion(
    watermark: ActivityWatermark,
    activities: &[Activity],
) -> CompletionReceipt {
    let observed = watermark.observed;

    // De-duplicate by revision, keeping the first occurrence and
    // ordering by revision. Revision 0 is never a real event.
    let mut seen: Vec<Activity> = Vec::new();
    for activity in activities {
        if activity.revision <= ActivityRevision(0) {
            continue;
        }
        if !seen.iter().any(|a| a.revision == activity.revision) {
            seen.push(activity.clone());
        }
    }
    seen.sort_by_key(|a| a.revision);

    let current = seen
        .iter()
        .map(|a| a.revision)
        .max()
        .unwrap_or(observed)
        .max(observed);

    let late_activity: Vec<Activity> = seen.into_iter().filter(|a| a.revision > observed).collect();

    let mut late_classes: Vec<ActivityClass> = late_activity
        .iter()
        .filter(|a| a.class.invalidates_completion())
        .map(|a| a.class)
        .collect();
    late_classes.sort();
    late_classes.dedup();

    let freshness = if late_classes.is_empty() {
        Freshness::Current
    } else {
        Freshness::Limited {
            observed,
            current,
            late: late_classes,
        }
    };
    let merge_allowed = !matches!(freshness, Freshness::Limited { .. });

    CompletionReceipt {
        observed,
        current,
        freshness,
        late_activity,
        merge_allowed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The REL-09 oracle: a review injected between the completion
    /// read (refresh) and the merge is reported at the completion
    /// boundary, and the receipt exposes the freshness limit
    /// instead of claiming clean reconciliation.
    #[test]
    fn pmo_rel_late_review_after_refresh_is_reported_at_completion_boundary() {
        // The completion read pins revision 7.
        let watermark = ActivityWatermark::at(7);

        // A review lands after the read and before the merge is
        // reported.
        let late_review = Activity::new(8, ActivityClass::Review, "review submitted after refresh");

        let receipt = evaluate_completion(watermark, &[late_review.clone()]);

        assert_eq!(receipt.observed, ActivityRevision(7));
        assert_eq!(receipt.current, ActivityRevision(8));
        assert_eq!(
            receipt.freshness,
            Freshness::Limited {
                observed: ActivityRevision(7),
                current: ActivityRevision(8),
                late: vec![ActivityClass::Review],
            }
        );
        assert!(receipt.is_freshness_limited());
        assert!(
            !receipt.merge_allowed,
            "a late invalidating review must hold the merge for a fresh read"
        );
        assert_eq!(receipt.late_activity, vec![late_review]);
    }

    /// Negative: a tiny comment race is real activity but cannot
    /// change the decision, so the receipt stays current and the
    /// merge is allowed.
    #[test]
    fn pmo_rel_tiny_comment_race_does_not_limit_freshness() {
        let watermark = ActivityWatermark::at(4);
        let late_comment = Activity::new(5, ActivityClass::Comment, "thanks, looks fine");

        let receipt = evaluate_completion(watermark, &[late_comment]);

        assert_eq!(receipt.freshness, Freshness::Current);
        assert!(receipt.merge_allowed);
        // The comment is still carried for audit.
        assert_eq!(receipt.late_activity.len(), 1);
        assert_eq!(receipt.late_activity[0].class, ActivityClass::Comment);
    }

    /// Negative: delayed/duplicate webhook delivery is idempotent by
    /// revision — the same event twice is one late class, not two.
    #[test]
    fn pmo_rel_delayed_duplicate_webhook_delivery_is_idempotent() {
        let watermark = ActivityWatermark::at(2);
        let block = Activity::new(3, ActivityClass::ExplicitBlock, "changes requested");

        let once = evaluate_completion(watermark, &[block.clone()]);
        let twice = evaluate_completion(watermark, &[block.clone(), block]);

        assert_eq!(once.freshness, twice.freshness);
        assert_eq!(
            once.freshness,
            Freshness::Limited {
                observed: ActivityRevision(2),
                current: ActivityRevision(3),
                late: vec![ActivityClass::ExplicitBlock],
            }
        );
    }

    /// Negative: an event after the completion boundary is ignored
    /// and never reopens the reported receipt.
    #[test]
    fn pmo_rel_post_terminal_event_does_not_reopen_receipt() {
        let watermark = ActivityWatermark::at(9);
        let receipt = evaluate_completion(watermark, &[]);
        assert_eq!(receipt.freshness, Freshness::Current);

        let later = Activity::new(10, ActivityClass::Comment, "post-completion note");
        assert_eq!(
            receipt.note_post_terminal(&later),
            PostTerminalOutcome::IgnoredAfterCompletion
        );
        // The receipt itself is unchanged.
        assert_eq!(receipt.freshness, Freshness::Current);
        assert!(receipt.merge_allowed);

        let covered = Activity::new(9, ActivityClass::Comment, "already read");
        assert_eq!(
            receipt.note_post_terminal(&covered),
            PostTerminalOutcome::AlreadyCovered
        );
    }

    /// An event type nobody mapped holds the completion instead of
    /// passing as harmless (REL-09 design record §3).
    #[test]
    fn pmo_rel_unclassified_activity_holds_completion() {
        let watermark = ActivityWatermark::at(3);
        let odd = Activity::new(4, ActivityClass::Unclassified, "unmapped event type");

        let receipt = evaluate_completion(watermark, &[odd]);

        assert_eq!(
            receipt.freshness,
            Freshness::Limited {
                observed: ActivityRevision(3),
                current: ActivityRevision(4),
                late: vec![ActivityClass::Unclassified],
            }
        );
        assert!(!receipt.merge_allowed);
        assert!(!ActivityClass::Unclassified.is_terminal());
    }

    /// A push after the refresh is not an activity-gate matter: the
    /// receipt stays current, and the moved head is refused by the
    /// REL-06 expected-head-SHA precondition instead (§4).
    #[test]
    fn pmo_rel_push_after_refresh_is_left_to_head_sha_gate() {
        let watermark = ActivityWatermark::at(6);
        let push = Activity::new(7, ActivityClass::Push, "new commits");

        let receipt = evaluate_completion(watermark, &[push]);

        assert_eq!(receipt.freshness, Freshness::Current);
        assert!(receipt.merge_allowed);
        assert!(!ActivityClass::Push.invalidates_completion());
        assert_eq!(receipt.late_activity.len(), 1, "the push is kept for audit");
    }

    /// A review after completion raises a triage hint and leaves the
    /// receipt exactly as reported (§6).
    #[test]
    fn pmo_rel_post_completion_review_raises_hint_not_reopen() {
        let receipt = evaluate_completion(ActivityWatermark::at(5), &[]);
        let before = receipt.clone();

        let review = Activity::new(6, ActivityClass::Review, "changes requested");
        assert_eq!(
            receipt.post_completion_effect(&review),
            PostCompletionEffect::JournalAndTriageHint
        );
        let comment = Activity::new(7, ActivityClass::Comment, "thanks");
        assert_eq!(
            receipt.post_completion_effect(&comment),
            PostCompletionEffect::JournalOnly
        );
        let covered = Activity::new(5, ActivityClass::Review, "read before completion");
        assert_eq!(
            receipt.post_completion_effect(&covered),
            PostCompletionEffect::Covered
        );
        assert_eq!(
            receipt, before,
            "post-completion events never mutate the receipt"
        );
    }

    /// A reopen after completion is a hint for a person to decide,
    /// never an automatic re-queue (§6).
    #[test]
    fn pmo_rel_reopen_after_completion_never_requeues() {
        let receipt = evaluate_completion(ActivityWatermark::at(2), &[]);
        let reopen = Activity::new(3, ActivityClass::Reopen, "reopened");

        assert_eq!(
            receipt.post_completion_effect(&reopen),
            PostCompletionEffect::JournalAndTriageHint
        );
        assert_eq!(
            receipt.note_post_terminal(&reopen),
            PostTerminalOutcome::IgnoredAfterCompletion
        );
        assert!(receipt.merge_allowed);
        assert_eq!(receipt.freshness, Freshness::Current);
    }
}
