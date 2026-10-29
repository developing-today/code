//! Range reconciliation: the set-difference protocol that `/id-range-sync/1` runs.
//!
//! The set is the store's sync set (see [`super::sync`]): one key per stored event and per
//! fork proof header, `author || seq || id`, sorted. A range is `[lower, upper)` over those
//! keys, where `lower` is the previous entry's upper, and the first range starts at the
//! beginning of the key space. A message is a list of entries; each entry carries an explicit
//! upper bound (a key prefix, or `None` for +infinity) and a mode:
//!
//! - **Skip**: the receiver has nothing to say about this range.
//! - **Fingerprint**: the XOR of the 16-byte truncations of the range's IDs, and its count.
//! - **IDs**: the sender's IDs for the range. `reply` asks the receiver to answer with its own.
//!
//! The responder answers each entry from its own keys. A matching fingerprint is skipped. A
//! mismatched range with at most `T` keys sends its IDs. A larger one splits into `min(K, n)`
//! children. K and T are local policy and never go on the wire, so two peers with different
//! values still converge: each side applies only its own values when it answers.
//!
//! A flight is one message. It is written as frames, each at most a byte budget, and the
//! receiver reads the frames back as one list. Chunking never changes what a message means.
//!
//! Frame layout: `more` (one byte: 1 if another frame of this flight follows), a varint entry
//! count, then the entries. An entry is the upper bound (one length byte, 0 for +infinity, then
//! the prefix), a mode byte (0 skip, 1 fingerprint, 2 IDs, 3 IDs with reply), and the payload.
//!
//! Changing K or T needs no version bump. The mode bytes, the fingerprint, the key layout, the
//! framing, and the frame cap are wire-visible, and would need `/id-range-sync/2`.

use std::collections::HashSet;

use anyhow::{Result, bail, ensure};

/// Bytes in a sync key: the author (32), the sequence number (8, big-endian), then the ID (32).
pub const KEY_LEN: usize = 72;
/// A sync key.
pub type Key = [u8; KEY_LEN];
/// An event ID: the BLAKE3 hash of the signed header.
pub type Id = [u8; 32];

/// The default fan-out.
pub const DEFAULT_K: usize = 16;
/// The default leaf threshold.
pub const DEFAULT_T: usize = 16;
/// The largest frame the transport accepts, in bytes.
pub const FRAME_CAP: usize = 16 << 20;
/// The default byte budget for one frame: a quarter of the cap, so one frame is never near it.
pub const DEFAULT_FRAME_BUDGET: usize = 4 << 20;
/// The default session budget for reconciliation reads: 64 MiB.
///
/// It holds the 17.6 MB ID reply of a 524,288-event bootstrap with room to spare. It also bounds
/// what a peer can make one session buffer.
pub const DEFAULT_SESSION_BUDGET: usize = 64 << 20;
/// Bytes a decoded entry costs against the session budget, on top of its wire bytes.
///
/// That is the entry's own size, and the heap block that holds its bound (at most [`KEY_LEN`]
/// bytes, with allocator slack). Without it, a peer's two-byte entries would cost about thirty
/// times their wire size.
pub const ENTRY_CHARGE: usize = size_of::<Entry>() + KEY_LEN + 16;
// Two bytes per ID, twice over, for a store at the 524,288-event limit: the bootstrap fits.
const _: () = assert!(DEFAULT_SESSION_BUDGET >= 2 * 32 * 524_288);
/// The smallest budget that fits an entry.
const MIN_FRAME_BUDGET: usize = 256;
/// Bytes reserved in a frame for its `more` flag and its entry count.
const FRAME_HEADER_RESERVE: usize = 6;

/// Why a set of local parameters is refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ParamsError {
    /// K below 2 makes a split that never shrinks the range.
    #[error("K must be at least 2, or a split makes no progress (got {0})")]
    FanOut(usize),
    /// T of 0 never ends on one mismatched key: a single child is the same range again.
    #[error("T must be at least 1, or one mismatched key never ends the sync (got {0})")]
    LeafThreshold(usize),
    /// A budget too small for an entry, or above the frame cap.
    #[error(
        "the frame budget must be from {MIN_FRAME_BUDGET} bytes to the {FRAME_CAP}-byte frame cap (got {0})"
    )]
    FrameBudget(usize),
    /// A session budget too small for one entry.
    #[error("the session budget must be at least {MIN_FRAME_BUDGET} bytes (got {0})")]
    SessionBudget(usize),
}

