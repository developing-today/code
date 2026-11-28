//! Candidate-engine encoding bake-off for graph triples (slice
//! A-GRAPH-9125-03, issue #10219; design part A §4, #9125, statements
//! S5.3–S5.7 and S5.11).
//!
//! One deterministic typed-byte corpus, built from the canonical terms of
//! slice A-GRAPH-9125-01, is loaded into four real candidate engines:
//!
//! - `chunked_binary`: sorted canonical triples in length-framed chunks of
//!   at most 4 KiB, with a chunk directory and no secondary index. The
//!   format is immutable, so a write rewrites the file.
//! - `redb_tuple`: the current redb tuple-key materialization: SPO and POS
//!   tables whose keys are `id_core::tuple` encodings of the canonical term
//!   bytes.
//! - `sqlite_jsonb`: one SQLite JSONB document per triple, with structured
//!   terms and expression indexes over hex term keys.
//! - `sqlite_blob_rows`: normalized SQLite rows: an interned `terms` table
//!   of canonical bytes and a `WITHOUT ROWID` SPO table with a POS index.
//!
//! Every engine runs the same workloads (load, storage, byte/type round
//! trip, subject query, predicate/object query, paged stream, single-row
//! writes and deletes). Each output is checked here against a reference
//! model of the corpus. With `BAKEOFF_RESULTS` set to a path, the harness
//! runs the whole matrix three times and writes raw results there; the
//! independent validator next to the report
//! (`validate_real_encoding_bakeoff.py --run-and-check`) runs this test that
//! way, regenerates the corpus itself and checks every cell. Nothing here
//! picks a winner.
//!
//! `graph/mod.rs` does not export the term modules yet (slice 9110-09 wires
//! them), so this test includes them, and the modules they build on, by
//! path.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::cast_possible_truncation,
    missing_docs
)]

#[path = "../../id-core/src/canonical_cbor.rs"]
#[allow(dead_code, unreachable_pub)]
mod canonical_cbor;

#[path = "../../id-core/src/id_shape.rs"]
#[allow(dead_code, unreachable_pub)]
mod id_shape;

#[path = "../../id-core/src/hash_profile.rs"]
#[allow(dead_code, unreachable_pub)]
mod hash_profile;

#[path = "../../id-core/src/signed_envelope.rs"]
#[allow(dead_code, unreachable_pub)]
mod signed_envelope;

#[path = "../../id-core/src/graph/term.rs"]
#[allow(dead_code, unreachable_pub)]
mod term;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::ops::Bound;
use std::path::{Path, PathBuf};
use std::time::Instant;

use id_core::tuple::{TupleEncoder, TupleValue, decode as decode_tuple};
use id_shape::{ContentId, HashProfile};
use redb::{Database, ReadableDatabase, TableDefinition};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value as Json, json};
use sha2::{Digest, Sha256};
use signed_envelope::EnvelopeId;
use term::{ObjectRef, Term, Triple};

// ---------------------------------------------------------------------------
// Fixture: the shared deterministic corpus. The validator regenerates it
// from this description, so any change here is a fixture change.
// ---------------------------------------------------------------------------

const SEED: u64 = 0x9125_0003;
const SUBJECTS: u64 = 1000;
const SUBJECT_QUERIES: usize = 32;
const PO_QUERIES: usize = 16;
const PAGE_SIZE: usize = 128;
const NEW_TRIPLES: usize = 200;
const DELETE_STRIDE: usize = 37;
const RUNS: usize = 3;
const CHUNK_BYTES: usize = 4096;

const TAG_IRI: u64 = 1;
const TAG_TEXT: u64 = 2;
const TAG_BYTES: u64 = 3;
const TAG_I64: u64 = 4;

const ENGINES: [&str; 4] = [
    "chunked_binary",
    "redb_tuple",
    "sqlite_jsonb",
    "sqlite_blob_rows",
];
const WORKLOADS: [&str; 7] = [
    "load",
    "storage",
    "round_trip",
    "query_subject",
    "query_predicate_object",
    "page_stream",
    "write",
];

/// splitmix64, as the validator implements it.
struct Rng(u64);

impl Rng {
    const fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn digest(&mut self) -> [u8; 32] {
        let mut out = [0; 32];
        for chunk in out.chunks_mut(8) {
            chunk.copy_from_slice(&self.next().to_be_bytes());
        }
        out
    }
}

fn iri(text: &str) -> Term {
    Term::typed(TAG_IRI, text.as_bytes().to_vec())
}

fn text(value: &str) -> Term {
    Term::typed(TAG_TEXT, value.as_bytes().to_vec())
}

fn content_ref(digest: [u8; 32]) -> Term {
    Term::Object(ObjectRef::Content(ContentId::new(
        HashProfile::RawBlake3_256,
        digest,
    )))
}

fn envelope_ref(digest: [u8; 32]) -> Term {
    Term::Object(ObjectRef::Envelope(EnvelopeId::from_content_id(
        ContentId::new(HashProfile::RawBlake3_256, digest),
    )))
}

fn user(index: u64) -> Term {
    iri(&format!("urn:user:{index}"))
}

