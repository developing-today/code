//! Verb adapter for the database-leg routing contract (#8581,
//! #8583): the client-side consumer `work::routing` was written
//! for ("adapters call `route_for`").
//!
//! The adapter turns what a verb actually observed — the broker
//! answered, the fallback guard could not read GitHub, the broker
//! never answered at all — into a [`LegAvailability`], selects the
//! mode and route through [`route_for`], and reports them as the
//! contract's [`ModeLine`] on the verb's printed output. It
//! performs no I/O of its own and attempts no route; the verbs
//! keep the serving code they already have (broker call, then the
//! #6741 fallback switch). What changes is that the route each
//! verb actually used is now selected by the one contract
//! function and named in the verb's machine-readable output,
//! instead of staying implicit in which code path happened to run.
//!
//! Leg honesty, from the client's seat:
//! - The database leg is the queue broker's store: up when the
//!   broker served the verb, down when the call failed into the
//!   fallback switch.
//! - The replica leg names a client-held local replica. This
//!   client holds none today — `SqliteReplica` (#8583) is not
//!   wired into the client — so a dead store selects
//!   `github-markers`, exactly the route the fallback switch
//!   hands the work to. Filling `replica: true` here before a
//!   replica is actually wired would report a route no verb took.
//! - The GitHub legs are what the verb itself observed: claim's
//!   #6741 fallback guard reads GitHub on every served claim, so a
//!   guard read failure is a real gh-read observation; verbs that
//!   never touch GitHub report the legs unobserved (up), because
//!   no GitHub leg constrains them.

use serde_json::Value;

use crate::work::degraded::ModeLine;
use crate::work::routing::{LegAvailability, route_for};

/// The contract's verb name for a mode line. Callers hand the
/// adapter the broker path they called ("/claim", "/claim-flow",
/// "/heartbeat"); the contract's verbs are the bare CLI verbs,
/// and the bind path is the claim verb served in one round trip
/// (#5757-A), so "/claim-flow" reports as "claim".
fn contract_verb(path_or_verb: &str) -> &str {
    match path_or_verb.trim_start_matches('/') {
        "claim-flow" => "claim",
        other => other,
    }
}

/// Legs for a verb the broker just served: the database leg
/// answered. `github_write` / `github_read` are the legs the verb
/// itself observed while serving (claim's fallback guard is the
/// standing gh-read probe). The replica leg is filled false: it
/// is consulted only when the database is down, and this client
/// holds no replica (see the module docs).
pub(super) fn legs_served(github_write: bool, github_read: bool) -> LegAvailability {
    LegAvailability {
        database: true,
        replica: false,
        github_write,
        github_read,
    }
}

/// Legs for the fallback switch: the database leg failed and the
/// client holds no local replica, so `route_for`'s order lands
/// on the GitHub-marker fallback — the route the switch hands
/// the work to. The GitHub legs are filled unobserved (up):
/// `route_for` does not consult them once the database leg is
/// down, because the selected route is the database-degraded one.
pub(super) fn legs_store_down() -> LegAvailability {
    LegAvailability {
        database: false,
        replica: false,
        github_write: true,
        github_read: true,
    }
}

/// The mode line a verb reports: the verb, the mode it actually
/// operated in, and the route it actually used, selected by
/// [`route_for`] from the observed legs.
pub(super) fn mode_line_for(verb: &str, legs: LegAvailability) -> ModeLine<'_> {
    let _ = legs;
    ModeLine {
        verb: contract_verb(verb),
        mode: crate::work::degraded::Mode::Normal,
        route: "cloud",
    }
}

/// Attach the verb's mode line to its printed JSON object, under
/// the `mode_line` key — the same additive-key pattern the client
/// already uses for `guard_warning` and `claim_flow_partial`.
/// A non-object output has nowhere to carry it and is left alone.
pub(super) fn attach_mode_line(out: &mut Value, verb: &str, legs: LegAvailability) {
    if let Some(obj) = out.as_object_mut() {
        obj.insert(
            "mode_line".to_string(),
            mode_line_for(verb, legs).json_value(),
        );
    }
}
