//! Wire compatibility of ed25519-dalek 2.2.0 (p2panda-core's pin) and 3.0.0 (pkgs/id, iroh-base).
//! Both versions are linked into one binary. A value crosses between them only as a byte array.
//! Prints one line per check, then a summary. Research code, not product code.

mod rfc_vectors;

use curve25519_dalek_v4::{constants, scalar::Scalar};
use ed25519_dalek_v2 as v2;
use ed25519_dalek_v3 as v3;
use sha2::{Digest, Sha512};

#[derive(Default)]
struct Report {
    pass: usize,
    fail: usize,
    lines: Vec<String>,
}

impl Report {
    fn check(&mut self, name: &str, ok: bool, detail: impl Into<String>) {
        if ok {
            self.pass += 1
        } else {
            self.fail += 1
        }
        self.lines
            .push(format!("{} {name}: {}", if ok { "PASS" } else { "FAIL" }, detail.into()));
    }

    /// Observations that are not pass/fail (printed, not counted).
    fn info(&mut self, name: &str, detail: impl Into<String>) {
        self.lines.push(format!("INFO {name}: {}", detail.into()));
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex<const N: usize>(s: &str) -> [u8; N] {
    hex::decode(s).expect("hex").try_into().expect("length")
}

/// splitmix64: deterministic seeds and messages without an RNG crate.
struct Mix(u64);

impl Mix {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn bytes<const N: usize>(&mut self) -> [u8; N] {
        let mut out = [0u8; N];
        for chunk in out.chunks_mut(8) {
            let r = self.next().to_le_bytes();
            chunk.copy_from_slice(&r[..chunk.len()]);
        }
        out
    }
}

/// Little-endian 32-byte add (a + b). Panics on carry out, which the callers never hit.
fn add_le(a: &[u8; 32], b: &[u8; 32]) -> [u8; 32] {
    let mut out = [0u8; 32];
    let mut carry = 0u16;
    for i in 0..32 {
        let s = a[i] as u16 + b[i] as u16 + carry;
        out[i] = s as u8;
        carry = s >> 8;
    }
    assert_eq!(carry, 0);
    out
}

/// Little-endian 32-byte value a + d (d small). Wraps at 2^256; only used far below that.
fn le_add_small(a: &[u8; 32], d: u32) -> [u8; 32] {
    let mut out = *a;
    let mut carry = d as u64;
    for byte in out.iter_mut() {
        let s = *byte as u64 + carry;
        *byte = s as u8;
        carry = s >> 8;
    }
    out
}

/// Little-endian 32-byte value a - d (d small). Only used for a >= d.
fn le_sub_small(a: &[u8; 32], d: u32) -> [u8; 32] {
    let mut out = *a;
    let mut borrow = d as i64;
    for byte in out.iter_mut() {
        let s = *byte as i64 - (borrow & 0xff);
        borrow >>= 8;
        if s < 0 {
            *byte = (s + 256) as u8;
            borrow += 1;
        } else {
            *byte = s as u8;
        }
    }
    out
}

const P_LE: &str = "edffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff7f";
/// Canonical identity point, encoded.
const IDENTITY: &str = "0100000000000000000000000000000000000000000000000000000000000000";
/// Non-canonical encodings of small-order points: y = p + 1 (identity) and y = p (order 4).
const IDENTITY_NONCANON: &str = "eeffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff7f";
const ORDER4_NONCANON: &str = P_LE;
/// L, the group order, little-endian: 2^252 + 27742317777372353535851937790883648493.
const L_LE: &str = "edd3f55c1a631258d69cf7a2def9de1400000000000000000000000000000010";

// ---- one helper pair per version; each takes and returns bytes only ----

fn sign2(seed: &[u8; 32], msg: &[u8]) -> [u8; 64] {
    use v2::Signer;
    v2::SigningKey::from_bytes(seed).sign(msg).to_bytes()
}

fn sign3(seed: &[u8; 32], msg: &[u8]) -> [u8; 64] {
    use v3::Signer;
    v3::SigningKey::from_bytes(seed).sign(msg).to_bytes()
}

fn pk2(seed: &[u8; 32]) -> [u8; 32] {
    v2::SigningKey::from_bytes(seed).verifying_key().to_bytes()
}

fn pk3(seed: &[u8; 32]) -> [u8; 32] {
    v3::SigningKey::from_bytes(seed).verifying_key().to_bytes()
}

/// (verify, verify_strict) under ed25519-dalek 2.2.0. A key that fails to decode counts as (false, false).
fn check2(pk: &[u8; 32], msg: &[u8], sig: &[u8; 64]) -> (bool, bool) {
    use v2::Verifier;
    match v2::VerifyingKey::from_bytes(pk) {
        Err(_) => (false, false),
        Ok(vk) => {
            let s = v2::Signature::from_bytes(sig);
            (vk.verify(msg, &s).is_ok(), vk.verify_strict(msg, &s).is_ok())
        }
    }
}

/// (verify, verify_strict) under ed25519-dalek 3.0.0.
fn check3(pk: &[u8; 32], msg: &[u8], sig: &[u8; 64]) -> (bool, bool) {
    use v3::Verifier;
    match v3::VerifyingKey::from_bytes(pk) {
        Err(_) => (false, false),
        Ok(vk) => {
            let s = v3::Signature::from_bytes(sig);
            (vk.verify(msg, &s).is_ok(), vk.verify_strict(msg, &s).is_ok())
        }
    }
}

fn torsion_encodings() -> Vec<[u8; 32]> {
    constants::EIGHT_TORSION
        .iter()
        .map(|p| p.compress().to_bytes())
        .collect()
}

// ---- checks ----

/// Keys and keypairs from the same seed; RFC 8032 section 7.1 vectors through both versions.
fn keys_and_rfc(r: &mut Report) {
    let mut mix = Mix(1);
    let seeds: Vec<[u8; 32]> = (0..256).map(|_| mix.bytes()).collect();
    let key_mismatch = seeds.iter().filter(|s| pk2(s) != pk3(s)).count();
    let kp_mismatch = seeds
        .iter()
        .filter(|s| {
            v2::SigningKey::from_bytes(s).to_keypair_bytes()
                != v3::SigningKey::from_bytes(s).to_keypair_bytes()
        })
        .count();
    r.check(
        "keys: public key from the same 32-byte seed, 256 seeds",
        key_mismatch == 0,
        format!("mismatches={key_mismatch}"),
    );
    r.check(
        "keys: 64-byte keypair (seed||pk), 256 seeds",
        kp_mismatch == 0,
        format!("mismatches={kp_mismatch}"),
    );

    for (name, sk, pk, msg, sig) in rfc_vectors::RFC8032 {
        let sk: [u8; 32] = unhex(sk);
        let pk: [u8; 32] = unhex(pk);
        let msg = hex::decode(msg).expect("hex");
        let sig: [u8; 64] = unhex(sig);
        let (s2, s3) = (sign2(&sk, &msg), sign3(&sk, &msg));
        let (v2v, v2s) = check2(&pk, &msg, &sig);
        let (v3v, v3s) = check3(&pk, &msg, &sig);
        let ok = pk2(&sk) == pk
            && pk3(&sk) == pk
            && s2 == sig
            && s3 == sig
            && v2v
            && v2s
            && v3v
            && v3s;
        r.check(
            &format!("RFC 8032 {name} ({} byte msg)", msg.len()),
            ok,
            format!(
                "pk2={} pk3={} sig2={} sig3={} v2 verify/strict={v2v}/{v2s} v3 verify/strict={v3v}/{v3s}",
                pk2(&sk) == pk,
                pk3(&sk) == pk,
                s2 == sig,
                s3 == sig
            ),
        );
    }
}

/// Signatures are deterministic: same seed and message must give the same 64 bytes in both versions.
/// Then cross-verify in both directions, verify and verify_strict.
fn deterministic_and_cross(r: &mut Report) {
    let lens = [0usize, 1, 2, 31, 32, 55, 56, 63, 64, 111, 112, 127, 128, 129, 1000];
    let mut mix = Mix(2);
    let (mut n, mut sig_diff) = (0usize, 0usize);
    let (mut v3_rejects_v2_sig, mut v3_strict_rejects_v2_sig) = (0usize, 0usize);
    let (mut v2_rejects_v3_sig, mut v2_strict_rejects_v3_sig) = (0usize, 0usize);
    for _ in 0..64 {
        let seed: [u8; 32] = mix.bytes();
        let pk = pk2(&seed);
        for &len in &lens {
            let msg: Vec<u8> = (0..len).map(|_| mix.next() as u8).collect();
            let (s2, s3) = (sign2(&seed, &msg), sign3(&seed, &msg));
            n += 1;
            sig_diff += (s2 != s3) as usize;
            let (a, b) = check3(&pk, &msg, &s2);
            v3_rejects_v2_sig += (!a) as usize;
            v3_strict_rejects_v2_sig += (!b) as usize;
            let (c, d) = check2(&pk, &msg, &s3);
            v2_rejects_v3_sig += (!c) as usize;
            v2_strict_rejects_v3_sig += (!d) as usize;
        }
    }
    r.check(
        "sign: same seed+message gives identical 64 bytes in 2.2.0 and 3.0.0",
        sig_diff == 0,
        format!("cases={n} differing={sig_diff}"),
    );
    r.check(
        "cross-verify: 2.2.0 signatures accepted by 3.0.0 (verify and verify_strict)",
        v3_rejects_v2_sig == 0 && v3_strict_rejects_v2_sig == 0,
        format!("cases={n} rejected by verify={v3_rejects_v2_sig} by verify_strict={v3_strict_rejects_v2_sig}"),
    );
    r.check(
        "cross-verify: 3.0.0 signatures accepted by 2.2.0 (verify and verify_strict)",
        v2_rejects_v3_sig == 0 && v2_strict_rejects_v3_sig == 0,
        format!("cases={n} rejected by verify={v2_rejects_v3_sig} by verify_strict={v2_strict_rejects_v3_sig}"),
    );
}

/// Tampered inputs must be rejected by both versions, in both verify and verify_strict.
fn negatives(r: &mut Report) {
    let mut mix = Mix(3);
    let mut accepted = [0usize; 4]; // [v2 verify, v2 strict, v3 verify, v3 strict]
    let mut cases = 0usize;
    for _ in 0..16 {
        let seed: [u8; 32] = mix.bytes();
        let other: [u8; 32] = mix.bytes();
        let msg: Vec<u8> = (0..40).map(|_| mix.next() as u8).collect();
        let sig = sign2(&seed, &msg);
        let pk = pk2(&seed);
        let mut tampered: Vec<([u8; 32], Vec<u8>, [u8; 64])> = Vec::new();
        let mut m2 = msg.clone();
        m2[7] ^= 0x10;
        tampered.push((pk, m2, sig)); // message bit flip
        for byte in [0usize, 31, 32, 63] {
            let mut s = sig;
            s[byte] ^= 0x01; // R or S bit flip
            tampered.push((pk, msg.clone(), s));
        }
        tampered.push((pk2(&other), msg.clone(), sig)); // wrong key
        for (p, m, s) in &tampered {
            cases += 1;
            let (a, b) = check2(p, m, s);
            let (c, d) = check3(p, m, s);
            accepted[0] += a as usize;
            accepted[1] += b as usize;
            accepted[2] += c as usize;
            accepted[3] += d as usize;
        }
    }
    r.check(
        "tampered message, R, S or key: all rejected",
        accepted.iter().all(|&x| x == 0),
        format!("cases={cases} accepted [v2 verify, v2 strict, v3 verify, v3 strict]={accepted:?}"),
    );
}

/// S and S + L encode the same scalar. RFC 8032 requires 0 <= S < L. Both versions should refuse S + L
/// under default features (the canonical check in check_scalar), and S + L is below 2^253, so the old
/// top-three-bits check would not catch it.
fn non_canonical_s(r: &mut Report) {
    let l: [u8; 32] = unhex(L_LE);
    let seed = [7u8; 32];
    let msg = b"non-canonical S test".to_vec();
    let sig = sign2(&seed, &msg);
    let pk = pk2(&seed);
    let s: [u8; 32] = sig[32..].try_into().expect("32");
    let s_plus_l = add_le(&s, &l);
    let mut malleated = sig;
    malleated[32..].copy_from_slice(&s_plus_l);
    let mut high_bit = sig;
    high_bit[63] |= 0x80; // S >= 2^255: rejected by the old top-bits check too
    let control = (check2(&pk, &msg, &sig), check3(&pk, &msg, &sig));
    let mal = (check2(&pk, &msg, &malleated), check3(&pk, &msg, &malleated));
    let hib = (check2(&pk, &msg, &high_bit), check3(&pk, &msg, &high_bit));
    r.check(
        "non-canonical S: control (canonical S) accepted by all four paths",
        control.0 == (true, true) && control.1 == (true, true),
        format!("v2={:?} v3={:?}", control.0, control.1),
    );
    r.check(
        "non-canonical S+L: rejected by v2 verify and verify_strict and v3 verify and verify_strict",
        mal.0 == (false, false) && mal.1 == (false, false),
        format!(
            "v2={:?} v3={:?} top byte of S+L={:#04x} (old top-3-bits check would pass)",
            mal.0, mal.1, s_plus_l[31]
        ),
    );
    r.check(
        "S with bit 255 set: rejected by all four paths",
        hib.0 == (false, false) && hib.1 == (false, false),
        format!("v2={:?} v3={:?}", hib.0, hib.1),
    );
}

/// Small-order public keys and R values. For each small-order key, try all 8 torsion R values with S = 0.
/// verify_strict must reject every one (it rejects small-order A and R). verify (cofactorless, byte compare)
/// is expected to accept only the forgeries where [k]A is the identity, which means A is the identity.
fn small_order(r: &mut Report) {
    let mut keys: Vec<(String, [u8; 32])> = Vec::new();
    for (i, e) in torsion_encodings().into_iter().enumerate() {
        keys.push((format!("EIGHT_TORSION[{i}]"), e));
    }
    keys.push(("identity, non-canonical (y=p+1)".into(), unhex(IDENTITY_NONCANON)));
    keys.push(("order-4 point, non-canonical (y=p)".into(), unhex(ORDER4_NONCANON)));
    let torsion_r = torsion_encodings();
    let msg = b"small-order forgery probe".to_vec();

    let mut strict_accepts_any = false;
    let mut versions_differ = false;
    let mut identity_non_strict_accepts = 0usize;
    for (name, a) in &keys {
        let dec2 = v2::VerifyingKey::from_bytes(a).is_ok();
        let dec3 = v3::VerifyingKey::from_bytes(a).is_ok();
        let weak2 = v2::VerifyingKey::from_bytes(a).map(|k| k.is_weak()).unwrap_or(false);
        let weak3 = v3::VerifyingKey::from_bytes(a).map(|k| k.is_weak()).unwrap_or(false);
        let mut acc = [0usize; 4];
        for t in &torsion_r {
            let mut sig = [0u8; 64];
            sig[..32].copy_from_slice(t); // R = torsion point, S = 0
            let (x, y) = check2(a, &msg, &sig);
            let (z, w) = check3(a, &msg, &sig);
            acc[0] += x as usize;
            acc[1] += y as usize;
            acc[2] += z as usize;
            acc[3] += w as usize;
        }
        strict_accepts_any |= acc[1] > 0 || acc[3] > 0;
        versions_differ |= acc[0] != acc[2] || acc[1] != acc[3] || dec2 != dec3 || weak2 != weak3;
        if name.starts_with("EIGHT_TORSION") && a == &unhex::<32>(IDENTITY) {
            identity_non_strict_accepts += acc[0];
        }
        r.info(
            &format!("small-order key {name}"),
            format!(
                "decodes v2={dec2} v3={dec3} is_weak v2={weak2} v3={weak3} | of 8 torsion R with S=0 accepted: v2 verify={} v2 strict={} v3 verify={} v3 strict={} | key={}",
                acc[0], acc[1], acc[2], acc[3], hex(a)
            ),
        );
    }
    r.check(
        "small-order keys: verify_strict accepts no torsion-R forgery (2.2.0 and 3.0.0)",
        !strict_accepts_any,
        "see the per-key lines above",
    );
    r.check(
        "small-order keys: 2.2.0 and 3.0.0 give identical decode, is_weak and verify counts",
        !versions_differ,
        "per-key counts above",
    );
    r.check(
        "identity key: plain verify accepts a torsion-R forgery (2.2.0), as predicted",
        identity_non_strict_accepts > 0,
        format!("accepted={identity_non_strict_accepts}"),
    );

    // Identity public key, R = [s]B, S = s: valid under plain verify for any message, without a secret key.
    let ident: [u8; 32] = unhex(IDENTITY);
    let mut universal = [0usize; 4];
    let mut trials = 0usize;
    for n in [1u64, 2, 12345, 0xdead_beef] {
        let s = Scalar::from(n);
        let mut sig = [0u8; 64];
        sig[..32].copy_from_slice(&(s * constants::ED25519_BASEPOINT_POINT).compress().to_bytes());
        sig[32..].copy_from_slice(&s.to_bytes());
        for m in [b"any message".as_slice(), b"another message, another length".as_slice()] {
            trials += 1;
            let (a, b) = check2(&ident, m, &sig);
            let (c, d) = check3(&ident, m, &sig);
            universal[0] += a as usize;
            universal[1] += b as usize;
            universal[2] += c as usize;
            universal[3] += d as usize;
        }
    }
    r.check(
        "identity key: forgery R=[s]B, S=s accepted by verify, rejected by verify_strict (both versions)",
        universal[0] == trials && universal[2] == trials && universal[1] == 0 && universal[3] == 0,
        format!("trials={trials} accepted [v2 verify, v2 strict, v3 verify, v3 strict]={universal:?}"),
    );

    // Control: a real key with R = identity and S = 0 must still be rejected.
    let seed = [9u8; 32];
    let real = pk2(&seed);
    let mut sig = [0u8; 64];
    sig[..32].copy_from_slice(&unhex::<32>(IDENTITY));
    let (a, b) = check2(&real, &msg, &sig);
    let (c, d) = check3(&real, &msg, &sig);
    r.check(
        "control: non-small-order key with R=identity, S=0 rejected by all four paths",
        !a && !b && !c && !d,
        format!("v2={a}/{b} v3={c}/{d}"),
    );
}

/// Seed vs expanded key vs keypair vs PKCS#8. SHA-512(seed) is computed here by hand, then fed to hazmat.
fn expanded_and_formats(r: &mut Report) {
    let mut mix = Mix(6);
    let (mut hz_bad, mut kp_bad, mut der_bad, mut trials) = (0usize, 0usize, 0usize, 0usize);
    for _ in 0..32 {
        trials += 1;
        let seed: [u8; 32] = mix.bytes();
        let msg: Vec<u8> = (0..50).map(|_| mix.next() as u8).collect();
        let expanded: [u8; 64] = Sha512::digest(seed).into();

        let esk2 = v2::hazmat::ExpandedSecretKey::from_bytes(&expanded);
        let vk2 = v2::VerifyingKey::from(&esk2);
        let hz2 = v2::hazmat::raw_sign::<v2::Sha512>(&esk2, &msg, &vk2).to_bytes();
        let esk3 = v3::hazmat::ExpandedSecretKey::from_bytes(&expanded);
        let vk3 = v3::VerifyingKey::from(&esk3);
        let hz3 = v3::hazmat::raw_sign::<v3::Sha512>(&esk3, &msg, &vk3).to_bytes();
        let same = hz2 == sign2(&seed, &msg)
            && hz3 == sign3(&seed, &msg)
            && vk2.to_bytes() == pk2(&seed)
            && vk3.to_bytes() == pk3(&seed);
        hz_bad += (!same) as usize;

        let kp2 = v2::SigningKey::from_bytes(&seed).to_keypair_bytes();
        let kp_ok = matches!(v3::SigningKey::from_keypair_bytes(&kp2), Ok(k) if k.to_bytes() == seed);
        let mut tampered = kp2;
        tampered[63] ^= 1;
        let kp_tamper_rejected = v2::SigningKey::from_keypair_bytes(&tampered).is_err()
            && v3::SigningKey::from_keypair_bytes(&tampered).is_err();
        kp_bad += (!(kp_ok && kp_tamper_rejected)) as usize;

        use v2::pkcs8::EncodePrivateKey as _;
        let der2 = v2::SigningKey::from_bytes(&seed).to_pkcs8_der().expect("der2");
        use v3::pkcs8::EncodePrivateKey as _;
        let der3 = v3::SigningKey::from_bytes(&seed).to_pkcs8_der().expect("der3");
        use v3::pkcs8::DecodePrivateKey as _;
        let back = v3::SigningKey::from_pkcs8_der(der2.as_bytes()).map(|k| k.to_bytes() == seed);
        der_bad += (!(der2.as_bytes() == der3.as_bytes() && back == Ok(true))) as usize;
        if trials == 1 {
            let same = der2.as_bytes() == der3.as_bytes();
            r.info("pkcs8 DER of seed 0", format!("2.2.0 == 3.0.0: {same}; hex={}", hex(der2.as_bytes())));
        }
    }
    r.check(
        "expanded key: SHA-512(seed) through hazmat gives the same signatures and keys as seed signing",
        hz_bad == 0,
        format!("seeds={trials} mismatches={hz_bad}"),
    );
    r.check(
        "keypair (seed||pk): 2.2.0 bytes load in 3.0.0, and a tampered public half is refused by both",
        kp_bad == 0,
        format!("seeds={trials} failures={kp_bad}"),
    );
    r.check(
        "PKCS#8 DER of the seed: byte-identical in 2.2.0 and 3.0.0, and 3.0.0 reads 2.2.0's",
        der_bad == 0,
        format!("seeds={trials} failures={der_bad}"),
    );
}

/// Which 32-byte strings decode, and is_weak, across the two versions. Covers canonical y, non-canonical y
/// (y >= p), both sign bits, and random bytes.
fn decoding_table(r: &mut Report) {
    let p: [u8; 32] = unhex(P_LE);
    let mut encodings: Vec<[u8; 32]> = Vec::new();
    for d in 0..=300u32 {
        let small = le_add_small(&[0u8; 32], d);
        encodings.push(small);
        encodings.push(le_add_small(&p, d)); // y = p + d, non-canonical for every d >= 0
        encodings.push(le_sub_small(&p, d + 1)); // y = p - (d+1), canonical
    }
    let mut mix = Mix(8);
    for _ in 0..2000 {
        encodings.push(mix.bytes());
    }
    let mut both_sign = Vec::new();
    for e in &encodings {
        both_sign.push(*e);
        let mut flipped = *e;
        flipped[31] ^= 0x80;
        both_sign.push(flipped);
    }
    let (mut total, mut ok2, mut ok3, mut decode_diff, mut weak_diff, mut weak_true) = (0usize, 0usize, 0usize, 0usize, 0usize, 0usize);
    for e in &both_sign {
        total += 1;
        let k2 = v2::VerifyingKey::from_bytes(e);
        let k3 = v3::VerifyingKey::from_bytes(e);
        ok2 += k2.is_ok() as usize;
        ok3 += k3.is_ok() as usize;
        if k2.is_ok() != k3.is_ok() {
            decode_diff += 1;
        }
        if let (Ok(a), Ok(b)) = (k2, k3) {
            if a.is_weak() != b.is_weak() {
                weak_diff += 1;
            }
            weak_true += b.is_weak() as usize;
        }
    }
    r.check(
        "decode: VerifyingKey::from_bytes accepts the same 32-byte strings in 2.2.0 and 3.0.0",
        decode_diff == 0,
        format!("encodings={total} decodes v2={ok2} v3={ok3} disagreements={decode_diff}"),
    );
    r.check(
        "decode: is_weak agrees for every decodable key",
        weak_diff == 0,
        format!("disagreements={weak_diff} small-order among decodable={weak_true}"),
    );
}

/// Error text and generate(). Display strings are printed, not compared, because they are a detail.
fn errors_and_generate(r: &mut Report) {
    let mut bad = None;
    for y in 2u32..400 {
        let e = le_add_small(&[0u8; 32], y);
        if v2::VerifyingKey::from_bytes(&e).is_err() {
            bad = Some(e);
            break;
        }
    }
    let bad = bad.expect("an off-curve y below 400");
    let e2 = v2::VerifyingKey::from_bytes(&bad).expect_err("v2 rejects");
    let e3 = v3::VerifyingKey::from_bytes(&bad).expect_err("v3 rejects");
    r.info(
        "error: off-curve public key (both versions return Err)",
        format!(
            "v2 Display={:?} Debug={:?} | v3 Display={:?} Debug={:?} | bytes={}",
            e2.to_string(),
            format!("{e2:?}"),
            e3.to_string(),
            format!("{e3:?}"),
            hex(&bad)
        ),
    );
    let s2 = v2::Signature::from_slice(&[0u8; 63]).expect_err("63 bytes");
    let s3 = v3::Signature::from_slice(&[0u8; 63]).expect_err("63 bytes");
    r.info(
        "error: 63-byte signature slice (both versions return Err)",
        format!("v2 Display={:?} | v3 Display={:?}", s2.to_string(), s3.to_string()),
    );

    let mut rng2 = rand_core_v06::OsRng;
    let sk2 = v2::SigningKey::generate(&mut rng2);
    let mut rng3 = getrandom_v04::rand_core::UnwrapErr(getrandom_v04::SysRng);
    let sk3 = v3::SigningKey::generate(&mut rng3);
    let msg = b"generated keys";
    let s2 = {
        use v2::Signer;
        sk2.sign(msg).to_bytes()
    };
    let s3 = {
        use v3::Signer;
        sk3.sign(msg).to_bytes()
    };
    let x = check3(&sk2.verifying_key().to_bytes(), msg, &s2);
    let y = check2(&sk3.verifying_key().to_bytes(), msg, &s3);
    r.check(
        "generate(): keys from OsRng (2.2.0, rand_core 0.6) and SysRng via UnwrapErr (3.0.0, rand_core 0.10) cross-verify",
        x == (true, true) && y == (true, true),
        format!("v3 verifies v2-generated key: {x:?}; v2 verifies v3-generated key: {y:?}"),
    );
}

fn main() {
    let mut r = Report::default();
    keys_and_rfc(&mut r);
    deterministic_and_cross(&mut r);
    negatives(&mut r);
    non_canonical_s(&mut r);
    small_order(&mut r);
    expanded_and_formats(&mut r);
    decoding_table(&mut r);
    errors_and_generate(&mut r);
    for line in &r.lines {
        println!("{line}");
    }
    println!("SUMMARY pass={} fail={}", r.pass, r.fail);
    if r.fail > 0 {
        std::process::exit(1);
    }
}