/// The corpus, keyed and ordered by canonical triple bytes.
fn corpus() -> BTreeMap<Vec<u8>, Triple> {
    let mut rng = Rng(SEED);
    let mut out = BTreeMap::new();
    let mut add = |s: &Term, p: &str, o: Term| {
        let triple = Triple::new(s.clone(), iri(p), o);
        out.insert(triple.to_bytes().unwrap(), triple);
    };
    for i in 0..SUBJECTS {
        let s = user(i);
        add(&s, "name", text(&format!("User {i}")));
        let score = rng.next();
        add(
            &s,
            "score",
            Term::typed(TAG_I64, score.to_be_bytes().to_vec()),
        );
        let follows = rng.next() % 6;
        for _ in 0..follows {
            add(&s, "follows", user(rng.next() % SUBJECTS));
        }
        if rng.next().is_multiple_of(3) {
            add(&s, "avatar", content_ref(rng.digest()));
        }
        if rng.next().is_multiple_of(4) {
            add(&s, "cites", envelope_ref(rng.digest()));
        }
        // Every blob holds a NUL and a 0xFF byte, which the tuple codec
        // escapes and JSON must carry as hex.
        let len = rng.next() % 38;
        let mut blob = vec![0x00, 0xFF];
        for _ in 0..len {
            blob.push((rng.next() & 0xFF) as u8);
        }
        add(&s, "blob", Term::typed(TAG_BYTES, blob));
    }
    out
}

fn subject_queries() -> Vec<Term> {
    let mut rng = Rng(SEED ^ 0x5157);
    (0..SUBJECT_QUERIES)
        .map(|_| user(rng.next() % SUBJECTS))
        .collect()
}

fn po_queries() -> Vec<(Term, Term)> {
    let mut rng = Rng(SEED ^ 0x504F);
    (0..PO_QUERIES)
        .map(|_| (iri("follows"), user(rng.next() % SUBJECTS)))
        .collect()
}

fn new_triples() -> Vec<Triple> {
    (0..NEW_TRIPLES)
        .map(|j| {
            Triple::new(
                iri(&format!("urn:new:{j}")),
                iri("name"),
                text(&format!("New {j}")),
            )
        })
        .collect()
}

fn deleted(corpus: &BTreeMap<Vec<u8>, Triple>) -> Vec<Triple> {
    corpus.values().step_by(DELETE_STRIDE).cloned().collect()
}

fn bytes_of(term: &Term) -> Vec<u8> {
    term.to_bytes().unwrap()
}

fn hex(bytes: &[u8]) -> String {
    hex::encode(bytes)
}

/// SHA-256 over the sorted items, each framed by its u32 big-endian length.
fn set_digest(items: impl IntoIterator<Item = Vec<u8>>) -> String {
    let sorted: BTreeSet<Vec<u8>> = items.into_iter().collect();
    let mut hasher = Sha256::new();
    for item in &sorted {
        hasher.update((item.len() as u32).to_be_bytes());
        hasher.update(item);
    }
    hex::encode(hasher.finalize())
}

fn micros(start: Instant) -> u64 {
    u64::try_from(start.elapsed().as_micros()).unwrap().max(1)
}

fn file_len(path: &Path) -> u64 {
    fs::metadata(path).unwrap().len()
}

// ---------------------------------------------------------------------------
// The engine interface every candidate implements.
// ---------------------------------------------------------------------------

trait Engine {
    fn load(&mut self, triples: &[Triple]);
    fn subject(&self, subject: &Term) -> Vec<Triple>;
    fn predicate_object(&self, predicate: &Term, object: &Term) -> Vec<Term>;
    /// One page after the opaque cursor; the cursor for the next page, if
    /// the page was full.
    fn page(&self, after: Option<&[u8]>, limit: usize) -> (Vec<Triple>, Option<Vec<u8>>);
    fn insert(&mut self, triple: &Triple);
    fn delete(&mut self, triple: &Triple);
    fn file_bytes(&self) -> u64;
    fn compact(&mut self) -> u64;
    fn metadata(&self) -> Json;
}

// --- chunked_binary --------------------------------------------------------

const CHUNK_MAGIC: &[u8; 6] = b"IDCB\x00\x01";

struct Chunk {
    offset: u64,
    len: u32,
    first: Vec<u8>,
}

struct Chunked {
    path: PathBuf,
    directory: Vec<Chunk>,
}

impl Chunked {
    fn new(dir: &Path) -> Self {
        Self {
            path: dir.join("chunked.bin"),
            directory: Vec::new(),
        }
    }

    /// Writes the sorted triples as one immutable file and reloads its
    /// directory from disk.
    fn write_all(&mut self, sorted: &[Vec<u8>]) {
        let mut out = CHUNK_MAGIC.to_vec();
        let mut directory: Vec<(u64, u32, Vec<u8>)> = Vec::new();
        let mut chunk: Vec<u8> = Vec::new();
        let mut first: Option<&Vec<u8>> = None;
        let mut flush = |out: &mut Vec<u8>, chunk: &mut Vec<u8>, first: &mut Option<&Vec<u8>>| {
            if let Some(f) = first.take() {
                directory.push((out.len() as u64, chunk.len() as u32, f.clone()));
                out.append(chunk);
            }
        };
        for item in sorted {
            if !chunk.is_empty() && chunk.len() + 4 + item.len() > CHUNK_BYTES {
                flush(&mut out, &mut chunk, &mut first);
            }
            if first.is_none() {
                first = Some(item);
            }
            chunk.extend_from_slice(&(item.len() as u32).to_be_bytes());
            chunk.extend_from_slice(item);
        }
        flush(&mut out, &mut chunk, &mut first);
        let dir_offset = out.len() as u64;
        let count = directory.len() as u32;
        for (offset, len, first) in directory {
            out.extend_from_slice(&offset.to_be_bytes());
            out.extend_from_slice(&len.to_be_bytes());
            out.extend_from_slice(&(first.len() as u32).to_be_bytes());
            out.extend_from_slice(&first);
        }
        out.extend_from_slice(&dir_offset.to_be_bytes());
        out.extend_from_slice(&count.to_be_bytes());
        out.extend_from_slice(b"IDCB");
        fs::write(&self.path, out).unwrap();
        self.reload_directory();
    }

