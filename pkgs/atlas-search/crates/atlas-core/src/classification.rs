//! Per-item classification records and their composition into an
//! effective classification.
//!
//! A [`ClassificationRecord`] is the persisted,
//! provenance-carrying judgement about one staged item: its
//! sensitivity on the [`Sensitivity`] ladder, an optional
//! exclusion disposition, and who decided it, when, and by what
//! method. Staging persists the records beside the item (see
//! [`crate::stage::chunk_classifications`]).
//! [`EffectiveClassification::compose`] folds an item's records
//! into the single value disclosure and search enforce — the
//! strictest sensitivity wins, and any personal-life exclusion
//! excludes — and [`crate::disclosure`] and [`crate::classify`]
//! build on these types.

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
    /// Unrecognized additive provenance fields, preserved verbatim.
    pub extras: Map<String, Json>,
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
        let provenance_extras = p;
        if invalid_sensitivity {
            object.insert("sensitivity".into(), raw_sensitivity.unwrap());
        }
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
                extras: provenance_extras,
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
        let mut provenance = p.extras.clone();
        provenance.insert("actor".into(), Json::from(p.actor.clone()));
        provenance.insert("decided_at".into(), Json::from(p.decided_at.clone()));
        provenance.insert("method".into(), Json::from(p.method.clone()));
        provenance.insert(
            "method_version".into(),
            Json::from(p.method_version.clone()),
        );
        provenance.insert(
            "config_version".into(),
            Json::from(p.config_version.clone()),
        );
        provenance.insert(
            "evidence_refs".into(),
            serde_json::to_value(&p.evidence_refs).expect("strings serialize"),
        );
        provenance.insert(
            "previous_decision_id".into(),
            serde_json::to_value(&p.previous_decision_id).expect("optional string serializes"),
        );
        value.insert("provenance".into(), Json::Object(provenance));
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
    /// Build directly from a resolved sensitivity and exclusion
    /// flag, for surfaces whose store carries the legacy label
    /// string rather than full classification records (the search
    /// detail rows). Composition from records ([`Self::compose`])
    /// remains the rule wherever records exist.
    pub fn from_parts(sensitivity: Sensitivity, excluded: bool) -> Self {
        Self {
            sensitivity,
            excluded,
        }
    }

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
    use proptest::prelude::*;
    use serde_json::{Value as Json, json};

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
                "previous_decision_id": "decision-0",
                "future_provenance": {"keep": ["nested", 7]}
            }
        });
        let parsed = ClassificationRecord::parse(value.clone()).expect("valid record");
        assert_eq!(parsed.provenance.actor, "reviewer/1");
        assert_eq!(
            parsed.provenance.extras.get("future_provenance"),
            Some(&json!({"keep": ["nested", 7]}))
        );
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

    fn sensitivity_strategy() -> impl Strategy<Value = Sensitivity> {
        prop_oneof![
            Just(Sensitivity::Shareable),
            Just(Sensitivity::Personal),
            Just(Sensitivity::LocalOnly),
            Just(Sensitivity::SecretBearing),
        ]
    }

    fn disposition_strategy() -> impl Strategy<Value = Option<ClassificationDisposition>> {
        prop_oneof![
            Just(None),
            Just(Some(ClassificationDisposition::Included)),
            Just(Some(ClassificationDisposition::ExcludedPersonalLife)),
        ]
    }

    fn record_for_properties(
        sensitivity: Sensitivity,
        disposition: Option<ClassificationDisposition>,
        embed_id: usize,
    ) -> ClassificationRecord {
        let name = match sensitivity {
            Sensitivity::Shareable => "shareable",
            Sensitivity::Personal => "personal",
            Sensitivity::LocalOnly => "local-only",
            Sensitivity::SecretBearing => "secret-bearing",
        };
        let disposition = disposition.map(ClassificationDisposition::name);
        let mut value = json!({
            "schema_version": 1,
            "embed_id": embed_id.to_string(),
            "sensitivity": name,
            "provenance": {
                "actor": "property-test",
                "decided_at": "2026-11-22T00:00:00Z",
                "method": "generated",
                "method_version": "1",
                "config_version": "1",
                "evidence_refs": [],
                "previous_decision_id": null
            }
        });
        if let Some(disposition) = disposition {
            value["disposition"] = Json::from(disposition);
        }
        ClassificationRecord::parse(value).expect("generated record is valid")
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        #[test]
        fn composition_is_max_order_independent_and_exclusion_is_sticky(
            fields in proptest::collection::vec((sensitivity_strategy(), disposition_strategy()), 0..40),
        ) {
            let records: Vec<_> = fields.into_iter().enumerate()
                .map(|(i, (sensitivity, disposition))| record_for_properties(sensitivity, disposition, i))
                .collect();
            let expected_sensitivity = records.iter().map(|record| record.sensitivity).max().unwrap_or(Sensitivity::Shareable);
            let expected_excluded = records.iter().any(|record| record.disposition == Some(ClassificationDisposition::ExcludedPersonalLife));
            let forward = EffectiveClassification::compose(records.iter());
            let backward = EffectiveClassification::compose(records.iter().rev());

            prop_assert_eq!(forward.sensitivity, expected_sensitivity);
            prop_assert_eq!(forward.is_excluded(), expected_excluded);
            prop_assert_eq!(backward, forward);
        }

        #[test]
        fn unknown_fields_and_malformed_sensitivity_round_trip_fail_closed(
            unknown in prop_oneof![
                Just(Json::Null),
                any::<bool>().prop_map(Json::from),
                any::<i64>().prop_map(Json::from),
                "[a-zA-Z0-9 _-]{0,24}".prop_map(Json::from),
                proptest::collection::vec(any::<i32>(), 0..8).prop_map(Json::from),
            ],
            malformed in prop_oneof![
                Just(Json::Null),
                any::<bool>().prop_map(Json::from),
                any::<i64>().prop_map(Json::from),
                "[A-Z]{1,20}".prop_map(Json::from),
                proptest::collection::vec(any::<i32>(), 0..8).prop_map(Json::from),
            ],
        ) {
            let mut value = json!({
                "schema_version": 1,
                "embed_id": "property-test",
                "sensitivity": "shareable",
                "future_field": unknown,
                "provenance": {
                    "actor": "property-test",
                    "decided_at": "2026-11-22T00:00:00Z",
                    "method": "generated",
                    "method_version": "1",
                    "config_version": "1",
                    "evidence_refs": [],
                    "previous_decision_id": null,
                    "future_provenance": {"value": unknown}
                }
            });
            value["sensitivity"] = malformed;
            let parsed = ClassificationRecord::parse(value.clone()).expect("well-formed envelope remains readable");
            prop_assert_eq!(parsed.sensitivity, Sensitivity::SecretBearing);
            prop_assert_eq!(parsed.to_json(), value);
        }
    }
}
