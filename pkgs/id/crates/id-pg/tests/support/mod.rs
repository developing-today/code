//! Shared PostgREST contract fixtures and transport used by offline and live tests.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use id_pg::{
    Audience, CollectionAllowlist, ColumnDecl, Consistency, EndpointConfig, EndpointProbe,
    ErrorCode, HttpResponse, MappedValue, ModeChoices, ObjectKind, PgType, PostgrestRequest,
    Privilege, Query, ReadError, ReadMode, Refusal, RegisteredEndpoint, RoleGrant, Transport,
    TransportError, TypedDecoder, live_read, register,
};

pub const SCHEMA: &str = "api";
pub const RELATION: &str = "contract_values";

/// One row exercises every scalar/structured PgType from design §5.1.
pub fn contract_types() -> BTreeMap<String, PgType> {
    use PgType as T;
    [
        ("id", T::Int8),
        ("bool_v", T::Boolean),
        ("int2_v", T::Int2),
        ("int4_v", T::Int4),
        ("int8_v", T::Int8),
        ("float4_v", T::Float4),
        ("float8_v", T::Float8),
        ("numeric_v", T::Numeric),
        ("text_v", T::Text),
        ("varchar_v", T::Varchar),
        ("char_v", T::Char),
        (
            "enum_v",
            T::Enum {
                labels: vec!["ready".into(), "done".into()],
            },
        ),
        ("bytea_v", T::Bytea),
        ("timestamptz_v", T::Timestamptz),
        ("date_v", T::Date),
        ("time_v", T::Time),
        ("timetz_v", T::Timetz),
        ("interval_v", T::Interval),
        ("json_v", T::Json { shape: None }),
        ("jsonb_v", T::Jsonb { shape: None }),
        (
            "json_record_v",
            T::Json {
                shape: Some(id_pg::JsonShape::Record {
                    fields: vec![id_pg::FieldShape {
                        name: "label".into(),
                        shape: id_pg::JsonShape::String,
                    }],
                }),
            },
        ),
        (
            "jsonb_list_v",
            T::Jsonb {
                shape: Some(id_pg::JsonShape::List {
                    element: Box::new(id_pg::JsonShape::Integer),
                }),
            },
        ),
        ("uuid_v", T::Uuid),
        (
            "int4_array_v",
            T::Array {
                element: Box::new(T::Int4),
            },
        ),
        ("null_v", T::Text),
    ]
    .into_iter()
    .map(|(name, ty)| (name.to_owned(), ty))
    .collect()
}

pub fn collection(relation: &str) -> CollectionAllowlist {
    let types = types_for(relation);
    CollectionAllowlist {
        schema: SCHEMA.into(),
        relation: relation.into(),
        columns: types
            .keys()
            .map(|name| ColumnDecl {
                name: name.clone(),
                filterable: true,
                orderable: name == "id",
            })
            .collect(),
        unique_key: vec!["id".into()],
        max_limit: Some(20),
    }
}

pub fn endpoint(base_url: String) -> RegisteredEndpoint {
    register(
        EndpointConfig {
            alias: "contract".into(),
            base_url,
            schema: SCHEMA.into(),
            credential_ref: "env:ID_PG_POSTGREST_TOKEN".into(),
            collections: vec![
                collection(RELATION),
                collection("nan_value"),
                collection("sql_failure"),
                collection("slow_value"),
            ],
        },
        &EndpointProbe {
            db_schemas: vec![SCHEMA.into()],
            role_grants: vec![
                RoleGrant {
                    kind: ObjectKind::Schema,
                    object: SCHEMA.into(),
                    privilege: Privilege::Usage,
                },
                RoleGrant {
                    kind: ObjectKind::Relation,
                    object: RELATION.into(),
                    privilege: Privilege::Select,
                },
            ],
        },
    )
    .unwrap()
}

fn types_for(relation: &str) -> BTreeMap<String, PgType> {
    if relation == RELATION {
        contract_types()
    } else {
        [("id".into(), PgType::Int8), ("v".into(), PgType::Float8)]
            .into_iter()
            .collect()
    }
}

pub fn decoder(relation: &str) -> TypedDecoder {
    TypedDecoder::new(&collection(relation), types_for(relation)).unwrap()
}