    fn reload_directory(&mut self) {
        let bytes = fs::read(&self.path).unwrap();
        assert_eq!(&bytes[..6], CHUNK_MAGIC);
        let tail = &bytes[bytes.len() - 16..];
        assert_eq!(&tail[12..], b"IDCB");
        let mut at = usize::try_from(u64::from_be_bytes(tail[..8].try_into().unwrap())).unwrap();
        let count = u32::from_be_bytes(tail[8..12].try_into().unwrap());
        let mut directory = Vec::new();
        for _ in 0..count {
            let offset = u64::from_be_bytes(bytes[at..at + 8].try_into().unwrap());
            let len = u32::from_be_bytes(bytes[at + 8..at + 12].try_into().unwrap());
            let flen = u32::from_be_bytes(bytes[at + 12..at + 16].try_into().unwrap()) as usize;
            let first = bytes[at + 16..at + 16 + flen].to_vec();
            at += 16 + flen;
            directory.push(Chunk { offset, len, first });
        }
        self.directory = directory;
    }

    fn read_chunk(file: &mut fs::File, chunk: &Chunk) -> Vec<Vec<u8>> {
        let mut body = vec![0; chunk.len as usize];
        file.seek(SeekFrom::Start(chunk.offset)).unwrap();
        file.read_exact(&mut body).unwrap();
        let mut items = Vec::new();
        let mut at = 0;
        while at < body.len() {
            let len = u32::from_be_bytes(body[at..at + 4].try_into().unwrap()) as usize;
            items.push(body[at + 4..at + 4 + len].to_vec());
            at += 4 + len;
        }
        items
    }

    /// Visits entries at or after `start` in order until `visit` says stop.
    fn scan_from(&self, start: &[u8], mut visit: impl FnMut(&[u8]) -> bool) {
        let first_after = self
            .directory
            .partition_point(|c| c.first.as_slice() <= start);
        let begin = first_after.saturating_sub(1);
        let mut file = fs::File::open(&self.path).unwrap();
        for chunk in &self.directory[begin..] {
            for item in Self::read_chunk(&mut file, chunk) {
                if item.as_slice() < start {
                    continue;
                }
                if !visit(&item) {
                    return;
                }
            }
        }
    }

    fn all(&self) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        self.scan_from(&[], |item| {
            out.push(item.to_vec());
            true
        });
        out
    }
}

impl Engine for Chunked {
    fn load(&mut self, triples: &[Triple]) {
        let mut sorted: Vec<Vec<u8>> = triples.iter().map(|t| t.to_bytes().unwrap()).collect();
        sorted.sort();
        sorted.dedup();
        self.write_all(&sorted);
    }

    fn subject(&self, subject: &Term) -> Vec<Triple> {
        // A canonical triple is the array head 0x83 then the subject's own
        // self-delimiting encoding, so one subject's triples are contiguous.
        let mut prefix = vec![0x83];
        prefix.extend(bytes_of(subject));
        let mut out = Vec::new();
        self.scan_from(&prefix, |item| {
            if !item.starts_with(&prefix) {
                return false;
            }
            out.push(Triple::from_bytes(item).unwrap());
            true
        });
        out
    }

    fn predicate_object(&self, predicate: &Term, object: &Term) -> Vec<Term> {
        // No secondary index: a full scan.
        let mut out = Vec::new();
        self.scan_from(&[], |item| {
            let triple = Triple::from_bytes(item).unwrap();
            if &triple.predicate == predicate && &triple.object == object {
                out.push(triple.subject);
            }
            true
        });
        out
    }

    fn page(&self, after: Option<&[u8]>, limit: usize) -> (Vec<Triple>, Option<Vec<u8>>) {
        let mut out = Vec::new();
        let mut last = None;
        self.scan_from(after.unwrap_or(&[]), |item| {
            if after == Some(item) {
                return true;
            }
            out.push(Triple::from_bytes(item).unwrap());
            last = Some(item.to_vec());
            out.len() < limit
        });
        let next = if out.len() == limit { last } else { None };
        (out, next)
    }

    fn insert(&mut self, triple: &Triple) {
        let mut all = self.all();
        let item = triple.to_bytes().unwrap();
        if let Err(at) = all.binary_search(&item) {
            all.insert(at, item);
        }
        self.write_all(&all);
    }

    fn delete(&mut self, triple: &Triple) {
        let mut all = self.all();
        if let Ok(at) = all.binary_search(&triple.to_bytes().unwrap()) {
            all.remove(at);
        }
        self.write_all(&all);
    }

    fn file_bytes(&self) -> u64 {
        file_len(&self.path)
    }

    fn compact(&mut self) -> u64 {
        self.file_bytes()
    }

    fn metadata(&self) -> Json {
        json!({
            "engine": "in-test chunked binary file",
            "engine_version": "idcb/1",
            "schema": "magic IDCB 00 01; chunks of u32-BE-length-framed canonical triples, at most 4096 bytes; directory of (u64 offset, u32 length, first triple); trailer (u64 directory offset, u32 chunk count, IDCB)",
            "indexes": ["chunk directory over sorted canonical triple bytes (subject prefix)"],
            "query_plans": {
                "subject": "binary search the chunk directory, scan forward while the 0x83+subject prefix matches",
                "predicate_object": "full scan of every chunk (no secondary index)"
            },
            "durability": "whole-file rewrite with no fsync"
        })
    }
}

// --- redb_tuple ------------------------------------------------------------

