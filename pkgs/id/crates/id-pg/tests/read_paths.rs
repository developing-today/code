//! Live and materialized read paths (design #5805, §5.2–§5.4, §9
//! invariants 7, 8 and 10; slice #13763), over an injectable transport.
//!
//! The done check of #13763: live responses state `per_request`
//! consistency, snapshot caps are enforced before copy, and an interrupted
//! build keeps the prior snapshot.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unnecessary_wraps
)]

use std::collections::VecDeque;

use id_pg::{
    Audience, CollectionAllowlist, ColumnDecl, Consistency, ContentRange, DecodeError,
    EndpointConfig, EndpointProbe, Failure, HttpResponse, ModeChoices, ObjectKind, OrderKey,
    PageDecoder, Pagination, PostgrestRequest, Privilege, Query, ReadError, ReadMode, Refusal,
    RegisteredEndpoint, RoleGrant, SelectItem, SnapshotCaps, SnapshotStore, Transport,
    TransportError, Value, live_read, register,
};

type Row = serde_json::Map<String, serde_json::Value>;

fn endpoint() -> RegisteredEndpoint {
    let collection = |relation: &str| CollectionAllowlist {
        schema: "api".to_owned(),
        relation: relation.to_owned(),
        columns: vec![
            ColumnDecl {
                name: "id".to_owned(),
                filterable: true,
                orderable: true,
            },
            ColumnDecl {
                name: "title".to_owned(),
                filterable: true,
                orderable: false,
            },
        ],
        unique_key: vec!["id".to_owned()],
        max_limit: Some(2),
    };
    let config = EndpointConfig {
        alias: "books-db".to_owned(),
        base_url: "https://pg.example.org/rest".to_owned(),
        schema: "api".to_owned(),
        credential_ref: "secret://pg-books-reader".to_owned(),
        collections: vec![collection("items"), collection("authors")],
    };
    let probe = EndpointProbe {
        db_schemas: vec!["api".to_owned()],
        role_grants: vec![
            RoleGrant {
                kind: ObjectKind::Schema,
                object: "api".to_owned(),
                privilege: Privilege::Usage,
            },
            RoleGrant {
                kind: ObjectKind::Relation,
                object: "items".to_owned(),
                privilege: Privilege::Select,
            },
        ],
    };
    register(config, &probe).unwrap()
}

fn modes(mode: ReadMode) -> ModeChoices {
    let mut modes = ModeChoices::new();
    modes.choose("items", mode);
    modes
}

/// A scripted transport that records every request it was asked to send.
#[derive(Default)]
struct Scripted {
    responses: VecDeque<Result<HttpResponse, TransportError>>,
    sent: Vec<PostgrestRequest>,
}

impl Scripted {
    fn new(responses: Vec<Result<HttpResponse, TransportError>>) -> Self {
        Self {
            responses: responses.into(),
            sent: Vec::new(),
        }
    }
}

impl Transport for Scripted {
    fn get(
        &mut self,
        endpoint: &RegisteredEndpoint,
        request: &PostgrestRequest,
    ) -> Result<HttpResponse, TransportError> {
        assert_eq!(request.method, "GET");
        assert_eq!(request.accept_profile, endpoint.schema());
        self.sent.push(request.clone());
        self.responses
            .pop_front()
            .expect("the read sent more requests than the script has")
    }
}

/// Decodes a JSON array of objects; the cursor reads integer columns.
struct JsonRows;

impl PageDecoder for JsonRows {
    type Row = Row;

    fn decode(&self, body: &[u8]) -> Result<Vec<Row>, DecodeError> {
        let value: serde_json::Value = serde_json::from_slice(body)
            .ok()
            .ok_or(DecodeError::NotJson)?;
        let serde_json::Value::Array(items) = value else {
            return Err(DecodeError::NotJson);
        };
        items
            .into_iter()
            .map(|item| match item {
                serde_json::Value::Object(row) => Ok(row),
                _ => Err(DecodeError::NotJson),
            })
            .collect()
    }

    fn cursor(&self, row: &Row, ordering: &[OrderKey]) -> Result<Vec<Value>, DecodeError> {
        ordering
            .iter()
            .map(|key| {
                row.get(&key.column)
                    .and_then(serde_json::Value::as_i64)
                    .map(Value::Int)
                    .ok_or_else(|| DecodeError::TypeMapping {
                        column: key.column.clone(),
                        declared_type: "bigint".to_owned(),
                    })
            })
            .collect()
    }

    fn drop_columns(&self, row: &mut Row, columns: &[String]) {
        for column in columns {
            row.remove(column);
        }
    }
}

