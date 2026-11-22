//! Serving operations of the world core: participants, schedules,
//! capability issuance and delegation, chat and input, entity
//! handoffs, and snapshots.
//! Extracted from `core.rs` for the production-line guard (#10456;
//! split tracked in #1438): moves only, no behaviour change.

use super::*;

use anyhow::{Context, Result, ensure};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::sync::Arc;

impl WorldCore {
    /// The world's active (non-revoked) participants.
    #[must_use]
    pub(crate) fn participants(&self) -> Vec<Participant> {
        self.capabilities
            .values()
            .filter(|record| !record.revoked)
            .map(|record| record.participant.clone())
            .collect()
    }

    /// One participant, whether revoked or not.
    #[must_use]
    pub(crate) fn participant(&self, id: u64) -> Option<Participant> {
        self.capabilities
            .get(&id)
            .map(|record| record.participant.clone())
    }

    /// Commit a host-authored chat line as participant 0.
    ///
    /// # Errors
    ///
    /// Fails when the text is empty, oversized, or entirely sanitized away.
    pub(crate) fn prepare_system_chat(&self, text: &str) -> Result<WorldEvent> {
        ensure!(
            text.len() <= self.limits.chat_bytes,
            "chat message exceeds {} bytes",
            self.limits.chat_bytes
        );
        let text = sanitize_chat(text);
        ensure!(!text.is_empty(), "chat message must not be empty");
        Ok(WorldEvent {
            sequence: self
                .sequence
                .checked_add(1)
                .context("world event sequence exhausted")?,
            participant_id: 0,
            kind: WorldEventKind::Chat(text),
        })
    }

    /// Whether a `schedule.at` for `id` may be pended, given the host clock
    /// `now_ms`. The error says why; the program sees only `bad_args`.
    ///
    /// # Errors
    ///
    /// A malformed id, a duplicate pending id, a full schedule, or a time past
    /// the horizon.
    pub(crate) fn check_schedule(&self, id: &str, at_ms: u64, now_ms: u64) -> Result<()> {
        self.check_schedule_inner(id, at_ms, None, None, None, now_ms, false)
    }

    /// Whether a recurring `schedule.at` (#1355) may be pended: the one-shot
    /// checks, the recurrence pair's own bounds, and the whole series —
    /// first fire plus `interval * (count - 1)` — inside the horizon.
    pub(crate) fn check_schedule_recurring(
        &self,
        id: &str,
        at_ms: u64,
        interval_ms: Option<u64>,
        count: Option<u32>,
        now_ms: u64,
    ) -> Result<()> {
        self.check_schedule_inner(id, at_ms, interval_ms, count, None, now_ms, false)
    }

    /// Whether a `schedule.at` may be pended for delivery to another
    /// world (#1355 part 2): the recurring checks, plus the target must
    /// be a different, validly named world, this world must be
    /// unisolated, and this world's admin must have granted send consent
    /// for that peer. The target's receive consent is the target's own
    /// journal state, checked there at delivery time.
    pub(crate) fn check_schedule_targeted(
        &self,
        id: &str,
        at_ms: u64,
        interval_ms: Option<u64>,
        count: Option<u32>,
        target: Option<&str>,
        now_ms: u64,
    ) -> Result<()> {
        self.check_schedule_inner(id, at_ms, interval_ms, count, target, now_ms, false)
    }

    /// The `kv` store's total charge, in bytes: key bytes plus value
    /// bytes (the stored JSON encoding's length) per entry (#1062).
    pub(crate) fn kv_bytes(&self) -> usize {
        self.kv
            .iter()
            .map(|(key, value)| key.len() + value.len())
            .sum()
    }

    /// Whether a `kv.set` of `key` to a value of `value_len` serialized
    /// bytes may be stored. The error says why; the program sees only
    /// `bad_args`. A set of an existing key is a replacement: it is
    /// validated against the store as it will be after the write, so
    /// it consumes neither a second entry slot nor a second byte
    /// charge.
    ///
    /// # Errors
    ///
    /// A malformed key, an oversized value, a full store, or a store
    /// whose total byte budget the write would exceed.
    pub(crate) fn check_kv_set(&self, key: &str, value_len: usize) -> Result<()> {
        crate::world_caps::validate_kv_key(key)?;
        ensure!(
            value_len <= crate::world_caps::MAX_KV_VALUE_BYTES,
            "kv value exceeds {} bytes",
            crate::world_caps::MAX_KV_VALUE_BYTES
        );
        let replaced = self.kv.get(key);
        ensure!(
            replaced.is_some() || self.kv.len() < crate::world_caps::MAX_KV_ENTRIES,
            "world already has {} kv entries",
            crate::world_caps::MAX_KV_ENTRIES
        );
        let replaced_charge = replaced.map_or(0, |value| key.len() + value.len());
        let total = self
            .kv_bytes()
            .saturating_sub(replaced_charge)
            .saturating_add(key.len() + value_len);
        ensure!(
            total <= crate::world_caps::MAX_KV_TOTAL_BYTES,
            "kv store would exceed {} total bytes",
            crate::world_caps::MAX_KV_TOTAL_BYTES
        );
        Ok(())
    }

    /// Apply one fire of a pending schedule: re-arm it in place while
    /// fires remain (fixed-rate: the next due time advances by the
    /// interval from the planned time, never from now), or consume it.
    /// Returns false when no schedule by that id is pending. Shared by
    /// journal replay and the actor's fire path, so both derive the same
    /// next state from the same journaled state.
    pub(crate) fn apply_schedule_fired(&mut self, id: &str) -> bool {
        let Some(schedule) = self.schedules.get(id).cloned() else {
            return false;
        };
        match schedule.after_fire() {
            Some(rearmed) => {
                self.schedules.insert(id.to_owned(), rearmed);
            }
            None => {
                self.schedules.remove(id);
            }
        }
        true
    }

    /// The pending schedules as id to next due time: the projection the
    /// admin layer has always returned, whatever a schedule's recurrence.
    pub(crate) fn schedule_times(&self) -> BTreeMap<String, u64> {
        self.schedules
            .iter()
            .map(|(id, schedule)| (id.clone(), schedule.at_ms))
            .collect()
    }

