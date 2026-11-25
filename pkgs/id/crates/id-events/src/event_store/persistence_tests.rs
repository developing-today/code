//! Native-redb oracle for the shared persistence helpers, not full ingest.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::{Arc, Mutex};

use base64::Engine as _;
use id_store::chain::{ChainExt, sign};
use id_store::event_rows::{
    EventWriteTransaction, RowRead, RowWrite, TenantWrite, with_write_transaction,
};
use id_store::turso::{PipelineTransport, TenantStore, TursoError};
use p2panda_core::SigningKey;
use rusqlite::types::Value;
use serde_json::{Value as Json, json};

use super::*;

/// Real SQLite executing the pipeline's bound SQL. This fixture deliberately
/// owns one session/connection; it makes no claims about HTTP or baton routing.
#[derive(Clone, Debug)]
struct SqliteEndpoint(Arc<Mutex<rusqlite::Connection>>);

impl SqliteEndpoint {
    fn new() -> Self {
        Self(Arc::new(Mutex::new(
            rusqlite::Connection::open_in_memory().unwrap(),
        )))
    }
}

impl PipelineTransport for SqliteEndpoint {
    fn pipeline(&self, body: &[u8]) -> Result<Vec<u8>, TursoError> {
        let request: Json = serde_json::from_slice(body).unwrap();
        let db = self.0.lock().unwrap();
        let mut results = Vec::new();
        for request in request["requests"].as_array().unwrap() {
            if request["type"] == "close" {
                // Model session disposal, including an uncommitted transaction.
                if !db.is_autocommit() {
                    db.execute_batch("ROLLBACK").unwrap();
                }
                results.push(json!({"type":"ok","response":{"type":"close"}}));
                continue;
            }
            let stmt = &request["stmt"];
            let args: Vec<Value> = stmt["args"]
                .as_array()
                .unwrap()
                .iter()
                .map(|arg| match arg["type"].as_str().unwrap() {
                    "text" => Value::Text(arg["value"].as_str().unwrap().to_owned()),
                    "blob" => Value::Blob(
                        base64::engine::general_purpose::STANDARD
                            .decode(arg["value"].as_str().unwrap())
                            .unwrap(),
                    ),
                    "integer" => Value::Integer(arg["value"].as_str().unwrap().parse().unwrap()),
                    other => unreachable!("row fixture only expects text/blob, got {other}"),
                })
                .collect();
            let mut prepared = db.prepare(stmt["sql"].as_str().unwrap()).unwrap();
            let columns: Vec<_> = prepared
                .column_names()
                .into_iter()
                .map(str::to_owned)
                .collect();
            let mut rows = Vec::new();
            let affected = if columns.is_empty() {
                prepared.execute(rusqlite::params_from_iter(args)).unwrap()
            } else {
                let mut query = prepared.query(rusqlite::params_from_iter(args)).unwrap();
                while let Some(row) = query.next().unwrap() {
                    let values: Vec<_> = (0..columns.len()).map(|i| match row.get::<_, Value>(i).unwrap() {
                        Value::Blob(bytes) => json!({"type":"blob","value":base64::engine::general_purpose::STANDARD.encode(bytes)}),
                        Value::Text(text) => json!({"type":"text","value":text}),
                        other => unreachable!("row fixture returned {other:?}"),
                    }).collect();
                    rows.push(values);
                }
                0
            };
            results.push(json!({"type":"ok","response":{"type":"execute","result":{
                "cols":columns.iter().map(|name| json!({"name":name})).collect::<Vec<_>>(),
                "rows":rows,"affected_row_count":affected,"last_insert_rowid":null
            }}}));
        }
        Ok(serde_json::to_vec(&json!({"baton":"single-session","results":results})).unwrap())
    }
}

fn event(kind: &str, group: Option<Hash>) -> Event {
    sign(
        &SigningKey::from_bytes(&[73; 32]),
        0,
        None,
        None,
        ChainExt {
            kind: kind.to_owned(),
            group,
            after: Vec::new(),
            tag: None,
            document: None,
            removes: None,
            note: None,
            link: None,
            checkpoint: None,
            rotation: None,
            chat: None,
            anchor: None,
        },
    )
    .unwrap()
}