/// The local parameters of one side: fan-out K, leaf threshold T, the frame budget, and the
/// session budget. None of them is sent to the peer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Params {
    k: usize,
    t: usize,
    frame_budget: usize,
    session_budget: usize,
}

impl Params {
    /// Parameters with fan-out `k` and leaf threshold `t`, and the default frame budget.
    ///
    /// # Errors
    ///
    /// Refuses `k` below 2, and `t` below 1.
    pub fn new(k: usize, t: usize) -> Result<Self, ParamsError> {
        if k < 2 {
            return Err(ParamsError::FanOut(k));
        }
        if t < 1 {
            return Err(ParamsError::LeafThreshold(t));
        }
        Ok(Self {
            k,
            t,
            frame_budget: DEFAULT_FRAME_BUDGET,
            session_budget: DEFAULT_SESSION_BUDGET,
        })
    }

    /// The same parameters with another frame budget.
    ///
    /// # Errors
    ///
    /// Refuses a budget below one entry, or above [`FRAME_CAP`].
    pub fn with_frame_budget(self, bytes: usize) -> Result<Self, ParamsError> {
        if !(MIN_FRAME_BUDGET..=FRAME_CAP).contains(&bytes) {
            return Err(ParamsError::FrameBudget(bytes));
        }
        Ok(Self {
            frame_budget: bytes,
            ..self
        })
    }

    /// The same parameters with another session budget: the most a session may read in
    /// reconciliation before it ends with [`super::sync::BudgetExceeded`].
    ///
    /// # Errors
    ///
    /// Refuses a budget below one entry.
    pub fn with_session_budget(self, bytes: usize) -> Result<Self, ParamsError> {
        if bytes < MIN_FRAME_BUDGET {
            return Err(ParamsError::SessionBudget(bytes));
        }
        Ok(Self {
            session_budget: bytes,
            ..self
        })
    }

    /// The fan-out.
    pub const fn k(&self) -> usize {
        self.k
    }

    /// The leaf threshold.
    pub const fn t(&self) -> usize {
        self.t
    }

    /// The byte budget for one frame.
    pub const fn frame_budget(&self) -> usize {
        self.frame_budget
    }

    /// The byte budget for one session's reconciliation reads.
    pub const fn session_budget(&self) -> usize {
        self.session_budget
    }

    /// The most IDs one entry carries. Two such entries always fit one frame.
    const fn ids_per_entry(&self) -> usize {
        let per = self.frame_budget / 64;
        if per == 0 { 1 } else { per }
    }
}

impl Default for Params {
    fn default() -> Self {
        Self {
            k: DEFAULT_K,
            t: DEFAULT_T,
            frame_budget: DEFAULT_FRAME_BUDGET,
            session_budget: DEFAULT_SESSION_BUDGET,
        }
    }
}

/// What one entry says about its range.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Nothing to say about the range.
    Skip,
    /// The XOR fingerprint and the count of the sender's keys in the range.
    Fp {
        /// XOR of the 16-byte truncations of the IDs.
        fp: [u8; 16],
        /// How many keys the sender holds in the range.
        count: u32,
    },
    /// The sender's IDs for the range. `reply` asks the receiver for its own.
    Ids {
        /// The IDs, in key order.
        ids: Vec<Id>,
        /// Whether the receiver should answer with its own IDs for the range.
        reply: bool,
    },
}

/// One range of a message: its explicit upper bound, and what the sender says about it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// Exclusive upper bound as a key prefix. `None` is +infinity.
    pub upper: Option<Vec<u8>>,
    /// What the sender says about the range.
    pub mode: Mode,
}

/// Whether every entry of `msg` is a skip. A flight of skips ends the session.
pub fn all_skip(msg: &[Entry]) -> bool {
    msg.iter().all(|e| e.mode == Mode::Skip)
}

/// One side's sorted keys and IDs, with prefix XORs for O(1) fingerprints.
#[derive(Debug)]
pub struct Party {
    keys: Vec<Key>,
    ids: Vec<Id>,
    xor: Vec<[u8; 16]>,
}

impl Party {
    /// The set of `entries`, in key order. Keys must be distinct.
    ///
    /// # Errors
    ///
    /// Refuses two entries with one key.
    pub fn new(mut entries: Vec<(Key, Id)>) -> Result<Self> {
        entries.sort_unstable_by_key(|e| e.0);
        ensure!(
            entries.windows(2).all(|w| w[0].0 != w[1].0),
            "sync keys are not distinct"
        );
        let (keys, ids): (Vec<Key>, Vec<Id>) = entries.into_iter().unzip();
        let mut xor = Vec::with_capacity(ids.len() + 1);
        let mut acc = [0u8; 16];
        xor.push(acc);
        for id in &ids {
            for (a, b) in acc.iter_mut().zip(id) {
                *a ^= b;
            }
            xor.push(acc);
        }
        Ok(Self { keys, ids, xor })
    }

