//! Session recording to replayable fixtures.
//!
//! A fixture is a JSON Lines file: a header line, then one line per retained,
//! sequenced or host-originated event, in journal order. [`record`] reads one
//! from a live world through an admin-authorized export; [`replay`] runs one offline through the sandbox under a
//! seed the operator supplies. A fixture holds no seed, no capability, and no
//! module bytes, and it adds no authority to the capability that recorded it.
//! [`verify`] replays a fixture twice and compares the records after every
//! event, as the determinism self-check behind `id world verify`.
//! The design record is `doc/2026-10-17T09-00-00Z_feature_world_fixtures`.

use anyhow::{Context, Result, bail, ensure};
use serde_json::{Map, Value};

#[cfg(feature = "sandbox")]
use crate::sandbox::Sandbox;
#[cfg(feature = "sandbox")]
use crate::world::WorldProgram;
use crate::world::{Records, WorldEvent, WorldEventKind, parse_records};
use crate::world_net::WorldClient;
use crate::world_session::{decode_hex, encode_hex, module_hash};

/// The only fixture version this build writes or reads.
pub const FIXTURE_VERSION: u64 = 1;

/// Largest fixture file the CLI reads, in bytes. It exceeds what a world with
/// default limits can record: 1024 retained inputs of 16 KiB, hex-encoded,
/// plus 4096 records of 8 KiB each.
pub const MAX_FIXTURE_BYTES: u64 = 128 * 1024 * 1024;

/// The `name` of the install boundary in an `other` line.
pub const INSTALL_NAME: &str = "program_installed";

/// The first line of a fixture: which world, which module, the head, and the
/// records the head must reproduce. Never the seed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FixtureHeader {
    /// The world the history belongs to.
    pub world_id: String,
    /// Iroh hash of the module active at `current_sequence`, if any.
    pub module_hash: Option<String>,
    /// Sequence of the last recorded event; the records were read here.
    pub current_sequence: u64,
    /// True when the history starts at sequence one and needed no snapshot.
    pub complete: bool,
    /// The host's published records at `current_sequence`, if recorded.
    pub records: Option<Records>,
}

/// One recorded event, in the fixture's own vocabulary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FixtureEvent {
    /// A chat line. Host-managed: a program never receives it.
    Chat {
        /// Committed sequence.
        sequence: u64,
        /// Participant who sent the line.
        participant_id: u64,
        /// The line's text.
        text: String,
    },
    /// Opaque input from a participant.
    Input {
        /// Committed sequence.
        sequence: u64,
        /// Participant who sent the input.
        participant_id: u64,
        /// The input bytes, as the program receives them.
        bytes: Vec<u8>,
    },
    /// A host event the program received: presence, capability results, ticks.
    /// It is available only in an admin-authorized export and is never sequenced.
    System {
        /// The event's JSON.
        event: String,
    },
    /// Any other event kind, by name. Only [`INSTALL_NAME`] can be replayed.
    Other {
        /// Committed sequence.
        sequence: u64,
        /// The kind's name, such as [`INSTALL_NAME`].
        name: String,
    },
}

impl FixtureEvent {
    /// The committed sequence. Host events are never sequenced and report zero.
    #[must_use]
    pub const fn sequence(&self) -> u64 {
        match self {
            Self::Chat { sequence, .. }
            | Self::Input { sequence, .. }
            | Self::Other { sequence, .. } => *sequence,
            Self::System { .. } => 0,
        }
    }
}

impl From<&WorldEvent> for FixtureEvent {
    fn from(event: &WorldEvent) -> Self {
        let sequence = event.sequence;
        match &event.kind {
            WorldEventKind::Chat(text) => Self::Chat {
                sequence,
                participant_id: event.participant_id,
                text: text.clone(),
            },
            WorldEventKind::Input(bytes) => Self::Input {
                sequence,
                participant_id: event.participant_id,
                bytes: bytes.clone(),
            },
            WorldEventKind::EntityInput { input, .. } => Self::Input {
                sequence,
                participant_id: event.participant_id,
                bytes: input.clone(),
            },
            WorldEventKind::System { event: text } => Self::System {
                event: text.clone(),
            },
            WorldEventKind::ProgramInstalled { .. } => Self::Other {
                sequence,
                name: INSTALL_NAME.to_owned(),
            },
        }
    }
}

/// A parsed fixture: the header and the events after it, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fixture {
    /// The header line.
    pub header: FixtureHeader,
    /// The event lines, in the order they were recorded.
    pub events: Vec<FixtureEvent>,
}

/// How a replay's records compare with the fixture's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// The replayed records equal the recorded ones, as JSON values.
    Match,
    /// The replayed records differ from the recorded ones.
    Mismatch,
    /// Nothing was compared; the reason says why.
    NotCompared(&'static str),
}

/// What a replay produced, and how its records compared.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Replayed {
    /// The program's records after the last event, as the guest wrote them.
    pub records: Option<String>,
    /// The program's view after the last event.
    pub view: Option<String>,
    /// How the replayed records compare with the fixture's.
    pub verdict: Verdict,
}

/// The records of one replay at one step: after the event at `sequence`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordSnapshot {
    /// Sequence of the event after which the records were read.
    pub sequence: u64,
    /// The program's records at that step (empty when it keeps none).
    pub records: Records,
}

/// The first step at which two replays of the same fixture disagree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Divergence {
    /// Sequence of the event after which the records first differ.
    pub sequence: u64,
    /// The first record key, in sorted order, whose value differs.
    pub key: String,
    /// The key's value in the first replay (`None`: the key is absent).
    pub first: Option<Value>,
    /// The key's value in the second replay (`None`: the key is absent).
    pub second: Option<Value>,
}

impl std::fmt::Display for Divergence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = |v: &Option<Value>| {
            v.as_ref()
                .map_or_else(|| "<absent>".to_owned(), Value::to_string)
        };
        write!(
            f,
            "sequence {}: record `{}` differs (first replay: {}, second replay: {})",
            self.sequence,
            self.key,
            value(&self.first),
            value(&self.second)
        )
    }
}

/// The outcome of [`verify`]: two replays of one fixture, compared step by step.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Verification {
    /// The first replay, as [`replay`] would return it.
    pub first: Replayed,
    /// The second replay, as [`replay`] would return it.
    pub second: Replayed,
    /// How many record snapshots were compared.
    pub steps: usize,
    /// The first divergence between the two replays, if any.
    pub divergence: Option<Divergence>,
}

impl Verification {
    /// True when the two replays agreed at every compared step.
    #[must_use]
    pub const fn is_deterministic(&self) -> bool {
        self.divergence.is_none()
    }
}

