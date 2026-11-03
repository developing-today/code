//! F11.8 slice 1: benchmark harness for the pkgs/id world stack (#539).
//!
//! Hand-rolled harness (no criterion): each benchmark prints one stable,
//! machine-parseable line —
//!
//! ```text
//! bench <name> unit=<unit> median=<value> min=<value> reps=<n>
//! ```
//!
//! so slice 3's CI job can diff medians against the committed baseline
//! (slice 2) without parsing prose. `--quick` shrinks every workload to a
//! smoke size; `--only <name>` runs a single benchmark.
//!
//! The four workloads from the issue, and the surface each one measures:
//!
//! - `guest_replay` — replaying N committed events: `WorldCore::restore`
//!   over a journal of N `Committed` chat events, then the restored
//!   `replay` events are pushed through a `WorldProgram` (an in-process
//!   counting guest). The wasm sandbox replay path is environment-gated
//!   (it needs the nix-built host), so this measures the host-side replay
//!   pipeline every guest replay pays for; sandbox trap costs are on top.
//! - `journal_append_fsync` — `FileJournal` append of one entry per call,
//!   each ending in `sync_data`: the durable cost of acknowledging one
//!   world mutation.
//! - `checkpoint_compaction` — `FileJournal::compact` replacing a journal
//!   of N entries with a checkpoint-sized set (header + 64 entries): one
//!   batched encode, one write, one file fsync, one directory fsync.
//! - `session_fanout` — one committed chat event delivered to K session
//!   subscribers through the actor's broadcast channel: spawn a world,
//!   subscribe K receivers, publish, drain all K.
//!
//! Numbers are only comparable on the same machine class; baselines must
//! record where they were taken (slice 2).

use std::hint::black_box;
use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context, Result};
use id::world::{
    CapabilityBounds, JournalEntry, WorldCore, WorldEvent, WorldEventKind, WorldHandle,
    WorldJournal, WorldLimits, WorldProgram, WorldScopes,
};
use id::world_store::FileJournal;

/// A guest that counts the events replayed through it.
#[derive(Default)]
struct CountingProgram {
    seen: u64,
}

impl WorldProgram for CountingProgram {
    fn update(&mut self, _event: &WorldEvent) -> Result<()> {
        self.seen += 1;
        Ok(())
    }
}

fn chat_event(sequence: u64, text: &str) -> WorldEvent {
    WorldEvent {
        sequence,
        participant_id: 1,
        kind: WorldEventKind::Chat(text.to_owned()),
    }
}

fn input_event(sequence: u64, text: &str) -> WorldEvent {
    WorldEvent {
        sequence,
        participant_id: 1,
        kind: WorldEventKind::Input(text.as_bytes().to_vec()),
    }
}

fn median(mut samples: Vec<f64>) -> f64 {
    samples.sort_by(|a, b| a.total_cmp(b));
    samples[samples.len() / 2]
}

fn report(name: &str, unit: &str, samples: Vec<f64>) {
    let med = median(samples.clone());
    let min = samples.iter().copied().fold(f64::INFINITY, f64::min);
    println!(
        "bench {name} unit={unit} median={med:.3} min={min:.3} reps={}",
        samples.len()
    );
}

fn bench_journal_append(dir: &PathBuf, entries: u64, reps: usize) -> Result<Vec<f64>> {
    let mut samples = Vec::new();
    for rep in 0..reps {
        let path = dir.join(format!("append-{rep}.jsonl"));
        let (mut journal, _) = FileJournal::open(&path, "bench-append")?;
        let start = Instant::now();
        for seq in 1..=entries {
            journal.append(&JournalEntry::Committed {
                event: chat_event(seq, "benchmark chat payload"),
            })?;
        }
        samples.push(start.elapsed().as_secs_f64() * 1e6 / entries as f64);
        black_box(journal);
    }
    Ok(samples)
}

fn bench_compaction(dir: &PathBuf, entries: u64, reps: usize) -> Result<Vec<f64>> {
    // The set a real compaction writes: header + participants + checkpoint,
    // far smaller than the journal it replaces. 64 stand-in entries keep the
    // payload honest without building a full checkpoint state here.
    let compacted: Vec<JournalEntry> = (1..=64)
        .map(|seq| JournalEntry::Committed {
            event: chat_event(seq, "checkpoint stand-in payload"),
        })
        .collect();
    let mut samples = Vec::new();
    for rep in 0..reps {
        let path = dir.join(format!("compact-{rep}.jsonl"));
        let (mut journal, _) = FileJournal::open(&path, "bench-compact")?;
        for seq in 1..=entries {
            journal.append(&JournalEntry::Committed {
                event: chat_event(seq, "benchmark chat payload"),
            })?;
        }
        let start = Instant::now();
        journal.compact(&compacted)?;
        samples.push(start.elapsed().as_secs_f64() * 1e6);
        black_box(journal);
    }
    Ok(samples)
}