    /// Like [`Self::check_schedule`], but for a set that replaces the pending
    /// schedule with the same id. Whether this is a replacement is derived
    /// from this core's schedule map: the entry being replaced counts as
    /// already cancelled, so it is neither a duplicate nor a use of capacity.
    /// Callers validate with this BEFORE persisting the replacement's
    /// cancellation, so a refused replacement (say, past the horizon) leaves
    /// the existing schedule standing (#807).
    pub(crate) fn check_schedule_replacing(&self, id: &str, at_ms: u64, now_ms: u64) -> Result<()> {
        let replacing = self.schedules.contains_key(id);
        self.check_schedule_inner(id, at_ms, None, None, None, now_ms, replacing)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn check_schedule_inner(
        &self,
        id: &str,
        at_ms: u64,
        interval_ms: Option<u64>,
        count: Option<u32>,
        target: Option<&str>,
        now_ms: u64,
        replacing: bool,
    ) -> Result<()> {
        crate::world_caps::validate_schedule_id(id)?;
        crate::world_caps::validate_schedule_recurrence(interval_ms, count)?;
        if let Some(target) = target {
            crate::world_hub::validate_world_name(target)
                .context("invalid schedule target world")?;
            ensure!(
                target != self.world_id,
                "schedule {id:?} targets its own world; omit the world to schedule it here"
            );
            ensure!(
                self.isolation == Isolation::Unisolated,
                "schedule {id:?} cannot target another world while this world is isolated"
            );
            ensure!(
                self.schedule_grants
                    .get(target)
                    .is_some_and(|grant| grant.send),
                "schedule {id:?} has no send grant for world {target:?}; an admin must grant it first"
            );
        }
        ensure!(
            replacing || !self.schedules.contains_key(id),
            "schedule {id:?} is already pending; cancel it before scheduling it again"
        );
        let pending = self.schedules.len() - usize::from(replacing);
        ensure!(
            pending < crate::world_caps::MAX_PENDING_SCHEDULES,
            "world already has {} pending schedules",
            crate::world_caps::MAX_PENDING_SCHEDULES
        );
        let horizon = now_ms.saturating_add(crate::world_caps::MAX_SCHEDULE_HORIZON_MS);
        ensure!(
            at_ms <= horizon,
            "schedule {id:?} is more than {} days past the host clock",
            crate::world_caps::MAX_SCHEDULE_HORIZON_DAYS
        );
        if let (Some(interval), Some(count)) = (interval_ms, count) {
            // The series' last fire is as much a schedule time as its
            // first: it must land inside the same horizon.
            let last_fire = interval
                .checked_mul(u64::from(count - 1))
                .and_then(|span| at_ms.checked_add(span));
            ensure!(
                last_fire.is_some_and(|last| last <= horizon),
                "schedule {id:?} series ends more than {} days past the host clock",
                crate::world_caps::MAX_SCHEDULE_HORIZON_DAYS
            );
        }
        Ok(())
    }

    /// Journal entry for an already-issued capability.
    pub(crate) fn issued_entry(&self, participant_id: u64) -> Option<JournalEntry> {
        self.capabilities
            .get(&participant_id)
            .map(|record| JournalEntry::Issued {
                participant: record.participant.clone(),
                digest: hex_encode(&record.digest),
                scopes: record.scopes.bits(),
                parent: record.parent,
                expires_at: record.bounds.expires_at,
                uses: record.bounds.uses,
                used: record.used,
                subject: record.subject.clone(),
                issued_at: record.issued_at,
            })
    }

    /// Latest committed event sequence.
    #[must_use]
    pub const fn current_sequence(&self) -> u64 {
        self.sequence
    }

    /// The world's identifier.
    #[must_use]
    pub fn world_id(&self) -> &str {
        &self.world_id
    }

    /// Active region-seam handoffs this world is a side of (#3870 phase 1).
    #[must_use]
    pub(crate) fn handoff_transfers(&self) -> &[HandoffTransferState] {
        &self.handoff_transfers
    }

    /// The world's journal-resident native-spill write lease
    /// (design #2819, phase 2 of issue #3804).
    #[must_use]
    pub(crate) fn write_lease(&self) -> &WriteLease {
        &self.write_lease
    }

    /// The per-world resource requests, independent of current server ceilings.
    /// The world's total-memory budget ledger (#557): the effective
    /// limit, the logical charge measured at restore, its high-water
    /// mark, and the refusal count. Logical accounting, not RSS.
    pub const fn memory_budget(&self) -> &crate::world_limits::WorldMemoryBudget {
        &self.memory_budget
    }

    pub const fn execution_profile(&self) -> crate::world_limits::WorldExecutionProfile {
        self.execution_profile
    }

    /// Issue a capability for a participant. Only `JOIN`-scoped capabilities
    /// count toward active participant capacity.
    pub fn issue_capability(
        &mut self,
        display_name: &str,
        scopes: WorldScopes,
        bounds: CapabilityBounds,
    ) -> Result<(Participant, JoinCapability)> {
        self.mint(display_name, scopes, None, None, bounds)
    }

    /// Issue a capability that acts for the account `credential` belongs to.
    /// Its scopes may not exceed that account's ceiling.
    pub fn issue_for_account(
        &mut self,
        credential: &str,
        scopes: WorldScopes,
        bounds: CapabilityBounds,
        display_name: &str,
    ) -> Result<(Participant, JoinCapability)> {
        let account = self
            .directory
            .account_for(credential)
            .context("invalid account credential")?;
        ensure!(
            self.directory.ceiling(&account).implied().contains(scopes),
            "this account may not hold those scopes"
        );
        self.mint(display_name, scopes, None, Some(account), bounds)
    }

    /// Delegate a capability: a new participant whose scopes are a subset of
    /// the delegating capability's, and which is revoked with it. Its expiry
    /// may not be later than the delegator's. It carries no use count: each
    /// action it takes counts against the host-issued limit above it, if any.
    pub fn attenuate(
        &mut self,
        token: &JoinCapability,
        scopes: WorldScopes,
        bounds: CapabilityBounds,
        display_name: &str,
    ) -> Result<(Participant, JoinCapability)> {
        self.delegate(token, scopes, bounds, display_name, None)
    }

    /// Delegate to a friend of the account this capability acts for. The
    /// friend's ceiling must already hold the scopes, and the friendship must
    /// still hold each time the capability is used.
    pub fn delegate_to_friend(
        &mut self,
        token: &JoinCapability,
        friend: &str,
        scopes: WorldScopes,
        bounds: CapabilityBounds,
        display_name: &str,
    ) -> Result<(Participant, JoinCapability)> {
        let parent = self.authorize(token, WorldScopes::JOIN)?;
        let owner = self
            .capabilities
            .get(&parent)
            .and_then(|record| record.subject.clone())
            .context("only an account's capability can delegate to a friend")?;
        ensure!(
            self.directory.are_friends(&owner, friend),
            "you can only delegate to a friend"
        );
        ensure!(
            self.directory.ceiling(friend).implied().contains(scopes),
            "that friend may not hold those scopes"
        );
        self.delegate(token, scopes, bounds, display_name, Some(friend.to_owned()))
    }

    pub(crate) fn delegate(
        &mut self,
        token: &JoinCapability,
        scopes: WorldScopes,
        bounds: CapabilityBounds,
        display_name: &str,
        subject: Option<String>,
    ) -> Result<(Participant, JoinCapability)> {
        let parent = self.authorize(token, WorldScopes::JOIN)?;
        let parent_scopes = self
            .effective_scopes(parent)
            .context("world participant is unavailable")?;
        let parent_record = self
            .capabilities
            .get(&parent)
            .context("world participant is unavailable")?;
        let scopes = parent_scopes
            .attenuate(scopes)
            .context("this capability may not delegate those scopes")?;
        if let Some(expires_at) = bounds.expires_at {
            ensure!(
                expires_at > self.clock().now_ms(),
                "delegated expiry must be in the future"
            );
            ensure!(
                parent_record
                    .bounds
                    .expires_at
                    .is_none_or(|limit| expires_at <= limit),
                "this capability may not delegate a later expiry"
            );
        }
        ensure!(
            bounds.uses.is_none(),
            "a delegated capability carries no use count; the host's limit still applies"
        );
        self.mint(display_name, scopes, Some(parent), subject, bounds)
    }

    /// What a capability may do right now: its own scopes, held to the ceiling
    /// of each account in its chain. A delegation to a friend stops once the
    /// friendship ends.
    pub(crate) fn effective_scopes(&self, participant_id: u64) -> Option<WorldScopes> {
        self.effective_scopes_seeded(participant_id, WorldScopes::ALL)
    }

    /// [`Self::effective_scopes`] with a different starting set. Seeding
    /// with [`WorldScopes::all_recognized`] keeps admin-issued authority
    /// bits (`BUILDER` / `MODERATOR`) visible to the intersection; the
    /// public [`Self::effective_scopes`] seeds with [`WorldScopes::ALL`],
    /// which deliberately excludes them, so no ceiling, delegation, or
    /// projection path can ever surface them.
    pub(super) fn effective_scopes_seeded(
        &self,
        participant_id: u64,
        seed: WorldScopes,
    ) -> Option<WorldScopes> {
        self.capabilities.get(&participant_id)?;
        let mut scopes = seed;
        let mut current = Some(participant_id);
        while let Some(record) = current.and_then(|id| self.capabilities.get(&id)) {
            scopes = scopes.intersect(record.scopes.implied());
            if let Some(subject) = &record.subject {
                scopes = scopes.intersect(self.directory.ceiling(subject).implied());
            }
            let parent = record.parent.and_then(|id| self.capabilities.get(&id));
            if let Some(subject) = &record.subject
                && let Some(parent_subject) = parent.and_then(|p| p.subject.as_ref())
                && subject != parent_subject
                && !self.directory.are_friends(parent_subject, subject)
            {
                return Some(WorldScopes::EMPTY);
            }
            current = record.parent;
        }
        if self.replica_mode {
            Some(scopes.intersect(WorldScopes::JOIN))
        } else {
            Some(scopes)
        }
    }

    /// Fewest actions the capability or any ancestor may still take, or `None`
    /// if no capability in the chain is limited.
    pub(crate) fn uses_left(&self, participant_id: u64) -> Option<u32> {
        let mut left = None;
        let mut current = Some(participant_id);
        while let Some(record) = current.and_then(|id| self.capabilities.get(&id)) {
            if let Some(limit) = record.bounds.uses {
                let remaining = limit.saturating_sub(record.used);
                left = Some(left.map_or(remaining, |so_far: u32| so_far.min(remaining)));
            }
            current = record.parent;
        }
        left
    }

    pub(crate) fn ensure_not_expired(&self, participant_id: u64) -> Result<()> {
        let now = self.clock().now_ms();
        let mut current = Some(participant_id);
        while let Some(record) = current.and_then(|id| self.capabilities.get(&id)) {
            if !record.bounds.expires_at.is_none_or(|expires| now < expires) {
                return Err(CapabilityDenied::new("world capability has expired").into());
            }
            current = record.parent;
        }
        Ok(())
    }

    pub(crate) fn ensure_uses_left(&self, participant_id: u64) -> Result<()> {
        ensure!(
            self.uses_left(participant_id).is_none_or(|left| left > 0),
            "world capability has no uses left"
        );
        Ok(())
    }

    /// Count one committed action against the participant and each ancestor.
    pub(crate) fn spend(&mut self, participant_id: u64) {
        let mut current = Some(participant_id);
        while let Some(record) = current.and_then(|id| self.capabilities.get_mut(&id)) {
            record.used = record.used.saturating_add(1);
            current = record.parent;
        }
    }

    pub(crate) fn mint(
        &mut self,
        display_name: &str,
        scopes: WorldScopes,
        parent: Option<u64>,
        subject: Option<String>,
        bounds: CapabilityBounds,
    ) -> Result<(Participant, JoinCapability)> {
        let active = self
            .capabilities
            .values()
            .filter(|record| !record.revoked)
            .count();
        ensure!(
            active < self.limits.participants,
            "world participant limit reached"
        );
        ensure!(
            scopes.contains(WorldScopes::JOIN),
            "world capability must include join scope"
        );
        let display_name = sanitize_display_name(display_name);
        ensure!(
            !display_name.is_empty(),
            "participant display name must not be empty"
        );
        let id = self.next_participant_id;
        self.next_participant_id = self
            .next_participant_id
            .checked_add(1)
            .context("participant ID exhausted")?;
        let mut secret = [0_u8; 32];
        rand::rng().fill(&mut secret);
        let token = format!("{id}.{}", hex_encode(&secret));
        let digest = token_digest(&token);
        let participant = Participant { id, display_name };
        let issued_at = Some(self.clock().now_ms());
        self.capabilities.insert(
            id,
            CapabilityRecord {
                digest,
                participant: participant.clone(),
                scopes,
                parent,
                revoked: false,
                revoked_by: None,
                bounds,
                used: 0,
                subject,
                issued_at,
                revoked_at: None,
            },
        );
        Ok((participant, JoinCapability(token)))
    }

    /// Revoke a capability and every capability delegated from it. Returns
    /// the participants newly revoked; empty if none was active.
    pub fn revoke_tree(&mut self, participant_id: u64) -> Vec<u64> {
        let mut revoked = Vec::new();
        let mut pending = vec![participant_id];
        while let Some(id) = pending.pop() {
            let Some(record) = self.capabilities.get_mut(&id) else {
                continue;
            };
            if record.revoked {
                continue;
            }
            record.revoked = true;
            revoked.push(id);
            pending.extend(
                self.capabilities
                    .values()
                    .filter(|child| child.parent == Some(id))
                    .map(|child| child.participant.id),
            );
        }
        revoked
    }

    /// [`Self::revoke_tree`], recording `by` (the moderator participant
    /// who ordered it; `None` for the admin) on the revoked root so a
    /// checkpoint keeps the audit actor (#11778).
    pub fn revoke_tree_by(&mut self, participant_id: u64, by: Option<u64>) -> Vec<u64> {
        self.revoke_tree_at(participant_id, by, None)
    }

    /// [`Self::revoke_tree_by`], also recording `at` (Unix ms) as the
    /// revoke time of every capability it newly revokes (#11960), so the
    /// owner report and a checkpoint keep when each one stopped working.
    pub(crate) fn revoke_tree_at(
        &mut self,
        participant_id: u64,
        by: Option<u64>,
        at: Option<u64>,
    ) -> Vec<u64> {
        let revoked = self.revoke_tree(participant_id);
        if !revoked.is_empty()
            && let Some(record) = self.capabilities.get_mut(&participant_id)
        {
            record.revoked_by = by;
        }
        if at.is_some() {
            for id in &revoked {
                if let Some(record) = self.capabilities.get_mut(id) {
                    record.revoked_at = at;
                }
            }
        }
        revoked
    }

    /// The world's isolation.
    #[must_use]
    pub const fn isolation(&self) -> Isolation {
        self.isolation
    }

    /// The world's sharing and file settings.
    #[must_use]
    pub const fn policy(&self) -> WorldPolicy {
        WorldPolicy {
            isolation: self.isolation,
            cross_world_write: self.cross_world_write,
            cross_world_forced: self.cross_world_forced,
            file_scope: self.file_scope,
            file_write_override: self.file_write_override,
        }
    }

    /// The directory of accounts, groups and friends this world is held to.
    #[must_use]
    pub const fn directory(&self) -> &crate::directory::Directory {
        &self.directory
    }

    /// The directory of accounts, groups and friends this world is held to.
    pub const fn directory_mut(&mut self) -> &mut crate::directory::Directory {
        &mut self.directory
    }

    /// The directory and the private petnames, borrowed together.
    pub const fn directory_and_petnames_mut(
        &mut self,
    ) -> (
        &mut crate::directory::Directory,
        &mut crate::petname::Petnames,
    ) {
        (&mut self.directory, &mut self.petnames)
    }

    /// Replace the petname store, as the world's data directory is opened.
    pub fn set_petnames(&mut self, petnames: crate::petname::Petnames) {
        self.petnames = petnames;
    }

    /// Commit a chat message after validating capability and bounds.
    pub fn chat(&mut self, token: &JoinCapability, text: &str) -> Result<WorldEvent> {
        let event = self.prepare_chat(token, text)?;
        self.commit_prepared(event)
    }

    pub(crate) fn prepare_chat(
        &mut self,
        token: &JoinCapability,
        text: &str,
    ) -> Result<WorldEvent> {
        let participant_id = self.authorize(token, WorldScopes::CHAT)?;
        self.ensure_uses_left(participant_id)?;
        ensure!(
            text.len() <= self.limits.chat_bytes,
            "chat message exceeds {} bytes",
            self.limits.chat_bytes
        );
        let text = sanitize_chat(text);
        ensure!(!text.is_empty(), "chat message must not be empty");
        // Consume one token from the participant's shared-bucket frame
        // allowance; a refusal is the structured #2244 refusal (reason +
        // bounded retry value), not prose.
        let now_ms = u64::try_from(
            self.clock()
                .now_instant()
                .saturating_duration_since(self.chat_epoch)
                .as_millis(),
        )
        .unwrap_or(u64::MAX);
        self.chat_abuse.prune_idle_endpoints(now_ms);
        self.chat_abuse.prune_counters(now_ms, 60 * 60 * 1_000);
        self.chat_abuse
            .check_frame(chat_endpoint(participant_id), true, now_ms)
            .map_err(crate::abuse::RefusalError::from)?;
        // Prepare only: the caller journals the event and commit_prepared
        // applies it (chat() above does exactly that). Main's version of this
        // body ended in self.commit(...); merged into prepare_chat that
        // double-applied the event and commit_prepared refused it as stale.
        Ok(WorldEvent {
            sequence: self
                .sequence
                .checked_add(1)
                .context("world event sequence exhausted")?,
            participant_id,
            kind: WorldEventKind::Chat(text),
        })
    }

    /// Commit an opaque game input after validating capability and bounds.
    pub fn input(&mut self, token: &JoinCapability, input: &[u8]) -> Result<WorldEvent> {
        let event = self.prepare_input(token, input)?;
        self.commit_prepared(event)
    }

    /// Bind `author_key` (the player's ed25519 verifying key) to the
    /// participant the capability names, opting that participant into
    /// signed play (#2814): from now on its inputs must arrive as
    /// player-signed moves via [`WorldCore::input_signed`], and the
    /// unsigned path refuses them. Re-binding the same key is a no-op;
    /// binding a different key is refused (key rotation is out of scope).
    ///
    /// # Errors
    ///
    /// Fails if the capability does not authorize input, or a different
    /// key is already bound to the participant.
    pub fn bind_author_key(&mut self, token: &JoinCapability, author_key: [u8; 32]) -> Result<u64> {
        self.bind_author_key_entry(token, author_key)
            .map(|(participant_id, _entry)| participant_id)
    }

    /// As [`WorldCore::bind_author_key`], also returning the journal
    /// entry for a binding this call newly made (`None` for a same-key
    /// re-bind), so the actor can persist it (#4346).
    pub(crate) fn bind_author_key_entry(
        &mut self,
        token: &JoinCapability,
        author_key: [u8; 32],
    ) -> Result<(u64, Option<JournalEntry>)> {
        let participant_id = self.authorize(token, WorldScopes::INPUT)?;
        if let Some(bound) = self.author_keys.get(&participant_id) {
            ensure!(
                *bound == author_key,
                "participant {participant_id} already has a different author key bound"
            );
            Ok((participant_id, None))
        } else {
            self.author_keys.insert(participant_id, author_key);
            self.move_chains.entry(participant_id).or_default();
            Ok((
                participant_id,
                Some(JournalEntry::AuthorKeyBound {
                    participant_id,
                    author: hex_encode(&author_key),
                }),
            ))
        }
    }

    /// Validate a player-signed input and prepare its event, verifying
    /// the signed move against the participant's bound key and current
    /// chain position without advancing the chain (see
    /// [`WorldCore::record_signed_move`]).
    ///
    /// # Errors
    ///
    /// As [`WorldCore::prepare_input`], plus: the participant has no
    /// bound author key, or the signed move fails
    /// [`signed::MoveChain::verify`].
    pub(crate) fn prepare_input_signed(
        &self,
        token: &JoinCapability,
        input: &[u8],
        signed_move: &[u8],
    ) -> Result<(WorldEvent, crate::chain::Event)> {
        let participant_id = self.authorize(token, WorldScopes::INPUT)?;
        ensure!(
            !self.limits.region_sharded,
            "region-sharded worlds require entity-addressed input"
        );
        self.ensure_uses_left(participant_id)?;
        ensure!(
            input.len() <= self.limits.input_bytes,
            "world input exceeds {} bytes",
            self.limits.input_bytes
        );
        ensure!(!input.is_empty(), "world input must not be empty");
        let author = self.author_keys.get(&participant_id).copied().ok_or_else(|| {
            anyhow::anyhow!(
                "participant {participant_id} has no bound author key; bind one before signed play"
            )
        })?;
        let chain = self
            .move_chains
            .get(&participant_id)
            .cloned()
            .unwrap_or_default();
        let signed = chain.verify(signed_move, &self.world_id, input, &author)?;
        let sequence = self
            .sequence
            .checked_add(1)
            .context("world event sequence exhausted")?;
        Ok((
            WorldEvent {
                sequence,
                participant_id,
                kind: WorldEventKind::Input(input.to_vec()),
            },
            signed,
        ))
    }

    /// Record a signed move whose prepared event has just committed:
    /// advance the author's chain past it and keep the cross-referenced
    /// record. The actor calls this immediately after
    /// [`WorldCore::commit_prepared`] for the prepared event, while no
    /// other input can interleave.
    ///
    /// # Errors
    ///
    /// Fails if the participant has no bound author key, or the signed
    /// move is no longer the author's expected next move at commit time.
    pub(crate) fn record_signed_move(
        &mut self,
        event: &WorldEvent,
        signed: &crate::chain::Event,
    ) -> Result<signed::SignedMoveRecord> {
        let record = self.signed_move_record(event, signed)?;
        let chain = self.move_chains.entry(record.participant_id).or_default();
        chain.accept(signed);
        self.signed_moves.push(record.clone());
        Ok(record)
    }

    /// Compute the journal record for a signed move WITHOUT applying it:
    /// validate the move against the author's bound key and current chain
    /// position and build the record the journal persists. The actor
    /// persists this record atomically with the committed input (one
    /// journal transaction) BEFORE applying either in memory, then calls
    /// [`WorldCore::record_signed_move`] to apply — which recomputes the
    /// same record from unchanged state and advances the chain.
    ///
    /// # Errors
    ///
    /// Fails if the participant has no bound author key, or the signed
    /// move is not the author's expected next move.
    pub(crate) fn signed_move_record(
        &self,
        event: &WorldEvent,
        signed: &crate::chain::Event,
    ) -> Result<signed::SignedMoveRecord> {
        let participant_id = event.participant_id;
        let author = self
            .author_keys
            .get(&participant_id)
            .copied()
            .ok_or_else(|| {
                anyhow::anyhow!("participant {participant_id} has no bound author key")
            })?;
        let chain = self
            .move_chains
            .get(&participant_id)
            .cloned()
            .unwrap_or_default();
        ensure!(
            signed.header.seq_num == chain.next_seq() && signed.header.backlink == chain.frontier(),
            "signed move is no longer the author's expected next move at commit"
        );
        Ok(signed::SignedMoveRecord {
            world_sequence: event.sequence,
            participant_id,
            author,
            author_seq: signed.header.seq_num,
            event_id: signed.id,
            bytes: signed.bytes.clone(),
        })
    }

    /// Commit a player-signed game input (#2814): the move is verified
    /// against the participant's bound key and chain position, committed
    /// as a normal input event, and recorded as signed evidence.
    ///
    /// # Errors
    ///
    /// As [`WorldCore::prepare_input_signed`].
    pub fn input_signed(
        &mut self,
        token: &JoinCapability,
        input: &[u8],
        signed_move: &[u8],
    ) -> Result<WorldEvent> {
        let (event, signed) = self.prepare_input_signed(token, input, signed_move)?;
        let event = self.commit_prepared(event)?;
        self.record_signed_move(&event, &signed)?;
        Ok(event)
    }

    /// Accepted signed moves, in world sequence order (#2814): the
    /// evidence base an attestation witness replays.
    #[must_use]
    pub fn signed_moves(&self) -> &[signed::SignedMoveRecord] {
        &self.signed_moves
    }

    /// Ingest every accepted signed move's bytes into `store`, in world
    /// sequence order (#4346): the documented hand-off between the world
    /// and the event store. The store's existing ingest rules do the
    /// rest — a rival signed move at an already-accepted author sequence
    /// produces a fork proof there, whether it arrives before or after
    /// this hand-off. Returns one ingest outcome per record, in the
    /// same order; re-ingesting the same records is a duplicate, not an
    /// error.
    ///
    /// # Errors
    ///
    /// Returns the event store's error if an ingest fails at the
    /// storage layer. Refusals are outcomes, not errors.
    pub fn ingest_signed_moves(
        &self,
        store: &mut id_events::event_store::EventStore,
        now_ms: u64,
    ) -> Result<Vec<id_events::event_store::Ingest>> {
        let mut outcomes = Vec::with_capacity(self.signed_moves.len());
        for record in &self.signed_moves {
            outcomes.push(store.ingest(&record.bytes, now_ms)?);
        }
        Ok(outcomes)
    }

    pub(crate) fn prepare_input(&self, token: &JoinCapability, input: &[u8]) -> Result<WorldEvent> {
        let sequence = self
            .sequence
            .checked_add(1)
            .context("world event sequence exhausted")?;
        self.prepare_input_at(token, input, sequence, &HashMap::new())
    }

    /// Prepare one input of a group-commit batch (#8239): the caller assigns
    /// `sequence` explicitly (batch staging cannot use `prepare_input`,
    /// which always prepares at `sequence + 1` without advancing it), and
    /// `staged_spends` carries the uses already staged earlier in the batch
    /// for each capability in the chain, so a batch from one participant
    /// cannot over-admit past a uses limit that `spend` only reaches at
    /// commit time.
    pub(crate) fn prepare_input_staged(
        &self,
        token: &JoinCapability,
        input: &[u8],
        sequence: u64,
        staged_spends: &HashMap<u64, u32>,
    ) -> Result<WorldEvent> {
        self.prepare_input_at(token, input, sequence, staged_spends)
    }

    pub(super) fn prepare_input_at(
        &self,
        token: &JoinCapability,
        input: &[u8],
        sequence: u64,
        staged_spends: &HashMap<u64, u32>,
    ) -> Result<WorldEvent> {
        let participant_id = self.authorize(token, WorldScopes::INPUT)?;
        ensure!(
            !self.author_keys.contains_key(&participant_id),
            "participant {participant_id} has a bound author key; inputs must be player-signed (#2814)"
        );
        ensure!(
            !self.limits.region_sharded,
            "region-sharded worlds require entity-addressed input"
        );
        self.ensure_uses_left_staged(participant_id, staged_spends)?;
        ensure!(
            input.len() <= self.limits.input_bytes,
            "world input exceeds {} bytes",
            self.limits.input_bytes
        );
        ensure!(!input.is_empty(), "world input must not be empty");
        Ok(WorldEvent {
            sequence,
            participant_id,
            kind: WorldEventKind::Input(input.to_vec()),
        })
    }

    /// `ensure_uses_left` against committed uses plus the spends a batch has
    /// already staged (#8239): each capability in the chain must still have
    /// a use left after both.
    pub(crate) fn ensure_uses_left_staged(
        &self,
        participant_id: u64,
        staged_spends: &HashMap<u64, u32>,
    ) -> Result<()> {
        let mut left: Option<u32> = None;
        let mut current = Some(participant_id);
        while let Some(id) = current {
            let Some(record) = self.capabilities.get(&id) else {
                break;
            };
            if let Some(limit) = record.bounds.uses {
                let staged = staged_spends.get(&id).copied().unwrap_or(0);
                let remaining = limit.saturating_sub(record.used.saturating_add(staged));
                left = Some(left.map_or(remaining, |so_far: u32| so_far.min(remaining)));
            }
            current = record.parent;
        }
        ensure!(
            left.is_none_or(|left| left > 0),
            "world capability has no uses left"
        );
        Ok(())
    }

    /// Record one staged spend against the participant and each ancestor,
    /// mirroring `spend`'s chain walk, so the next staged prepare in the
    /// same batch sees it (#8239).
    pub(crate) fn note_staged_spend(
        &self,
        participant_id: u64,
        staged_spends: &mut HashMap<u64, u32>,
    ) {
        let mut current = Some(participant_id);
        while let Some(id) = current {
            *staged_spends.entry(id).or_insert(0) += 1;
            current = self.capabilities.get(&id).and_then(|record| record.parent);
        }
    }

    pub(crate) fn prepare_entity_input(
        &self,
        token: &JoinCapability,
        entity_ref: &str,
        input: &[u8],
    ) -> Result<WorldEvent> {
        validate_entity_ref(entity_ref)?;
        let participant_id = self.authorize(token, WorldScopes::INPUT)?;
        ensure!(
            !self.author_keys.contains_key(&participant_id),
            "participant {participant_id} has a bound author key; inputs must be player-signed (#2814)"
        );
        self.ensure_uses_left(participant_id)?;
        ensure!(
            input.len() <= self.limits.input_bytes,
            "world input exceeds {} bytes",
            self.limits.input_bytes
        );
        ensure!(!input.is_empty(), "world input must not be empty");
        ensure!(
            !self.entity_is_fenced(entity_ref),
            "entity {entity_ref:?} is fenced after export"
        );
        let sequence = self
            .sequence
            .checked_add(1)
            .context("world event sequence exhausted")?;
        Ok(WorldEvent {
            sequence,
            participant_id,
            kind: WorldEventKind::EntityInput {
                entity_ref: entity_ref.to_owned(),
                input: input.to_vec(),
            },
        })
    }

    /// The most recent ownership transfer recorded for `entity_ref`, if
    /// any. The ledger is append-only and chronological, so the last entry
    /// for an entity is its current ownership state; earlier entries are
    /// history, retained for handoff-ID deduplication.
    pub(crate) fn latest_transfer(&self, entity_ref: &str) -> Option<&HandoffTransferState> {
        self.handoff_transfers
            .iter()
            .rev()
            .find(|transfer| transfer.entity_ref == entity_ref)
    }

    /// An entity is fenced while its latest ownership transfer is an
    /// export: the fence derives from the current ownership state, not
    /// from any historical export. An entity imported back (design #2818
    /// §3 permits later crossings) is writable again; every historical
    /// receipt stays in the ledger for deduplication.
    pub(crate) fn entity_is_fenced(&self, entity_ref: &str) -> bool {
        matches!(
            self.latest_transfer(entity_ref),
            Some(transfer) if transfer.direction == HandoffDirection::Export
        )
    }

    /// Install a durable source export in the in-memory transfer ledger. The
    /// caller must append the matching journal entry first.
    pub(crate) fn validate_entity_export(&self, transfer: &HandoffTransferState) -> Result<()> {
        ensure!(
            self.limits.region_sharded,
            "entity export requires a region-sharded world"
        );
        ensure!(
            transfer.direction == HandoffDirection::Export,
            "source export has the wrong transfer direction"
        );
        validate_entity_ref(&transfer.entity_ref)?;
        ensure!(
            transfer
                .payload
                .as_ref()
                .is_some_and(|payload| payload.len() <= MAX_ENTITY_TRANSFER_PAYLOAD_BYTES),
            "entity transfer payload exceeds {MAX_ENTITY_TRANSFER_PAYLOAD_BYTES} bytes"
        );
        let payload = transfer.payload.as_deref().unwrap_or_default();
        ensure!(
            transfer.payload_digest == hex_encode(blake3::hash(payload).as_bytes()),
            "entity transfer payload digest does not match"
        );
        ensure!(
            !self
                .handoff_transfers
                .iter()
                .any(|existing| existing.handoff_id == transfer.handoff_id)
                && !matches!(
                    self.latest_transfer(&transfer.entity_ref),
                    Some(latest) if latest.direction == HandoffDirection::Export
                ),
            "entity already has an active export or handoff ID is duplicated"
        );
        ensure_epoch_chains(transfer, &self.handoff_transfers)?;
        Ok(())
    }

    /// Install a source fence after its matching journal record is durable.
    pub(crate) fn record_entity_export(&mut self, transfer: HandoffTransferState) {
        debug_assert!(self.validate_entity_export(&transfer).is_ok());
        self.handoff_transfers.push(transfer);
        self.since_checkpoint = self.since_checkpoint.saturating_add(1);
    }

    /// Validate a destination import against the dedupe ledger (#3870
    /// phase 3). The actor has already verified the envelope's module
    /// hash, schema and payload digest before mutation; this checks the
    /// durable-state invariants: direction, owner-epoch advance, no
    /// duplicate payload copy, and that neither the handoff ID nor the
    /// entity already carries an import.
    pub(crate) fn validate_entity_import(&self, transfer: &HandoffTransferState) -> Result<()> {
        ensure!(
            self.limits.region_sharded,
            "entity import requires a region-sharded world"
        );
        ensure!(
            transfer.direction == HandoffDirection::Import,
            "destination import has the wrong transfer direction"
        );
        validate_entity_ref(&transfer.entity_ref)?;
        ensure!(
            transfer.new_owner_epoch.is_some_and(|new_owner_epoch| {
                transfer.prior_owner_epoch.checked_add(1) == Some(new_owner_epoch)
            }),
            "destination import does not advance the owner epoch"
        );
        ensure!(
            transfer.payload.is_none(),
            "destination import carries a duplicate payload outside its event"
        );
        ensure!(
            !self
                .handoff_transfers
                .iter()
                .any(|existing| existing.handoff_id == transfer.handoff_id)
                && !matches!(
                    self.latest_transfer(&transfer.entity_ref),
                    Some(latest) if latest.direction == HandoffDirection::Import
                ),
            "entity already has an active import or handoff ID is duplicated"
        );
        ensure_epoch_chains(transfer, &self.handoff_transfers)?;
        Ok(())
    }

    /// Install an import receipt after its matching journal record is durable.
    pub(crate) fn record_entity_import(&mut self, transfer: HandoffTransferState) {
        debug_assert!(self.validate_entity_import(&transfer).is_ok());
        self.handoff_transfers.push(transfer);
        self.since_checkpoint = self.since_checkpoint.saturating_add(1);
    }

    pub(crate) fn commit_prepared(&mut self, event: WorldEvent) -> Result<WorldEvent> {
        let next_sequence = self
            .sequence
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("world sequence exhausted"))?;
        ensure!(
            event.sequence == next_sequence,
            "prepared world event is stale"
        );
        self.sequence = event.sequence;
        self.spend(event.participant_id);
        self.since_checkpoint = self.since_checkpoint.saturating_add(1);
        self.events.push_back(event.clone());
        while self.events.len() > self.limits.retained_events {
            self.events.pop_front();
        }
        self.record_admin_history(event.clone());
        Ok(event)
    }

