//! Secret-backed, owner-specific pattern matching for internal-tier inputs.
//!
//! The matcher is intentionally a narrow component: callers must explicitly
//! pass internal-tier content to [`OwnerInventory::scan_internal`]. It does
//! not write findings or inputs to logs or persistent storage. The inventory
//! is fetched by one Bitwarden Secrets Manager key and is never included in
//! errors, findings, or their serialization.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::{Error, Result, filters::Sensitivity};

pub const INVENTORY_FORMAT: &str = "atlas-owner-inventory/v1";
pub const SECRET_REFERENCE_ENV: &str = "ATLAS_OWNER_INVENTORY_SECRET_NAME";
const MAX_INVENTORY_BYTES: usize = 2 * 1024 * 1024;
const MAX_RECORDS: usize = 10_000;
const MAX_VALUE_BYTES: usize = 4_096;

/// The value-free class reported for an inventory match.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PatternClass {
    Address,
    Phone,
    FamilyName,
    AccountIdFragment,
}

impl PatternClass {
    fn as_str(self) -> &'static str {
        match self {
            Self::Address => "address",
            Self::Phone => "phone",
            Self::FamilyName => "family_name",
            Self::AccountIdFragment => "account_id_fragment",
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InventoryDocument {
    format: String,
    records: Vec<InventoryRecord>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InventoryRecord {
    id: String,
    class: PatternClass,
    value: String,
}

/// Validated in-memory inventory. It deliberately has no `Debug` or
/// `Serialize` implementation, so a routine diagnostic cannot expose values.
pub struct OwnerInventory {
    records: Vec<InventoryRecord>,
}

/// A safe, value-free candidate. Findings contain no matched text or pattern.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OwnerInventoryFinding {
    pub class: &'static str,
    pub record_id: String,
    pub severity: &'static str,
    pub action: &'static str,
}

impl OwnerInventory {
    /// Load from the existing Bitwarden secret path using exactly one key.
    /// Only the key name is configured in the environment; plaintext
    /// inventory values must never be supplied through environment variables.
    pub fn load_runtime() -> Result<Self> {
        let name = std::env::var(SECRET_REFERENCE_ENV).map_err(|_| {
            Error::fatal("owner inventory unavailable: secret reference is not configured")
        })?;
        if name.trim().is_empty() || name.trim() != name {
            return Err(Error::fatal(
                "owner inventory unavailable: secret reference is invalid",
            ));
        }
        Self::from_secret_value(crate::turso::bws_secret(&name))
    }

    fn from_secret_value(raw: Option<String>) -> Result<Self> {
        let raw = raw.ok_or_else(|| {
            Error::fatal("owner inventory unavailable: secret could not be loaded")
        })?;
        Self::from_json(&raw)
    }

    /// Parse a secret value. All failure messages are deliberately generic and
    /// never include parser errors, which can quote portions of the secret.
    pub fn from_json(raw: &str) -> Result<Self> {
        if raw.len() > MAX_INVENTORY_BYTES {
            return Err(invalid_inventory());
        }
        let document: InventoryDocument =
            serde_json::from_str(raw).map_err(|_| invalid_inventory())?;
        if document.format != INVENTORY_FORMAT
            || document.records.is_empty()
            || document.records.len() > MAX_RECORDS
        {
            return Err(invalid_inventory());
        }

        let mut ids = HashSet::with_capacity(document.records.len());
        for record in &document.records {
            if !valid_opaque_id(&record.id)
                || !ids.insert(record.id.as_str())
                || record.value.trim().is_empty()
                || record.value.len() > MAX_VALUE_BYTES
            {
                return Err(invalid_inventory());
            }
        }
        Ok(Self {
            records: document.records,
        })
    }

    /// Match content that the caller has already classified as internal tier.
    /// Returns one candidate per matching record, never the matched content.
    pub fn scan_internal(
        &self,
        input: &str,
        sensitivity: Sensitivity,
    ) -> Result<Vec<OwnerInventoryFinding>> {
        if sensitivity == Sensitivity::Shareable {
            return Err(Error::fatal(
                "owner inventory scan rejected: input is not internal tier",
            ));
        }
        Ok(self
            .records
            .iter()
            .filter(|record| matches_value(record.class, &record.value, input))
            .map(|record| OwnerInventoryFinding {
                class: record.class.as_str(),
                record_id: record.id.clone(),
                severity: "high",
                action: "quarantine_candidate",
            })
            .collect())
    }
}

fn invalid_inventory() -> Error {
    Error::fatal("owner inventory unavailable: secret has invalid format")
}

fn valid_opaque_id(id: &str) -> bool {
    (1..=96).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

fn matches_value(class: PatternClass, pattern: &str, input: &str) -> bool {
    if class == PatternClass::Phone {
        let needle: String = pattern.chars().filter(char::is_ascii_digit).collect();
        let haystack: String = input.chars().filter(char::is_ascii_digit).collect();
        return needle.len() >= 7 && haystack.contains(&needle);
    }
    input.to_lowercase().contains(&pattern.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const SYNTHETIC_INVENTORY: &str = r#"{
      "format":"atlas-owner-inventory/v1",
      "records":[
        {"id":"rec_address_01","class":"address","value":"742 Juniper Example Road"},
        {"id":"rec_phone_02","class":"phone","value":"+1 (555) 010-2048"},
        {"id":"rec_name_03","class":"family_name","value":"Exampleton"},
        {"id":"rec_account_04","class":"account_id_fragment","value":"ACCT-SYNTH-8842"}
      ]
    }"#;

    #[test]
    fn matches_each_class_and_emits_only_opaque_metadata() {
        let inventory = OwnerInventory::from_json(SYNTHETIC_INVENTORY).unwrap();
        let input = "Deliver to 742 Juniper Example Road; call 1-555-010-2048. Exampleton owns ACCT-SYNTH-8842.";
        let findings = inventory
            .scan_internal(input, Sensitivity::SecretBearing)
            .unwrap();
        assert_eq!(findings.len(), 4, "all four synthetic classes should match");
        assert_eq!(
            findings.iter().map(|f| f.class).collect::<Vec<_>>(),
            ["address", "phone", "family_name", "account_id_fragment"]
        );
        assert!(findings.iter().all(|f| f.severity == "high"));
        assert!(findings.iter().all(|f| f.action == "quarantine_candidate"));

        let serialized = serde_json::to_string(&findings).unwrap();
        for synthetic_value in [
            "742 Juniper Example Road",
            "+1 (555) 010-2048",
            "Exampleton",
            "ACCT-SYNTH-8842",
        ] {
            assert!(
                !serialized.contains(synthetic_value),
                "serialized finding leaked a synthetic pattern value"
            );
            assert!(
                !format!("{findings:?}").contains(synthetic_value),
                "finding diagnostics leaked a synthetic pattern value"
            );
        }
        assert!(serialized.contains("rec_address_01"));
    }

    #[test]
    fn absent_or_malformed_inventory_fails_closed_without_echoing_input() {
        let absent = OwnerInventory::from_secret_value(None);
        let malformed = OwnerInventory::from_json(
            r#"{"format":"atlas-owner-inventory/v1","records":[{"id":"synthetic-secret-literal","class":"address","value":"malformed-value"}]"#,
        );
        assert!(absent.is_err());
        let absent_error = match absent {
            Ok(_) => panic!("absent inventory unexpectedly loaded"),
            Err(error) => error.to_string(),
        };
        assert!(absent_error.contains("could not be loaded"));
        let error = match malformed {
            Ok(_) => panic!("malformed inventory unexpectedly loaded"),
            Err(error) => error.to_string(),
        };
        assert!(error.contains("invalid format"));
        assert!(!error.contains("malformed-value"));
        assert!(!error.contains("synthetic-secret-literal"));
    }

    #[test]
    fn rejects_unknown_format_duplicate_ids_and_short_phone_patterns() {
        assert!(OwnerInventory::from_json(
            r#"{"format":"atlas-owner-inventory/v9","records":[{"id":"r1","class":"address","value":"synthetic"}]}"#
        )
        .is_err());
        assert!(OwnerInventory::from_json(
            r#"{"format":"atlas-owner-inventory/v1","records":[{"id":"same","class":"address","value":"synthetic one"},{"id":"same","class":"address","value":"synthetic two"}]}"#
        )
        .is_err());
        let inventory = OwnerInventory::from_json(
            r#"{"format":"atlas-owner-inventory/v1","records":[{"id":"short_phone","class":"phone","value":"555-010"}]}"#,
        )
        .unwrap();
        assert!(
            inventory
                .scan_internal("555-010", Sensitivity::LocalOnly)
                .unwrap()
                .is_empty()
        );
        assert!(
            inventory
                .scan_internal("synthetic", Sensitivity::Shareable)
                .is_err()
        );
    }

    proptest::proptest! {
        #[test]
        fn serialized_findings_never_contain_arbitrary_synthetic_values(
            value in "SYNTHETIC-[A-Z0-9]{8,32}",
            surrounding in "[a-zA-Z0-9 ]{0,40}"
        ) {
            let raw = serde_json::json!({
                "format": INVENTORY_FORMAT,
                "records": [{"id":"prop_record_1", "class":"account_id_fragment", "value":value}]
            }).to_string();
            let inventory = OwnerInventory::from_json(&raw).unwrap();
            let input = format!("{surrounding} {value} {surrounding}");
            let findings = inventory.scan_internal(&input, Sensitivity::Personal).unwrap();
            let serialized = serde_json::to_string(&findings).unwrap();
            prop_assert!(!serialized.contains(&value));
        }
    }
}
