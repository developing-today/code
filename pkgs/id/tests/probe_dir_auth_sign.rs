// Probe (not for commit): does sign_request produce Signed values that
// verify_signed always rejects, purely because of the nonce?
use ed25519_dalek::SigningKey;
use id::directory_auth::{ReplayGuard, sign_request, verify_signed};

const AT: u64 = 1_700_000_000_000;

fn verifies(nonce: &str) -> bool {
    let key = SigningKey::from_bytes(&[7; 32]);
    let signed = sign_request(&key, "GET", "/x", b"", AT, nonce);
    let mut replays = ReplayGuard::default();
    verify_signed(&mut replays, &signed, "GET", "/x", b"", AT).is_ok()
}

#[test]
fn probe_sign_request_nonce_domain() {
    // Control: a nonce inside verify_signed's stated domain verifies.
    assert!(verifies("n1"), "control nonce should verify");
    assert!(verifies("abc-DEF-123"), "alphanumeric+dash should verify");
    assert!(verifies(&"a".repeat(64)), "64-char nonce should verify");

    // Malformed nonces: sign_request returns a Signed infallibly, and
    // verify_signed rejects every one (nonce rule at directory_auth.rs).
    let bad: Vec<String> = vec![
        String::new(),          // empty
        "a".repeat(65),         // one over the 64-char cap
        "has space".to_owned(), // space
        "under_score".to_owned(),
        "plus+sign".to_owned(),
        "dot.name".to_owned(),
        "uni\u{00e9}".to_owned(), // non-ASCII
    ];
    for nonce in &bad {
        assert!(
            !verifies(nonce),
            "nonce {nonce:?} unexpectedly verified"
        );
        println!("sign Ok but verify rejects nonce {nonce:?}");
    }
}