    /// Add an event to the bounded, ordered history reserved for admin export.
    pub(crate) fn record_admin_history(&mut self, event: WorldEvent) {
        if let WorldEventKind::System { event } = &event.kind
            && let Some((participant, joined)) = presence_change(event)
        {
            if joined {
                self.open_presence.insert(participant);
            } else {
                self.open_presence.remove(&participant);
            }
        }
        self.admin_history_ledger
            .track(admin_history_event_bytes(&event));
        self.admin_history.push_back(event);
        while self.admin_history.len() > self.limits.retained_events.min(MAX_ADMIN_HISTORY_EVENTS)
            || self.admin_history_ledger.effective_bytes() > MAX_ADMIN_HISTORY_BYTES
        {
            if self.admin_history.pop_front().is_none() {
                break;
            }
            self.admin_history_ledger.untrack_front();
            self.admin_history_complete = false;
        }
    }

    pub(super) fn restore_admin_history(
        &mut self,
        events: Vec<WorldEvent>,
        complete: bool,
        sequence: u64,
    ) -> Result<()> {
        ensure!(
            events.len() <= self.limits.retained_events.min(MAX_ADMIN_HISTORY_EVENTS),
            "checkpoint admin history exceeds its event bound"
        );
        ensure!(
            admin_history_bytes(&events) <= MAX_ADMIN_HISTORY_BYTES,
            "checkpoint admin history exceeds its byte bound"
        );
        let mut previous: Option<u64> = None;
        for event in &events {
            match &event.kind {
                WorldEventKind::System { .. } => ensure!(
                    event.sequence == 0 && event.participant_id == 0,
                    "checkpoint admin history system event claims a sequence"
                ),
                _ => {
                    ensure!(
                        event.sequence > 0,
                        "checkpoint admin history event has no sequence"
                    );
                    if let Some(last) = previous {
                        ensure!(
                            event.sequence == last.saturating_add(1),
                            "checkpoint admin history sequence jumps from {last} to {}",
                            event.sequence
                        );
                    }
                    previous = Some(event.sequence);
                }
            }
        }
        ensure!(
            previous.is_none_or(|last| last <= sequence),
            "checkpoint admin history contains a sequence beyond {sequence}"
        );
        ensure!(
            !complete || previous.unwrap_or(0) == sequence,
            "complete checkpoint admin history ends at sequence {}, not {sequence}",
            previous.unwrap_or(0)
        );
        ensure!(
            !complete
                || sequence == 0
                || events
                    .iter()
                    .find(|event| event.sequence > 0)
                    .is_some_and(|event| event.sequence == 1),
            "complete checkpoint admin history does not start at sequence 1"
        );
        self.admin_history_ledger = AdminHistoryByteLedger::rebuild(&events);
        self.admin_history = events.into();
        self.admin_history_complete = complete;
        Ok(())
    }

