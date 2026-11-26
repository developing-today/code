//! The central plane over the libSQL HTTP pipeline (Hrana v2), the transport
//! shape the fleet queue already uses (design §1).
//!
//! One [`Pipeline::transaction`] is one HTTP request holding one Hrana
//! `batch`: `BEGIN IMMEDIATE`, then each statement on the condition that the
//! step before it succeeded, then `COMMIT` on the same condition, then
//! `ROLLBACK` if the commit did not run or failed. So either every statement
//! commits or none does, exactly as [`crate::central::SqlitePrimary`] does.
//!
//! The endpoint is configured, never assumed: `FLEET_KV_URL` names it (there
//! is no default hostname) and `FLEET_KV_TOKEN_FILE` names a file holding the
//! bearer the endpoint authenticates (an RP/agent JWT or a route-scoped
//! service token, design §9). A per-user Turso data-plane token is refused by
//! the endpoint, and that refusal is surfaced as `data_plane_credential_refused`.

use std::time::Duration;

use serde_json::{Value as Json, json};

use crate::blob::{KvBlobRefV1, Partition};
use crate::blob_read::BlobReader;
use crate::central::{Address, CentralError, Pipeline, Result, Rows, Statement, Value};
use crate::sweep::{BlobUpload, BlobUploader};

/// The environment variable naming the central endpoint.
pub const URL_VAR: &str = "FLEET_KV_URL";
/// The environment variable naming the file that holds the bearer.
pub const TOKEN_FILE_VAR: &str = "FLEET_KV_TOKEN_FILE";
/// Every request gives up after this long. It stays near the 5 s bound a
/// `clear-override` waits for an in-progress submission.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

pub struct HttpPipeline {
    url: String,
    bearer: String,
    agent: ureq::Agent,
}

impl HttpPipeline {
    /// A pipeline to `url` (https, or http on a loopback host only).
    pub fn new(url: &str, bearer: &str) -> Result<Self> {
        let url = url.trim().trim_end_matches('/').to_string();
        if !(url.starts_with("https://") || is_loopback_http(&url)) {
            return Err(CentralError::NotConfigured(format!(
                "{URL_VAR} must be an https URL (got {url:?})"
            )));
        }
        let bearer = bearer.trim().to_string();
        if bearer.is_empty() {
            return Err(CentralError::NotConfigured(
                "the central credential is empty".into(),
            ));
        }
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(REQUEST_TIMEOUT))
            .build()
            .into();
        Ok(Self { url, bearer, agent })
    }

    /// The pipeline the environment configures, or `None` when no central
    /// endpoint is configured.
    pub fn from_env() -> Result<Option<Self>> {
        let Some(url) = std::env::var(URL_VAR)
            .ok()
            .filter(|url| !url.trim().is_empty())
        else {
            return Ok(None);
        };
        let path = std::env::var(TOKEN_FILE_VAR).map_err(|_| {
            CentralError::NotConfigured(format!("{URL_VAR} is set but {TOKEN_FILE_VAR} is not"))
        })?;
        let bearer = std::fs::read_to_string(&path).map_err(|error| {
            CentralError::NotConfigured(format!("cannot read {TOKEN_FILE_VAR} ({path}): {error}"))
        })?;
        Self::new(&url, &bearer).map(Some)
    }
}

/// An upload streams a value of any size, so it gets far longer than a
/// pipeline request.
pub const UPLOAD_TIMEOUT: Duration = Duration::from_secs(600);

/// The trusted upload door (design §9) beside the central endpoint:
/// `PUT {FLEET_KV_URL}/blobs`, with the same bearer, which must be allowed
/// the `kv-upload` door.
pub struct HttpBlobUploader {
    url: String,
    bearer: String,
    agent: ureq::Agent,
}

impl HttpBlobUploader {
    pub fn new(url: &str, bearer: &str) -> Result<Self> {
        // The same URL and credential rules as the pipeline.
        let pipeline = HttpPipeline::new(url, bearer)?;
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(UPLOAD_TIMEOUT))
            .build()
            .into();
        Ok(Self {
            url: pipeline.url,
            bearer: pipeline.bearer,
            agent,
        })
    }

    /// The uploader the environment configures beside the central
    /// endpoint, or `None` when no central endpoint is configured. The
    /// partition is not the host's to pick: each blob goes to its
    /// namespace's placement (design §10).
    pub fn from_env() -> Result<Option<Self>> {
        let Some(pipeline) = HttpPipeline::from_env()? else {
            return Ok(None);
        };
        Self::new(&pipeline.url, &pipeline.bearer).map(Some)
    }
}

