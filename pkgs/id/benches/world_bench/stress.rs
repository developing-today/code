//! Scale-program stress workloads (#13057): the `#6304` scale and
//! performance stress program benches — seeded world generation
//! (`stress_gen`), memory profiling (`stress_mem`), frame cost
//! (`frame_cost`), and join-time profiling (`stress_join`). Split out
//! of `world_bench.rs` to keep that file under the production-line
//! limit (#1411). Everything here resolves through the parent bench
//! target (`use super::*`), exactly like the code did inline.

use super::*;

/// The codified document shape the stress mix's document edits are
/// stated against: 64 KiB per document, 512 list items (design
/// limits inventory, #6281 findings). An edit event names a document
/// and a list item inside those bounds; it does not embed a whole
/// document snapshot.
const STRESS_DOC_BYTES: u64 = 65_536;
const STRESS_DOC_LIST_ITEMS: u64 = 512;
/// The codified compile-input cap the mix's program installs are
/// stated against: 8 files / 256 KiB total per install.
const STRESS_INSTALL_FILES: u64 = 8;
const STRESS_INSTALL_BYTES: u64 = 262_144;

/// The stress event mix, as counts per 1,000 events in the same
/// discipline as the loadgen core's mix tables: the shape at stream
/// position `i` is a pure function of `i`, never sampled, so the
/// same (seed, tier) always produces the same stream. The schedule
/// interleaves the mix with a period of 10 events (7 chat, 2 doc
/// edits, 1 program install per period, so the per-1,000 counts are
/// exactly the mix constants) instead of placing each shape in one
/// block: a blocked schedule left the quick tier's 200-event hot
/// world and every 100-event active world with no document edits or
/// program installs at all, while the manifest claimed all three
/// shapes for them (#16042).
const STRESS_MIX_CHAT: u64 = 700;
const STRESS_MIX_DOC_EDIT: u64 = 200;
const STRESS_MIX_PROGRAM_INSTALL: u64 = 100;

#[derive(Clone, Copy, PartialEq, Eq)]
enum StressShape {
    Chat,
    DocEdit,
    ProgramInstall,
}

impl StressShape {
    fn name(self) -> &'static str {
        match self {
            StressShape::Chat => "chat",
            StressShape::DocEdit => "doc_edit",
            StressShape::ProgramInstall => "program_install",
        }
    }
}

fn stress_shape_at(index: u64) -> StressShape {
    // One period of the interleaved schedule: positions 2 and 8 are
    // document edits, position 5 is a program install, the rest are
    // chat. Every window of 10 consecutive events — and therefore
    // every generated fixture, down to the 20-event quick active
    // worlds — contains all three declared shapes.
    match index % 10 {
        2 | 8 => StressShape::DocEdit,
        5 => StressShape::ProgramInstall,
        _ => StressShape::Chat,
    }
}

/// One `ID_BENCH_STRESS_*` axis override, falling back to the tier's
/// committed value when the variable is unset or unparseable (the
/// `ManyWorldsConfig` profiling-override pattern).
fn stress_env(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

/// A scale tier of the #6304 stress program: the design's tier table
/// (idle worlds / active worlds / single-world history / members on
/// the hot world), plus the per-active-world history the generator
/// gives every active world. A tier is a measurement target, not a
/// promise: a tier whose set cannot be generated or held is a
/// measured limit, recorded as such by the run that hits it. The
/// members axis meets such a wall today: the design targets 320
/// members at 10x, but a world refuses more participants than the
/// codified `WorldLimits::default().participants` (256), so the
/// generator caps the hot world there and records the wall in the
/// manifest rather than emitting a world the store rejects.
#[derive(Clone, Copy)]
struct StressTier {
    name: &'static str,
    idle_worlds: usize,
    active_worlds: usize,
    hot_events: u64,
    active_events: u64,
    hot_members: usize,
}

impl StressTier {
    fn with_env_overrides(mut self) -> Self {
        self.idle_worlds = stress_env("ID_BENCH_STRESS_IDLE_WORLDS", self.idle_worlds);
        self.active_worlds = stress_env("ID_BENCH_STRESS_ACTIVE_WORLDS", self.active_worlds);
        self.hot_events = stress_env("ID_BENCH_STRESS_HOT_EVENTS", self.hot_events as usize) as u64;
        self.active_events =
            stress_env("ID_BENCH_STRESS_ACTIVE_EVENTS", self.active_events as usize) as u64;
        self.hot_members = stress_env("ID_BENCH_STRESS_HOT_MEMBERS", self.hot_members);
        self
    }

    fn quick() -> Self {
        Self {
            name: "quick",
            idle_worlds: 10,
            active_worlds: 4,
            hot_events: 200,
            active_events: 20,
            hot_members: 4,
        }
        .with_env_overrides()
    }

    fn tier_1x() -> Self {
        Self {
            name: "1x",
            idle_worlds: 1_000,
            active_worlds: 100,
            hot_events: 10_000,
            active_events: 100,
            hot_members: 32,
        }
        .with_env_overrides()
    }

    fn tier_10x() -> Self {
        Self {
            name: "10x",
            idle_worlds: 10_000,
            active_worlds: 1_000,
            hot_events: 100_000,
            active_events: 100,
            hot_members: 320,
        }
        .with_env_overrides()
    }

    fn tier_100x() -> Self {
        Self {
            name: "100x",
            idle_worlds: 100_000,
            active_worlds: 10_000,
            hot_events: 1_000_000,
            active_events: 100,
            hot_members: 3_200,
        }
        .with_env_overrides()
    }

    fn named(name: &str) -> Result<Self> {
        match name {
            "quick" => Ok(Self::quick()),
            "1x" => Ok(Self::tier_1x()),
            "10x" => Ok(Self::tier_10x()),
            "100x" => Ok(Self::tier_100x()),
            other => anyhow::bail!(
                "unknown stress tier '{other}' in ID_BENCH_STRESS_TIER (valid: quick, 1x, 10x, 100x)"
            ),
        }
    }
}

/// FNV-1a (64-bit) over a byte stream: the manifest's expected state
/// hash. A change-detection hash, not a cryptographic one — its job
/// is to make two runs of the same (seed, tier) comparable and a
/// fast-but-wrong replay visible, cheaply and with no dependency.
struct Fnv1a(u64);

/// What the manifest's `hash.input` records for the state hash, and
/// the value the join profile requires before reusing a fixture: a
/// fixture generated under any other hash definition is regenerated,
/// never verified against the wrong oracle (#16042).
const STRESS_STATE_HASH_INPUT: &str = "canonical restored semantic state v1: restored     current_sequence, the installed program (module_hash, install seed, snapshot,     provenance), and every replay event (sequence, participant, kind) in order —     never the source journal bytes";

/// The program artifact every stress program install installs: the
/// checked-in counter guest the many-worlds bench also runs
/// (`COUNTER_WASM` in the parent bench target), named by the
/// service's own `module_hash`. A generated install therefore
/// records a real, runnable artifact whose hash `WorldCore::restore`
/// surfaces as the restored world's active program (#16042), not
/// descriptive text about an install that never happened.
fn stress_program_module_hash() -> String {
    module_hash(COUNTER_WASM)
}

/// The expected state hash of one restored stress world: FNV-1a over
/// a canonical representation of the *restored semantic state* —
/// the restored sequence, the installed program the restore loaded
/// (module hash, install seed, snapshot, provenance), and the
/// replay state (every event the installed program will apply, in
/// order) — never the source journal bytes. A replay that changes
/// state while preserving the journal or the sequence therefore
/// changes the hash (#16042).
fn stress_state_hash(restored: &RestoredWorld) -> String {
    let mut hash = Fnv1a::new();
    hash.write(b"id-stress-state-v1\0");
    hash.write(&restored.core.current_sequence().to_le_bytes());
    match &restored.program {
        Some(program) => {
            hash.write(b"program\0");
            hash.write(program.module_hash.as_bytes());
            hash.write(&program.seed.to_le_bytes());
            match &program.snapshot {
                Some(snapshot) => {
                    hash.write(b"snapshot\0");
                    hash.write(snapshot.as_bytes());
                }
                None => hash.write(b"no-snapshot\0"),
            }
            if let Ok(bytes) = serde_json::to_vec(&program.provenance) {
                hash.write(&bytes);
            }
        }
        None => hash.write(b"no-program\0"),
    }
    hash.write(&(restored.replay.len() as u64).to_le_bytes());
    for event in &restored.replay {
        hash.write(&event.sequence.to_le_bytes());
        hash.write(&event.participant_id.to_le_bytes());
        if let Ok(bytes) = serde_json::to_vec(&event.kind) {
            hash.write(&bytes);
            hash.write(b"\n");
        }
    }
    hash.finish()
}

/// The number of replay inputs a restore of `entries` must hold:
/// inputs accumulate until a program install replaces the program
/// and wipes the replay slate (the restore's own semantics), so
/// only inputs after the last install remain.
fn stress_expected_replay_len(entries: &[JournalEntry]) -> u64 {
    let mut replay_len = 0_u64;
    for entry in entries {
        if let JournalEntry::Committed { event } = entry {
            match &event.kind {
                WorldEventKind::ProgramInstalled { .. } => replay_len = 0,
                WorldEventKind::Input(_) | WorldEventKind::EntityInput { .. } => {
                    replay_len += 1;
                }
                _ => {}
            }
        }
    }
    replay_len
}

impl Fnv1a {
    fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }

    fn write(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.0 ^= u64::from(byte);
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }

    fn finish(&self) -> String {
        format!("{:016x}", self.0)
    }
}

