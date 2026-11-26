//! Shared staging core for conversation-export adapters (#14162).
//!
//! This mirrors the stable core in `atlas-ingest.py`: session IDs are
//! `<client>:<session-id>`, message IDs hash the session ID, sequence and
//! content hash, only user-facing roles are kept, and message text is split
//! into 3,500-character chunks at paragraph boundaries when possible. Each
//! retained message is one `atlas_stage` v1 item; its metadata carries the
//! source session and the stable Python-compatible message/content hashes.

use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::error::{Error, Result};
use crate::shards::Tier;
use crate::stage::check_stage_schema;

pub const SESSION_CHUNK_CHARS: usize = 3_500;
const SESSION_MESSAGE_KIND: &str = "conversation-message";
const KEEP_ROLES: [&str; 3] = ["user", "assistant", "human"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionMessage {
    pub role: String,
    pub timestamp: Option<String>,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRecord {
    pub client: String,
    pub session_id: String,
    pub title: Option<String>,
    pub created_at: Option<String>,
    pub source_path: String,
    pub messages: Vec<SessionMessage>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SessionStageReport {
    pub messages_staged: usize,
    pub messages_skipped: usize,
    pub messages_already_staged: usize,
    pub chunks_staged: usize,
}

#[derive(Serialize)]
struct MessageMeta<'a> {
    client: &'a str,
    session_id: &'a str,
    session_title: &'a Option<String>,
    session_created_at: &'a Option<String>,
    session_source: &'a str,
    message_id: String,
    sequence: usize,
    role: &'a str,
    message_timestamp: &'a Option<String>,
    content_hash: String,
    chunks: usize,
}

/// Store a session's retained messages in an existing `atlas_stage` v1
/// database. One item represents one message, so role and timestamp remain
/// attached to the original text while the downstream embedder sees bounded
/// chunks. Replaying an unchanged session is idempotent by stable message ID.
pub fn stage_session(
    conn: &mut Connection,
    unit_id: &str,
    session: &SessionRecord,
    now: &str,
) -> Result<SessionStageReport> {
    check_stage_schema(conn)?;
    let mut report = SessionStageReport::default();
    let session_key = format!("{}:{}", session.client, session.session_id);
    let tx = conn
        .transaction()
        .map_err(|e| Error::backend(format!("session staging: begin transaction: {e}")))?;

    for (sequence, message) in session.messages.iter().enumerate() {
        if !KEEP_ROLES.contains(&message.role.as_str()) {
            report.messages_skipped += 1;
            continue;
        }
        let text = message.text.trim();
        if text.is_empty() {
            report.messages_skipped += 1;
            continue;
        }

        let content_hash = sha256_hex(text);
        let message_id = sha256_hex(&format!("{session_key}|{sequence}|{content_hash}"));
        let source = format!("session-store:{session_key}/message/{message_id}");
        let exists: Option<i64> = tx
            .query_row(
                "SELECT id FROM items WHERE unit_id = ?1 AND kind = ?2 AND source = ?3 LIMIT 1",
                params![unit_id, SESSION_MESSAGE_KIND, source],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| Error::backend(format!("session staging: checking message: {e}")))?;
        if exists.is_some() {
            report.messages_already_staged += 1;
            continue;
        }

        let chunks = chunk_session_text(text);
        let meta = serde_json::to_string(&MessageMeta {
            client: &session.client,
            session_id: &session.session_id,
            session_title: &session.title,
            session_created_at: &session.created_at,
            session_source: &session.source_path,
            message_id,
            sequence,
            role: &message.role,
            message_timestamp: &message.timestamp,
            content_hash,
            chunks: chunks.len(),
        })
        .map_err(|e| Error::backend(format!("session staging: encoding metadata: {e}")))?;
        tx.execute(
            "INSERT INTO items (unit_id, kind, source, text, sensitivity, created_at, meta) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                unit_id,
                SESSION_MESSAGE_KIND,
                source,
                text,
                Tier::Internal.name(),
                now,
                meta,
            ],
        )
        .map_err(|e| Error::backend(format!("session staging: inserting message: {e}")))?;
        let item_id = tx.last_insert_rowid();
        for (index, chunk) in chunks.iter().enumerate() {
            tx.execute(
                "INSERT INTO chunks_raw (item_id, idx, text, content_hash, created_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![item_id, index as i64, chunk, sha256_hex(chunk), now],
            )
            .map_err(|e| {
                Error::backend(format!("session staging: inserting chunk {index}: {e}"))
            })?;
        }
        report.messages_staged += 1;
        report.chunks_staged += chunks.len();
    }

    tx.commit()
        .map_err(|e| Error::backend(format!("session staging: commit: {e}")))?;
    Ok(report)
}

/// Python-compatible splitter: prefer the last paragraph break within the
/// 3,500-character window if it is in the latter half; skip the separator
/// newlines after each break, matching `atlas-ingest.py`'s `chunk_text`.
pub fn chunk_session_text(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= SESSION_CHUNK_CHARS {
        return vec![text.to_string()];
    }
    let mut out = Vec::new();
    let mut start = 0usize;
    while chars.len() - start > SESSION_CHUNK_CHARS {
        let limit = start + SESSION_CHUNK_CHARS;
        let cut = (start..limit.saturating_sub(1))
            .rev()
            .find(|&index| chars[index] == '\n' && chars[index + 1] == '\n')
            .filter(|&index| index >= start + SESSION_CHUNK_CHARS / 2)
            .unwrap_or(limit);
        out.push(chars[start..cut].iter().collect());
        start = cut;
        while start < chars.len() && chars[start] == '\n' {
            start += 1;
        }
    }
    if start < chars.len() {
        out.push(chars[start..].iter().collect());
    }
    out
}

fn sha256_hex(text: &str) -> String {
    let digest = Sha256::digest(text.as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stage::{list_staged_items, provision_staging_db};
    use rusqlite::params;

    fn session(messages: Vec<SessionMessage>) -> SessionRecord {
        SessionRecord {
            client: "muse".into(),
            session_id: "session-1".into(),
            title: Some("test conversation".into()),
            created_at: Some("2026-11-25T00:00:00Z".into()),
            source_path: "/exports/session.jsonl".into(),
            messages,
        }
    }

    fn database() -> (std::path::PathBuf, Connection) {
        let dir = std::env::temp_dir().join(format!(
            "atlas-session-stage-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("temp directory");
        let path = dir.join("stage.db");
        provision_staging_db(&path).expect("provision v1 staging DB");
        let conn = Connection::open(path).expect("open staging DB");
        (dir, conn)
    }

    /// Oracle: read-back rows from the actual v1 staging schema must preserve
    /// the Python adapter's session/message identity contract and role filter.
    #[test]
    fn stages_kept_messages_with_python_compatible_identity_and_metadata() {
        let (_dir, mut conn) = database();
        let input = session(vec![
            SessionMessage {
                role: "user".into(),
                timestamp: Some("t1".into()),
                text: " hello ".into(),
            },
            SessionMessage {
                role: "developer".into(),
                timestamp: None,
                text: "private trace".into(),
            },
            SessionMessage {
                role: "assistant".into(),
                timestamp: Some("t3".into()),
                text: "world".into(),
            },
            SessionMessage {
                role: "human".into(),
                timestamp: None,
                text: "   ".into(),
            },
        ]);
        let report = stage_session(&mut conn, "unit-1", &input, "ingested").expect("stage");
        assert_eq!(report.messages_staged, 2);
        assert_eq!(report.messages_skipped, 2);
        assert_eq!(report.chunks_staged, 2);
        let items = list_staged_items(&conn, Some("unit-1")).expect("items");
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].text, "hello");
        assert_eq!(items[1].text, "world");
        let meta: serde_json::Value =
            serde_json::from_str(items[0].meta.as_deref().unwrap()).unwrap();
        assert_eq!(meta["client"], "muse");
        assert_eq!(meta["session_id"], "session-1");
        assert_eq!(meta["sequence"], 0);
        assert_eq!(
            meta["message_id"],
            "96706cad409bc86141f30578b791a37efa516ff388db913f74a725cbbead2da1"
        );
        assert_eq!(
            meta["content_hash"],
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
        let chunk: (String, String) = conn
            .query_row(
                "SELECT text, content_hash FROM chunks_raw WHERE item_id = ?1",
                params![items[0].id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("chunk row");
        assert_eq!(chunk.0, "hello");
        assert_eq!(
            chunk.1,
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }

    /// Oracle: these golden boundaries and contents are copied from the
    /// original Python splitter, including the paragraph-separator removal.
    #[test]
    fn chunking_matches_python_paragraph_boundary_contract() {
        let text = format!("{}\n\n{}", "a".repeat(1_800), "b".repeat(1_700));
        let chunks = chunk_session_text(&text);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0], "a".repeat(1_800));
        assert_eq!(chunks[1], "b".repeat(1_700));
        assert_eq!(
            chunk_session_text(&"🙂".repeat(SESSION_CHUNK_CHARS)).len(),
            1
        );
    }

    #[test]
    fn replay_is_idempotent_and_new_content_gets_a_new_message_identity() {
        let (_dir, mut conn) = database();
        let first = session(vec![SessionMessage {
            role: "user".into(),
            timestamp: None,
            text: "same".into(),
        }]);
        assert_eq!(
            stage_session(&mut conn, "unit-1", &first, "t")
                .unwrap()
                .messages_staged,
            1
        );
        assert_eq!(
            stage_session(&mut conn, "unit-1", &first, "t")
                .unwrap()
                .messages_already_staged,
            1
        );
        let changed = session(vec![SessionMessage {
            role: "user".into(),
            timestamp: None,
            text: "changed".into(),
        }]);
        assert_eq!(
            stage_session(&mut conn, "unit-1", &changed, "t")
                .unwrap()
                .messages_staged,
            1
        );
        let items = list_staged_items(&conn, Some("unit-1")).unwrap();
        assert_eq!(items.len(), 2);
    }
}
