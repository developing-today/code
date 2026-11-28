//! S1 report: header sizes (a), sequence base (c), and sync cost (d).
//! Run the verification tests with `cargo test -p s1-transport`. The transport checks (e, f)
//! are the `dial` and `relay` binaries.

use p2panda_core::{Hash, Header, OperationError, SigningKey, validate_backlink, validate_header};
use s1_transport::header::{ChainExt, Event, build_event, check_received};
use s1_transport::log_sizes::{self, author_key};
use s1_transport::range_sim::{Party, key_of, reconcile};

fn sample_ext(after: usize, group: Option<Hash>) -> ChainExt {
    ChainExt {
        kind: "group.policy".into(),
        after: (0..after as u32)
            .map(|i| Hash::digest(i.to_be_bytes()))
            .collect(),
        group,
        domain: "id.events.v1".into(),
    }
}

fn part_a() {
    println!("## (a) Header size, bytes on the wire (p2panda-core 0.7.1 header + ChainExt)");
    println!("kind = \"group.policy\", domain = \"id.events.v1\", group = None unless noted\n");
    println!("| after | body | seq 1 (backlink) | seq 0 (no backlink) | decode round trip |");
    println!("|---|---|---|---|---|");
    let key = SigningKey::generate();
    let prior = Hash::digest(b"previous event");
    for after in [0usize, 1, 8, 32] {
        for body in [None, Some(vec![7u8; 100])] {
            let body_ref = body.as_deref();
            let ev1 = build_event(&key, 1, Some(prior), sample_ext(after, None), body_ref);
            let ev0 = build_event(&key, 0, None, sample_ext(after, None), body_ref);
            let mut round_ok = true;
            for ev in [&ev1, &ev0] {
                let bytes = ev.encode();
                let back = Header::<ChainExt>::decode(&bytes);
                round_ok &= matches!(&back, Ok(b) if b == ev && b.encode() == bytes);
                round_ok &= check_received(&bytes).is_ok();
            }
            println!(
                "| {after} | {} | {} | {} | {} |",
                match &body {
                    None => "none".to_string(),
                    Some(b) => format!("{} B", b.len()),
                },
                ev1.encode().len(),
                ev0.encode().len(),
                if round_ok { "ok" } else { "FAIL" }
            );
        }
    }
    let with_group = build_event(
        &key,
        1,
        Some(prior),
        sample_ext(1, Some(Hash::digest(b"group"))),
        None,
    );
    println!(
        "\ngroup = Some adds {} bytes (after = 1, no body, seq 1).",
        with_group.encode().len()
            - build_event(&key, 1, Some(prior), sample_ext(1, None), None)
                .encode()
                .len()
    );
    let bare = Header::<()>::builder().seq_num(0).build(&key, ());
    println!(
        "Bare p2panda header (seq 0, no body, no extension): {} bytes.\n",
        bare.encode().len()
    );
}

fn part_c() {
    println!("## (c) Sequence base: validate_header and validate_backlink");
    let key = SigningKey::generate();
    let h0: Event = build_event(&key, 0, None, sample_ext(0, None), None);
    let h1: Event = build_event(&key, 1, Some(h0.hash()), sample_ext(0, None), None);
    println!(
        "base 0: validate_header(seq0) = {:?}",
        validate_header(&h0).is_ok()
    );
    println!(
        "base 0: validate_header(seq1, backlink) = {:?}",
        validate_header(&h1).is_ok()
    );
    println!(
        "base 0: validate_backlink(seq0, seq1) = {:?}",
        validate_backlink(&h0, &h1).is_ok()
    );

    let first_at_1: Event = build_event(&key, 1, None, sample_ext(0, None), None);
    println!(
        "base 1: first event at seq 1, no backlink: validate_header = {:?}",
        validate_header(&first_at_1).err()
    );
    let phantom: Event = build_event(
        &key,
        1,
        Some(Hash::digest(b"no such event")),
        sample_ext(0, None),
        None,
    );
    println!(
        "base 1: first event at seq 1 with a phantom backlink: validate_header = {:?}",
        validate_header(&phantom).is_ok()
    );
    println!(
        "base 1: its backlink resolves to no held event, so validate_backlink has no past header to check against."
    );
    let decode_err = Header::<ChainExt>::decode(&first_at_1.encode()).err();
    println!("base 1: decode of the seq-1 first event = {decode_err:?}");

    // Fork check: two different events at seq 1 with the same predecessor.
    let fork_a: Event = build_event(&key, 1, Some(h0.hash()), sample_ext(0, None), Some(b"A"));
    let fork_b: Event = build_event(&key, 1, Some(h0.hash()), sample_ext(0, None), Some(b"B"));
    let both: Result<(), OperationError> =
        validate_backlink(&h0, &fork_a).and(validate_backlink(&h0, &fork_b));
    println!(
        "fork: two different seq-1 events both pass validate_backlink against h0: {}\n",
        both.is_ok()
    );
}