/// Expected values independent of the decoder; oracle: design §5.1 and init.sql literals.
pub fn expected_values() -> Vec<(&'static str, MappedValue)> {
    use MappedValue as V;
    let tag = |id: &str, payload: &str| V::Tagged {
        type_id: id.into(),
        payload: payload.into(),
    };
    vec![
        ("id", V::Integer(1)),
        ("bool_v", V::Boolean(true)),
        ("int2_v", V::Integer(7)),
        ("int4_v", V::Integer(70000)),
        ("int8_v", V::Integer(9_223_372_036_854_775_000)),
        ("float4_v", V::Float64(1.25)),
        ("float8_v", V::Float64(2.5)),
        (
            "numeric_v",
            tag("pg.numeric", "12345678901234567890.123456789"),
        ),
        ("text_v", V::String("hello".into())),
        ("varchar_v", V::String("varchar".into())),
        ("char_v", V::String("x".into())),
        ("enum_v", V::String("ready".into())),
        ("bytea_v", V::Bytes(vec![0, 255, 16, 66])),
        // 2024-02-03T02:05:06.123456Z, independently calculated from UTC epoch.
        (
            "timestamptz_v",
            V::Timestamp {
                micros: 1_706_925_906_123_456,
            },
        ),
        ("date_v", tag("pg.date", "2024-02-29")),
        ("time_v", tag("pg.time", "12:34:56.123456")),
        ("timetz_v", tag("pg.timetz", "12:34:56+02:00")),
        ("interval_v", tag("pg.interval", "1 day 02:03:04")),
        ("json_v", tag("pg.json", "{\"n\":7}")),
        ("jsonb_v", tag("pg.jsonb", "{\"ok\":true}")),
        (
            "json_record_v",
            V::Record(vec![("label".into(), V::String("row".into()))]),
        ),
        (
            "jsonb_list_v",
            V::List(vec![V::Integer(1), V::Integer(2), V::Integer(3)]),
        ),
        (
            "uuid_v",
            V::String("a0b1c2d3-e4f5-4678-9abc-0123456789ab".into()),
        ),
        (
            "int4_array_v",
            V::List(vec![V::Integer(1), V::Integer(2), V::Integer(3)]),
        ),
        ("null_v", V::Null),
    ]
}

/// Offline bytes mirror wire values, but expected mappings are specified separately above.
pub const HEALTHY_BODY: &str = r#"[{"id":1,"bool_v":true,"int2_v":7,"int4_v":70000,"int8_v":9223372036854775000,"float4_v":1.25,"float8_v":2.5,"numeric_v":12345678901234567890.123456789,"text_v":"hello","varchar_v":"varchar","char_v":"x","enum_v":"ready","bytea_v":"\\x00ff1042","timestamptz_v":"2024-02-03T04:05:06.123456+02:00","date_v":"2024-02-29","time_v":"12:34:56.123456","timetz_v":"12:34:56+02:00","interval_v":"1 day 02:03:04","json_v":{"n":7},"jsonb_v":{"ok":true},"json_record_v":{"label":"row"},"jsonb_list_v":[1,2,3],"uuid_v":"a0b1c2d3-e4f5-4678-9abc-0123456789ab","int4_array_v":[1,2,3],"null_v":null}]"#;

pub fn assert_healthy<T: Transport>(base: &str, transport: &mut T) {
    let mut modes = ModeChoices::new();
    modes.choose(RELATION, ReadMode::Live);
    let result = live_read(
        &endpoint(base.into()),
        &modes,
        transport,
        &decoder(RELATION),
        &Query::read(RELATION),
        1,
    )
    .unwrap();
    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0].columns.len(), expected_values().len());
    for (column, expected) in expected_values() {
        assert_eq!(
            result.rows[0].get(column),
            Some(&expected),
            "§5.1 column {column}"
        );
    }
    assert_eq!(result.consistency, Consistency::PerRequest);
    assert!(result.observed_at_ms > 0);
    assert_eq!(result.pages, 1);
    assert!(result.complete);
    assert!(!result.truncated);
}

#[derive(Clone, Copy, Debug)]
pub enum Fault {
    Nan,
    Deadline,
    Server,
    NonJson,
    Unauthorized,
}

/// Single inventory shared by both layers, oracle: design §5.1, §6.1–§6.2.
pub const FAULTS: [Fault; 5] = [
    Fault::Nan,
    Fault::Deadline,
    Fault::Server,
    Fault::NonJson,
    Fault::Unauthorized,
];

impl Fault {
    pub fn relation(self) -> &'static str {
        match self {
            Self::Nan => "nan_value",
            Self::Server => "sql_failure",
            Self::Deadline => "slow_value",
            Self::NonJson | Self::Unauthorized => RELATION,
        }
    }
    pub fn expected(self) -> ReadError {
        match self {
            Self::Nan => ReadError::TypeMapping {
                column: "v".into(),
                declared_type: "float8".into(),
            },
            Self::Deadline => ReadError::Deadline,
            Self::Server => ReadError::Status(503),
            Self::NonJson => ReadError::NonJsonBody,
            Self::Unauthorized => ReadError::Status(401),
        }
    }
    pub fn response(self) -> Result<HttpResponse, TransportError> {
        match self {
            Self::Deadline => Err(TransportError::Deadline),
            Self::Nan => Ok(response(
                200,
                "application/json",
                br#"[{"id":1,"v":"NaN"}]"#,
            )),
            Self::Server => Ok(response(
                503,
                "application/json",
                b"contract fixture failure",
            )),
            Self::NonJson => Ok(response(
                200,
                "text/html",
                b"<html>contract fixture failure</html>",
            )),
            Self::Unauthorized => Ok(response(401, "application/json", b"invalid-contract-jwt")),
        }
    }
}