fn page(ids: &[i64], range: Option<&str>, at: u64) -> Result<HttpResponse, TransportError> {
    let rows: Vec<serde_json::Value> = ids
        .iter()
        .map(|id| serde_json::json!({"id": id, "title": format!("t{id}")}))
        .collect();
    Ok(HttpResponse {
        status: 200,
        content_type: Some("application/json; charset=utf-8".to_owned()),
        content_range: range.map(str::to_owned),
        body: serde_json::to_vec(&rows).unwrap(),
        observed_at_ms: at,
    })
}

fn status(code: u16) -> Result<HttpResponse, TransportError> {
    Ok(HttpResponse {
        status: code,
        content_type: Some("application/json".to_owned()),
        content_range: None,
        body: b"{\"message\":\"nope\"}".to_vec(),
        observed_at_ms: 1,
    })
}

fn ids(rows: &[Row]) -> Vec<i64> {
    rows.iter().map(|row| row["id"].as_i64().unwrap()).collect()
}

fn param<'r>(request: &'r PostgrestRequest, name: &str) -> Option<&'r str> {
    request
        .params
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.as_str())
}

// ---- live reads (§5.2) ----

#[test]
fn live_short_page_is_complete_and_states_per_request_consistency() {
    let mut transport = Scripted::new(vec![page(&[1], Some("0-0/*"), 1_000)]);
    let result = live_read(
        &endpoint(),
        &modes(ReadMode::Live),
        &mut transport,
        &JsonRows,
        &Query::read("items"),
        10,
    )
    .unwrap();
    assert_eq!(ids(&result.rows), vec![1]);
    assert_eq!(result.consistency, Consistency::PerRequest);
    assert_eq!(result.consistency.name(), "per_request");
    assert_eq!(result.observed_at_ms, 1_000);
    assert!(result.complete);
    assert!(!result.truncated);
    assert_eq!(result.next_cursor, None);
    assert_eq!(transport.sent.len(), 1);
    assert_eq!(transport.sent[0].path, "/items");
}

#[test]
fn live_short_page_with_explicit_incomplete_range_continues_keyset_read() {
    // Oracle: #15704 — a short body must not override an explicit
    // Content-Range total that says more rows remain.
    let mut transport = Scripted::new(vec![
        page(&[1], Some("0-0/100"), 1_000),
        page(&[2], Some("1-1/2"), 2_000),
    ]);
    let result = live_read(
        &endpoint(),
        &modes(ReadMode::Live),
        &mut transport,
        &JsonRows,
        &Query::read("items"),
        2,
    )
    .unwrap();

    assert_eq!(ids(&result.rows), vec![1, 2]);
    assert!(result.complete);
    assert!(!result.truncated);
    assert_eq!(result.pages, 2);
    assert_eq!(result.next_cursor, None);
    assert_eq!(transport.sent.len(), 2);
    assert_eq!(param(&transport.sent[1], "or"), Some("(id.gt.1)"));
}

#[test]
fn live_full_page_without_a_total_is_truncated_with_a_cursor() {
    let mut transport = Scripted::new(vec![page(&[1, 2], Some("0-1/*"), 5)]);
    let result = live_read(
        &endpoint(),
        &modes(ReadMode::Live),
        &mut transport,
        &JsonRows,
        &Query::read("items"),
        1,
    )
    .unwrap();
    assert!(!result.complete);
    assert!(result.truncated);
    assert_eq!(result.next_cursor, Some(vec![Value::Int(2)]));
    assert_eq!(result.pages, 1);
    assert_eq!(transport.sent.len(), 1);
}

#[test]
fn live_content_range_total_marks_a_full_page_complete() {
    let mut transport = Scripted::new(vec![page(&[1, 2], Some("0-1/2"), 5)]);
    let result = live_read(
        &endpoint(),
        &modes(ReadMode::Live),
        &mut transport,
        &JsonRows,
        &Query::read("items"),
        5,
    )
    .unwrap();
    assert!(result.complete);
    assert!(!result.truncated);
    assert_eq!(transport.sent.len(), 1);
}