/// Compare two step-by-step record sequences from replays of the same
/// fixture and report the first divergence, if any.
///
/// Steps are compared in order; within a step, record keys are compared in
/// sorted (map) order, so the reported divergence is the earliest step and,
/// inside it, the first differing key. A step present in only one sequence
/// diverges at that step.
#[must_use]
pub fn first_divergence(first: &[RecordSnapshot], second: &[RecordSnapshot]) -> Option<Divergence> {
    let steps = first.len().max(second.len());
    for index in 0..steps {
        let (Some(a), Some(b)) = (first.get(index), second.get(index)) else {
            // One sequence ended: the extra step is the divergence.
            let extra = first.get(index).or(second.get(index))?;
            let key = extra
                .records
                .keys()
                .next()
                .cloned()
                .unwrap_or_else(|| "<records>".to_owned());
            return Some(Divergence {
                sequence: extra.sequence,
                key: key.clone(),
                first: first.get(index).and_then(|s| s.records.get(&key)).cloned(),
                second: second.get(index).and_then(|s| s.records.get(&key)).cloned(),
            });
        };
        if a.records == b.records {
            continue;
        }
        // First key, in sorted order, whose value differs (or is absent).
        let mut keys: Vec<&String> = a.records.keys().chain(b.records.keys()).collect();
        keys.sort();
        keys.dedup();
        for key in keys {
            let (va, vb) = (a.records.get(key), b.records.get(key));
            if va != vb {
                return Some(Divergence {
                    sequence: a.sequence.max(b.sequence),
                    key: key.clone(),
                    first: va.cloned(),
                    second: vb.cloned(),
                });
            }
        }
    }
    None
}

/// Serialize a fixture: the header line, then one line per event.
///
/// The same header and events always give the same text.
#[must_use]
pub fn encode(header: &FixtureHeader, events: &[FixtureEvent]) -> String {
    let mut text = header_line(header);
    text.push('\n');
    for event in events {
        text.push_str(&event_line(event));
        text.push('\n');
    }
    text
}

/// Parse a fixture, refusing anything it cannot replay faithfully.
///
/// # Errors
///
/// An unknown version, a malformed or unexpected line, a blank line, or a
/// window whose sequenced events are not contiguous, exceed the header's
/// sequence, or (when complete) do not start at one and reach the header.
pub fn decode(text: &str) -> Result<Fixture> {
    let mut lines = text.lines();
    let header = decode_header(lines.next().context("the fixture is empty")?)?;
    let mut events = Vec::new();
    for (index, line) in lines.enumerate() {
        let number = index + 2;
        events.push(decode_event(line).with_context(|| format!("fixture line {number}"))?);
    }
    check_window(&header, &events)?;
    Ok(Fixture { header, events })
}

/// Read a world into a fixture through the host's admin-authorized export.
/// Sequenced and host-originated events, as well as their order and the records
/// at the head, are captured by one actor operation.
///
/// # Errors
///
/// A missing or invalid admin token, a broken stream, or malformed history.
pub async fn record(client: &mut WorldClient, admin_token: &str) -> Result<Fixture> {
    let history = client.admin_event_history(admin_token).await?;
    let events: Vec<FixtureEvent> = history.events.iter().map(FixtureEvent::from).collect();
    let header = FixtureHeader {
        world_id: history.world_id,
        module_hash: history.module_hash,
        current_sequence: history.current_sequence,
        complete: history.complete,
        records: Some(history.records),
    };
    check_window(&header, &events)?;
    Ok(Fixture { header, events })
}

/// Check that `wasm` is the module the fixture was recorded under.
///
/// # Errors
///
/// The fixture records no active module, or the bytes hash to a different one.
pub fn check_module(header: &FixtureHeader, wasm: &[u8]) -> Result<()> {
    let recorded = header
        .module_hash
        .as_deref()
        .context("the fixture records no active module")?;
    let actual = module_hash(wasm);
    ensure!(
        actual == recorded,
        "the module hashes to {actual}, but the fixture was recorded under {recorded}"
    );
    Ok(())
}

/// Replay a fixture through `runner` under `seed`, offline.
///
/// `runner` must be the module the fixture was recorded under; call
/// [`check_module`] first, because this function cannot see the module's bytes.
/// The module is the code that runs, in the sandbox a live world uses. The
/// fixture supplies only inputs and the records they must reproduce.
///
/// A window that does not start at sequence one is refused unless
/// `allow_partial` is set, and its result is never compared.
///
/// # Errors
///
/// A partial window without `allow_partial`; a recording that spans an
/// install; an event kind that cannot be replayed; or a guest failure.
#[cfg(feature = "sandbox")]
pub fn replay(
    runner: &Sandbox,
    seed: u64,
    fixture: &Fixture,
    allow_partial: bool,
) -> Result<Replayed> {
    let header = &fixture.header;
    ensure!(
        header.complete || allow_partial,
        "the fixture is partial: its history does not start at sequence 1, so a fresh replay cannot reproduce it; pass --allow-partial to replay the retained window, whose records are then not compared"
    );
    ensure!(
        header.module_hash.is_some(),
        "the fixture records no active module, so there is nothing to replay"
    );
    let start = replay_start(&fixture.events)?;
    let mut world = runner.instantiate(seed)?;
    for event in &fixture.events[start..] {
        if let Some(world_event) = replay_event(event) {
            WorldProgram::update(&mut world, &world_event)
                .with_context(|| format!("replay sequence {}", world_event.sequence))?;
        }
    }
    let records = WorldProgram::records(&mut world).context("replay records")?;
    let view = String::from_utf8(world.view()?).context("the replayed view is not UTF-8")?;
    let verdict = verdict(header, records.as_deref())?;
    Ok(Replayed {
        records,
        view: Some(view),
        verdict,
    })
}

/// Verify a fixture's determinism: replay it twice under `seed` and compare
/// the records after every event the program sees, plus the final records.
///
/// This is the self-check behind `id world verify` (F11.16). [`replay`]
/// compares one replay against the records the fixture's header recorded;
/// `verify` compares two fresh replays against each other, so it also catches
/// a divergence that appears mid-stream and later reconverges, and it does not
/// trust the header.
///
/// Host-originated events in an admin-authorized fixture are replayed in their
/// recorded order alongside participant inputs. Verification still proves
/// determinism of the fixture, not correctness of any host policy.
///
/// # Errors
///
/// The same refusals as [`replay`], from either run.
#[cfg(feature = "sandbox")]
pub fn verify(
    runner: &Sandbox,
    seed: u64,
    fixture: &Fixture,
    allow_partial: bool,
) -> Result<Verification> {
    let (first, first_steps) = replay_collecting(runner, seed, fixture, allow_partial)?;
    let (second, second_steps) = replay_collecting(runner, seed, fixture, allow_partial)?;
    let steps = first_steps.len().max(second_steps.len());
    let divergence = first_divergence(&first_steps, &second_steps);
    Ok(Verification {
        first,
        second,
        steps,
        divergence,
    })
}