pub fn response(status: u16, content_type: &str, body: &[u8]) -> HttpResponse {
    HttpResponse {
        status,
        content_type: Some(content_type.into()),
        content_range: None,
        body: body.into(),
        observed_at_ms: 1,
    }
}

pub fn assert_fault<T: Transport>(base: &str, transport: &mut T, fault: Fault) {
    let relation = fault.relation();
    let mut modes = ModeChoices::new();
    modes.choose(relation, ReadMode::Live);
    let error = live_read(
        &endpoint(base.into()),
        &modes,
        transport,
        &decoder(relation),
        &Query::read(relation),
        1,
    )
    .unwrap_err();
    assert_eq!(error, fault.expected(), "{fault:?}");
    let failure = error.failure().unwrap();
    let owner = failure.to_error(Audience::Owner);
    let grantee = failure.to_error(Audience::Grantee);
    let expected_code = match fault {
        Fault::Nan => ErrorCode::TypeMappingFailed,
        Fault::Deadline => ErrorCode::Timeout,
        Fault::Server | Fault::NonJson => ErrorCode::AdapterUnavailable,
        Fault::Unauthorized => ErrorCode::CredentialUnavailable,
    };
    assert_eq!(owner.code, expected_code);
    if matches!(fault, Fault::Unauthorized) {
        assert_eq!(
            grantee.body(),
            id_pg::Failure::Refused(Refusal::not_available())
                .to_error(Audience::Grantee)
                .body()
        );
    } else {
        assert_eq!(grantee.code, expected_code);
    }
    assert!(grantee.detail.is_none());
    for body in [owner.body(), grantee.body()] {
        for source_value in [
            "NaN",
            "invalid-contract-jwt",
            "contract fixture failure",
            "<html>",
        ] {
            assert!(!body.contains(source_value));
        }
    }
}

pub fn start_non_json_server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut request = [0_u8; 1024];
            let _ = stream.read(&mut request);
            let body = b"<html>not a PostgREST success body</html>";
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(body);
            let _ = stream.flush();
        }
    });
    format!("http://{address}")
}

/// Real HTTP transport through the installed curl binary (no Rust HTTP dependency).
pub struct CurlTransport {
    pub timeout_seconds: u32,
    pub invalid_jwt: bool,
}

impl Default for CurlTransport {
    fn default() -> Self {
        Self {
            timeout_seconds: 8,
            invalid_jwt: false,
        }
    }
}

impl Transport for CurlTransport {
    fn get(
        &mut self,
        endpoint: &RegisteredEndpoint,
        request: &PostgrestRequest,
    ) -> Result<HttpResponse, TransportError> {
        let mut command = Command::new("curl");
        command.args([
            "--disable",
            "--silent",
            "--show-error",
            "--noproxy",
            "*",
            "--get",
            "--max-time",
            &self.timeout_seconds.to_string(),
            "--proto",
            "=http,https",
            "--write-out",
            "\n__ID_PG_HTTP__%{http_code}\n%{content_type}\n%{header_json}",
        ]);
        for (name, value) in &request.params {
            command
                .arg("--data-urlencode")
                .arg(format!("{name}={value}"));
        }
        command
            .arg("--header")
            .arg(format!("Accept-Profile: {}", request.accept_profile));
        if self.invalid_jwt {
            command.args(["--header", "Authorization: Bearer invalid-contract-jwt"]);
        }
        let url = format!(
            "{}{}",
            endpoint.base_url().trim_end_matches('/'),
            request.path
        );
        command.arg("--url").arg(url);
        let output = command.output().map_err(|_| TransportError::Connection)?;
        if !output.status.success() {
            return Err(if output.status.code() == Some(28) {
                TransportError::Deadline
            } else {
                TransportError::Connection
            });
        }
        let marker = b"\n__ID_PG_HTTP__";
        let Some(index) = output
            .stdout
            .windows(marker.len())
            .rposition(|window| window == marker)
        else {
            return Err(TransportError::Connection);
        };
        let body = output.stdout[..index].to_vec();
        let meta = String::from_utf8_lossy(&output.stdout[index + marker.len()..]);
        let mut lines = meta.lines();
        let status = lines
            .next()
            .and_then(|s| s.parse().ok())
            .ok_or(TransportError::Connection)?;
        let content_type = lines.next().filter(|s| !s.is_empty()).map(str::to_owned);
        let headers: serde_json::Value =
            serde_json::from_str(&lines.collect::<Vec<_>>().join("\n"))
                .map_err(|_| TransportError::Connection)?;
        let content_range = headers
            .get("content-range")
            .and_then(|v| v.get(0))
            .and_then(|v| v.as_str())
            .map(str::to_owned);
        let observed_at_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .min(u128::from(u64::MAX)) as u64;
        Ok(HttpResponse {
            status,
            content_type,
            content_range,
            body,
            observed_at_ms,
        })
    }
}
