//! Typed failure modes for the Fleet verb routing contract (#8581).
//!
//! This module is deliberately a pure contract: it names the failure legs,
//! the mode each leg selects, and the machine-readable mode line a routed
//! verb can report. It does not perform I/O or choose a route. Adapters must
//! pass the route they actually used; the #8583 replica remains a later
//! implementation of the database-degraded path.

/// A dependency leg whose failure changes how a Fleet verb is served.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureLeg {
    /// GitHub writes are unavailable; durable outcomes must be buffered.
    GitHubWrite,
    /// GitHub reads are unavailable; reads use the available mirror/cache.
    GitHubRead,
    /// The coordination database is unavailable; a local replica is the
    /// planned route, with the GitHub-marker fallback behind it.
    Database,
}

/// The operating mode required by #8581.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// All preferred dependencies are available.
    #[default]
    Normal,
    /// A GitHub write failed; continue and buffer the durable outcome.
    GhWriteDegraded,
    /// A GitHub read failed; continue from the mirror/cache.
    GhReadDegraded,
    /// The coordination database failed; continue through the replica path.
    DbDegraded,
}

impl Mode {
    /// Select the mode for one failed dependency leg.
    ///
    /// A verb that encounters multiple failed legs should report the mode
    /// corresponding to the route it actually selected, rather than infer a
    /// precedence that the contract has not defined.
    pub fn for_failed_leg(leg: FailureLeg) -> Self {
        match leg {
            FailureLeg::GitHubWrite => Self::GhWriteDegraded,
            FailureLeg::GitHubRead => Self::GhReadDegraded,
            FailureLeg::Database => Self::DbDegraded,
        }
    }

    /// The stable machine-readable spelling required by #8581.
    pub fn label(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::GhWriteDegraded => "gh-write-degraded",
            Self::GhReadDegraded => "gh-read-degraded",
            Self::DbDegraded => "db-degraded",
        }
    }
}

/// Machine-readable report of a verb's actual mode and selected route.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModeLine<'a> {
    /// Queue verb that produced this report.
    pub verb: &'a str,
    /// Mode the verb actually operated in.
    pub mode: Mode,
    /// Route actually selected by the adapter (not a preferred route).
    pub route: &'a str,
}

impl<'a> ModeLine<'a> {
    /// Build the JSON value to emit as the verb's machine-readable mode line.
    pub fn json_value(&self) -> serde_json::Value {
        serde_json::json!({
            "verb": self.verb,
            "mode": self.mode.label(),
            "route": self.route,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{FailureLeg, Mode, ModeLine};

    #[test]
    fn every_failure_leg_maps_to_its_contract_mode() {
        let cases = [
            (
                FailureLeg::GitHubWrite,
                Mode::GhWriteDegraded,
                "gh-write-degraded",
            ),
            (
                FailureLeg::GitHubRead,
                Mode::GhReadDegraded,
                "gh-read-degraded",
            ),
            (FailureLeg::Database, Mode::DbDegraded, "db-degraded"),
        ];

        for (leg, expected_mode, expected_label) in cases {
            let mode = Mode::for_failed_leg(leg);
            assert_eq!(mode, expected_mode);
            assert_eq!(mode.label(), expected_label);
        }
    }

    #[test]
    fn mode_line_serializes_the_actual_mode_and_route() {
        let value = ModeLine {
            verb: "claim",
            mode: Mode::DbDegraded,
            route: "local-replica",
        }
        .json_value();

        assert_eq!(
            value,
            serde_json::json!({
                "verb": "claim",
                "mode": "db-degraded",
                "route": "local-replica"
            })
        );
    }

    #[test]
    fn normal_mode_has_the_contract_label() {
        assert_eq!(Mode::default().label(), "normal");
    }
}
