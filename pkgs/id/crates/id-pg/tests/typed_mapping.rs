//! Typed mapping, mapping version 1 (design #5805, §5.1, §9 invariants 5
//! and 6; slice #13761), as contract tests over page bodies the way
//! `PostgREST` writes them.
//!
//! The done check of #13761: the decoder keeps JSON numbers as text under
//! the declared type, and every §5.1 case passes. The bodies are written
//! by hand, not by a JSON library, so the number text the source sends is
//! exactly the text under test.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::{BTreeMap, VecDeque};

use id_pg::{
    CollectionAllowlist, ColumnDecl, DecodeError, Direction, EndpointConfig, EndpointProbe,
    FieldShape, HttpResponse, JsonShape, MAPPING_VERSION, MappedValue, ModeChoices, ObjectKind,
    OrderKey, PageDecoder, PgType, PostgrestRequest, Privilege, Query, ReadError, ReadMode,
    RegisteredEndpoint, RoleGrant, Transport, TransportError, TypedDecoder, TypedRow, Value,
    live_read, register,
};

/// One collection whose single typed column `v` is declared as `ty`, plus
/// an `int8` unique key `id`.
fn collection(ty: &PgType) -> (CollectionAllowlist, BTreeMap<String, PgType>) {
    let column = |name: &str| ColumnDecl {
        name: name.to_owned(),
        filterable: true,
        orderable: true,
    };
    let collection = CollectionAllowlist {
        schema: "api".to_owned(),
        relation: "items".to_owned(),
        columns: vec![column("id"), column("v")],
        unique_key: vec!["id".to_owned()],
        max_limit: Some(2),
    };
    let types = BTreeMap::from([
        ("id".to_owned(), PgType::Int8),
        ("v".to_owned(), ty.clone()),
    ]);
    (collection, types)
}

fn decoder(ty: &PgType) -> TypedDecoder {
    let (collection, types) = collection(ty);
    TypedDecoder::new(&collection, types).unwrap()
}