/// One generated measured world, as recorded in the manifest.
struct StressWorldRecord {
    name: String,
    role: &'static str,
    members: usize,
    events: u64,
    chat: u64,
    doc_edits: u64,
    program_installs: u64,
    expected_state_hash: String,
}

/// The deterministic event for one stream position of one generated
/// world. Every choice derives from the run seed through the loadgen
/// corpus / SplitMix64, so the event stream is a pure function of
/// (seed, world, position).
fn stress_event(
    seed: u64,
    world_index: u32,
    sequence: u64,
    participant_id: u64,
) -> (WorldEvent, StressShape) {
    let shape = stress_shape_at(sequence - 1);
    let corpus = loadgen::Corpus::new(seed);
    let kind = match shape {
        StressShape::Chat => WorldEventKind::Chat(corpus.text(world_index, sequence, 48)),
        StressShape::DocEdit => {
            let mut rng = loadgen::SplitMix64::new(
                seed ^ (u64::from(world_index) << 32)
                    ^ sequence.wrapping_mul(0x9E37_79B9_7F4A_7C15),
            );
            let doc = rng.next_u64() % 64;
            let item = rng.next_u64() % STRESS_DOC_LIST_ITEMS;
            WorldEventKind::Input(
                format!(
                    "doc-edit doc={doc} item={item} doc_bytes={STRESS_DOC_BYTES} \
                     list_items={STRESS_DOC_LIST_ITEMS} fill={}",
                    corpus.text(world_index, sequence, 96)
                )
                .into_bytes(),
            )
        }
        // A real install (#16042): a host-authored `ProgramInstalled`
        // event naming the bench's counter-guest artifact (see
        // `stress_program_module_hash`), installed with the run seed
        // as its init seed. `stress_world_entries` zeroes the event's
        // participant id — installs are host-authored, never a
        // participant's input — and the inputs after it in the stream
        // are the replay the restored program applies. The codified
        // compile-input caps the mix states installs against
        // (`STRESS_INSTALL_FILES` / `STRESS_INSTALL_BYTES`) remain
        // recorded in the manifest's shape table.
        StressShape::ProgramInstall => WorldEventKind::ProgramInstalled {
            module_hash: stress_program_module_hash(),
            seed,
            provenance: Some(id::world::ModuleProvenance::Artifact),
            trust: None,
        },
    };
    (
        WorldEvent {
            sequence,
            participant_id,
            kind,
        },
        shape,
    )
}

/// The journal entries of one generated stress world, in write
/// order, with the per-shape event counts. Shared by the generator
/// (which writes them to disk and verifies the file) and the memory
/// profile (#13086, which restores them in-process): both must see
/// exactly the same world for a given (seed, world, events, members).
fn stress_world_entries(
    seed: u64,
    name: &str,
    world_index: u32,
    events: u64,
    members: usize,
) -> Result<(Vec<JournalEntry>, u64, u64, u64)> {
    ensure!(
        members >= 1,
        "stress world {name} needs at least one member"
    );
    let mut entries: Vec<JournalEntry> = Vec::with_capacity((events + members as u64 + 1) as usize);
    entries.push(JournalEntry::Created {
        world_id: name.to_owned(),
        version: 1,
    });
    for member in 0..members {
        entries.push(JournalEntry::Issued {
            participant: id::world::Participant {
                id: member as u64 + 1,
                display_name: format!("stress-{member}"),
            },
            digest: "0".repeat(64),
            scopes: WorldScopes::GUEST.bits(),
            parent: None,
            expires_at: None,
            uses: None,
            used: 0,
            subject: None,
            issued_at: None,
        });
    }
    let (mut chat, mut doc_edits, mut program_installs) = (0_u64, 0_u64, 0_u64);
    for sequence in 1..=events {
        let participant_id = (sequence - 1) % members as u64 + 1;
        let (mut event, shape) = stress_event(seed, world_index, sequence, participant_id);
        if shape == StressShape::ProgramInstall {
            // Program installs are host-authored: participant zero,
            // as the world itself records them.
            event.participant_id = 0;
        }
        match shape {
            StressShape::Chat => chat += 1,
            StressShape::DocEdit => doc_edits += 1,
            StressShape::ProgramInstall => program_installs += 1,
        }
        entries.push(JournalEntry::Committed { event });
    }
    Ok((entries, chat, doc_edits, program_installs))
}