/// The native-redb oracle is the pre-port persisted row format: header bytes
/// under position, hash->position, group->position, Chain::encode and the
/// canonical proof encoding. Run exactly the same helper script on both stores.
fn script(mut rows: impl RowWrite, events: &[Event; 3], body: &[u8]) {
    let [first, rival, third] = events;
    let who = *first.header.verifying_key.as_bytes();
    let pos = position_key(&who, 0);
    store_event(&mut rows, first).unwrap();
    assert!(matches!(
        store_event(&mut rows, rival),
        Err(StoreError::Occupied { seq: 0 })
    ));
    assert_eq!(rows.get("events", &pos).unwrap(), Some(first.bytes.clone()));
    assert_eq!(
        rows.get("hashes", first.id.as_bytes()).unwrap(),
        Some(pos.to_vec())
    );
    assert_eq!(rows.get("hashes", rival.id.as_bytes()).unwrap(), None);
    store_body(&mut rows, &first.id, body).unwrap();
    assert_eq!(
        rows.get("bodies", first.id.as_bytes()).unwrap(),
        Some(body.to_vec())
    );
    for chain in [
        Chain::default(),
        Chain {
            next_seq: 1,
            head: Some(first.id),
            frozen: true,
        },
    ] {
        store_chain(&mut rows, &who, chain).unwrap();
        assert_eq!(
            Chain::decode(&rows.get("chains", &who).unwrap().unwrap()).unwrap(),
            chain
        );
    }
    store_fork(&mut rows, &who, 0, first, rival).unwrap();
    store_fork(&mut rows, &who, 0, first, third).unwrap();
    assert_eq!(
        rows.get("forks", &pos).unwrap(),
        Some(encode_proof(0, first, third).unwrap())
    );
    for header in events {
        assert_eq!(
            rows.get("proof_ids", header.id.as_bytes()).unwrap(),
            Some(who.to_vec())
        );
        assert_eq!(
            rows.get("proof_headers", header.id.as_bytes()).unwrap(),
            Some(header.bytes.clone())
        );
    }
}

#[test]
fn test_native_redb_oracle_shared_persistence_helpers() {
    compare_helpers(b"retained verified body", Hash::digest(b"original-group"));
}

