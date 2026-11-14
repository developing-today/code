//! needs-drew throat emission — PM-stack notifications Phase 1
//! (developing-today/code#8568).
//!
//! When a queue task blocks with reason-class `escalated-to-user`
//! and carries a needs-drew payload, the block path raises one
//! alert into the PM aggregator's alert throat so Drew's phone
//! hears about it. The throat dedupes by (source, key), so the
//! raise is idempotent: the CLI half and the broker half may both
//! post for the same block, and a retry is safe.
//!
//! Wire contract (fixed; the aggregator lane owns the receiving
//! side — do not deviate):
//!
//!   POST http://127.0.0.1:4870/api/alerts
//!     (env override: FLEET_ALERTS_URL, the full URL)
//!   Authorization: Bearer <token>
//!     (token = the Bitwarden secret named by
//!     [`ALERTS_INTAKE_TOKEN_SECRET`], resolved by name through
//!     fleet-secrets — never read from a file here)
//!   {"source": "fleet-queue",
//!    "key": "needs-drew:<ledgerId>",
//!    "severity": "warning",
//!    "title": <question>,
//!    "body": <context or blocker text>,
//!    "needsDrew": {"ledgerId": ..., "question": ...,
//!                  "context": ...,            // when present
//!                  "options": [{"id": ..., "label": ...}],  // non-empty
//!                  "links": [...]}}          // when non-empty
//!
//! The transport is a minimal HTTP/1.1 POST over a plain
//! `TcpStream`, in the style of client.rs's direct-broker leg:
//! the throat is a loopback plain-http endpoint, so no TLS or
//! HTTP-client dependency is needed for this one call.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use serde_json::{Value, json};

/// The alerts-intake token's name in Bitwarden Secrets Manager,
/// resolved by name through fleet-secrets at post time. The value
/// itself is never written to a file or a log line.
pub const ALERTS_INTAKE_TOKEN_SECRET: &str = "pm-aggregator.alerts-intake-token";

/// The default throat endpoint; `FLEET_ALERTS_URL` overrides it
/// (the full URL, path included).
pub const DEFAULT_ALERTS_URL: &str = "http://127.0.0.1:4870/api/alerts";

/// One answer option offered to Drew inside the needs-drew push.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NeedsDrewOption {
    pub id: String,
    pub label: String,
}

/// The block-side needs-drew payload: everything the throat body
/// is built from. `ledger_id` is the PM ledger item the question
/// belongs to (an opaque id string, as the ledger names it);
/// `blocker` is the /block call's blocker text, used as the alert
/// body when no richer `context` was supplied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockNeedsDrew {
    pub ledger_id: String,
    pub question: String,
    pub context: Option<String>,
    pub options: Vec<NeedsDrewOption>,
    pub links: Vec<String>,
    pub blocker: String,
}

/// Build the exact throat POST body for one block. Pure — the
/// unit tests pin the shape (the aggregator lane parses it).
pub fn build_throat_body(block: &BlockNeedsDrew) -> Value {
    let mut needs_drew = json!({
        "ledgerId": block.ledger_id,
        "question": block.question,
    });
    if let Some(context) = &block.context {
        needs_drew["context"] = json!(context);
    }
    if !block.options.is_empty() {
        needs_drew["options"] = Value::Array(
            block
                .options
                .iter()
                .map(|o| json!({"id": o.id, "label": o.label}))
                .collect(),
        );
    }
    if !block.links.is_empty() {
        needs_drew["links"] = json!(block.links);
    }
    // The alert body is the context when the caller supplied one,
    // else the block's blocker text (the contract's wording).
    let body = block.context.as_deref().unwrap_or(&block.blocker);
    json!({
        "source": "fleet-queue",
        "key": format!("needs-drew:{}", block.ledger_id),
        "severity": "warning",
        "title": block.question,
        "body": body,
        "needsDrew": needs_drew,
    })
}

/// The throat URL for this process: `FLEET_ALERTS_URL` when set
/// and non-empty, else the loopback default.
pub fn alerts_url() -> String {
    alerts_url_from_env(std::env::var("FLEET_ALERTS_URL").ok().as_deref())
}

fn alerts_url_from_env(value: Option<&str>) -> String {
    match value {
        Some(u) if !u.trim().is_empty() => u.trim().to_string(),
        _ => DEFAULT_ALERTS_URL.to_string(),
    }
}

/// Split an `http://host:port[/path]` URL into (host:port, path).
/// Plain http only — the throat is a loopback endpoint.
fn parse_http_url(url: &str) -> Result<(String, String), String> {
    let rest = url
        .strip_prefix("http://")
        .ok_or_else(|| format!("alerts URL must be http://host:port[/path], got {url:?}"))?;
    let (hostport, path) = match rest.split_once('/') {
        Some((h, p)) => (h, format!("/{p}")),
        None => (rest, "/".to_string()),
    };
    if hostport.is_empty() {
        return Err(format!("alerts URL has no host: {url:?}"));
    }
    Ok((hostport.to_string(), path))
}