/// Generate one measured world directory (`<out>/<name>/journal.jsonl`)
/// and verify it by restoring the written journal through the real
/// restore path. The expected state hash is the canonical
/// restored-state hash (`stress_state_hash`): it is derived from
/// what the restore actually rebuilt — sequence, installed program,
/// and replay state — so a replay that changes state while
/// preserving the journal bytes or the sequence cannot match the
/// manifest (#16042).
fn generate_stress_world(
    out: &Path,
    name: &str,
    role: &'static str,
    seed: u64,
    world_index: u32,
    events: u64,
    members: usize,
) -> Result<StressWorldRecord> {
    let (entries, chat, doc_edits, program_installs) =
        stress_world_entries(seed, name, world_index, events, members)?;

    let dir = out.join(name);
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("journal.jsonl");
    let mut bytes: Vec<u8> = Vec::new();
    for entry in &entries {
        serde_json::to_writer(&mut bytes, entry).context("serialize stress journal entry")?;
        bytes.push(b'\n');
    }
    std::fs::write(&path, &bytes).with_context(|| format!("write stress journal {path:?}"))?;

    // Verify before recording: read the journal back in the store's
    // own line format and restore it. A generator that emits worlds
    // the store cannot open is worse than no generator.
    let text = std::fs::read_to_string(&path)?;
    let read_back: Vec<JournalEntry> = text
        .lines()
        .map(serde_json::from_str)
        .collect::<std::result::Result<_, _>>()
        .context("re-parse generated stress journal")?;
    ensure!(
        read_back.len() == entries.len(),
        "{name}: read back {} entries, wrote {}",
        read_back.len(),
        entries.len()
    );
    let restored = WorldCore::restore(name, WorldLimits::default(), read_back)
        .map_err(|e| anyhow::anyhow!("restore generated stress world {name}: {e:#}"))?;
    ensure!(
        restored.core.current_sequence() == events,
        "{name}: restored at sequence {}, generated {events} events",
        restored.core.current_sequence()
    );
    // The installed program is part of the restored state: a world
    // whose stream installs programs must restore with the bench
    // artifact active, at the run seed, and its replay must hold
    // exactly the inputs committed after the last install (an
    // install replaces the program and wipes the replay slate).
    if program_installs > 0 {
        let program = restored.program.as_ref().with_context(|| {
            format!("{name}: restored world has no program after {program_installs} installs")
        })?;
        ensure!(
            program.module_hash == stress_program_module_hash() && program.seed == seed,
            "{name}: restored program is {} seed {}, generated installs name {} seed {seed}",
            program.module_hash,
            program.seed,
            stress_program_module_hash(),
        );
    } else {
        ensure!(
            restored.program.is_none(),
            "{name}: restored world has a program, generated no installs"
        );
    }
    let expected_replay = stress_expected_replay_len(&read_back);
    ensure!(
        restored.replay.len() as u64 == expected_replay,
        "{name}: restored replay holds {} inputs, generated {} after the last install",
        restored.replay.len(),
        expected_replay
    );

    Ok(StressWorldRecord {
        name: name.to_owned(),
        role,
        members,
        events,
        chat,
        doc_edits,
        program_installs,
        expected_state_hash: stress_state_hash(&restored),
    })
}

/// The `stress_gen` profile (#13083): materialize the tier's world
/// directory set plus its manifest under the output root. The output
/// root is a pure function of (seed, tier): generation materializes
/// into a staging sibling and publishes by rename (#13181), so a
/// completed generation is regenerated in place, an interrupted one
/// is recovered on retry, and anything unverifiable is refused
/// rather than deleted.
pub fn bench_stress_gen(quick: bool) -> Result<()> {
    let tier = if quick {
        StressTier::quick()
    } else {
        match std::env::var("ID_BENCH_STRESS_TIER") {
            Ok(name) => StressTier::named(&name)?,
            Err(_) => StressTier::tier_1x(),
        }
    };
    let seed = std::env::var("ID_BENCH_STRESS_SEED")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(BENCH_SEED);
    let out = std::env::var("ID_BENCH_STRESS_OUT").map_or_else(
        |_| std::env::temp_dir().join(format!("id-stress-{}-{}", tier.name, seed)),
        PathBuf::from,
    );
    let staging =
        stress_output::prepare_stress_output(&out, seed, tier.name, loadgen::GENERATOR_VERSION)?;
    let started = Instant::now();

    // Idle worlds: durable headers only — a Created entry and nothing
    // else, matching what the many-worlds workload leaves on disk for
    // a world that was opened and evicted without a guest.
    for index in 0..tier.idle_worlds {
        let name = format!("idle-{index:05}");
        let dir = staging.join(&name);
        std::fs::create_dir_all(&dir)?;
        let entry = JournalEntry::Created {
            world_id: name,
            version: 1,
        };
        let mut line = serde_json::to_vec(&entry).context("serialize idle world header")?;
        line.push(b'\n');
        std::fs::write(dir.join("journal.jsonl"), line)?;
    }

    let mut records: Vec<StressWorldRecord> = Vec::with_capacity(tier.active_worlds + 1);
    for index in 0..tier.active_worlds {
        let name = format!("active-{index:04}");
        records.push(generate_stress_world(
            &staging,
            &name,
            "active",
            seed,
            index as u32,
            tier.active_events,
            1,
        )?);
    }
    // The members wall (see StressTier): generate at the codified
    // participant cap when the tier targets more, and record the
    // wall in the manifest — a wall is a result, per the design's
    // run protocol, not a silent clamp.
    let participant_cap = WorldLimits::default().participants;
    let hot_members = tier.hot_members.min(participant_cap);
    let mut walls: Vec<serde_json::Value> = Vec::new();
    if hot_members < tier.hot_members {
        walls.push(serde_json::json!({
            "axis": "hot_members",
            "tier_target": tier.hot_members,
            "generated": hot_members,
            "limit": "codified: WorldLimits::default().participants",
            "limit_value": participant_cap,
        }));
    }
    let hot = generate_stress_world(
        &staging,
        "hot-000",
        "hot",
        seed,
        u32::MAX,
        tier.hot_events,
        hot_members,
    )?;
    let hot_hash = hot.expected_state_hash.clone();
    records.push(hot);

    let wall_count = walls.len();
    let total_events: u64 = records.iter().map(|record| record.events).sum();
    let worlds_json: Vec<serde_json::Value> = records
        .iter()
        .map(|record| {
            serde_json::json!({
                "name": record.name,
                "role": record.role,
                "members": record.members,
                "events": record.events,
                "counts": {
                    "chat": record.chat,
                    "doc_edit": record.doc_edits,
                    "program_install": record.program_installs,
                },
                "expected_state_hash": record.expected_state_hash,
            })
        })
        .collect();
    let mut mix = serde_json::Map::new();
    mix.insert(StressShape::Chat.name().to_owned(), STRESS_MIX_CHAT.into());
    mix.insert(
        StressShape::DocEdit.name().to_owned(),
        STRESS_MIX_DOC_EDIT.into(),
    );
    mix.insert(
        StressShape::ProgramInstall.name().to_owned(),
        STRESS_MIX_PROGRAM_INSTALL.into(),
    );
    let manifest = serde_json::json!({
        "type": "stress-manifest",
        "schema_version": 1,
        "generator_version": loadgen::GENERATOR_VERSION,
        "seed": seed,
        "tier": tier.name,
        "mix_per_1000": serde_json::Value::Object(mix),
        "shapes": {
            "doc_bytes": STRESS_DOC_BYTES,
            "doc_list_items": STRESS_DOC_LIST_ITEMS,
            "install_files_max": STRESS_INSTALL_FILES,
            "install_bytes_max": STRESS_INSTALL_BYTES,
        },
        "hash": {
            "algorithm": "fnv1a64",
            "input": STRESS_STATE_HASH_INPUT,
            "format": "16 lowercase hex digits",
        },
        "idle_worlds": {"count": tier.idle_worlds, "events_per_world": 0},
        "walls": walls,
        "worlds": worlds_json,
        "totals": {
            "worlds": tier.idle_worlds + tier.active_worlds + 1,
            "measured_worlds": records.len(),
            "events": total_events,
        },
    });
    let manifest_path = stress_output::publish_stress_output(&out, &staging, &manifest)?;

    println!(
        "profile stress_gen tier={} seed={} worlds={} measured_worlds={} events={} hot_state_hash={} walls={} elapsed_secs={:.3} out={} manifest={}",
        tier.name,
        seed,
        tier.idle_worlds + tier.active_worlds + 1,
        records.len(),
        total_events,
        hot_hash,
        wall_count,
        started.elapsed().as_secs_f64(),
        out.display(),
        manifest_path.display(),
    );
    Ok(())
}

