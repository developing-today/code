//! S1's range reconciliation (`/home/user/work/spikes/s1-transport/src/range_sim.rs`), run over
//! a store's sorted (author, seq) keys, with a wire encoding.
//!
//! A range is `[lower, upper)`. Bounds are the shortest key prefixes that separate neighbouring
//! keys. A fingerprint is the XOR of the 16-byte truncations of the event IDs, plus the count.
//! A mismatched range with at most `T` items sends its IDs. A larger one splits into `K`
//! children.
//!
//! Differences from S1: keys are 40 bytes (S6's layout). A split uses `min(K, n)` children.
//! Read from S1's code, not run: when a mismatched range starts at key 0 and has fewer than K
//! items, S1 computes a bound at position 0 and panics. S1's runs did not reach that case.

use crate::store::{Id, Key, KEY_LEN};
use anyhow::{Result, bail, ensure};
use std::collections::HashSet;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Mode {
    Skip,
    Fp { fp: [u8; 16], count: u32 },
    /// `reply` asks the receiver to answer with its own IDs for the same range.
    Ids { ids: Vec<Id>, reply: bool },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// Exclusive upper bound as a key prefix. `None` is +infinity.
    pub upper: Option<Vec<u8>>,
    pub mode: Mode,
}

pub fn all_skip(msg: &[Entry]) -> bool {
    msg.iter().all(|e| e.mode == Mode::Skip)
}

/// One side's sorted keys and IDs, with prefix XORs for O(1) fingerprints.
pub struct Party {
    keys: Vec<Key>,
    ids: Vec<Id>,
    xor: Vec<[u8; 16]>,
}

