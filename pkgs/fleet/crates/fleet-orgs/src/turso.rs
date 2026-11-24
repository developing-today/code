//! The Turso store over its HTTP pipeline (`POST /v2/pipeline`), one
//! execute per request.

use crate::sql::{Rows, Sql};
use serde_json::{Value, json};

pub struct Turso {
    url: String,
    token: String,
    agent: ureq::Agent,
}

impl Turso {
    pub fn new(url: &str, token: &str) -> Turso {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(std::time::Duration::from_secs(60)))
            .http_status_as_error(false)
            .tls_config(
                ureq::tls::TlsConfig::builder()
                    .root_certs(ureq::tls::RootCerts::PlatformVerifier)
                    .build(),
            )
            .build()
            .into();
        Turso {
            url: url.trim_end_matches('/').to_string(),
            token: token.to_string(),
            agent,
        }
    }
}

/// JSON argument → Turso typed argument.
pub fn arg_json(v: &Value) -> Value {
    match v {
        Value::Null => json!({"type": "null"}),
        Value::Bool(b) => json!({"type": "integer", "value": if *b { "1" } else { "0" }}),
        Value::Number(n) if n.is_i64() => json!({"type": "integer", "value": n.to_string()}),
        Value::Number(n) => json!({"type": "float", "value": n.as_f64().unwrap_or(0.0)}),
        Value::String(s) => json!({"type": "text", "value": s}),
        other => json!({"type": "text", "value": other.to_string()}),
    }
}

pub fn pipeline_body(sql: &str, args: &[Value]) -> Value {
    json!({
        "requests": [
            {"type": "execute", "stmt": {
                "sql": sql,
                "args": args.iter().map(arg_json).collect::<Vec<_>>(),
            }},
            {"type": "close"}
        ]
    })
}

fn cell_to_json(cell: &Value) -> Value {
    let raw = cell.get("value");
    match cell.get("type").and_then(Value::as_str).unwrap_or("null") {
        "text" => raw
            .and_then(Value::as_str)
            .map_or(Value::Null, |s| json!(s)),
        "integer" => raw
            .and_then(Value::as_str)
            .and_then(|s| s.parse::<i64>().ok())
            .map_or(Value::Null, |n| json!(n)),
        "float" => raw
            .and_then(Value::as_f64)
            .and_then(serde_json::Number::from_f64)
            .map_or(Value::Null, Value::Number),
        _ => Value::Null,
    }
}

/// The first execute result of a pipeline response. A per-request error
/// or a missing `ok` result is a failure, never an empty result set.
pub fn parse_rows(body: &Value) -> Result<Rows, String> {
    if let Some(err) = body.get("error") {
        return Err(format!("Turso request failed: {err}"));
    }
    let first = body
        .get("results")
        .and_then(Value::as_array)
        .and_then(|r| r.first())
        .ok_or("Turso answered without results")?;
    if first.get("type").and_then(Value::as_str) != Some("ok") {
        return Err(format!("Turso statement failed: {}", first));
    }
    let result = first
        .pointer("/response/result")
        .ok_or("Turso execute result missing")?;
    let rows = result
        .get("rows")
        .and_then(Value::as_array)
        .ok_or("Turso execute result has no rows array")?
        .iter()
        .map(|row| {
            row.as_array()
                .map(|cells| cells.iter().map(cell_to_json).collect())
                .unwrap_or_default()
        })
        .collect();
    Ok(Rows { rows })
}

impl Sql for Turso {
    fn execute(&self, sql: &str, args: &[Value]) -> Result<Rows, String> {
        let url = format!("{}/v2/pipeline", self.url);
        let body = serde_json::to_string(&pipeline_body(sql, args)).map_err(|e| e.to_string())?;
        let mut resp = self
            .agent
            .post(&url)
            .header("Authorization", &format!("Bearer {}", self.token))
            .header("Content-Type", "application/json")
            .send(body.as_str())
            .map_err(|e| format!("Turso store unreachable: {e}"))?;
        let status = resp.status().as_u16();
        let text = resp
            .body_mut()
            .read_to_string()
            .map_err(|e| format!("cannot read Turso response: {e}"))?;
        if !(200..300).contains(&status) {
            let snippet: String = text.chars().take(300).collect();
            return Err(format!("Turso store HTTP {status}: {snippet}"));
        }
        let parsed: Value =
            serde_json::from_str(&text).map_err(|e| format!("Turso non-JSON body: {e}"))?;
        parse_rows(&parsed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_args() {
        assert_eq!(arg_json(&json!("x")), json!({"type": "text", "value": "x"}));
        assert_eq!(
            arg_json(&json!(7)),
            json!({"type": "integer", "value": "7"})
        );
        assert_eq!(arg_json(&Value::Null), json!({"type": "null"}));
    }

    #[test]
    fn parses_typed_rows() {
        let body = json!({"results": [
            {"type": "ok", "response": {"type": "execute", "result": {
                "cols": [], "rows": [[
                    {"type": "text", "value": "acme"},
                    {"type": "integer", "value": "42"},
                    {"type": "null"}
                ]]
            }}},
            {"type": "ok", "response": {"type": "close"}}
        ]});
        let rows = parse_rows(&body).unwrap();
        assert_eq!(rows.rows, vec![vec![json!("acme"), json!(42), Value::Null]]);
    }

    #[test]
    fn failures_are_errors_not_empty_results() {
        assert!(parse_rows(&json!({"error": "boom"})).is_err());
        let per_request =
            json!({"results": [{"type": "error", "error": {"message": "no such table"}}]});
        assert!(parse_rows(&per_request).is_err());
    }
}
