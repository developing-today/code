//! The person-wait guard (FDR-0013; settles FDR-0012 OQ-1).
//!
//! A block or park may wait on a person only as a **hands item**: a
//! step that needs a human body or a login only a person can finish,
//! written as one action. Anything else that would wait on a person is
//! a decision, and a decision is the worker's to make. This module is
//! the check `fleet queue block` and `fleet queue park` run before they
//! contact the broker.
//!
//! What counts as waiting on a person:
//!
//! * a block reason class in [`PERSON_WAIT_CLASSES`];
//! * a park ready-condition kind in [`PERSON_WAIT_CONDITION_KINDS`];
//! * a reason or blocker text that names the owner as the release
//!   ([`names_owner_wait`]): the ownership handoff FDR-0012 removed, or
//!   the owner decision FDR-0013 removes.
//!
//! The text match is deliberately a short list of phrases lanes have
//! actually written. It is not a classifier: a wait it misses is still
//! against the rule, and a real hands item it catches passes by saying
//! so with `--hands` and `--action`.

/// Block reason classes whose release is a person.
pub const PERSON_WAIT_CLASSES: [&str; 3] = ["escalated-to-user", "drew_decision", "person-answer"];

/// Park ready-condition kinds whose release is a person.
pub const PERSON_WAIT_CONDITION_KINDS: [&str; 1] = ["person_answer"];

/// What a person's hands can be needed for.
pub const HANDS_KINDS: [&str; 4] = ["physical", "interactive-login", "payment", "signature"];

/// An action is one sentence, not a brief.
pub const MAX_ACTION_CHARS: usize = 200;

/// Phrases that make the owner the release condition. Lowercase.
const OWNER_WAIT_PHRASES: [&str; 18] = [
    "ownership handoff",
    "owner handoff",
    "owner release",
    "awaiting reassignment",
    "awaiting owner",
    "awaiting drew",
    "waiting on drew",
    "waiting for drew",
    "waiting on the owner",
    "waiting for the owner",
    "needs drew",
    "needs the owner",
    "owner decides",
    "owner decision",
    "drew decides",
    "drew decision",
    "owner to re-scope",
    "owner approval",
];

/// The phrase in `text` that names the owner as the release, if any.
pub fn names_owner_wait(text: &str) -> Option<&'static str> {
    let lower = text.to_lowercase();
    OWNER_WAIT_PHRASES
        .iter()
        .find(|phrase| lower.contains(*phrase))
        .copied()
}

/// One block or park, as far as this check cares.
#[derive(Debug, Clone, Copy, Default)]
pub struct Wait<'a> {
    pub reason_class: Option<&'a str>,
    pub condition_kind: Option<&'a str>,
    /// The blocker or reason text.
    pub text: &'a str,
    pub hands: Option<&'a str>,
    pub action: Option<&'a str>,
}

/// A hands item that passed: its kind and its action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hands {
    pub kind: String,
    pub action: String,
}

const WHAT_TO_DO: &str = "A choice is yours to make: decide it, record the decision on the issue, \
and keep working (FDR-0013; .agents/skills/decide-and-proceed). Only a hands item may wait on a \
person: add --hands <physical|interactive-login|payment|signature> and --action \"<one sentence a \
person can do in five minutes>\". It blocks this step only; take the next task.";

/// Check one block or park.
///
/// `Ok(None)`: it does not wait on a person. `Ok(Some(hands))`: it
/// waits on a person's hands, validly. `Err(message)`: refused; the
/// message says what to do instead.
pub fn check(wait: &Wait) -> Result<Option<Hands>, String> {
    let why = if let Some(class) = wait
        .reason_class
        .filter(|c| PERSON_WAIT_CLASSES.contains(c))
    {
        Some(format!("reason class '{class}'"))
    } else if let Some(kind) = wait
        .condition_kind
        .filter(|k| PERSON_WAIT_CONDITION_KINDS.contains(k))
    {
        Some(format!("ready condition '{kind}'"))
    } else {
        names_owner_wait(wait.text).map(|phrase| format!("the text says \"{phrase}\""))
    };
    let hands = wait.hands.map(str::trim).filter(|h| !h.is_empty());
    let action = wait.action.map(str::trim).filter(|a| !a.is_empty());
    let Some(why) = why else {
        return match (hands, action) {
            (None, None) => Ok(None),
            _ => Err(
                "--hands and --action describe a wait on a person: use them with \
                 --reason-class escalated-to-user (block) or a person_answer ready condition (park)"
                    .to_string(),
            ),
        };
    };
    let (Some(kind), Some(action)) = (hands, action) else {
        return Err(format!(
            "refusing to wait on a person ({why}). {WHAT_TO_DO}"
        ));
    };
    if !HANDS_KINDS.contains(&kind) {
        return Err(format!(
            "--hands '{kind}' is not a hands kind: one of {}",
            HANDS_KINDS.join(", ")
        ));
    }
    if action.contains('\n') || action.chars().count() > MAX_ACTION_CHARS {
        return Err(format!(
            "--action must be one sentence of at most {MAX_ACTION_CHARS} characters: the single \
             thing a person does. If it does not fit, it is a decision, not a hands item."
        ));
    }
    if let Some(phrase) = names_owner_wait(action) {
        return Err(format!(
            "--action says \"{phrase}\": that asks a person to decide, which is not a hands item. \
             {WHAT_TO_DO}"
        ));
    }
    Ok(Some(Hands {
        kind: kind.to_string(),
        action: action.to_string(),
    }))
}