/// (Current RSS, peak RSS) of this process in bytes, from
/// `/proc/self/status` (`VmRSS` / `VmHWM`). Linux-only by design:
/// the reference machines are Linux, and a memory measurement that
/// cannot read its own resident set must fail, never fabricate.
fn rss_bytes() -> Result<(u64, u64)> {
    let status = std::fs::read_to_string("/proc/self/status")
        .context("read /proc/self/status for RSS sampling")?;
    let mut rss_kib: Option<u64> = None;
    let mut peak_kib: Option<u64> = None;
    for line in status.lines() {
        if let Some(value) = line.strip_prefix("VmRSS:") {
            rss_kib = value.trim().trim_end_matches("kB").trim().parse().ok();
        } else if let Some(value) = line.strip_prefix("VmHWM:") {
            peak_kib = value.trim().trim_end_matches("kB").trim().parse().ok();
        }
    }
    Ok((
        rss_kib.context("VmRSS missing from /proc/self/status")? * 1024,
        peak_kib.context("VmHWM missing from /proc/self/status")? * 1024,
    ))
}

/// The tier + seed selection `stress_gen` runs under, shared with
/// the memory profile so both measure the same fixture.
fn stress_selection(quick: bool) -> Result<(StressTier, u64)> {
    let tier = if quick {
        StressTier::quick()
    } else {
        match std::env::var("ID_BENCH_STRESS_TIER") {
            Ok(name) => StressTier::named(&name)?,
            Err(_) => StressTier::tier_1x(),
        }
    };
    let seed = std::env::var("ID_BENCH_STRESS_SEED")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(BENCH_SEED);
    Ok((tier, seed))
}

/// One compaction measurement of the `stress_mem` profile (#13086):
/// the peak additional resident memory while compacting a journal
/// of `events` events. The journal is bulk-written in the store's
/// on-disk format (a fixture, as in `stress_gen` — not the measured
/// append path) with the generator's own entry construction, then
/// the measured operation runs the stack's real path: recover
/// through `FileJournal::open` (which parses every entry into
/// memory), restore the world, hold it resident, and replace the
/// journal through `FileJournal::compact` with the checkpoint-sized
/// set the compaction bench uses (64 stand-in entries). The spike
/// is (peak RSS during the operation) − (RSS just before it), so it
/// attributes memory to the operation, not to whatever the process
/// already held.
fn bench_stress_mem_compact(dir: &Path, events: u64) -> Result<()> {
    let name = "stress-mem-compact";
    let (entries, ..) = stress_world_entries(BENCH_SEED, name, 0, events, 1)?;
    let path = dir.join(format!("stress-mem-compact-{events}.jsonl"));
    {
        let mut bytes: Vec<u8> = Vec::new();
        for entry in &entries {
            serde_json::to_writer(&mut bytes, entry).context("serialize compact fixture entry")?;
            bytes.push(b'\n');
        }
        std::fs::write(&path, &bytes)
            .with_context(|| format!("write compact fixture journal {path:?}"))?;
    }
    drop(entries);
    let replacement: Vec<JournalEntry> = (1..=64)
        .map(|seq| JournalEntry::Committed {
            event: chat_event(seq, "checkpoint stand-in payload"),
        })
        .collect();

    let (rss_before, _) = rss_bytes()?;
    let started = Instant::now();
    let (mut journal, recovered) = FileJournal::open(&path, name)
        .with_context(|| format!("open compact fixture journal {path:?}"))?;
    ensure!(
        recovered.len() as u64 == events + 2,
        "compact fixture: recovered {} entries, wrote {}",
        recovered.len(),
        events + 2
    );
    let restored = WorldCore::restore(name, WorldLimits::default(), recovered)
        .map_err(|e| anyhow::anyhow!("restore compact fixture world: {e:#}"))?;
    black_box(&restored);
    journal
        .compact(&replacement)
        .context("compact the fixture journal")?;
    let elapsed = started.elapsed();
    let (_, peak) = rss_bytes()?;
    let spike = peak.saturating_sub(rss_before);
    println!(
        "profile stress_mem_compact events={} recovered_entries={} spike_bytes={} spike_mib={:.2} elapsed_us={}",
        events,
        events + 2,
        spike,
        spike as f64 / 1_048_576.0,
        elapsed.as_micros(),
    );
    drop(restored);
    drop(journal);
    let _ = std::fs::remove_file(&path);
    Ok(())
}

/// The resident-set measurement of the `stress_mem` profile
/// (#13086): restore the tier's worlds through the real restore
/// path and hold them all resident, sampling RSS between phases —
/// idle worlds first (a lone `Created` header each: the resident
/// form of a world opened and evicted without a guest), then the
/// active worlds, then the hot world at the codified participant
/// cap (the generator's wall, applied the same way). A restore
/// failure stops the growth and is recorded as the tier's wall:
/// per the design's run protocol a wall is a measured limit, not a
/// failed run.
fn bench_stress_mem_resident(tier: &StressTier, seed: u64) -> Result<()> {
    let started = Instant::now();
    let (rss_start, _) = rss_bytes()?;
    let mut resident: Vec<RestoredWorld> = Vec::new();
    let mut walls: Vec<String> = Vec::new();

    for index in 0..tier.idle_worlds {
        let name = format!("idle-{index:05}");
        let entries = vec![JournalEntry::Created {
            world_id: name.clone(),
            version: 1,
        }];
        match WorldCore::restore(&name, WorldLimits::default(), entries) {
            Ok(restored) => resident.push(restored),
            Err(error) => {
                walls.push(format!("idle world {name}: {error:#}"));
                break;
            }
        }
    }
    let (rss_after_idle, _) = rss_bytes()?;
    let idle_held = resident.len();

    if walls.is_empty() {
        for index in 0..tier.active_worlds {
            let name = format!("active-{index:04}");
            let (entries, ..) =
                stress_world_entries(seed, &name, index as u32, tier.active_events, 1)?;
            match WorldCore::restore(&name, WorldLimits::default(), entries) {
                Ok(restored) => resident.push(restored),
                Err(error) => {
                    walls.push(format!("active world {name}: {error:#}"));
                    break;
                }
            }
        }
    }
    let (rss_after_active, _) = rss_bytes()?;
    let active_held = resident.len() - idle_held;

    let mut hot_held = 0_usize;
    if walls.is_empty() {
        let participant_cap = WorldLimits::default().participants;
        let hot_members = tier.hot_members.min(participant_cap);
        let (entries, ..) =
            stress_world_entries(seed, "hot-000", u32::MAX, tier.hot_events, hot_members)?;
        match WorldCore::restore("hot-000", WorldLimits::default(), entries) {
            Ok(restored) => {
                resident.push(restored);
                hot_held = 1;
            }
            Err(error) => walls.push(format!("hot world: {error:#}")),
        }
    }
    let (rss_final, peak) = rss_bytes()?;
    black_box(&resident);

    let held = resident.len();
    let per_world =
        |delta: u64, count: usize| -> u64 { if count == 0 { 0 } else { delta / count as u64 } };
    println!(
        "profile stress_mem tier={} seed={} worlds_held={} idle_held={} active_held={} hot_held={} rss_start_bytes={} rss_after_idle_bytes={} rss_after_active_bytes={} rss_final_bytes={} peak_rss_bytes={} rss_per_world_bytes={} rss_per_idle_world_bytes={} rss_per_active_world_bytes={} rss_hot_delta_bytes={} walls={} elapsed_secs={:.3}",
        tier.name,
        seed,
        held,
        idle_held,
        active_held,
        hot_held,
        rss_start,
        rss_after_idle,
        rss_after_active,
        rss_final,
        peak,
        per_world(rss_final.saturating_sub(rss_start), held),
        per_world(rss_after_idle.saturating_sub(rss_start), idle_held),
        per_world(rss_after_active.saturating_sub(rss_after_idle), active_held),
        rss_final.saturating_sub(rss_after_active),
        walls.len(),
        started.elapsed().as_secs_f64(),
    );
    for wall in &walls {
        println!("profile stress_mem_wall tier={} detail={wall}", tier.name);
    }
    Ok(())
}

