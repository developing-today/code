//! The `--text` renderer: a line-for-line port of the Python's
//! Ported verbatim from pkgs/atlas-search's bin crate in fleet step 5
//! (#5433): the output-parity contract lives in these writers, so the
//! fleet verb renders service envelopes byte-identically to atlas-search.
//! Drift guard: .github/scripts/check-fleet-search-parity.sh (#5920).
//! `render_text` / `snippet`. Values render the way CPython's
//! f-strings render them (floats via repr, None as "None",
//! `rank_sources` as a Python dict repr).

use serde_json::Value as Json;

use crate::pyjson::{dumps_compact, py_float};

/// CPython f-string interpolation of a JSON value: strings raw,
/// null as `None`, bools as `True`/`False`, numbers via repr,
/// anything else via the compact dumps (the Python would repr a
/// dict/list; the fields rendered here are scalars in practice).
fn py_str(v: Option<&Json>) -> String {
    match v {
        None | Some(Json::Null) => "None".to_string(),
        Some(Json::String(s)) => s.clone(),
        Some(Json::Bool(b)) => {
            if *b {
                "True".to_string()
            } else {
                "False".to_string()
            }
        }
        Some(Json::Number(n)) => {
            if let Some(f) = n.as_f64() {
                if n.is_i64() || n.is_u64() {
                    n.to_string()
                } else {
                    py_float(f)
                }
            } else {
                n.to_string()
            }
        }
        Some(other) => dumps_compact(other),
    }
}

/// `{'fts': 1, 'vector': 2}` — a Python dict repr over the JSON
/// object, in the object's (insertion) order.
fn rank_sources_repr(v: Option<&Json>) -> String {
    let Some(Json::Object(map)) = v else {
        return "{}".to_string();
    };
    let parts: Vec<String> = map
        .iter()
        .map(|(k, val)| format!("'{k}': {}", py_str(Some(val))))
        .collect();
    format!("{{{}}}", parts.join(", "))
}

/// The Python's `snippet`: whitespace-collapsed, 300 code points,
/// ellipsis when longer.
fn snippet(text: &str) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut chars = collapsed.chars();
    let head: String = chars.by_ref().take(300).collect();
    if chars.next().is_some() {
        format!("{head}…")
    } else {
        head
    }
}

fn session_line(s: &Json, indent: &str) -> String {
    let topics = match s.get("topics") {
        Some(Json::Array(items)) => items
            .iter()
            .map(|t| match t {
                Json::String(s) => s.clone(),
                other => dumps_compact(other),
            })
            .collect::<Vec<_>>()
            .join(","),
        _ => String::new(),
    };
    format!(
        "{indent}{} · {} · {} · {} · {} · {}",
        py_str(s.get("title")),
        py_str(s.get("client")),
        py_str(s.get("created_at")),
        py_str(s.get("kind")),
        topics,
        py_str(s.get("sensitivity")),
    )
}

/// The Python's `render_text` (which returns the joined lines;
/// the caller adds print's trailing newline).
pub fn render_text(out: &Json) -> String {
    let mut lines: Vec<String> = Vec::new();
    if out.get("query").is_some() {
        lines.push(format!(
            "query: {}  mode: {}  backend: {}",
            py_str(out.get("query")),
            py_str(out.get("mode")),
            py_str(out.get("backend")),
        ));
        let empty = Json::Object(serde_json::Map::new());
        lines.push(format!(
            "filters: {}",
            dumps_compact(out.get("filters_applied").unwrap_or(&empty))
        ));
        if let Some(notes) = out.get("notes").and_then(Json::as_array) {
            for n in notes {
                lines.push(format!("note: {}", py_str(Some(n))));
            }
        }
        if let Some(results) = out.get("results").and_then(Json::as_array) {
            for (i, r) in results.iter().enumerate() {
                let i = i + 1;
                if r.get("chunks").is_some() {
                    // grouped by session
                    lines.push(format!(
                        "\n[{i}] session {} score={} hits={}",
                        py_str(r.get("session_id")),
                        py_str(r.get("score")),
                        py_str(r.get("hit_count")),
                    ));
                    if let Some(s) = r.get("session") {
                        lines.push(session_line(s, "    "));
                    }
                    if let Some(chunks) = r.get("chunks").and_then(Json::as_array) {
                        for ce in chunks {
                            lines.push(format!(
                                "    chunk {} score={} {}",
                                py_str(ce.get("chunk_id")),
                                py_str(ce.get("score")),
                                rank_sources_repr(ce.get("rank_sources")),
                            ));
                            lines.push(format!(
                                "      {}",
                                snippet(ce.get("text").and_then(Json::as_str).unwrap_or(""))
                            ));
                        }
                    }
                } else {
                    lines.push(format!(
                        "\n[{i}] chunk {} score={} {}",
                        py_str(r.get("chunk_id")),
                        py_str(r.get("score")),
                        rank_sources_repr(r.get("rank_sources")),
                    ));
                    if let Some(s) = r.get("session") {
                        lines.push(session_line(s, "    "));
                    }
                    lines.push(format!(
                        "    {}",
                        snippet(r.get("text").and_then(Json::as_str).unwrap_or(""))
                    ));
                }
            }
        }
    } else {
        // get-chunk
        lines.push(format!(
            "chunk {} (idx {}) session {}",
            py_str(out.get("chunk_id")),
            py_str(out.get("chunk_idx")),
            py_str(out.get("session_id")),
        ));
        if let Some(s) = out.get("session") {
            lines.push(session_line(s, ""));
        }
        lines.push(format!(
            "message {} role={}",
            py_str(out.get("message").and_then(|m| m.get("id"))),
            py_str(out.get("message").and_then(|m| m.get("role"))),
        ));
        lines.push(py_str(out.get("text")));
        if let Some(ctx) = out.get("context") {
            for side in ["before", "after"] {
                if let Some(items) = ctx.get(side).and_then(Json::as_array) {
                    for c in items {
                        lines.push(format!(
                            "[{side} chunk {}] {}",
                            py_str(c.get("chunk_id")),
                            snippet(c.get("text").and_then(Json::as_str).unwrap_or(""))
                        ));
                    }
                }
            }
        }
    }
    lines.join("\n")
}
