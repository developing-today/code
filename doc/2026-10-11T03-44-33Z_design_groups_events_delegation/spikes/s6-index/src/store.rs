//! The operations both stores implement. The benchmark, durability, concurrency and
//! uniqueness runs drive each store through this trait, so the calls are identical.

use crate::keys::{Claim, Id};
use anyhow::Result;
use std::path::Path;

/// What one batch did. Writing an identical claim again counts as a duplicate.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Batch {
    pub inserted: usize,
    pub duplicates: usize,
}

/// Rows per table, in order: claims, alpha, omega, author_seq.
pub type Counts = [u64; 4];

pub trait Store: Sized {
    const NAME: &'static str;

    /// Opens the store at `path`, creating it if it does not exist.
    fn open(path: &Path) -> Result<Self>;

    /// Writes `claims` in one transaction. A claim that forks an occupied
    /// (author, seq) position fails the whole batch, and nothing from it is kept.
    fn insert_batch(&mut self, claims: &[Claim]) -> Result<Batch>;

    /// Every claim whose subject is `subject`.
    fn by_subject(&self, subject: &Id) -> Result<Vec<Claim>>;

    /// The subject of every claim that carries `name` = `value`.
    fn subjects_with(&self, name: &[u8], value: &[u8]) -> Result<Vec<Id>>;

    /// Every claim by `author`, in seq order.
    fn by_author(&self, author: &Id) -> Result<Vec<Claim>>;

    fn counts(&self) -> Result<Counts>;

    fn all_claims(&self) -> Result<Vec<Claim>>;
}
