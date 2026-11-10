//! Throwaway PoC for issue #2241: intermittent-signature live stream.
//!
//! Implements the wire shape settled by the design in issue #2240
//! (`doc/2026-11-01_design_id2240.md`, merged in PR #3867) against a
//! toy collaborative document (append/backspace ops standing in for the
//! `web/collab.rs` steps/cursors):
//!
//! - a session header committing to a fresh per-session MAC key
//!   (BLAKE3 commitment; the raw key never appears in any record),
//! - per-frame HMAC-SHA-256 authentication (no per-event signatures),
//! - segment checkpoints every 256 frames or 2 s, whichever first,
//!   plus flushes on commit/blur/disconnect, each checkpoint an
//!   Ed25519-signed record chained to its predecessor and committing
//!   to an ordered rolling BLAKE3 root over the segment's frames.
//!
//! It then demonstrates the three scenarios issue #2241 requires:
//!
//! 1. a late joiner reconstructs from the latest checkpoint,
//! 2. a reconnecting peer reconciles at a checkpoint boundary,
//! 3. a verifier detects a truncated segment tail,
//!
//! and measures signature count/bytes versus signing every event.
//!
//! THROWAWAY: this is an `examples/` prototype for review only. Per
//! the program rule in issue #2056 it must not be merged into the
//! permanent feature, and no permanent feature may be built on it
//! before a review round. Simplifications versus the design: the
//! session ID is a BLAKE3 hash of the header core instead of a
//! `chain::sign` event ID, the checkpoint snapshot travels inside the
//! segment body instead of the content-addressed body path, and
//! SHA-256/HMAC are implemented locally (known-answer-checked at
//! startup) so the example needs no optional-feature dependencies.
//!
//! Run with: `cargo run --example session_checkpoint_poc`

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use rand::RngExt as _;

// ---------------------------------------------------------------------------
// SHA-256 + HMAC-SHA-256 (FIPS 180-4 / RFC 2104), self-contained for the PoC.
// Correctness is pinned by the known-answer checks in `self_check_crypto`.
// ---------------------------------------------------------------------------

const SHA256_K: [u32; 64] = [
    0x428a_2f98,
    0x7137_4491,
    0xb5c0_fbcf,
    0xe9b5_dba5,
    0x3956_c25b,
    0x59f1_11f1,
    0x923f_82a4,
    0xab1c_5ed5,
    0xd807_aa98,
    0x1283_5b01,
    0x2431_85be,
    0x550c_7dc3,
    0x72be_5d74,
    0x80de_b1fe,
    0x9bdc_06a7,
    0xc19b_f174,
    0xe49b_69c1,
    0xefbe_4786,
    0x0fc1_9dc6,
    0x240c_a1cc,
    0x2de9_2c6f,
    0x4a74_84aa,
    0x5cb0_a9dc,
    0x76f9_88da,
    0x983e_5152,
    0xa831_c66d,
    0xb003_27c8,
    0xbf59_7fc7,
    0xc6e0_0bf3,
    0xd5a7_9147,
    0x06ca_6351,
    0x1429_2967,
    0x27b7_0a85,
    0x2e1b_2138,
    0x4d2c_6dfc,
    0x5338_0d13,
    0x650a_7354,
    0x766a_0abb,
    0x81c2_c92e,
    0x9272_2c85,
    0xa2bf_e8a1,
    0xa81a_664b,
    0xc24b_8b70,
    0xc76c_51a3,
    0xd192_e819,
    0xd699_0624,
    0xf40e_3585,
    0x106a_a070,
    0x19a4_c116,
    0x1e37_6c08,
    0x2748_774c,
    0x34b0_bcb5,
    0x391c_0cb3,
    0x4ed8_aa4a,
    0x5b9c_ca4f,
    0x682e_6ff3,
    0x748f_82ee,
    0x78a5_636f,
    0x84c8_7814,
    0x8cc7_0208,
    0x90be_fffa,
    0xa450_6ceb,
    0xbef9_a3f7,
    0xc671_78f2,
];