impl Party {
    /// Keys must be sorted and distinct, as a store's index is.
    pub fn new(keys: Vec<Key>, ids: Vec<Id>) -> Self {
        assert_eq!(keys.len(), ids.len());
        assert!(keys.windows(2).all(|w| w[0] < w[1]), "keys must be sorted and distinct");
        let mut xor = Vec::with_capacity(ids.len() + 1);
        let mut acc = [0u8; 16];
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

    pub fn ids(&self) -> &[Id] {
        &self.ids
    }

    /// Index of the first key whose prefix is not below `bound`.
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

    /// Shortest prefix of keys[p] that sorts above keys[p - 1]. Requires 0 < p < len.
    fn separator(&self, p: usize) -> Vec<u8> {
        let (a, b) = (&self.keys[p - 1], &self.keys[p]);
        let d = (0..KEY_LEN).find(|&i| a[i] != b[i]).expect("keys are distinct");
        b[..=d].to_vec()
    }

    /// The whole key space as one fingerprint range.
    pub fn full_range(&self) -> Entry {
        let (fp, count) = self.fp(0, self.len());
        Entry { upper: None, mode: Mode::Fp { fp, count } }
    }

    /// Answers each entry of a message from the other side. IDs in the symmetric difference
    /// go into `learned`.
    pub fn respond(&self, msg: &[Entry], thresh: usize, k: usize, learned: &mut HashSet<Id>) -> Vec<Entry> {
        assert!(k >= 2, "K must be at least 2, or a split makes no progress");
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
                        push(&mut out, entry.upper.clone(), Mode::Ids { ids, reply: true });
                    } else {
                        // n > thresh >= 1, so parts >= 2 and every child holds at least one key.
                        let parts = k.min(n as usize);
                        let pos: Vec<usize> = (0..=parts).map(|j| lo + j * (hi - lo) / parts).collect();
                        for j in 0..parts {
                            let child_upper = if j + 1 < parts {
                                Some(self.separator(pos[j + 1]))
                            } else {
                                entry.upper.clone()
                            };
                            let (cfp, ccount) = self.fp(pos[j], pos[j + 1]);
                            out.push(Entry {
                                upper: child_upper,
                                mode: Mode::Fp { fp: cfp, count: ccount },
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
                        push(&mut out, entry.upper.clone(), Mode::Ids { ids, reply: false });
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

fn put_varint(out: &mut Vec<u8>, mut x: usize) {
    while x >= 0x80 {
        out.push((x as u8 & 0x7f) | 0x80);
        x >>= 7;
    }
    out.push(x as u8);
}

/// Wire form of one message, without the frame length: varint entry count; per entry, a bound
/// (one length byte, 0 for +infinity, then the prefix), one mode byte (0 skip, 1 fingerprint,
/// 2 IDs, 3 IDs with reply), and the payload (fingerprint + varint count, or varint count + IDs).
pub fn encode(msg: &[Entry]) -> Vec<u8> {
    let mut out = Vec::new();
    put_varint(&mut out, msg.len());
    for e in msg {
        match &e.upper {
            None => out.push(0),
            Some(b) => {
                out.push(b.len() as u8);
                out.extend_from_slice(b);
            }
        }
        match &e.mode {
            Mode::Skip => out.push(0),
            Mode::Fp { fp, count } => {
                out.push(1);
                out.extend_from_slice(fp);
                put_varint(&mut out, *count as usize);
            }
            Mode::Ids { ids, reply } => {
                out.push(if *reply { 3 } else { 2 });
                put_varint(&mut out, ids.len());
                for id in ids {
                    out.extend_from_slice(id);
                }
            }
        }
    }
    out
}

struct Reader<'a> {
    buf: &'a [u8],
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        ensure!(self.buf.len() >= n, "message truncated");
        let (head, rest) = self.buf.split_at(n);
        self.buf = rest;
        Ok(head)
    }

    fn byte(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    fn varint(&mut self) -> Result<usize> {
        let mut x = 0usize;
        for shift in (0..).step_by(7).take(5) {
            let b = self.byte()?;
            x |= ((b & 0x7f) as usize) << shift;
            if b < 0x80 {
                return Ok(x);
            }
        }
        bail!("varint longer than 5 bytes")
    }
}

/// Parses a message written by [`encode`]. Rejects truncated and trailing bytes.
pub fn decode(bytes: &[u8]) -> Result<Vec<Entry>> {
    let mut r = Reader { buf: bytes };
    let n = r.varint()?;
    let mut out = Vec::new();
    for _ in 0..n {
        let len = r.byte()? as usize;
        let upper = if len == 0 { None } else { Some(r.take(len)?.to_vec()) };
        let mode = match r.byte()? {
            0 => Mode::Skip,
            1 => {
                let fp: [u8; 16] = r.take(16)?.try_into()?;
                let count = u32::try_from(r.varint()?)?;
                Mode::Fp { fp, count }
            }
            m @ (2 | 3) => {
                let count = r.varint()?;
                let mut ids = Vec::new();
                for _ in 0..count {
                    ids.push(r.take(32)?.try_into()?);
                }
                Mode::Ids { ids, reply: m == 3 }
            }
            m => bail!("unknown mode byte {m}"),
        };
        out.push(Entry { upper, mode });
    }
    ensure!(r.buf.is_empty(), "trailing bytes after message");
    Ok(out)
}

/// Both sides run the protocol in memory, as S1's `reconcile`. Used to check the wire run
/// byte for byte, and to find the symmetric difference.
pub struct Sim {
    pub flights: u32,
    /// Encoded bytes each side wrote, without frame lengths.
    pub bytes_a: usize,
    pub bytes_b: usize,
    pub learned_a: HashSet<Id>,
    pub learned_b: HashSet<Id>,
}

pub fn simulate(a: &Party, b: &Party, thresh: usize, k: usize) -> Sim {
    let mut learned_a = HashSet::new();
    let mut learned_b = HashSet::new();
    let mut msg = vec![a.full_range()];
    let mut a_sends = true;
    let (mut bytes_a, mut bytes_b, mut flights) = (0usize, 0usize, 0u32);
    loop {
        flights += 1;
        let n = encode(&msg).len();
        if a_sends {
            bytes_a += n;
        } else {
            bytes_b += n;
        }
        if all_skip(&msg) {
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
    Sim { flights, bytes_a, bytes_b, learned_a, learned_b }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(seed: u64, n: usize) -> Vec<u8> {
        let mut h = blake3::Hasher::new();
        h.update(&seed.to_be_bytes());
        let mut out = vec![0u8; n];
        h.finalize_xof().fill(&mut out);
        out
    }

    /// Keys built only from 0x00 and 0x01 bytes, so bounds end in zeros often.
    fn zero_heavy(n: usize, seed: u64) -> Vec<Key> {
        let mut keys: Vec<Key> = (0..n as u64)
            .map(|i| {
                let r = bytes(seed ^ (i << 8), KEY_LEN);
                let mut k = [0u8; KEY_LEN];
                for (dst, src) in k.iter_mut().zip(r) {
                    *dst = src & 1;
                }
                k
            })
            .collect();
        keys.sort();
        keys.dedup();
        keys
    }

    /// IDs derived from the whole key, so two keys never share an ID.
    fn party(keys: &[Key], seed: u64) -> Party {
        let ids = keys
            .iter()
            .map(|k| *blake3::hash(&[k.as_slice(), &seed.to_be_bytes()].concat()).as_bytes())
            .collect();
        Party::new(keys.to_vec(), ids)
    }

    #[test]
    fn separators_split_keys_with_nul_bytes() {
        let keys = zero_heavy(300, 7);
        let p = party(&keys, 1);
        for pos in 1..p.len() {
            let s = p.separator(pos);
            assert_eq!(p.idx(&s), pos, "separator {} at {pos} splits the keys wrongly", hex::encode(&s));
        }
    }

    #[test]
    fn respond_learns_exact_difference() {
        for (keys_seed, k, t) in [(11u64, 2, 4), (12, 16, 16), (13, 16, 4), (14, 32, 16), (15, 32, 1)] {
            let all = zero_heavy(700, keys_seed);
            let a_keys: Vec<Key> = all.iter().copied().filter(|k| k[0] == 0 || k[1] == 0).collect();
            let b_keys: Vec<Key> = all.iter().copied().filter(|k| k[2] == 0 || k[31] == 1).collect();
            let a = party(&a_keys, 5);
            let b = party(&b_keys, 5);
            let sim = simulate(&a, &b, t, k);
            let a_set: HashSet<Id> = a.ids().iter().copied().collect();
            let b_set: HashSet<Id> = b.ids().iter().copied().collect();
            let truth: HashSet<Id> = a_set.symmetric_difference(&b_set).copied().collect();
            assert!(!truth.is_empty());
            assert_eq!(sim.learned_a, truth, "K={k} T={t}");
            assert_eq!(sim.learned_b, truth, "K={k} T={t}");
        }
    }

    #[test]
    fn wire_round_trip() {
        let keys = zero_heavy(200, 3);
        let p = party(&keys, 2);
        let mut learned = HashSet::new();
        let msg = p.respond(&[p.full_range()], 4, 16, &mut learned);
        let bytes = encode(&msg);
        assert_eq!(decode(&bytes).unwrap(), msg);
        assert!(decode(&bytes[..bytes.len() - 1]).is_err());
        let mut extra = bytes.clone();
        extra.push(0);
        assert!(decode(&extra).is_err());
    }
}