impl BlobUploader for HttpBlobUploader {
    fn upload(
        &self,
        upload: &BlobUpload<'_>,
        bytes: &mut dyn std::io::Read,
    ) -> Result<KvBlobRefV1> {
        let mut response = self
            .agent
            .put(&format!("{}/blobs", self.url))
            .query("registry", upload.registry_id)
            .query("namespace", upload.namespace)
            .query("scope", upload.scope_id)
            .query("jurisdiction", &upload.partition.jurisdiction)
            .query("tier", &upload.partition.tier)
            .query("digest", upload.digest)
            .query("bytes_len", upload.bytes_len.to_string())
            .header("Authorization", &format!("Bearer {}", self.bearer))
            .header("Content-Type", "application/octet-stream")
            .send(ureq::SendBody::from_reader(bytes))
            .map_err(|error| CentralError::Unavailable(error.to_string()))?;
        let status = response.status().as_u16();
        let text = response
            .body_mut()
            .read_to_string()
            .map_err(|error| CentralError::Unavailable(format!("reading the answer: {error}")))?;
        if status == 429 || status >= 500 {
            return Err(CentralError::Unavailable(format!(
                "HTTP {status}: {}",
                snippet(&text)
            )));
        }
        if !(200..300).contains(&status) {
            return Err(refusal(status, &text));
        }
        parse_upload_answer(&text)
    }
}

/// The read side of the blob door (design §9, D011): `POST
/// {FLEET_KV_URL}/blobs/capability` mints a short-lived capability from the
/// key's live reference, and `GET {FLEET_KV_URL}/blobs/read` streams the
/// bytes it names. The caller verifies the stream against the reference.
pub struct HttpBlobReader {
    url: String,
    bearer: String,
    agent: ureq::Agent,
}

impl HttpBlobReader {
    pub fn new(url: &str, bearer: &str) -> Result<Self> {
        let pipeline = HttpPipeline::new(url, bearer)?;
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(UPLOAD_TIMEOUT))
            .build()
            .into();
        Ok(Self {
            url: pipeline.url,
            bearer: pipeline.bearer,
            agent,
        })
    }

    /// The reader the environment configures beside the central endpoint,
    /// or `None` when no central endpoint is configured.
    pub fn from_env() -> Result<Option<Self>> {
        let Some(pipeline) = HttpPipeline::from_env()? else {
            return Ok(None);
        };
        Self::new(&pipeline.url, &pipeline.bearer).map(Some)
    }
}

impl BlobReader for HttpBlobReader {
    fn open(&self, address: &Address, reference: &KvBlobRefV1) -> Result<Box<dyn std::io::Read>> {
        let body = json!({
            "registry": address.registry_id,
            "namespace": address.namespace,
            "scope": address.scope_id,
            "key": address.key,
        })
        .to_string();
        let mut response = self
            .agent
            .post(&format!("{}/blobs/capability", self.url))
            .header("Authorization", &format!("Bearer {}", self.bearer))
            .header("Content-Type", "application/json")
            .send(body.as_str())
            .map_err(|error| CentralError::Unavailable(error.to_string()))?;
        let status = response.status().as_u16();
        let text = response
            .body_mut()
            .read_to_string()
            .map_err(|error| CentralError::Unavailable(format!("reading the answer: {error}")))?;
        if status == 429 || status >= 500 {
            return Err(CentralError::Unavailable(format!(
                "HTTP {status}: {}",
                snippet(&text)
            )));
        }
        if !(200..300).contains(&status) {
            return Err(refusal(status, &text));
        }
        let query = parse_capability_answer(&text, reference)?;
        // Only the query travels: the read goes to this endpoint, never to
        // a host the answer names.
        let response = self
            .agent
            .get(&format!("{}/blobs/read?{query}", self.url))
            .call()
            .map_err(|error| CentralError::Unavailable(error.to_string()))?;
        let status = response.status().as_u16();
        if !(200..300).contains(&status) {
            let text = response.into_body().read_to_string().unwrap_or_default();
            if status == 429 || status >= 500 {
                return Err(CentralError::Unavailable(format!(
                    "HTTP {status}: {}",
                    snippet(&text)
                )));
            }
            return Err(refusal(status, &text));
        }
        Ok(Box::new(response.into_body().into_reader()))
    }
}