/// Native recovery policy is the oracle: recover only verified retained
/// bodies, skip existing rows, report missing bodies, and mark the pass once.
/// The 258-event history crosses the shared recovery page boundary.
#[test]
fn test_native_redb_oracle_paged_device_recovery() {
    use crate::groups::{Body, DeviceOp, Event as GroupEvent, Role};

    let signer = SigningKey::from_bytes(&[73; 32]);
    let group = Hash::digest(b"recovery-group");
    let body = crate::groups::event::encode(&GroupEvent {
        id: Hash::digest(b"grant"),
        author: *signer.verifying_key().as_bytes(),
        after: vec![],
        group: Some(group),
        at: None,
        body: Body::Device(DeviceOp::Add {
            device: [91; 32],
            scope: Role::Admin,
        }),
    })
    .unwrap();
    let ext = event("group.event", Some(group)).header.extensions;
    let mut events = Vec::new();
    for seq in 0..259 {
        let backlink = events.last().map(|event: &Event| event.id);
        events.push(sign(&signer, seq, backlink, Some(&body), ext.clone()).unwrap());
    }
    let expected_op = resolve::encode_device_op(
        &DeviceOp::Add {
            device: [91; 32],
            scope: Role::Admin,
        },
        Some(&group),
    )
    .unwrap();
    let existing_op = resolve::encode_device_op(
        &DeviceOp::Add {
            device: [92; 32],
            scope: Role::Admin,
        },
        Some(&group),
    )
    .unwrap();
    let script = |mut rows: &mut dyn RowWrite| {
        for (index, event) in events.iter().take(258).enumerate() {
            store_event(&mut *rows, event).unwrap();
            if index != 1 && index != 257 {
                store_body(&mut *rows, &event.id, &body).unwrap();
            }
        }
        rows.put("device_ops", events[0].id.as_bytes(), &existing_op)
            .unwrap();
        store_body(&mut *rows, &events[2].id, b"invalid commitment").unwrap();
        let mut pool = id_core::waiting::WaitingPool::new(WaitingLimits::default());
        let mut arrivals = 0;
        let held = events[258].clone();
        assert_eq!(
            waiting_rows::Mirror {
                txn: &mut rows,
                pool: &mut pool,
                arrivals: &mut arrivals,
            }
            .hold(held.clone(), vec![events[257].id], false, 0)
            .unwrap(),
            waiting_rows::Hold::Held
        );
        store_body(&mut *rows, &held.id, &body).unwrap();
        assert_eq!(
            ingest::recover_device_ops_rows(&mut rows).unwrap(),
            (255, true)
        );
        let recovered = rows.scan_all("device_ops").unwrap();
        assert_eq!(recovered.len(), 256);
        assert_eq!(
            rows.get("device_ops", events[0].id.as_bytes()).unwrap(),
            Some(existing_op.clone())
        );
        assert_eq!(
            rows.get("device_ops", events[2].id.as_bytes()).unwrap(),
            None
        );
        assert_eq!(
            rows.get("device_ops", held.id.as_bytes()).unwrap(),
            Some(expected_op.clone())
        );
        assert!(recovered.iter().all(|(key, value)| value
            == if key.as_slice() == events[0].id.as_bytes() {
                &existing_op
            } else {
                &expected_op
            }));
        store_body(&mut *rows, &events[1].id, &body).unwrap();
        assert_eq!(
            ingest::recover_device_ops_rows(&mut rows).unwrap(),
            (0, false)
        );
        assert_eq!(
            rows.get("device_ops", events[1].id.as_bytes()).unwrap(),
            None,
            "the completed recovery marker suppresses later passes"
        );
    };
    let dir = tempfile::tempdir().unwrap();
    let native = Database::create(dir.path().join("recovery.redb")).unwrap();
    let mut tenant = TenantStore::open(SqliteEndpoint::new()).unwrap();
    let txn = native.begin_write().unwrap();
    script(&mut &txn);
    txn.commit().unwrap();
    let mut txn = TenantWrite::begin(&mut tenant).unwrap();
    script(&mut &mut txn);
    txn.commit().unwrap();

    let mut native = native.begin_read().unwrap();
    let mut tenant = TenantWrite::begin(&mut tenant).unwrap();
    for table in ["device_ops", "meta"] {
        assert_eq!(
            RowRead::scan_all(&mut native, table).unwrap(),
            RowRead::scan_all(&mut &mut tenant, table).unwrap(),
            "native-redb committed recovery oracle: {table}",
        );
    }
}

fn compare_helpers(body: &[u8], group: Hash) {
    let dir = tempfile::tempdir().unwrap();
    let native = Database::create(dir.path().join("oracle.redb")).unwrap();
    let mut tenant = TenantStore::open(SqliteEndpoint::new()).unwrap();
    let rejected_group = Hash::digest(b"rejected-group");
    let events = [
        event("message", Some(group)),
        event("rival", Some(rejected_group)),
        event("third", None),
    ];
    let txn = native.begin_write().unwrap();
    script(&txn, &events, body);
    txn.commit().unwrap();
    let mut txn = TenantWrite::begin(&mut tenant).unwrap();
    script(&mut txn, &events, body);
    txn.commit().unwrap();

    // Compare committed rows, not just reads within the transaction. This also
    // ensures the rejected position never polluted the group multimap.
    let read = native.begin_read().unwrap();
    for name in [
        "events",
        "hashes",
        "bodies",
        "chains",
        "forks",
        "proof_ids",
        "proof_headers",
    ] {
        let table = read
            .open_table(TableDefinition::<&[u8], &[u8]>::new(name))
            .unwrap();
        let expected: Vec<_> = table
            .iter()
            .unwrap()
            .map(|row| {
                let (k, v) = row.unwrap();
                (k.value().to_vec(), v.value().to_vec())
            })
            .collect();
        assert_eq!(
            tenant.scan(name, .., usize::MAX).unwrap(),
            expected,
            "native-redb oracle: {name}"
        );
    }
    let groups = read.open_multimap_table(GROUPS).unwrap();
    for group in [group, rejected_group] {
        let expected: Vec<_> = groups
            .get(group.as_bytes().as_slice())
            .unwrap()
            .map(|v| v.unwrap().value().to_vec())
            .collect();
        assert_eq!(
            tenant.multi_values("groups", group.as_bytes()).unwrap(),
            expected
        );
    }
}