/// The `stress_mem` profile (#13086): the memory profile of the
/// world stack at scale — RSS per resident world at the selected
/// tier, and the compaction spike at a 100,000-entry journal
/// alongside the 2,000-entry size of the committed baseline. The
/// compaction measurements run first and in increasing size order,
/// so a fresh process attributes each spike cleanly;
/// `ID_BENCH_STRESS_MEM_PART` (`all` / `compact` / `resident`)
/// selects one part when a run wants it alone.
pub fn bench_stress_mem(quick: bool) -> Result<()> {
    let (tier, seed) = stress_selection(quick)?;
    let part = std::env::var("ID_BENCH_STRESS_MEM_PART").unwrap_or_else(|_| "all".to_owned());
    ensure!(
        part == "all" || part == "compact" || part == "resident",
        "unknown ID_BENCH_STRESS_MEM_PART '{part}' (valid: all, compact, resident)"
    );
    let dir = std::env::temp_dir().join(format!("id-stress-mem-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    if part != "resident" {
        bench_stress_mem_compact(&dir, 2_000)?;
        bench_stress_mem_compact(&dir, 100_000)?;
    }
    if part != "compact" {
        bench_stress_mem_resident(&tier, seed)?;
    }
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

/// The `frame_cost` profile (#13085): view (and layout) fuel and wall
/// time as a function of applied state size, for the Territory-class
/// guest the scale design names as the frame-cost reference. The world
/// takes the perf-budgets baseline shape — two seats joined — then the
/// applied events are alternating cursor-move inputs from the two
/// seats, so the applied-event axis grows without ending the game.
/// The frame is measured at full state: fuel is the dedicated call's
/// `fuel_last` from `WorldInstance::metrics` (the instrumentation
/// `id world measure` reports), wall time the median of five calls.
/// Sizes are the stress tiers' hot-world event counts (10k / 100k /
/// 1M); `ID_BENCH_FRAME_EVENTS` selects a single size, and `--quick`
/// runs a 200-event smoke size.
pub fn bench_frame_cost(quick: bool) -> Result<()> {
    let sizes: Vec<u64> = if quick {
        vec![200]
    } else if let Ok(value) = std::env::var("ID_BENCH_FRAME_EVENTS") {
        vec![value.parse().context("parse ID_BENCH_FRAME_EVENTS")?]
    } else {
        vec![10_000, 100_000, 1_000_000]
    };
    for events in sizes {
        frame_cost_at(events)?;
    }
    Ok(())
}

fn frame_cost_at(events: u64) -> Result<()> {
    let sandbox = Sandbox::compile(TERRITORY_WASM, SandboxLimits::default())?;
    let mut world = sandbox.instantiate(1)?;
    for (id, name) in [(1, "ann"), (2, "bob")] {
        let join = format!(
            r#"{{"cap":"players","event":"joined","participant":{{"id":{id},"name":"{name}"}}}}"#
        );
        world.update(0, join.as_bytes())?;
    }
    const MOVES: [&[u8]; 4] = [b"l", b"j", b"h", b"k"];
    let applied_start = Instant::now();
    for seq in 0..events {
        world.update(seq % 2 + 1, MOVES[(seq % 4) as usize])?;
    }
    let applied_secs = applied_start.elapsed().as_secs_f64();
    let mut view_walls = Vec::new();
    let mut layout_walls = Vec::new();
    for _ in 0..5 {
        let start = Instant::now();
        black_box(world.view()?);
        view_walls.push(start.elapsed().as_secs_f64() * 1e6);
        let start = Instant::now();
        black_box(world.layout_at(40, 20)?);
        layout_walls.push(start.elapsed().as_secs_f64() * 1e6);
    }
    let metrics = world.metrics();
    let view = metrics.get(GuestCall::View);
    let layout = metrics.get(GuestCall::Layout);
    let update = metrics.get(GuestCall::Update);
    println!(
        "profile frame_cost guest=territory seats=2 events={events} view_fuel={} view_wall_us_med={:.1} layout_fuel={} layout_calls={} layout_wall_us_med={:.1} update_fuel_max={} applied_secs={applied_secs:.3}",
        view.fuel_last,
        median(view_walls),
        layout.fuel_last,
        layout.calls,
        median(layout_walls),
        update.fuel_max,
    );
    Ok(())
}

/// The largest journal tail that can sit behind the latest
/// checkpoint — the mid-interval worst case of the join design.
/// Session checkpoints fall due at 256 frames or 2,000 ms, whichever
/// comes first (`CheckpointPolicy` in
/// `crates/id-store/src/session_checkpoint.rs`: the frame limit is
/// `DEFAULT_CHECKPOINT_EVENT_LIMIT`, the interval
/// `DEFAULT_CHECKPOINT_INTERVAL_MS`), so a journal that has not
/// checkpointed again carries at most 255 events past its latest
/// checkpoint.
const JOIN_TAIL_MAX: u64 = 255;

/// The bench guest of the join profile: an in-process counting
/// program in the `guest_replay` mould. Its whole state is the
/// number of participant inputs applied, so its checkpoint snapshot
/// is that count in decimal and its view frame renders the count —
/// enough state to prove a join rebuilt the same program the
/// manifest-verified journal describes, with the same accounting
/// the generator itself uses (sequence + replay inputs). The wasm
/// sandbox path is environment-gated (see `guest_replay`), so this
/// measures the host-side join pipeline every guest join pays for;
/// sandbox trap costs are on top.
struct JoinProgram {
    inputs: u64,
}

impl JoinProgram {
    /// Instantiate the program the way the service does after a
    /// restore: from the checkpoint snapshot when the journal was
    /// compacted, from a fresh init otherwise. The tail inputs in
    /// `restored.replay` are then applied by the caller.
    fn from_restored(restored: &RestoredWorld) -> Result<Self> {
        let inputs = match restored.program.as_ref().and_then(|p| p.snapshot.as_ref()) {
            Some(snapshot) => snapshot
                .parse()
                .with_context(|| format!("parse join program snapshot {snapshot:?}"))?,
            None => 0,
        };
        Ok(Self { inputs })
    }

    fn frame(&self) -> String {
        format!("join-frame inputs={}", self.inputs)
    }
}

impl WorldProgram for JoinProgram {
    fn update(&mut self, event: &WorldEvent) -> Result<()> {
        if is_input_event(event) {
            self.inputs += 1;
        }
        Ok(())
    }

    fn view(&mut self, _viewer: &Participant) -> Result<Option<String>> {
        Ok(Some(self.frame()))
    }
}

/// Whether an event is a participant input — the events a restored
/// world replays through its program (chat is host-managed history,
/// never program input).
fn is_input_event(event: &WorldEvent) -> bool {
    matches!(
        event.kind,
        WorldEventKind::Input(_) | WorldEventKind::EntityInput { .. }
    )
}

/// Nearest-rank percentile of an ascending-sorted sample set.
fn join_percentile(sorted: &[f64], pct: f64) -> f64 {
    let rank = (pct / 100.0 * sorted.len() as f64).ceil() as usize;
    sorted[rank.clamp(1, sorted.len()) - 1]
}

/// The generated stress fixture the join profile measures against:
/// the tier's world set plus its manifest, generated on demand
/// through the `stress_gen` profile when no completed generation
/// for this (tier, seed) is on disk. The join profile never writes
/// into the fixture root — its checkpoint variants live in its own
/// scratch dir — so a later `stress_gen` run still finds the
/// generation it published.
struct StressFixture {
    tier: StressTier,
    seed: u64,
    out: PathBuf,
    manifest: serde_json::Value,
}

fn ensure_stress_fixture(quick: bool) -> Result<StressFixture> {
    let (tier, seed) = stress_selection(quick)?;
    let out = std::env::var("ID_BENCH_STRESS_OUT").map_or_else(
        |_| std::env::temp_dir().join(format!("id-stress-{}-{}", tier.name, seed)),
        PathBuf::from,
    );
    let manifest_path = out.join("manifest.json");
    let reusable = stress_output::stress_dir_is_completed_generation(&out)
        && std::fs::read_to_string(&manifest_path)
            .ok()
            .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
            .is_some_and(|manifest| {
                manifest["seed"].as_u64() == Some(seed)
                    && manifest["tier"].as_str() == Some(tier.name)
                    && manifest["generator_version"].as_u64() == Some(loadgen::GENERATOR_VERSION)
                    && manifest["hash"]["input"].as_str() == Some(STRESS_STATE_HASH_INPUT)
            });
    if !reusable {
        bench_stress_gen(quick)?;
    }
    let manifest: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&manifest_path)
            .with_context(|| format!("read stress manifest {manifest_path:?}"))?,
    )
    .context("parse stress manifest")?;
    ensure!(
        manifest["seed"].as_u64() == Some(seed) && manifest["tier"].as_str() == Some(tier.name),
        "stress manifest at {manifest_path:?} does not describe tier {} seed {seed}",
        tier.name
    );
    Ok(StressFixture {
        tier,
        seed,
        out,
        manifest,
    })
}