/// [`HttpBlobReader`] as the environment configures it, opened only when a
/// read meets a blob-backed value, so an inline read never needs the blob
/// door.
pub struct EnvBlobReader;

impl BlobReader for EnvBlobReader {
    fn open(&self, address: &Address, reference: &KvBlobRefV1) -> Result<Box<dyn std::io::Read>> {
        HttpBlobReader::from_env()?
            .ok_or_else(|| {
                CentralError::NotConfigured(format!(
                    "this value is a blob; set {URL_VAR} and {TOKEN_FILE_VAR} to read it"
                ))
            })?
            .open(address, reference)
    }
}

/// The read query in the capability door's answer, once the capability
/// names exactly the reference the caller read. A capability for other
/// bytes means the head moved since: retry the read.
pub fn parse_capability_answer(text: &str, reference: &KvBlobRefV1) -> Result<String> {
    let parsed: Json = serde_json::from_str(text)
        .map_err(|error| CentralError::Store(format!("a non-JSON capability answer: {error}")))?;
    let field = |name: &str| parsed.get(name).and_then(Json::as_str).unwrap_or_default();
    let read_url = field("read_url");
    let query = read_url
        .strip_prefix("/v1/kv/blobs/read?")
        .filter(|query| !query.is_empty() && !query.contains(['#', ' ']))
        .ok_or_else(|| CentralError::Store("the capability answer has no read URL".into()))?;
    let same = field("digest") == reference.digest
        && parsed.get("bytes_len").and_then(Json::as_u64) == Some(reference.bytes_len)
        && field("jurisdiction") == reference.partition.jurisdiction
        && field("tier") == reference.partition.tier;
    if !same {
        return Err(CentralError::Unavailable(
            "the key's head moved while it was being read; read it again".into(),
        ));
    }
    Ok(query.to_string())
}

/// The reference in the upload door's answer.
pub fn parse_upload_answer(text: &str) -> Result<KvBlobRefV1> {
    let parsed: Json = serde_json::from_str(text)
        .map_err(|error| CentralError::Store(format!("a non-JSON upload answer: {error}")))?;
    let reference = parsed
        .get("reference")
        .and_then(Json::as_str)
        .ok_or_else(|| CentralError::Store("the upload answer has no reference".into()))?;
    KvBlobRefV1::decode(reference.as_bytes())
        .map_err(|error| CentralError::Store(format!("the upload answer's reference: {error}")))
}

fn is_loopback_http(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("http://") else {
        return false;
    };
    let host = rest.split(['/', ':']).next().unwrap_or_default();
    matches!(host, "127.0.0.1" | "localhost") || rest.starts_with("[::1]")
}

impl Pipeline for HttpPipeline {
    fn transaction(&self, statements: &[Statement]) -> Result<Vec<Rows>> {
        let body = request_body(statements).to_string();
        let mut response = self
            .agent
            .post(&format!("{}/v2/pipeline", self.url))
            .header("Authorization", &format!("Bearer {}", self.bearer))
            .header("Content-Type", "application/json")
            .send(body.as_str())
            .map_err(|error| CentralError::Unavailable(error.to_string()))?;
        let status = response.status().as_u16();
        let text = response
            .body_mut()
            .read_to_string()
            .map_err(|error| CentralError::Unavailable(format!("reading the answer: {error}")))?;
        if status == 429 || status >= 500 {
            return Err(CentralError::Unavailable(format!(
                "HTTP {status}: {}",
                snippet(&text)
            )));
        }
        if !(200..300).contains(&status) {
            return Err(refusal(status, &text));
        }
        let parsed: Json = serde_json::from_str(&text)
            .map_err(|error| CentralError::Store(format!("a non-JSON answer: {error}")))?;
        parse_response(&parsed, statements.len())
    }
}

fn snippet(text: &str) -> String {
    text.chars().take(300).collect()
}