const SPO: TableDefinition<'static, &'static [u8], ()> = TableDefinition::new("spo");
const POS: TableDefinition<'static, &'static [u8], ()> = TableDefinition::new("pos");

struct RedbTuple {
    path: PathBuf,
    db: Database,
}

fn tuple3(a: &[u8], b: &[u8], c: &[u8]) -> Vec<u8> {
    TupleEncoder::new().bytes(a).bytes(b).bytes(c).build()
}

fn untuple3(key: &[u8]) -> [Term; 3] {
    let values = decode_tuple(key).unwrap();
    let terms: Vec<Term> = values
        .iter()
        .map(|v| match v {
            TupleValue::Bytes(b) => Term::from_bytes(b).unwrap(),
            other => panic!("tuple key element is not bytes: {other:?}"),
        })
        .collect();
    terms.try_into().unwrap()
}

impl RedbTuple {
    fn new(dir: &Path) -> Self {
        let path = dir.join("redb.redb");
        let db = Database::create(&path).unwrap();
        Self { path, db }
    }

    fn write(&self, triples: &[Triple], insert: bool) {
        let txn = self.db.begin_write().unwrap();
        {
            let mut spo = txn.open_table(SPO).unwrap();
            let mut pos = txn.open_table(POS).unwrap();
            for t in triples {
                let [s, p, o] = t.terms().map(bytes_of);
                if insert {
                    spo.insert(tuple3(&s, &p, &o).as_slice(), ()).unwrap();
                    pos.insert(tuple3(&p, &o, &s).as_slice(), ()).unwrap();
                } else {
                    spo.remove(tuple3(&s, &p, &o).as_slice()).unwrap();
                    pos.remove(tuple3(&p, &o, &s).as_slice()).unwrap();
                }
            }
        }
        txn.commit().unwrap();
    }

    fn prefix_scan(
        &self,
        table: TableDefinition<'static, &'static [u8], ()>,
        prefix: &[u8],
    ) -> Vec<[Term; 3]> {
        let txn = self.db.begin_read().unwrap();
        let table = txn.open_table(table).unwrap();
        let mut out = Vec::new();
        for entry in table.range::<&[u8]>(prefix..).unwrap() {
            let (key, _) = entry.unwrap();
            let key = key.value();
            if !key.starts_with(prefix) {
                break;
            }
            out.push(untuple3(key));
        }
        out
    }
}

impl Engine for RedbTuple {
    fn load(&mut self, triples: &[Triple]) {
        self.write(triples, true);
    }

    fn subject(&self, subject: &Term) -> Vec<Triple> {
        let prefix = TupleEncoder::new().bytes(&bytes_of(subject)).build();
        self.prefix_scan(SPO, &prefix)
            .into_iter()
            .map(|[s, p, o]| Triple::new(s, p, o))
            .collect()
    }

    fn predicate_object(&self, predicate: &Term, object: &Term) -> Vec<Term> {
        let prefix = TupleEncoder::new()
            .bytes(&bytes_of(predicate))
            .bytes(&bytes_of(object))
            .build();
        self.prefix_scan(POS, &prefix)
            .into_iter()
            .map(|[_, _, s]| s)
            .collect()
    }

    fn page(&self, after: Option<&[u8]>, limit: usize) -> (Vec<Triple>, Option<Vec<u8>>) {
        let txn = self.db.begin_read().unwrap();
        let table = txn.open_table(SPO).unwrap();
        let lower = after.map_or(Bound::Unbounded, Bound::Excluded);
        let mut out = Vec::new();
        let mut last = None;
        for entry in table
            .range::<&[u8]>((lower, Bound::Unbounded))
            .unwrap()
            .take(limit)
        {
            let (key, _) = entry.unwrap();
            let [s, p, o] = untuple3(key.value());
            out.push(Triple::new(s, p, o));
            last = Some(key.value().to_vec());
        }
        let next = if out.len() == limit { last } else { None };
        (out, next)
    }

    fn insert(&mut self, triple: &Triple) {
        self.write(std::slice::from_ref(triple), true);
    }

    fn delete(&mut self, triple: &Triple) {
        self.write(std::slice::from_ref(triple), false);
    }

    fn file_bytes(&self) -> u64 {
        file_len(&self.path)
    }

    fn compact(&mut self) -> u64 {
        while self.db.compact().unwrap() {}
        self.file_bytes()
    }

    fn metadata(&self) -> Json {
        json!({
            "engine": "redb",
            "engine_version": "4.6.0",
            "schema": "tables spo and pos: TableDefinition<&[u8], ()>; key = id_core::tuple bytes(s).bytes(p).bytes(o) (pos: p, o, s) over canonical term bytes",
            "indexes": ["spo tuple-key table", "pos tuple-key table"],
            "query_plans": {
                "subject": "range scan of spo from tuple prefix bytes(subject)",
                "predicate_object": "range scan of pos from tuple prefix bytes(predicate).bytes(object)"
            },
            "durability": "Durability::Immediate (default) on every commit"
        })
    }
}

// --- sqlite helpers --------------------------------------------------------

fn sqlite_version(conn: &Connection) -> String {
    conn.query_row("SELECT sqlite_version()", [], |r| r.get(0))
        .unwrap()
}

fn query_plan(conn: &Connection, sql: &str, params: &[&dyn rusqlite::ToSql]) -> String {
    let mut stmt = conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
    let rows: Vec<String> = stmt
        .query_map(params, |r| r.get::<_, String>(3))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    rows.join("; ")
}

fn sqlite_compact(conn: &Connection, path: &Path) -> u64 {
    conn.execute_batch("VACUUM").unwrap();
    file_len(path)
}

