//! Muse JSONL conversation-export adapter (#14162).
//!
//! Mirrors `adapt_muse` in the original `atlas-ingest.py`: read sorted
//! `<root>/agent-*/sessions/*.jsonl` files, accept both flat messages and
//! text-part messages, then hand each session to the shared staging core.

use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::Connection;
use serde_json::Value;

use crate::error::{Error, Result};
use crate::session_stage::{SessionMessage, SessionRecord, SessionStageReport, stage_session};

/// Parse one Muse JSONL session. Malformed lines and unsupported JSON shapes
/// are ignored, matching the export adapter's best-effort line handling.
/// Bytes are decoded with replacement characters, as Python's
/// `open(..., errors="replace")` does.
pub fn parse_muse_session(path: &Path, bytes: &[u8]) -> Option<SessionRecord> {
    let mut created_at = None;
    let mut messages = Vec::new();
    let decoded = String::from_utf8_lossy(bytes);

    for line in decoded.lines() {
        let Ok(record) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let Some(record) = record.as_object() else {
            continue;
        };
        if record.get("type").and_then(Value::as_str) == Some("session_header") {
            // A later header replaces an earlier value; a missing or invalid
            // timestamp therefore clears it, as in the Python adapter.
            created_at = record.get("created_at").and_then(value_as_string);
            continue;
        }

        let Some(item) = record.get("item").and_then(Value::as_object) else {
            continue;
        };
        let role = item
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let timestamp = record.get("created_at").and_then(value_as_string);

        match item.get("type").and_then(Value::as_str) {
            Some("message") => {
                // Python's truthiness check rejects empty strings; requiring a
                // string also safely skips malformed values it would not handle.
                if let Some(text) = item.get("text").and_then(Value::as_str)
                    && !text.is_empty()
                {
                    messages.push(SessionMessage {
                        role,
                        timestamp,
                        text: text.to_string(),
                    });
                }
            }
            Some("message_parts") => {
                let parts = item.get("parts").and_then(Value::as_array);
                let text = parts
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_object)
                    .filter(|part| part.get("type").and_then(Value::as_str) == Some("text"))
                    .filter_map(|part| part.get("text").and_then(Value::as_str))
                    .collect::<Vec<_>>()
                    .join("\n");
                if !text.trim().is_empty() {
                    messages.push(SessionMessage {
                        role,
                        timestamp,
                        text,
                    });
                }
            }
            _ => {}
        }
    }

    if messages.is_empty() {
        return None;
    }
    Some(SessionRecord {
        client: "muse".into(),
        session_id: path.file_stem()?.to_string_lossy().into_owned(),
        title: None,
        created_at,
        source_path: path.to_string_lossy().into_owned(),
        messages,
    })
}

fn value_as_string(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

/// Ingest all Muse session files under a root in lexical path order.
/// A nonexistent root is an empty input, matching Python's glob behavior;
/// other directory and file read failures are reported. Each session stages
/// in its own transaction, so an error may leave earlier sessions committed.
pub fn ingest_muse_root(
    conn: &mut Connection,
    unit_id: &str,
    root: &Path,
    now: &str,
) -> Result<SessionStageReport> {
    let mut files = discover_muse_files(root)?;
    files.sort();

    let mut total = SessionStageReport::default();
    for path in files {
        let bytes = fs::read(&path).map_err(|error| {
            Error::backend(format!("Muse adapter: reading {}: {error}", path.display()))
        })?;
        if let Some(session) = parse_muse_session(&path, &bytes) {
            add_report(&mut total, stage_session(conn, unit_id, &session, now)?);
        }
    }
    Ok(total)
}

fn discover_muse_files(root: &Path) -> Result<Vec<PathBuf>> {
    let agents = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(Error::backend(format!(
                "Muse adapter: listing {}: {error}",
                root.display()
            )));
        }
    };

    let mut files = Vec::new();
    for agent in agents {
        let agent = agent.map_err(|error| {
            Error::backend(format!(
                "Muse adapter: reading entry in {}: {error}",
                root.display()
            ))
        })?;
        if !agent.file_name().to_string_lossy().starts_with("agent-") {
            continue;
        }
        let sessions = agent.path().join("sessions");
        let entries = match fs::read_dir(&sessions) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(Error::backend(format!(
                    "Muse adapter: listing {}: {error}",
                    sessions.display()
                )));
            }
        };
        for entry in entries {
            let entry = entry.map_err(|error| {
                Error::backend(format!(
                    "Muse adapter: reading entry in {}: {error}",
                    sessions.display()
                ))
            })?;
            let path = entry.path();
            if path
                .extension()
                .is_some_and(|extension| extension == "jsonl")
                && path.is_file()
            {
                files.push(path);
            }
        }
    }
    Ok(files)
}

