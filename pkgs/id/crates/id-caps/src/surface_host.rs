//! Concurrency and conflict handling on shared surfaces — v3
//! shared-agent-worlds step 2 (#10050, parent #6302; outline §2.2, §6).
//!
//! The world host is the single writer: [`SurfaceHost`] puts a world's
//! [`WorkSurfaces`] behind one lock, so writes apply one revision at a
//! time. Each write names the revision it was decided against (v1 §4.3's
//! stale-decision token); [`WorkSurfaces`] refuses a write against a
//! revision the surface has moved past and hands back what the writer
//! missed. Nothing is ever merged silently.
//!
//! If the writer fails mid-write (the lock is poisoned), the host can no
//! longer promise one revision at a time, so it refuses every later
//! write with [`SurfaceError::HostCannotSerialize`], audited as
//! [`EVENT_HOST_REFUSED`], until it is restarted from its state
//! ([`SurfaceHost::into_surfaces`]). Reads still work.
//!
//! # The retry contract
//!
//! An agent whose edit is refused as a stale decision recovers without
//! rejoining: it re-reads the document, reconciles its intent against
//! the edits it missed, and retries against the new revision under the
//! same grant. [`SurfaceHost::edit_with_retry`] is that loop. The
//! agent's `decide` callback sees the current document and, on a retry,
//! the refusal that sent it back; it returns the edit to make, or `None`
//! to give up because the change is no longer wanted.

use std::sync::{Mutex, MutexGuard, PoisonError};

use crate::role_config::RoleRegistry;
use crate::work_surfaces::{
    DocumentView, EditOp, StaleDecision, SurfaceActor, SurfaceError, SurfaceId, WorkSurfaces,
};

/// Audit kind for a write the host refused because it cannot serialize.
pub const EVENT_HOST_REFUSED: &str = "surface.host.refused";

/// How a retried edit ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RetryOutcome {
    /// The edit landed.
    Edited {
        /// The revision it produced.
        revision: u64,
        /// Stale-decision refusals before it landed.
        rejections: u32,
    },
    /// The agent's reconciliation decided the edit is no longer wanted.
    Abandoned {
        /// Stale-decision refusals before it gave up.
        rejections: u32,
    },
}

/// A world's shared surfaces behind the host's single writer.
#[derive(Debug)]
pub struct SurfaceHost {
    surfaces: Mutex<WorkSurfaces>,
}

impl SurfaceHost {
    /// Host `surfaces`.
    #[must_use]
    pub const fn new(surfaces: WorkSurfaces) -> Self {
        Self {
            surfaces: Mutex::new(surfaces),
        }
    }

    /// Whether the host can still serialize writes.
    #[must_use]
    pub fn serializes(&self) -> bool {
        !self.surfaces.is_poisoned()
    }

    /// Apply one write to `surface` as `actor`, alone. A host whose
    /// writer failed refuses it, audited, rather than risk a merge.
    ///
    /// # Errors
    ///
    /// The write's own refusal, or [`SurfaceError::HostCannotSerialize`].
    pub fn write<T>(
        &self,
        actor: &SurfaceActor,
        surface: &SurfaceId,
        now: u64,
        act: impl FnOnce(&mut WorkSurfaces) -> Result<T, SurfaceError>,
    ) -> Result<T, SurfaceError> {
        match self.surfaces.lock() {
            Ok(mut surfaces) => act(&mut surfaces),
            Err(poisoned) => {
                let error = SurfaceError::HostCannotSerialize;
                poisoned.into_inner().record_refusal(
                    EVENT_HOST_REFUSED,
                    surface,
                    actor,
                    &error,
                    now,
                );
                Err(error)
            }
        }
    }

    /// Read the surfaces. Reads are audited too, so they take the lock;
    /// they still work on a host that can no longer serialize writes.
    pub fn read<T>(&self, read: impl FnOnce(&mut WorkSurfaces) -> T) -> T {
        read(&mut self.lock_even_if_poisoned())
    }

