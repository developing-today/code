//! One authenticated client session against a test world.
//!
//! Slice-1 acceptance probe (#7938 child #13403): connect one client to
//! one test world and record the observed result. Transport-free by
//! design — the "connection" is challenge issuance, Ed25519 signing,
//! and verification — so the probe runs on host and on Android alike
//! with no hosting surface.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use id_client::{ClientIdentity, TestWorld};

#[test]
fn one_authenticated_client_session_connects() {
    let client = ClientIdentity::generate();
    let world = TestWorld::new();
    let signature = client.sign_challenge(world.challenge()).unwrap();
    world
        .verify_session(&client.verifying_key(), &signature)
        .unwrap();
}