fn refusal(status: u16, text: &str) -> CentralError {
    let code = serde_json::from_str::<Json>(text).ok().and_then(|body| {
        body.get("error")
            .and_then(|error| error.get("code").or(Some(error)))
            .or_else(|| body.get("code"))
            .and_then(Json::as_str)
            .map(str::to_string)
    });
    let code = match (code.as_deref(), status) {
        (Some("data_plane_credential_refused"), _) => "data_plane_credential_refused",
        (_, 401) => "unauthorized",
        (_, 403) => "forbidden",
        _ => "central_refused",
    };
    CentralError::Refused {
        code,
        message: format!("HTTP {status}: {}", snippet(text)),
    }
}

/// The Hrana request for one transaction.
pub fn request_body(statements: &[Statement]) -> Json {
    let count = statements.len();
    let mut steps = vec![json!({"stmt": {"sql": "BEGIN IMMEDIATE"}})];
    for (index, statement) in statements.iter().enumerate() {
        steps.push(json!({
            "condition": {"type": "ok", "step": index},
            "stmt": {
                "sql": statement.sql,
                "args": statement.args.iter().map(encode_value).collect::<Vec<_>>(),
                "want_rows": true,
            },
        }));
    }
    steps.push(json!({
        "condition": {"type": "ok", "step": count},
        "stmt": {"sql": "COMMIT"},
    }));
    steps.push(json!({
        "condition": {"type": "not", "cond": {"type": "ok", "step": count + 1}},
        "stmt": {"sql": "ROLLBACK"},
    }));
    json!({
        "requests": [
            {"type": "batch", "batch": {"steps": steps}},
            {"type": "close"},
        ]
    })
}

fn error_message(error: &Json) -> String {
    error
        .get("message")
        .and_then(Json::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| error.to_string())
}