    /// The hosted state, for a restart: a new host over it serializes
    /// again.
    #[must_use]
    pub fn into_surfaces(self) -> WorkSurfaces {
        self.surfaces
            .into_inner()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Edit a document under the retry contract: read it, let `decide`
    /// choose the edit, write it against the revision read, and on a
    /// stale-decision refusal re-read and let `decide` reconcile, at most
    /// `max_attempts` writes.
    ///
    /// # Errors
    ///
    /// Any refusal other than a stale decision, or the last stale
    /// decision once `max_attempts` writes were refused.
    pub fn edit_with_retry(
        &self,
        registry: &RoleRegistry,
        actor: &SurfaceActor,
        document: &str,
        max_attempts: u32,
        now: u64,
        mut decide: impl FnMut(&DocumentView, Option<&StaleDecision>) -> Option<EditOp>,
    ) -> Result<RetryOutcome, SurfaceError> {
        let surface = SurfaceId::Document(document.to_owned());
        let mut rejections = 0;
        let mut refused: Option<Box<StaleDecision>> = None;
        loop {
            let view = self.read(|s| s.document(registry, actor, document, now))?;
            let Some(op) = decide(&view, refused.as_deref()) else {
                return Ok(RetryOutcome::Abandoned { rejections });
            };
            let written = self.write(actor, &surface, now, |s| {
                s.edit_document(registry, actor, document, op, view.revision, now)
            });
            match written {
                Ok(revision) => {
                    return Ok(RetryOutcome::Edited {
                        revision,
                        rejections,
                    });
                }
                Err(SurfaceError::StaleDecision(stale)) => {
                    rejections += 1;
                    if rejections >= max_attempts {
                        return Err(SurfaceError::StaleDecision(stale));
                    }
                    refused = Some(stale);
                }
                Err(other) => return Err(other),
            }
        }
    }

    fn lock_even_if_poisoned(&self) -> MutexGuard<'_, WorkSurfaces> {
        self.surfaces.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::agent_delegation::AgentId;
    use crate::work_surfaces::{
        BoardDefinition, ConflictModel, DocumentDefinition, DocumentPermissions, ItemDraft,
        ReviewState,
    };

    const WORLD: &str = "studio";
    const T0: u64 = 1_800_000_000;
    const EDITORS: u8 = 8;

    fn agent(byte: u8) -> AgentId {
        AgentId([byte; 32])
    }

    /// Agents 1..=EDITORS are participants.
    fn registry() -> RoleRegistry {
        let mut registry = RoleRegistry::new(WORLD);
        registry.install_starters("owner", T0);
        for byte in 1..=EDITORS {
            assert!(
                registry
                    .grant("participant", agent(byte), "owner", T0)
                    .decision
                    .is_ok()
            );
        }
        registry
    }

    fn editor(byte: u8) -> SurfaceActor {
        SurfaceActor::agent(agent(byte), "participant")
    }

    fn owner() -> SurfaceActor {
        SurfaceActor::owner("owner")
    }

    fn document(name: &str, model: ConflictModel) -> DocumentDefinition {
        DocumentDefinition {
            name: name.to_owned(),
            conflict_model: model,
            roles: BTreeMap::from([(
                "participant".to_owned(),
                DocumentPermissions {
                    read: true,
                    edit: true,
                    review: true,
                },
            )]),
        }
    }

    fn host() -> SurfaceHost {
        let mut surfaces = WorkSurfaces::new(WORLD);
        surfaces
            .define_document(
                &owner(),
                document("plan", ConflictModel::LastWriterWins),
                T0,
            )
            .unwrap();
        surfaces
            .define_document(&owner(), document("log", ConflictModel::AppendOnly), T0)
            .unwrap();
        surfaces
            .define_board(&owner(), BoardDefinition::starter("work"), T0)
            .unwrap();
        SurfaceHost::new(surfaces)
    }

    fn plan() -> SurfaceId {
        SurfaceId::Document("plan".to_owned())
    }

    fn replace(text: &str) -> EditOp {
        EditOp::Replace {
            text: text.to_owned(),
        }
    }

    fn outcomes(host: &SurfaceHost, kind: &str) -> Vec<String> {
        host.read(|s| {
            s.events()
                .iter()
                .filter(|e| e.kind == kind)
                .map(|e| e.outcome.clone())
                .collect()
        })
    }

    #[test]
    fn a_second_edit_decided_against_an_old_revision_is_rejected_with_the_diff() {
        let r = registry();
        let host = host();
        // Both agents read revision 0 and decide.
        let seen = host.read(|s| s.document(&r, &editor(1), "plan", T0).unwrap().revision);
        assert_eq!(seen, 0);

        let first = host.write(&editor(1), &plan(), T0 + 1, |s| {
            s.edit_document(&r, &editor(1), "plan", replace("ship friday"), seen, T0 + 1)
        });
        assert_eq!(first, Ok(1));

        let second = host.write(&editor(2), &plan(), T0 + 2, |s| {
            s.edit_document(&r, &editor(2), "plan", replace("ship monday"), seen, T0 + 2)
        });
        let Err(SurfaceError::StaleDecision(stale)) = second else {
            panic!("expected a stale decision, got {second:?}");
        };
        assert_eq!((stale.decided_against, stale.current), (0, 1));
        assert_eq!(stale.surface, plan());
        assert_eq!(stale.missed.len(), 1);
        assert_eq!(stale.missed[0].author, editor(1));
        assert_eq!(stale.missed[0].op, replace("ship friday"));

        // No silent merge, no lost update: the first edit stands.
        let view = host.read(|s| s.document(&r, &editor(2), "plan", T0).unwrap());
        assert_eq!(view.content, "ship friday");
        assert_eq!(view.revision, 1);

        // The refusal is an audited world event.
        assert_eq!(
            outcomes(&host, crate::work_surfaces::EVENT_DOCUMENT_EDIT),
            ["executed", "refused:stale_decision"]
        );

        // The agent reconciles and retries under the same grant, without
        // rejoining; the first agent's text is kept in history.
        let retried = host.write(&editor(2), &plan(), T0 + 3, |s| {
            s.edit_document(
                &r,
                &editor(2),
                "plan",
                replace("ship friday, monday if blocked"),
                stale.current,
                T0 + 3,
            )
        });
        assert_eq!(retried, Ok(2));
        let view = host.read(|s| s.document(&r, &editor(2), "plan", T0).unwrap());
        assert_eq!(view.history[1].replaced.as_deref(), Some("ship friday"));
    }

    #[test]
    fn appends_are_stale_checked_too() {
        let r = registry();
        let host = host();
        let log = SurfaceId::Document("log".to_owned());
        let append = |text: &str| EditOp::Append {
            text: text.to_owned(),
        };
        host.write(&editor(1), &log, T0, |s| {
            s.edit_document(&r, &editor(1), "log", append("one\n"), 0, T0)
        })
        .unwrap();
        let refused = host.write(&editor(2), &log, T0, |s| {
            s.edit_document(&r, &editor(2), "log", append("two\n"), 0, T0)
        });
        assert!(matches!(refused, Err(SurfaceError::StaleDecision(_))));
        // A revision from the future is refused as well.
        let refused = host.write(&editor(2), &log, T0, |s| {
            s.edit_document(&r, &editor(2), "log", append("two\n"), 9, T0)
        });
        let Err(SurfaceError::StaleDecision(stale)) = refused else {
            panic!("expected a stale decision");
        };
        assert!(stale.missed.is_empty());
        assert_eq!(stale.current, 1);
    }

    #[test]
    fn a_review_of_a_revision_that_moved_on_is_refused() {
        let r = registry();
        let host = host();
        host.write(&editor(1), &plan(), T0, |s| {
            s.edit_document(&r, &editor(1), "plan", replace("draft"), 0, T0)
        })
        .unwrap();
        host.write(&editor(2), &plan(), T0, |s| {
            s.edit_document(&r, &editor(2), "plan", replace("draft 2"), 1, T0)
        })
        .unwrap();
        // Agent 1 approves what it read, revision 1: refused.
        let approve = |seen| {
            host.write(&editor(1), &plan(), T0, |s| {
                s.review_document(&r, &editor(1), "plan", ReviewState::Approved, seen, T0)
            })
        };
        assert!(matches!(approve(1), Err(SurfaceError::StaleDecision(_))));
        let view = host.read(|s| s.document(&r, &editor(1), "plan", T0).unwrap());
        assert_eq!(view.review, ReviewState::Draft);
        assert_eq!(approve(2), Ok(()));
    }

    #[test]
    fn a_stale_claim_names_the_item_as_it_stands() {
        let r = registry();
        let host = host();
        let board = SurfaceId::Board("work".to_owned());
        let id = host
            .write(&editor(1), &board, T0, |s| {
                s.create_item(
                    &r,
                    &editor(1),
                    "work",
                    ItemDraft {
                        title: "fix the gate".to_owned(),
                        ..ItemDraft::default()
                    },
                    T0,
                )
            })
            .unwrap();
        let seen = host.read(|s| s.board(&r, &editor(1), "work", T0).unwrap().items[0].version);
        host.write(&editor(1), &board, T0, |s| {
            s.claim(&r, &editor(1), "work", id, seen, T0)
        })
        .unwrap();
        let refused = host.write(&editor(2), &board, T0, |s| {
            s.claim(&r, &editor(2), "work", id, seen, T0)
        });
        let Err(SurfaceError::Stale { item, .. }) = refused else {
            panic!("expected a stale claim, got {refused:?}");
        };
        assert_eq!(item.claim.unwrap().holder, editor(1));
    }

    #[test]
    fn no_lost_update_across_concurrent_editors() {
        let r = registry();
        let host = host();
        // Every editor increments a shared counter: the textbook lost
        // update when two read the same value and both write.
        host.write(&owner(), &plan(), T0, |s| {
            s.edit_document(&r, &owner(), "plan", replace("0"), 0, T0)
        })
        .unwrap();
        let rounds: u32 = 5;
        let rejections: u32 = std::thread::scope(|scope| {
            let workers: Vec<_> = (1..=EDITORS)
                .map(|byte| {
                    let (r, host) = (&r, &host);
                    scope.spawn(move || {
                        let mut refused = 0;
                        for _ in 0..rounds {
                            let outcome = host
                                .edit_with_retry(r, &editor(byte), "plan", 10_000, T0, |view, _| {
                                    let n: u64 = view.content.parse().unwrap();
                                    Some(replace(&(n + 1).to_string()))
                                })
                                .unwrap();
                            let RetryOutcome::Edited { rejections, .. } = outcome else {
                                panic!("an increment is never abandoned");
                            };
                            refused += rejections;
                            std::thread::yield_now();
                        }
                        refused
                    })
                })
                .collect();
            workers.into_iter().map(|w| w.join().unwrap()).sum()
        });
        let view = host.read(|s| s.document(&r, &owner(), "plan", T0).unwrap());
        let editors = u64::from(EDITORS);
        let total = editors * u64::from(rounds);
        assert_eq!(view.content, total.to_string(), "every increment landed");
        assert_eq!(view.revision, total + 1);
        // One revision per write: the history is a single line.
        assert!(
            view.history
                .iter()
                .enumerate()
                .all(|(i, e)| e.revision == u64::try_from(i).unwrap() + 1)
        );
        // Every refusal was audited.
        let refused = outcomes(&host, crate::work_surfaces::EVENT_DOCUMENT_EDIT)
            .iter()
            .filter(|o| *o == "refused:stale_decision")
            .count();
        assert_eq!(refused, usize::try_from(rejections).unwrap());
    }

    #[test]
    fn concurrent_appends_each_land_exactly_once() {
        let r = registry();
        let host = host();
        std::thread::scope(|scope| {
            let workers: Vec<_> = (1..=EDITORS)
                .map(|byte| {
                    let (r, host) = (&r, &host);
                    scope.spawn(move || {
                        host.edit_with_retry(r, &editor(byte), "log", 10_000, T0, |_, _| {
                            Some(EditOp::Append {
                                text: format!("editor {byte}\n"),
                            })
                        })
                    })
                })
                .collect();
            for worker in workers {
                assert!(matches!(
                    worker.join().unwrap(),
                    Ok(RetryOutcome::Edited { .. })
                ));
            }
        });
        let view = host.read(|s| s.document(&r, &editor(1), "log", T0).unwrap());
        let mut lines: Vec<&str> = view.content.lines().collect();
        lines.sort_unstable();
        let expected: Vec<String> = (1..=EDITORS).map(|b| format!("editor {b}")).collect();
        assert_eq!(lines, expected);
    }

    #[test]
    fn reconciliation_sees_what_was_missed_and_may_give_up() {
        let r = registry();
        let host = host();
        let mut calls = Vec::new();
        // While editor 2 decides, editor 1 lands the same change.
        let outcome = host
            .edit_with_retry(&r, &editor(2), "plan", 5, T0, |view, refused| {
                calls.push((view.revision, refused.map(|s| s.missed.len())));
                if view.content == "ship friday" {
                    return None;
                }
                if refused.is_none() {
                    host.write(&editor(1), &plan(), T0, |s| {
                        s.edit_document(&r, &editor(1), "plan", replace("ship friday"), 0, T0)
                    })
                    .unwrap();
                }
                Some(replace("ship friday"))
            })
            .unwrap();
        assert_eq!(outcome, RetryOutcome::Abandoned { rejections: 1 });
        assert_eq!(calls, [(0, None), (1, Some(1))]);
        let view = host.read(|s| s.document(&r, &editor(1), "plan", T0).unwrap());
        assert_eq!(view.history.len(), 1, "nothing duplicated");
    }

    #[test]
    fn retries_are_bounded() {
        let r = registry();
        let host = host();
        let mut n = 0;
        // Someone always lands an edit first.
        let result = host.edit_with_retry(&r, &editor(2), "plan", 3, T0, |view, _| {
            n += 1;
            host.write(&editor(1), &plan(), T0, |s| {
                s.edit_document(
                    &r,
                    &editor(1),
                    "plan",
                    replace(&format!("v{n}")),
                    view.revision,
                    T0,
                )
            })
            .unwrap();
            Some(replace("mine"))
        });
        let Err(SurfaceError::StaleDecision(stale)) = result else {
            panic!("expected the last stale decision, got {result:?}");
        };
        assert_eq!(n, 3);
        assert_eq!(stale.current, 3);
    }

    #[test]
    fn a_host_that_cannot_serialize_refuses_rather_than_merging() {
        let r = registry();
        let host = host();
        host.write(&editor(1), &plan(), T0, |s| {
            s.edit_document(&r, &editor(1), "plan", replace("before"), 0, T0)
        })
        .unwrap();
        // The writer fails mid-write.
        let crashed = std::thread::scope(|scope| {
            scope
                .spawn(|| {
                    host.write(&editor(1), &plan(), T0, |_| -> Result<(), SurfaceError> {
                        panic!("writer failed mid-write")
                    })
                })
                .join()
        });
        assert!(crashed.is_err());
        assert!(!host.serializes());

        let refused = host.write(&editor(2), &plan(), T0 + 1, |s| {
            s.edit_document(&r, &editor(2), "plan", replace("after"), 1, T0 + 1)
        });
        assert_eq!(refused, Err(SurfaceError::HostCannotSerialize));
        assert_eq!(
            host.edit_with_retry(&r, &editor(2), "plan", 3, T0 + 1, |_, _| Some(replace(
                "after"
            ))),
            Err(SurfaceError::HostCannotSerialize)
        );
        // Audited, and nothing changed; reads still work.
        assert_eq!(
            outcomes(&host, EVENT_HOST_REFUSED),
            ["refused:host_cannot_serialize"; 2]
        );
        let view = host.read(|s| s.document(&r, &editor(2), "plan", T0).unwrap());
        assert_eq!((view.content.as_str(), view.revision), ("before", 1));

        // A restart over the same state serializes again.
        let host = SurfaceHost::new(host.into_surfaces());
        assert!(host.serializes());
        let edited = host.write(&editor(2), &plan(), T0 + 2, |s| {
            s.edit_document(&r, &editor(2), "plan", replace("after"), 1, T0 + 2)
        });
        assert_eq!(edited, Ok(2));
    }
}
