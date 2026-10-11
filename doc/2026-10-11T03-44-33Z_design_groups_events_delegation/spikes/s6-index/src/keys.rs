//! The claim, its stored body, and the alpha and omega keys both stores index.
//!
//! Key fields follow the tuple rules of `pkgs/id/src/tuple.rs`. A variable-length
//! field is a tag, its bytes with 0x00 escaped as 0x00 0xFF, and a 0x00 terminator.
//! Byte order then equals tuple order. A fixed 32-byte ID that follows a variable
//! field gets the tag 0x03, so the byte after any terminator is below 0xFF.
//!
//! Prefix scans have one trap. A prefix that ends with a terminator also matches
//! the escaped form of a longer value: name "a" is `02 61 00`, and name "a\0" is
//! `02 61 00 FF 00`. Such a scan must stop before `prefix + 0xFF`; see `omega_bounds`.
//! Fixed-width prefixes (a subject, an author) use the ordinary successor.

use anyhow::{Result, bail, ensure};

/// A 32-byte subject, author, or claim ID.
pub type Id = [u8; 32];

const NULL: u8 = 0x00;
const BYTES: u8 = 0x01;
const STRING: u8 = 0x02;
const FIXED: u8 = 0x03;
const ESCAPE: u8 = 0xFF;

/// One signed claim: `subject` carries `name` = `value` (or null), written as the
/// `seq`-th claim of `author`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Claim {
    pub author: Id,
    pub seq: u64,
    pub subject: Id,
    pub name: Vec<u8>,
    pub value: Option<Vec<u8>>,
}

impl Claim {
    /// The stored body. Fixed fields come first, then length-prefixed name and value.
    pub fn body(&self) -> Vec<u8> {
        assert!(self.name.len() <= 255, "name longer than 255 bytes");
        let value_len = self.value.as_ref().map_or(0, Vec::len);
        assert!(value_len <= 65535, "value longer than 65535 bytes");
        let mut out = Vec::with_capacity(74 + self.name.len() + value_len);
        out.extend_from_slice(&self.author);
        out.extend_from_slice(&self.seq.to_be_bytes());
        out.extend_from_slice(&self.subject);
        out.push(self.name.len() as u8);
        out.extend_from_slice(&self.name);
        match &self.value {
            None => out.push(0),
            Some(v) => {
                out.push(1);
                out.extend_from_slice(&(v.len() as u16).to_be_bytes());
                out.extend_from_slice(v);
            }
        }
        out
    }

    pub fn from_body(b: &[u8]) -> Result<Claim> {
        ensure!(b.len() >= 74, "claim body too short");
        let author: Id = b[0..32].try_into()?;
        let seq = u64::from_be_bytes(b[32..40].try_into()?);
        let subject: Id = b[40..72].try_into()?;
        let name_len = b[72] as usize;
        let mut i = 73;
        ensure!(b.len() > i + name_len, "claim body truncated in name");
        let name = b[i..i + name_len].to_vec();
        i += name_len;
        let value = match b[i] {
            0 => {
                i += 1;
                None
            }
            1 => {
                ensure!(b.len() >= i + 3, "claim body truncated in value length");
                let len = u16::from_be_bytes([b[i + 1], b[i + 2]]) as usize;
                ensure!(b.len() >= i + 3 + len, "claim body truncated in value");
                let v = b[i + 3..i + 3 + len].to_vec();
                i += 3 + len;
                Some(v)
            }
            tag => bail!("bad value tag {tag}"),
        };
        ensure!(i == b.len(), "claim body has trailing bytes");
        Ok(Claim {
            author,
            seq,
            subject,
            name,
            value,
        })
    }

    /// Claim ID: BLAKE3 over the stored body.
    pub fn id(&self) -> Id {
        *blake3::hash(&self.body()).as_bytes()
    }
}

/// Appends `tag`, the escaped bytes, and the terminator.
fn put_escaped(out: &mut Vec<u8>, tag: u8, bytes: &[u8]) {
    out.push(tag);
    for &b in bytes {
        out.push(b);
        if b == NULL {
            out.push(ESCAPE);
        }
    }
    out.push(NULL);
}

fn put_name(out: &mut Vec<u8>, name: &[u8]) {
    put_escaped(out, STRING, name);
}

/// Null is a single byte, below every present value.
fn put_value(out: &mut Vec<u8>, value: Option<&[u8]>) {
    match value {
        None => out.push(NULL),
        Some(v) => put_escaped(out, BYTES, v),
    }
}

/// Alpha key: (subject, name, value or null, claim ID).
pub fn alpha_key(c: &Claim, id: &Id) -> Vec<u8> {
    let mut out = Vec::with_capacity(96 + c.name.len());
    out.extend_from_slice(&c.subject);
    put_name(&mut out, &c.name);
    put_value(&mut out, c.value.as_deref());
    out.push(FIXED);
    out.extend_from_slice(id);
    out
}

/// Omega key: (value or null, name, subject, claim ID).
pub fn omega_key(c: &Claim, id: &Id) -> Vec<u8> {
    let mut out = omega_prefix(&c.name, c.value.as_deref());
    out.push(FIXED);
    out.extend_from_slice(&c.subject);
    out.extend_from_slice(id);
    out
}

/// The omega prefix for one (name, value or null) pair.
pub fn omega_prefix(name: &[u8], value: Option<&[u8]>) -> Vec<u8> {
    let mut out = Vec::new();
    put_value(&mut out, value);
    put_name(&mut out, name);
    out
}

/// Scan bounds `[lo, hi)` for the omega keys of one (name, value or null) pair.
/// The prefix ends with a terminator, so `hi` is `prefix + 0xFF`. The standard
/// successor would also admit names that continue the prefix with an escaped 0x00.
pub fn omega_bounds(name: &[u8], value: Option<&[u8]>) -> (Vec<u8>, Vec<u8>) {
    let lo = omega_prefix(name, value);
    let mut hi = lo.clone();
    hi.push(ESCAPE);
    (lo, hi)
}

/// Key for the unique (author, seq) mapping: 32-byte author, then seq big-endian.
pub fn author_seq_key(author: &Id, seq: u64) -> Vec<u8> {
    let mut out = author.to_vec();
    out.extend_from_slice(&seq.to_be_bytes());
    out
}

/// Subject of an omega key: the 32 bytes before the claim ID.
pub fn omega_subject(key: &[u8]) -> Id {
    key[key.len() - 64..key.len() - 32]
        .try_into()
        .expect("omega key holds a 32-byte subject")
}

/// Claim ID at the end of an alpha or omega key.
pub fn key_id(key: &[u8]) -> Id {
    key[key.len() - 32..]
        .try_into()
        .expect("key ends with a 32-byte claim ID")
}

/// The smallest key greater than every key that starts with `prefix`, or None when
/// the prefix is all 0xFF bytes and the scan has no upper bound.
pub fn prefix_end(prefix: &[u8]) -> Option<Vec<u8>> {
    let mut end = prefix.to_vec();
    while let Some(last) = end.pop() {
        if last != 0xFF {
            end.push(last + 1);
            return Some(end);
        }
    }
    None
}
