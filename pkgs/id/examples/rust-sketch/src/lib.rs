//! Sketch's immutable-stroke domain model for the `sketch/redo-as-new-stroke`
//! first-app scenario (#13608). Draft geometry is transient UI state; only
//! confirmed strokes and undo tombstones enter portable records/snapshots.

use plaza_sdk::json::{self, Value};
use plaza_sdk::World;

#[derive(Clone, Debug, PartialEq)]
pub struct Stroke {
    pub id: String,
    pub author: u32,
    pub points: Vec<[i64; 2]>,
    pub copy_of: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sketch {
    draft: Vec<[i64; 2]>,
    draft_owner: Option<u32>,
    strokes: Vec<Stroke>,
    retained: Vec<Stroke>,
    tombstones: Vec<String>,
    folded: Vec<String>,
}

pub struct SketchWorld;

impl World for SketchWorld {
    type Model = Sketch;

    fn init(_seed: u64) -> Sketch {
        Sketch::default()
    }

    fn update(mut model: Sketch, participant: u32, event: &[u8]) -> Sketch {
        let Ok(text) = std::str::from_utf8(event) else {
            return model;
        };
        let Ok(Value::Object(fields)) = json::decode(text) else {
            return model;
        };
        let get = |key: &str| fields.iter().find(|(k, _)| k == key).map(|(_, v)| v);
        let Some(kind) = get("kind").and_then(Value::as_str) else {
            return model;
        };
        let event_id = get("event").and_then(Value::as_str).map(str::to_owned);
        if let Some(id) = event_id.as_deref() {
            if model.folded.iter().any(|known| known == id) {
                return model;
            }
        }

        match kind {
            "BeginDraft" => {
                if let Some(point) = get("at").and_then(parse_point) {
                    model.draft = vec![point];
                    model.draft_owner = Some(participant);
                }
            }
            "AppendDraftVertices" => {
                if model.draft_owner == Some(participant) {
                    if let Some(points) = get("points").and_then(parse_points) {
                        model.draft.extend(points);
                    }
                }
            }
            "ConfirmStroke" => {
                if let (Some(id), Some(event_id)) =
                    (get("id").and_then(Value::as_str), event_id.as_deref())
                {
                    if model.draft_owner == Some(participant)
                        && !model.draft.is_empty()
                        && !model.strokes.iter().any(|s| s.id == id)
                    {
                        model.strokes.push(Stroke {
                            id: id.to_owned(),
                            author: participant,
                            points: std::mem::take(&mut model.draft),
                            copy_of: None,
                        });
                        model.folded.push(event_id.to_owned());
                        model.draft_owner = None;
                    }
                }
            }
            "UndoOwnStroke" => {
                if let (Some(id), Some(event_id)) =
                    (get("id").and_then(Value::as_str), event_id.as_deref())
                {
                    if let Some(index) = model
                        .strokes
                        .iter()
                        .position(|s| s.id == id && s.author == participant)
                    {
                        model.retained.push(model.strokes.remove(index));
                        model.tombstones.push(id.to_owned());
                        model.folded.push(event_id.to_owned());
                    }
                }
            }
            "CopyOwnStroke" => {
                if let (Some(id), Some(source), Some(event_id)) = (
                    get("id").and_then(Value::as_str),
                    get("copy_of").and_then(Value::as_str),
                    event_id.as_deref(),
                ) {
                    if !model.strokes.iter().any(|s| s.id == id) {
                        if let Some(source_stroke) = model
                            .strokes
                            .iter()
                            .find(|s| s.id == source && s.author == participant)
                            .or_else(|| {
                                model
                                    .retained
                                    .iter()
                                    .find(|s| s.id == source && s.author == participant)
                            })
                            .cloned()
                        {
                            model.strokes.push(Stroke {
                                id: id.to_owned(),
                                author: participant,
                                points: source_stroke.points,
                                copy_of: Some(source.to_owned()),
                            });
                            model.folded.push(event_id.to_owned());
                        }
                    }
                }
            }
            "Redeliver" => {
                // Host-level redelivery is idempotent: guest event history is
                // not rewritten, and the referenced accepted event stays folded.
                let _ = get("ref").and_then(Value::as_str);
            }
            _ => {}
        }
        model
    }