// --- sqlite_jsonb ----------------------------------------------------------

const JSONB_DDL: &str = "\
CREATE TABLE triples(id INTEGER PRIMARY KEY, doc BLOB NOT NULL);
CREATE UNIQUE INDEX triples_spo ON triples(doc->>'$.sk', doc->>'$.pk', doc->>'$.ok');
CREATE INDEX triples_po ON triples(doc->>'$.pk', doc->>'$.ok');";
const JSONB_SUBJECT: &str =
    "SELECT json(doc) FROM triples WHERE doc->>'$.sk' = ?1 ORDER BY doc->>'$.pk', doc->>'$.ok'";
const JSONB_PO: &str =
    "SELECT json(doc->'$.s') FROM triples WHERE doc->>'$.pk' = ?1 AND doc->>'$.ok' = ?2";

struct SqliteJsonb {
    path: PathBuf,
    conn: Connection,
}

fn term_json(term: &Term) -> Json {
    match term {
        Term::Typed { type_tag, bytes } => json!({"k": 0, "t": type_tag, "b": hex(bytes)}),
        Term::Object(ObjectRef::Content(id)) => {
            json!({"k": 1, "h": id.profile().token(), "d": hex(id.digest())})
        }
        Term::Object(ObjectRef::Envelope(id)) => {
            let id = id.content_id();
            json!({"k": 2, "h": id.profile().token(), "d": hex(id.digest())})
        }
    }
}

fn json_term(value: &Json) -> Term {
    let unhex = |field: &str| hex::decode(value[field].as_str().unwrap()).unwrap();
    let id = || {
        let profile = HashProfile::from_token(value["h"].as_str().unwrap()).unwrap();
        ContentId::new(profile, unhex("d").try_into().unwrap())
    };
    match value["k"].as_u64().unwrap() {
        0 => Term::typed(value["t"].as_u64().unwrap(), unhex("b")),
        1 => Term::Object(ObjectRef::Content(id())),
        2 => Term::Object(ObjectRef::Envelope(EnvelopeId::from_content_id(id()))),
        other => panic!("unknown kind {other}"),
    }
}

fn triple_doc(t: &Triple) -> String {
    let [s, p, o] = t.terms();
    json!({
        "sk": hex(&bytes_of(s)), "pk": hex(&bytes_of(p)), "ok": hex(&bytes_of(o)),
        "s": term_json(s), "p": term_json(p), "o": term_json(o),
    })
    .to_string()
}

fn doc_triple(text: &str) -> Triple {
    let doc: Json = serde_json::from_str(text).unwrap();
    Triple::new(
        json_term(&doc["s"]),
        json_term(&doc["p"]),
        json_term(&doc["o"]),
    )
}

impl SqliteJsonb {
    fn new(dir: &Path) -> Self {
        let path = dir.join("jsonb.sqlite");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(JSONB_DDL).unwrap();
        Self { path, conn }
    }

    fn insert_all(&self, triples: &[Triple]) {
        let txn = self.conn.unchecked_transaction().unwrap();
        {
            let mut stmt = txn
                .prepare_cached("INSERT OR IGNORE INTO triples(doc) VALUES (jsonb(?1))")
                .unwrap();
            for t in triples {
                stmt.execute([triple_doc(t)]).unwrap();
            }
        }
        txn.commit().unwrap();
    }
}

impl Engine for SqliteJsonb {
    fn load(&mut self, triples: &[Triple]) {
        self.insert_all(triples);
    }

    fn subject(&self, subject: &Term) -> Vec<Triple> {
        let mut stmt = self.conn.prepare_cached(JSONB_SUBJECT).unwrap();
        stmt.query_map([hex(&bytes_of(subject))], |r| r.get::<_, String>(0))
            .unwrap()
            .map(|r| doc_triple(&r.unwrap()))
            .collect()
    }

    fn predicate_object(&self, predicate: &Term, object: &Term) -> Vec<Term> {
        let mut stmt = self.conn.prepare_cached(JSONB_PO).unwrap();
        stmt.query_map([hex(&bytes_of(predicate)), hex(&bytes_of(object))], |r| {
            r.get::<_, String>(0)
        })
        .unwrap()
        .map(|r| json_term(&serde_json::from_str(&r.unwrap()).unwrap()))
        .collect()
    }

