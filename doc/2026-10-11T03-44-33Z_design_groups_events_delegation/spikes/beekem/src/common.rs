//! Shared helpers: a simulated member, wire size, timing, and a blocking runner for the async API.

use beekem::{id::MemberId, keys::ShareKeyMap, operation::CgkaOperation};
use keyhive_crypto::{
    share_key::{ShareKey, ShareSecretKey},
    signed::Signed,
    signer::memory::MemorySigner,
    verifiable::Verifiable,
};
use rand::rngs::StdRng;
use std::{future::Future, time::Duration};

/// A simulated member: a signing key, and the share key pair at its leaf.
pub struct Member {
    pub id: MemberId,
    pub signer: MemorySigner,
    pub pk: ShareKey,
    pub sk: ShareSecretKey,
}

impl Member {
    pub fn new(rng: &mut StdRng) -> Self {
        let signer = MemorySigner::generate(rng);
        let id = MemberId(signer.verifying_key());
        let sk = ShareSecretKey::generate(rng);
        let pk = sk.share_key();
        Self { id, signer, pk, sk }
    }

    /// The owner map a replica starts with: this member's leaf key pair.
    pub fn keys(&self) -> ShareKeyMap {
        let mut keys = ShareKeyMap::new();
        keys.insert(self.pk, self.sk);
        keys
    }
}

/// Wire size of a signed op: bincode, the encoding the crate itself hashes and signs.
pub fn wire_len(op: &Signed<CgkaOperation>) -> usize {
    bincode::serialized_size(op).expect("ops serialize") as usize
}

pub fn block<T>(future: impl Future<Output = T>) -> T {
    futures::executor::block_on(future)
}

pub fn micros(d: Duration) -> f64 {
    d.as_secs_f64() * 1e6
}

pub fn median(values: &mut [f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(|a, b| a.total_cmp(b));
    values[values.len() / 2]
}

pub fn mean(values: &[usize]) -> f64 {
    values.iter().sum::<usize>() as f64 / values.len().max(1) as f64
}

/// Whether `needle` occurs in `haystack`. Used to check what a serialized replica leaks.
pub fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}
