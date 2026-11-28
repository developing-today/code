//! Prints each derivation step for Keybase's vector token, then one sample proof and its admission.
//! Run with: cargo run --example dump_vectors

use std::time::Instant;

use seitan_v2::{Invite, Token, accept, derive};

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn main() {
    let token = Token::parse("4uywza+b3cga7rd6yc").unwrap();
    let sikey = derive::stretch(&token);
    let id = derive::invite_id(&sikey);
    let public = derive::signing_key(&sikey).verifying_key().to_bytes();

    println!("token              {}", token.as_str());
    println!("siKey (hex)        {}", hex(sikey.as_bytes()));
    println!("invite ID          {id}   (Keybase vector 6303ec43bd61d21edb95a433faf06227)");
    println!("ed25519 public key {}", hex(&public));

    let created = 1_800_000_000_000;
    let now = created + 60_000;
    let mut invite = Invite::create(&token, created, created + 86_400_000, 1);
    let proof = accept::sign(&token, "joiner-uid", 1, now);
    println!(
        "accept message     {} bytes: {}",
        proof.message.len(),
        hex(&proof.message)
    );
    println!("signature          {}", hex(&proof.signature));
    println!("admit (first)      {:?}", invite.admit(&proof, now));
    println!("admit (replay)     {:?}", invite.admit(&proof, now));

    // Cost of one offline guess: an attacker who has the invite ID tries a candidate token and compares IDs.
    let guesses = 2000;
    let started = Instant::now();
    for _ in 0..guesses {
        let candidate = Token::generate().unwrap();
        std::hint::black_box(derive::invite_id(&derive::stretch(&candidate)));
    }
    let per_guess = started.elapsed() / guesses;
    println!("offline guess      {per_guess:?} each (stretch + invite ID, {guesses} guesses)");
}