    /// Return the current bounded snapshot after validating join scope. Its
    /// events are the viewer's projection: no chat without `chat` or
    /// `history`, and no install seeds (#240).
    pub fn snapshot(&self, token: &JoinCapability) -> Result<WorldSnapshot> {
        let participant_id = self.authorize(token, WorldScopes::JOIN)?;
        Ok(self.snapshot_unchecked(self.viewer_scopes(participant_id)))
    }

    /// What the viewer holding `token` may see of `event`, as it is delivered
    /// live. `Err` if the capability no longer joins (revoked, expired, or its
    /// chain was cut), so a session can end; `Ok(None)` if the event is not
    /// for this viewer.
    pub fn project_for(
        &self,
        token: &JoinCapability,
        event: &WorldEvent,
    ) -> Result<Option<WorldEvent>> {
        let participant_id = self.authorize(token, WorldScopes::JOIN)?;
        Ok(project_event_for(self.viewer_scopes(participant_id), event))
    }

    pub(crate) fn viewer_scopes(&self, participant_id: u64) -> WorldScopes {
        self.effective_scopes(participant_id)
            .unwrap_or(WorldScopes::EMPTY)
    }

    pub(crate) fn viewer(&self, token: &JoinCapability) -> Result<Participant> {
        let id = self.authorize(token, WorldScopes::JOIN)?;
        self.capabilities
            .get(&id)
            .map(|record| record.participant.clone())
            .context("world participant is unavailable")
    }

