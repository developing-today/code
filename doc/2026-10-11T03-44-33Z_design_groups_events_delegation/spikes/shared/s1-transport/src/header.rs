//! Event header on p2panda-core's 0-based header, with the design's extension
//! fields, and our own check that works from received bytes only.

use cbor_core::{DecodeOptions, Strictness, Value};
use p2panda_core::{Hash, Header, SigningKey};
use serde::{Deserialize, Serialize};

/// `MAX_AFTER` from the design (§4.3).
pub const MAX_AFTER: usize = 32;

/// The extension fields the design adds to p2panda's header.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChainExt {
    pub kind: String,
    pub after: Vec<Hash>,
    pub group: Option<Hash>,
    pub domain: String,
}

/// Same fields as [`ChainExt`] plus one extra key, used to show that p2panda's typed
/// decode ignores unknown extension keys.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChainExtPlus {
    pub kind: String,
    pub after: Vec<Hash>,
    pub group: Option<Hash>,
    pub domain: String,
    pub extra: u8,
}

/// A signed event header with our extension.
pub type Event = Header<ChainExt>;

/// Builds and signs an event with p2panda's builder.
pub fn build_event(
    key: &SigningKey,
    seq: u32,
    backlink: Option<Hash>,
    ext: ChainExt,
    body: Option<&[u8]>,
) -> Event {
    let mut builder = Header::<ChainExt>::builder()
        .seq_num(seq)
        .backlink(backlink);
    if let Some(body) = body {
        builder = builder.body(body);
    }
    builder.build(key, ext)
}

/// What our own check learned from received bytes.
#[derive(Debug)]
pub struct Checked {
    /// BLAKE3 over the received bytes. The event ID, recomputed on our side.
    pub id: Hash,
    pub author: [u8; 32],
    pub seq: u32,
    pub after_len: usize,
}

/// Verifies received header bytes without calling p2panda's `Header::verify`.
///
/// 1. Strict CBOR decode. The re-encoded AST must equal the input (canonical form).
/// 2. The field layout follows payload_size (hash iff > 0) and seq (backlink iff > 0).
/// 3. The extension must survive a typed decode and re-encode unchanged, so unknown or
///    non-canonical keys are rejected. `after` must hold at most `MAX_AFTER` hashes.
/// 4. The signed bytes are the array without the signature element, re-encoded from the
///    received AST. `verify_strict` checks them with the key and signature from the bytes.
pub fn check_received(bytes: &[u8]) -> Result<Checked, String> {
    let value = DecodeOptions::new()
        .strictness(Strictness::STRICT)
        .recursion_limit(64)
        .length_limit(512)
        .oom_mitigation(64)
        .decode(bytes)
        .map_err(|e| format!("strict cbor decode: {e:?}"))?;
    if value.encode() != bytes {
        return Err("non-canonical CBOR".into());
    }

    let mut items = value
        .into_array()
        .map_err(|e| format!("not an array: {e:?}"))?;
    if items.len() < 6 {
        return Err(format!("too few fields: {}", items.len()));
    }

    let version = u16::try_from(&items[0]).map_err(|e| format!("version: {e:?}"))?;
    if version != 1 {
        return Err(format!("unsupported version {version}"));
    }
    let key_bytes: [u8; 32] = items[1]
        .as_bytes()
        .map_err(|e| format!("key: {e:?}"))?
        .try_into()
        .map_err(|_| "key length".to_string())?;
    let sig_bytes: [u8; 64] = items[2]
        .as_bytes()
        .map_err(|e| format!("sig: {e:?}"))?
        .try_into()
        .map_err(|_| "signature length".to_string())?;
    let payload_size = u32::try_from(&items[3]).map_err(|e| format!("payload_size: {e:?}"))?;

    let mut idx = 4;
    if payload_size > 0 {
        idx += 1;
    }
    let seq = u32::try_from(&items[idx]).map_err(|e| format!("seq: {e:?}"))?;
    idx += 1;
    if seq > 0 {
        idx += 1;
    }
    if idx + 1 != items.len() {
        return Err(format!("field count {} does not match layout", items.len()));
    }

    let ext_value = &items[idx];
    let typed: ChainExt = ext_value
        .deserialized()
        .map_err(|e| format!("extension: {e:?}"))?;
    let ext_again = Value::serialized(&typed)
        .map_err(|e| format!("re-encode extension: {e:?}"))?
        .encode();
    if ext_again != ext_value.encode() {
        return Err("extension has unknown or non-canonical fields".into());
    }
    if typed.after.len() > MAX_AFTER {
        return Err(format!(
            "after has {} hashes, max {MAX_AFTER}",
            typed.after.len()
        ));
    }
    let after_len = typed.after.len();

    items.remove(2);
    let signed = Value::from(items).encode();

    let key = ed25519_dalek::VerifyingKey::from_bytes(&key_bytes)
        .map_err(|e| format!("verifying key: {e}"))?;
    let sig = ed25519_dalek::Signature::from_bytes(&sig_bytes);
    key.verify_strict(&signed, &sig)
        .map_err(|e| format!("verify_strict: {e}"))?;

    Ok(Checked {
        id: Hash::digest(bytes),
        author: key_bytes,
        seq,
        after_len,
    })
}