    fn view(model: &Sketch) -> Vec<u8> {
        format!("Sketch: {} committed stroke(s)", model.strokes.len()).into_bytes()
    }

    fn records(model: &Sketch) -> Option<Vec<u8>> {
        encode_state(model, false)
    }

    fn snapshot(model: &Sketch) -> Option<Vec<u8>> {
        encode_state(model, true)
    }

    fn restore(bytes: &[u8]) -> Sketch {
        let Ok(text) = std::str::from_utf8(bytes) else {
            return Sketch::default();
        };
        let Ok(Value::Object(fields)) = json::decode(text) else {
            return Sketch::default();
        };
        decode_state(&fields)
    }
}

fn parse_point(value: &Value) -> Option<[i64; 2]> {
    let values = value.as_array()?;
    Some([values.first()?.as_i64()?, values.get(1)?.as_i64()?])
}

fn parse_points(value: &Value) -> Option<Vec<[i64; 2]>> {
    value.as_array()?.iter().map(parse_point).collect()
}

fn encode_state(model: &Sketch, include_folded: bool) -> Option<Vec<u8>> {
    let strokes = model
        .strokes
        .iter()
        .map(|stroke| stroke_value(stroke, include_folded))
        .collect();
    let tombstones = model.tombstones.iter().cloned().map(Value::Str).collect();
    let mut fields = vec![
        ("visible_strokes".into(), Value::Array(strokes)),
        ("tombstones".into(), Value::Array(tombstones)),
    ];
    if include_folded {
        fields.push((
            "folded".into(),
            Value::Array(model.folded.iter().cloned().map(Value::Str).collect()),
        ));
        fields.push((
            "retained_strokes".into(),
            Value::Array(
                model
                    .retained
                    .iter()
                    .map(|s| stroke_value(s, true))
                    .collect(),
            ),
        ));
    }
    Some(json::encode(&Value::Object(fields)).ok()?.into_bytes())
}

fn decode_state(fields: &[(String, Value)]) -> Sketch {
    let get = |key: &str| fields.iter().find(|(k, _)| k == key).map(|(_, v)| v);
    let strokes: Vec<Stroke> = get("visible_strokes")
        .and_then(Value::as_array)
        .unwrap_or_default()
        .iter()
        .filter_map(|value| parse_stroke(value, 0))
        .collect();
    let retained: Vec<Stroke> = get("retained_strokes")
        .and_then(Value::as_array)
        .unwrap_or_default()
        .iter()
        .filter_map(|value| parse_stroke(value, 0))
        .collect();
    let strings = |key: &str| {
        get(key)
            .and_then(Value::as_array)
            .unwrap_or_default()
            .iter()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect()
    };
    Sketch {
        draft: Vec::new(),
        draft_owner: None,
        strokes,
        retained,
        tombstones: strings("tombstones"),
        folded: strings("folded"),
    }
}

fn stroke_value(stroke: &Stroke, include_owner: bool) -> Value {
    let mut fields = vec![
        ("id".into(), Value::Str(stroke.id.clone())),
        ("author".into(), Value::Str("A".into())),
        (
            "points".into(),
            Value::Array(
                stroke
                    .points
                    .iter()
                    .map(|[x, y]| {
                        Value::Array(vec![
                            Value::Number(x.to_string()),
                            Value::Number(y.to_string()),
                        ])
                    })
                    .collect(),
            ),
        ),
        (
            "copy_of".into(),
            stroke
                .copy_of
                .clone()
                .map(Value::Str)
                .unwrap_or(Value::Null),
        ),
    ];
    if include_owner {
        fields.push(("owner_id".into(), Value::Number(stroke.author.to_string())));
    }
    Value::Object(fields)
}

fn parse_stroke(value: &Value, owner: u32) -> Option<Stroke> {
    let Value::Object(parts) = value else {
        return None;
    };
    let field = |key: &str| parts.iter().find(|(k, _)| k == key).map(|(_, v)| v);
    Some(Stroke {
        id: field("id")?.as_str()?.to_owned(),
        author: field("owner_id")
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok())
            .unwrap_or(owner),
        points: parse_points(field("points")?)?,
        copy_of: field("copy_of")?.as_str().map(str::to_owned),
    })
}

