//! Range-based set reconciliation over (author, seq, hash) items. Simulation only, no network.
//!
//! - Items are sorted by key = author (32 bytes) || seq (4 bytes, big-endian).
//! - A range is `[lower, upper)` where bounds are key prefixes. The shortest prefix that
//!   separates two neighbouring keys is used, so a bound between two authors costs one byte.
//! - Fingerprint = XOR of the 16-byte truncations of the event IDs (BLAKE3 of the header
//!   bytes), plus the count.
//! - A receiver with fewer than `thresh` items in a mismatched range sends its IDs.
//!   Otherwise it splits the range into `k` children and sends their fingerprints.
//! - Consecutive skip ranges are merged into one entry.
//! - Both sides learn the symmetric difference: the IDs reply makes the sender answer
//!   with its own IDs.

use std::collections::HashSet;

pub const KEY_LEN: usize = 36;
pub type Key = [u8; KEY_LEN];
pub type Id = [u8; 32];

pub fn key_of(author: &[u8; 32], seq: u32) -> Key {
    let mut key = [0u8; KEY_LEN];
    key[..32].copy_from_slice(author);
    key[32..].copy_from_slice(&seq.to_be_bytes());
    key
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Mode {
    Skip,
    Fp { fp: [u8; 16], count: u32 },
    Ids { ids: Vec<Id>, reply: bool },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// Exclusive upper bound as a key prefix. `None` is +infinity.
    pub upper: Option<Vec<u8>>,
    pub mode: Mode,
}

/// One side's sorted items with prefix XORs for O(1) fingerprints.
pub struct Party {
    keys: Vec<Key>,
    ids: Vec<Id>,
    xor: Vec<Id>,
}

impl Party {
    pub fn new(mut items: Vec<(Key, Id)>) -> Self {
        items.sort_by(|a, b| a.0.cmp(&b.0));
        let keys: Vec<Key> = items.iter().map(|i| i.0).collect();
        let ids: Vec<Id> = items.iter().map(|i| i.1).collect();
        let mut xor = Vec::with_capacity(ids.len() + 1);
        let mut acc = [0u8; 32];
        xor.push(acc);
        for id in &ids {
            for (a, b) in acc.iter_mut().zip(id) {
                *a ^= b;
            }
            xor.push(acc);
        }
        Party { keys, ids, xor }
    }

    pub fn len(&self) -> usize {
        self.keys.len()
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// Index of the first key that is not below `bound`, in prefix order.
    fn idx(&self, bound: &[u8]) -> usize {
        self.keys.partition_point(|k| &k[..bound.len()] < bound)
    }

    fn fp(&self, lo: usize, hi: usize) -> ([u8; 16], u32) {
        let mut out = [0u8; 16];
        for (o, (a, b)) in out.iter_mut().zip(self.xor[hi].iter().zip(&self.xor[lo])) {
            *o = a ^ b;
        }
        (out, (hi - lo) as u32)
    }

    /// Shortest prefix of keys[p] that sorts above keys[p - 1].
    fn separator(&self, p: usize) -> Vec<u8> {
        let (a, b) = (&self.keys[p - 1], &self.keys[p]);
        let d = (0..KEY_LEN)
            .find(|&i| a[i] != b[i])
            .expect("keys are distinct");
        b[..=d].to_vec()
    }

    /// The whole key space as one fingerprint range.
    pub fn full_range(&self) -> Entry {
        let (fp, count) = self.fp(0, self.len());
        Entry {
            upper: None,
            mode: Mode::Fp { fp, count },
        }
    }

    /// Answers each entry of a message from the other side.
    pub fn respond(
        &self,
        msg: &[Entry],
        thresh: usize,
        k: usize,
        learned: &mut HashSet<Id>,
    ) -> Vec<Entry> {
        let mut out: Vec<Entry> = Vec::new();
        let mut lower: Option<&[u8]> = None;
        for entry in msg {
            let upper = entry.upper.as_deref();
            let lo = lower.map_or(0, |b| self.idx(b));
            let hi = upper.map_or(self.len(), |b| self.idx(b));
            match &entry.mode {
                Mode::Skip => push(&mut out, entry.upper.clone(), Mode::Skip),
                Mode::Fp { fp, count } => {
                    let (mine, n) = self.fp(lo, hi);
                    if mine == *fp && n == *count {
                        push(&mut out, entry.upper.clone(), Mode::Skip);
                    } else if (n as usize) <= thresh {
                        let ids = self.ids[lo..hi].to_vec();
                        push(
                            &mut out,
                            entry.upper.clone(),
                            Mode::Ids { ids, reply: true },
                        );
                    } else {
                        let pos: Vec<usize> = (0..=k).map(|j| lo + j * (hi - lo) / k).collect();
                        for j in 0..k {
                            let child_upper = if j + 1 < k {
                                Some(self.separator(pos[j + 1]))
                            } else {
                                entry.upper.clone()
                            };
                            let (cfp, ccount) = self.fp(pos[j], pos[j + 1]);
                            out.push(Entry {
                                upper: child_upper,
                                mode: Mode::Fp {
                                    fp: cfp,
                                    count: ccount,
                                },
                            });
                        }
                    }
                }
                Mode::Ids { ids, reply } => {
                    let mine: HashSet<Id> = self.ids[lo..hi].iter().copied().collect();
                    let theirs: HashSet<Id> = ids.iter().copied().collect();
                    learned.extend(mine.symmetric_difference(&theirs).copied());
                    if *reply {
                        let ids = self.ids[lo..hi].to_vec();
                        push(
                            &mut out,
                            entry.upper.clone(),
                            Mode::Ids { ids, reply: false },
                        );
                    } else {
                        push(&mut out, entry.upper.clone(), Mode::Skip);
                    }
                }
            }
            lower = upper;
        }
        out
    }
}

/// Appends an entry, merging it into the previous one when both are skips.
fn push(out: &mut Vec<Entry>, upper: Option<Vec<u8>>, mode: Mode) {
    if mode == Mode::Skip {
        if let Some(last) = out.last_mut() {
            if last.mode == Mode::Skip {
                last.upper = upper;
                return;
            }
        }
    }
    out.push(Entry { upper, mode });
}

fn varint_len(mut x: usize) -> usize {
    let mut n = 1;
    while x >= 0x80 {
        x >>= 7;
        n += 1;
    }
    n
}

/// Bytes of a message on the wire: count, then per entry the bound (length byte + prefix),
/// the mode byte, and the payload (fingerprint + count, or a count + IDs).
pub fn encoded_len(msg: &[Entry]) -> usize {
    let mut n = varint_len(msg.len());
    for e in msg {
        n += match &e.upper {
            None => 1,
            Some(b) => 1 + b.len(),
        };
        n += 1;
        n += match &e.mode {
            Mode::Skip => 0,
            Mode::Fp { count, .. } => 16 + varint_len(*count as usize),
            Mode::Ids { ids, .. } => varint_len(ids.len()) + 32 * ids.len(),
        };
    }
    n
}

#[derive(Debug, Clone)]
pub struct Outcome {
    /// Sequential one-way messages, including the final all-skip message.
    pub flights: u32,
    pub bytes_a: usize,
    pub bytes_b: usize,
    /// Size of the symmetric difference. Each item is one event payload to move.
    pub diff: usize,
    /// Both sides learned exactly the symmetric difference.
    pub correct: bool,
}

/// Runs the protocol with `a` as the initiator. Returns sizes and round counts.
pub fn reconcile(a: &Party, b: &Party, thresh: usize, k: usize) -> Outcome {
    let a_set: HashSet<Id> = a.ids.iter().copied().collect();
    let b_set: HashSet<Id> = b.ids.iter().copied().collect();
    let truth: HashSet<Id> = a_set.symmetric_difference(&b_set).copied().collect();

    let mut learned_a = HashSet::new();
    let mut learned_b = HashSet::new();
    let mut msg = vec![a.full_range()];
    let mut a_sends = true;
    let (mut bytes_a, mut bytes_b, mut flights) = (0usize, 0usize, 0u32);
    loop {
        flights += 1;
        let n = encoded_len(&msg);
        if a_sends {
            bytes_a += n;
        } else {
            bytes_b += n;
        }
        if msg.iter().all(|e| e.mode == Mode::Skip) {
            break;
        }
        msg = if a_sends {
            b.respond(&msg, thresh, k, &mut learned_b)
        } else {
            a.respond(&msg, thresh, k, &mut learned_a)
        };
        a_sends = !a_sends;
        assert!(flights < 10_000, "reconciliation did not terminate");
    }

    let correct = learned_a == truth && learned_b == truth;
    Outcome {
        flights,
        bytes_a,
        bytes_b,
        diff: truth.len(),
        correct,
    }
}
