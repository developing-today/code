//! Runs the workload with one power-cut plan, reopens the disk image, and classifies the result.
//!
//! Acknowledged = a commit that returned `Ok` before the cut. A commit in flight at the cut is not
//! acknowledged. The reopened state must be the model after `acked` transactions, or after
//! `acked + 1` (the in-flight one may have become durable). Anything else is a failure.

use crate::faultfs::{FaultFs, Plan, Trace};
use crate::workload::{self, Model, TXNS};
use redb::Database;
use std::any::Any;
use std::panic::{self, AssertUnwindSafe};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Reopened state is the model after every acknowledged commit.
    OkAcked,
    /// Reopened state includes the in-flight commit as well.
    OkAckedPlusOne,
    /// The image does not open, but no commit had been acknowledged yet. Not data loss; reported.
    OpenErrorPreCommit,
    /// Reopened state is an older prefix than the acknowledged one.
    LostCommitted,
    /// Reopened state matches no prefix of the workload (for example, half of a batch).
    Partial,
    /// Reopened state is a later prefix than the in-flight commit. Cannot happen if the cut works.
    Future,
    /// The image did not open, or a read failed, after acknowledged commits.
    OpenError,
    /// `check_integrity` returned false or failed.
    Integrity,
    /// A commit after recovery failed, or the state did not survive a clean reopen.
    PostRecovery,
    /// The workload returned an error although no cut happened.
    NoCrashError,
    /// The workload, or the reopen of the image, panicked.
    Panic,
}

impl Verdict {
    pub const fn name(self) -> &'static str {
        match self {
            Self::OkAcked => "ok_acked",
            Self::OkAckedPlusOne => "ok_acked_plus_one",
            Self::OpenErrorPreCommit => "open_error_pre_commit",
            Self::LostCommitted => "lost_committed",
            Self::Partial => "partial",
            Self::Future => "future",
            Self::OpenError => "open_error",
            Self::Integrity => "integrity",
            Self::PostRecovery => "post_recovery",
            Self::NoCrashError => "no_crash_error",
            Self::Panic => "panic",
        }
    }

    /// The verdict with this name, if there is one. The corpus stores verdicts by name.
    pub fn from_name(name: &str) -> Option<Self> {
        [
            Self::OkAcked,
            Self::OkAckedPlusOne,
            Self::OpenErrorPreCommit,
            Self::LostCommitted,
            Self::Partial,
            Self::Future,
            Self::OpenError,
            Self::Integrity,
            Self::PostRecovery,
            Self::NoCrashError,
            Self::Panic,
        ]
        .into_iter()
        .find(|v| v.name() == name)
    }

    /// Whether this verdict is a durability or consistency failure. Only `OkAcked`,
    /// `OkAckedPlusOne` and `OpenErrorPreCommit` are not.
    pub const fn is_failure(self) -> bool {
        !matches!(
            self,
            Self::OkAcked | Self::OkAckedPlusOne | Self::OpenErrorPreCommit
        )
    }
}

#[derive(Debug)]
pub struct Outcome {
    pub verdict: Verdict,
    pub acked: u64,
    /// Repair callbacks redb ran while reopening the image.
    pub repairs: u64,
    pub detail: String,
}

fn run_workload(fs: &FaultFs, acked: &mut u64) -> Result<(), String> {
    let db = Database::builder()
        .create_with_backend(fs.clone())
        .map_err(|e| format!("create: {e}"))?;
    for b in 0..TXNS {
        let result = workload::commit(&db, &workload::txn_ops(b));
        if fs.cut() {
            // The process died inside this call. The commit is not acknowledged.
            return Ok(());
        }
        result.map_err(|e| format!("commit {b}: {e}"))?;
        *acked = b + 1;
    }
    drop(db); // a clean close; its writes are storage calls too
    Ok(())
}