/// One minimal HTTP/1.1 JSON POST (client.rs transport style):
/// Content-Length, Connection: close, status line parsed from the
/// reply. Returns the parsed JSON body on a 2xx; any transport
/// failure or non-2xx status is an Err naming the status only —
/// never the token, never the payload.
fn post_json(url: &str, token: &str, body: &Value) -> Result<Value, String> {
    let (hostport, path) = parse_http_url(url)?;
    let payload = serde_json::to_vec(body).map_err(|e| format!("alerts body encode: {e}"))?;
    let mut s = TcpStream::connect(&hostport)
        .map_err(|e| format!("cannot connect to alerts throat at {hostport}: {e}"))?;
    s.set_read_timeout(Some(Duration::from_secs(30))).ok();
    s.set_write_timeout(Some(Duration::from_secs(10))).ok();
    let head = format!(
        "POST {path} HTTP/1.1\r\nHost: {hostport}\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        payload.len()
    );
    s.write_all(head.as_bytes())
        .and_then(|_| s.write_all(&payload))
        .map_err(|e| format!("alerts throat write failed: {e}"))?;
    let mut raw = Vec::new();
    s.read_to_end(&mut raw)
        .map_err(|e| format!("alerts throat read failed: {e}"))?;
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or("alerts throat bad reply")?;
    let head_txt = String::from_utf8_lossy(&raw[..split]).to_string();
    let status: u64 = head_txt
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .unwrap_or(0);
    if !(200..300).contains(&status) {
        return Err(format!("alerts throat HTTP {status}"));
    }
    let text = String::from_utf8_lossy(&raw[split + 4..]).to_string();
    if text.trim().is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_str(&text).map_err(|e| format!("alerts throat returned non-JSON: {e}"))
}

/// Raise one needs-drew alert into the throat for `nd`, with the
/// intake `token` resolved by the caller (by name, through
/// fleet-secrets). Returns the response's `answerUrl` when the
/// throat includes one (top-level, or nested under `alert`);
/// an empty string when the raise succeeded without one. The
/// raise is idempotent at the throat ((source, key) dedupe), so
/// posting twice for one block is safe.
pub fn post_needs_drew(token: &str, nd: &BlockNeedsDrew) -> Result<String, String> {
    post_needs_drew_to(&alerts_url(), token, nd)
}

/// [`post_needs_drew`] against an explicit URL (tests, overrides).
pub fn post_needs_drew_to(url: &str, token: &str, nd: &BlockNeedsDrew) -> Result<String, String> {
    let resp = post_json(url, token, &build_throat_body(nd))?;
    let answer = resp
        .get("answerUrl")
        .or_else(|| resp.get("alert").and_then(|a| a.get("answerUrl")))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    Ok(answer)
}

