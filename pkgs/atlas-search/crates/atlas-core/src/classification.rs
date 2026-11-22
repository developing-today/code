use serde_json::{Map, Value as Json};

use crate::filters::Sensitivity;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClassificationDisposition {
    Included,
    ExcludedPersonalLife,
}

impl ClassificationDisposition {
    fn name(self) -> &'static str {
        match self {
            Self::Included => "included",
            Self::ExcludedPersonalLife => "excluded_personal_life",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassificationProvenance {
    pub actor: String,
    pub decided_at: String,
    pub method: String,
    pub method_version: String,
    pub config_version: String,
    pub evidence_refs: Vec<String>,
    pub previous_decision_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassificationRecord {
    pub schema_version: u64,
    pub embed_id: String,
    pub sensitivity: Sensitivity,
    pub disposition: Option<ClassificationDisposition>,
    pub provenance: ClassificationProvenance,
    /// Unrecognized fields, including malformed safety values, preserved verbatim.
    pub extras: Map<String, Json>,
}

impl ClassificationRecord {
    pub fn parse(value: Json) -> Result<Self, String> {
        let mut object = value
            .as_object()
            .cloned()
            .ok_or("record must be an object")?;
        let schema_version = object
            .remove("schema_version")
            .and_then(|v| v.as_u64())
            .ok_or("schema_version must be an unsigned integer")?;
        let embed_id = take_string(&mut object, "embed_id")?;
        let raw_sensitivity = object.remove("sensitivity");
        let sensitivity = raw_sensitivity
            .as_ref()
            .and_then(Json::as_str)
            .and_then(Sensitivity::from_name)
            .unwrap_or(Sensitivity::SecretBearing);
        let invalid_sensitivity = raw_sensitivity
            .as_ref()
            .is_some_and(|v| v.as_str().and_then(Sensitivity::from_name).is_none());
        let disposition = match object.remove("disposition") {
            None => None,
            Some(v) => Some(match v.as_str() {
                Some("included") => ClassificationDisposition::Included,
                Some("excluded_personal_life") => ClassificationDisposition::ExcludedPersonalLife,
                _ => return Err("disposition must be included or excluded_personal_life".into()),
            }),
        };
        let provenance_json = object
            .remove("provenance")
            .ok_or("provenance is required")?;
        let mut p = provenance_json
            .as_object()
            .cloned()
            .ok_or("provenance must be an object")?;
        let actor = take_string(&mut p, "actor")?;
        let decided_at = take_string(&mut p, "decided_at")?;
        let method = take_string(&mut p, "method")?;
        let method_version = take_string(&mut p, "method_version")?;
        let config_version = take_string(&mut p, "config_version")?;
        let evidence_refs = p
            .remove("evidence_refs")
            .and_then(|v| v.as_array().cloned())
            .ok_or("evidence_refs must be an array")?
            .into_iter()
            .map(|v| {
                v.as_str()
                    .map(str::to_owned)
                    .ok_or("evidence_refs entries must be strings")
            })
            .collect::<Result<Vec<_>, _>>()?;
        let previous_decision_id = match p.remove("previous_decision_id") {
            None | Some(Json::Null) => None,
            Some(Json::String(s)) => Some(s),
            _ => return Err("previous_decision_id must be a string or null".into()),
        };
        if !p.is_empty() {
            return Err("unknown provenance fields are not supported".into());
        }
        if invalid_sensitivity {
            object.insert("sensitivity".into(), raw_sensitivity.unwrap());
        }
        object.extend(p);
        Ok(Self {
            schema_version,
            embed_id,
            sensitivity,
            disposition,
            provenance: ClassificationProvenance {
                actor,
                decided_at,
                method,
                method_version,
                config_version,
                evidence_refs,
                previous_decision_id,
            },
            extras: object,
        })
    }

    pub fn to_json(&self) -> Json {
        let mut value = self.extras.clone();
        value.insert("schema_version".into(), Json::from(self.schema_version));
        value.insert("embed_id".into(), Json::from(self.embed_id.clone()));
        if !self.extras.contains_key("sensitivity") {
            value.insert("sensitivity".into(), Json::from(self.sensitivity.name()));
        }
        if let Some(disposition) = self.disposition {
            value.insert("disposition".into(), Json::from(disposition.name()));
        }
        let p = &self.provenance;
        value.insert("provenance".into(), serde_json::json!({"actor":p.actor,"decided_at":p.decided_at,"method":p.method,"method_version":p.method_version,"config_version":p.config_version,"evidence_refs":p.evidence_refs,"previous_decision_id":p.previous_decision_id}));
        Json::Object(value)
    }
}

fn take_string(object: &mut Map<String, Json>, key: &str) -> Result<String, String> {
    object
        .remove(key)
        .and_then(|v| v.as_str().map(str::to_owned))
        .ok_or_else(|| format!("{key} must be a string"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EffectiveClassification {
    pub sensitivity: Sensitivity,
    excluded: bool,
}

impl EffectiveClassification {
    pub fn compose<'a>(records: impl IntoIterator<Item = &'a ClassificationRecord>) -> Self {
        let mut result = Self {
            sensitivity: Sensitivity::Shareable,
            excluded: false,
        };
        for record in records {
            result.sensitivity = result.sensitivity.max(record.sensitivity);
            result.excluded |=
                record.disposition == Some(ClassificationDisposition::ExcludedPersonalLife);
        }
        result
    }

    pub fn is_excluded(self) -> bool {
        self.excluded
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{ClassificationDisposition, ClassificationRecord, EffectiveClassification};
    use crate::filters::Sensitivity;

    fn record(sensitivity: &str, embed_id: &str) -> ClassificationRecord {
        ClassificationRecord::parse(json!({
            "schema_version": 1,
            "embed_id": embed_id,
            "sensitivity": sensitivity,
            "provenance": {
                "actor": "classifier/test",
                "decided_at": "2026-11-22T00:00:00Z",
                "method": "rule-set",
                "method_version": "3",
                "config_version": "sha256:test",
                "evidence_refs": ["issue:1"],
                "previous_decision_id": null
            }
        }))
        .expect("valid classification record")
    }

    #[test]
    fn strict_sensitivity_values_are_parsed_without_coercion() {
        for (name, expected) in [
            ("shareable", Sensitivity::Shareable),
            ("personal", Sensitivity::Personal),
            ("local-only", Sensitivity::LocalOnly),
            ("secret-bearing", Sensitivity::SecretBearing),
        ] {
            assert_eq!(record(name, "embed-1").sensitivity, expected, "{name}");
        }
        let unknown = ClassificationRecord::parse(json!({
            "schema_version": 1,
            "embed_id": "embed-1",
            "sensitivity": "Shareable",
            "provenance": {
                "actor": "reviewer/1",
                "decided_at": "2026-11-22T00:00:00Z",
                "method": "manual",
                "method_version": "1",
                "config_version": "1",
                "evidence_refs": [],
                "previous_decision_id": null
            }
        }))
        .expect("unknown safety values remain readable");
        assert_eq!(unknown.sensitivity, Sensitivity::SecretBearing);
        assert_eq!(unknown.extras.get("sensitivity"), Some(&json!("Shareable")));
    }

    #[test]
    fn missing_or_wrong_typed_sensitivity_is_locked_and_preserved() {
        let missing = ClassificationRecord::parse(json!({
            "schema_version": 1,
            "embed_id": "embed-1",
            "provenance": {
                "actor": "reviewer/1",
                "decided_at": "2026-11-22T00:00:00Z",
                "method": "manual",
                "method_version": "1",
                "config_version": "1",
                "evidence_refs": [],
                "previous_decision_id": null
            }
        }))
        .expect("legacy record remains readable");
        assert_eq!(missing.sensitivity, Sensitivity::SecretBearing);

        let wrong_type = ClassificationRecord::parse(json!({
            "schema_version": 1,
            "embed_id": "embed-1",
            "sensitivity": ["shareable"],
            "provenance": {
                "actor": "reviewer/1",
                "decided_at": "2026-11-22T00:00:00Z",
                "method": "manual",
                "method_version": "1",
                "config_version": "1",
                "evidence_refs": [],
                "previous_decision_id": null
            }
        }))
        .expect("wrong-typed safety values remain readable");
        assert_eq!(wrong_type.sensitivity, Sensitivity::SecretBearing);
        assert_eq!(
            wrong_type.extras.get("sensitivity"),
            Some(&json!(["shareable"]))
        );
    }

    #[test]
    fn effective_classification_only_raises_and_exclusion_is_sticky() {
        let embed = record("personal", "span-1");
        let source_version = record("secret-bearing", "source-v2");
        let recipient_raise = record("local-only", "recipient-1");

        let effective =
            EffectiveClassification::compose([&embed, &source_version, &recipient_raise]);
        assert_eq!(effective.sensitivity, Sensitivity::SecretBearing);

        let excluded = ClassificationRecord::parse(json!({
            "schema_version": 1,
            "embed_id": "span-2",
            "sensitivity": "shareable",
            "disposition": "excluded_personal_life",
            "provenance": {
                "actor": "reviewer/1",
                "decided_at": "2026-11-22T00:00:00Z",
                "method": "manual",
                "method_version": "1",
                "config_version": "1",
                "evidence_refs": [],
                "previous_decision_id": null
            }
        }))
        .expect("valid exclusion disposition");
        assert_eq!(
            excluded.disposition,
            Some(ClassificationDisposition::ExcludedPersonalLife)
        );
        let still_excluded =
            EffectiveClassification::compose([&excluded, &record("shareable", "copy")]);
        assert!(still_excluded.is_excluded());
    }

    #[test]
    fn provenance_and_unknown_fields_survive_round_trip() {
        let value = json!({
            "schema_version": 1,
            "embed_id": "embed-1",
            "sensitivity": "local-only",
            "future_field": {"keep": [1, true]},
            "provenance": {
                "actor": "reviewer/1",
                "decided_at": "2026-11-22T00:00:00Z",
                "method": "manual",
                "method_version": "1",
                "config_version": "1",
                "evidence_refs": ["issue:1"],
                "previous_decision_id": "decision-0"
            }
        });
        let parsed = ClassificationRecord::parse(value.clone()).expect("valid record");
        assert_eq!(parsed.provenance.actor, "reviewer/1");
        assert_eq!(
            parsed.provenance.previous_decision_id.as_deref(),
            Some("decision-0")
        );
        assert_eq!(
            parsed.extras.get("future_field"),
            Some(&json!({"keep": [1, true]}))
        );
        assert_eq!(parsed.to_json(), value);
    }
}