    fn page(&self, after: Option<&[u8]>, limit: usize) -> (Vec<Triple>, Option<Vec<u8>>) {
        let after = after.map_or(0, |c| i64::from_be_bytes(c.try_into().unwrap()));
        let mut stmt = self
            .conn
            .prepare_cached("SELECT id, json(doc) FROM triples WHERE id > ?1 ORDER BY id LIMIT ?2")
            .unwrap();
        let rows: Vec<(i64, String)> = stmt
            .query_map(params![after, i64::try_from(limit).unwrap()], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap()
            .map(Result::unwrap)
            .collect();
        let next = (rows.len() == limit).then(|| rows.last().unwrap().0.to_be_bytes().to_vec());
        (rows.iter().map(|(_, d)| doc_triple(d)).collect(), next)
    }

    fn insert(&mut self, triple: &Triple) {
        self.insert_all(std::slice::from_ref(triple));
    }

    fn delete(&mut self, triple: &Triple) {
        let [s, p, o] = triple.terms().map(|t| hex(&bytes_of(t)));
        self.conn
            .execute(
                "DELETE FROM triples WHERE doc->>'$.sk' = ?1 AND doc->>'$.pk' = ?2 AND doc->>'$.ok' = ?3",
                [s, p, o],
            )
            .unwrap();
    }

    fn file_bytes(&self) -> u64 {
        file_len(&self.path)
    }

    fn compact(&mut self) -> u64 {
        sqlite_compact(&self.conn, &self.path)
    }

    fn metadata(&self) -> Json {
        let probe = hex(&bytes_of(&user(0)));
        let follows = hex(&bytes_of(&iri("follows")));
        json!({
            "engine": "SQLite (rusqlite 0.42, bundled)",
            "engine_version": sqlite_version(&self.conn),
            "schema": JSONB_DDL,
            "indexes": ["triples_spo unique expression index (sk, pk, ok)", "triples_po expression index (pk, ok)"],
            "query_plans": {
                "subject": query_plan(&self.conn, JSONB_SUBJECT, &[&probe]),
                "predicate_object": query_plan(&self.conn, JSONB_PO, &[&follows, &probe]),
            },
            "durability": "journal_mode DELETE, synchronous FULL (defaults) on every commit"
        })
    }
}

// --- sqlite_blob_rows ------------------------------------------------------

const ROWS_DDL: &str = "\
CREATE TABLE terms(id INTEGER PRIMARY KEY, bytes BLOB NOT NULL UNIQUE);
CREATE TABLE triples(s INTEGER NOT NULL, p INTEGER NOT NULL, o INTEGER NOT NULL, PRIMARY KEY(s, p, o)) WITHOUT ROWID;
CREATE INDEX triples_pos ON triples(p, o, s);";
const ROWS_SELECT: &str = "SELECT ts.bytes, tp.bytes, tobj.bytes FROM triples t \
JOIN terms ts ON ts.id = t.s JOIN terms tp ON tp.id = t.p JOIN terms tobj ON tobj.id = t.o";
const ROWS_SUBJECT: &str = "SELECT tp.bytes, tobj.bytes FROM terms ts \
JOIN triples t ON t.s = ts.id JOIN terms tp ON tp.id = t.p JOIN terms tobj ON tobj.id = t.o \
WHERE ts.bytes = ?1 ORDER BY t.p, t.o";
const ROWS_PO: &str = "SELECT ts.bytes FROM terms tp, terms tobj \
JOIN triples t ON t.p = tp.id AND t.o = tobj.id JOIN terms ts ON ts.id = t.s \
WHERE tp.bytes = ?1 AND tobj.bytes = ?2";

struct SqliteRows {
    path: PathBuf,
    conn: Connection,
}

impl SqliteRows {
    fn new(dir: &Path) -> Self {
        let path = dir.join("rows.sqlite");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(ROWS_DDL).unwrap();
        Self { path, conn }
    }

    fn term_id(conn: &Connection, bytes: &[u8], create: bool) -> Option<i64> {
        if create {
            conn.prepare_cached(
                "INSERT INTO terms(bytes) VALUES (?1) ON CONFLICT(bytes) DO NOTHING",
            )
            .unwrap()
            .execute([bytes])
            .unwrap();
        }
        conn.prepare_cached("SELECT id FROM terms WHERE bytes = ?1")
            .unwrap()
            .query_row([bytes], |r| r.get(0))
            .optional()
            .unwrap()
    }

    fn insert_all(&self, triples: &[Triple]) {
        let txn = self.conn.unchecked_transaction().unwrap();
        for t in triples {
            let [s, p, o] = t
                .terms()
                .map(|term| Self::term_id(&txn, &bytes_of(term), true).unwrap());
            txn.prepare_cached("INSERT OR IGNORE INTO triples(s, p, o) VALUES (?1, ?2, ?3)")
                .unwrap()
                .execute([s, p, o])
                .unwrap();
        }
        txn.commit().unwrap();
    }
}

impl Engine for SqliteRows {
    fn load(&mut self, triples: &[Triple]) {
        self.insert_all(triples);
    }