/// Parse the snake_case `needs_drew` object the /block body
/// carries (the shape cmd_block sends and the broker receives):
/// {ledger_id, question, context?, options?, links?}, with
/// `blocker` taken from the block itself. Returns None when the
/// object is absent or lacks a usable ledger_id / question — a
/// malformed payload must never fail or fake a block.
pub fn from_block_request(req: &Value, blocker: &str) -> Option<BlockNeedsDrew> {
    let nd = req.get("needs_drew").filter(|v| v.is_object())?;
    let ledger_id = match nd.get("ledger_id") {
        Some(Value::String(s)) if !s.trim().is_empty() => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => return None,
    };
    let question = nd
        .get("question")
        .and_then(|q| q.as_str())
        .filter(|q| !q.trim().is_empty())?
        .to_string();
    let context = nd
        .get("context")
        .and_then(|c| c.as_str())
        .filter(|c| !c.is_empty())
        .map(|c| c.to_string());
    let options = nd
        .get("options")
        .and_then(|o| o.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|o| {
                    Some(NeedsDrewOption {
                        id: o.get("id")?.as_str()?.to_string(),
                        label: o.get("label")?.as_str()?.to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let links = nd
        .get("links")
        .and_then(|l| l.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|l| l.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();
    Some(BlockNeedsDrew {
        ledger_id,
        question,
        context,
        options,
        links,
        blocker: blocker.to_string(),
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn full_block() -> BlockNeedsDrew {
        BlockNeedsDrew {
            ledger_id: "ledger-42".to_string(),
            question: "Ship the red or the blue build?".to_string(),
            context: Some("Both pass CI; blue is 2 days older.".to_string()),
            options: vec![
                NeedsDrewOption {
                    id: "red".to_string(),
                    label: "Ship red".to_string(),
                },
                NeedsDrewOption {
                    id: "blue".to_string(),
                    label: "Ship blue".to_string(),
                },
            ],
            links: vec!["https://example.invalid/pr/1".to_string()],
            blocker: "escalated: awaiting Drew's pick".to_string(),
        }
    }

    #[test]
    fn throat_body_matches_the_fixed_contract_shape() {
        let body = build_throat_body(&full_block());
        assert_eq!(
            body,
            json!({
                "source": "fleet-queue",
                "key": "needs-drew:ledger-42",
                "severity": "warning",
                "title": "Ship the red or the blue build?",
                "body": "Both pass CI; blue is 2 days older.",
                "needsDrew": {
                    "ledgerId": "ledger-42",
                    "question": "Ship the red or the blue build?",
                    "context": "Both pass CI; blue is 2 days older.",
                    "options": [
                        {"id": "red", "label": "Ship red"},
                        {"id": "blue", "label": "Ship blue"},
                    ],
                    "links": ["https://example.invalid/pr/1"],
                },
            })
        );
    }

    #[test]
    fn throat_body_omits_absent_optionals_and_falls_back_to_blocker() {
        let block = BlockNeedsDrew {
            ledger_id: "99".to_string(),
            question: "Proceed?".to_string(),
            context: None,
            options: vec![],
            links: vec![],
            blocker: "blocked on Drew".to_string(),
        };
        let body = build_throat_body(&block);
        assert_eq!(body["key"], json!("needs-drew:99"));
        assert_eq!(body["title"], json!("Proceed?"));
        // No context: the blocker text is the alert body.
        assert_eq!(body["body"], json!("blocked on Drew"));
        let nd = &body["needsDrew"];
        assert_eq!(nd["ledgerId"], json!("99"));
        assert_eq!(nd["question"], json!("Proceed?"));
        assert!(nd.get("context").is_none());
        assert!(nd.get("options").is_none());
        assert!(nd.get("links").is_none());
    }

    #[test]
    fn alerts_url_env_override_wins_and_blank_falls_back() {
        assert_eq!(alerts_url_from_env(None), DEFAULT_ALERTS_URL);
        assert_eq!(alerts_url_from_env(Some("  ")), DEFAULT_ALERTS_URL);
        assert_eq!(
            alerts_url_from_env(Some("http://127.0.0.1:9999/x")),
            "http://127.0.0.1:9999/x"
        );
    }

    #[test]
    fn parse_http_url_splits_hostport_and_path() {
        assert_eq!(
            parse_http_url(DEFAULT_ALERTS_URL).unwrap(),
            ("127.0.0.1:4870".to_string(), "/api/alerts".to_string())
        );
        assert_eq!(
            parse_http_url("http://127.0.0.1:4870").unwrap(),
            ("127.0.0.1:4870".to_string(), "/".to_string())
        );
        assert!(parse_http_url("https://127.0.0.1:4870/api/alerts").is_err());
        assert!(parse_http_url("http:///api/alerts").is_err());
    }

    #[test]
    fn from_block_request_round_trips_the_snake_case_shape() {
        let req = json!({
            "blocker": "waiting on Drew",
            "needs_drew": {
                "ledger_id": "ledger-7",
                "question": "Which scope?",
                "context": "ctx",
                "options": [{"id": "a", "label": "A"}],
                "links": ["https://example.invalid/x"],
            },
        });
        let nd = from_block_request(&req, "waiting on Drew").unwrap();
        assert_eq!(nd.ledger_id, "ledger-7");
        assert_eq!(nd.question, "Which scope?");
        assert_eq!(nd.context.as_deref(), Some("ctx"));
        assert_eq!(nd.options.len(), 1);
        assert_eq!(nd.links.len(), 1);
        assert_eq!(nd.blocker, "waiting on Drew");
        // And the throat body built from it carries the same key.
        assert_eq!(build_throat_body(&nd)["key"], json!("needs-drew:ledger-7"));
    }

    #[test]
    fn from_block_request_accepts_a_numeric_ledger_id() {
        let req = json!({"needs_drew": {"ledger_id": 123, "question": "Q?"}});
        let nd = from_block_request(&req, "b").unwrap();
        assert_eq!(nd.ledger_id, "123");
    }

    #[test]
    fn from_block_request_rejects_absent_or_incomplete_payloads() {
        assert!(from_block_request(&json!({}), "b").is_none());
        assert!(from_block_request(&json!({"needs_drew": {"question": "Q?"}}), "b").is_none());
        assert!(from_block_request(&json!({"needs_drew": {"ledger_id": "x"}}), "b").is_none());
        assert!(
            from_block_request(
                &json!({"needs_drew": {"ledger_id": " ", "question": "Q?"}}),
                "b"
            )
            .is_none()
        );
    }
}
