//! The freshness window (N23): how far an accept's `ctime` may sit from the invite's creation, and from the admin's
//! clock.
//!
//! The window is a local acceptance rule, not a protocol rule. The accept bytes do not carry it, so an admin may set
//! it without changing what a joiner sends. It is set per server, per world, or per inviter. The most specific
//! setting that is present applies: inviter, then world, then server, then [`DEFAULT_WINDOW`].

use std::time::Duration;

/// The window when nothing is set: 5 minutes (N23).
pub const DEFAULT_WINDOW: Duration = Duration::from_mins(5);

/// The window settings at each level. Unset levels fall through to the next one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Freshness {
    /// The server's setting.
    pub server: Option<Duration>,
    /// The world's setting. It overrides the server's.
    pub world: Option<Duration>,
    /// The inviter's setting. It overrides the world's.
    pub inviter: Option<Duration>,
}

impl Freshness {
    /// The window that applies: the inviter's, else the world's, else the server's, else [`DEFAULT_WINDOW`].
    #[must_use]
    pub fn window(&self) -> Duration {
        self.inviter
            .or(self.world)
            .or(self.server)
            .unwrap_or(DEFAULT_WINDOW)
    }
}