fn add_report(total: &mut SessionStageReport, next: SessionStageReport) {
    total.messages_staged += next.messages_staged;
    total.messages_skipped += next.messages_skipped;
    total.messages_already_staged += next.messages_already_staged;
    total.chunks_staged += next.chunks_staged;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stage::{list_staged_items, provision_staging_db};

    fn temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "atlas-muse-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        fs::create_dir_all(&dir).expect("temporary directory");
        dir
    }

    /// Oracle: this golden JSONL is transcribed from `adapt_muse` and checks
    /// both record shapes, malformed-line tolerance, header replacement,
    /// attachment filtering, and original per-line order.
    #[test]
    fn parses_python_muse_jsonl_contract() {
        let path = Path::new("/exports/agent-a/sessions/session-1.jsonl");
        let input = concat!(
            "{\"type\":\"session_header\",\"created_at\":\"first\"}\n",
            "not-json\n",
            "{\"type\":\"item\",\"created_at\":\"t1\",\"item\":{\"type\":\"message\",\"role\":\"user\",\"text\":\"hello\"}}\n",
            "{\"created_at\":\"t2\",\"item\":{\"type\":\"message_parts\",\"role\":\"assistant\",\"parts\":[{\"type\":\"text\",\"text\":\"one\"},{\"type\":\"file_ref\",\"text\":\"ignored\"},null,{\"type\":\"text\",\"text\":\"two\"}]}}\n",
            "{\"item\":{\"type\":\"message_parts\",\"role\":\"user\",\"parts\":[{\"type\":\"file_ref\"}]}}\n",
            "{\"type\":\"session_header\",\"created_at\":\"last\"}\n",
            "{\"type\":\"session_header\"}\n"
        );
        let session = parse_muse_session(path, input.as_bytes()).expect("parsed session");
        assert_eq!(session.client, "muse");
        assert_eq!(session.session_id, "session-1");
        assert_eq!(session.title, None);
        assert_eq!(
            session.created_at, None,
            "last header without time clears it"
        );
        assert_eq!(session.source_path, path.to_string_lossy());
        assert_eq!(
            session.messages,
            vec![
                SessionMessage {
                    role: "user".into(),
                    timestamp: Some("t1".into()),
                    text: "hello".into(),
                },
                SessionMessage {
                    role: "assistant".into(),
                    timestamp: Some("t2".into()),
                    text: "one\ntwo".into(),
                }
            ]
        );
    }

    /// Oracle: only the Python glob shape is discovered and lexical file order
    /// determines stable sequence IDs; deeper, wrong-extension and unrelated
    /// files are excluded.
    #[test]
    fn walks_sorted_muse_files_and_stages_with_python_sequence_identity() {
        let dir = temp_dir("walk");
        let first = dir.join("agent-a/sessions/a.jsonl");
        let second = dir.join("agent-b/sessions/b.jsonl");
        fs::create_dir_all(first.parent().unwrap()).unwrap();
        fs::create_dir_all(second.parent().unwrap()).unwrap();
        fs::write(
            &second,
            "{\"item\":{\"type\":\"message\",\"role\":\"user\",\"text\":\"second\"}}\n",
        )
        .unwrap();
        fs::write(
            &first,
            concat!(
                "{\"item\":{\"type\":\"message\",\"role\":\"developer\",\"text\":\"skip role\"}}\n",
                "{\"item\":{\"type\":\"message\",\"role\":\"user\",\"text\":\"first\"}}\n"
            ),
        )
        .unwrap();
        fs::create_dir_all(dir.join("other/sessions")).unwrap();
        fs::write(
            dir.join("other/sessions/ignored.jsonl"),
            "{\"item\":{\"type\":\"message\",\"role\":\"user\",\"text\":\"ignored\"}}\n",
        )
        .unwrap();
        fs::write(
            dir.join("agent-a/sessions/not-json.txt"),
            "{\"item\":{\"type\":\"message\",\"role\":\"user\",\"text\":\"ignored\"}}\n",
        )
        .unwrap();
        fs::create_dir_all(dir.join("agent-a/sessions/deeper")).unwrap();
        fs::write(
            dir.join("agent-a/sessions/deeper/ignored.jsonl"),
            "{\"item\":{\"type\":\"message\",\"role\":\"user\",\"text\":\"ignored\"}}\n",
        )
        .unwrap();

        let db_dir = temp_dir("db");
        let db = db_dir.join("stage.db");
        provision_staging_db(&db).unwrap();
        let mut conn = Connection::open(&db).unwrap();
        let report = ingest_muse_root(&mut conn, "unit-1", &dir, "ingested").unwrap();
        assert_eq!(report.messages_staged, 2);
        assert_eq!(report.messages_skipped, 1);
        assert_eq!(report.chunks_staged, 2);
        let items = list_staged_items(&conn, Some("unit-1")).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].text, "first");
        assert_eq!(items[1].text, "second");
        let first_meta: Value = serde_json::from_str(items[0].meta.as_deref().unwrap()).unwrap();
        assert_eq!(first_meta["sequence"], 1, "role filtering is downstream");
        assert_eq!(first_meta["session_id"], "a");

        let replay = ingest_muse_root(&mut conn, "unit-1", &dir, "later").unwrap();
        assert_eq!(replay.messages_already_staged, 2);
        assert_eq!(replay.messages_staged, 0);
    }

    #[test]
    fn missing_root_is_an_empty_input_and_invalid_utf8_is_replaced() {
        let missing = temp_dir("missing").join("no-root");
        let mut conn = Connection::open_in_memory().unwrap();
        let report = ingest_muse_root(&mut conn, "unit-1", &missing, "now").unwrap();
        assert_eq!(report, SessionStageReport::default());

        let session = parse_muse_session(
            Path::new("session.jsonl"),
            b"{\"item\":{\"type\":\"message\",\"role\":\"user\",\"text\":\"bad\xffbyte\"}}\n",
        )
        .unwrap();
        assert_eq!(session.messages[0].text, "bad\u{fffd}byte");
    }
}