fn bench_guest_replay(events: u64, reps: usize) -> Result<Vec<f64>> {
    // Build the journal once: world created, one participant issued, then
    // N committed input events — the committed kind restore() actually
    // puts in the replay list after a restart. (Committed chat is applied
    // to core state directly and never replayed, which is how the slice-2
    // baseline came to measure an empty guest pass; #1403.)
    let mut core = WorldCore::new("bench-replay", WorldLimits::default())?;
    let (participant, _token) = core.issue_capability(
        "bench",
        WorldScopes::JOIN.union(WorldScopes::CHAT),
        CapabilityBounds {
            expires_at: None,
            uses: None,
        },
    )?;
    let mut journal: Vec<JournalEntry> = vec![JournalEntry::Created {
        world_id: "bench-replay".to_owned(),
        version: 1,
    }];
    // Rebuild the Issued entry from the minted participant via restore's own
    // expectations: issue through a scratch core and harvest its journal by
    // replaying chat through the public API instead — simpler: committed
    // events only need a participant id that exists, and restore validates
    // the header + ordering, so issue the capability in the restored core's
    // place by using participant 1 events after an Issued entry we construct
    // from the scratch participant's public fields.
    journal.push(JournalEntry::Issued {
        participant,
        digest: "0".repeat(64),
        scopes: (WorldScopes::JOIN.union(WorldScopes::CHAT)).bits(),
        parent: None,
        expires_at: None,
        uses: None,
        used: 0,
        subject: None,
    });
    for seq in 1..=events {
        journal.push(JournalEntry::Committed {
            event: input_event(seq, "replay me"),
        });
    }

    let mut samples = Vec::new();
    for _ in 0..reps {
        let start = Instant::now();
        let restored = WorldCore::restore("bench-replay", WorldLimits::default(), journal.clone())
            .context("restore")?;
        let mut guest = CountingProgram::default();
        for event in &restored.replay {
            guest.update(event)?;
        }
        // The guest must have seen every committed event: an empty or
        // short replay is a harness bug (#1403 shipped a baseline whose
        // guest saw zero events), never a timing to record.
        anyhow::ensure!(
            guest.seen == events,
            "guest replay saw {} of {events} committed events",
            guest.seen
        );
        samples.push(start.elapsed().as_secs_f64() * 1e6 / events as f64);
        black_box(guest.seen);
        black_box(restored.replay.len());
    }
    Ok(samples)
}

fn bench_fanout(clients: usize, rounds: u64, reps: usize) -> Result<Vec<f64>> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let mut samples = Vec::new();
    for _ in 0..reps {
        samples.push(runtime.block_on(async {
            let mut core = WorldCore::new("bench-fanout", WorldLimits::default())?;
            // Rotate across several senders: the per-participant chat rate
            // limit (50 messages / 10 s window) would otherwise refuse the
            // full-size workload (100 rounds from one sender) partway
            // through. Four senders x 25 rounds stay under the burst cap
            // while measuring the same delivery path.
            let mut tokens = Vec::new();
            for sender in ["bench-0", "bench-1", "bench-2", "bench-3"] {
                let (_participant, token) = core.issue_capability(
                    sender,
                    WorldScopes::JOIN.union(WorldScopes::CHAT),
                    CapabilityBounds {
                        expires_at: None,
                        uses: None,
                    },
                )?;
                tokens.push(token);
            }
            let handle = WorldHandle::spawn(core);
            let mut receivers: Vec<_> = (0..clients).map(|_| handle.subscribe()).collect();
            let start = Instant::now();
            for round in 0..rounds {
                let token = tokens[(round as usize) % tokens.len()].clone();
                handle.chat(token, format!("fanout {round}")).await?;
                for receiver in &mut receivers {
                    let event = receiver.recv().await.context("fanout recv")?;
                    black_box(event.sequence);
                }
            }
            Ok::<f64, anyhow::Error>(
                start.elapsed().as_secs_f64() * 1e6 / (rounds * clients as u64) as f64,
            )
        })?);
    }
    Ok(samples)
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let quick = args.iter().any(|arg| arg == "--quick");
    let only = args
        .iter()
        .position(|arg| arg == "--only")
        .and_then(|idx| args.get(idx + 1))
        .map(String::as_str);

    let (events, clients, rounds, reps): (u64, usize, u64, usize) = if quick {
        (200, 4, 20, 3)
    } else {
        (10_000, 32, 100, 5)
    };

    let dir = std::env::temp_dir().join(format!("id-bench-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;

    let run = |name: &str| only.is_none() || only == Some(name);

    if run("journal_append_fsync") {
        report(
            "journal_append_fsync",
            "us_per_append",
            bench_journal_append(&dir, events.min(2_000), reps)?,
        );
    }
    if run("checkpoint_compaction") {
        report(
            "checkpoint_compaction",
            "us_per_compact",
            bench_compaction(&dir, events.min(2_000), reps)?,
        );
    }
    if run("guest_replay") {
        report(
            "guest_replay",
            "us_per_event",
            bench_guest_replay(events, reps)?,
        );
    }
    if run("session_fanout") {
        report(
            "session_fanout",
            "us_per_delivery",
            bench_fanout(clients, rounds, reps)?,
        );
    }

    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}