    /// The number of keys.
    pub fn len(&self) -> usize {
        self.keys.len()
    }

    /// Whether the set is empty.
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// The IDs, in key order.
    pub fn ids(&self) -> &[Id] {
        &self.ids
    }

    /// Index of the first key whose prefix is not below `bound`.
    fn idx(&self, bound: &[u8]) -> usize {
        self.keys.partition_point(|k| k[..bound.len()] < *bound)
    }

    fn fp(&self, lo: usize, hi: usize) -> ([u8; 16], u32) {
        let mut out = [0u8; 16];
        for (o, (a, b)) in out.iter_mut().zip(self.xor[hi].iter().zip(&self.xor[lo])) {
            *o = a ^ b;
        }
        (out, (hi - lo) as u32)
    }

    /// Shortest prefix of `keys[p]` that sorts above `keys[p - 1]`. Requires `0 < p < len`.
    fn separator(&self, p: usize) -> Vec<u8> {
        let (a, b) = (&self.keys[p - 1], &self.keys[p]);
        // Keys are distinct, so they differ at some byte.
        let d = (0..KEY_LEN).find(|&i| a[i] != b[i]).unwrap_or(KEY_LEN - 1);
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

    /// Answers each entry of a message from the other side, with this side's own `params`.
    /// IDs in the symmetric difference go into `learned`. The message's bounds must already
    /// be checked with [`check_bounds`].
    pub fn respond(&self, msg: &[Entry], params: Params, learned: &mut Vec<Id>) -> Vec<Entry> {
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
                    } else if (n as usize) <= params.t() {
                        self.push_ids(&mut out, lo, hi, entry.upper.clone(), true, params);
                    } else {
                        // n > T >= 1, so parts >= 2 and every child holds at least one key.
                        let parts = params.k().min(n as usize);
                        let pos: Vec<usize> =
                            (0..=parts).map(|j| lo + j * (hi - lo) / parts).collect();
                        for j in 0..parts {
                            let child_upper = if j + 1 < parts {
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
                        self.push_ids(&mut out, lo, hi, entry.upper.clone(), false, params);
                    } else {
                        push(&mut out, entry.upper.clone(), Mode::Skip);
                    }
                }
            }
            lower = upper;
        }
        out
    }

    /// Sends the IDs of `[lo, hi)` as one or more entries, each with at most
    /// [`Params::ids_per_entry`] IDs, so no entry outgrows a frame. The split points are
    /// separators between this side's keys, so the pieces tile the range exactly.
    fn push_ids(
        &self,
        out: &mut Vec<Entry>,
        lo: usize,
        hi: usize,
        upper: Option<Vec<u8>>,
        reply: bool,
        params: Params,
    ) {
        let per = params.ids_per_entry();
        let mut start = lo;
        loop {
            let end = (start + per).min(hi);
            let piece_upper = if end < hi {
                Some(self.separator(end))
            } else {
                upper.clone()
            };
            out.push(Entry {
                upper: piece_upper,
                mode: Mode::Ids {
                    ids: self.ids[start..end].to_vec(),
                    reply,
                },
            });
            if end >= hi {
                break;
            }
            start = end;
        }
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

fn encode_entry(e: &Entry) -> Vec<u8> {
    let mut out = Vec::new();
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
    out
}

/// Splits a message into frames of at most `budget` bytes each. Every frame but the last is
/// marked `more`. The entries keep their order, so the receiver reads them back as one list.
///
/// # Errors
///
/// Refuses an entry that does not fit a frame by itself.
pub fn frames(msg: &[Entry], budget: usize) -> Result<Vec<Vec<u8>>> {
    let mut out = Vec::new();
    let mut body: Vec<u8> = Vec::new();
    let mut count = 0usize;
    for entry in msg {
        let encoded = encode_entry(entry);
        ensure!(
            encoded.len() + FRAME_HEADER_RESERVE <= budget,
            "an entry of {} bytes does not fit a {budget}-byte frame",
            encoded.len()
        );
        if count > 0 && body.len() + encoded.len() + FRAME_HEADER_RESERVE > budget {
            out.push(finish_frame(true, count, &body));
            body.clear();
            count = 0;
        }
        body.extend_from_slice(&encoded);
        count += 1;
    }
    out.push(finish_frame(false, count, &body));
    Ok(out)
}

fn finish_frame(more: bool, count: usize, body: &[u8]) -> Vec<u8> {
    let mut frame = vec![u8::from(more)];
    put_varint(&mut frame, count);
    frame.extend_from_slice(body);
    frame
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

    fn entry(&mut self) -> Result<Entry> {
        let len = self.byte()? as usize;
        ensure!(
            len <= KEY_LEN,
            "a bound of {len} bytes is longer than a key"
        );
        let upper = if len == 0 {
            None
        } else {
            Some(self.take(len)?.to_vec())
        };
        let mode = match self.byte()? {
            0 => Mode::Skip,
            1 => {
                let fp: [u8; 16] = self.take(16)?.try_into()?;
                let count = u32::try_from(self.varint()?)?;
                Mode::Fp { fp, count }
            }
            m @ (2 | 3) => {
                let count = self.varint()?;
                let mut ids = Vec::new();
                for _ in 0..count {
                    ids.push(self.take(32)?.try_into()?);
                }
                Mode::Ids { ids, reply: m == 3 }
            }
            m => bail!("unknown mode byte {m}"),
        };
        Ok(Entry { upper, mode })
    }
}

/// Parses one frame written by [`frames`]. Returns its `more` flag and its entries. Rejects
/// truncated bytes, trailing bytes, and unknown flags and modes.
///
/// # Errors
///
/// Returns an error for any malformed frame.
pub fn decode_frame(bytes: &[u8]) -> Result<(bool, Vec<Entry>)> {
    let mut r = Reader { buf: bytes };
    let more = match r.byte()? {
        0 => false,
        1 => true,
        m => bail!("unknown frame flag {m}"),
    };
    let n = r.varint()?;
    let mut out = Vec::new();
    for _ in 0..n {
        out.push(r.entry()?);
    }
    ensure!(r.buf.is_empty(), "trailing bytes after frame");
    Ok((more, out))
}

/// The entry count in one frame's header. A reader calls it before it decodes the frame, so it
/// can charge the entries against a budget first.
///
/// # Errors
///
/// Returns an error for an empty frame, an unknown flag, or a count that is not a varint.
pub fn frame_entry_count(bytes: &[u8]) -> Result<usize> {
    let mut r = Reader { buf: bytes };
    match r.byte()? {
        0 | 1 => {}
        m => bail!("unknown frame flag {m}"),
    }
    r.varint()
}

/// Checks a whole flight's bounds: each upper bound sorts above the one before it, and only the
/// last entry may be +infinity. A flight that breaks this would make a range with `lo > hi`.
///
/// # Errors
///
/// Refuses a bound that does not increase, and an entry after +infinity.
pub fn check_bounds(msg: &[Entry]) -> Result<()> {
    let mut prev: Option<&[u8]> = None;
    let mut ended = false;
    for entry in msg {
        ensure!(!ended, "an entry follows +infinity");
        match entry.upper.as_deref() {
            None => ended = true,
            Some(bound) => {
                if let Some(p) = prev {
                    ensure!(bound > p, "bounds do not increase");
                }
                prev = Some(bound);
            }
        }
    }
    Ok(())
}

/// Result of an in-memory run: flights, the bytes each side wrote, and what each side learned.
#[derive(Debug)]
pub struct Sim {
    /// Messages written, both sides together.
    pub flights: u32,
    /// Frame bytes the first side wrote, including the frame flags and counts.
    pub bytes_a: usize,
    /// Frame bytes the second side wrote.
    pub bytes_b: usize,
    /// IDs the first side learned, each once.
    pub learned_a: Vec<Id>,
    /// IDs the second side learned, each once.
    pub learned_b: Vec<Id>,
    /// The largest frame written, in bytes.
    pub max_frame: usize,
}

/// Runs the protocol between two in-memory sets, through the same frames as the wire, and with
/// each side's own parameters. `a` opens. Used to check convergence and the symmetric
/// difference without a transport.
///
/// # Errors
///
/// Returns an error for a malformed flight, or if the sync does not end within `max_flights`.
pub fn simulate(a: &Party, pa: Params, b: &Party, pb: Params, max_flights: u32) -> Result<Sim> {
    let mut learned_a = Vec::new();
    let mut learned_b = Vec::new();
    let mut msg = vec![a.full_range()];
    let mut a_sends = true;
    let (mut bytes_a, mut bytes_b, mut flights) = (0usize, 0usize, 0u32);
    let mut max_frame = 0usize;
    loop {
        flights += 1;
        ensure!(flights <= max_flights, "reconciliation did not terminate");
        let (sender_params, receiver_params) = if a_sends { (pa, pb) } else { (pb, pa) };
        let wire = frames(&msg, sender_params.frame_budget())?;
        let bytes: usize = wire.iter().map(Vec::len).sum();
        max_frame = wire.iter().map(Vec::len).fold(max_frame, usize::max);
        if a_sends {
            bytes_a += bytes;
        } else {
            bytes_b += bytes;
        }
        let mut incoming = Vec::new();
        for frame in &wire {
            let (_, mut entries) = decode_frame(frame)?;
            incoming.append(&mut entries);
        }
        check_bounds(&incoming)?;
        if all_skip(&incoming) {
            break;
        }
        msg = if a_sends {
            b.respond(&incoming, receiver_params, &mut learned_b)
        } else {
            a.respond(&incoming, receiver_params, &mut learned_a)
        };
        a_sends = !a_sends;
    }
    Ok(Sim {
        flights,
        bytes_a,
        bytes_b,
        learned_a,
        learned_b,
        max_frame,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use p2panda_core::Hash;

    /// Mixes a seed and an index into one 32-byte digest.
    fn digest(seed: u64, n: u64) -> [u8; 32] {
        let mut input = [0u8; 16];
        input[..8].copy_from_slice(&seed.to_be_bytes());
        input[8..].copy_from_slice(&n.to_be_bytes());
        *Hash::digest(input).as_bytes()
    }

    /// A key and its ID. Keys are built from digests, so they are spread over the key space.
    /// With `zero_heavy`, every key byte is 0 or 1, so bounds end in zero bytes often.
    fn entry(seed: u64, n: u64, zero_heavy: bool) -> (Key, Id) {
        let a = digest(seed, n);
        let b = digest(seed ^ 0xa5a5, n);
        let mut key = [0u8; KEY_LEN];
        key[..32].copy_from_slice(&a);
        key[32..40].copy_from_slice(&n.to_be_bytes());
        key[40..].copy_from_slice(&b[..32]);
        if zero_heavy {
            for byte in &mut key {
                *byte &= 1;
            }
        }
        // The ID is a digest of the key, so two keys never share one.
        let id = *Hash::digest(key).as_bytes();
        (key, id)
    }

    /// `count` distinct entries from `seed`, with `zero_heavy` keys if asked.
    fn set(seed: u64, count: u64, zero_heavy: bool) -> Vec<(Key, Id)> {
        let mut entries: Vec<(Key, Id)> = (0..count).map(|n| entry(seed, n, zero_heavy)).collect();
        entries.sort_unstable_by_key(|e| e.0);
        entries.dedup_by(|a, b| a.0 == b.0);
        entries
    }

    fn party(entries: Vec<(Key, Id)>) -> Party {
        Party::new(entries).unwrap()
    }

    /// The symmetric difference of two sets of IDs.
    fn truth(a: &Party, b: &Party) -> HashSet<Id> {
        let a: HashSet<Id> = a.ids().iter().copied().collect();
        let b: HashSet<Id> = b.ids().iter().copied().collect();
        a.symmetric_difference(&b).copied().collect()
    }

    /// A pseudo-random subset: each entry stays with probability `keep` out of 256.
    fn subset(entries: &[(Key, Id)], seed: u64, keep: u64) -> Vec<(Key, Id)> {
        entries
            .iter()
            .enumerate()
            .filter(|(i, _)| u64::from(digest(seed, *i as u64)[0]) < keep)
            .map(|(_, e)| *e)
            .collect()
    }

    #[test]
    fn test_params_refuse_k_below_two() {
        assert_eq!(Params::new(1, 16), Err(ParamsError::FanOut(1)));
        assert_eq!(Params::new(0, 16), Err(ParamsError::FanOut(0)));
        assert!(Params::new(2, 16).is_ok());
    }

    /// P3.1b: T=0 never ends on one mismatched key, so it is refused before any session runs.
    #[test]
    fn test_params_refuse_t_zero() {
        assert_eq!(Params::new(16, 0), Err(ParamsError::LeafThreshold(0)));
        assert_eq!(Params::new(2, 0), Err(ParamsError::LeafThreshold(0)));
        assert!(Params::new(16, 1).is_ok());
    }

    #[test]
    fn test_params_default_is_k16_t16_with_the_four_mib_budget() {
        let p = Params::default();
        assert_eq!((p.k(), p.t(), p.frame_budget()), (16, 16, 4 << 20));
    }

    #[test]
    fn test_frame_budget_is_bounded_by_the_entry_size_and_the_cap() {
        let p = Params::default();
        assert_eq!(p.with_frame_budget(100), Err(ParamsError::FrameBudget(100)));
        assert_eq!(
            p.with_frame_budget(FRAME_CAP + 1),
            Err(ParamsError::FrameBudget(FRAME_CAP + 1))
        );
        assert_eq!(
            p.with_frame_budget(FRAME_CAP).unwrap().frame_budget(),
            FRAME_CAP
        );
    }

    #[test]
    fn test_session_budget_refuses_below_one_entry() {
        let p = Params::default();
        assert_eq!(p.session_budget(), 64 << 20);
        assert_eq!(
            p.with_session_budget(100),
            Err(ParamsError::SessionBudget(100))
        );
        assert_eq!(p.with_session_budget(256).unwrap().session_budget(), 256);
    }

    /// The count is read from the header alone, so a reader can charge it before it decodes.
    #[test]
    fn test_frame_entry_count_reads_the_header_only() {
        let p = party(set(41, 300, false));
        let empty = party(Vec::new());
        let mut learned = Vec::new();
        let params = Params::new(16, 400)
            .unwrap()
            .with_frame_budget(512)
            .unwrap();
        let msg = p.respond(&[empty.full_range()], params, &mut learned);
        let wire = frames(&msg, params.frame_budget()).unwrap();
        assert!(wire.len() > 1);
        for frame in &wire {
            let (_, entries) = decode_frame(frame).unwrap();
            assert_eq!(frame_entry_count(frame).unwrap(), entries.len());
        }
        assert!(frame_entry_count(&[]).is_err(), "an empty frame");
        assert!(frame_entry_count(&[2, 0]).is_err(), "unknown frame flag");
    }

    #[test]
    fn test_separators_split_keys_with_nul_bytes() {
        let p = party(set(7, 300, true));
        for pos in 1..p.len() {
            let s = p.separator(pos);
            assert_eq!(
                p.idx(&s),
                pos,
                "separator {} at {pos} splits the keys wrongly",
                hex(&s)
            );
        }
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// S7's cases: the exact difference with every K and T in a grid, and with zero-heavy keys.
    #[test]
    fn test_respond_learns_exact_difference_for_s7_parameters() {
        for (seed, k, t) in [
            (11u64, 2, 4),
            (12, 16, 16),
            (13, 16, 4),
            (14, 32, 16),
            (15, 32, 1),
        ] {
            let all = set(seed, 700, true);
            let a_keys: Vec<(Key, Id)> = all
                .iter()
                .copied()
                .filter(|e| e.0[0] == 0 || e.0[1] == 0)
                .collect();
            let b_keys: Vec<(Key, Id)> = all
                .iter()
                .copied()
                .filter(|e| e.0[2] == 0 || e.0[71] == 1)
                .collect();
            let (a, b) = (party(a_keys), party(b_keys));
            let p = Params::new(k, t).unwrap();
            let sim = simulate(&a, p, &b, p, 10_000).unwrap();
            let expected = truth(&a, &b);
            assert!(!expected.is_empty());
            assert_eq!(
                sim.learned_a.iter().copied().collect::<HashSet<_>>(),
                expected,
                "K={k} T={t}"
            );
            assert_eq!(
                sim.learned_b.iter().copied().collect::<HashSet<_>>(),
                expected,
                "K={k} T={t}"
            );
            assert_eq!(
                sim.learned_a.len(),
                expected.len(),
                "no ID is learned twice"
            );
        }
    }

    /// Property: random sets, random K and T on each side, over a grid of K in {2, 4, 16, 32}
    /// and T in {1, 4, 16}. Both sides learn exactly the symmetric difference, once each.
    #[test]
    fn test_property_both_sides_learn_the_exact_difference_for_every_k_and_t_pair() {
        let ks = [2usize, 4, 16, 32];
        let ts = [1usize, 4, 16];
        let mut run = 0u64;
        for &ka in &ks {
            for &ta in &ts {
                for &kb in &ks {
                    for &tb in &ts {
                        run += 1;
                        let universe = set(100 + run, 260, run.is_multiple_of(2));
                        let a = party(subset(&universe, run, 200));
                        let b = party(subset(&universe, run ^ 0x55, 200));
                        let (pa, pb) = (Params::new(ka, ta).unwrap(), Params::new(kb, tb).unwrap());
                        let sim = simulate(&a, pa, &b, pb, 10_000).unwrap();
                        let expected = truth(&a, &b);
                        let label = format!("A K={ka} T={ta}, B K={kb} T={tb}");
                        assert_eq!(
                            sim.learned_a.iter().copied().collect::<HashSet<_>>(),
                            expected,
                            "{label}"
                        );
                        assert_eq!(
                            sim.learned_b.iter().copied().collect::<HashSet<_>>(),
                            expected,
                            "{label}"
                        );
                        assert_eq!(sim.learned_a.len(), expected.len(), "{label}: no duplicate");
                        assert_eq!(sim.learned_b.len(), expected.len(), "{label}: no duplicate");
                    }
                }
            }
        }
    }

    /// The leaf threshold and the fan-out decide the shape of an answer to a mismatched range of
    /// 40 keys: at most T keys are sent as IDs, and more than T split into min(K, n) children.
    #[test]
    fn test_the_leaf_threshold_and_the_fan_out_shape_the_answer() {
        let responder = party(set(31, 40, false));
        let root = party(Vec::new()).full_range();
        let mut learned = Vec::new();

        let leaf = responder.respond(&[root.clone()], Params::new(2, 40).unwrap(), &mut learned);
        assert!(
            leaf.iter()
                .all(|e| matches!(e.mode, Mode::Ids { reply: true, .. })),
            "n = T sends IDs"
        );

        let split = responder.respond(&[root.clone()], Params::new(4, 39).unwrap(), &mut learned);
        assert_eq!(split.len(), 4, "n > T splits into K children");
        assert!(split.iter().all(|e| matches!(e.mode, Mode::Fp { .. })));

        let narrow = responder.respond(&[root.clone()], Params::new(2, 1).unwrap(), &mut learned);
        assert_eq!(narrow.len(), 2, "K = 2 gives two children");

        let small = party(set(32, 3, false));
        let answer = small.respond(&[root], Params::new(16, 1).unwrap(), &mut learned);
        assert_eq!(
            answer.len(),
            3,
            "with fewer keys than K, each key is its own child"
        );
    }

    #[test]
    fn test_identical_sets_end_in_one_flight_of_skips_after_the_root() {
        let entries = set(3, 100, false);
        let (a, b) = (party(entries.clone()), party(entries));
        let sim = simulate(&a, Params::default(), &b, Params::default(), 10).unwrap();
        assert_eq!(
            sim.flights, 2,
            "the root fingerprint matches, so B answers with skips"
        );
        assert!(sim.learned_a.is_empty() && sim.learned_b.is_empty());
    }

    /// Each mode, encoded and decoded, comes back as written.
    #[test]
    fn test_each_mode_round_trips_through_a_frame() {
        let msg = vec![
            Entry {
                upper: Some(vec![1, 0, 7]),
                mode: Mode::Skip,
            },
            Entry {
                upper: Some(vec![9]),
                mode: Mode::Fp {
                    fp: [3; 16],
                    count: 70_000,
                },
            },
            Entry {
                upper: Some(vec![0x80, 0]),
                mode: Mode::Ids {
                    ids: vec![[4; 32], [5; 32]],
                    reply: false,
                },
            },
            Entry {
                upper: None,
                mode: Mode::Ids {
                    ids: vec![],
                    reply: true,
                },
            },
        ];
        let wire = frames(&msg, 4096).unwrap();
        assert_eq!(wire.len(), 1);
        assert_eq!(decode_frame(&wire[0]).unwrap(), (false, msg));
    }

    #[test]
    fn test_wire_rejects_truncated_trailing_and_unknown_bytes() {
        let p = party(set(5, 200, false));
        let mut learned = Vec::new();
        let msg = p.respond(
            &[p.full_range()],
            Params::new(16, 16).unwrap(),
            &mut learned,
        );
        let wire = frames(&msg, DEFAULT_FRAME_BUDGET).unwrap();
        let bytes = &wire[0];
        assert!(decode_frame(&bytes[..bytes.len() - 1]).is_err());
        let mut extra = bytes.clone();
        extra.push(0);
        assert!(decode_frame(&extra).is_err());
        assert!(decode_frame(&[2, 0]).is_err(), "unknown frame flag");
        assert!(decode_frame(&[0, 1, 0, 9]).is_err(), "unknown mode byte");
        assert!(
            decode_frame(&[0, 1, 65]).is_err(),
            "bound longer than a key"
        );
    }

    #[test]
    fn test_bounds_must_increase_and_end_at_infinity() {
        let skip = |upper: Option<Vec<u8>>| Entry {
            upper,
            mode: Mode::Skip,
        };
        assert!(check_bounds(&[skip(Some(vec![2])), skip(Some(vec![3])), skip(None)]).is_ok());
        assert!(check_bounds(&[skip(Some(vec![3])), skip(Some(vec![2]))]).is_err());
        assert!(check_bounds(&[skip(Some(vec![3])), skip(Some(vec![3]))]).is_err());
        assert!(check_bounds(&[skip(None), skip(Some(vec![1]))]).is_err());
    }

    #[test]
    fn test_frames_fit_the_budget_and_read_back_in_order() {
        let p = party(set(9, 3000, false));
        let empty = party(Vec::new());
        let mut learned = Vec::new();
        // T above the set size, so a mismatched root answers with every ID, in pieces of the budget.
        let params = Params::new(16, 3000)
            .unwrap()
            .with_frame_budget(512)
            .unwrap();
        let msg = p.respond(&[empty.full_range()], params, &mut learned);
        let wire = frames(&msg, params.frame_budget()).unwrap();
        assert!(wire.len() > 1);
        let mut back = Vec::new();
        for (i, frame) in wire.iter().enumerate() {
            assert!(
                frame.len() <= params.frame_budget(),
                "frame {i} is {} bytes",
                frame.len()
            );
            let (more, mut entries) = decode_frame(frame).unwrap();
            assert_eq!(more, i + 1 < wire.len());
            back.append(&mut entries);
        }
        assert_eq!(back, msg);
    }

    /// An ID list above the budget is split into entries that each fit, and the pieces tile the
    /// range, so the receiver learns the same IDs.
    #[test]
    fn test_an_id_reply_above_the_budget_is_split_and_still_exact() {
        let full = party(set(21, 2000, false));
        let empty = party(Vec::new());
        let params = Params::new(16, 16).unwrap().with_frame_budget(256).unwrap();
        // The empty side answers the full side's root with an empty ID list, so the full side
        // must send every one of its IDs back, in pieces.
        let msg = vec![Entry {
            upper: None,
            mode: Mode::Ids {
                ids: vec![],
                reply: true,
            },
        }];
        let mut learned = Vec::new();
        let reply = full.respond(&msg, params, &mut learned);
        assert!(reply.len() > 1, "the reply is split");
        let wire = frames(&reply, params.frame_budget()).unwrap();
        assert!(wire.iter().all(|f| f.len() <= 256));
        let mut back = Vec::new();
        for frame in &wire {
            back.extend(decode_frame(frame).unwrap().1);
        }
        check_bounds(&back).unwrap();
        // The empty side learns every ID the pieces carry.
        let mut learned_by_empty = Vec::new();
        let answer = empty.respond(&back, params, &mut learned_by_empty);
        assert!(all_skip(&answer), "the empty side has nothing to send back");
        assert_eq!(learned_by_empty.len(), full.len());
        let got: HashSet<Id> = learned_by_empty.into_iter().collect();
        assert_eq!(got, full.ids().iter().copied().collect::<HashSet<Id>>());
    }

    /// Bootstrap above the frame limit, both directions, in memory. Each side learns the whole
    /// symmetric difference, which is all 550,000 IDs, and no frame goes above the cap. One flight
    /// of that size would be about 17.6 MB, so the reply has to be chunked.
    #[test]
    fn test_bootstrap_above_the_frame_cap_in_both_directions() {
        let full = party(set(77, 550_000, false));
        let empty = party(Vec::new());
        let p = Params::default();
        let expected: HashSet<Id> = full.ids().iter().copied().collect();
        for (label, a, b) in [
            ("empty dials full", &empty, &full),
            ("full dials empty", &full, &empty),
        ] {
            let sim = simulate(a, p, b, p, 100_000).unwrap();
            let learned_a: HashSet<Id> = sim.learned_a.iter().copied().collect();
            let learned_b: HashSet<Id> = sim.learned_b.iter().copied().collect();
            assert_eq!(learned_a, expected, "{label}");
            assert_eq!(learned_b, expected, "{label}");
            assert!(sim.flights > 1, "{label}: the set was chunked over flights");
            assert!(
                sim.max_frame <= FRAME_CAP,
                "{label}: a {}-byte frame",
                sim.max_frame
            );
        }
    }
}