#[cfg(test)]
mod tests {
    //! Oracle: FDR-0013's decision, rule by rule. Each case states the
    //! rule it stands for.

    use super::*;

    fn block<'a>(class: &'a str, text: &'a str) -> Wait<'a> {
        Wait {
            reason_class: Some(class),
            text,
            ..Wait::default()
        }
    }

    #[test]
    fn an_ordinary_block_is_not_a_person_wait() {
        // Positive control for every refusal below: the guard lets real blocks through.
        for (class, text) in [
            ("dependency", "blocked by PR #12061 until it merges"),
            ("ci-red", "main is red on id-test; fix is #12140"),
            ("capacity", "needs a 90-minute window"),
        ] {
            assert_eq!(check(&block(class, text)), Ok(None), "{class}: {text}");
        }
    }

    #[test]
    fn every_person_wait_class_is_refused_without_hands() {
        // Rule 2: nothing waits on Drew to decide, approve, confirm or re-scope.
        for class in PERSON_WAIT_CLASSES {
            let err = check(&block(class, "which encoding should F12.11 use?")).unwrap_err();
            assert!(err.contains("refusing to wait on a person"), "{err}");
            assert!(err.contains(class), "{err}");
            assert!(
                err.contains("decide-and-proceed"),
                "the refusal says what to do: {err}"
            );
            assert!(err.contains("--hands"), "{err}");
        }
    }

    #[test]
    fn a_person_answer_park_is_refused_without_hands() {
        let wait = Wait {
            condition_kind: Some("person_answer"),
            text: "planning-window confirmation",
            ..Wait::default()
        };
        assert!(
            check(&wait)
                .unwrap_err()
                .contains("ready condition 'person_answer'")
        );
        // A park waiting on something that is not a person is untouched.
        let ci = Wait {
            condition_kind: Some("ci_green"),
            text: "waiting for the nightly",
            ..Wait::default()
        };
        assert_eq!(check(&ci), Ok(None));
    }

    #[test]
    fn text_that_makes_the_owner_the_release_is_refused_whatever_the_class() {
        // FDR-0012 OQ-1 and FDR-0013 rule 7: the phrases lanes actually wrote.
        for text in [
            "Parked awaiting ownership handoff",
            "blocked as packet_defect for the owner to re-scope",
            "Next: owner decides whether the remaining items are still wanted",
            "needs Drew to confirm the planning window",
            "Waiting on Drew",
        ] {
            let err = check(&block("packet_defect", text)).unwrap_err();
            assert!(err.contains("the text says"), "{text}: {err}");
        }
        // "owner" alone is not a wait: an upstream repo has an owner too.
        assert_eq!(
            check(&block(
                "dependency",
                "the upstream repo owner merged the fix; waiting for a release"
            )),
            Ok(None)
        );
    }

    #[test]
    fn a_hands_item_passes_with_its_kind_and_action() {
        // Rule 6: a human body or a login only a person can finish.
        let wait = Wait {
            reason_class: Some("escalated-to-user"),
            text: "codespace surface needs the codespace OAuth scope",
            hands: Some("interactive-login"),
            action: Some(
                "Run `gh auth refresh -s codespace` on the laptop and approve the browser prompt.",
            ),
            ..Wait::default()
        };
        assert_eq!(
            check(&wait),
            Ok(Some(Hands {
                kind: "interactive-login".to_string(),
                action: "Run `gh auth refresh -s codespace` on the laptop and approve the browser prompt."
                    .to_string(),
            }))
        );
    }

    #[test]
    fn hands_must_be_a_known_kind_and_one_short_action() {
        let base = |hands, action| Wait {
            reason_class: Some("escalated-to-user"),
            text: "x",
            hands: Some(hands),
            action: Some(action),
            ..Wait::default()
        };
        assert!(
            check(&base("decision", "pick one"))
                .unwrap_err()
                .contains("not a hands kind")
        );
        assert!(
            check(&base("physical", "plug it in\nthen reboot"))
                .unwrap_err()
                .contains("one sentence")
        );
        let long = "x".repeat(MAX_ACTION_CHARS + 1);
        assert!(
            check(&base("physical", &long))
                .unwrap_err()
                .contains("one sentence")
        );
        // Exactly at the limit is fine: the bound is "at most".
        let at_limit = "x".repeat(MAX_ACTION_CHARS);
        assert!(check(&base("physical", &at_limit)).is_ok());
        // One of the two alone is not a hands item.
        let half = Wait {
            reason_class: Some("escalated-to-user"),
            text: "x",
            hands: Some("physical"),
            ..Wait::default()
        };
        assert!(
            check(&half)
                .unwrap_err()
                .contains("refusing to wait on a person")
        );
    }

    #[test]
    fn a_decision_cannot_be_dressed_as_a_hands_item() {
        let wait = Wait {
            reason_class: Some("escalated-to-user"),
            text: "x",
            hands: Some("interactive-login"),
            action: Some("Owner decides which encoding to use"),
            ..Wait::default()
        };
        assert!(check(&wait).unwrap_err().contains("not a hands item"));
    }

    #[test]
    fn hands_flags_on_a_wait_that_is_not_a_person_wait_are_a_usage_error() {
        let wait = Wait {
            reason_class: Some("dependency"),
            text: "blocked by PR #1",
            hands: Some("physical"),
            action: Some("plug it in"),
            ..Wait::default()
        };
        assert!(check(&wait).unwrap_err().contains("escalated-to-user"));
    }
}