#[test]
fn live_rejects_content_range_inconsistent_with_decoded_rows_or_total() {
    let run = |ids: &[i64], range: &str| {
        let mut transport = Scripted::new(vec![page(ids, Some(range), 1)]);
        live_read(
            &endpoint(),
            &modes(ReadMode::Live),
            &mut transport,
            &JsonRows,
            &Query::read("items"),
            5,
        )
    };

    // Oracle: inclusive Content-Range interval length must equal the
    // number of decoded rows; any declared total must contain the range.
    assert_eq!(
        run(&[1], "0-1/2").unwrap_err(),
        ReadError::MalformedPage,
        "a range claiming two rows cannot accompany one decoded row"
    );
    assert_eq!(
        run(&[1, 2], "0-1/1").unwrap_err(),
        ReadError::MalformedPage,
        "the declared total cannot end before the last row index"
    );
    assert_eq!(
        run(&[1], "*/0").unwrap_err(),
        ReadError::MalformedPage,
        "an empty range cannot accompany decoded rows"
    );
    assert_eq!(
        run(&[], "0-0/1").unwrap_err(),
        ReadError::MalformedPage,
        "a row range cannot accompany an empty body"
    );

    let empty = run(&[], "*/0").unwrap();
    assert!(empty.complete);
    assert!(empty.rows.is_empty());
}

#[test]
fn live_follows_the_keyset_cursor_one_request_per_page() {
    let mut transport = Scripted::new(vec![
        page(&[1, 2], Some("0-1/*"), 10),
        page(&[3], Some("0-0/*"), 20),
    ]);
    let result = live_read(
        &endpoint(),
        &modes(ReadMode::Live),
        &mut transport,
        &JsonRows,
        &Query::read("items"),
        5,
    )
    .unwrap();
    assert_eq!(ids(&result.rows), vec![1, 2, 3]);
    assert_eq!(result.pages, 2);
    assert!(result.complete);
    // The observation time is the last page's.
    assert_eq!(result.observed_at_ms, 20);
    assert_eq!(param(&transport.sent[0], "or"), None);
    assert_eq!(param(&transport.sent[1], "or"), Some("(id.gt.2)"));
}

/// The query of #15010: project `title` alone from a collection whose
/// unique key is `id`.
fn title_only() -> Query {
    let mut query = Query::read("items");
    query.select = vec![SelectItem::Column("title".to_owned())];
    query
}

