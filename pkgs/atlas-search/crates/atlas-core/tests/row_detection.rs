//! Row-store detector tests (#9982, design
//! `doc/2026-11-10T22-56-00Z_design_atlas_delta_detection_join` §2.3).
//!
//! Oracles: the rows the test itself wrote, hashed with an
//! independent implementation of the canonical encoding the design
//! names (BLAKE3 over tag/length/payload per column, in
//! table-definition order) — written here from the design text, not
//! imported from the detector. The #6028 guarantee is pinned the
//! same way as for the other mechanisms: an unchanged store lands
//! zero verdicts and an identical landed map. The §2.3 cost claims
//! are pinned through `bytes_read`: rows below the landed watermark
//! are never read; only the boundary region is verified per row.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use atlas_core::{
    DetectionLedger, Mechanism, RowDetectOptions, RowMode, RowStoreSpec, TableSpec, Verdict,
    WatermarkSpec, WatermarkValue,
};
use rusqlite::types::Value;

/// The canonical encoding, from the design: per column one tag byte
/// (`n`/`i`/`f`/`t`/`b`), an 8-byte big-endian length, the payload.
fn canonical(values: &[Value]) -> Vec<u8> {
    let mut out = Vec::new();
    for value in values {
        let (tag, payload): (u8, Vec<u8>) = match value {
            Value::Null => (b'n', Vec::new()),
            Value::Integer(i) => (b'i', i.to_be_bytes().to_vec()),
            Value::Real(f) => (b'f', f.to_bits().to_be_bytes().to_vec()),
            Value::Text(t) => (b't', t.as_bytes().to_vec()),
            Value::Blob(b) => (b'b', b.clone()),
        };
        out.push(tag);
        out.extend_from_slice(&(payload.len() as u64).to_be_bytes());
        out.extend_from_slice(&payload);
    }
    out
}

fn row_hash(values: &[Value]) -> String {
    blake3::hash(&canonical(values)).to_hex().to_string()
}

fn text(s: &str) -> Value {
    Value::Text(s.to_string())
}

fn int(i: i64) -> Value {
    Value::Integer(i)
}

struct Store {
    dir: PathBuf,
    db: PathBuf,
}

impl Store {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "atlas-rows-{tag}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("store.db");
        let store = Store { dir, db };
        store.exec(
            "CREATE TABLE messages (id INTEGER PRIMARY KEY, body TEXT, updated_at INTEGER);
             CREATE TABLE events (seq INTEGER PRIMARY KEY, payload TEXT);
             CREATE TABLE kv (k TEXT PRIMARY KEY, v TEXT);",
        );
        store
    }

    fn path(&self) -> &Path {
        &self.db
    }

    /// Arrange store state through a short-lived writable
    /// connection, closed before any detection pass opens the store
    /// read-only.
    fn exec(&self, sql: &str) {
        let con = rusqlite::Connection::open(&self.db).unwrap();
        con.execute_batch(sql).unwrap();
    }
}

