//! Resume tokens and a bounded screen-baseline cache (F12.11 phase 2).
//!
//! A resume token names the exact view revision a client applied. The token is
//! an opaque cache key, never an authorization credential: a reconnect still
//! has to join with a valid capability, and a baseline is honored only for the
//! participant, screen version and dimensions that produced it. Anything else
//! falls back to a full view, which is always correct.
//!
//! The cache is bounded in both size ([`MAX_RESUME_BASELINES`]) and age
//! ([`RESUME_BASELINE_TTL`]); eviction is least-recently-issued first, matching
//! the design's "bounded baseline with predictable fallback" recommendation
//! (`doc/2026-11-01_design_id805.md` §2.4).

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use crate::world_presentation::ScreenSurface;

/// Most retained screen baselines per world service.
pub(crate) const MAX_RESUME_BASELINES: usize = 64;
/// How long a retained baseline may be resumed from.
pub(crate) const RESUME_BASELINE_TTL: Duration = Duration::from_secs(600);
/// Entropy in one resume token, before hex encoding.
const RESUME_TOKEN_SEED_BYTES: usize = 16;

/// One retained screen a client may resume from.
#[derive(Clone, Debug)]
pub(crate) struct ScreenBaseline {
    /// The participant whose joined session applied this screen.
    pub(crate) participant: u64,
    /// The view revision the client holds.
    pub(crate) revision: u64,
    /// Screen format version; a patch never crosses versions.
    pub(crate) version: u8,
    /// Screen columns; a patch never crosses viewports.
    pub(crate) width: u16,
    /// Screen rows; a patch never crosses viewports.
    pub(crate) height: u16,
    /// The applied screen, retained so the catch-up patch can be diffed.
    pub(crate) screen: ScreenSurface,
    issued: Instant,
}

impl ScreenBaseline {
    /// A baseline for the screen revision `revision`, stamped at issuance.
    pub(crate) fn new(
        participant: u64,
        revision: u64,
        version: u8,
        width: u16,
        height: u16,
        screen: ScreenSurface,
    ) -> Self {
        Self {
            participant,
            revision,
            version,
            width,
            height,
            screen,
            issued: Instant::now(),
        }
    }
}

/// Bounded least-recently-issued cache of screen baselines.
#[derive(Debug)]
pub(crate) struct BaselineCache {
    capacity: usize,
    ttl: Duration,
    /// Monotonic seed: guarantees a fresh token per issuance.
    next_token: u64,
    /// Oldest entry first.
    entries: VecDeque<(String, ScreenBaseline)>,
}

impl Default for BaselineCache {
    fn default() -> Self {
        Self::new(MAX_RESUME_BASELINES, RESUME_BASELINE_TTL)
    }
}

impl BaselineCache {
    pub(crate) const fn new(capacity: usize, ttl: Duration) -> Self {
        Self {
            capacity,
            ttl,
            next_token: 0,
            entries: VecDeque::new(),
        }
    }

    /// Retain `baseline` and return the opaque token that names it.
    pub(crate) fn issue(&mut self, baseline: ScreenBaseline) -> String {
        self.evict_expired();
        while self.entries.len() >= self.capacity.max(1) {
            let _ = self.entries.pop_front();
        }
        let token = self.next_token();
        self.entries.push_back((
            token.clone(),
            ScreenBaseline {
                issued: Instant::now(),
                ..baseline
            },
        ));
        token
    }

    /// Take the baseline `token` names, when it still belongs to `participant`
    /// and matches the screen shape the caller is about to send.
    ///
    /// A token that is unknown, expired or bound to another participant (or
    /// shape) is dropped and reported as a miss, so a caller always has to
    /// fall back to a full view.
    pub(crate) fn take_matching(
        &mut self,
        token: &str,
        participant: u64,
        version: u8,
        width: u16,
        height: u16,
    ) -> Option<ScreenBaseline> {
        self.evict_expired();
        let position = self
            .entries
            .iter()
            .position(|(candidate, _)| candidate == token)?;
        let (_, baseline) = self.entries.remove(position)?;
        if baseline.participant != participant
            || baseline.version != version
            || baseline.width != width
            || baseline.height != height
        {
            return None;
        }
        Some(baseline)
    }

