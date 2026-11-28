//! Wire sizes of p2panda-sync's LogSync messages, built from p2panda-core's `LogHeights`.

use std::collections::BTreeMap;

use p2panda_core::logs::{LogHeights, SeqNum};
use p2panda_core::{SigningKey, VerifyingKey};
use p2panda_sync::protocols::LogSyncMessage;

/// One log per author. The log ID is `u64`, the smallest LogId that p2panda-sync accepts.
pub type Log = u64;

/// Deterministic author key for chain `i`.
pub fn author_key(i: usize) -> VerifyingKey {
    let seed = blake3::hash(&(i as u64).to_be_bytes());
    SigningKey::from_bytes(seed.as_bytes()).verifying_key()
}

/// `LogHeights` for `n` chains. Chain `i` has its last 0-based seq at `height`, and chains
/// listed in `extra` have one more event.
pub fn heights(
    keys: &[VerifyingKey],
    height: SeqNum,
    extra: &[usize],
) -> LogHeights<VerifyingKey, Log> {
    let mut out = BTreeMap::new();
    for (i, key) in keys.iter().enumerate() {
        let h = if extra.contains(&i) {
            height + 1
        } else {
            height
        };
        out.insert(*key, BTreeMap::from([(0 as Log, h)]));
    }
    out
}

/// Serialized sizes of one LogSync message under two encodings.
#[derive(Debug, Clone, Copy)]
pub struct Sizes {
    pub cbor: usize,
    pub postcard: usize,
}

fn sizes(msg: &LogSyncMessage<Log>) -> Result<Sizes, String> {
    let cbor = cbor_core::Value::serialized(msg)
        .map_err(|e| format!("cbor serialize: {e:?}"))?
        .encode()
        .len();
    let postcard = postcard::to_stdvec(msg)
        .map_err(|e| format!("postcard serialize: {e}"))?
        .len();
    Ok(Sizes { cbor, postcard })
}

/// Size of a `Have` message carrying `heights`.
pub fn have(heights: &LogHeights<VerifyingKey, Log>) -> Result<Sizes, String> {
    sizes(&LogSyncMessage::Have(heights.clone()))
}

/// Sizes of the control messages LogSync sends around the Have exchange.
pub fn control(
    heights: &LogHeights<VerifyingKey, Log>,
) -> Result<[(&'static str, Sizes); 3], String> {
    Ok([
        ("Have", have(heights)?),
        (
            "PreSync",
            sizes(&LogSyncMessage::PreSync {
                total_operations: 2,
                total_bytes: 400,
            })?,
        ),
        ("Done", sizes(&LogSyncMessage::Done)?),
    ])
}