/// The shared transaction runner is the oracle for commit, no-op abort, and
/// error rollback across native redb and tenant SQLite transactions.
#[test]
fn test_shared_write_transaction_lifecycle_matches_native_and_tenant() {
    let dir = tempfile::tempdir().unwrap();
    let native = Database::create(dir.path().join("transaction-driver.redb")).unwrap();
    let mut tenant = TenantStore::open(SqliteEndpoint::new()).unwrap();

    let native_txn = native.begin_write().unwrap();
    with_write_transaction(native_txn, |rows| {
        rows.put("meta", b"committed", b"native")?;
        Ok(((), true))
    })
    .unwrap();
    let tenant_txn = TenantWrite::begin(&mut tenant).unwrap();
    with_write_transaction(tenant_txn, |rows| {
        rows.put("meta", b"committed", b"tenant")?;
        Ok(((), true))
    })
    .unwrap();

    let native_txn = native.begin_write().unwrap();
    with_write_transaction(native_txn, |rows| {
        rows.put("meta", b"aborted", b"native")?;
        Ok(((), false))
    })
    .unwrap();
    let tenant_txn = TenantWrite::begin(&mut tenant).unwrap();
    with_write_transaction(tenant_txn, |rows| {
        rows.put("meta", b"aborted", b"tenant")?;
        Ok(((), false))
    })
    .unwrap();

    let native_txn = native.begin_write().unwrap();
    let native_error = with_write_transaction(native_txn, |rows| {
        rows.put("meta", b"failed", b"native")?;
        Err::<((), bool), _>(StoreError::Corrupt("injected operation failure".to_owned()))
    })
    .unwrap_err();
    assert!(matches!(native_error, StoreError::Corrupt(_)));
    let tenant_txn = TenantWrite::begin(&mut tenant).unwrap();
    let tenant_error = with_write_transaction(tenant_txn, |rows| {
        rows.put("meta", b"failed", b"tenant")?;
        Err::<((), bool), _>(StoreError::Corrupt("injected operation failure".to_owned()))
    })
    .unwrap_err();
    assert!(matches!(tenant_error, StoreError::Corrupt(_)));

    let native_read = native.begin_read().unwrap();
    let native_rows = native_read
        .open_table(TableDefinition::<&[u8], &[u8]>::new("meta"))
        .unwrap();
    assert_eq!(
        native_rows
            .get(b"committed".as_slice())
            .unwrap()
            .unwrap()
            .value(),
        b"native"
    );
    assert!(native_rows.get(b"aborted".as_slice()).unwrap().is_none());
    assert!(native_rows.get(b"failed".as_slice()).unwrap().is_none());
    assert_eq!(
        tenant.get("meta", b"committed").unwrap(),
        Some(b"tenant".to_vec())
    );
    assert_eq!(tenant.get("meta", b"aborted").unwrap(), None);
    assert_eq!(tenant.get("meta", b"failed").unwrap(), None);
}

proptest::proptest! {
    #![proptest_config(proptest::test_runner::Config::with_cases(16))]
    /// Native-redb oracle: arbitrary retained bytes and group keys preserve the
    /// same indexes, occupied-position behavior and fork/header retention.
    #[test]
    fn test_native_redb_oracle_generated_rows(
        body in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..512),
        group in proptest::array::uniform32(proptest::prelude::any::<u8>()),
    ) {
        compare_helpers(&body, Hash::digest(group));
    }
}

#[derive(Debug)]
struct Offline;

impl PipelineTransport for Offline {
    fn pipeline(&self, _body: &[u8]) -> Result<Vec<u8>, TursoError> {
        Err(TursoError::Transport("offline".to_owned()))
    }
}

/// The tenant's original typed error must remain a storage error, never an
/// event refusal or corruption. Opening a transaction exercises the adapter's
/// From<TursoError> path without manufacturing a redb error.
#[test]
fn test_native_redb_oracle_tenant_transport_error_is_typed() {
    // TenantStore::open fails before a transaction exists on an offline peer.
    let error: StoreError = TenantStore::open(Offline).unwrap_err().into();
    assert!(
        matches!(error, StoreError::Tenant(TursoError::Transport(message)) if message == "offline")
    );
}