    fn subject(&self, subject: &Term) -> Vec<Triple> {
        let mut stmt = self.conn.prepare_cached(ROWS_SUBJECT).unwrap();
        stmt.query_map([bytes_of(subject)], |r| {
            Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, Vec<u8>>(1)?))
        })
        .unwrap()
        .map(|r| {
            let (p, o) = r.unwrap();
            Triple::new(
                subject.clone(),
                Term::from_bytes(&p).unwrap(),
                Term::from_bytes(&o).unwrap(),
            )
        })
        .collect()
    }

    fn predicate_object(&self, predicate: &Term, object: &Term) -> Vec<Term> {
        let mut stmt = self.conn.prepare_cached(ROWS_PO).unwrap();
        stmt.query_map([bytes_of(predicate), bytes_of(object)], |r| {
            r.get::<_, Vec<u8>>(0)
        })
        .unwrap()
        .map(|r| Term::from_bytes(&r.unwrap()).unwrap())
        .collect()
    }

    fn page(&self, after: Option<&[u8]>, limit: usize) -> (Vec<Triple>, Option<Vec<u8>>) {
        let cursor: [i64; 3] = after.map_or([0, 0, 0], |c| {
            [0, 8, 16].map(|i| i64::from_be_bytes(c[i..i + 8].try_into().unwrap()))
        });
        let sql = format!(
            "SELECT t.s, t.p, t.o, ts.bytes, tp.bytes, tobj.bytes FROM triples t \
             JOIN terms ts ON ts.id = t.s JOIN terms tp ON tp.id = t.p JOIN terms tobj ON tobj.id = t.o \
             WHERE (t.s, t.p, t.o) > (?1, ?2, ?3) ORDER BY t.s, t.p, t.o LIMIT {limit}"
        );
        let mut stmt = self.conn.prepare_cached(&sql).unwrap();
        let mut last = None;
        let rows: Vec<Triple> = stmt
            .query_map(cursor, |r| {
                Ok((
                    [r.get::<_, i64>(0)?, r.get(1)?, r.get(2)?],
                    [r.get::<_, Vec<u8>>(3)?, r.get(4)?, r.get(5)?],
                ))
            })
            .unwrap()
            .map(|r| {
                let (ids, [s, p, o]) = r.unwrap();
                last = Some(
                    ids.iter()
                        .flat_map(|i| i.to_be_bytes())
                        .collect::<Vec<u8>>(),
                );
                Triple::new(
                    Term::from_bytes(&s).unwrap(),
                    Term::from_bytes(&p).unwrap(),
                    Term::from_bytes(&o).unwrap(),
                )
            })
            .collect();
        let next = if rows.len() == limit { last } else { None };
        (rows, next)
    }

    fn insert(&mut self, triple: &Triple) {
        self.insert_all(std::slice::from_ref(triple));
    }

    fn delete(&mut self, triple: &Triple) {
        let ids: Option<Vec<i64>> = triple
            .terms()
            .iter()
            .map(|t| Self::term_id(&self.conn, &bytes_of(t), false))
            .collect();
        if let Some(ids) = ids {
            self.conn
                .execute(
                    "DELETE FROM triples WHERE s = ?1 AND p = ?2 AND o = ?3",
                    [ids[0], ids[1], ids[2]],
                )
                .unwrap();
        }
    }

    fn file_bytes(&self) -> u64 {
        file_len(&self.path)
    }

    fn compact(&mut self) -> u64 {
        sqlite_compact(&self.conn, &self.path)
    }

    fn metadata(&self) -> Json {
        let probe = bytes_of(&user(0));
        let follows = bytes_of(&iri("follows"));
        json!({
            "engine": "SQLite (rusqlite 0.42, bundled)",
            "engine_version": sqlite_version(&self.conn),
            "schema": ROWS_DDL,
            "indexes": ["terms.bytes unique", "triples primary key (s, p, o)", "triples_pos (p, o, s)"],
            "query_plans": {
                "subject": query_plan(&self.conn, ROWS_SUBJECT, &[&probe]),
                "predicate_object": query_plan(&self.conn, ROWS_PO, &[&follows, &probe]),
            },
            "durability": "journal_mode DELETE, synchronous FULL (defaults) on every commit; deleted triples leave their interned terms",
            "select_all": ROWS_SELECT,
        })
    }
}

// ---------------------------------------------------------------------------
// The matrix.
// ---------------------------------------------------------------------------

fn open(name: &str, dir: &Path) -> Box<dyn Engine> {
    match name {
        "chunked_binary" => Box::new(Chunked::new(dir)),
        "redb_tuple" => Box::new(RedbTuple::new(dir)),
        "sqlite_jsonb" => Box::new(SqliteJsonb::new(dir)),
        "sqlite_blob_rows" => Box::new(SqliteRows::new(dir)),
        other => panic!("unknown engine {other}"),
    }
}

/// Every triple the engine holds, through its own pager.
fn stream(engine: &dyn Engine) -> (Vec<Triple>, usize) {
    let mut out = Vec::new();
    let mut pages = 0;
    let mut cursor: Option<Vec<u8>> = None;
    loop {
        let (page, next) = engine.page(cursor.as_deref(), PAGE_SIZE);
        assert!(page.len() <= PAGE_SIZE);
        if !page.is_empty() {
            pages += 1;
        }
        out.extend(page);
        match next {
            Some(n) => cursor = Some(n),
            None => break,
        }
    }
    (out, pages)
}

fn triple_digest(triples: &[Triple]) -> String {
    set_digest(triples.iter().map(|t| t.to_bytes().unwrap()))
}

fn check_set(name: &str, what: &str, got: &[Triple], expected: &BTreeMap<Vec<u8>, Triple>) {
    let got: BTreeMap<Vec<u8>, Triple> = got
        .iter()
        .map(|t| (t.to_bytes().unwrap(), t.clone()))
        .collect();
    assert_eq!(
        &got, expected,
        "{name}: {what} differs from the reference model"
    );
}