impl Drop for Store {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// The session-store table: mutable, `updated_at` watermark.
fn messages_spec() -> RowStoreSpec {
    RowStoreSpec {
        tables: vec![TableSpec {
            table: "messages".to_string(),
            key_columns: vec!["id".to_string()],
            watermark: WatermarkSpec::UpdateSensitive {
                column: "updated_at".to_string(),
                tiebreaker: None,
            },
            append_only: false,
        }],
    }
}

fn seeded_messages(tag: &str) -> Store {
    let store = Store::new(tag);
    store.exec(
        "INSERT INTO messages VALUES (1, 'alpha', 10);
         INSERT INTO messages VALUES (2, 'bravo', 20);
         INSERT INTO messages VALUES (3, 'charlie', 30);",
    );
    store
}

fn message_hash(id: i64, body: &str, updated_at: i64) -> String {
    row_hash(&[int(id), text(body), int(updated_at)])
}

#[test]
fn first_ingest_lands_every_row_new_and_advances_the_watermark() {
    let store = seeded_messages("first");
    let mut ledger = DetectionLedger::new();
    let pass = ledger
        .detect_row_store(
            store.path(),
            "sessions",
            &messages_spec(),
            &RowDetectOptions::default(),
        )
        .unwrap();

    assert_eq!(pass.mechanism, Mechanism::RowWatermark);
    assert_eq!(pass.from, None);
    assert_eq!(pass.counts.new, 3);
    assert_eq!(
        pass.counts.unchanged + pass.counts.changed + pass.counts.gone,
        0
    );
    // Every row was read exactly once.
    let expected_bytes: u64 = [
        canonical(&[int(1), text("alpha"), int(10)]).len(),
        canonical(&[int(2), text("bravo"), int(20)]).len(),
        canonical(&[int(3), text("charlie"), int(30)]).len(),
    ]
    .iter()
    .sum::<usize>() as u64;
    assert_eq!(pass.bytes_read, expected_bytes);

    assert_eq!(pass.tables.len(), 1);
    let table = &pass.tables[0];
    assert_eq!(table.mode, RowMode::Watermark);
    assert_eq!(table.watermark_from, None);
    assert_eq!(
        table.watermark_to,
        Some(WatermarkValue {
            primary: 30,
            tiebreaker: "i:3".to_string(),
        })
    );

    let landed = ledger.source("sessions").unwrap().landed.as_ref().unwrap();
    let messages = &landed.tables["messages"];
    assert_eq!(messages.rows.len(), 3);
    assert_eq!(
        messages.rows.get("i:2"),
        Some(&message_hash(2, "bravo", 20))
    );
    assert_eq!(
        messages.watermark,
        Some(WatermarkValue {
            primary: 30,
            tiebreaker: "i:3".to_string(),
        })
    );
}

#[test]
fn unchanged_store_lands_no_verdicts_and_reads_only_the_boundary() {
    let store = seeded_messages("unchanged");
    let mut ledger = DetectionLedger::new();
    let spec = messages_spec();
    let first = ledger
        .detect_row_store(
            store.path(),
            "sessions",
            &spec,
            &RowDetectOptions::default(),
        )
        .unwrap();
    let landed_before = ledger.source("sessions").unwrap().landed.clone().unwrap();

    let pass = ledger
        .detect_row_store(
            store.path(),
            "sessions",
            &spec,
            &RowDetectOptions::default(),
        )
        .unwrap();

    // No false deltas: the watermark does not cry wolf.
    assert!(pass.verdicts.is_empty(), "verdicts: {:?}", pass.verdicts);
    assert_eq!(pass.counts.unchanged, 3);
    assert_eq!(pass.counts.changed + pass.counts.new + pass.counts.gone, 0);
    // Only the boundary region (rows sharing the landed primary,
    // here just row 3) was read and verified per row.
    assert_eq!(
        pass.bytes_read,
        canonical(&[int(3), text("charlie"), int(30)]).len() as u64
    );
    assert_eq!(pass.stat_presumption_rate, Some(2.0 / 3.0));
    // The landed state is byte-identical: no writes of any kind.
    assert_eq!(pass.from.as_deref(), Some(first.to.as_str()));
    assert_eq!(pass.to, first.to);
    assert_eq!(
        ledger.source("sessions").unwrap().landed.as_ref().unwrap(),
        &landed_before
    );
}

#[test]
fn in_place_update_with_advanced_watermark_is_an_edit() {
    let store = seeded_messages("edit");
    let mut ledger = DetectionLedger::new();
    let spec = messages_spec();
    ledger
        .detect_row_store(
            store.path(),
            "sessions",
            &spec,
            &RowDetectOptions::default(),
        )
        .unwrap();

    store.exec("UPDATE messages SET body = 'bravo edited', updated_at = 40 WHERE id = 2;");
    let pass = ledger
        .detect_row_store(
            store.path(),
            "sessions",
            &spec,
            &RowDetectOptions::default(),
        )
        .unwrap();

    assert_eq!(pass.counts.changed, 1);
    assert_eq!(pass.counts.unchanged, 2);
    assert_eq!(pass.verdicts.len(), 1);
    let verdict = &pass.verdicts[0];
    assert_eq!(verdict.path, "messages/i:2");
    assert_eq!(verdict.verdict, Verdict::Changed);
    // The superseded version is named, not silently replaced.
    assert_eq!(verdict.previous_blob, Some(message_hash(2, "bravo", 20)));
    assert_eq!(verdict.blob, Some(message_hash(2, "bravo edited", 40)));
    assert_eq!(pass.tables[0].watermark_to.as_ref().unwrap().primary, 40);

    let messages = &ledger
        .source("sessions")
        .unwrap()
        .landed
        .as_ref()
        .unwrap()
        .tables["messages"];
    assert_eq!(
        messages.rows.get("i:2"),
        Some(&message_hash(2, "bravo edited", 40))
    );
    // Untouched rows keep their landed hashes exactly.
    assert_eq!(
        messages.rows.get("i:1"),
        Some(&message_hash(1, "alpha", 10))
    );
    assert_eq!(
        messages.rows.get("i:3"),
        Some(&message_hash(3, "charlie", 30))
    );
}

#[test]
fn edit_inside_the_boundary_region_is_caught_by_hash_verification() {
    let store = seeded_messages("boundary");
    let mut ledger = DetectionLedger::new();
    let spec = messages_spec();
    ledger
        .detect_row_store(
            store.path(),
            "sessions",
            &spec,
            &RowDetectOptions::default(),
        )
        .unwrap();

    // Row 3 sits at the landed watermark (30). Editing it without
    // advancing the watermark hides it from the watermark itself —
    // the boundary region's per-row hash verification is what
    // catches it.
    store.exec("UPDATE messages SET body = 'charlie edited' WHERE id = 3;");
    let pass = ledger
        .detect_row_store(
            store.path(),
            "sessions",
            &spec,
            &RowDetectOptions::default(),
        )
        .unwrap();

    assert_eq!(pass.counts.changed, 1);
    assert_eq!(pass.verdicts.len(), 1);
    assert_eq!(pass.verdicts[0].path, "messages/i:3");
    assert_eq!(
        pass.verdicts[0].previous_blob,
        Some(message_hash(3, "charlie", 30))
    );
    assert_eq!(
        pass.verdicts[0].blob,
        Some(message_hash(3, "charlie edited", 30))
    );
}

#[test]
fn edit_below_the_watermark_without_advancing_it_stays_presumed() {
    // The §2.3 contract, pinned honestly: the watermark must be
    // update-sensitive, and detection trusts it below the boundary.
    // A store that edits in place without advancing its watermark
    // has broken the declaration, not the detector — which is why a
    // mutable table may never declare a bare sequence watermark
    // (see mutable_table_with_sequence_watermark_is_refused).
    let store = seeded_messages("presumed");
    let mut ledger = DetectionLedger::new();
    let spec = messages_spec();
    ledger
        .detect_row_store(
            store.path(),
            "sessions",
            &spec,
            &RowDetectOptions::default(),
        )
        .unwrap();

    store.exec("UPDATE messages SET body = 'alpha edited' WHERE id = 1;");
    let pass = ledger
        .detect_row_store(
            store.path(),
            "sessions",
            &spec,
            &RowDetectOptions::default(),
        )
        .unwrap();

    assert!(pass.verdicts.is_empty());
    assert_eq!(pass.counts.unchanged, 3);
    // Row 1 was never read: only the boundary row's bytes were.
    assert_eq!(
        pass.bytes_read,
        canonical(&[int(3), text("charlie"), int(30)]).len() as u64
    );
}

#[test]
fn append_after_the_watermark_is_new_and_below_rows_are_not_reread() {
    let store = seeded_messages("append");
    let mut ledger = DetectionLedger::new();
    let spec = messages_spec();
    ledger
        .detect_row_store(
            store.path(),
            "sessions",
            &spec,
            &RowDetectOptions::default(),
        )
        .unwrap();

    store.exec("INSERT INTO messages VALUES (4, 'delta', 40);");
    let pass = ledger
        .detect_row_store(
            store.path(),
            "sessions",
            &spec,
            &RowDetectOptions::default(),
        )
        .unwrap();

    assert_eq!(pass.counts.new, 1);
    assert_eq!(pass.counts.unchanged, 3);
    assert_eq!(pass.verdicts.len(), 1);
    assert_eq!(pass.verdicts[0].path, "messages/i:4");
    // Read: the boundary row (30) and the new row (40). Rows 1–2
    // stayed below the watermark and were never opened.
    let expected = (canonical(&[int(3), text("charlie"), int(30)]).len()
        + canonical(&[int(4), text("delta"), int(40)]).len()) as u64;
    assert_eq!(pass.bytes_read, expected);
    assert_eq!(pass.tables[0].watermark_to.as_ref().unwrap().primary, 40);
}

#[test]
fn hard_delete_is_invisible_until_the_tombstone_sweep() {
    let store = seeded_messages("delete");
    let mut ledger = DetectionLedger::new();
    let spec = messages_spec();
    ledger
        .detect_row_store(
            store.path(),
            "sessions",
            &spec,
            &RowDetectOptions::default(),
        )
        .unwrap();

    store.exec("DELETE FROM messages WHERE id = 1;");
    // Without the sweep the watermark cannot see a hard delete: the
    // key is simply never read, and the row stays presumed.
    let pass = ledger
        .detect_row_store(
            store.path(),
            "sessions",
            &spec,
            &RowDetectOptions::default(),
        )
        .unwrap();
    assert_eq!(pass.counts.gone, 0);
    assert_eq!(pass.counts.unchanged, 3);
    assert!(
        ledger
            .source("sessions")
            .unwrap()
            .landed
            .as_ref()
            .unwrap()
            .tables["messages"]
            .rows
            .contains_key("i:1")
    );

    // The sweep's full key listing catches it.
    let pass = ledger
        .detect_row_store(
            store.path(),
            "sessions",
            &spec,
            &RowDetectOptions {
                tombstone_sweep: true,
            },
        )
        .unwrap();
    assert!(pass.tables[0].swept);
    assert_eq!(pass.counts.gone, 1);
    assert_eq!(pass.counts.unchanged, 2);
    assert_eq!(pass.verdicts.len(), 1);
    let verdict = &pass.verdicts[0];
    assert_eq!(verdict.path, "messages/i:1");
    assert_eq!(verdict.verdict, Verdict::Gone);
    assert_eq!(verdict.blob, None);
    assert_eq!(verdict.previous_blob, Some(message_hash(1, "alpha", 10)));
    assert!(
        !ledger
            .source("sessions")
            .unwrap()
            .landed
            .as_ref()
            .unwrap()
            .tables["messages"]
            .rows
            .contains_key("i:1")
    );
}

#[test]
fn mutable_table_with_sequence_watermark_is_refused() {
    let store = seeded_messages("refused");
    let mut ledger = DetectionLedger::new();
    let spec = RowStoreSpec {
        tables: vec![TableSpec {
            table: "messages".to_string(),
            key_columns: vec!["id".to_string()],
            watermark: WatermarkSpec::Sequence {
                column: "id".to_string(),
            },
            append_only: false,
        }],
    };
    let err = ledger
        .detect_row_store(
            store.path(),
            "sessions",
            &spec,
            &RowDetectOptions::default(),
        )
        .unwrap_err();
    assert!(
        err.to_string().contains("mutable"),
        "unexpected error: {err}"
    );
    // Nothing was recorded: the refusal happens before any pass.
    assert!(ledger.source("sessions").is_none());
}

#[test]
fn append_only_table_may_use_a_sequence_watermark() {
    let store = Store::new("seq");
    store.exec("INSERT INTO events VALUES (1, 'boot'); INSERT INTO events VALUES (2, 'ready');");
    let spec = RowStoreSpec {
        tables: vec![TableSpec {
            table: "events".to_string(),
            key_columns: vec!["seq".to_string()],
            watermark: WatermarkSpec::Sequence {
                column: "seq".to_string(),
            },
            append_only: true,
        }],
    };
    let mut ledger = DetectionLedger::new();
    let pass = ledger
        .detect_row_store(
            store.path(),
            "sessions",
            &spec,
            &RowDetectOptions::default(),
        )
        .unwrap();
    assert_eq!(pass.counts.new, 2);
    assert_eq!(pass.tables[0].watermark_to.as_ref().unwrap().primary, 2);

    store.exec("INSERT INTO events VALUES (3, 'steady');");
    let pass = ledger
        .detect_row_store(
            store.path(),
            "sessions",
            &spec,
            &RowDetectOptions::default(),
        )
        .unwrap();
    assert_eq!(pass.counts.new, 1);
    assert_eq!(pass.counts.unchanged, 2);
    assert_eq!(pass.verdicts[0].path, "events/i:3");
}

#[test]
fn no_watermark_table_falls_back_to_hashing_every_row() {
    let store = Store::new("fallback");
    store.exec("INSERT INTO kv VALUES ('a', 'one'); INSERT INTO kv VALUES ('b', 'two');");
    let spec = RowStoreSpec {
        tables: vec![TableSpec {
            table: "kv".to_string(),
            key_columns: vec!["k".to_string()],
            watermark: WatermarkSpec::None,
            append_only: false,
        }],
    };
    let mut ledger = DetectionLedger::new();
    let pass = ledger
        .detect_row_store(
            store.path(),
            "sessions",
            &spec,
            &RowDetectOptions::default(),
        )
        .unwrap();
    assert_eq!(pass.tables[0].mode, RowMode::HashEveryRow);
    assert_eq!(pass.tables[0].watermark_to, None);
    assert_eq!(pass.counts.new, 2);

    // An edit and a delete in one pass, with no watermark anywhere
    // and no sweep requested: hashing every row sees both.
    store.exec("UPDATE kv SET v = 'one edited' WHERE k = 'a'; DELETE FROM kv WHERE k = 'b';");
    let pass = ledger
        .detect_row_store(
            store.path(),
            "sessions",
            &spec,
            &RowDetectOptions::default(),
        )
        .unwrap();
    assert_eq!(pass.counts.changed, 1);
    assert_eq!(pass.counts.gone, 1);
    let by_path: BTreeMap<&str, Verdict> = pass
        .verdicts
        .iter()
        .map(|v| (v.path.as_str(), v.verdict))
        .collect();
    assert_eq!(by_path["kv/t:a"], Verdict::Changed);
    assert_eq!(by_path["kv/t:b"], Verdict::Gone);
    // Every remaining row was read: exactly row a's canonical bytes.
    assert_eq!(
        pass.bytes_read,
        canonical(&[text("a"), text("one edited")]).len() as u64
    );
}

#[test]
fn mechanism_mixing_is_refused() {
    let store = seeded_messages("mixing");
    let mut ledger = DetectionLedger::new();
    ledger
        .detect_plain_tree(
            store.dir.as_path(),
            "sessions",
            &atlas_core::PlainTreeOptions::default(),
        )
        .unwrap();
    let err = ledger
        .detect_row_store(
            store.path(),
            "sessions",
            &messages_spec(),
            &RowDetectOptions::default(),
        )
        .unwrap_err();
    assert!(
        err.to_string().contains("plain_tree_hash_walk"),
        "unexpected error: {err}"
    );
}

/// One mutation in the model test. `Insert` upserts (a session
/// store write), `Update` touches only an existing row, `Delete`
/// removes it if present.
#[derive(Debug, Clone)]
enum Op {
    Insert { id: i64, body: String },
    Update { id: i64, body: String },
    Delete { id: i64 },
}

fn op_strategy() -> impl proptest::strategy::Strategy<Value = Op> {
    use proptest::prelude::*;
    let id = 1..=6i64;
    let body = "[a-z]{1,8}";
    prop_oneof![
        (id.clone(), body.clone()).prop_map(|(id, body)| Op::Insert { id, body }),
        (id.clone(), body).prop_map(|(id, body)| Op::Update { id, body }),
        id.prop_map(|id| Op::Delete { id }),
    ]
}

/// Model-based test (the testing doctrine's MBT): the oracle is a
/// plain map of the rows the test wrote. Random op sequences are
/// applied to the store and the model together; after each batch,
/// the detector's verdicts — run with the tombstone sweep on — must
/// equal the model diff exactly, and the landed hashes must equal
/// the oracle hashes of the model rows.
#[test]
fn verdicts_match_a_simple_model() {
    use proptest::prelude::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static CASE: AtomicU64 = AtomicU64::new(0);

    proptest!(|(ops in proptest::collection::vec(op_strategy(), 0..12))| {
        let store = Store::new(&format!("model{}", CASE.fetch_add(1, Ordering::Relaxed)));
        let spec = messages_spec();
        let options = RowDetectOptions { tombstone_sweep: true };
        let mut ledger = DetectionLedger::new();
        // The model: id -> (body, updated_at). The clock is the
        // store's update-sensitive watermark source: every write
        // takes the next tick, so an edit always advances its row
        // above the landed watermark, as §2.3 requires of the store.
        let mut model: BTreeMap<i64, (String, i64)> = BTreeMap::new();
        let mut clock = 100i64;

        // Two batches, so watermark advancement between passes is
        // exercised, not just first-ingest diffing.
        let mid = ops.len() / 2;
        for batch in [&ops[..mid], &ops[mid..]] {
            let before = model.clone();
            for op in batch {
                match op {
                    Op::Insert { id, body } => {
                        clock += 1;
                        store.exec(&format!(
                            "INSERT OR REPLACE INTO messages VALUES ({id}, '{body}', {clock});"
                        ));
                        model.insert(*id, (body.clone(), clock));
                    }
                    Op::Update { id, body } => {
                        if model.contains_key(id) {
                            clock += 1;
                            store.exec(&format!(
                                "UPDATE messages SET body = '{body}', updated_at = {clock} WHERE id = {id};"
                            ));
                            model.insert(*id, (body.clone(), clock));
                        }
                    }
                    Op::Delete { id } => {
                        store.exec(&format!("DELETE FROM messages WHERE id = {id};"));
                        model.remove(id);
                    }
                }
            }

            let pass = ledger
                .detect_row_store(store.path(), "sessions", &spec, &options)
                .unwrap();

            let mut expected: BTreeMap<String, Verdict> = BTreeMap::new();
            for (id, row) in &model {
                match before.get(id) {
                    None => {
                        expected.insert(format!("messages/i:{id}"), Verdict::New);
                    }
                    Some(old) if old != row => {
                        expected.insert(format!("messages/i:{id}"), Verdict::Changed);
                    }
                    _ => {}
                }
            }
            for id in before.keys() {
                if !model.contains_key(id) {
                    expected.insert(format!("messages/i:{id}"), Verdict::Gone);
                }
            }
            let actual: BTreeMap<String, Verdict> = pass
                .verdicts
                .iter()
                .map(|v| (v.path.clone(), v.verdict))
                .collect();
            prop_assert_eq!(&actual, &expected);

            let changed = expected.values().filter(|v| **v == Verdict::Changed).count() as u64;
            let gone = expected.values().filter(|v| **v == Verdict::Gone).count() as u64;
            prop_assert_eq!(pass.counts.changed, changed);
            prop_assert_eq!(pass.counts.gone, gone);
            prop_assert_eq!(
                pass.counts.unchanged,
                before.len() as u64 - changed - gone
            );

            // The landed state is the model, hash for hash.
            let landed = ledger.source("sessions").unwrap().landed.as_ref().unwrap();
            let rows = &landed.tables["messages"].rows;
            prop_assert_eq!(rows.len(), model.len());
            for (id, (body, updated_at)) in &model {
                prop_assert_eq!(
                    rows.get(&format!("i:{id}")),
                    Some(&message_hash(*id, body, *updated_at))
                );
            }
        }
    });
}

#[test]
fn non_integer_watermark_fails_closed() {
    let store = Store::new("badwm");
    store.exec("INSERT INTO messages VALUES (1, 'alpha', 'soon');");
    let mut ledger = DetectionLedger::new();
    let err = ledger
        .detect_row_store(
            store.path(),
            "sessions",
            &messages_spec(),
            &RowDetectOptions::default(),
        )
        .unwrap_err();
    assert!(
        err.to_string().contains("not an integer"),
        "unexpected error: {err}"
    );
}