/// Replay a fixture exactly as [`replay`] does, also collecting the records
/// after every event the program sees (and at the head) as snapshots.
/// `records` is documented as a pure function of state and the sandbox resets
/// fuel per guest call, so reading it per step cannot disturb the replay.
#[cfg(feature = "sandbox")]
fn replay_collecting(
    runner: &Sandbox,
    seed: u64,
    fixture: &Fixture,
    allow_partial: bool,
) -> Result<(Replayed, Vec<RecordSnapshot>)> {
    let header = &fixture.header;
    ensure!(
        header.complete || allow_partial,
        "the fixture is partial: its history does not start at sequence 1, so a fresh replay cannot reproduce it; pass --allow-partial to replay the retained window, whose records are then not compared"
    );
    ensure!(
        header.module_hash.is_some(),
        "the fixture records no active module, so there is nothing to replay"
    );
    let start = replay_start(&fixture.events)?;
    let mut world = runner.instantiate(seed)?;
    let mut snapshots = Vec::new();
    for event in &fixture.events[start..] {
        if let Some(world_event) = replay_event(event) {
            WorldProgram::update(&mut world, &world_event)
                .with_context(|| format!("replay sequence {}", world_event.sequence))?;
            snapshots.push(RecordSnapshot {
                sequence: world_event.sequence,
                records: snapshot_records(&mut world)?,
            });
        }
    }
    let records = WorldProgram::records(&mut world).context("replay records")?;
    let view = String::from_utf8(world.view()?).context("the replayed view is not UTF-8")?;
    let verdict = verdict(header, records.as_deref())?;
    let final_records = match records.as_deref() {
        Some(text) => parse_records(text).context("the replayed records")?,
        None => Records::new(),
    };
    if snapshots
        .last()
        .is_none_or(|snapshot| snapshot.sequence != header.current_sequence)
    {
        snapshots.push(RecordSnapshot {
            sequence: header.current_sequence,
            records: final_records,
        });
    }
    Ok((
        Replayed {
            records,
            view: Some(view),
            verdict,
        },
        snapshots,
    ))
}

/// The records of a replay in progress; a program that keeps none reads as
/// an empty map, as in [`verdict`].
#[cfg(feature = "sandbox")]
fn snapshot_records(world: &mut impl WorldProgram) -> Result<Records> {
    match WorldProgram::records(world).context("replay records")? {
        Some(text) => parse_records(&text).context("the replayed records"),
        None => Ok(Records::new()),
    }
}

/// The header line, with its keys in a fixed order.
fn header_line(header: &FixtureHeader) -> String {
    let module_hash = header
        .module_hash
        .as_deref()
        .map_or_else(|| "null".to_owned(), json_text);
    let records = header.records.as_ref().map_or_else(
        || "null".to_owned(),
        |records| Value::Object(records.clone().into_iter().collect()).to_string(),
    );
    format!(
        "{{\"fixture\":{FIXTURE_VERSION},\"world\":{},\"module_hash\":{module_hash},\"current_sequence\":{},\"complete\":{},\"records\":{records}}}",
        json_text(&header.world_id),
        header.current_sequence,
        header.complete,
    )
}

/// One event line, with its keys in a fixed order.
fn event_line(event: &FixtureEvent) -> String {
    match event {
        FixtureEvent::Chat {
            sequence,
            participant_id,
            text,
        } => format!(
            "{{\"sequence\":{sequence},\"participant_id\":{participant_id},\"kind\":\"chat\",\"text\":{}}}",
            json_text(text)
        ),
        FixtureEvent::Input {
            sequence,
            participant_id,
            bytes,
        } => format!(
            "{{\"sequence\":{sequence},\"participant_id\":{participant_id},\"kind\":\"input\",\"hex\":\"{}\"}}",
            encode_hex(bytes)
        ),
        FixtureEvent::System { event } => format!(
            "{{\"sequence\":0,\"participant_id\":0,\"kind\":\"system\",\"event\":{}}}",
            json_text(event)
        ),
        FixtureEvent::Other { sequence, name } => format!(
            "{{\"sequence\":{sequence},\"kind\":\"other\",\"name\":{}}}",
            json_text(name)
        ),
    }
}

/// A JSON string literal for `text`. Serializing a `Value` cannot fail.
fn json_text(text: &str) -> String {
    Value::String(text.to_owned()).to_string()
}

fn decode_header(line: &str) -> Result<FixtureHeader> {
    let value: Value = serde_json::from_str(line).context("the fixture header is not JSON")?;
    let object = value
        .as_object()
        .context("the fixture header is not an object")?;
    let version = object
        .get("fixture")
        .and_then(Value::as_u64)
        .context("the fixture header has no version")?;
    ensure!(
        version == FIXTURE_VERSION,
        "fixture version {version} is not supported; this build reads version {FIXTURE_VERSION}"
    );
    check_keys(
        object,
        &[
            "fixture",
            "world",
            "module_hash",
            "current_sequence",
            "complete",
            "records",
        ],
    )?;
    let module_hash = match object.get("module_hash") {
        Some(Value::Null) => None,
        Some(Value::String(hash)) => Some(hash.clone()),
        _ => bail!("the header's module_hash must be a string or null"),
    };
    let records = match object.get("records") {
        Some(Value::Null) => None,
        Some(records @ Value::Object(_)) => {
            Some(parse_records(&records.to_string()).context("the header's records")?)
        }
        _ => bail!("the header's records must be an object or null"),
    };
    Ok(FixtureHeader {
        world_id: string_field(object, "world")?.to_owned(),
        module_hash,
        current_sequence: u64_field(object, "current_sequence")?,
        complete: bool_field(object, "complete")?,
        records,
    })
}

fn decode_event(line: &str) -> Result<FixtureEvent> {
    ensure!(!line.is_empty(), "the line is blank");
    let value: Value = serde_json::from_str(line).context("the event is not JSON")?;
    let object = value.as_object().context("the event is not an object")?;
    let sequence = u64_field(object, "sequence")?;
    match string_field(object, "kind")? {
        "chat" => {
            check_keys(object, &["sequence", "participant_id", "kind", "text"])?;
            Ok(FixtureEvent::Chat {
                sequence,
                participant_id: u64_field(object, "participant_id")?,
                text: string_field(object, "text")?.to_owned(),
            })
        }
        "input" => {
            check_keys(object, &["sequence", "participant_id", "kind", "hex"])?;
            let bytes = decode_hex(string_field(object, "hex")?).context("the input is not hex")?;
            Ok(FixtureEvent::Input {
                sequence,
                participant_id: u64_field(object, "participant_id")?,
                bytes,
            })
        }
        "system" => {
            check_keys(object, &["sequence", "participant_id", "kind", "event"])?;
            ensure!(
                sequence == 0 && u64_field(object, "participant_id")? == 0,
                "a host event is never sequenced or attributed to a participant"
            );
            Ok(FixtureEvent::System {
                event: string_field(object, "event")?.to_owned(),
            })
        }
        "other" => {
            check_keys(object, &["sequence", "kind", "name"])?;
            Ok(FixtureEvent::Other {
                sequence,
                name: string_field(object, "name")?.to_owned(),
            })
        }
        kind => bail!("unknown event kind `{kind}`"),
    }
}