    /// Return retained events newer than `after_sequence`, plus the current
    /// cursor. If the cursor fell behind retention, the caller must resnapshot.
    /// The events are the viewer's projection (see [`project_event_for`]), so
    /// their sequences may skip withheld lines: a caller advances to
    /// `current_sequence`, not to the last sequence it was given plus one.
    /// Retention and `needs_snapshot` do not depend on the viewer.
    pub fn events_after(&self, token: &JoinCapability, after_sequence: u64) -> Result<EventPage> {
        let participant_id = self.authorize(token, WorldScopes::JOIN)?;
        let scopes = self.viewer_scopes(participant_id);
        let oldest = self.oldest_sequence();
        let needs_snapshot = after_sequence.saturating_add(1) < oldest;
        let events = if needs_snapshot {
            Vec::new()
        } else {
            self.events
                .iter()
                .filter(|event| event.sequence > after_sequence)
                .filter_map(|event| project_event_for(scopes, event))
                .collect()
        };
        Ok(EventPage {
            current_sequence: self.sequence,
            oldest_retained_sequence: oldest,
            needs_snapshot,
            events,
        })
    }

    pub(crate) fn authorize(&self, token: &JoinCapability, required: WorldScopes) -> Result<u64> {
        let (participant, _) = token
            .expose()
            .split_once('.')
            .ok_or_else(|| CapabilityDenied::new("malformed world capability"))?;
        let participant_id = participant
            .parse::<u64>()
            .map_err(|_| CapabilityDenied::new("malformed world capability participant ID"))?;
        let digest = token_digest(token.expose());
        let record = self.capabilities.get(&participant_id).ok_or_else(|| {
            CapabilityDenied::new("invalid, revoked, or insufficient world capability")
        })?;
        if !constant_time_eq(&digest, &record.digest) {
            return Err(CapabilityDenied::new(
                "invalid, revoked, or insufficient world capability",
            )
            .into());
        }
        // The authority check must not seed from `ALL`: that set excludes
        // the admin bits by design, which would make `BUILDER` /
        // `MODERATOR` unusable through this path. Seed with every
        // recognized bit instead — the ancestor-chain intersection in
        // `effective_scopes_seeded` only removes bits, so a required
        // authority bit survives only when every record in the chain
        // actually carries it.
        if !(!record.revoked
            && self
                .effective_scopes_seeded(participant_id, WorldScopes::all_recognized())
                .is_some_and(|scopes| scopes.contains(required)))
        {
            return Err(CapabilityDenied::new(
                "invalid, revoked, or insufficient world capability",
            )
            .into());
        }
        self.ensure_not_expired(participant_id)?;
        Ok(participant_id)
    }