plaza_sdk::plaza_guest!(SketchWorld);

#[cfg(test)]
mod tests {
    use super::*;

    fn step(model: Sketch, participant: u32, event: &str) -> Sketch {
        SketchWorld::update(model, participant, event.as_bytes())
    }

    fn projection(model: &Sketch) -> Value {
        let bytes = SketchWorld::records(model).unwrap_or_default();
        json::decode(std::str::from_utf8(&bytes).unwrap_or("{}")).unwrap_or(Value::Object(vec![]))
    }

    #[test]
    fn six_step_sketch_redo_vector_matches_literal_projections() {
        let mut model = SketchWorld::init(0);
        model = step(model, 1, r#"{"kind":"BeginDraft","at":[1,1]}"#);
        assert_eq!(
            projection(&model),
            json::decode(r#"{"visible_strokes":[],"tombstones":[]}"#).unwrap()
        );
        model = step(
            model,
            1,
            r#"{"kind":"AppendDraftVertices","points":[[3,1],[3,2]]}"#,
        );
        assert_eq!(
            projection(&model),
            json::decode(r#"{"visible_strokes":[],"tombstones":[]}"#).unwrap()
        );
        model = step(
            model,
            1,
            r#"{"kind":"ConfirmStroke","id":"A.1","event":"add1"}"#,
        );
        assert_eq!(projection(&model), json::decode(r#"{"visible_strokes":[{"id":"A.1","author":"A","points":[[1,1],[3,1],[3,2]],"copy_of":null}],"tombstones":[]}"#).unwrap());
        model = step(
            model,
            1,
            r#"{"kind":"UndoOwnStroke","id":"A.1","event":"u1"}"#,
        );
        assert_eq!(
            projection(&model),
            json::decode(r#"{"visible_strokes":[],"tombstones":["A.1"]}"#).unwrap()
        );
        model = step(
            model,
            1,
            r#"{"kind":"CopyOwnStroke","id":"A.2","copy_of":"A.1","event":"copy1"}"#,
        );
        assert_eq!(projection(&model), json::decode(r#"{"visible_strokes":[{"id":"A.2","author":"A","points":[[1,1],[3,1],[3,2]],"copy_of":"A.1"}],"tombstones":["A.1"]}"#).unwrap());
        let redelivered = step(model.clone(), 1, r#"{"kind":"Redeliver","ref":"u1"}"#);
        assert_eq!(projection(&redelivered), projection(&model));
    }

    #[test]
    fn draft_is_not_snapshotted_and_copy_cannot_claim_another_participants_stroke() {
        let mut model = step(
            SketchWorld::init(0),
            1,
            r#"{"kind":"BeginDraft","at":[1,1]}"#,
        );
        let restored = SketchWorld::restore(&SketchWorld::snapshot(&model).unwrap_or_default());
        assert!(restored.draft.is_empty());
        model = step(
            model,
            1,
            r#"{"kind":"ConfirmStroke","id":"A.1","event":"add1"}"#,
        );
        let denied = step(
            model.clone(),
            2,
            r#"{"kind":"CopyOwnStroke","id":"B.1","copy_of":"A.1","event":"copy1"}"#,
        );
        assert_eq!(denied.strokes, model.strokes);
    }

    #[test]
    fn snapshot_restore_preserves_portable_view_and_redelivery_fold() {
        let mut model = SketchWorld::init(0);
        model = step(model, 1, r#"{"kind":"BeginDraft","at":[1,1]}"#);
        model = step(
            model,
            1,
            r#"{"kind":"ConfirmStroke","id":"A.1","event":"add1"}"#,
        );
        let snapshot = SketchWorld::snapshot(&model).unwrap_or_default();
        let restored = SketchWorld::restore(&snapshot);
        assert_eq!(projection(&restored), projection(&model));
        assert_eq!(SketchWorld::view(&restored), SketchWorld::view(&model));
        let duplicate = step(
            restored.clone(),
            1,
            r#"{"kind":"ConfirmStroke","id":"A.2","event":"add1"}"#,
        );
        assert_eq!(duplicate.strokes, restored.strokes);
    }
}
