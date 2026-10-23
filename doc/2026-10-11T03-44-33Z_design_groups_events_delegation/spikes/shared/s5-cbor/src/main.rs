//! Prints the S5 measurements: the rejection matrix, the decode-then-re-encode counts, the golden
//! vectors, and the signature checks. Run it with `cargo run -p s5-cbor --release`.

use cbor_core::Value;
use ed25519_dalek::Verifier;
use p2panda_core::Header;
use s5_cbor::{canonical, fuzz, golden, matrix};

const SEED: u64 = 20_261_011;
const VALUES: u64 = 10_000;
const MUTANTS_PER_VALUE: u64 = 16;

fn main() {
    // The fuzz run catches decoder panics on purpose. Keep stderr quiet while it does.
    std::panic::set_hook(Box::new(|_| {}));
    matrix_section();
    fuzz_section();
    golden_section();
    signature_section();
}

fn verdict(check: &matrix::Check) -> String {
    match check {
        Ok(()) => "accept".into(),
        Err(reason) => format!("reject ({reason})"),
    }
}

fn matrix_section() {
    println!("## (b) rejection matrix");
    println!("| input | bytes | cbor-core STRICT | dcbor | re-encode (LENIENT) | wrapper |");
    println!("|---|---|---|---|---|---|");
    for row in matrix::rows() {
        let out = matrix::evaluate(&row.input);
        println!(
            "| {} | `{}` | {} | {} | {} | {} |",
            row.name,
            hex::encode(&row.input),
            verdict(&out.strict),
            verdict(&out.dcbor),
            verdict(&out.reencode),
            verdict(&out.wrapper),
        );
        if !out.normalized.is_empty() && out.normalized != row.input {
            println!(
                "|  normalized to | `{}` | | | | |",
                hex::encode(&out.normalized)
            );
        }
    }
}

fn fuzz_section() {
    println!(
        "## (c) decode-then-re-encode, {VALUES} values x {MUTANTS_PER_VALUE} mutants, seed {SEED}"
    );
    let stats = fuzz::run(SEED, VALUES, MUTANTS_PER_VALUE);
    println!("{stats:#?}");
}

fn golden_section() {
    println!("## (d) golden vectors");
    for vector in golden::vectors() {
        println!(
            "- {}: cbor-core `{}` ({} bytes); dcbor agrees: {}",
            vector.name,
            hex::encode(&vector.cbor_core),
            vector.cbor_core.len(),
            vector.agree(),
        );
        if !vector.agree() {
            println!("  dcbor: `{}`", hex::encode(&vector.dcbor));
        }
    }
    let header = golden::header_vector();
    println!(
        "- p2panda header (seq 0, body, fixed key), {} bytes:",
        header.bytes.len()
    );
    println!("  header: `{}`", hex::encode(&header.bytes));
    println!("  signed: `{}`", hex::encode(&header.signed));
    println!("  public key: `{}`", hex::encode(header.key));
    println!(
        "  operation id (BLAKE3 of header): `{}`",
        hex::encode(header.operation_id)
    );
    println!("  signature: `{}`", hex::encode(header.signature));
    println!(
        "  dcbor header agrees: {}",
        header.dcbor_header == header.bytes
    );
    println!(
        "  dcbor signed part agrees: {}",
        header.dcbor_signed == header.signed
    );
    println!(
        "  p2panda decodes it: {}",
        Header::<()>::decode(&header.bytes).is_ok()
    );
    let key = canonical::admit_key(&header.key).expect("the golden key is admitted");
    let verdict = canonical::verify(&key, &header.signed, &header.signature)
        .map_err(|err| format!("{err:?}"));
    println!("  verify_strict over signed bytes: {verdict:?}");
}

fn signature_section() {
    println!("## (e) signature forgeries");
    let golden = golden::header_vector();
    let hash = golden.header.payload_hash.expect("body is non-empty");
    let size = golden.body.len() as u64;
    for forgery in golden::forgeries(&golden) {
        let admitted = canonical::admit_key(&forgery.key).is_some();
        // Plain verification, without admission, to show what each check adds.
        let plain =
            ed25519_dalek::VerifyingKey::from_bytes(&forgery.key).map_err(|err| format!("{err:?}"));
        let signature = ed25519_dalek::Signature::from_bytes(&forgery.signature);
        let strict = plain.as_ref().map_err(Clone::clone).and_then(|key| {
            canonical::verify(key, &golden.signed, &forgery.signature)
                .map_err(|err| format!("{err:?}"))
        });
        let non_strict = plain.as_ref().map_err(Clone::clone).and_then(|key| {
            key.verify(&golden.signed, &signature)
                .map_err(|err| format!("{err:?}"))
        });
        let forged = canonical::encode(&Value::from(vec![
            Value::Unsigned(1),
            Value::from(forgery.key.to_vec()),
            Value::from(forgery.signature.to_vec()),
            Value::Unsigned(size),
            Value::from(hash.as_bytes().to_vec()),
            Value::Unsigned(0),
        ]));
        let p2panda = Header::<()>::decode(&forged)
            .map(drop)
            .map_err(|err| format!("{err:?}"));
        println!("- {}", forgery.name);
        println!("  admitted as a key: {admitted}");
        println!("  verify_strict: {strict:?}");
        println!("  non-strict verify: {non_strict:?}");
        println!("  p2panda Header::decode: {p2panda:?}");
    }
}