    fn next_token(&mut self) -> String {
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"developing-today/id-world-resume-token/v1\0");
        hasher.update(&self.next_token.to_be_bytes());
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        hasher.update(&now.as_nanos().to_be_bytes());
        self.next_token = self.next_token.wrapping_add(1);
        let digest = hasher.finalize();
        digest.as_bytes()[..RESUME_TOKEN_SEED_BYTES].iter().fold(
            String::with_capacity(RESUME_TOKEN_SEED_BYTES * 2),
            |mut hex, byte| {
                use std::fmt::Write as _;
                let _ = write!(hex, "{byte:02x}");
                hex
            },
        )
    }

    fn evict_expired(&mut self) {
        let now = Instant::now();
        let ttl = self.ttl;
        self.entries
            .retain(|(_, baseline)| now.duration_since(baseline.issued) < ttl);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world_presentation::{ScreenCell, ScreenSurface};

    fn screen(glyph: &str) -> ScreenSurface {
        ScreenSurface::from_cells(
            1,
            1,
            vec![ScreenCell {
                symbol: glyph.to_owned(),
                fg: 0,
                bg: 0,
                bold: false,
                columns: 1,
            }],
        )
        .expect("a 1x1 screen")
    }

    fn baseline(participant: u64, revision: u64) -> ScreenBaseline {
        ScreenBaseline::new(participant, revision, 1, 1, 1, screen("a"))
    }

    #[test]
    fn a_token_names_one_bound_baseline_and_is_consumed() {
        let mut cache = BaselineCache::default();
        let token = cache.issue(baseline(7, 3));
        assert!(
            cache.take_matching(&token, 8, 1, 1, 1).is_none(),
            "another participant misses"
        );
        let again = cache.issue(baseline(7, 4));
        assert!(
            cache.take_matching(&again, 7, 1, 2, 1).is_none(),
            "another viewport misses"
        );
        let third = cache.issue(baseline(7, 5));
        assert!(
            cache.take_matching(&third, 7, 2, 1, 1).is_none(),
            "another version misses"
        );
        let fourth = cache.issue(baseline(7, 6));
        let taken = cache
            .take_matching(&fourth, 7, 1, 1, 1)
            .expect("a matching token resumes");
        assert_eq!(taken.revision, 6);
        assert!(
            cache.take_matching(&fourth, 7, 1, 1, 1).is_none(),
            "a token is single-use"
        );
    }

    #[test]
    fn tampered_tokens_are_misses() {
        let mut cache = BaselineCache::default();
        let token = cache.issue(baseline(1, 0));
        let mut tampered = token.clone();
        tampered.replace_range(0..1, if token.starts_with('0') { "1" } else { "0" });
        assert!(cache.take_matching(&tampered, 1, 1, 1, 1).is_none());
        assert!(cache.take_matching("not-a-token", 1, 1, 1, 1).is_none());
        assert_eq!(
            cache.entries.len(),
            1,
            "a miss does not consume another token"
        );
    }

    #[test]
    fn the_cache_is_bounded_in_age_and_size() {
        let mut expired = BaselineCache::new(8, Duration::ZERO);
        let token = expired.issue(baseline(1, 0));
        assert!(
            expired.take_matching(&token, 1, 1, 1, 1).is_none(),
            "expired baselines miss"
        );

        let mut small = BaselineCache::new(1, RESUME_BASELINE_TTL);
        let first = small.issue(baseline(1, 0));
        let second = small.issue(baseline(1, 1));
        assert!(
            small.take_matching(&first, 1, 1, 1, 1).is_none(),
            "the oldest entry is evicted"
        );
        assert!(small.take_matching(&second, 1, 1, 1, 1).is_some());
    }

    #[test]
    fn tokens_are_unique() {
        let mut cache = BaselineCache::default();
        let tokens: std::collections::BTreeSet<String> = (0..64)
            .map(|revision| cache.issue(baseline(1, revision)))
            .collect();
        assert_eq!(tokens.len(), 64);
    }
}