fn titles(rows: &[Row]) -> Vec<String> {
    rows.iter()
        .map(|row| row["title"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn projection_without_the_unique_key_reads_a_full_page_and_hides_the_key() {
    // Oracle: #15010 — a full page used to fail as a malformed page
    // because the cursor could not find `id` in the rows. The key is
    // now fetched for the cursor and removed from the returned rows.
    let mut transport = Scripted::new(vec![page(&[1, 2], Some("0-1/*"), 5)]);
    let result = live_read(
        &endpoint(),
        &modes(ReadMode::Live),
        &mut transport,
        &JsonRows,
        &title_only(),
        1,
    )
    .unwrap();
    assert_eq!(param(&transport.sent[0], "select"), Some("title,id"));
    assert_eq!(titles(&result.rows), vec!["t1", "t2"]);
    assert!(result.rows.iter().all(|row| !row.contains_key("id")));
    assert!(result.truncated);
    assert_eq!(result.next_cursor, Some(vec![Value::Int(2)]));
}

#[test]
fn projection_without_the_unique_key_continues_across_pages() {
    let mut transport = Scripted::new(vec![
        page(&[1, 2], Some("0-1/*"), 10),
        page(&[3], Some("0-0/*"), 20),
    ]);
    let result = live_read(
        &endpoint(),
        &modes(ReadMode::Live),
        &mut transport,
        &JsonRows,
        &title_only(),
        5,
    )
    .unwrap();
    assert_eq!(titles(&result.rows), vec!["t1", "t2", "t3"]);
    assert!(result.rows.iter().all(|row| row.len() == 1));
    assert!(result.complete);
    assert_eq!(result.pages, 2);
    // The second page continues after the first page's last key and
    // still fetches the key for its own cursor.
    assert_eq!(param(&transport.sent[1], "or"), Some("(id.gt.2)"));
    assert_eq!(param(&transport.sent[1], "select"), Some("title,id"));
}

#[test]
fn live_failed_later_page_is_an_error_not_a_short_success() {
    let mut transport = Scripted::new(vec![page(&[1, 2], Some("0-1/*"), 10), status(503)]);
    let err = live_read(
        &endpoint(),
        &modes(ReadMode::Live),
        &mut transport,
        &JsonRows,
        &Query::read("items"),
        5,
    )
    .unwrap_err();
    assert_eq!(err, ReadError::Status(503));
    assert_eq!(err.code(), "adapter_unavailable");
}

#[test]
fn read_errors_collapse_for_a_grantee_through_the_error_mapping() {
    let grantee = |err: ReadError| err.failure().unwrap().to_error(Audience::Grantee).body();
    let not_available = grantee(ReadError::Refused(Refusal::not_available()));
    assert_eq!(grantee(ReadError::Status(401)), not_available);
    assert_eq!(grantee(ReadError::Status(404)), not_available);
    assert_eq!(grantee(ReadError::ModeNotChosen), not_available);
    assert_eq!(
        grantee(ReadError::Deadline),
        Failure::Deadline.to_error(Audience::Grantee).body()
    );
    assert_eq!(ReadError::SnapshotTooLarge.failure(), None);
}

#[test]
fn live_transport_and_body_failures_are_errors() {
    let run = |response: Result<HttpResponse, TransportError>| {
        let mut transport = Scripted::new(vec![response]);
        live_read(
            &endpoint(),
            &modes(ReadMode::Live),
            &mut transport,
            &JsonRows,
            &Query::read("items"),
            5,
        )
        .unwrap_err()
    };
    let deadline = run(Err(TransportError::Deadline));
    assert_eq!(deadline, ReadError::Deadline);
    assert_eq!(deadline.code(), "timeout");
    assert_eq!(
        run(Err(TransportError::Connection)).code(),
        "adapter_unavailable"
    );

    let mut html = page(&[], None, 1).unwrap();
    html.content_type = Some("text/html".to_owned());
    assert_eq!(run(Ok(html)), ReadError::NonJsonBody);

    let mut garbage = page(&[], None, 1).unwrap();
    garbage.body = b"<html>".to_vec();
    assert_eq!(run(Ok(garbage)), ReadError::NonJsonBody);

    let mut missing_type = page(&[], None, 1).unwrap();
    missing_type.content_type = None;
    assert_eq!(run(Ok(missing_type)), ReadError::NonJsonBody);

    assert_eq!(run(status(401)).code(), "credential_unavailable");
    assert_eq!(run(status(403)).code(), "credential_unavailable");
    assert_eq!(run(status(404)).code(), "not_available");
    assert_eq!(run(status(429)).code(), "rate_limited");
    assert_eq!(run(status(302)).code(), "adapter_unavailable");

    assert_eq!(
        run(page(&[1, 2, 3], None, 1)),
        ReadError::MalformedPage,
        "more rows than the limit"
    );
    assert_eq!(run(page(&[1], Some("bogus"), 1)), ReadError::MalformedPage);
}

#[test]
fn reads_wait_for_the_owner_to_choose_a_mode() {
    let mut transport = Scripted::new(vec![]);
    let err = live_read(
        &endpoint(),
        &ModeChoices::new(),
        &mut transport,
        &JsonRows,
        &Query::read("items"),
        5,
    )
    .unwrap_err();
    assert_eq!(err, ReadError::ModeNotChosen);
    assert_eq!(err.code(), "not_available");

    let err = live_read(
        &endpoint(),
        &modes(ReadMode::Materialized),
        &mut transport,
        &JsonRows,
        &Query::read("items"),
        5,
    )
    .unwrap_err();
    assert_eq!(
        err,
        ReadError::WrongMode {
            chosen: ReadMode::Materialized
        }
    );

    let mut store: SnapshotStore<Row> = SnapshotStore::new("items");
    let err = store
        .rebuild(
            &endpoint(),
            &modes(ReadMode::Live),
            &mut transport,
            &JsonRows,
            caps(10, 10_000),
        )
        .unwrap_err();
    assert_eq!(
        err,
        ReadError::WrongMode {
            chosen: ReadMode::Live
        }
    );
    assert!(transport.sent.is_empty(), "nothing is sent without a mode");
}

#[test]
fn refused_queries_send_nothing() {
    let mut transport = Scripted::new(vec![]);
    let mut query = Query::read("items");
    query.pagination = Pagination::Offset { offset: 10 };
    let err = live_read(
        &endpoint(),
        &modes(ReadMode::Live),
        &mut transport,
        &JsonRows,
        &query,
        5,
    )
    .unwrap_err();
    assert_eq!(err.code(), "query_shape_refused");

    let err = live_read(
        &endpoint(),
        &modes(ReadMode::Live),
        &mut transport,
        &JsonRows,
        &Query::read("secrets"),
        5,
    )
    .unwrap_err();
    assert_eq!(err.code(), "not_available");
    assert_eq!(transport.sent, [] as [id_pg::PostgrestRequest; 0]);
}

// ---- materialized snapshots (§5.3) ----

const fn caps(max_rows: u64, max_bytes: u64) -> SnapshotCaps {
    SnapshotCaps {
        max_rows,
        max_bytes,
    }
}

fn build(
    store: &mut SnapshotStore<Row>,
    responses: Vec<Result<HttpResponse, TransportError>>,
    limits: SnapshotCaps,
) -> Result<Vec<i64>, ReadError> {
    let mut transport = Scripted::new(responses);
    store
        .rebuild(
            &endpoint(),
            &modes(ReadMode::Materialized),
            &mut transport,
            &JsonRows,
            limits,
        )
        .map(|snapshot| ids(&snapshot.rows))
}

#[test]
fn snapshot_pages_the_whole_set_and_reports_per_request_consistency() {
    let mut store = SnapshotStore::new("items");
    let built = build(
        &mut store,
        vec![
            page(&[1, 2], Some("0-1/*"), 100),
            page(&[3, 4], Some("0-1/*"), 200),
            page(&[], Some("*/*"), 300),
        ],
        caps(10, 10_000),
    )
    .unwrap();
    assert_eq!(built, vec![1, 2, 3, 4]);
    let snapshot = store.current().unwrap();
    assert_eq!(snapshot.consistency, Consistency::PerRequest);
    assert_eq!(snapshot.first_observed_at_ms, 100);
    assert_eq!(snapshot.last_observed_at_ms, 300);
    assert_eq!(snapshot.pages, 3);
    assert!(snapshot.bytes > 0);
}

#[test]
fn snapshot_row_cap_is_checked_before_copy_and_keeps_the_prior_snapshot() {
    let mut store = SnapshotStore::new("items");
    build(&mut store, vec![page(&[7], None, 1)], caps(10, 10_000)).unwrap();

    let err = build(
        &mut store,
        vec![
            page(&[1, 2], Some("0-1/*"), 2),
            page(&[3, 4], Some("0-1/*"), 3),
        ],
        caps(3, 10_000),
    )
    .unwrap_err();
    assert_eq!(err, ReadError::SnapshotTooLarge);
    assert_eq!(err.code(), "snapshot_too_large");
    assert_eq!(ids(&store.current().unwrap().rows), vec![7]);
}

#[test]
fn snapshot_byte_cap_is_enforced() {
    let mut store: SnapshotStore<Row> = SnapshotStore::new("items");
    let err = build(&mut store, vec![page(&[1], None, 1)], caps(10, 4)).unwrap_err();
    assert_eq!(err, ReadError::SnapshotTooLarge);
    assert!(store.current().is_none(), "no partial snapshot is kept");
}

#[test]
fn interrupted_snapshot_build_keeps_the_prior_snapshot() {
    let mut store = SnapshotStore::new("items");
    build(&mut store, vec![page(&[7], None, 1)], caps(10, 10_000)).unwrap();
    for failure in [status(500), Err(TransportError::Deadline), status(404)] {
        let err = build(
            &mut store,
            vec![page(&[1, 2], Some("0-1/*"), 2), failure],
            caps(10, 10_000),
        )
        .unwrap_err();
        assert_ne!(err, ReadError::SnapshotTooLarge);
        assert_eq!(ids(&store.current().unwrap().rows), vec![7]);
    }
}

#[test]
fn snapshot_build_replaces_the_prior_snapshot_only_on_success() {
    let mut store = SnapshotStore::new("items");
    build(&mut store, vec![page(&[7], None, 1)], caps(10, 10_000)).unwrap();
    build(
        &mut store,
        vec![page(&[8, 9], Some("0-1/2"), 2)],
        caps(10, 10_000),
    )
    .unwrap();
    assert_eq!(ids(&store.current().unwrap().rows), vec![8, 9]);
    assert_eq!(store.relation(), "items");
}

// ---- Content-Range ----

#[test]
fn content_range_parses_postgrest_forms() {
    assert_eq!(
        ContentRange::parse("0-24/*"),
        Some(ContentRange::Rows {
            first: 0,
            last: 24,
            total: None
        })
    );
    assert_eq!(
        ContentRange::parse("0-24/25").unwrap().says_complete(),
        Some(true)
    );
    assert_eq!(
        ContentRange::parse("0-24/100").unwrap().says_complete(),
        Some(false)
    );
    assert_eq!(ContentRange::parse("0-24/*").unwrap().says_complete(), None);
    assert_eq!(
        ContentRange::parse("*/*").unwrap().says_complete(),
        Some(true)
    );
    assert_eq!(
        ContentRange::parse("*/0").unwrap().says_complete(),
        Some(true)
    );
    assert_eq!(
        ContentRange::parse("items 0-1/2").unwrap().says_complete(),
        Some(true)
    );
    for bad in ["", "0-1", "1-0/*", "a-b/*", "0-1/x", "*"] {
        assert_eq!(ContentRange::parse(bad), None, "{bad:?}");
    }
}
