//! S12: welcome size as epochs accumulate. Method and results are in RESULTS.md.
//!
//! Runs one scenario on three builds of p2panda-encryption: stock, the serde_bytes variant, and the
//! S3 path copy as found. Prints one table per build, then cross-decodes two welcomes between the
//! stock and serde_bytes builds.
#![allow(dead_code, unused_imports)]

mod stock {
    use p2panda_encryption as enc;

    type Dgm = enc::data_scheme::test_utils::dgm::TestDgm<
        enc::test_utils::MemberId,
        enc::test_utils::MessageId,
    >;

    include!("harness.rs");
}

mod sb {
    use p2panda_encryption_sb as enc;

    type Dgm = enc::data_scheme::test_utils::dgm::TestDgm<
        enc::test_utils::MemberId,
        enc::test_utils::MessageId,
    >;

    include!("harness.rs");
}

mod found {
    use p2panda_encryption_found as enc;

    type Dgm = enc::data_scheme::test_utils::dgm::TestDgm<
        enc::test_utils::MemberId,
        enc::test_utils::MessageId,
    >;

    include!("harness.rs");
}

fn main() {
    let seed = std::env::args()
        .skip_while(|arg| arg != "--seed")
        .nth(1)
        .map(|value| value.parse::<u8>().expect("seed is a byte"))
        .unwrap_or(7);

    let (stock_k1, stock_readd) = stock::run("stock", seed);
    let (sb_k1, sb_readd) = sb::run("serde-bytes", seed);
    found::run("found-as-is", seed);

    println!("## cross-decode (encoded by one build, decoded by the other)");
    println!(
        "K=1 welcome, stock -> serde-bytes: {:?}",
        sb::decode_welcome(&stock_k1)
    );
    println!(
        "K=1 welcome, serde-bytes -> stock: {:?}",
        stock::decode_welcome(&sb_k1)
    );
    println!(
        "re-add welcome, stock -> serde-bytes: {:?}",
        sb::decode_welcome(&stock_readd)
    );
    println!(
        "re-add welcome, serde-bytes -> stock: {:?}",
        stock::decode_welcome(&sb_readd)
    );
}