/// Require exactly the expected fields: no unknown ones, none missing.
fn check_keys(object: &Map<String, Value>, expected: &[&str]) -> Result<()> {
    if let Some(unknown) = object.keys().find(|key| !expected.contains(&key.as_str())) {
        bail!("unexpected field `{unknown}`");
    }
    if let Some(missing) = expected.iter().find(|key| !object.contains_key(**key)) {
        bail!("missing field `{missing}`");
    }
    Ok(())
}

fn u64_field(object: &Map<String, Value>, key: &str) -> Result<u64> {
    object
        .get(key)
        .and_then(Value::as_u64)
        .with_context(|| format!("field `{key}` must be an unsigned integer"))
}

fn string_field<'a>(object: &'a Map<String, Value>, key: &str) -> Result<&'a str> {
    object
        .get(key)
        .and_then(Value::as_str)
        .with_context(|| format!("field `{key}` must be a string"))
}

fn bool_field(object: &Map<String, Value>, key: &str) -> Result<bool> {
    object
        .get(key)
        .and_then(Value::as_bool)
        .with_context(|| format!("field `{key}` must be a boolean"))
}

/// Check that the sequenced events are contiguous and stay at or before the
/// header's sequence. Complete history starts at one and reaches the head;
/// partial history may end earlier or contain only host events. Host events
/// carry no sequence and are skipped.
fn check_window(header: &FixtureHeader, events: &[FixtureEvent]) -> Result<()> {
    let mut previous: Option<u64> = None;
    for event in events {
        if matches!(event, FixtureEvent::System { .. }) {
            continue;
        }
        let sequence = event.sequence();
        ensure!(sequence >= 1, "sequence 0 is reserved for host events");
        ensure!(
            sequence <= header.current_sequence,
            "event sequence {sequence} is beyond the header sequence {}",
            header.current_sequence
        );
        match previous {
            None => ensure!(
                !header.complete || sequence == 1,
                "a complete fixture must start at sequence 1, not {sequence}"
            ),
            Some(last) => ensure!(
                last.checked_add(1) == Some(sequence),
                "sequence {sequence} follows {last}; the history has a gap"
            ),
        }
        previous = Some(sequence);
    }
    match previous {
        Some(last) if header.complete => ensure!(
            last == header.current_sequence,
            "complete history ends at sequence {last}, but the header is at sequence {}",
            header.current_sequence
        ),
        Some(_) => {}
        None => ensure!(
            !header.complete || header.current_sequence == 0,
            "complete history is at sequence {} but has no sequenced events",
            header.current_sequence
        ),
    }
    Ok(())
}

/// Whether `event` is the install boundary.
#[cfg(feature = "sandbox")]
fn is_install(event: &FixtureEvent) -> bool {
    matches!(event, FixtureEvent::Other { name, .. } if name == INSTALL_NAME)
}

/// Where a replay begins: the last install, which stands in for the fresh
/// instance that the seed creates. Chat and earlier installs may precede it,
/// because no program saw them. An input or a host event may not, because a
/// different module ran it.
#[cfg(feature = "sandbox")]
fn replay_start(events: &[FixtureEvent]) -> Result<usize> {
    for event in events {
        if let FixtureEvent::Other { sequence, name } = event {
            ensure!(
                name == INSTALL_NAME,
                "event kind `{name}` at sequence {sequence} cannot be replayed"
            );
        }
    }
    let Some(last) = events.iter().rposition(is_install) else {
        return Ok(0);
    };
    let installed = events[last].sequence();
    for event in &events[..last] {
        let ran_before = match event {
            FixtureEvent::Chat { .. } | FixtureEvent::Other { .. } => None,
            FixtureEvent::Input { sequence, .. } => {
                Some(format!("the input at sequence {sequence}"))
            }
            FixtureEvent::System { .. } => Some("a host event".to_owned()),
        };
        if let Some(what) = ran_before {
            bail!(
                "the recording spans a module install at sequence {installed}: {what} ran under an earlier module, and a fixture replays under one module; record after the last install"
            );
        }
    }
    Ok(last)
}

/// The event a replay feeds the program, or `None` for events it never sees.
/// Input and host events take the same path as the live actor's `update`.
#[cfg(feature = "sandbox")]
fn replay_event(event: &FixtureEvent) -> Option<WorldEvent> {
    match event {
        FixtureEvent::Input {
            sequence,
            participant_id,
            bytes,
        } => Some(WorldEvent {
            sequence: *sequence,
            participant_id: *participant_id,
            kind: WorldEventKind::Input(bytes.clone()),
        }),
        FixtureEvent::System { event } => Some(WorldEvent {
            sequence: 0,
            participant_id: 0,
            kind: WorldEventKind::System {
                event: event.clone(),
            },
        }),
        FixtureEvent::Chat { .. } | FixtureEvent::Other { .. } => None,
    }
}

