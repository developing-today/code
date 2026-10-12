//! The reason a refusal gives, one enum for every surface that refuses something.
//!
//! The event store's ingest result ([`crate::event_store::Ingest`]), the groups fold
//! ([`crate::groups::Outcome`]), and the encryption adapter's errors
//! ([`crate::encryption::SchemeError`]) each give a reason through [`Reason`], so a caller shows the
//! same three words whichever layer refused. The reasons are the design's (task 10): voided,
//! waiting, frozen.
//!
//! A surface maps its own refusals to a reason with a `reason()` method. A refusal that is not a
//! judgement on an event or an operation has no reason. Those are a malformed encoding, a failed
//! crate call, or a local precondition, and each surface's `reason()` returns `None` for them.

use std::fmt;

/// Why an event or an operation was refused or held.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Reason {
    /// A rule decided that the item has no effect at its place in the causal order: an authority
    /// rule (a removed member's later operation, an action its author may not take), or a rule of
    /// the group or the scheme.
    Voided,
    /// The item waits for something it depends on that has not arrived or is not in force yet: a
    /// missing event, a welcome, an answer, or a waiting cap that holds it out.
    Waiting,
    /// The item cannot take effect because what it acts on is frozen: its author's chain forked, a
    /// removal dissolved the group, or the session lost its state.
    Frozen,
}

impl fmt::Display for Reason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Voided => "voided",
            Self::Waiting => "waiting",
            Self::Frozen => "frozen",
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn test_each_reason_has_its_design_name() {
        assert_eq!(Reason::Voided.to_string(), "voided");
        assert_eq!(Reason::Waiting.to_string(), "waiting");
        assert_eq!(Reason::Frozen.to_string(), "frozen");
    }
}