/// Commits once more on the recovered database, then reopens the result and checks that the
/// recovered state and the new commit both survive.
fn post_recovery(db: Database, backing: &FaultFs, expected: &Model) -> Result<(), String> {
    let txn = db.begin_write().map_err(|e| format!("begin: {e}"))?;
    {
        let mut meta = txn
            .open_table(workload::META)
            .map_err(|e| format!("open meta: {e}"))?;
        meta.insert(b"post-recovery".as_slice(), b"ok".as_slice())
            .map_err(|e| format!("insert: {e}"))?;
    }
    txn.commit().map_err(|e| format!("commit: {e}"))?;
    drop(db);
    let again = Database::builder()
        .create_with_backend(FaultFs::new(backing.disk_image(), None, false))
        .map_err(|e| format!("reopen: {e}"))?;
    let mut want = expected.clone();
    want.insert(
        (workload::META_TAG, b"post-recovery".to_vec()),
        b"ok".to_vec(),
    );
    let got = workload::dump(&again).map_err(|e| format!("dump: {e}"))?;
    if got == want {
        Ok(())
    } else {
        Err("state after the post-recovery commit and clean reopen differs".into())
    }
}

fn verify(image: Vec<u8>, acked: u64, models: &[Model]) -> Outcome {
    let reopened = FaultFs::new(image, None, false);
    let repairs = Arc::new(AtomicU64::new(0));
    let counter = Arc::clone(&repairs);
    let mut builder = Database::builder();
    builder.set_repair_callback(move |_| {
        counter.fetch_add(1, Ordering::SeqCst);
    });
    let outcome = |verdict: Verdict, detail: String| Outcome {
        verdict,
        acked,
        repairs: repairs.load(Ordering::SeqCst),
        detail,
    };
    let mut db = match builder.create_with_backend(reopened.clone()) {
        Ok(db) => db,
        Err(e) => {
            let verdict = if acked == 0 {
                Verdict::OpenErrorPreCommit
            } else {
                Verdict::OpenError
            };
            return outcome(verdict, format!("open: {e}"));
        }
    };
    let state = match workload::dump(&db) {
        Ok(state) => state,
        Err(e) => return outcome(Verdict::OpenError, format!("dump: {e}")),
    };
    let n = acked as usize;
    let verdict = if state == models[n] {
        Verdict::OkAcked
    } else if n + 1 < models.len() && state == models[n + 1] {
        Verdict::OkAckedPlusOne
    } else if models[..n].contains(&state) {
        Verdict::LostCommitted
    } else if models.iter().skip(n + 2).any(|m| *m == state) {
        Verdict::Future
    } else {
        Verdict::Partial
    };
    if verdict.is_failure() {
        return outcome(verdict, String::new());
    }
    match db.check_integrity() {
        Ok(true) => {}
        Ok(false) => return outcome(Verdict::Integrity, "check_integrity returned false".into()),
        Err(e) => return outcome(Verdict::Integrity, format!("check_integrity: {e}")),
    }
    if let Err(e) = post_recovery(db, &reopened, &state) {
        return outcome(Verdict::PostRecovery, e);
    }
    outcome(verdict, String::new())
}

fn panic_text(payload: &(dyn Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|s| (*s).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "non-string panic payload".into())
}

/// Runs one plan (or no plan, for the clean run) and returns the outcome and the storage-call
/// trace. Panics never escape: a panic before the cut, or while reopening, is a verdict.
pub fn run_one(plan: Option<Plan>, lying_sync: bool, models: &[Model]) -> (Outcome, Trace) {
    let fs = FaultFs::new(Vec::new(), plan, lying_sync);
    let mut acked = 0u64;
    let attempt = panic::catch_unwind(AssertUnwindSafe(|| run_workload(&fs, &mut acked)));
    let trace = fs.trace();
    let outcome = match attempt {
        Err(payload) if !fs.cut() => Outcome {
            verdict: Verdict::Panic,
            acked,
            repairs: 0,
            detail: format!(
                "panic before the power cut: {}",
                panic_text(payload.as_ref())
            ),
        },
        Ok(Err(e)) if !fs.cut() => Outcome {
            verdict: Verdict::NoCrashError,
            acked,
            repairs: 0,
            detail: e,
        },
        // A cut (or a clean finish) leaves a disk image to reopen. A panic or error after the cut
        // is the dead process failing, not a finding.
        _ => match panic::catch_unwind(AssertUnwindSafe(|| verify(fs.disk_image(), acked, models)))
        {
            Ok(outcome) => outcome,
            Err(payload) => Outcome {
                verdict: Verdict::Panic,
                acked,
                repairs: 0,
                detail: format!(
                    "panic while reopening the image: {}",
                    panic_text(payload.as_ref())
                ),
            },
        },
    };
    (outcome, trace)
}