    pub(crate) fn oldest_sequence(&self) -> u64 {
        self.events
            .front()
            .map_or_else(|| self.sequence.saturating_add(1), |event| event.sequence)
    }

    pub(crate) fn snapshot_unchecked(&self, viewer: WorldScopes) -> WorldSnapshot {
        let participants = self
            .capabilities
            .values()
            .filter(|record| !record.revoked && record.scopes.contains(WorldScopes::JOIN))
            .map(|record| record.participant.clone())
            .collect();
        // Take the most recent events that fit the snapshot budget, newest
        // first. A snapshot is a baseline, not a complete replay: clients
        // adopt its sequence as their cursor. Events the viewer may not see
        // are dropped before the budget is spent, so it counts only what the
        // snapshot carries.
        let mut budget = self.limits.snapshot_bytes;
        let mut events: VecDeque<WorldEvent> = VecDeque::new();
        for event in self
            .events
            .iter()
            .rev()
            .filter_map(|event| project_event_for(viewer, event))
        {
            let size = serde_json::to_vec(&event).map_or(budget, |encoded| encoded.len());
            if size > budget {
                break;
            }
            budget -= size;
            events.push_front(event);
        }
        let oldest_retained_sequence = events
            .front()
            .map_or_else(|| self.sequence.saturating_add(1), |event| event.sequence);
        WorldSnapshot {
            world_id: self.world_id.clone(),
            active_module_hash: self.active_module_hash.clone(),
            active_module_provenance: self.active_module_provenance.clone(),
            current_sequence: self.sequence,
            oldest_retained_sequence,
            events: events.into_iter().collect(),
            participants,
        }
    }
}
