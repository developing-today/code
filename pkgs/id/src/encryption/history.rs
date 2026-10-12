//! History modes: how much of the group's past a joiner is welcomed with (P4.5).
//!
//! A welcome carries the group secrets the joiner may read. `Full` carries every secret the adder
//! holds, so the joiner reads the whole history. `Recent(n)` carries the newest `n`, so the joiner
//! reads the last `n` epochs and no older ones. A world sets its mode in its policy
//! ([`crate::groups::policy::History`]). The default is `full`.
//!
//! An epoch is a group secret. Creating the group, a removal and an explicit update each mint one,
//! and an add does not, so a window of `n` covers the last `n` rotations and not the last `n`
//! joins. See `explainers.md` section 4.

use std::cmp::Reverse;
use std::fmt;
use std::str::FromStr;

use p2panda_encryption::data_scheme::group_secret::{GroupSecret, SecretBundle, SecretBundleState};
use thiserror::Error;

use crate::groups::policy::History;

/// The window of a preset that keeps a join small: 32 epochs (5.3 KB per join with the stock
/// encoding, 1.7 KB with `serde_bytes`, at 1,000 rotations; round 2 `welcome-recent`).
pub const RECENT_SMALL: u32 = 32;

/// The window of a preset for churn-heavy worlds that want a longer memory: 128 epochs.
pub const RECENT_LARGE: u32 = 128;

/// How much of the group's past a joiner is welcomed with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistoryMode {
    /// Every secret the adder holds.
    Full,
    /// The newest `n` secrets by timestamp. `n` is at least 1.
    Recent(u32),
}

/// A history mode that cannot be used.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum HistoryModeError {
    /// The text is neither `full` nor `recent:N`.
    #[error("unknown history mode {0:?}: use \"full\" or \"recent:N\"")]
    Unknown(String),
    /// A window of zero epochs would welcome nothing.
    #[error("a recent window must hold at least one epoch")]
    EmptyWindow,
}

impl HistoryMode {
    /// A window of the newest `epochs` secrets.
    pub const fn recent(epochs: u32) -> Result<Self, HistoryModeError> {
        if epochs == 0 {
            return Err(HistoryModeError::EmptyWindow);
        }
        Ok(Self::Recent(epochs))
    }

    /// The mode a world's policy names. `from_join` and `causal` are not welcome bundles, so they
    /// have no mode here.
    pub fn from_policy(history: History) -> Option<Self> {
        match history {
            History::Full => Some(Self::Full),
            History::Recent(epochs) => Self::recent(epochs).ok(),
            History::FromJoin | History::Causal => None,
        }
    }

    /// The bundle a welcome carries under this mode: every secret in `secrets` for `Full`, and the
    /// newest `n` for `Recent(n)`.
    pub fn welcome_bundle(self, secrets: &SecretBundleState) -> SecretBundleState {
        let mut newest: Vec<GroupSecret> = secrets.secrets().cloned().collect();
        newest.sort_by_key(|secret| Reverse((secret.timestamp(), secret.id())));
        if let Self::Recent(epochs) = self {
            newest.truncate(usize::try_from(epochs).unwrap_or(usize::MAX));
        }
        SecretBundle::from_secrets(newest)
    }
}

impl FromStr for HistoryMode {
    type Err = HistoryModeError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        if text == "full" {
            return Ok(Self::Full);
        }
        let epochs = text
            .strip_prefix("recent:")
            .and_then(|count| count.parse::<u32>().ok())
            .ok_or_else(|| HistoryModeError::Unknown(text.to_owned()))?;
        Self::recent(epochs)
    }
}

impl fmt::Display for HistoryMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Full => write!(f, "full"),
            Self::Recent(epochs) => write!(f, "recent:{epochs}"),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use p2panda_encryption::crypto::Rng;

    use super::*;

    /// `count` secrets in the order they were generated. Each one is newer than the last, since
    /// `generate` moves a timestamp past the latest secret it sees.
    fn generated(count: usize) -> (Vec<GroupSecret>, SecretBundleState) {
        let rng = Rng::default();
        let mut bundle = SecretBundle::init();
        let mut secrets = Vec::new();
        for _ in 0..count {
            let secret = SecretBundle::generate(&bundle, &rng).expect("generate");
            secrets.push(secret.clone());
            bundle = SecretBundle::insert(bundle, secret);
        }
        (secrets, bundle)
    }

    #[test]
    fn test_history_mode_parses_and_prints() {
        assert_eq!("full".parse::<HistoryMode>(), Ok(HistoryMode::Full));
        assert_eq!(
            "recent:32".parse::<HistoryMode>(),
            Ok(HistoryMode::Recent(RECENT_SMALL))
        );
        assert_eq!(HistoryMode::Recent(RECENT_LARGE).to_string(), "recent:128");
        assert_eq!(HistoryMode::Full.to_string(), "full");
    }

    #[test]
    fn test_history_mode_refuses_an_empty_window_and_unknown_text() {
        assert_eq!(
            "recent:0".parse::<HistoryMode>(),
            Err(HistoryModeError::EmptyWindow)
        );
        assert!(matches!(
            "all".parse::<HistoryMode>(),
            Err(HistoryModeError::Unknown(_))
        ));
        assert!(matches!(
            "recent:x".parse::<HistoryMode>(),
            Err(HistoryModeError::Unknown(_))
        ));
    }

    #[test]
    fn test_policy_history_maps_to_a_welcome_mode() {
        assert_eq!(
            HistoryMode::from_policy(History::Full),
            Some(HistoryMode::Full)
        );
        assert_eq!(
            HistoryMode::from_policy(History::Recent(RECENT_LARGE)),
            Some(HistoryMode::Recent(RECENT_LARGE))
        );
        assert_eq!(HistoryMode::from_policy(History::Recent(0)), None);
        assert_eq!(HistoryMode::from_policy(History::FromJoin), None);
        assert_eq!(HistoryMode::from_policy(History::Causal), None);
    }

    #[test]
    fn test_recent_welcome_carries_the_newest_secrets_by_timestamp() {
        let (secrets, held) = generated(5);

        let recent = HistoryMode::Recent(2).welcome_bundle(&held);
        assert_eq!(recent.len(), 2);
        assert_eq!(recent.latest().map(GroupSecret::id), Some(secrets[4].id()));
        assert!(recent.contains(&secrets[4].id()));
        assert!(recent.contains(&secrets[3].id()));
        assert!(!recent.contains(&secrets[2].id()));

        let full = HistoryMode::Full.welcome_bundle(&held);
        assert_eq!(full.len(), 5);
    }

    #[test]
    fn test_recent_window_larger_than_the_bundle_carries_everything() {
        let (_, held) = generated(2);
        let welcome = HistoryMode::Recent(RECENT_LARGE).welcome_bundle(&held);
        assert_eq!(welcome.len(), 2);
    }
}