/// Event ID for chain `author` at `seq`. BLAKE3 over the key, standing in for the header hash.
fn event_id(author: &[u8; 32], seq: u32) -> [u8; 32] {
    *blake3::hash(&key_of(author, seq)).as_bytes()
}

/// Builds the two sides: `base` chains of `per_chain` events each, and the local side with
/// `diff` extra events on distinct chains.
fn parties(n: usize, per_chain: u32, diff: usize) -> (Party, Party, Vec<usize>) {
    let keys: Vec<[u8; 32]> = (0..n).map(|i| *author_key(i).as_bytes()).collect();
    let mut base = Vec::with_capacity(n * per_chain as usize);
    for a in &keys {
        for s in 0..per_chain {
            base.push((key_of(a, s), event_id(a, s)));
        }
    }
    let extra_chains: Vec<usize> = (0..diff).map(|j| j * n / diff.max(1)).collect();
    let mut local = base.clone();
    for &c in &extra_chains {
        local.push((key_of(&keys[c], per_chain), event_id(&keys[c], per_chain)));
    }
    (Party::new(local), Party::new(base), extra_chains)
}

fn part_d() {
    println!("## (d) Sync cost: 1,000 and 10,000 chains of 50 events; local side has 2 new events");
    println!(
        "Control bytes only. The 2 event payloads are the same under both options and are left out.\n"
    );

    // Per-chain LogSync sizes.
    for n in [1_000usize, 10_000] {
        let keys: Vec<_> = (0..n).map(author_key).collect();
        let extra = [0usize, n / 2];
        let have_local = log_sizes::heights(&keys, 49, &extra);
        let have_remote = log_sizes::heights(&keys, 49, &[]);
        let l = log_sizes::have(&have_local).expect("have");
        let r = log_sizes::have(&have_remote).expect("have");
        let ctl = log_sizes::control(&have_local).expect("control");
        println!(
            "LogSync, {n} chains: Have local {} B (CBOR) / {} B (postcard); Have remote {} B / {} B",
            l.cbor, l.postcard, r.cbor, r.postcard
        );
        for (name, s) in ctl.iter().skip(1) {
            println!(
                "  {name}: {} B (CBOR) / {} B (postcard)",
                s.cbor, s.postcard
            );
        }
    }

    println!(
        "\n| chains | LogSync control, CBOR (both Haves + PreSync + 2 Done) | LogSync, postcard | range K=2, T=16 bytes (flights) | range K=16, T=16 bytes (flights) | correct |"
    );
    println!("|---|---|---|---|---|---|");
    for n in [10usize, 30, 100, 300, 1_000, 3_000, 10_000] {
        let keys: Vec<_> = (0..n).map(author_key).collect();
        let extra = [0usize, n / 2];
        let have_local = log_sizes::heights(&keys, 49, &extra);
        let have_remote = log_sizes::heights(&keys, 49, &[]);
        let l = log_sizes::have(&have_local).expect("have");
        let r = log_sizes::have(&have_remote).expect("have");
        let ctl = log_sizes::control(&have_local).expect("control");
        let presync = ctl[1].1;
        let done = ctl[2].1;
        let logsync_cbor = l.cbor + r.cbor + presync.cbor + 2 * done.cbor;
        let logsync_pc = l.postcard + r.postcard + presync.postcard + 2 * done.postcard;

        let (local, remote, _) = parties(n, 50, 2);
        let k2 = reconcile(&local, &remote, 16, 2);
        let k16 = reconcile(&local, &remote, 16, 16);
        println!(
            "| {n} | {logsync_cbor} | {logsync_pc} | {} ({}) | {} ({}) | {} |",
            k2.bytes_a + k2.bytes_b,
            k2.flights,
            k16.bytes_a + k16.bytes_b,
            k16.flights,
            k2.correct && k16.correct
        );
    }

    println!("\nDiff-size sweep at 1,000 and 10,000 chains (K=2, T=16)\n");
    println!("| chains | new events | range bytes (flights) | LogSync CBOR control |");
    println!("|---|---|---|---|");
    for n in [1_000usize, 10_000] {
        let keys: Vec<_> = (0..n).map(author_key).collect();
        let l = log_sizes::have(&log_sizes::heights(&keys, 49, &[0])).expect("have");
        let r = log_sizes::have(&log_sizes::heights(&keys, 49, &[])).expect("have");
        let ctl = log_sizes::control(&log_sizes::heights(&keys, 49, &[])).expect("control");
        let logsync = l.cbor + r.cbor + ctl[1].1.cbor + 2 * ctl[2].1.cbor;
        for diff in [2usize, 20, 200] {
            let (local, remote, _) = parties(n, 50, diff);
            let out = reconcile(&local, &remote, 16, 2);
            println!(
                "| {n} | {diff} | {} ({}) | {logsync} |",
                out.bytes_a + out.bytes_b,
                out.flights
            );
        }
    }
    println!();
}