/// Lose the COMMIT response either before or after SQLite applies it, then
/// refuse ROLLBACK. The guard must retire this uncertain session in both cases.
#[derive(Debug)]
struct LostCommit {
    endpoint: SqliteEndpoint,
    applied: bool,
    rollback_fails: bool,
}

impl PipelineTransport for LostCommit {
    fn pipeline(&self, body: &[u8]) -> Result<Vec<u8>, TursoError> {
        let request: Json = serde_json::from_slice(body).unwrap();
        let sql = request["requests"][0]["stmt"]["sql"].as_str();
        if sql == Some("COMMIT") {
            if self.applied {
                self.endpoint.pipeline(body)?;
            }
            return Err(TursoError::Transport("lost COMMIT response".to_owned()));
        }
        if sql == Some("ROLLBACK") && self.rollback_fails {
            return Err(TursoError::Transport("lost ROLLBACK response".to_owned()));
        }
        self.endpoint.pipeline(body)
    }
}

/// Oracle: a lost commit response cannot tell the caller whether rows became
/// durable. In either outcome, an unconfirmed rollback must close the baton;
/// a fresh connection observes exactly SQLite's committed/aborted state.
#[test]
fn test_native_redb_oracle_uncertain_commit_retires_baton() {
    for applied in [false, true] {
        let endpoint = SqliteEndpoint::new();
        let mut tenant = TenantStore::open(LostCommit {
            endpoint: endpoint.clone(),
            applied,
            rollback_fails: true,
        })
        .unwrap();
        let mut txn = TenantWrite::begin(&mut tenant).unwrap();
        (&mut txn).put("bodies", b"key", b"value").unwrap();
        assert!(matches!(
            txn.commit(),
            Err(StoreError::Tenant(TursoError::Transport(message)))
                if message == "lost COMMIT response"
        ));
        assert!(matches!(
            tenant.get("bodies", b"key"),
            Err(TursoError::Closed)
        ));
        let mut fresh = TenantStore::open(endpoint).unwrap();
        assert_eq!(
            fresh.get("bodies", b"key").unwrap(),
            applied.then(|| b"value".to_vec())
        );
    }
}

/// Oracle: a commit refused before execution leaves no rows when the rollback
/// is confirmed, and this confirmed-clean session remains usable.
#[test]
fn test_native_redb_oracle_failed_commit_confirmed_rollback() {
    let mut tenant = TenantStore::open(LostCommit {
        endpoint: SqliteEndpoint::new(),
        applied: false,
        rollback_fails: false,
    })
    .unwrap();
    let mut txn = TenantWrite::begin(&mut tenant).unwrap();
    (&mut txn).put("bodies", b"key", b"value").unwrap();
    assert!(matches!(
        txn.commit(),
        Err(StoreError::Tenant(TursoError::Transport(_)))
    ));
    assert_eq!(tenant.get("bodies", b"key").unwrap(), None);
}

#[derive(Debug)]
struct LostBegin(SqliteEndpoint);

impl PipelineTransport for LostBegin {
    fn pipeline(&self, body: &[u8]) -> Result<Vec<u8>, TursoError> {
        let response = self.0.pipeline(body)?;
        let request: Json = serde_json::from_slice(body).unwrap();
        if request["requests"][0]["stmt"]["sql"] == "BEGIN" {
            return Err(TursoError::Transport("lost BEGIN response".to_owned()));
        }
        Ok(response)
    }
}

/// Oracle: BEGIN may have executed even when its response is lost. Retire that
/// session rather than allowing later writes to enter an untracked transaction.
#[test]
fn test_native_redb_oracle_uncertain_begin_retires_baton() {
    let mut tenant = TenantStore::open(LostBegin(SqliteEndpoint::new())).unwrap();
    assert!(matches!(
        TenantWrite::begin(&mut tenant),
        Err(StoreError::Tenant(TursoError::Transport(_)))
    ));
    assert!(matches!(
        tenant.get("bodies", b"key"),
        Err(TursoError::Closed)
    ));
}