fn run_engine(name: &str, run: usize, reference: &BTreeMap<Vec<u8>, Triple>) -> (Json, Json) {
    let dir = tempfile::tempdir().unwrap();
    let mut engine = open(name, dir.path());
    let triples: Vec<Triple> = reference.values().cloned().collect();

    let start = Instant::now();
    engine.load(&triples);
    let load = json!({"triples": triples.len(), "micros": micros(start)});

    let file_bytes = engine.file_bytes();
    let compacted = engine.compact();
    let storage = json!({"file_bytes": file_bytes, "compacted_bytes": compacted});

    // Byte/type round trip: every stored triple decodes to exactly the
    // original terms, type tags and reference kinds included.
    let start = Instant::now();
    let (all, _) = stream(engine.as_ref());
    let mismatches = all
        .iter()
        .filter(|t| reference.get(&t.to_bytes().unwrap()) != Some(t))
        .count()
        + reference.len().abs_diff(all.len());
    let round_trip =
        json!({"checked": all.len(), "mismatches": mismatches, "micros": micros(start)});
    assert_eq!(mismatches, 0, "{name}: round trip");

    let start = Instant::now();
    let mut subject_rows = Vec::new();
    for s in subject_queries() {
        let got = engine.subject(&s);
        let expected: BTreeMap<_, _> = reference
            .iter()
            .filter(|(_, t)| t.subject == s)
            .map(|(k, t)| (k.clone(), t.clone()))
            .collect();
        check_set(name, "subject query", &got, &expected);
        assert_eq!(
            got.len(),
            expected.len(),
            "{name}: subject query repeats a row"
        );
        subject_rows.push(json!({
            "subject": hex(&bytes_of(&s)), "count": got.len(), "sha256": triple_digest(&got),
        }));
    }
    let query_subject = json!({"queries": subject_rows, "micros": micros(start)});

    let start = Instant::now();
    let mut po_rows = Vec::new();
    for (p, o) in po_queries() {
        let got = engine.predicate_object(&p, &o);
        let expected: BTreeSet<Vec<u8>> = reference
            .values()
            .filter(|t| t.predicate == p && t.object == o)
            .map(|t| bytes_of(&t.subject))
            .collect();
        let got_bytes: Vec<Vec<u8>> = got.iter().map(bytes_of).collect();
        assert_eq!(got_bytes.len(), expected.len(), "{name}: po query count");
        assert_eq!(
            got_bytes.iter().cloned().collect::<BTreeSet<_>>(),
            expected,
            "{name}: po query"
        );
        po_rows.push(json!({
            "predicate": hex(&bytes_of(&p)), "object": hex(&bytes_of(&o)),
            "count": got.len(), "sha256": set_digest(got_bytes),
        }));
    }
    let query_po = json!({"queries": po_rows, "micros": micros(start)});

    let start = Instant::now();
    let (paged, page_count) = stream(engine.as_ref());
    let micros_page = micros(start);
    let distinct: BTreeSet<Vec<u8>> = paged.iter().map(|t| t.to_bytes().unwrap()).collect();
    let duplicates = paged.len() - distinct.len();
    check_set(name, "paged stream", &paged, reference);
    let page_stream = json!({
        "page_size": PAGE_SIZE, "pages": page_count, "count": paged.len(),
        "duplicates": duplicates, "sha256": triple_digest(&paged), "micros": micros_page,
    });

    let inserts = new_triples();
    let deletes = deleted(reference);
    let start = Instant::now();
    for t in &inserts {
        engine.insert(t);
    }
    for t in &deletes {
        engine.delete(t);
    }
    let micros_write = micros(start);
    let mut after = reference.clone();
    for t in &inserts {
        after.insert(t.to_bytes().unwrap(), t.clone());
    }
    for t in &deletes {
        after.remove(&t.to_bytes().unwrap());
    }
    let (final_rows, _) = stream(engine.as_ref());
    check_set(name, "state after writes", &final_rows, &after);
    let write = json!({
        "inserts": inserts.len(), "deletes": deletes.len(), "transactions": inserts.len() + deletes.len(),
        "micros": micros_write, "final_count": final_rows.len(), "final_sha256": triple_digest(&final_rows),
        "file_bytes_after": engine.file_bytes(),
    });

    let cell = json!({
        "engine": name, "run": run,
        "load": load, "storage": storage, "round_trip": round_trip,
        "query_subject": query_subject, "query_predicate_object": query_po,
        "page_stream": page_stream, "write": write,
    });
    (cell, engine.metadata())
}

fn fixture(reference: &BTreeMap<Vec<u8>, Triple>) -> Json {
    json!({
        "generator": "splitmix64/v1",
        "seed": SEED,
        "subjects": SUBJECTS,
        "triples": reference.len(),
        "sha256": set_digest(reference.keys().cloned()),
        "subject_queries": SUBJECT_QUERIES,
        "predicate_object_queries": PO_QUERIES,
        "new_triples": NEW_TRIPLES,
        "delete_stride": DELETE_STRIDE,
        "chunk_bytes": CHUNK_BYTES,
        "type_tags": {"iri": TAG_IRI, "text": TAG_TEXT, "bytes": TAG_BYTES, "i64_be": TAG_I64},
    })
}

const LIMITS: [&str; 6] = [
    "single machine, single process, one thread; timings are wall-clock microseconds for the given build profile and are only comparable within one run",
    "corpus is synthetic (about 6000 triples, 1000 subjects); production graphs may differ in size, skew and term mix",
    "durability differs by engine: chunked_binary does not fsync, redb and SQLite fsync on each commit with their default settings",
    "chunked_binary has no secondary index, so predicate/object queries scan the whole file and every write rewrites it",
    "sqlite_blob_rows never collects interned terms whose triples were deleted",
    "storage bytes depend on the pinned engine versions and page sizes; other versions or settings may differ",
];

#[test]
fn graph_encoding_bakeoff() {
    let reference = corpus();
    let out = std::env::var_os("BAKEOFF_RESULTS");
    let runs = if out.is_some() { RUNS } else { 1 };
    let mut cells = Vec::new();
    let mut engines = Vec::new();
    for run in 0..runs {
        for name in ENGINES {
            let (cell, meta) = run_engine(name, run, &reference);
            cells.push(cell);
            if run == 0 {
                let mut meta = meta;
                meta["name"] = json!(name);
                engines.push(meta);
            }
        }
    }
    let Some(out) = out else { return };
    let results = json!({
        "schema": "id-graph-encoding-bakeoff/v1",
        "issue": 10219,
        "slice": "A-GRAPH-9125-03",
        "build": {"profile": if cfg!(debug_assertions) { "debug" } else { "release" }},
        "fixture": fixture(&reference),
        "engines": engines,
        "workloads": WORKLOADS,
        "runs": runs,
        "cells": cells,
        "limits": LIMITS,
    });
    let mut text = serde_json::to_string_pretty(&results).unwrap();
    text.push('\n');
    fs::write(out, text).unwrap();
}