/// Rewrite a generated stress journal as the same world seen from
/// behind a checkpoint at `checkpoint_at`: header, the generated
/// participant entries, one inline `Checkpoint` carrying the
/// retained window and the bench program's snapshot, then the
/// committed tail past the checkpoint. The shape mirrors what
/// `WorldCore::checkpoint_entries` writes on compaction, in the
/// legacy admin-history form (`admin_history: None`): restore
/// supports the legacy form explicitly, and the retained window
/// (the checkpoint's dominant term) is carried in full. The
/// retained window at these sizes serializes under the
/// writer's own inline bound (`MAX_CHECKPOINT_PART_BYTES`), so the
/// single inline entry is the form the writer itself would choose
/// for a checkpoint of this size without the admin window.
fn join_checkpoint_journal(
    name: &str,
    generated: &[JournalEntry],
    checkpoint_at: u64,
    seed: u64,
) -> Result<Vec<JournalEntry>> {
    let mut issued: Vec<JournalEntry> = Vec::new();
    let mut events: Vec<WorldEvent> = Vec::new();
    for entry in generated {
        match entry {
            JournalEntry::Created { .. } => {}
            JournalEntry::Issued { .. } => issued.push(entry.clone()),
            JournalEntry::Committed { event } => events.push(event.clone()),
            _ => anyhow::bail!(
                "generated stress journal for {name} holds an entry the join fixture does not model"
            ),
        }
    }
    let at = checkpoint_at as usize;
    ensure!(
        at <= events.len(),
        "checkpoint at {checkpoint_at} is past {name}'s {} events",
        events.len()
    );
    let retained = WorldLimits::default().retained_events;
    let window: Vec<WorldEvent> = events[at.saturating_sub(retained)..at].to_vec();
    // The snapshot is the bench program's input count at the
    // checkpoint: inputs applied to the currently installed program.
    // An install replaces the program, so inputs committed before
    // the latest install at or before the checkpoint are not part of
    // its state and do not count (#16042).
    let mut snapshot_inputs = 0_u64;
    for event in &events[..at] {
        match &event.kind {
            WorldEventKind::ProgramInstalled { .. } => snapshot_inputs = 0,
            _ if is_input_event(event) => snapshot_inputs += 1,
            _ => {}
        }
    }
    let snapshot = snapshot_inputs.to_string();
    let mut entries = Vec::with_capacity(issued.len() + 2 + (events.len() - at));
    // The header version is the maximum the entries require: the
    // checkpoint's `open_presence` is the binding requirement
    // (JOURNAL_VERSION_OPEN_PRESENCE, 5) for these fixtures — the
    // same bump compaction performs when it re-emits a journal.
    entries.push(JournalEntry::Created {
        world_id: name.to_owned(),
        version: 5,
    });
    entries.extend(issued);
    entries.push(JournalEntry::Checkpoint {
        sequence: checkpoint_at,
        module_hash: "bench-join-program".to_owned(),
        seed,
        provenance: None,
        execution_profile: Some(id::world_limits::WorldExecutionProfile::default()),
        caps_ceiling: None,
        schedules: None,
        kv: None,
        blobs: None,
        transfers: None,
        lease: None,
        snapshot,
        events: window,
        admin_history: None,
        open_presence: Some(Vec::new()),
    });
    entries.extend(
        events[at..]
            .iter()
            .cloned()
            .map(|event| JournalEntry::Committed { event }),
    );
    Ok(entries)
}

/// Serialize journal entries in the store's on-disk line format.
fn write_journal_file(path: &Path, entries: &[JournalEntry]) -> Result<()> {
    let mut bytes: Vec<u8> = Vec::new();
    for entry in entries {
        serde_json::to_writer(&mut bytes, entry).context("serialize join fixture entry")?;
        bytes.push(b'\n');
    }
    std::fs::write(path, &bytes).with_context(|| format!("write join fixture journal {path:?}"))?;
    Ok(())
}

/// Which of the design's two journal states a join measures.
#[derive(Clone, Copy)]
enum JoinState {
    /// Journal at a checkpoint boundary: the latest checkpoint
    /// covers the whole history, the tail is empty.
    Boundary,
    /// Mid-interval worst case: the largest tail the checkpoint
    /// cadence allows behind the latest checkpoint.
    Mid,
}

impl JoinState {
    fn name(self) -> &'static str {
        match self {
            Self::Boundary => "checkpoint_boundary",
            Self::Mid => "mid_interval",
        }
    }
}

/// One measured world of the join profile: the manifest-verified
/// reference state plus the two journal forms the design measures.
struct JoinWorldFixture {
    name: String,
    events: u64,
    total_inputs: u64,
    boundary_journal: PathBuf,
    mid_journal: PathBuf,
    mid_tail: u64,
    /// Highest participant id the generated journal issues: the
    /// roster member warm setup revokes when the world already
    /// runs at the participant cap and cannot mint a fresh joiner.
    last_member_id: Option<u64>,
}

impl JoinWorldFixture {
    fn frame(&self) -> String {
        format!("join-frame inputs={}", self.total_inputs)
    }

    fn journal(&self, state: JoinState) -> &Path {
        match state {
            JoinState::Boundary => &self.boundary_journal,
            JoinState::Mid => &self.mid_journal,
        }
    }

    fn tail(&self, state: JoinState) -> u64 {
        match state {
            JoinState::Boundary => 0,
            JoinState::Mid => self.mid_tail,
        }
    }
}

