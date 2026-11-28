// Moderation, policy, and capability-ceiling operations.

use super::*;

impl WorldActor {
    pub(in crate::world::actor) fn require_moderation(
        &self,
        authority: &Authority,
        what: &str,
    ) -> Result<()> {
        let directory = self.core.directory();
        let allowed = match authority {
            Authority::Admin => true,
            Authority::Signed(key) => directory
                .account_for_key(key)
                .is_some_and(|account| directory.is_moderator(&account)),
        };
        if allowed {
            Ok(())
        } else {
            Err(crate::directory::Refusal::Forbidden(format!(
                "only an admin or a moderator {what}"
            ))
            .into())
        }
    }

    /// The world's owner (an account holding an admin seat in one of its
    /// groups) or the operator (the process admin) may change its
    /// lifecycle; everyone else gets the same forbidden refusal any other
    /// owner-only act gives (#10694).
    pub(in crate::world::actor) fn require_ownership(
        &self,
        authority: &Authority,
        what: &str,
    ) -> Result<()> {
        let directory = self.core.directory();
        let allowed = match authority {
            Authority::Admin => true,
            Authority::Signed(key) => directory.account_for_key(key).is_some_and(|account| {
                directory.groups().any(|group| {
                    directory.level_in(&account, group.id) == Some(crate::directory::Level::Admin)
                })
            }),
        };
        if allowed {
            Ok(())
        } else {
            Err(crate::directory::Refusal::Forbidden(format!(
                "only the world's owner or the operator {what}"
            ))
            .into())
        }
    }

    pub(in crate::world::actor) fn set_isolation(
        &mut self,
        authority: &Authority,
        isolation: Isolation,
    ) -> Result<Isolation> {
        self.check_writable()?;
        self.require_moderation(authority, "changes isolation")?;
        self.persist(JournalEntry::Isolation { isolation })?;
        self.core.isolation = isolation;
        Ok(isolation)
    }

    pub(in crate::world::actor) fn set_cross_world_write(
        &mut self,
        authority: &Authority,
        enabled: bool,
    ) -> Result<WorldPolicy> {
        self.check_writable()?;
        self.require_moderation(authority, "changes cross-world writes")?;
        if !enabled && self.core.cross_world_forced && *authority != Authority::Admin {
            return Err(crate::directory::Refusal::Forbidden(
                "the admin forced cross-world writes on; only the admin can turn them off"
                    .to_owned(),
            )
            .into());
        }
        self.persist(JournalEntry::CrossWorldWrite { enabled })?;
        self.core.cross_world_write = enabled;
        if !enabled {
            self.core.cross_world_forced = false;
        }
        Ok(self.core.policy())
    }

    pub(in crate::world::actor) fn force_cross_world_write(
        &mut self,
        authority: &Authority,
    ) -> Result<WorldPolicy> {
        self.check_writable()?;
        if *authority != Authority::Admin {
            return Err(crate::directory::Refusal::Forbidden(
                "only the admin forces cross-world writes".to_owned(),
            )
            .into());
        }
        if self.core.isolation == Isolation::Isolated {
            return Err(crate::directory::Refusal::Forbidden(
                "an isolated world cannot be forced to accept cross-world writes; unisolate it first"
                    .to_owned(),
            )
            .into());
        }
        self.persist(JournalEntry::CrossWorldWriteForced)?;
        self.core.cross_world_write = true;
        self.core.cross_world_forced = true;
        Ok(self.core.policy())
    }

    pub(in crate::world::actor) fn set_file_scope(
        &mut self,
        authority: &Authority,
        scope: FileScope,
        write_override: Option<bool>,
    ) -> Result<WorldPolicy> {
        self.check_writable()?;
        self.require_moderation(authority, "changes the file scope")?;
        if write_override.is_some() && *authority != Authority::Admin {
            return Err(crate::directory::Refusal::Forbidden(
                "only the admin sets the file write override".to_owned(),
            )
            .into());
        }
        self.persist(JournalEntry::FileScope { scope })?;
        self.core.file_scope = scope;
        if let Some(enabled) = write_override {
            self.persist(JournalEntry::FileWriteOverride { enabled })?;
            self.core.file_write_override = enabled;
        }
        Ok(self.core.policy())
    }

    /// Replace the world's capability ceiling. It takes effect now: uses and
    /// subscriptions the ceiling excludes are refused from here on. Grants are
    /// kept, so lifting the ceiling again restores them.
    pub(in crate::world::actor) fn set_caps_ceiling(
        &mut self,
        ceiling: Vec<String>,
    ) -> Result<CapsReport> {
        self.check_writable()?;
        crate::world_caps::validate_ceiling_names(&ceiling)?;
        let ceiling: BTreeSet<String> = ceiling.into_iter().collect();
        self.persist(JournalEntry::CapsCeiling {
            ceiling: ceiling.iter().cloned().collect(),
        })?;
        self.core.caps_ceiling = ceiling;
        self.core.sync_ledger_ceiling();
        self.reconcile_subscriptions();
        self.caps_report()
    }

    /// Re-check subscriptions after an authority change. An armed one that is
    /// no longer granted is dropped and its tick disarmed. Every refused one is
    /// dropped too, so the next pass decides it again: a refusal that still
    /// holds is announced again, and one the change now grants is armed.
    pub(in crate::world::actor) fn reconcile_subscriptions(&mut self) {
        let ledger = &self.core.ledger;
        let refused = &self.denied_subs;
        self.active_subs
            .retain(|sub| !refused.contains(&sub.spec()) && ledger.is_granted(sub.cap()));
        self.denied_subs.clear();
        let active = &self.active_subs;
        self.ticks
            .retain(|ms, _| active.contains(&Subscription::Tick(*ms)));
        // Location watches disarm alongside ticks (#2341, PR #8643
        // Codex P1): a watch whose capability authority was removed
        // must stop delivering readings, not merely lose its
        // subscription record.
        self.loc_watches
            .retain(|ms, _| active.contains(&Subscription::LocWatch(*ms)));
    }

    /// The capability report an admin sees.
    pub(in crate::world::actor) fn caps_report(&mut self) -> Result<CapsReport> {
        let requested = match self.program.wants() {
            Ok(Some(text)) => crate::world_caps::parse_wants(&text)
                .map(|wants| wants.caps().into_iter().collect::<Vec<_>>())
                .unwrap_or_default(),
            Ok(None) => Vec::new(),
            Err(_) => Vec::new(),
        };
        let granted: Vec<String> = self.core.ledger.granted().iter().cloned().collect();
        let effective: Vec<String> = self.core.ledger.effective().into_iter().collect();
        let missing: Vec<String> = requested
            .iter()
            .filter(|cap| !effective.contains(cap))
            .cloned()
            .collect();
        Ok(CapsReport {
            granted,
            requested,
            missing,
            usage: self.core.ledger.usage().clone(),
            policy: if self.core.ledger.grant_on_use() {
                "grant-on-use"
            } else {
                "deny"
            }
            .to_owned(),
            catalog: crate::world_caps::PROVIDER_REGISTRY
                .ordered_names()
                .into_iter()
                .map(str::to_owned)
                .collect(),
            ceiling: self.core.caps_ceiling.iter().cloned().collect(),
            effective,
            view_failures: self.view_failures,
        })
    }
}