/// Compare the replayed records with the header's. A partial window is never
/// compared; a program that keeps no records replays to an empty map.
#[cfg(feature = "sandbox")]
fn verdict(header: &FixtureHeader, records: Option<&str>) -> Result<Verdict> {
    if !header.complete {
        return Ok(Verdict::NotCompared("partial history"));
    }
    let Some(expected) = &header.records else {
        return Ok(Verdict::NotCompared("the fixture has no records"));
    };
    let actual = match records {
        Some(text) => parse_records(text).context("the replayed records")?,
        None => Records::new(),
    };
    Ok(if *expected == actual {
        Verdict::Match
    } else {
        Verdict::Mismatch
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn install(sequence: u64) -> FixtureEvent {
        FixtureEvent::Other {
            sequence,
            name: INSTALL_NAME.to_owned(),
        }
    }

    fn input(sequence: u64, participant_id: u64, text: &str) -> FixtureEvent {
        FixtureEvent::Input {
            sequence,
            participant_id,
            bytes: text.as_bytes().to_vec(),
        }
    }

    fn header(current_sequence: u64, complete: bool) -> FixtureHeader {
        FixtureHeader {
            world_id: "lobby".to_owned(),
            module_hash: None,
            current_sequence,
            complete,
            records: Some(Records::new()),
        }
    }

    #[test]
    fn encode_decode_round_trips_every_kind() {
        let header = FixtureHeader {
            world_id: "lobby \"north\"".to_owned(),
            module_hash: Some("ab".repeat(32)),
            current_sequence: 6,
            complete: true,
            records: Some(
                parse_records(r#"{"board":"X--------","n":{"a":[1,2.5,null]}}"#).unwrap(),
            ),
        };
        let events = vec![
            install(1),
            FixtureEvent::Chat {
                sequence: 2,
                participant_id: 3,
                text: "say \"hi\"\nünï".to_owned(),
            },
            FixtureEvent::Input {
                sequence: 3,
                participant_id: 1,
                bytes: vec![0x00, 0xff, b'a'],
            },
            FixtureEvent::System {
                event: r#"{"cap":"players","event":"joined"}"#.to_owned(),
            },
            FixtureEvent::Other {
                sequence: 4,
                name: "future_kind".to_owned(),
            },
            FixtureEvent::Input {
                sequence: 5,
                participant_id: 2,
                bytes: Vec::new(),
            },
            FixtureEvent::Chat {
                sequence: 6,
                participant_id: 2,
                text: String::new(),
            },
        ];
        let text = encode(&header, &events);
        assert_eq!(text.lines().count(), 1 + events.len());
        let fixture = decode(&text).unwrap();
        assert_eq!(fixture.header, header);
        assert_eq!(fixture.events, events);
    }

    #[test]
    fn header_without_module_or_records_round_trips() {
        let header = FixtureHeader {
            world_id: "lobby".to_owned(),
            module_hash: None,
            current_sequence: 0,
            complete: true,
            records: None,
        };
        let text = encode(&header, &[]);
        assert_eq!(
            text,
            "{\"fixture\":1,\"world\":\"lobby\",\"module_hash\":null,\"current_sequence\":0,\"complete\":true,\"records\":null}\n"
        );
        assert_eq!(
            decode(&text).unwrap(),
            Fixture {
                header,
                events: Vec::new()
            }
        );
    }

    #[test]
    fn decode_refuses_an_unknown_version() {
        let text = encode(&header(0, true), &[]).replace("\"fixture\":1", "\"fixture\":2");
        let error = decode(&text).unwrap_err().to_string();
        assert!(error.contains("version 2"), "{error}");
    }

    #[test]
    fn decode_refuses_malformed_lines() {
        let head = encode(&header(1, true), &[]);
        for line in [
            r#"{"sequence":1,"participant_id":1,"kind":"input"}"#,
            r#"{"sequence":1,"participant_id":1,"kind":"input","hex":"zz"}"#,
            r#"{"sequence":1,"participant_id":1,"kind":"chat","text":"hi","extra":true}"#,
            r#"{"sequence":1,"participant_id":1,"kind":"chat"}"#,
            r#"{"sequence":1,"participant_id":"1","kind":"chat","text":"hi"}"#,
            r#"{"sequence":1,"participant_id":1,"kind":"telepathy"}"#,
            r#"{"sequence":1,"participant_id":0,"kind":"system","event":"{}"}"#,
            r#"{"sequence":1,"kind":"other"}"#,
            "not json",
            "[1,2]",
            "",
        ] {
            let text = format!("{head}{line}\n");
            assert!(decode(&text).is_err(), "accepted {line:?}");
        }
    }

    #[test]
    fn decode_refuses_a_window_that_does_not_fit_its_header() {
        let cases = [
            (
                header(3, true),
                vec![input(1, 1, "a"), input(3, 1, "b"), input(4, 1, "c")],
            ),
            (header(3, true), vec![input(1, 1, "a")]),
            (header(0, true), vec![input(1, 1, "a")]),
            (header(2, false), vec![input(0, 1, "a"), input(1, 1, "b")]),
            (header(2, true), vec![input(2, 1, "a")]),
            (header(3, false), vec![input(4, 1, "a")]),
        ];
        for (bad_header, events) in cases {
            let text = encode(&bad_header, &events);
            assert!(decode(&text).is_err(), "accepted {bad_header:?}");
        }
        let partial = encode(&header(6, false), &[input(5, 1, "a"), input(6, 1, "b")]);
        assert!(decode(&partial).is_ok(), "a partial window must decode");
        let system = FixtureEvent::System {
            event: r#"{"cap":"time.tick"}"#.to_owned(),
        };
        let partial_host_tail = encode(&header(6, false), &[input(4, 1, "a"), system.clone()]);
        assert!(
            decode(&partial_host_tail).is_ok(),
            "a partial window may end before its current sequence"
        );
        let partial_host_only = encode(&header(6, false), &[system]);
        assert!(
            decode(&partial_host_only).is_ok(),
            "a partial window may retain only host events"
        );
    }

    fn snapshot(sequence: u64, records: &str) -> RecordSnapshot {
        RecordSnapshot {
            sequence,
            records: parse_records(records).unwrap(),
        }
    }

    #[test]
    fn identical_record_sequences_have_no_divergence() {
        let steps = vec![
            snapshot(1, r#"{"board":"---------","plays":0}"#),
            snapshot(3, r#"{"board":"X--------","plays":1}"#),
        ];
        assert_eq!(first_divergence(&steps, &steps.clone()), None);
        assert_eq!(first_divergence(&[], &[]), None);
    }

    #[test]
    fn first_divergence_reports_the_earliest_step_and_first_key() {
        let first = vec![
            snapshot(1, r#"{"board":"---------","plays":0}"#),
            snapshot(3, r#"{"board":"X--------","plays":1}"#),
            snapshot(5, r#"{"board":"XO-------","plays":2}"#),
        ];
        let second = vec![
            snapshot(1, r#"{"board":"---------","plays":0}"#),
            snapshot(3, r#"{"board":"X--------","plays":9}"#),
            snapshot(5, r#"{"board":"XX-------","plays":2}"#),
        ];
        let divergence = first_divergence(&first, &second).unwrap();
        assert_eq!(divergence.sequence, 3);
        assert_eq!(divergence.key, "plays");
        assert_eq!(divergence.first, Some(serde_json::json!(1)));
        assert_eq!(divergence.second, Some(serde_json::json!(9)));
        assert_eq!(
            divergence.to_string(),
            "sequence 3: record `plays` differs (first replay: 1, second replay: 9)"
        );
    }

    #[test]
    fn first_divergence_reports_a_key_absent_from_one_replay() {
        let first = vec![snapshot(2, r#"{"winner":"X"}"#)];
        let second = vec![snapshot(2, r#"{}"#)];
        let divergence = first_divergence(&first, &second).unwrap();
        assert_eq!(divergence.sequence, 2);
        assert_eq!(divergence.key, "winner");
        assert_eq!(divergence.first, Some(serde_json::json!("X")));
        assert_eq!(divergence.second, None);
        assert!(divergence.to_string().contains("<absent>"));
    }

    #[test]
    fn first_divergence_reports_an_extra_step() {
        let first = vec![snapshot(1, r#"{"plays":0}"#), snapshot(2, r#"{"plays":1}"#)];
        let second = vec![snapshot(1, r#"{"plays":0}"#)];
        let divergence = first_divergence(&first, &second).unwrap();
        assert_eq!(divergence.sequence, 2);
        assert_eq!(divergence.key, "plays");
        assert_eq!(divergence.second, None);
    }

    #[test]
    fn check_module_refuses_a_different_module() {
        let wasm: &[u8] = include_bytes!("../examples/tic-tac-toe/tic-tac-toe.wasm");
        let other: &[u8] = include_bytes!("../examples/roc-counter/counter.wasm");
        let mut recorded = header(0, true);
        assert!(check_module(&recorded, wasm).is_err(), "no module recorded");
        recorded.module_hash = Some(module_hash(wasm));
        check_module(&recorded, wasm).unwrap();
        assert!(check_module(&recorded, other).is_err());
    }

    #[cfg(feature = "sandbox")]
    mod replay_tests {
        use iroh::{
            Endpoint, EndpointAddr, TransportAddr,
            endpoint::{RelayMode, presets},
            protocol::Router,
        };
        use iroh_blobs::store::mem::MemStore;

        use super::*;
        use crate::sandbox::SandboxLimits;
        use crate::world::{CapabilityBounds, WorldCore, WorldHandle, WorldLimits, WorldScopes};
        use crate::world_net::{WORLD_ALPN, WorldProtocol};
        use crate::world_session::WorldService;

        const TIC_TAC_TOE: &[u8] = include_bytes!("../examples/tic-tac-toe/tic-tac-toe.wasm");

        /// A tic-tac-toe game as a live instance plays it: a leading install,
        /// one chat line, and five moves. Returns the runner and the fixture a
        /// recorder would write for those events and the records they produce.
        fn checked_in_game() -> (Sandbox, Fixture) {
            let runner = Sandbox::compile(TIC_TAC_TOE, SandboxLimits::default()).unwrap();
            let mut live = runner.instantiate(7).unwrap();
            let mut events = vec![
                install(1),
                FixtureEvent::Chat {
                    sequence: 2,
                    participant_id: 1,
                    text: "gl hf".to_owned(),
                },
            ];
            let mut sequence = 2;
            for (participant, cell) in [(1, "0"), (2, "3"), (1, "1"), (2, "4"), (1, "2")] {
                live.update(participant, cell.as_bytes()).unwrap();
                sequence += 1;
                events.push(input(sequence, participant, cell));
            }
            let records = String::from_utf8(live.records().unwrap().unwrap()).unwrap();
            let header = FixtureHeader {
                world_id: "lobby".to_owned(),
                module_hash: Some(module_hash(TIC_TAC_TOE)),
                current_sequence: sequence,
                complete: true,
                records: Some(parse_records(&records).unwrap()),
            };
            (runner, Fixture { header, events })
        }

        #[test]
        fn replay_reproduces_the_live_records_of_a_checked_in_game() {
            let (runner, fixture) = checked_in_game();
            let text = encode(&fixture.header, &fixture.events);
            assert!(
                !text.contains("seed"),
                "the fixture must not carry the seed"
            );
            assert_eq!(decode(&text).unwrap(), fixture);
            check_module(&fixture.header, TIC_TAC_TOE).unwrap();
            let replayed = replay(&runner, 7, &fixture, false).unwrap();
            assert_eq!(replayed.verdict, Verdict::Match);
            let live = fixture.header.records.as_ref().unwrap();
            assert_eq!(
                parse_records(replayed.records.as_deref().unwrap()).unwrap(),
                *live
            );
            assert!(replayed.view.unwrap().contains("winner=X"));
        }

        #[test]
        fn verify_agrees_with_itself_on_a_checked_in_game() {
            let (runner, fixture) = checked_in_game();
            let verification = verify(&runner, 7, &fixture, false).unwrap();
            assert!(verification.is_deterministic());
            assert_eq!(verification.divergence, None);
            assert_eq!(verification.first.verdict, Verdict::Match);
            assert_eq!(verification.second.verdict, Verdict::Match);
            assert_eq!(verification.first.records, verification.second.records);
            // One snapshot per move the program saw, at the head.
            assert_eq!(verification.steps, 5);
        }

        #[test]
        fn a_nondeterministic_guest_fails_verify() {
            // The sandbox gives a guest no nondeterminism channel: it receives
            // no imports (no clock, no random source; see sandbox.rs), so no
            // checked-in guest can actually produce different records on a
            // second instantiation. The failure is therefore exercised at the
            // comparison layer verify() is built on, with a test double for
            // the one channel such a guest would use: records that depend on
            // a value which changes between instantiations (here a counter
            // standing in for e.g. a wall clock or an entropy read). The
            // double's second "replay" must be caught, at the first step
            // where its records move.
            use std::sync::atomic::{AtomicU64, Ordering};
            static INSTANTIATION: AtomicU64 = AtomicU64::new(0);
            let run = || {
                let n = INSTANTIATION.fetch_add(1, Ordering::SeqCst);
                vec![
                    snapshot(1, r#"{"plays":0}"#),
                    snapshot(2, &format!(r#"{{"plays":1,"roll":{n}}}"#)),
                ]
            };
            let divergence =
                first_divergence(&run(), &run()).expect("a nondeterministic guest must diverge");
            assert_eq!(divergence.sequence, 2);
            assert_eq!(divergence.key, "roll");
            assert_ne!(divergence.first, divergence.second);
        }

        #[test]
        fn partial_fixture_is_refused_unless_partial_replay_is_allowed() {
            let (runner, mut fixture) = checked_in_game();
            fixture.header.complete = false;
            fixture.events.drain(..2);
            let refused = replay(&runner, 7, &fixture, false).unwrap_err().to_string();
            assert!(refused.contains("--allow-partial"), "{refused}");
            let replayed = replay(&runner, 7, &fixture, true).unwrap();
            assert_eq!(replayed.verdict, Verdict::NotCompared("partial history"));
        }

        #[test]
        fn fixture_spanning_a_reinstall_is_refused() {
            let (runner, base) = checked_in_game();
            let reinstalled = Fixture {
                header: base.header.clone(),
                events: vec![install(1), input(2, 1, "0"), install(3), input(4, 2, "3")],
            };
            let error = replay(&runner, 7, &reinstalled, false)
                .unwrap_err()
                .to_string();
            assert!(error.contains("record after the last install"), "{error}");

            let input_first = Fixture {
                header: base.header,
                events: vec![input(1, 1, "0"), install(2), input(3, 2, "3")],
            };
            assert!(replay(&runner, 7, &input_first, false).is_err());
        }

        #[test]
        fn chat_before_the_install_does_not_block_replay() {
            let (runner, base) = checked_in_game();
            let events = vec![
                FixtureEvent::Chat {
                    sequence: 1,
                    participant_id: 1,
                    text: "hi".to_owned(),
                },
                install(2),
                input(3, 1, "0"),
            ];
            let mut live = runner.instantiate(7).unwrap();
            live.update(1, b"0").unwrap();
            let records =
                parse_records(&String::from_utf8(live.records().unwrap().unwrap()).unwrap())
                    .unwrap();
            let fixture = Fixture {
                header: FixtureHeader {
                    current_sequence: 3,
                    records: Some(records),
                    ..base.header
                },
                events,
            };
            let replayed = replay(&runner, 7, &fixture, false).unwrap();
            assert_eq!(replayed.verdict, Verdict::Match);
        }

        #[test]
        fn unknown_event_kinds_are_refused_by_replay() {
            let (runner, base) = checked_in_game();
            let fixture = Fixture {
                header: base.header,
                events: vec![
                    install(1),
                    FixtureEvent::Other {
                        sequence: 2,
                        name: "future_kind".to_owned(),
                    },
                ],
            };
            let error = replay(&runner, 7, &fixture, false).unwrap_err().to_string();
            assert!(error.contains("future_kind"), "{error}");
        }

        #[test]
        fn tampered_header_records_are_a_mismatch() {
            let (runner, mut fixture) = checked_in_game();
            fixture.header.records =
                Some(parse_records(r#"{"board":"XXXXXXXXX","plays":9,"winner":"X"}"#).unwrap());
            let replayed = replay(&runner, 7, &fixture, false).unwrap();
            assert_eq!(replayed.verdict, Verdict::Mismatch);
        }

        #[test]
        fn territory_fixture_replays_host_events_interleaved_with_inputs() {
            let wasm: &[u8] = include_bytes!("../examples/territory/territory.wasm");
            let runner = Sandbox::compile(wasm, SandboxLimits::default()).unwrap();
            let joined = |id: u64, name: &str| {
                format!(
                    r#"{{"cap":"players","event":"joined","participant":{{"id":{id},"name":"{name}"}}}}"#
                )
            };
            let mut live = runner.instantiate(7).unwrap();
            live.update(0, joined(1, "ann").as_bytes()).unwrap();
            live.update(1, b"l ").unwrap();
            live.update(0, joined(2, "bob").as_bytes()).unwrap();
            live.update(2, b"l ").unwrap();
            let records = String::from_utf8(live.records().unwrap().unwrap()).unwrap();
            let fixture = Fixture {
                header: FixtureHeader {
                    world_id: "lobby".to_owned(),
                    module_hash: Some(module_hash(wasm)),
                    current_sequence: 3,
                    complete: true,
                    records: Some(parse_records(&records).unwrap()),
                },
                events: vec![
                    install(1),
                    FixtureEvent::System {
                        event: joined(1, "ann"),
                    },
                    input(2, 1, "l "),
                    FixtureEvent::System {
                        event: joined(2, "bob"),
                    },
                    input(3, 2, "l "),
                ],
            };
            let replayed = replay(&runner, 7, &fixture, false).unwrap();
            assert_eq!(replayed.verdict, Verdict::Match);
        }

        /// A loopback endpoint that uses no relay or discovery.
        async fn endpoint() -> Endpoint {
            Endpoint::builder(presets::Minimal)
                .relay_mode(RelayMode::Disabled)
                .bind()
                .await
                .unwrap()
        }

        /// Serve `service` over iroh on loopback.
        async fn serve(service: WorldService) -> (Router, EndpointAddr) {
            let ep = endpoint().await;
            let router = Router::builder(ep.clone())
                .accept(WORLD_ALPN, WorldProtocol::new(service))
                .spawn();
            let addr = EndpointAddr::from_parts(
                ep.id(),
                ep.bound_sockets().iter().map(|socket| {
                    let ip = if socket.ip().is_unspecified() {
                        std::net::Ipv4Addr::LOCALHOST.into()
                    } else {
                        socket.ip()
                    };
                    TransportAddr::Ip(std::net::SocketAddr::new(ip, socket.port()))
                }),
            );
            (router, addr)
        }

        #[tokio::test]
        async fn recording_over_iroh_replays_to_the_live_records() {
            let wasm = TIC_TAC_TOE.to_vec();
            let world =
                WorldHandle::spawn(WorldCore::new("lobby", WorldLimits::default()).unwrap());
            let service = WorldService::new(world.clone(), Some("admin".to_owned()))
                .with_blob_store(MemStore::new().into());
            service
                .install_wasm(Some("admin"), wasm.clone(), 7, &module_hash(&wasm))
                .await
                .unwrap();
            let (_, ann) = world.issue("ann", WorldScopes::GUEST).await.unwrap();
            let (_, bo) = world.issue("bo", WorldScopes::GUEST).await.unwrap();
            world.chat(ann.clone(), "gl hf").await.unwrap();
            for (token, cell) in [
                (&ann, "0"),
                (&bo, "3"),
                (&ann, "1"),
                (&bo, "4"),
                (&ann, "2"),
            ] {
                world
                    .input(token.clone(), cell.as_bytes().to_vec())
                    .await
                    .unwrap();
            }

            let (router, addr) = serve(service).await;
            let client_ep = endpoint().await;
            let mut client = WorldClient::connect(&client_ep, addr).await.unwrap();
            let fixture = record(&mut client, "admin").await.unwrap();
            client.close();

            assert_eq!(fixture.header.world_id, "lobby");
            assert_eq!(fixture.header.module_hash, Some(module_hash(&wasm)));
            assert!(fixture.header.complete);
            assert_eq!(fixture.header.current_sequence, 7);
            assert_eq!(fixture.events.len(), 7);
            assert_eq!(fixture.events[0], install(1));
            assert!(matches!(
                fixture.events[1],
                FixtureEvent::Chat { sequence: 2, .. }
            ));
            let (_, live) = world.records();
            assert_eq!(fixture.header.records.as_ref(), Some(&*live));

            let text = encode(&fixture.header, &fixture.events);
            assert!(
                !text.contains("seed"),
                "the fixture must not carry the seed"
            );
            assert_eq!(decode(&text).unwrap(), fixture);
            let runner = Sandbox::compile(&wasm, SandboxLimits::default()).unwrap();
            check_module(&fixture.header, &wasm).unwrap();
            assert_eq!(
                replay(&runner, 7, &fixture, false).unwrap().verdict,
                Verdict::Match
            );

            client_ep.close().await;
            router.shutdown().await.unwrap();
        }

        #[tokio::test]
        async fn recording_replays_host_presence_from_admin_history() {
            let wasm: &[u8] = include_bytes!("../examples/territory/territory.wasm");
            let mut core = WorldCore::new("lobby", WorldLimits::default()).unwrap();
            core.ledger
                .set_granted(std::collections::BTreeSet::from(["players".to_owned()]));
            let world = WorldHandle::spawn(core);
            let service = WorldService::new(world.clone(), Some("admin".to_owned()))
                .with_blob_store(MemStore::new().into());
            service
                .install_wasm(Some("admin"), wasm.to_vec(), 7, &module_hash(wasm))
                .await
                .unwrap();
            let (_, capability) = world.issue("ann", WorldScopes::GUEST).await.unwrap();

            let (router, addr) = serve(service).await;
            let client_ep = endpoint().await;
            let mut player = WorldClient::connect(&client_ep, addr.clone())
                .await
                .unwrap();
            player
                .send_json(&serde_json::json!({
                    "type": "join",
                    "capability": capability.expose(),
                }))
                .await
                .unwrap();
            let snapshot = player.recv_json().await.unwrap().unwrap();
            assert_eq!(snapshot["type"], "snapshot");
            player
                .send_json(&serde_json::json!({
                    "type": "input",
                    "data_hex": "6c20",
                }))
                .await
                .unwrap();
            let mut saw_input = false;
            for _ in 0..8 {
                let received =
                    tokio::time::timeout(std::time::Duration::from_secs(10), player.recv_json())
                        .await
                        .expect("timed out waiting for the participant input event")
                        .unwrap();
                let Some(frame) = received else {
                    break;
                };
                if frame["type"] == "event" {
                    assert_eq!(frame["event"]["kind"]["kind"], "input");
                    saw_input = true;
                    break;
                }
            }
            assert!(saw_input, "participant did not receive its input event");

            let mut admin = WorldClient::connect(&client_ep, addr).await.unwrap();
            let fixture = record(&mut admin, "admin").await.unwrap();
            let joined = fixture.events.iter().position(|event| {
                matches!(
                    event,
                    FixtureEvent::System { event }
                        if event.contains("\"cap\":\"players\"")
                            && event.contains("\"event\":\"joined\"")
                )
            });
            let input = fixture
                .events
                .iter()
                .position(|event| matches!(event, FixtureEvent::Input { sequence: 2, .. }));
            assert!(joined.is_some_and(|joined| input.is_some_and(|input| joined < input)));
            let runner = Sandbox::compile(wasm, SandboxLimits::default()).unwrap();
            assert_eq!(
                replay(&runner, 7, &fixture, false).unwrap().verdict,
                Verdict::Match
            );

            player.close();
            admin.close();
            client_ep.close().await;
            router.shutdown().await.unwrap();
        }

        #[tokio::test]
        async fn a_trimmed_history_records_as_incomplete_and_replays_only_partially() {
            let wasm = TIC_TAC_TOE.to_vec();
            let limits = WorldLimits {
                retained_events: 2,
                ..WorldLimits::default()
            };
            let world = WorldHandle::spawn(WorldCore::new("lobby", limits).unwrap());
            let service = WorldService::new(world.clone(), Some("admin".to_owned()))
                .with_blob_store(MemStore::new().into());
            service
                .install_wasm(Some("admin"), wasm.clone(), 7, &module_hash(&wasm))
                .await
                .unwrap();
            let (_, ann) = world.issue("ann", WorldScopes::GUEST).await.unwrap();
            for cell in ["0", "3", "1", "4"] {
                world
                    .input(ann.clone(), cell.as_bytes().to_vec())
                    .await
                    .unwrap();
            }

            let (router, addr) = serve(service).await;
            let client_ep = endpoint().await;
            let mut client = WorldClient::connect(&client_ep, addr).await.unwrap();
            let fixture = record(&mut client, "admin").await.unwrap();
            client.close();

            assert!(!fixture.header.complete);
            assert_eq!(fixture.header.current_sequence, 5);
            let sequences: Vec<u64> = fixture.events.iter().map(FixtureEvent::sequence).collect();
            assert_eq!(sequences, vec![4, 5]);
            let runner = Sandbox::compile(&wasm, SandboxLimits::default()).unwrap();
            assert!(replay(&runner, 7, &fixture, false).is_err());
            assert_eq!(
                replay(&runner, 7, &fixture, true).unwrap().verdict,
                Verdict::NotCompared("partial history")
            );

            client_ep.close().await;
            router.shutdown().await.unwrap();
        }

        #[tokio::test]
        async fn a_reopened_partial_checkpoint_records_and_decodes_its_admin_fixture() {
            let wasm = TIC_TAC_TOE.to_vec();
            let limits = WorldLimits {
                retained_events: 2,
                ..WorldLimits::default()
            };
            let mut core = WorldCore::new("lobby", limits.clone()).unwrap();
            core.active_module_hash = Some(module_hash(&wasm));
            core.active_seed = 7;
            let (_, capability) = core
                .issue_capability("ann", WorldScopes::GUEST, CapabilityBounds::default())
                .unwrap();
            for cell in ["0", "3", "1"] {
                core.input(&capability, cell.as_bytes()).unwrap();
            }
            assert!(!core.admin_history_complete);

            let checkpoint = core.checkpoint_entries("snapshot".to_owned()).unwrap();
            let reopened = WorldCore::restore("lobby", limits, checkpoint).unwrap();
            assert!(!reopened.core.admin_history_complete);
            let world = WorldHandle::spawn(reopened.core);
            let service = WorldService::new(world, Some("admin".to_owned()))
                .with_blob_store(MemStore::new().into());
            let (router, addr) = serve(service).await;
            let client_ep = endpoint().await;
            let mut client = WorldClient::connect(&client_ep, addr).await.unwrap();

            let fixture = record(&mut client, "admin").await.unwrap();
            assert!(!fixture.header.complete);
            assert_eq!(fixture.header.current_sequence, 3);
            assert_eq!(
                fixture
                    .events
                    .iter()
                    .map(FixtureEvent::sequence)
                    .collect::<Vec<_>>(),
                vec![2, 3]
            );
            let text = encode(&fixture.header, &fixture.events);
            assert_eq!(decode(&text).unwrap(), fixture);

            client.close();
            client_ep.close().await;
            router.shutdown().await.unwrap();
        }
    }
}
