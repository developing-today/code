use id_core::membership::body::{decode_body, encode_body};
use id_core::membership::AuthId;
use p2panda_auth::group::{GroupAction, GroupMember};
use p2panda_auth::Access;

fn individual(byte: u8) -> GroupMember<AuthId> {
    GroupMember::Individual(AuthId::Key([byte; 32]))
}

#[test]
fn probe_create_duplicate_member() {
    // Same member twice with conflicting access levels.
    let action = GroupAction::Create {
        initial_members: vec![
            (individual(7), Access::read()),
            (individual(7), Access::manage()),
        ],
    };
    let encoded = encode_body(&action);
    println!("encode dup-conflicting: {:?}", encoded.as_ref().map(|b| b.len()));
    if let Ok(bytes) = &encoded {
        let decoded = decode_body(bytes);
        println!("decode dup-conflicting: {:?}", decoded.as_ref().map(|o| o.as_ref().map(|a| format!("{a:?}"))));
        if let Ok(Some(GroupAction::Create { initial_members })) = decoded {
            println!("decoded initial_members len = {}", initial_members.len());
        }
    }
    // Same member twice with identical access.
    let action2 = GroupAction::Create {
        initial_members: vec![
            (individual(9), Access::read()),
            (individual(9), Access::read()),
        ],
    };
    let encoded2 = encode_body(&action2);
    println!("encode dup-identical: {:?}", encoded2.as_ref().map(|b| b.len()));
    if let Ok(bytes) = &encoded2 {
        println!("decode dup-identical ok = {:?}", decode_body(bytes).map(|o| o.is_some()));
    }
    // Control: two distinct members.
    let control = GroupAction::Create {
        initial_members: vec![
            (individual(1), Access::read()),
            (individual(2), Access::manage()),
        ],
    };
    let bytes = encode_body(&control).expect("control encodes");
    assert!(matches!(decode_body(&bytes), Ok(Some(GroupAction::Create { .. }))));
    println!("control ok");
}