/// The statements' rows from a Hrana answer to [`request_body`].
pub fn parse_response(parsed: &Json, count: usize) -> Result<Vec<Rows>> {
    if let Some(error) = parsed.get("error") {
        return Err(CentralError::Store(error_message(error)));
    }
    let first = parsed
        .get("results")
        .and_then(Json::as_array)
        .and_then(|results| results.first())
        .ok_or_else(|| CentralError::Store("the answer has no results".into()))?;
    if first.get("type").and_then(Json::as_str) != Some("ok") {
        let message = first.get("error").map(error_message).unwrap_or_default();
        return Err(CentralError::Store(format!("the batch failed: {message}")));
    }
    let result = first
        .get("response")
        .and_then(|response| response.get("result"))
        .ok_or_else(|| CentralError::Store("the batch answer has no result".into()))?;
    let step_results = result
        .get("step_results")
        .and_then(Json::as_array)
        .ok_or_else(|| CentralError::Store("the batch answer has no step results".into()))?;
    let step_errors = result.get("step_errors").and_then(Json::as_array);
    if let Some(errors) = step_errors {
        if let Some((step, error)) = errors
            .iter()
            .enumerate()
            .take(count + 2)
            .find(|(_, error)| !error.is_null())
        {
            return Err(CentralError::Store(format!(
                "step {step} failed: {}",
                error_message(error)
            )));
        }
    }
    if step_results.get(count + 1).is_none_or(Json::is_null) {
        return Err(CentralError::Store("the transaction did not commit".into()));
    }
    (1..=count)
        .map(|step| {
            let result = step_results
                .get(step)
                .filter(|result| !result.is_null())
                .ok_or_else(|| CentralError::Store(format!("step {step} did not run")))?;
            Ok(Rows {
                columns: result
                    .get("cols")
                    .and_then(Json::as_array)
                    .map(|columns| {
                        columns
                            .iter()
                            .map(|column| {
                                column
                                    .get("name")
                                    .and_then(Json::as_str)
                                    .unwrap_or_default()
                                    .to_string()
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
                rows: result
                    .get("rows")
                    .and_then(Json::as_array)
                    .map(|rows| {
                        rows.iter()
                            .map(|row| {
                                row.as_array().map_or_else(
                                    || Ok(Vec::new()),
                                    |cells| cells.iter().map(decode_value).collect(),
                                )
                            })
                            .collect::<Result<Vec<Vec<Value>>>>()
                    })
                    .transpose()?
                    .unwrap_or_default(),
            })
        })
        .collect()
}

/// A value in Hrana's JSON form.
pub fn encode_value(value: &Value) -> Json {
    match value {
        Value::Null => json!({"type": "null"}),
        Value::Integer(value) => json!({"type": "integer", "value": value.to_string()}),
        Value::Real(value) => json!({"type": "float", "value": value}),
        Value::Text(value) => json!({"type": "text", "value": value}),
        Value::Blob(value) => json!({"type": "blob", "base64": base64_encode(value)}),
    }
}

/// A Hrana JSON value.
pub fn decode_value(cell: &Json) -> Result<Value> {
    let raw = cell.get("value");
    let bad = || CentralError::Store(format!("an unreadable value {cell}"));
    Ok(match cell.get("type").and_then(Json::as_str) {
        Some("null") => Value::Null,
        Some("integer") => Value::Integer(
            raw.and_then(|value| {
                value
                    .as_str()
                    .and_then(|text| text.parse().ok())
                    .or_else(|| value.as_i64())
            })
            .ok_or_else(bad)?,
        ),
        Some("float") => Value::Real(
            raw.and_then(|value| {
                value
                    .as_f64()
                    .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
            })
            .ok_or_else(bad)?,
        ),
        Some("text") => Value::Text(raw.and_then(Json::as_str).ok_or_else(bad)?.to_string()),
        Some("blob") => Value::Blob(
            cell.get("base64")
                .and_then(Json::as_str)
                .and_then(base64_decode)
                .ok_or_else(bad)?,
        ),
        _ => return Err(bad()),
    })
}

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard base64 with padding.
pub fn base64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = u32::from(chunk[0]);
        let b1 = u32::from(chunk.get(1).copied().unwrap_or(0));
        let b2 = u32::from(chunk.get(2).copied().unwrap_or(0));
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(char::from(ALPHABET[(n >> 18) as usize & 63]));
        out.push(char::from(ALPHABET[(n >> 12) as usize & 63]));
        out.push(if chunk.len() > 1 {
            char::from(ALPHABET[(n >> 6) as usize & 63])
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            char::from(ALPHABET[n as usize & 63])
        } else {
            '='
        });
    }
    out
}

/// Standard base64, padded or not.
pub fn base64_decode(text: &str) -> Option<Vec<u8>> {
    let digits: Vec<u8> = text
        .trim_end_matches('=')
        .bytes()
        .map(|byte| ALPHABET.iter().position(|&a| a == byte).map(|p| p as u8))
        .collect::<Option<_>>()?;
    if digits.len() % 4 == 1 {
        return None;
    }
    let mut out = Vec::with_capacity(digits.len() * 3 / 4);
    for group in digits.chunks(4) {
        let mut n = 0u32;
        for (index, digit) in group.iter().enumerate() {
            n |= u32::from(*digit) << (18 - 6 * index);
        }
        out.push((n >> 16) as u8);
        if group.len() > 2 {
            out.push((n >> 8) as u8);
        }
        if group.len() > 3 {
            out.push(n as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::central::{Address, IdempotencyKey, Intent, Op, Outcome, SqlitePrimary, cas, head};
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::sync::Arc;

    #[test]
    fn central_http_base64_round_trips() {
        for length in 0..40usize {
            let bytes: Vec<u8> = (0..length).map(|i| (i * 37 % 256) as u8).collect();
            let encoded = base64_encode(&bytes);
            assert_eq!(encoded.len() % 4, 0);
            assert_eq!(base64_decode(&encoded).unwrap(), bytes);
            assert_eq!(base64_decode(encoded.trim_end_matches('=')).unwrap(), bytes);
        }
        assert_eq!(base64_encode(b"Man"), "TWFu");
        assert_eq!(base64_encode(b"Ma"), "TWE=");
        assert_eq!(base64_decode("not base64!"), None);
    }

    #[test]
    fn central_http_values_round_trip() {
        for value in [
            Value::Null,
            Value::Integer(-7),
            Value::Integer(i64::MAX),
            Value::Real(1.5),
            Value::Text("héllo".into()),
            Value::Blob(vec![0, 255, 1]),
        ] {
            assert_eq!(decode_value(&encode_value(&value)).unwrap(), value);
        }
    }

    #[test]
    fn central_http_batch_is_all_or_nothing() {
        let statements = vec![
            Statement {
                sql: "SELECT ?1".into(),
                args: vec![Value::Integer(1)],
            },
            Statement {
                sql: "SELECT 2".into(),
                args: vec![],
            },
        ];
        let body = request_body(&statements);
        let steps = body["requests"][0]["batch"]["steps"].as_array().unwrap();
        assert_eq!(steps.len(), 5);
        assert_eq!(steps[0]["stmt"]["sql"], "BEGIN IMMEDIATE");
        assert_eq!(steps[1]["condition"], json!({"type": "ok", "step": 0}));
        assert_eq!(steps[2]["condition"], json!({"type": "ok", "step": 1}));
        assert_eq!(steps[3]["stmt"]["sql"], "COMMIT");
        assert_eq!(steps[3]["condition"], json!({"type": "ok", "step": 2}));
        assert_eq!(steps[4]["stmt"]["sql"], "ROLLBACK");
        assert_eq!(
            steps[4]["condition"],
            json!({"type": "not", "cond": {"type": "ok", "step": 3}})
        );
        assert_eq!(body["requests"][1]["type"], "close");
    }

    #[test]
    fn central_http_a_failed_step_is_an_error_not_a_partial_commit() {
        let answer = json!({"results": [{"type": "ok", "response": {"type": "batch", "result": {
            "step_results": [{}, null, null, null, {}],
            "step_errors": [null, {"message": "constraint failed"}, null, null, null],
        }}}]});
        let error = parse_response(&answer, 2).unwrap_err();
        assert!(error.to_string().contains("constraint failed"), "{error}");
        let uncommitted = json!({"results": [{"type": "ok", "response": {"type": "batch", "result": {
            "step_results": [{}, {}, {}, null, {}],
            "step_errors": [null, null, null, null, null],
        }}}]});
        assert!(parse_response(&uncommitted, 2).is_err());
    }

    #[test]
    fn central_http_refuses_non_https_urls() {
        assert!(HttpPipeline::new("http://kv.example.com", "token").is_err());
        assert!(HttpPipeline::new("https://kv.example.com/", "token").is_ok());
        assert!(HttpPipeline::new("http://127.0.0.1:9", "token").is_ok());
        assert!(HttpPipeline::new("https://kv.example.com", " ").is_err());
    }

    #[test]
    fn central_http_refusals_are_typed() {
        let refused = refusal(
            403,
            r#"{"error":{"code":"data_plane_credential_refused","message":"no"}}"#,
        );
        assert_eq!(refused.code(), "data_plane_credential_refused");
        assert_eq!(refusal(401, "nope").code(), "unauthorized");
        assert_eq!(refusal(404, "{}").code(), "central_refused");
    }

    #[test]
    fn central_http_upload_answer_carries_a_published_reference() {
        let reference = crate::blob::KvBlobRefV1 {
            digest: "ab".repeat(32),
            bytes_len: 9,
            partition: Partition::new("eu", "standard"),
            locator: format!("r2:kv/blobs/eu/standard/{}", "ab".repeat(32)),
        };
        let encoded = String::from_utf8(reference.encode()).unwrap();
        let answer =
            json!({"reference": encoded, "digest": "ab".repeat(32), "deduplicated": false});
        assert_eq!(parse_upload_answer(&answer.to_string()).unwrap(), reference);
        for bad in [
            "not json".to_string(),
            json!({"digest": "x"}).to_string(),
            json!({"reference": encoded.replace("state=published", "state=staged")}).to_string(),
        ] {
            assert!(
                matches!(parse_upload_answer(&bad), Err(CentralError::Store(_))),
                "{bad}"
            );
        }
        assert!(HttpBlobUploader::new("http://kv.example.com", "token").is_err());
        assert!(HttpBlobUploader::new("https://kv.example.com/v1/kv", "token").is_ok());
    }

    fn blob_reference(bytes: &[u8]) -> crate::blob::KvBlobRefV1 {
        let digest = blake3::hash(bytes).to_hex().to_string();
        crate::blob::KvBlobRefV1 {
            locator: format!("r2:kv/blobs/eu/standard/{digest}"),
            digest,
            bytes_len: bytes.len() as u64,
            partition: Partition::new("eu", "standard"),
        }
    }

    fn capability_answer(reference: &crate::blob::KvBlobRefV1) -> Json {
        json!({
            "read_url": format!("/v1/kv/blobs/read?digest={}&mac=m", reference.digest),
            "expires_at_ms": 1,
            "digest": reference.digest,
            "bytes_len": reference.bytes_len,
            "jurisdiction": reference.partition.jurisdiction,
            "tier": reference.partition.tier,
        })
    }

    #[test]
    fn central_http_capability_answer_must_name_the_reference_read() {
        let reference = blob_reference(b"bytes");
        let answer = capability_answer(&reference);
        assert_eq!(
            parse_capability_answer(&answer.to_string(), &reference).unwrap(),
            format!("digest={}&mac=m", reference.digest)
        );
        // The head moved: another blob's capability is a retryable miss.
        let other = blob_reference(b"other");
        let moved = parse_capability_answer(&capability_answer(&other).to_string(), &reference)
            .unwrap_err();
        assert!(moved.is_retryable(), "{moved}");
        // A read URL elsewhere is never followed.
        let mut elsewhere = answer.clone();
        elsewhere["read_url"] = json!("https://evil.example/v1/kv/blobs/read?x=1");
        assert!(matches!(
            parse_capability_answer(&elsewhere.to_string(), &reference),
            Err(CentralError::Store(_))
        ));
        assert!(parse_capability_answer("not json", &reference).is_err());
    }

    /// A door with the capability and read routes, one request at a time.
    fn serve_blob_door(bytes: Vec<u8>, capability_status: &'static str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let reference = blob_reference(&bytes);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut request_line = String::new();
                reader.read_line(&mut request_line).unwrap_or(0);
                let mut length = 0usize;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                        break;
                    }
                    if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        length = value.trim().parse().unwrap_or(0);
                    }
                }
                let mut body = vec![0u8; length];
                reader.read_exact(&mut body).unwrap();
                let (status, content_type, answer) =
                    if request_line.starts_with("POST /v1/kv/blobs/capability ") {
                        let asked: Json = serde_json::from_slice(&body).unwrap();
                        assert_eq!(asked["key"], "big");
                        assert_eq!(asked["scope"], "shared:team");
                        (
                            capability_status,
                            "application/json",
                            if capability_status.starts_with("200") {
                                capability_answer(&reference).to_string().into_bytes()
                            } else {
                                json!({"error": "no_live_reference"})
                                    .to_string()
                                    .into_bytes()
                            },
                        )
                    } else if request_line.starts_with(&format!(
                        "GET /v1/kv/blobs/read?digest={}&mac=m ",
                        reference.digest
                    )) {
                        ("200 OK", "application/octet-stream", bytes.clone())
                    } else {
                        ("404 Not Found", "text/plain", b"no route".to_vec())
                    };
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\n\
                     Content-Length: {}\r\nConnection: close\r\n\r\n",
                    answer.len()
                );
                let _ = stream.write_all(&answer);
            }
        });
        format!("http://{address}/v1/kv")
    }

    #[test]
    fn central_http_blob_reader_mints_then_streams_the_bytes() {
        let bytes: Vec<u8> = (0..70_000u32).map(|i| (i % 251) as u8).collect();
        let reference = blob_reference(&bytes);
        let url = serve_blob_door(bytes.clone(), "200 OK");
        let reader = HttpBlobReader::new(&url, "agent-jwt").unwrap();
        let address = Address::new("shared:team", "status", "big");
        let mut out = Vec::new();
        crate::blob_read::write_value(
            crate::blob::REF_ENVELOPE,
            &reference.encode(),
            &address,
            Some(&reader),
            &mut out,
        )
        .unwrap();
        assert_eq!(out, bytes);

        let refusing = serve_blob_door(bytes, "404 Not Found");
        let reader = HttpBlobReader::new(&refusing, "agent-jwt").unwrap();
        let error = match reader.open(&address, &reference) {
            Ok(_) => panic!("a refused capability opened a stream"),
            Err(error) => error,
        };
        assert_eq!(error.code(), "central_refused", "{error}");
    }

    /// A one-endpoint Hrana server that runs each batch's statements on a
    /// SqlitePrimary inside its transaction.
    fn serve(primary: Arc<SqlitePrimary>, bearer: &'static str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut length = 0usize;
                let mut authorized = false;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                        break;
                    }
                    let lower = line.to_ascii_lowercase();
                    if let Some(value) = lower.strip_prefix("content-length:") {
                        length = value.trim().parse().unwrap_or(0);
                    }
                    if line.trim_end() == format!("Authorization: Bearer {bearer}")
                        || lower.trim_end() == format!("authorization: bearer {bearer}")
                    {
                        authorized = true;
                    }
                }
                let mut body = vec![0u8; length];
                reader.read_exact(&mut body).unwrap();
                let (status, answer) = if authorized {
                    let request: Json = serde_json::from_slice(&body).unwrap();
                    ("200 OK", run_batch(&primary, &request))
                } else {
                    (
                        "401 Unauthorized",
                        json!({"error": {"code": "unauthorized"}}),
                    )
                };
                let text = answer.to_string();
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\n\
                     Content-Length: {}\r\nConnection: close\r\n\r\n{text}",
                    text.len()
                );
            }
        });
        format!("http://{address}")
    }

    fn run_batch(primary: &SqlitePrimary, request: &Json) -> Json {
        let steps = request["requests"][0]["batch"]["steps"].as_array().unwrap();
        let inner: Vec<Statement> = steps[1..steps.len() - 2]
            .iter()
            .map(|step| Statement {
                sql: step["stmt"]["sql"].as_str().unwrap().to_string(),
                args: step["stmt"]["args"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|arg| decode_value(arg).unwrap())
                    .collect(),
            })
            .collect();
        let count = inner.len();
        let result = match primary.transaction(&inner) {
            Ok(rows) => {
                let mut step_results = vec![json!({"cols": [], "rows": []})];
                for rows in rows {
                    step_results.push(json!({
                        "cols": rows.columns.iter().map(|name| json!({"name": name})).collect::<Vec<_>>(),
                        "rows": rows.rows.iter().map(|row| row.iter().map(encode_value).collect::<Vec<_>>()).collect::<Vec<_>>(),
                    }));
                }
                step_results.push(json!({"cols": [], "rows": []}));
                step_results.push(Json::Null);
                json!({"step_results": step_results, "step_errors": vec![Json::Null; count + 3]})
            }
            Err(error) => {
                let mut step_errors = vec![Json::Null; count + 3];
                step_errors[1] = json!({"message": error.to_string()});
                json!({"step_results": vec![Json::Null; count + 3], "step_errors": step_errors})
            }
        };
        json!({"results": [
            {"type": "ok", "response": {"type": "batch", "result": result}},
            {"type": "ok", "response": {"type": "close"}},
        ]})
    }

    #[test]
    fn central_http_cas_round_trips_through_the_pipeline() {
        let primary = Arc::new(SqlitePrimary::in_memory().unwrap());
        let url = serve(Arc::clone(&primary), "agent-jwt");
        let pipeline = HttpPipeline::new(&url, "agent-jwt").unwrap();
        let address = Address::new("shared:team", "status", "agent");
        let write = Intent {
            address: address.clone(),
            expected_version: 0,
            op: Op::Set(vec![0, 159, 146, 150]),
            idempotency: IdempotencyKey {
                writer: "drew".into(),
                host_incarnation: "host".into(),
                outbox_id: "1".into(),
            },
        };
        let receipt = cas(&pipeline, &write).unwrap();
        assert_eq!(receipt.outcome, Outcome::Committed { version: 1 });
        assert!(cas(&pipeline, &write).unwrap().replayed);
        let current = head(&pipeline, &address).unwrap().unwrap();
        assert_eq!(current.value, vec![0, 159, 146, 150]);
        assert_eq!(head(primary.as_ref(), &address).unwrap(), Some(current));

        let stranger = HttpPipeline::new(&url, "wrong").unwrap();
        assert_eq!(
            head(&stranger, &address).unwrap_err().code(),
            "unauthorized"
        );
    }

    #[test]
    fn central_http_an_unreachable_endpoint_is_retryable() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        drop(listener);
        let pipeline = HttpPipeline::new(&url, "token").unwrap();
        let error = pipeline.transaction(&[]).unwrap_err();
        assert!(error.is_retryable(), "{error}");
    }
}
