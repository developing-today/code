//! Bounded audit of the real unsigned version and lineage projections (#11186).
//! Oracle: fixture-derived row facts, never another projector invocation.
//! This slice does not cover signed complete-unit §6.4, pinned replay §6.1,
//! correction §6.2 or retirement §6.3; those remain pending #11182/#11184.

use atlas_core::version_projection::{
    AtlasFileNoteBinding, AtlasRevisionEdge, AtlasVersionRow, ProjectedSelectedVersion,
    project_lineage_claims, project_selected_versions,
};
use serde_json::Value;

const FIXTURE: &str = include_str!("../../fixtures/id_projection_rows.json");

fn fixture() -> (Vec<Value>, Vec<AtlasVersionRow>, Vec<AtlasVersionKey>, Vec<AtlasFileNoteBinding>, Vec<AtlasRevisionEdge>) {
    let values: Vec<Value> = serde_json::from_str(FIXTURE).unwrap();
    let rows = values.iter().cloned().map(serde_json::from_value).collect::<Result<_, _>>().unwrap();
    let heads = values.iter().filter(|row| row["head_transition"] == true)
        .map(|row| serde_json::from_value(row.clone()).unwrap()).collect();
    let bindings = values.iter().filter_map(|row| row.get("file_note_binding"))
        .map(|binding| serde_json::from_value(binding.clone()).unwrap()).collect();
    let edges = values.iter().filter_map(|row| row.get("revision_edges").and_then(Value::as_array))
        .flatten().cloned().map(serde_json::from_value).collect::<Result<_, _>>().unwrap();
    (values, rows, heads, bindings, edges)
}

use atlas_core::version_projection::AtlasVersionKey;

/// Temporary permissive stub for the red-stage commit; replaced by the independent audit.
fn audit(_rows: &[AtlasVersionRow], _selected: &[ProjectedSelectedVersion]) -> Vec<String> {
    Vec::new()
}

#[test]
fn audit_rejects_corrupted_real_version_content() {
    let (_, rows, heads, _, _) = fixture();
    let mut selected = project_selected_versions(&rows, &heads).unwrap();
    selected[0].document.content[0] ^= 1;
    let failures = audit(&rows, &selected);
    assert!(failures.iter().any(|property| property == "version.content"),
        "oracle: fixture row bytes; expected version.content failure, got {failures:?}");
}