fn build_join_world(
    fixture: &StressFixture,
    dir: &Path,
    record: &serde_json::Value,
) -> Result<JoinWorldFixture> {
    let name = record["name"]
        .as_str()
        .context("manifest world without a name")?
        .to_owned();
    let events = record["events"]
        .as_u64()
        .context("manifest world without events")?;
    let expected_hash = record["expected_state_hash"]
        .as_str()
        .context("manifest world without expected_state_hash")?;
    let generated_path = fixture.out.join(&name).join("journal.jsonl");
    let reference_path = dir.join(format!("{name}-reference.jsonl"));
    std::fs::copy(&generated_path, &reference_path)
        .with_context(|| format!("copy generated journal {generated_path:?}"))?;

    let (journal, entries) = FileJournal::open(&reference_path, &name)?;
    drop(journal);
    let restored = WorldCore::restore(&name, WorldLimits::default(), entries.clone())
        .map_err(|e| anyhow::anyhow!("restore generated stress world {name}: {e:#}"))?;
    ensure!(
        restored.core.current_sequence() == events,
        "{name}: restored at sequence {}, manifest records {events}",
        restored.core.current_sequence()
    );
    // The manifest verification the generator itself defines: the
    // canonical restored-state hash over what this restore rebuilt
    // (sequence, installed program, replay state).
    let state_hash = stress_state_hash(&restored);
    ensure!(
        state_hash == expected_hash,
        "{name}: restored state hash {state_hash} does not match the manifest's {expected_hash}"
    );
    ensure!(
        restored
            .replay
            .iter()
            .all(|event| !matches!(event.kind, WorldEventKind::System { .. })),
        "{name}: reference replay holds a host system event; the join accounting does not model it"
    );
    let total_inputs = restored.replay.iter().filter(|e| is_input_event(e)).count() as u64;
    let last_member_id = entries
        .iter()
        .filter_map(|entry| match entry {
            JournalEntry::Issued { participant, .. } => Some(participant.id),
            _ => None,
        })
        .max();

    let boundary_journal = dir.join(format!("{name}-boundary.jsonl"));
    write_journal_file(
        &boundary_journal,
        &join_checkpoint_journal(&name, &entries, events, fixture.seed)?,
    )?;
    let (mid_journal, mid_tail) = if events > JOIN_TAIL_MAX {
        let path = dir.join(format!("{name}-mid.jsonl"));
        write_journal_file(
            &path,
            &join_checkpoint_journal(&name, &entries, events - JOIN_TAIL_MAX, fixture.seed)?,
        )?;
        (path, JOIN_TAIL_MAX)
    } else {
        // A world younger than one checkpoint segment has not
        // checkpointed yet: its generated journal is itself the
        // mid-interval state, the whole history its tail.
        (reference_path.clone(), events)
    };

    // Construction proof, untimed: both journal forms restore to
    // the manifest-verified state — same sequence, and the bench
    // program ends with every input applied exactly once.
    for path in [&boundary_journal, &mid_journal] {
        let (restored, program) = join_restore(path, &name)?;
        ensure!(
            restored.core.current_sequence() == events && program.inputs == total_inputs,
            "{name}: join fixture {path:?} restores to sequence {} with {} inputs, \
             reference is {events} / {total_inputs}",
            restored.core.current_sequence(),
            program.inputs
        );
    }

    Ok(JoinWorldFixture {
        name,
        events,
        total_inputs,
        boundary_journal,
        mid_journal,
        mid_tail,
        last_member_id,
    })
}

/// Restore one journal form and instantiate its bench program,
/// applying the restored tail inputs — the untimed twin of the
/// cold-join measurement, used to prove fixture construction and
/// to build resident worlds for warm joins.
fn join_restore(path: &Path, name: &str) -> Result<(RestoredWorld, JoinProgram)> {
    let (journal, entries) = FileJournal::open(path, name)?;
    let restored = WorldCore::restore(name, WorldLimits::default(), entries)?;
    drop(journal);
    let mut program = JoinProgram::from_restored(&restored)?;
    for event in &restored.replay {
        program.update(event)?;
    }
    Ok((restored, program))
}

/// One measured cold join: open the world store, load the latest
/// checkpoint, replay the journal tail through the program, and
/// produce the first view frame. The joined state is verified
/// against the manifest-verified reference after the clock stops.
fn join_cold_once(path: &Path, viewer: &Participant, expected: &JoinWorldFixture) -> Result<f64> {
    let started = Instant::now();
    let (journal, entries) = FileJournal::open(path, &expected.name)?;
    let restored = WorldCore::restore(&expected.name, WorldLimits::default(), entries)?;
    let mut program = JoinProgram::from_restored(&restored)?;
    for event in &restored.replay {
        program.update(event)?;
    }
    let frame = program.view(viewer)?;
    let elapsed_us = started.elapsed().as_secs_f64() * 1e6;
    drop(journal);
    ensure!(
        restored.core.current_sequence() == expected.events
            && program.inputs == expected.total_inputs
            && frame.as_deref() == Some(expected.frame().as_str()),
        "{}: cold join rebuilt sequence {} / {} inputs / frame {frame:?}, \
         reference is {} / {} / {:?}",
        expected.name,
        restored.core.current_sequence(),
        program.inputs,
        expected.events,
        expected.total_inputs,
        expected.frame()
    );
    Ok(elapsed_us)
}

/// A resident world for warm joins: restored core, live program,
/// and a joiner capability minted once in setup.
struct ResidentJoinWorld {
    core: WorldCore,
    program: JoinProgram,
    token: JoinCapability,
    viewer: Participant,
}

fn join_resident(path: &Path, expected: &JoinWorldFixture) -> Result<ResidentJoinWorld> {
    let (restored, program) = join_restore(path, &expected.name)?;
    ensure!(
        restored.core.current_sequence() == expected.events
            && program.inputs == expected.total_inputs,
        "{}: resident world restores to sequence {} / {} inputs, reference is {} / {}",
        expected.name,
        restored.core.current_sequence(),
        program.inputs,
        expected.events,
        expected.total_inputs
    );
    let mut core = restored.core;
    let joiner_scopes = WorldScopes::JOIN
        .union(WorldScopes::CHAT)
        .union(WorldScopes::HISTORY);
    let (viewer, token) = match core.issue_capability(
        "bench-joiner",
        joiner_scopes,
        CapabilityBounds {
            expires_at: None,
            uses: None,
        },
    ) {
        Ok(pair) => pair,
        Err(error) => {
            // A world already at the participant cap (the 10x hot
            // world runs at the 256-member codified cap) cannot mint
            // a fresh joiner. Free one slot in the resident core —
            // memory only; the journal on disk is untouched — by
            // revoking the last roster member, then mint again.
            ensure!(
                format!("{error:#}").contains("world participant limit reached"),
                "minting the warm joiner failed: {error:#}"
            );
            let member = expected
                .last_member_id
                .context("world at the participant cap has no roster member to revoke")?;
            core.revoke_tree(member);
            core.issue_capability(
                "bench-joiner",
                joiner_scopes,
                CapabilityBounds {
                    expires_at: None,
                    uses: None,
                },
            )
            .context("minting the warm joiner after revoking one roster member")?
        }
    };
    Ok(ResidentJoinWorld {
        core,
        program,
        token,
        viewer,
    })
}

/// One measured warm join against a resident world: the host
/// projects the joiner's catch-up snapshot from the live core and
/// the live program produces their first view frame.
fn join_warm_once(resident: &mut ResidentJoinWorld, expected: &JoinWorldFixture) -> Result<f64> {
    let started = Instant::now();
    let snapshot = resident.core.snapshot(&resident.token)?;
    let frame = resident.program.view(&resident.viewer)?;
    let elapsed_us = started.elapsed().as_secs_f64() * 1e6;
    ensure!(
        snapshot.current_sequence == expected.events
            && frame.as_deref() == Some(expected.frame().as_str()),
        "{}: warm join produced sequence {} / frame {frame:?}, reference is {} / {:?}",
        expected.name,
        snapshot.current_sequence,
        expected.events,
        expected.frame()
    );
    black_box(&snapshot);
    Ok(elapsed_us)
}

fn report_join(
    mode: &str,
    tier: &StressTier,
    role: &str,
    state: JoinState,
    world: &JoinWorldFixture,
    worlds: usize,
    mut samples: Vec<f64>,
) {
    samples.sort_by(|a, b| a.total_cmp(b));
    println!(
        "bench stress_join_{mode} unit=us_per_join median={:.3} min={:.3} reps={} tier={} world={} journal={} p50_us={:.3} p95_us={:.3} p99_us={:.3} events={} tail_events={} worlds={worlds}",
        join_percentile(&samples, 50.0),
        samples[0],
        samples.len(),
        tier.name,
        role,
        state.name(),
        join_percentile(&samples, 50.0),
        join_percentile(&samples, 95.0),
        join_percentile(&samples, 99.0),
        world.events,
        world.tail(state),
    );
}