/// Decode `[{"id":1,"v":<json>}]` under `ty` and return `v`.
fn map(ty: &PgType, json: &str) -> Result<MappedValue, DecodeError> {
    let body = format!(r#"[{{"id":1,"v":{json}}}]"#);
    let mut rows = decoder(ty).decode(body.as_bytes())?;
    assert_eq!(rows.len(), 1);
    let row = rows.remove(0);
    Ok(row.get("v").unwrap().clone())
}

fn failed(ty: &PgType) -> DecodeError {
    DecodeError::TypeMapping {
        column: "v".to_owned(),
        declared_type: ty.name(),
    }
}

fn tagged(type_id: &str, payload: &str) -> MappedValue {
    MappedValue::Tagged {
        type_id: type_id.to_owned(),
        payload: payload.to_owned(),
    }
}

#[test]
fn mapping_version_is_one() {
    assert_eq!(MAPPING_VERSION, 1);
    assert_eq!(decoder(&PgType::Text).mapping_version(), 1);
}

#[test]
fn boolean_maps_directly() {
    assert_eq!(
        map(&PgType::Boolean, "true"),
        Ok(MappedValue::Boolean(true))
    );
    assert_eq!(
        map(&PgType::Boolean, "false"),
        Ok(MappedValue::Boolean(false))
    );
    assert_eq!(
        map(&PgType::Boolean, "\"t\""),
        Err(failed(&PgType::Boolean))
    );
    assert_eq!(map(&PgType::Boolean, "1"), Err(failed(&PgType::Boolean)));
}

#[test]
fn int8_near_two_to_the_63_is_exact() {
    // Oracle: §9 invariant 5 and §10 ("a bigint near 2^63"). Each of these
    // rounds when read through f64.
    for (text, value) in [
        ("9223372036854775807", i64::MAX),
        ("-9223372036854775808", i64::MIN),
        ("9223372036854775806", i64::MAX - 1),
        ("9007199254740993", 9_007_199_254_740_993),
    ] {
        assert_eq!(map(&PgType::Int8, text), Ok(MappedValue::Integer(value)));
    }
    assert_eq!(
        map(&PgType::Int8, "9223372036854775808"),
        Err(failed(&PgType::Int8))
    );
}

#[test]
fn smaller_integers_are_range_checked_and_never_fractional() {
    assert_eq!(
        map(&PgType::Int2, "-32768"),
        Ok(MappedValue::Integer(-32_768))
    );
    assert_eq!(map(&PgType::Int2, "32768"), Err(failed(&PgType::Int2)));
    assert_eq!(
        map(&PgType::Int4, "2147483647"),
        Ok(MappedValue::Integer(2_147_483_647))
    );
    assert_eq!(map(&PgType::Int4, "2147483648"), Err(failed(&PgType::Int4)));
    assert_eq!(map(&PgType::Int4, "1.0"), Err(failed(&PgType::Int4)));
    assert_eq!(map(&PgType::Int4, "1e2"), Err(failed(&PgType::Int4)));
    assert_eq!(map(&PgType::Int4, "\"1\""), Err(failed(&PgType::Int4)));
}

#[test]
fn floats_refuse_nan_and_infinities() {
    // Oracle: §5.1 and §10 ("NaN in a float"). Postgres writes NaN and the
    // infinities as JSON strings; a number that overflows f64 is the same
    // failure.
    assert_eq!(map(&PgType::Float8, "1.5"), Ok(MappedValue::Float64(1.5)));
    assert_eq!(
        map(&PgType::Float4, "-2e-3"),
        Ok(MappedValue::Float64(-0.002))
    );
    for bad in ["\"NaN\"", "\"Infinity\"", "\"-Infinity\"", "1e400"] {
        assert_eq!(
            map(&PgType::Float8, bad),
            Err(failed(&PgType::Float8)),
            "{bad}"
        );
    }
    assert_eq!(
        parse_body_error("[{\"id\":1,\"v\":NaN}]"),
        DecodeError::NotJson
    );
}

fn parse_body_error(body: &str) -> DecodeError {
    decoder(&PgType::Float8)
        .decode(body.as_bytes())
        .unwrap_err()
}

#[test]
fn numeric_is_tagged_with_its_exact_decimal_text() {
    for text in [
        "1.50",
        "12345678901234567890.123456789012345678",
        "-0.000",
        "1E+3",
    ] {
        assert_eq!(map(&PgType::Numeric, text), Ok(tagged("pg.numeric", text)));
    }
    assert_eq!(
        map(&PgType::Numeric, "\"NaN\""),
        Err(failed(&PgType::Numeric))
    );
}

#[test]
fn text_types_and_enums_map_to_strings() {
    for ty in [PgType::Text, PgType::Varchar, PgType::Char] {
        assert_eq!(
            map(&ty, r#""café 😀""#),
            Ok(MappedValue::String("café 😀".to_owned()))
        );
        assert_eq!(map(&ty, "1"), Err(failed(&ty)));
    }
    let mood = PgType::Enum {
        labels: vec!["happy".to_owned(), "sad".to_owned()],
    };
    assert_eq!(
        map(&mood, "\"sad\""),
        Ok(MappedValue::String("sad".to_owned()))
    );
    assert_eq!(map(&mood, "\"angry\""), Err(failed(&mood)));
}

#[test]
fn bytea_is_decoded_from_hex_never_read_as_text() {
    assert_eq!(
        map(&PgType::Bytea, r#""\\xdeadbeef""#),
        Ok(MappedValue::Bytes(vec![0xde, 0xad, 0xbe, 0xef]))
    );
    assert_eq!(
        map(&PgType::Bytea, r#""deadbeef""#),
        Err(failed(&PgType::Bytea))
    );
    assert_eq!(
        map(&PgType::Bytea, r#""\\xabc""#),
        Err(failed(&PgType::Bytea))
    );
}

#[test]
fn timestamptz_becomes_epoch_microseconds() {
    assert_eq!(
        map(
            &PgType::Timestamptz,
            r#""2001-09-09T01:46:40.000001+00:00""#
        ),
        Ok(MappedValue::Timestamp {
            micros: 1_000_000_000_000_001
        })
    );
    assert_eq!(
        map(&PgType::Timestamptz, r#""2001-09-09T03:46:40+02:00""#),
        Ok(MappedValue::Timestamp {
            micros: 1_000_000_000_000_000
        })
    );
    assert_eq!(
        map(&PgType::Timestamptz, r#""infinity""#),
        Err(failed(&PgType::Timestamptz))
    );
}

#[test]
fn date_time_and_interval_keep_their_own_type_ids() {
    assert_eq!(
        map(&PgType::Date, "\"2026-11-25\""),
        Ok(tagged("pg.date", "2026-11-25"))
    );
    assert_eq!(
        map(&PgType::Time, "\"06:00:00\""),
        Ok(tagged("pg.time", "06:00:00"))
    );
    assert_eq!(
        map(&PgType::Timetz, "\"06:00:00+05:30\""),
        Ok(tagged("pg.timetz", "06:00:00+05:30"))
    );
    assert_eq!(
        map(&PgType::Interval, "\"1 day 02:00:00\""),
        Ok(tagged("pg.interval", "1 day 02:00:00"))
    );
    assert_eq!(map(&PgType::Date, "20261125"), Err(failed(&PgType::Date)));
}

#[test]
fn undeclared_json_stays_opaque_and_keeps_number_text() {
    let ty = PgType::Jsonb { shape: None };
    assert_eq!(
        map(&ty, r#"{ "n" : 9223372036854775807, "x" : [1.10, null] }"#),
        Ok(tagged(
            "pg.jsonb",
            r#"{"n":9223372036854775807,"x":[1.10,null]}"#
        ))
    );
    assert_eq!(
        map(&PgType::Json { shape: None }, "\"s\""),
        Ok(tagged("pg.json", "\"s\""))
    );
}

#[test]
fn declared_json_shapes_become_records_and_lists() {
    let ty = PgType::Jsonb {
        shape: Some(JsonShape::Record {
            fields: vec![
                FieldShape {
                    name: "n".to_owned(),
                    shape: JsonShape::Integer,
                },
                FieldShape {
                    name: "tags".to_owned(),
                    shape: JsonShape::List {
                        element: Box::new(JsonShape::String),
                    },
                },
                FieldShape {
                    name: "note".to_owned(),
                    shape: JsonShape::Nullable {
                        inner: Box::new(JsonShape::String),
                    },
                },
            ],
        }),
    };
    assert_eq!(
        map(&ty, r#"{"tags":["a"],"note":null,"n":9223372036854775807}"#),
        Ok(MappedValue::Record(vec![
            ("n".to_owned(), MappedValue::Integer(i64::MAX)),
            (
                "tags".to_owned(),
                MappedValue::List(vec![MappedValue::String("a".to_owned())])
            ),
            ("note".to_owned(), MappedValue::Null),
        ]))
    );
    // An extra field, a missing field, a null where the shape has none,
    // and a wrong element type all fail the declared type.
    for bad in [
        r#"{"n":1,"tags":[],"note":null,"extra":1}"#,
        r#"{"n":1,"tags":[]}"#,
        r#"{"n":null,"tags":[],"note":null}"#,
        r#"{"n":1,"tags":[1],"note":null}"#,
        "[1]",
    ] {
        assert_eq!(map(&ty, bad), Err(failed(&ty)), "{bad}");
    }
}

#[test]
fn uuid_is_canonical_lowercase() {
    assert_eq!(
        map(&PgType::Uuid, "\"A0EEBC99-9C0B-4EF8-BB6D-6BB9BD380A11\""),
        Ok(MappedValue::String(
            "a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11".to_owned()
        ))
    );
    assert_eq!(
        map(&PgType::Uuid, "\"not-a-uuid\""),
        Err(failed(&PgType::Uuid))
    );
}

#[test]
fn arrays_map_each_element_under_the_element_type() {
    let ty = PgType::Array {
        element: Box::new(PgType::Int8),
    };
    assert_eq!(ty.name(), "int8[]");
    assert_eq!(
        map(&ty, "[9223372036854775807, null, -1]"),
        Ok(MappedValue::List(vec![
            MappedValue::Integer(i64::MAX),
            MappedValue::Null,
            MappedValue::Integer(-1),
        ]))
    );
    assert_eq!(map(&ty, "[1, 1.5]"), Err(failed(&ty)));
    let nested = PgType::Array {
        element: Box::new(ty),
    };
    assert_eq!(
        map(&nested, "[[1],[2]]"),
        Ok(MappedValue::List(vec![
            MappedValue::List(vec![MappedValue::Integer(1)]),
            MappedValue::List(vec![MappedValue::Integer(2)]),
        ]))
    );
}

#[test]
fn null_is_null_under_every_type_and_distinct_from_absent() {
    for ty in [
        PgType::Boolean,
        PgType::Int8,
        PgType::Numeric,
        PgType::Bytea,
        PgType::Timestamptz,
        PgType::Jsonb { shape: None },
        PgType::Uuid,
    ] {
        assert_eq!(map(&ty, "null"), Ok(MappedValue::Null));
    }
    let rows = decoder(&PgType::Text).decode(br#"[{"id":1}]"#).unwrap();
    assert_eq!(rows[0].get("v"), None);
}

#[test]
fn a_failure_names_column_and_type_but_never_the_value() {
    // Oracle: §9 invariant 6.
    let err = map(&PgType::Int4, "424242424242").unwrap_err();
    let ReadError::TypeMapping {
        column,
        declared_type,
    } = ReadError::from(err.clone())
    else {
        panic!("not a type mapping failure");
    };
    assert_eq!((column.as_str(), declared_type.as_str()), ("v", "int4"));
    assert!(!format!("{err:?}").contains("424242"));
    assert!(!ReadError::from(err).to_string().contains("424242"));
}

#[test]
fn bodies_that_are_not_an_array_of_objects_are_not_json() {
    let decoder = decoder(&PgType::Text);
    for body in [&b"{}"[..], b"[1]", b"[{\"id\":1}] trailing", b"", b"\xff"] {
        assert_eq!(decoder.decode(body), Err(DecodeError::NotJson));
    }
    // A repeated column would give one row two values.
    assert_eq!(
        decoder.decode(br#"[{"id":1,"id":2}]"#),
        Err(DecodeError::NotJson)
    );
}

#[test]
fn an_undeclared_column_in_a_row_is_refused_without_naming_it() {
    let err = decoder(&PgType::Text)
        .decode(br#"[{"id":1,"secret_salary":100}]"#)
        .unwrap_err();
    assert_eq!(err, DecodeError::MalformedRow);
    assert_eq!(ReadError::from(err), ReadError::MalformedPage);
}

#[test]
fn construction_requires_a_type_for_exactly_the_declared_columns() {
    let (items, mut types) = collection(&PgType::Text);
    types.remove("v");
    assert!(
        TypedDecoder::new(&items, types.clone())
            .unwrap_err()
            .contains("\"v\"")
    );
    types.insert("v".to_owned(), PgType::Text);
    types.insert("ghost".to_owned(), PgType::Text);
    assert!(
        TypedDecoder::new(&items, types)
            .unwrap_err()
            .contains("ghost")
    );

    let (items, mut types) = collection(&PgType::Enum { labels: vec![] });
    assert!(TypedDecoder::new(&items, types.clone()).is_err());
    types.insert(
        "v".to_owned(),
        PgType::Enum {
            labels: vec!["a".to_owned(), "a".to_owned()],
        },
    );
    assert!(TypedDecoder::new(&items, types).is_err());
}

#[test]
fn declared_types_round_trip_through_serde() {
    let ty = PgType::Array {
        element: Box::new(PgType::Jsonb {
            shape: Some(JsonShape::Nullable {
                inner: Box::new(JsonShape::Integer),
            }),
        }),
    };
    let text = serde_json::to_string(&ty).unwrap();
    assert_eq!(serde_json::from_str::<PgType>(&text).unwrap(), ty);
    let from_owner: PgType = serde_json::from_str(r#"{"type":"int8"}"#).unwrap();
    assert_eq!(from_owner, PgType::Int8);
}

// ---- keyset cursors through a live read ----

fn cursor_of(ty: &PgType, json: &str) -> Result<Vec<Value>, DecodeError> {
    let decoder = decoder(ty);
    let body = format!(r#"[{{"id":7,"v":{json}}}]"#);
    let rows = decoder.decode(body.as_bytes())?;
    decoder.cursor(&rows[0], &order(&["v", "id"]))
}

fn order(columns: &[&str]) -> Vec<OrderKey> {
    columns
        .iter()
        .map(|column| OrderKey {
            column: (*column).to_owned(),
            direction: Direction::Asc,
        })
        .collect()
}

#[test]
fn cursor_values_are_the_literals_postgrest_reads_back() {
    assert_eq!(
        cursor_of(&PgType::Int8, "9223372036854775807"),
        Ok(vec![Value::Int(i64::MAX), Value::Int(7)])
    );
    assert_eq!(
        cursor_of(&PgType::Numeric, "1.50"),
        Ok(vec![Value::Text("1.50".to_owned()), Value::Int(7)])
    );
    assert_eq!(
        cursor_of(&PgType::Timestamptz, r#""2001-09-09T03:46:40.5+02:00""#),
        Ok(vec![
            Value::Text("2001-09-09T01:46:40.500000Z".to_owned()),
            Value::Int(7)
        ])
    );
    assert_eq!(
        cursor_of(&PgType::Bytea, r#""\\x00ff""#),
        Ok(vec![Value::Text("\\x00ff".to_owned()), Value::Int(7)])
    );
    let opaque = PgType::Jsonb { shape: None };
    assert_eq!(cursor_of(&opaque, "{}"), Err(failed(&opaque)));
    // A row that lacks an ordering column cannot give a cursor.
    let decoder = decoder(&PgType::Text);
    let rows = decoder.decode(br#"[{"id":7}]"#).unwrap();
    assert_eq!(
        decoder.cursor(&rows[0], &order(&["v", "id"])),
        Err(DecodeError::MalformedRow)
    );
}

fn endpoint() -> RegisteredEndpoint {
    let (collection, _) = collection(&PgType::Text);
    let config = EndpointConfig {
        alias: "books-db".to_owned(),
        base_url: "https://pg.example.org/rest".to_owned(),
        schema: "api".to_owned(),
        credential_ref: "secret://pg-books-reader".to_owned(),
        collections: vec![collection],
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

struct Scripted {
    bodies: VecDeque<&'static str>,
    sent: Vec<PostgrestRequest>,
}

impl Transport for Scripted {
    fn get(
        &mut self,
        _endpoint: &RegisteredEndpoint,
        request: &PostgrestRequest,
    ) -> Result<HttpResponse, TransportError> {
        self.sent.push(request.clone());
        let body = self.bodies.pop_front().expect("more requests than pages");
        Ok(HttpResponse {
            status: 200,
            content_type: Some("application/json; charset=utf-8".to_owned()),
            content_range: None,
            body: body.as_bytes().to_vec(),
            observed_at_ms: 1,
        })
    }
}

#[test]
fn a_live_read_pages_on_an_int8_key_near_two_to_the_63_exactly() {
    let mut modes = ModeChoices::new();
    modes.choose("items", ReadMode::Live);
    let mut transport = Scripted {
        bodies: VecDeque::from([
            r#"[{"id":9223372036854775805,"v":"a"},{"id":9223372036854775806,"v":"b"}]"#,
            r#"[{"id":9223372036854775807,"v":"c"}]"#,
        ]),
        sent: Vec::new(),
    };
    let result = live_read(
        &endpoint(),
        &modes,
        &mut transport,
        &decoder(&PgType::Text),
        &Query::read("items"),
        5,
    )
    .unwrap();
    let ids: Vec<&MappedValue> = result
        .rows
        .iter()
        .map(|row: &TypedRow| row.get("id").unwrap())
        .collect();
    assert_eq!(
        ids,
        vec![
            &MappedValue::Integer(i64::MAX - 2),
            &MappedValue::Integer(i64::MAX - 1),
            &MappedValue::Integer(i64::MAX),
        ]
    );
    assert!(result.complete);
    // The second page starts strictly after the exact last key of the
    // first, which an f64 round trip would have moved.
    let keyset = transport.sent[1]
        .params
        .iter()
        .find(|(key, _)| key == "or")
        .map(|(_, value)| value.clone())
        .unwrap();
    assert!(keyset.contains("9223372036854775806"), "{keyset}");
}

#[test]
fn a_type_mapping_failure_ends_a_live_read_as_an_error() {
    let mut modes = ModeChoices::new();
    modes.choose("items", ReadMode::Live);
    let mut transport = Scripted {
        bodies: VecDeque::from([r#"[{"id":1.5,"v":"a"}]"#]),
        sent: Vec::new(),
    };
    let err = live_read(
        &endpoint(),
        &modes,
        &mut transport,
        &decoder(&PgType::Text),
        &Query::read("items"),
        5,
    )
    .unwrap_err();
    assert_eq!(err.code(), "type_mapping_failed");
}
