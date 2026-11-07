// Probe: WriterSet check/encode/decode never verify set_id == derive_set_id(owner, scope)
use id::groups::Role;
use id::idset::{decode_writer_body, encode_writer_body, Entry, Excludes, WriterSet};
use p2panda_core::Hash;

fn owner() -> [u8; 32] { std::array::from_fn(|i| u8::try_from(i).unwrap()) }

#[test]
fn probe_idset_setid_mismatch_accepted() {
    // Control: correctly derived set_id encodes and decodes.
    let good = WriterSet {
        set_id: WriterSet::derive_set_id(&owner(), "dog-care"),
        version: 7,
        owner: owner(),
        scope: Some("dog-care".to_owned()),
        entries: vec![Entry::Public { include_history: false }],
        excludes: Excludes(vec![]),
    };
    assert!(good.check().is_ok());
    let body = encode_writer_body(&good).unwrap();
    assert_eq!(decode_writer_body(&body).unwrap(), good);

    // Mismatch 1: set_id derived for a different purpose than the body's scope.
    let wrong_purpose = WriterSet {
        set_id: WriterSet::derive_set_id(&owner(), "cat-care"),
        ..good.clone()
    };
    println!("wrong_purpose check: {:?}", wrong_purpose.check());
    println!("wrong_purpose encode: {:?}", encode_writer_body(&wrong_purpose).map(|b| b.len()));
    if let Ok(body) = encode_writer_body(&wrong_purpose) {
        println!("wrong_purpose decode: {:?}", decode_writer_body(&body).map(|s| s.set_id == wrong_purpose.set_id));
    }

    // Mismatch 2: set_id derived for a different owner.
    let wrong_owner = WriterSet {
        set_id: WriterSet::derive_set_id(&[1u8; 32], "dog-care"),
        ..good.clone()
    };
    println!("wrong_owner check: {:?}", wrong_owner.check());
    println!("wrong_owner encode: {:?}", encode_writer_body(&wrong_owner).map(|b| b.len()));

    // Mismatch 3: completely arbitrary set_id.
    let arbitrary = WriterSet { set_id: Hash::digest(b"anything"), ..good.clone() };
    println!("arbitrary check: {:?}", arbitrary.check());
    println!("arbitrary encode: {:?}", encode_writer_body(&arbitrary).map(|b| b.len()));
    let _ = Role::Read;
}