/// Crossover between LogSync and range sync at 2 new events, and the leaf threshold T.
fn part_d_extra() {
    println!(
        "\nCrossover at 2 new events (CBOR LogSync control vs range total, K=2 and K=16, T=16)\n"
    );
    println!("| chains | LogSync CBOR | range K=2 | range K=16 |");
    println!("|---|---|---|---|");
    for n in [35usize, 40, 45, 50, 60, 80] {
        let keys: Vec<_> = (0..n).map(author_key).collect();
        let l = log_sizes::have(&log_sizes::heights(&keys, 49, &[0])).expect("have");
        let r = log_sizes::have(&log_sizes::heights(&keys, 49, &[])).expect("have");
        let ctl = log_sizes::control(&log_sizes::heights(&keys, 49, &[])).expect("control");
        let logsync = l.cbor + r.cbor + ctl[1].1.cbor + 2 * ctl[2].1.cbor;
        let (local, remote, _) = parties(n, 50, 2);
        let k2 = reconcile(&local, &remote, 16, 2);
        let k16 = reconcile(&local, &remote, 16, 16);
        println!(
            "| {n} | {logsync} | {} | {} |",
            k2.bytes_a + k2.bytes_b,
            k16.bytes_a + k16.bytes_b
        );
    }

    println!("\nLeaf threshold T at 2 new events (bytes (flights))\n");
    println!("| chains | K | T=4 | T=8 | T=16 | T=32 |");
    println!("|---|---|---|---|---|---|");
    for n in [1_000usize, 10_000] {
        for k in [2usize, 16] {
            let (local, remote, _) = parties(n, 50, 2);
            let cells: Vec<String> = [4usize, 8, 16, 32]
                .iter()
                .map(|&t| {
                    let o = reconcile(&local, &remote, t, k);
                    assert!(
                        o.correct,
                        "T={t} K={k} n={n} must learn the exact difference"
                    );
                    format!("{} ({})", o.bytes_a + o.bytes_b, o.flights)
                })
                .collect();
            println!("| {n} | {k} | {} |", cells.join(" | "));
        }
    }
}

fn main() {
    part_a();
    part_c();
    part_d();
    part_d_extra();
}