/// The `stress_join` profile (#13084): cold and warm join time for
/// the stress tiers' worlds, each measured with the journal at a
/// checkpoint boundary and at the mid-interval worst case, against
/// the `stress_gen` fixture. The hot world (the tier's single-world
/// history axis) is measured in reps — the run protocol's five;
/// the active worlds are measured as a population, one join each,
/// so their percentiles are population percentiles. Tier and seed
/// follow the `stress_gen` knobs.
pub fn bench_stress_join(quick: bool) -> Result<()> {
    let fixture = ensure_stress_fixture(quick)?;
    let dir = std::env::temp_dir().join(format!("id-stress-join-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    let worlds = fixture.manifest["worlds"]
        .as_array()
        .context("stress manifest has no worlds list")?;
    let hot_record = worlds
        .iter()
        .find(|world| world["name"].as_str() == Some("hot-000"))
        .context("stress manifest has no hot world")?;
    let hot = build_join_world(&fixture, &dir, hot_record)?;
    let mut actives: Vec<JoinWorldFixture> = Vec::new();
    for record in worlds
        .iter()
        .filter(|world| world["role"].as_str() == Some("active"))
    {
        actives.push(build_join_world(&fixture, &dir, record)?);
    }
    ensure!(!actives.is_empty(), "stress manifest has no active worlds");
    println!(
        "profile stress_join tier={} seed={} fixture_out={} manifest={} hot_state_hash={} active_worlds={}",
        fixture.tier.name,
        fixture.seed,
        fixture.out.display(),
        fixture.out.join("manifest.json").display(),
        hot_record["expected_state_hash"].as_str().unwrap_or("?"),
        actives.len(),
    );

    let viewer = Participant {
        id: 1,
        display_name: "stress-0".to_owned(),
    };
    const REPS: usize = 5;
    for state in [JoinState::Boundary, JoinState::Mid] {
        let mut samples = Vec::with_capacity(REPS);
        for _ in 0..REPS {
            samples.push(join_cold_once(hot.journal(state), &viewer, &hot)?);
        }
        report_join("cold", &fixture.tier, "hot", state, &hot, 1, samples);

        let mut resident = join_resident(hot.journal(state), &hot)?;
        let mut samples = Vec::with_capacity(REPS);
        for _ in 0..REPS {
            samples.push(join_warm_once(&mut resident, &hot)?);
        }
        report_join("warm", &fixture.tier, "hot", state, &hot, 1, samples);
        drop(resident);

        let mut samples = Vec::with_capacity(actives.len());
        for world in &actives {
            samples.push(join_cold_once(world.journal(state), &viewer, world)?);
        }
        report_join(
            "cold",
            &fixture.tier,
            "active",
            state,
            &actives[0],
            actives.len(),
            samples,
        );

        let mut samples = Vec::with_capacity(actives.len());
        for world in &actives {
            let mut resident = join_resident(world.journal(state), world)?;
            samples.push(join_warm_once(&mut resident, world)?);
        }
        report_join(
            "warm",
            &fixture.tier,
            "active",
            state,
            &actives[0],
            actives.len(),
            samples,
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use anyhow::{Context as _, Result};
    use id::world::{JournalEntry, WorldCore, WorldEvent, WorldEventKind, WorldLimits};

    /// Oracle: the declared mix table itself — 700 chat / 200 doc
    /// edits / 100 program installs per 1,000 events — plus the
    /// fixture-size windows the tiers actually generate (20-event
    /// active worlds, 100-event active worlds, the 200-event quick
    /// hot world), each of which must contain all three shapes.
    #[test]
    fn stress_schedule_interleaves_the_declared_mix() {
        let counts = |window: u64| {
            let (mut chat, mut doc, mut install) = (0_u64, 0_u64, 0_u64);
            for index in 0..window {
                match stress_shape_at(index) {
                    StressShape::Chat => chat += 1,
                    StressShape::DocEdit => doc += 1,
                    StressShape::ProgramInstall => install += 1,
                }
            }
            (chat, doc, install)
        };
        assert_eq!(counts(1_000), (700, 200, 100));
        assert_eq!(counts(20), (14, 4, 2));
        assert_eq!(counts(100), (70, 20, 10));
        assert_eq!(counts(200), (140, 40, 20));
        // Every aligned 10-event window holds all three shapes.
        for start in (0..1_000).step_by(10) {
            let shapes: Vec<StressShape> = (start..start + 10).map(stress_shape_at).collect();
            assert!(shapes.contains(&StressShape::Chat));
            assert!(shapes.contains(&StressShape::DocEdit));
            assert!(shapes.contains(&StressShape::ProgramInstall));
        }
    }

    /// Oracle: `WorldCore::restore` over the generated entries — a
    /// program install must surface as the restored world's active
    /// program (the bench counter artifact at the run seed), and the
    /// restored replay must hold exactly the inputs committed after
    /// the last install.
    #[test]
    fn stress_program_install_restores_a_real_program() -> Result<()> {
        let (entries, chat, doc_edits, program_installs) =
            stress_world_entries(42, "stress-test-world", 0, 20, 1)?;
        assert_eq!((chat, doc_edits, program_installs), (14, 4, 2));
        let installs: Vec<&WorldEvent> = entries
            .iter()
            .filter_map(|entry| match entry {
                JournalEntry::Committed { event }
                    if matches!(event.kind, WorldEventKind::ProgramInstalled { .. }) =>
                {
                    Some(event)
                }
                _ => None,
            })
            .collect();
        assert_eq!(installs.len(), 2);
        for event in &installs {
            assert_eq!(event.participant_id, 0, "installs are host-authored");
            match &event.kind {
                WorldEventKind::ProgramInstalled {
                    module_hash, seed, ..
                } => {
                    assert_eq!(module_hash, &stress_program_module_hash());
                    assert_eq!(*seed, 42);
                }
                _ => unreachable!(),
            }
        }
        let expected_replay = stress_expected_replay_len(&entries);
        let restored = WorldCore::restore("stress-test-world", WorldLimits::default(), entries)?;
        let program = restored
            .program
            .as_ref()
            .context("restored stress world has no program")?;
        assert_eq!(program.module_hash, stress_program_module_hash());
        assert_eq!(program.seed, 42);
        // The last install is event 16 of 20; the only doc edit after
        // it is event 19, so the replay holds exactly that one input.
        assert_eq!(restored.replay.len(), 1);
        assert_eq!(restored.replay.len() as u64, expected_replay);
        Ok(())
    }

    /// Oracle: the hash definition itself — the same (seed, world)
    /// restored twice hashes identically; a different seed diverges
    /// (different program seed and corpus); and a different world at
    /// the same seed and the same restored sequence diverges too,
    /// because its replay state differs: the hash is over restored
    /// semantic state, not over journal bytes and sequence alone.
    #[test]
    fn stress_state_hash_tracks_restored_state() -> Result<()> {
        let hash_of = |seed: u64, world_index: u32, events: u64| -> Result<String> {
            let (entries, ..) =
                stress_world_entries(seed, "stress-hash-world", world_index, events, 1)?;
            let restored =
                WorldCore::restore("stress-hash-world", WorldLimits::default(), entries)?;
            Ok(stress_state_hash(&restored))
        };
        assert_eq!(hash_of(42, 0, 20)?, hash_of(42, 0, 20)?);
        assert_ne!(hash_of(42, 0, 20)?, hash_of(43, 0, 20)?);
        assert_ne!(hash_of(42, 0, 20)?, hash_of(42, 1, 20)?);
        Ok(())
    }
}