/// Oracle: nested BEGIN is a local refusal, not uncertainty; the existing
/// transaction remains available to its original owner for rollback.
#[test]
fn test_native_redb_oracle_nested_begin_preserves_transaction() {
    let mut tenant = TenantStore::open(SqliteEndpoint::new()).unwrap();
    tenant.begin().unwrap();
    tenant.put("bodies", b"key", b"value").unwrap();
    assert!(matches!(
        TenantWrite::begin(&mut tenant),
        Err(StoreError::Tenant(TursoError::Transaction(_)))
    ));
    assert_eq!(
        tenant.get("bodies", b"key").unwrap(),
        Some(b"value".to_vec())
    );
    tenant.rollback().unwrap();
    assert_eq!(tenant.get("bodies", b"key").unwrap(), None);
}

/// Inject failure after event and hash rows have been written, before group
/// indexing. The enclosing transaction must leave no rows from any helper.
struct FailMulti<R>(R);
impl<R: RowRead> RowRead for FailMulti<R> {
    fn get(&mut self, table: &'static str, key: &[u8]) -> Result<Option<Vec<u8>>, StoreError> {
        self.0.get(table, key)
    }
    fn multi_values(
        &mut self,
        table: &'static str,
        key: &[u8],
    ) -> Result<Vec<Vec<u8>>, StoreError> {
        self.0.multi_values(table, key)
    }
    fn scan_page(
        &mut self,
        table: &'static str,
        after_key: Option<&[u8]>,
        limit: usize,
    ) -> Result<Vec<(Vec<u8>, Vec<u8>)>, StoreError> {
        self.0.scan_page(table, after_key, limit)
    }
}
impl<R: RowWrite> RowWrite for FailMulti<R> {
    fn remove(&mut self, table: &'static str, key: &[u8]) -> Result<(), StoreError> {
        self.0.remove(table, key)
    }
    fn multi_remove(
        &mut self,
        table: &'static str,
        key: &[u8],
        value: &[u8],
    ) -> Result<(), StoreError> {
        self.0.multi_remove(table, key, value)
    }
    fn put(&mut self, table: &'static str, key: &[u8], value: &[u8]) -> Result<(), StoreError> {
        self.0.put(table, key, value)
    }
    fn multi_put(
        &mut self,
        _table: &'static str,
        _key: &[u8],
        _value: &[u8],
    ) -> Result<(), StoreError> {
        Err(TursoError::Transport("injected after hash write".to_owned()).into())
    }
}

#[test]
fn test_native_redb_oracle_partial_helper_rollback() {
    let dir = tempfile::tempdir().unwrap();
    let native = Database::create(dir.path().join("rollback.redb")).unwrap();
    let mut tenant = TenantStore::open(SqliteEndpoint::new()).unwrap();
    let event = event("message", Some(Hash::digest(b"group")));
    let who = *event.header.verifying_key.as_bytes();
    let txn = native.begin_write().unwrap();
    store_body(&txn, &event.id, b"body").unwrap();
    store_chain(&txn, &who, Chain::default()).unwrap();
    assert!(matches!(
        store_event(FailMulti(&txn), &event),
        Err(StoreError::Tenant(TursoError::Transport(_)))
    ));
    drop(txn);
    {
        let mut txn = TenantWrite::begin(&mut tenant).unwrap();
        store_body(&mut txn, &event.id, b"body").unwrap();
        store_chain(&mut txn, &who, Chain::default()).unwrap();
        assert!(matches!(
            store_event(FailMulti(&mut txn), &event),
            Err(StoreError::Tenant(TursoError::Transport(_)))
        ));
    }
    // redb abort may remove newly-created tables entirely; read through a new
    // write handle to create empty tables, exactly as the helper adapter does.
    let txn = native.begin_write().unwrap();
    let mut rows = &txn;
    for (table, key) in [
        ("events", position_key(&who, 0).to_vec()),
        ("hashes", event.id.as_bytes().to_vec()),
        ("bodies", event.id.as_bytes().to_vec()),
        ("chains", who.to_vec()),
    ] {
        assert_eq!(rows.get(table, &key).unwrap(), None);
        assert_eq!(tenant.get(table, &key).unwrap(), None);
    }
    assert!(
        tenant
            .multi_values("groups", event.header.extensions.group.unwrap().as_bytes())
            .unwrap()
            .is_empty()
    );
}

/// Fail after the replacement proof and its first proof-header have been put.
struct FailFourthPut<R> {
    rows: R,
    puts: usize,
}