fn sha256(data: &[u8]) -> [u8; 32] {
    let mut h: [u32; 8] = [
        0x6a09_e667,
        0xbb67_ae85,
        0x3c6e_f372,
        0xa54f_f53a,
        0x510e_527f,
        0x9b05_688c,
        0x1f83_d9ab,
        0x5be0_cd19,
    ];
    let bit_len = (data.len() as u64).wrapping_mul(8);
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());
    for block in msg.chunks_exact(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                block[i * 4],
                block[i * 4 + 1],
                block[i * 4 + 2],
                block[i * 4 + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh) =
            (h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]);
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(SHA256_K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }
    let mut out = [0u8; 32];
    for (i, word) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    let mut block_key = [0u8; 64];
    if key.len() > 64 {
        let hashed = sha256(key);
        block_key[..32].copy_from_slice(&hashed);
    } else {
        block_key[..key.len()].copy_from_slice(key);
    }
    let mut inner = Vec::with_capacity(64 + message.len());
    inner.extend(block_key.iter().map(|b| b ^ 0x36));
    inner.extend_from_slice(message);
    let inner_hash = sha256(&inner);
    let mut outer = Vec::with_capacity(64 + 32);
    outer.extend(block_key.iter().map(|b| b ^ 0x5c));
    outer.extend_from_slice(&inner_hash);
    sha256(&outer)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn self_check_crypto() -> anyhow::Result<()> {
    // FIPS 180-4 example: SHA-256("abc").
    anyhow::ensure!(
        hex(&sha256(b"abc")) == "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        "SHA-256 known-answer check failed"
    );
    // RFC 4231 test case 1: key = 20 x 0x0b, data = "Hi There".
    anyhow::ensure!(
        hex(&hmac_sha256(&[0x0b; 20], b"Hi There"))
            == "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7",
        "HMAC-SHA-256 RFC 4231 case 1 failed"
    );
    // RFC 4231 test case 2: key = "Jefe".
    anyhow::ensure!(
        hex(&hmac_sha256(b"Jefe", b"what do ya want for nothing?"))
            == "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843",
        "HMAC-SHA-256 RFC 4231 case 2 failed"
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Design primitives (issue #2240, section 1): exact domain strings and
// formulas from the merged design doc.
// ---------------------------------------------------------------------------

fn blake(data: &[u8]) -> [u8; 32] {
    *blake3::hash(data).as_bytes()
}

fn mac_key_commitment(nonce: &[u8; 32], key: &[u8; 32]) -> [u8; 32] {
    let mut buf = Vec::with_capacity(24 + 32 + 32);
    buf.extend_from_slice(b"id.session.mac-key.v1");
    buf.extend_from_slice(nonce);
    buf.extend_from_slice(key);
    blake(&buf)
}

fn frame_mac(key: &[u8; 32], session_id: &[u8; 32], seq: u64, canonical: &[u8]) -> [u8; 32] {
    let mut buf = Vec::with_capacity(20 + 32 + 8 + canonical.len());
    buf.extend_from_slice(b"id.session.frame.v1");
    buf.extend_from_slice(session_id);
    buf.extend_from_slice(&seq.to_be_bytes());
    buf.extend_from_slice(canonical);
    hmac_sha256(key, &buf)
}

fn frame_digest(session_id: &[u8; 32], seq: u64, canonical: &[u8], mac: &[u8; 32]) -> [u8; 32] {
    let mut buf = Vec::with_capacity(20 + 32 + 8 + canonical.len() + 32);
    buf.extend_from_slice(b"id.session.frame.v1");
    buf.extend_from_slice(session_id);
    buf.extend_from_slice(&seq.to_be_bytes());
    buf.extend_from_slice(canonical);
    buf.extend_from_slice(mac);
    blake(&buf)
}

/// Rolling segment root, seeded per the design: segment 0 seeds from
/// the session-start ID, later segments from the previous checkpoint ID.
fn segment_root(
    session_id: &[u8; 32],
    segment_seq: u64,
    seed_id: &[u8; 32],
    digests: &[[u8; 32]],
) -> [u8; 32] {
    let mut seed = Vec::with_capacity(22 + 32 + 8 + 32);
    seed.extend_from_slice(b"id.session.segment.v1");
    seed.extend_from_slice(session_id);
    seed.extend_from_slice(&segment_seq.to_be_bytes());
    seed.extend_from_slice(seed_id);
    let mut root = blake(&seed);
    for digest in digests {
        let mut step = Vec::with_capacity(27 + 32 + 32);
        step.extend_from_slice(b"id.session.segment.step.v1");
        step.extend_from_slice(&root);
        step.extend_from_slice(digest);
        root = blake(&step);
    }
    root
}

fn state_hash(doc: &str) -> [u8; 32] {
    blake(doc.as_bytes())
}

/// The canonical header core the session ID commits to (built by
/// `Producer::new`). Verifiers recompute it from the header's fields
/// and compare against `header.session_id`, so a header whose
/// participant list, policy limits, or initial state was modified
/// after creation is rejected instead of trusted (Codex P2 on
/// PR #4435: the signature check authenticates only the supplied
/// session ID, so un-recomputed fields were unauthenticated).
fn header_core_bytes(header: &SessionHeader) -> Vec<u8> {
    let mut core = Vec::new();
    core.extend_from_slice(b"id.session.start.v1");
    core.extend_from_slice(&header.participants[0]);
    core.extend_from_slice(&header.participants[1]);
    core.extend_from_slice(&header.initial_state);
    core.extend_from_slice(&header.nonce);
    core.extend_from_slice(&header.mac_key_commitment);
    core.extend_from_slice(&header.max_frames.to_be_bytes());
    core.extend_from_slice(&header.max_interval_ms.to_be_bytes());
    core
}

fn verify_header(header: &SessionHeader) -> anyhow::Result<()> {
    anyhow::ensure!(
        blake(&header_core_bytes(header)) == header.session_id,
        "header fields do not recompute to the session ID: participants/policy are not bound to it"
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Records
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CloseReason {
    Count,
    Interval,
    Commit,
    Blur,
    Disconnect,
}

impl CloseReason {
    fn byte(self) -> u8 {
        match self {
            Self::Count => 0,
            Self::Interval => 1,
            Self::Commit => 2,
            Self::Blur => 3,
            Self::Disconnect => 4,
        }
    }
    fn name(self) -> &'static str {
        match self {
            Self::Count => "count",
            Self::Interval => "interval",
            Self::Commit => "commit",
            Self::Blur => "blur",
            Self::Disconnect => "disconnect",
        }
    }
}

#[derive(Debug, Clone)]
struct SessionHeader {
    session_id: [u8; 32],
    participants: [[u8; 32]; 2],
    initial_state: [u8; 32],
    nonce: [u8; 32],
    mac_key_commitment: [u8; 32],
    max_frames: u32,
    max_interval_ms: u32,
}

#[derive(Debug, Clone)]
struct Checkpoint {
    segment_seq: u64,
    previous_checkpoint: Option<[u8; 32]>,
    first_frame_seq: u64,
    last_frame_seq: u64,
    frames_root: [u8; 32],
    state_before: [u8; 32],
    state_after: [u8; 32],
    close_reason: CloseReason,
    signature: [u8; 64],
    id: [u8; 32],
}

impl Checkpoint {
    fn core_bytes(&self, session_id: &[u8; 32]) -> Vec<u8> {
        let mut buf = Vec::with_capacity(256);
        buf.extend_from_slice(b"id.session.checkpoint.v1");
        buf.extend_from_slice(session_id);
        buf.extend_from_slice(&self.segment_seq.to_be_bytes());
        buf.extend_from_slice(&self.previous_checkpoint.unwrap_or([0u8; 32]));
        buf.extend_from_slice(&self.first_frame_seq.to_be_bytes());
        buf.extend_from_slice(&self.last_frame_seq.to_be_bytes());
        buf.extend_from_slice(&self.frames_root);
        buf.extend_from_slice(&self.state_before);
        buf.extend_from_slice(&self.state_after);
        buf.push(self.close_reason.byte());
        buf
    }
    /// Number of frames the checkpoint commits to, or `None` when
    /// the range is reversed or overflows. An owner-signed checkpoint
    /// can carry any values: unchecked arithmetic here panicked in
    /// debug builds and, for `0..=u64::MAX`, wrapped to a count of
    /// zero in release builds, which would have let an empty body
    /// pass the frame-count check (Codex P2 on PR #4435).
    fn frame_count(&self) -> Option<u64> {
        self.last_frame_seq
            .checked_sub(self.first_frame_seq)?
            .checked_add(1)
    }
}

/// One segment's body: the ordered (canonical frame, MAC) pairs plus
/// the document snapshot at the checkpoint (the PoC's stand-in for the
/// content-addressed body a late joiner fetches).
#[derive(Debug, Clone)]
struct SegmentBody {
    frames: Vec<(Vec<u8>, [u8; 32])>,
    state_snapshot: String,
    /// Document at the segment's first frame (the `state_before`
    /// content). Keyless verification replays the segment's frames
    /// onto this snapshot; without it "replay-verified" was a claim
    /// no verifier had actually checked (Codex P2 on PR #4435).
    state_before_snapshot: String,
}

// ---------------------------------------------------------------------------
// Toy collaborative document: append / backspace ops as canonical frames.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
enum Op {
    Append(u8),
    Backspace,
}

impl Op {
    fn canonical(self) -> Vec<u8> {
        match self {
            Self::Append(byte) => vec![0, byte],
            Self::Backspace => vec![1],
        }
    }
    fn apply(self, doc: &mut String) {
        match self {
            Self::Append(byte) => doc.push(byte as char),
            Self::Backspace => {
                doc.pop();
            }
        }
    }
    fn from_canonical(bytes: &[u8]) -> anyhow::Result<Self> {
        match bytes {
            [0, byte] => Ok(Self::Append(*byte)),
            [1] => Ok(Self::Backspace),
            _ => anyhow::bail!("unknown canonical frame: {bytes:?}"),
        }
    }
}

// ---------------------------------------------------------------------------
// Producer
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct Producer {
    header: SessionHeader,
    key: [u8; 32],
    signer: SigningKey,
    doc: String,
    next_seq: u64,
    segment_first_seq: u64,
    segment_start_ms: u64,
    segment_state_before: [u8; 32],
    segment_doc_before: String,
    segment_frames: Vec<(Vec<u8>, [u8; 32])>,
    checkpoints: Vec<Checkpoint>,
    bodies: Vec<SegmentBody>,
}

impl Producer {
    fn new(owner: &SigningKey, peer: &VerifyingKey) -> Self {
        let mut key = [0u8; 32];
        let mut nonce = [0u8; 32];
        rand::rng().fill(&mut key);
        rand::rng().fill(&mut nonce);
        let initial_state = state_hash("");
        let commitment = mac_key_commitment(&nonce, &key);
        // PoC session ID: BLAKE3 over the header core (the production
        // shape derives it from the signed session.start event bytes).
        let mut header = SessionHeader {
            session_id: [0u8; 32],
            participants: [*owner.verifying_key().as_bytes(), *peer.as_bytes()],
            initial_state,
            nonce,
            mac_key_commitment: commitment,
            max_frames: 256,
            max_interval_ms: 2000,
        };
        header.session_id = blake(&header_core_bytes(&header));
        let header = header;
        Self {
            segment_state_before: initial_state,
            segment_doc_before: String::new(),
            header,
            key,
            signer: owner.clone(),
            doc: String::new(),
            next_seq: 0,
            segment_first_seq: 0,
            segment_start_ms: 0,
            segment_frames: Vec::new(),
            checkpoints: Vec::new(),
            bodies: Vec::new(),
        }
    }

    fn push_frame(&mut self, op: Op, at_ms: u64, flush: Option<CloseReason>) -> anyhow::Result<()> {
        // Interval boundary: a frame arriving >= max_interval after the
        // segment start closes the previous segment first.
        if !self.segment_frames.is_empty()
            && at_ms.saturating_sub(self.segment_start_ms) >= u64::from(self.header.max_interval_ms)
        {
            self.close_segment(CloseReason::Interval)?;
        }
        if self.segment_frames.is_empty() {
            self.segment_start_ms = at_ms;
            self.segment_first_seq = self.next_seq;
            self.segment_doc_before = self.doc.clone();
        }
        let canonical = op.canonical();
        let mac = frame_mac(
            &self.key,
            &self.header.session_id,
            self.next_seq,
            &canonical,
        );
        op.apply(&mut self.doc);
        self.segment_frames.push((canonical, mac));
        self.next_seq += 1;
        if let Some(reason) = flush {
            self.close_segment(reason)?;
        } else if self.segment_frames.len() >= self.header.max_frames as usize {
            self.close_segment(CloseReason::Count)?;
        }
        Ok(())
    }

    /// Clock tick with no incoming frame: if the open segment has
    /// reached the interval boundary, checkpoint it now. Without
    /// this, a pause in frames left the segment unsigned until the
    /// next frame (or forever, if none arrived), so the advertised
    /// 2-second limit did not bound the unsigned tail (Codex P2 on
    /// PR #4435). A live driver calls this from its timer; the PoC
    /// calls it explicitly.
    fn tick(&mut self, at_ms: u64) -> anyhow::Result<()> {
        if !self.segment_frames.is_empty()
            && at_ms.saturating_sub(self.segment_start_ms) >= u64::from(self.header.max_interval_ms)
        {
            self.close_segment(CloseReason::Interval)?;
        }
        Ok(())
    }

    fn close_segment(&mut self, reason: CloseReason) -> anyhow::Result<()> {
        if self.segment_frames.is_empty() {
            return Ok(());
        }
        let segment_seq = self.checkpoints.len() as u64;
        let previous = self.checkpoints.last().map(|c| c.id);
        let seed_id = previous.unwrap_or(self.header.session_id);
        let digests: Vec<[u8; 32]> = self
            .segment_frames
            .iter()
            .enumerate()
            .map(|(i, (canonical, mac))| {
                frame_digest(
                    &self.header.session_id,
                    self.segment_first_seq + i as u64,
                    canonical,
                    mac,
                )
            })
            .collect();
        let mut checkpoint = Checkpoint {
            segment_seq,
            previous_checkpoint: previous,
            first_frame_seq: self.segment_first_seq,
            last_frame_seq: self.next_seq - 1,
            frames_root: segment_root(&self.header.session_id, segment_seq, &seed_id, &digests),
            state_before: self.segment_state_before,
            state_after: state_hash(&self.doc),
            close_reason: reason,
            signature: [0u8; 64],
            id: [0u8; 32],
        };
        let core = checkpoint.core_bytes(&self.header.session_id);
        checkpoint.id = blake(&core);
        checkpoint.signature = self.signer.sign(&core).to_bytes();
        self.segment_state_before = checkpoint.state_after;
        self.checkpoints.push(checkpoint);
        self.bodies.push(SegmentBody {
            frames: std::mem::take(&mut self.segment_frames),
            state_snapshot: self.doc.clone(),
            state_before_snapshot: std::mem::take(&mut self.segment_doc_before),
        });
        Ok(())
    }

    fn finish(&mut self) -> anyhow::Result<()> {
        self.close_segment(CloseReason::Disconnect)
    }
}

// ---------------------------------------------------------------------------
// Verifier (design section 3, steps 3/5/6 at PoC scope)
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq)]
enum VerifyLevel {
    /// Root recomputed from the body without the session key.
    ReplayVerifiedKeyless,
    /// Root recomputed and every frame MAC checked against the key.
    MacAuthenticated,
}

fn verify_segment(
    header: &SessionHeader,
    checkpoint: &Checkpoint,
    body: &SegmentBody,
    key: Option<&[u8; 32]>,
    owner: &VerifyingKey,
) -> anyhow::Result<VerifyLevel> {
    verify_header(header)?;
    owner
        .verify(
            &checkpoint.core_bytes(&header.session_id),
            &Signature::from_bytes(&checkpoint.signature),
        )
        .map_err(|_| anyhow::anyhow!("checkpoint signature invalid"))?;
    anyhow::ensure!(
        checkpoint.id == blake(&checkpoint.core_bytes(&header.session_id)),
        "checkpoint ID does not match its core"
    );
    let frame_count = checkpoint
        .frame_count()
        .ok_or_else(|| anyhow::anyhow!("checkpoint frame range is reversed or overflows"))?;
    anyhow::ensure!(
        frame_count == body.frames.len() as u64,
        "truncated or extended segment: checkpoint commits {} frames, body carries {}",
        frame_count,
        body.frames.len()
    );
    anyhow::ensure!(
        frame_count <= u64::from(header.max_frames),
        "segment exceeds the signed max_frames policy"
    );
    let seed_id = checkpoint.previous_checkpoint.unwrap_or(header.session_id);
    let mut digests = Vec::with_capacity(body.frames.len());
    for (i, (canonical, mac)) in body.frames.iter().enumerate() {
        let seq = checkpoint.first_frame_seq + i as u64;
        if let Some(key) = key {
            let expected = frame_mac(key, &header.session_id, seq, canonical);
            anyhow::ensure!(&expected == mac, "frame {seq} MAC mismatch");
        }
        digests.push(frame_digest(&header.session_id, seq, canonical, mac));
    }
    let root = segment_root(
        &header.session_id,
        checkpoint.segment_seq,
        &seed_id,
        &digests,
    );
    anyhow::ensure!(
        root == checkpoint.frames_root,
        "frames root mismatch: body is not the transcript the checkpoint committed to"
    );
    anyhow::ensure!(
        state_hash(&body.state_snapshot) == checkpoint.state_after,
        "segment snapshot does not hash to the checkpoint state_after"
    );
    // Replay: the snapshot hash check above only proves the snapshot
    // matches the signed state_after - an owner-signed checkpoint can
    // commit a snapshot its frames never produce. Actually apply the
    // segment's operations to the pre-segment state (parsing every
    // canonical frame) and require the replay to land exactly on the
    // committed snapshot before calling this replay-verified.
    anyhow::ensure!(
        state_hash(&body.state_before_snapshot) == checkpoint.state_before,
        "segment pre-state snapshot does not hash to the checkpoint state_before"
    );
    let mut replayed = body.state_before_snapshot.clone();
    for (canonical, _) in &body.frames {
        Op::from_canonical(canonical)?.apply(&mut replayed);
    }
    anyhow::ensure!(
        replayed == body.state_snapshot && state_hash(&replayed) == checkpoint.state_after,
        "replaying the segment frames does not produce the checkpoint snapshot"
    );
    Ok(if key.is_some() {
        VerifyLevel::MacAuthenticated
    } else {
        VerifyLevel::ReplayVerifiedKeyless
    })
}

fn verify_chain(header: &SessionHeader, checkpoints: &[Checkpoint]) -> anyhow::Result<()> {
    verify_header(header)?;
    let mut expected_state = header.initial_state;
    let mut expected_first = 0u64;
    for (i, checkpoint) in checkpoints.iter().enumerate() {
        anyhow::ensure!(
            checkpoint.segment_seq == i as u64,
            "segment sequence gap at {i}"
        );
        anyhow::ensure!(
            checkpoint.previous_checkpoint
                == checkpoints
                    .get(i.wrapping_sub(1))
                    .map(|c| c.id)
                    .filter(|_| i > 0),
            "checkpoint {i} does not chain to its immediate predecessor"
        );
        anyhow::ensure!(
            checkpoint.first_frame_seq == expected_first,
            "frame sequence gap before segment {i}"
        );
        anyhow::ensure!(
            checkpoint.state_before == expected_state,
            "state discontinuity at segment {i}"
        );
        expected_state = checkpoint.state_after;
        expected_first = checkpoint
            .last_frame_seq
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("frame sequence overflows at segment {i}"))?;
    }
    Ok(())
}

fn replay_from(
    mut doc: String,
    checkpoints: &[Checkpoint],
    bodies: &[SegmentBody],
) -> anyhow::Result<String> {
    for (checkpoint, body) in checkpoints.iter().zip(bodies.iter()) {
        anyhow::ensure!(
            state_hash(&doc) == checkpoint.state_before,
            "replay state_before mismatch at segment {}",
            checkpoint.segment_seq
        );
        for (canonical, _) in &body.frames {
            Op::from_canonical(canonical)?.apply(&mut doc);
        }
        anyhow::ensure!(
            state_hash(&doc) == checkpoint.state_after,
            "replay state_after mismatch at segment {}",
            checkpoint.segment_seq
        );
    }
    Ok(doc)
}

// ---------------------------------------------------------------------------
// Demo script + scenarios
// ---------------------------------------------------------------------------

fn build_session() -> anyhow::Result<Producer> {
    let owner = SigningKey::from_bytes(&[7u8; 32]);
    let peer = SigningKey::from_bytes(&[9u8; 32]).verifying_key();
    let mut producer = Producer::new(&owner, &peer);
    let text: Vec<u8> = b"the quick brown fox jumps over the lazy dog. ".to_vec();
    // 700 frames at 10 ms spacing, with one 3 s pause before frame 300
    // (interval boundary), a blur flush at frame 420, a commit flush at
    // frame 650, two backspaces
    // near the end, and a disconnect flush at the end.
    for i in 0..700u64 {
        let at_ms = i * 10 + if i >= 300 { 3000 } else { 0 };
        if i == 300 {
            // The 3 s pause crosses the 2 s boundary mid-pause: a
            // clock tick at the boundary (segment started at
            // 2560 ms) checkpoints the idle segment instead of
            // waiting for frame 300 to arrive at 6000 ms.
            producer.tick(4560)?;
        }
        let op = if i == 680 || i == 681 {
            Op::Backspace
        } else {
            Op::Append(text[(i as usize) % text.len()])
        };
        let flush = if i == 650 {
            Some(CloseReason::Commit)
        } else if i == 420 {
            Some(CloseReason::Blur)
        } else {
            None
        };
        producer.push_frame(op, at_ms, flush)?;
    }
    producer.finish()?;
    Ok(producer)
}

fn main() -> anyhow::Result<()> {
    self_check_crypto()?;
    println!("crypto self-checks: SHA-256 + HMAC-SHA-256 known answers OK");

    let producer = build_session()?;
    let header = &producer.header;
    let owner_key = producer.signer.verifying_key();
    let total_frames = producer.next_seq;
    println!(
        "session {} : {} frames, {} checkpoints",
        hex(&header.session_id[..8]),
        total_frames,
        producer.checkpoints.len()
    );
    for checkpoint in &producer.checkpoints {
        println!(
            "  segment {} frames {}..={} ({}) closed by {} -> state {}",
            checkpoint.segment_seq,
            checkpoint.first_frame_seq,
            checkpoint.last_frame_seq,
            checkpoint.frame_count().unwrap_or_default(),
            checkpoint.close_reason.name(),
            hex(&checkpoint.state_after[..8])
        );
    }

    // Key commitment: a participant holding the delivered key confirms
    // it matches the signed header; a wrong key does not.
    anyhow::ensure!(
        mac_key_commitment(&header.nonce, &producer.key) == header.mac_key_commitment,
        "delivered key does not match the header commitment"
    );
    println!(
        "header: {} participants (owner {}), delivered MAC key matches the signed commitment",
        header.participants.len(),
        hex(&header.participants[0][..8])
    );

    verify_chain(header, &producer.checkpoints)?;
    for (checkpoint, body) in producer.checkpoints.iter().zip(producer.bodies.iter()) {
        let level = verify_segment(header, checkpoint, body, Some(&producer.key), &owner_key)?;
        anyhow::ensure!(level == VerifyLevel::MacAuthenticated, "expected MAC level");
    }
    println!("chain: all checkpoints verify (signature, linkage, MAC-authenticated roots)");

    // Scenario 1 — late joiner: load the latest checkpoint's snapshot,
    // verify it against state_after, and confirm it equals the live doc.
    let latest = producer
        .checkpoints
        .last()
        .ok_or_else(|| anyhow::anyhow!("no checkpoints"))?;
    let latest_body = producer
        .bodies
        .last()
        .ok_or_else(|| anyhow::anyhow!("no bodies"))?;
    let level = verify_segment(header, latest, latest_body, None, &owner_key)?;
    anyhow::ensure!(
        latest_body.state_snapshot == producer.doc && level == VerifyLevel::ReplayVerifiedKeyless,
        "late joiner reconstruction mismatch"
    );
    println!(
        "scenario 1 (late joiner): checkpoint {} snapshot reconstructs the live document ({} bytes, keyless replay-verified)",
        latest.segment_seq,
        producer.doc.len()
    );

    // Scenario 2 — reconnect: a peer synced through segment 1 lost the
    // rest; it reconciles at the next checkpoint boundary and replays
    // the remaining segment bodies onto the segment-1 snapshot.
    let synced = 2; // checkpoints 0 and 1 are already held
    let peer_doc = producer.bodies[synced - 1].state_snapshot.clone();
    let reconciled = replay_from(
        peer_doc,
        &producer.checkpoints[synced..],
        &producer.bodies[synced..],
    )?;
    anyhow::ensure!(
        reconciled == producer.doc,
        "reconnect reconciliation mismatch"
    );
    println!(
        "scenario 2 (reconnect): peer holding segments 0..1 reconciled at the segment-2 boundary and replayed to the live document"
    );

    // Scenario 3 — truncated tail: drop the last three frames of the
    // final segment body; the verifier must reject it, distinguishing
    // an absent tail from a verified history.
    let mut truncated = latest_body.clone();
    truncated.frames.truncate(truncated.frames.len() - 3);
    let detection = verify_segment(header, latest, &truncated, Some(&producer.key), &owner_key);
    anyhow::ensure!(
        detection.is_err(),
        "verifier accepted a truncated segment tail"
    );
    println!(
        "scenario 3 (truncated tail): verifier rejected the shortened body ({})",
        detection
            .err()
            .map_or_else(|| "unknown".to_string(), |e| e.to_string())
    );

    // Tamper check: flipping one MAC byte must also fail verification.
    let mut tampered = latest_body.clone();
    if let Some(first) = tampered.frames.first_mut() {
        first.1[0] ^= 0xff;
    }
    anyhow::ensure!(
        verify_segment(header, latest, &tampered, Some(&producer.key), &owner_key).is_err(),
        "verifier accepted a tampered frame MAC"
    );
    println!("tamper check: a flipped frame MAC is rejected");

    // Measurements — intermittent signatures versus per-event signing.
    let checkpoint_count = producer.checkpoints.len() as u64;
    let baseline_sigs = total_frames;
    let baseline_bytes = total_frames * 64;
    let poc_sig_bytes = checkpoint_count * 64;
    let poc_mac_bytes = total_frames * 32;
    println!("measurements over {total_frames} frames:");
    println!("  per-event signing: {baseline_sigs} signatures, {baseline_bytes} signature bytes");
    println!(
        "  this PoC: {checkpoint_count} checkpoint signatures ({poc_sig_bytes} bytes) + {total_frames} frame MACs ({poc_mac_bytes} bytes) = {} auth bytes",
        poc_sig_bytes + poc_mac_bytes
    );
    println!(
        "  signatures reduced {:.1}% ; auth bytes reduced {:.1}%",
        100.0 * (1.0 - checkpoint_count as f64 / baseline_sigs as f64),
        100.0 * (1.0 - (poc_sig_bytes + poc_mac_bytes) as f64 / baseline_bytes as f64)
    );
    println!("ALL DEMOS PASSED");
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn crypto_known_answers() {
        self_check_crypto().unwrap();
    }

    #[test]
    fn session_chain_verifies_and_truncation_is_detected() {
        let producer = build_session().unwrap();
        let owner_key = producer.signer.verifying_key();
        verify_chain(&producer.header, &producer.checkpoints).unwrap();
        assert_eq!(producer.next_seq, 700);
        assert!(producer.checkpoints.len() >= 4);
        for (checkpoint, body) in producer.checkpoints.iter().zip(producer.bodies.iter()) {
            let level = verify_segment(
                &producer.header,
                checkpoint,
                body,
                Some(&producer.key),
                &owner_key,
            )
            .unwrap();
            assert_eq!(level, VerifyLevel::MacAuthenticated);
        }
        let latest = producer.checkpoints.last().unwrap();
        let mut truncated = producer.bodies.last().unwrap().clone();
        truncated.frames.pop();
        assert!(
            verify_segment(
                &producer.header,
                latest,
                &truncated,
                Some(&producer.key),
                &owner_key
            )
            .is_err()
        );
    }

    #[test]
    fn idle_segment_checkpoints_on_tick_at_time_boundary() {
        let owner = SigningKey::from_bytes(&[7u8; 32]);
        let peer = SigningKey::from_bytes(&[9u8; 32]).verifying_key();
        let mut producer = Producer::new(&owner, &peer);
        producer.push_frame(Op::Append(b'a'), 0, None).unwrap();
        producer.push_frame(Op::Append(b'b'), 1000, None).unwrap();
        assert!(producer.checkpoints.is_empty());
        // A tick before the 2 s boundary changes nothing.
        producer.tick(1999).unwrap();
        assert!(producer.checkpoints.is_empty());
        // At the boundary the idle segment checkpoints with no new frame.
        producer.tick(2000).unwrap();
        assert_eq!(producer.checkpoints.len(), 1);
        let checkpoint = &producer.checkpoints[0];
        assert_eq!(checkpoint.close_reason, CloseReason::Interval);
        assert_eq!(
            (checkpoint.first_frame_seq, checkpoint.last_frame_seq),
            (0, 1)
        );
        // A later frame starts a fresh segment; the chain still verifies.
        producer.push_frame(Op::Append(b'c'), 6000, None).unwrap();
        producer.finish().unwrap();
        assert_eq!(producer.checkpoints.len(), 2);
        verify_chain(&producer.header, &producer.checkpoints).unwrap();
    }

    #[test]
    fn keyless_verification_rejects_snapshot_the_frames_do_not_produce() {
        let producer = build_session().unwrap();
        let owner_key = producer.signer.verifying_key();
        let latest = producer.checkpoints.last().unwrap();
        let latest_body = producer.bodies.last().unwrap();
        // Genuine body: keyless verification passes and is replay-verified.
        let level =
            verify_segment(&producer.header, latest, latest_body, None, &owner_key).unwrap();
        assert_eq!(level, VerifyLevel::ReplayVerifiedKeyless);
        // Forged: the owner signs a checkpoint whose state_after commits
        // a snapshot unrelated to the segment frames. The snapshot hash
        // matches the signed state_after, so the old snapshot-only
        // check passed; replaying the frames must reject it.
        let mut forged = latest.clone();
        forged.state_after = state_hash("a snapshot the frames never produce");
        let core = forged.core_bytes(&producer.header.session_id);
        forged.id = blake(&core);
        forged.signature = producer.signer.sign(&core).to_bytes();
        let mut forged_body = latest_body.clone();
        forged_body.state_snapshot = "a snapshot the frames never produce".to_string();
        assert!(
            verify_segment(&producer.header, &forged, &forged_body, None, &owner_key).is_err(),
            "keyless verification accepted a snapshot its frames do not replay to"
        );
    }

    #[test]
    fn tampered_header_participant_is_rejected() {
        let producer = build_session().unwrap();
        let owner_key = producer.signer.verifying_key();
        let mut header = producer.header.clone();
        header.participants[1] = SigningKey::from_bytes(&[11u8; 32])
            .verifying_key()
            .to_bytes();
        let latest = producer.checkpoints.last().unwrap();
        let latest_body = producer.bodies.last().unwrap();
        assert!(
            verify_segment(
                &header,
                latest,
                latest_body,
                Some(&producer.key),
                &owner_key
            )
            .is_err(),
            "segment verification accepted a header whose participant list does not recompute to the session ID"
        );
        assert!(
            verify_chain(&header, &producer.checkpoints).is_err(),
            "chain verification accepted a tampered header"
        );
    }

    #[test]
    fn reversed_or_overflowing_frame_range_is_rejected() {
        let producer = build_session().unwrap();
        let owner_key = producer.signer.verifying_key();
        let latest = producer.checkpoints.last().unwrap();
        let latest_body = producer.bodies.last().unwrap();
        // Reversed range: the old `last - first + 1` panicked in debug
        // builds instead of returning a verification error.
        let mut reversed = latest.clone();
        reversed.first_frame_seq = latest.last_frame_seq + 1;
        let core = reversed.core_bytes(&producer.header.session_id);
        reversed.id = blake(&core);
        reversed.signature = producer.signer.sign(&core).to_bytes();
        assert!(reversed.frame_count().is_none());
        assert!(
            verify_segment(&producer.header, &reversed, latest_body, None, &owner_key).is_err(),
            "verification accepted a reversed frame range"
        );
        // Overflowing range: `0..=u64::MAX` wrapped to a count of zero
        // in release builds, matching an empty body.
        let mut overflowing = latest.clone();
        overflowing.first_frame_seq = 0;
        overflowing.last_frame_seq = u64::MAX;
        let core = overflowing.core_bytes(&producer.header.session_id);
        overflowing.id = blake(&core);
        overflowing.signature = producer.signer.sign(&core).to_bytes();
        assert!(overflowing.frame_count().is_none());
        let mut empty_body = latest_body.clone();
        empty_body.frames.clear();
        assert!(
            verify_segment(
                &producer.header,
                &overflowing,
                &empty_body,
                None,
                &owner_key
            )
            .is_err(),
            "verification accepted an overflowing frame range with an empty body"
        );
        // Chain advancement past u64::MAX is an error, not a wrap.
        let mut chain = producer.checkpoints.clone();
        chain.last_mut().unwrap().last_frame_seq = u64::MAX;
        assert!(
            verify_chain(&producer.header, &chain).is_err(),
            "chain verification accepted a frame sequence that overflows"
        );
    }

    #[test]
    fn late_joiner_snapshot_matches_live_doc() {
        let producer = build_session().unwrap();
        let latest_body = producer.bodies.last().unwrap();
        assert_eq!(latest_body.state_snapshot, producer.doc);
        assert_eq!(
            state_hash(&latest_body.state_snapshot),
            producer.checkpoints.last().unwrap().state_after
        );
    }
}
