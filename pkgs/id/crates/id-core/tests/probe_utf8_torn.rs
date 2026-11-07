//! Probe (creator-a, 2026-11-02): a crash-torn final journal line whose bytes
//! split a multibyte UTF-8 character. Petnames::open's doc promises a torn
//! final line is dropped and truncated; ASCII torn tails are (existing tests).
//! This probe checks the UTF-8-split tail for the petname journal, and the
//! ASCII torn tail as a control.
use std::io::Write as _;

use id_core::petname::Petnames;

const ALICE: &str = "aa11aa11aa11aa11aa11aa11aa11aa11aa11aa11aa11aa11aa11aa11aa11aa11";
const BOB: &str = "bb22bb22bb22bb22bb22bb22bb22bb22bb22bb22bb22bb22bb22bb22bb22bb22";
const OWNER: &str = "00000000000000000000000000000000000000000000000000000000000000ff";

fn names() -> Vec<(&'static str, &'static str)> {
    vec![(ALICE, "Alice"), (BOB, "Bob")]
}

fn scratch(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("probe-utf8torn-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("petnames.jsonl")
}

#[test]
fn control_ascii_torn_tail_is_dropped() {
    let path = scratch("ascii");
    {
        let mut store = Petnames::open(&path).unwrap();
        store.set(OWNER, ALICE, "Ali", names()).unwrap();
    }
    let good = std::fs::read(&path).unwrap();
    let mut torn = good.clone();
    torn.extend_from_slice(b"{\"op\":\"set\",\"owner\":\"x\",\"acc");
    std::fs::write(&path, &torn).unwrap();
    let store = Petnames::open(&path).expect("ASCII torn tail must be dropped");
    assert_eq!(store.get(OWNER, ALICE), Some("Ali"));
    println!("CONTROL ascii torn tail: OK (dropped, Ali kept)");
}

#[test]
fn probe_utf8_split_torn_tail() {
    let path = scratch("utf8");
    {
        let mut store = Petnames::open(&path).unwrap();
        // A complete, valid first record with a multibyte name.
        store.set(OWNER, ALICE, "José", names()).unwrap();
    }
    let good = std::fs::read(&path).unwrap();
    // Simulate a crash mid-append of a second record whose name contains
    // 'é' (0xC3 0xA9): only the first byte of the pair reached disk, and
    // no terminating newline.
    let mut torn = good.clone();
    torn.extend_from_slice(
        format!("{{\"op\":\"set\",\"owner\":\"{OWNER}\",\"account\":\"{BOB}\",\"name\":\"Ren")
            .as_bytes(),
    );
    torn.push(0xC3); // first half of 'é', second half never written
    std::fs::write(&path, &torn).unwrap();
    match Petnames::open(&path) {
        Ok(store) => {
            println!(
                "PROBE utf8-split torn tail: open OK, alice={:?} (expected if fixed)",
                store.get(OWNER, ALICE)
            );
        }
        Err(error) => {
            println!("PROBE utf8-split torn tail: open FAILED: {error:#}");
            panic!("BUG REPRODUCED: utf8-split torn tail bricks Petnames::open: {error:#}");
        }
    }
    // After a correct open, the next append must replay cleanly too.
    let mut store = Petnames::open(&path).unwrap();
    store.set(OWNER, BOB, "Bobby", names()).unwrap();
    drop(store);
    let store = Petnames::open(&path).unwrap();
    assert_eq!(store.get(OWNER, BOB), Some("Bobby"));
    let _ = std::fs::File::open(&path).unwrap();
}