impl<R: RowRead> RowRead for FailFourthPut<R> {
    fn get(&mut self, table: &'static str, key: &[u8]) -> Result<Option<Vec<u8>>, StoreError> {
        self.rows.get(table, key)
    }
    fn multi_values(
        &mut self,
        table: &'static str,
        key: &[u8],
    ) -> Result<Vec<Vec<u8>>, StoreError> {
        self.rows.multi_values(table, key)
    }
    fn scan_page(
        &mut self,
        table: &'static str,
        after_key: Option<&[u8]>,
        limit: usize,
    ) -> Result<Vec<(Vec<u8>, Vec<u8>)>, StoreError> {
        self.rows.scan_page(table, after_key, limit)
    }
}
impl<R: RowWrite> RowWrite for FailFourthPut<R> {
    fn remove(&mut self, table: &'static str, key: &[u8]) -> Result<(), StoreError> {
        self.rows.remove(table, key)
    }
    fn multi_remove(
        &mut self,
        table: &'static str,
        key: &[u8],
        value: &[u8],
    ) -> Result<(), StoreError> {
        self.rows.multi_remove(table, key, value)
    }
    fn put(&mut self, table: &'static str, key: &[u8], value: &[u8]) -> Result<(), StoreError> {
        self.puts += 1;
        if self.puts == 4 {
            return Err(TursoError::Transport("injected after proof header".to_owned()).into());
        }
        self.rows.put(table, key, value)
    }
    fn multi_put(
        &mut self,
        table: &'static str,
        key: &[u8],
        value: &[u8],
    ) -> Result<(), StoreError> {
        self.rows.multi_put(table, key, value)
    }
}

fn check_retained_fork(mut rows: impl RowWrite, events: &[Event; 3]) {
    let [first, rival, third] = events;
    let who = *first.header.verifying_key.as_bytes();
    assert_eq!(
        rows.get("forks", &position_key(&who, 0)).unwrap(),
        Some(encode_proof(0, first, rival).unwrap())
    );
    for header in [first, rival] {
        assert_eq!(
            rows.get("proof_headers", header.id.as_bytes()).unwrap(),
            Some(header.bytes.clone())
        );
        assert_eq!(
            rows.get("proof_ids", header.id.as_bytes()).unwrap(),
            Some(who.to_vec())
        );
    }
    assert_eq!(
        rows.get("proof_headers", third.id.as_bytes()).unwrap(),
        None
    );
    assert_eq!(rows.get("proof_ids", third.id.as_bytes()).unwrap(), None);
}

/// Native-redb oracle: aborting a partial fork replacement preserves the old
/// committed proof and headers, without retaining either new proof-header row.
#[test]
fn test_native_redb_oracle_partial_fork_preserves_committed_rows() {
    let dir = tempfile::tempdir().unwrap();
    let native = Database::create(dir.path().join("fork-rollback.redb")).unwrap();
    let mut tenant = TenantStore::open(SqliteEndpoint::new()).unwrap();
    let events = [
        event("first", None),
        event("rival", None),
        event("third", None),
    ];
    let [first, rival, third] = &events;
    let who = *first.header.verifying_key.as_bytes();
    let txn = native.begin_write().unwrap();
    store_fork(&txn, &who, 0, first, rival).unwrap();
    txn.commit().unwrap();
    let mut txn = TenantWrite::begin(&mut tenant).unwrap();
    store_fork(&mut txn, &who, 0, first, rival).unwrap();
    txn.commit().unwrap();

    let txn = native.begin_write().unwrap();
    assert!(
        store_fork(
            FailFourthPut {
                rows: &txn,
                puts: 0
            },
            &who,
            0,
            third,
            first
        )
        .is_err()
    );
    drop(txn);
    {
        let mut txn = TenantWrite::begin(&mut tenant).unwrap();
        assert!(
            store_fork(
                FailFourthPut {
                    rows: &mut txn,
                    puts: 0
                },
                &who,
                0,
                third,
                first
            )
            .is_err()
        );
    }
    let txn = native.begin_write().unwrap();
    check_retained_fork(&txn, &events);
    let mut txn = TenantWrite::begin(&mut tenant).unwrap();
    check_retained_fork(&mut txn, &events);
}
