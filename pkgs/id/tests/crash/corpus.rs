//! The crash corpus: `corpus.tsv`, a checked-in list of crash points, each with the verdict its run
//! must keep.
//!
//! Two header lines pin the workload: the storage-call count, and an FNV-1a hash of the call trace
//! (kind, offset and length of every call). If the workload or redb's write pattern changes, both
//! change, and the replay fails before it compares any verdict.
//!
//! Every sweep failure is added by hand as a `fix-<commit>` row, with the verdict the fixed code
//! produces. `write_corpus` keeps those rows when it regenerates the file.

use crate::faultfs::{Kind, Plan, Survive, Tear, Trace};
use crate::harness::Verdict;
use crate::sweep::Job;
use std::fmt::Write as _;

const STORAGE_CALLS: &str = "# storage_calls: ";
const TRACE_HASH: &str = "# trace_fnv1a: ";
const COLUMNS: &str =
    "# name\tlying_sync\tevent\tkind\tsurvive\ttear\tsync_completed\tseed\texpect";

const PREAMBLE: &str = "\
# Crash corpus for the redb fault model (tests/crash). Replayed by corpus_replays_with_recorded_verdicts.
# Each line is one crash point: the plan that cuts power there, and the verdict its run must keep.
# expect `ok` accepts either valid prefix (ok_acked or ok_acked_plus_one). Any other value must match.
# trace_fnv1a is FNV-1a over (kind, offset, length) of every storage call of the clean run.
# Regenerate, from pkgs/id: cargo test --release --test crash write_corpus -- --ignored
# Each sweep failure is added by hand as a fix-<commit> row, with the verdict the fixed code gives.
";

/// What the replay pins about the workload.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    pub storage_calls: u64,
    pub trace_fnv1a: u64,
}

/// The verdict a row must keep.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Expect {
    /// `ok`: either valid prefix, `ok_acked` or `ok_acked_plus_one`.
    Valid,
    /// Exactly this verdict.
    Exact(Verdict),
}

impl Expect {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Valid => "ok",
            Self::Exact(verdict) => verdict.name(),
        }
    }

    pub fn accepts(self, verdict: Verdict) -> bool {
        match self {
            Self::Valid => matches!(verdict, Verdict::OkAcked | Verdict::OkAckedPlusOne),
            Self::Exact(expected) => verdict == expected,
        }
    }
}

/// One crash point and the verdict it must keep.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub name: String,
    pub job: Job,
    /// The kind of the storage call at `job.plan.event`. The replay checks it against the trace.
    pub kind: Kind,
    pub expect: Expect,
}

/// FNV-1a over the trace. It detects a changed workload; it is not a security hash.
pub fn trace_fnv1a(trace: &Trace) -> u64 {
    const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut h = OFFSET_BASIS;
    let mut eat = |bytes: &[u8]| {
        for &b in bytes {
            h ^= u64::from(b);
            h = h.wrapping_mul(PRIME);
        }
    };
    for &(kind, offset, len) in trace {
        eat(kind.name().as_bytes());
        eat(&[0]);
        eat(&offset.to_le_bytes());
        eat(&(len as u64).to_le_bytes());
    }
    h
}

/// Parses the corpus text. Blank lines and comments are skipped, and the two header keys are read.
pub fn parse(text: &str) -> Result<(Header, Vec<Row>), String> {
    let mut storage_calls = None;
    let mut trace_fnv1a = None;
    let mut rows = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let at = |e: String| format!("line {}: {e}", index + 1);
        if let Some(value) = line.strip_prefix(STORAGE_CALLS) {
            storage_calls = Some(value.trim().parse::<u64>().map_err(|e| at(e.to_string()))?);
        } else if let Some(value) = line.strip_prefix(TRACE_HASH) {
            trace_fnv1a =
                Some(u64::from_str_radix(value.trim(), 16).map_err(|e| at(e.to_string()))?);
        } else if !line.trim().is_empty() && !line.starts_with('#') {
            rows.push(parse_row(line).map_err(at)?);
        }
    }
    let header = Header {
        storage_calls: storage_calls.ok_or("no `# storage_calls:` line")?,
        trace_fnv1a: trace_fnv1a.ok_or("no `# trace_fnv1a:` line")?,
    };
    Ok((header, rows))
}

fn parse_row(line: &str) -> Result<Row, String> {
    let fields: Vec<&str> = line.split('\t').collect();
    let &[
        name,
        lying,
        event,
        kind,
        survive,
        tear,
        sync_completed,
        seed,
        expect,
    ] = fields.as_slice()
    else {
        return Err(format!(
            "expected 9 tab-separated fields, found {}",
            fields.len()
        ));
    };
    Ok(Row {
        name: name.to_owned(),
        job: Job {
            plan: Plan {
                event: event.parse().map_err(|e| format!("event {event:?}: {e}"))?,
                survive: parse_survive(survive)?,
                tear: parse_tear(tear)?,
                sync_completed: parse_flag(sync_completed)?,
                seed: seed.parse().map_err(|e| format!("seed {seed:?}: {e}"))?,
            },
            lying: parse_flag(lying)?,
        },
        kind: parse_kind(kind)?,
        expect: parse_expect(expect)?,
    })
}

fn parse_flag(s: &str) -> Result<bool, String> {
    match s {
        "0" => Ok(false),
        "1" => Ok(true),
        other => Err(format!("expected 0 or 1, found {other:?}")),
    }
}

fn parse_kind(s: &str) -> Result<Kind, String> {
    [Kind::Write, Kind::SetLen, Kind::Sync]
        .into_iter()
        .find(|k| k.name() == s)
        .ok_or_else(|| format!("unknown storage call kind {s:?}"))
}

fn parse_survive(s: &str) -> Result<Survive, String> {
    [
        Survive::Drop,
        Survive::Keep,
        Survive::Random,
        Survive::Reorder,
    ]
    .into_iter()
    .find(|v| v.name() == s)
    .ok_or_else(|| format!("unknown survive policy {s:?}"))
}

fn parse_tear(s: &str) -> Result<Tear, String> {
    [Tear::Off, Tear::Byte, Tear::Sector]
        .into_iter()
        .find(|t| t.name() == s)
        .ok_or_else(|| format!("unknown tear mode {s:?}"))
}

fn parse_expect(s: &str) -> Result<Expect, String> {
    if s == "ok" {
        return Ok(Expect::Valid);
    }
    Verdict::from_name(s)
        .map(Expect::Exact)
        .ok_or_else(|| format!("unknown verdict {s:?}"))
}

/// The corpus text for a header and rows, in the order given.
pub fn format(header: &Header, rows: &[Row]) -> String {
    let mut out = String::from(PREAMBLE);
    writeln!(out, "{STORAGE_CALLS}{}", header.storage_calls).expect("write to a String");
    writeln!(out, "{TRACE_HASH}{:016x}", header.trace_fnv1a).expect("write to a String");
    writeln!(out, "{COLUMNS}").expect("write to a String");
    for row in rows {
        let p = row.job.plan;
        writeln!(
            out,
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            row.name,
            u8::from(row.job.lying),
            p.event,
            row.kind.name(),
            p.survive.name(),
            p.tear.name(),
            u8::from(p.sync_completed),
            p.seed,
            row.expect.name()
        )
        .expect("write to a String");
    }
    out
}
