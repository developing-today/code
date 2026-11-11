//! Checked JSON parsing: a duplicate object key is a parse error.
//!
//! Shared home (design #6520) for the duplicate-key refusal helper first
//! landed in `id`'s `world_session` for issue #6501 (PR #6508). Every
//! checked JSON gate in `pkgs/id` parses through this helper so no gate
//! ever validates, counts, stores, or forwards a collapsed (last-wins)
//! document.
//!
//! # The contract
//!
//! - **The rule.** In any JSON document a checked gate consumes, a
//!   duplicate object key is a parse error — at every depth, and the
//!   error names the repeated key. There is no last-wins collapse at a
//!   gate: the document a gate validates, counts, stores, or forwards
//!   is the document as written, or the parse refuses it.
//! - **The one mechanism.** [`parse_json_without_duplicate_keys`] is
//!   the only duplicate-refusing JSON parse in `pkgs/id`. A gate
//!   parses raw text or bytes through it first, then decodes the
//!   resulting `serde_json::Value` into its types. New checked gates
//!   route through it; a raw `serde_json` parse at a gate is a
//!   contract violation, not an alternative spelling.
//! - **What a gate is.** A parse is a gate when its result feeds a
//!   check, a store, a forward, or a comparison. Parses that do none
//!   of these — downstream `from_value` decodes of a value a gate
//!   already produced, files and state the same crate writes and
//!   reads back (the writer cannot emit a duplicate; refusal there
//!   would be hand-edit defence only), scalar decodes, and test code
//!   — are not gates. Every production `serde_json` parse site in
//!   `pkgs/id` is dispositioned, with its reason, in the inventory at
//!   `crates/id-core/JSON_PARSE_INVENTORY.md` in this crate (Phase 4
//!   of #6520, issue #6521); a gate that does not yet route here is
//!   listed in the inventory's conversion follow-up, never silently
//!   excepted.
//! - **Signed objects are in.** Receipt gates for signed objects
//!   take the refusal too (inventory disposition (a)): a signature
//!   covers the exact bytes, but a lenient decode collapses a
//!   duplicated key before any consumer sees the body, so two
//!   readers of the same signed bytes could still disagree about
//!   what was signed. Refusing at receipt protects every consumer
//!   at once; verification over the raw bytes is unchanged.
//! - **The boundary.** Non-JSON decoders are outside this contract,
//!   as a stated exception (inventory disposition (c)): postcard and
//!   MessagePack map types can also carry duplicate map keys, but no
//!   filed instance exists, both decode into typed values at a
//!   single layer with no two-reader `Value` stage for a
//!   disagreement to live in, and this helper is deliberately not
//!   extended to binary formats. A filed instance of a duplicate
//!   map key through either format reopens this boundary as its own
//!   follow-up.

/// Parse one JSON value while refusing repeated object keys at every depth.
/// `serde_json::Value` normally keeps only the last value for a repeated key,
/// which can make routing and later checks observe different bytes than the
/// client sent.
pub fn parse_json_without_duplicate_keys(
    text: &str,
) -> Result<serde_json::Value, serde_json::Error> {
    use serde::Deserialize;
    use serde::de::{self, MapAccess, SeqAccess, Visitor};
    use std::collections::HashSet;
    use std::fmt;

    struct UniqueValue(serde_json::Value);

    impl<'de> Deserialize<'de> for UniqueValue {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            struct UniqueValueVisitor;

            impl<'de> Visitor<'de> for UniqueValueVisitor {
                type Value = UniqueValue;

                fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                    formatter.write_str("a JSON value without duplicate object keys")
                }

                fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E>
                where
                    E: de::Error,
                {
                    Ok(UniqueValue(serde_json::Value::Bool(value)))
                }

                fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
                where
                    E: de::Error,
                {
                    Ok(UniqueValue(serde_json::Value::Number(value.into())))
                }

                fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
                where
                    E: de::Error,
                {
                    Ok(UniqueValue(serde_json::Value::Number(value.into())))
                }

                fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
                where
                    E: de::Error,
                {
                    serde_json::Number::from_f64(value)
                        .map(serde_json::Value::Number)
                        .map(UniqueValue)
                        .ok_or_else(|| E::custom("JSON number is not finite"))
                }

                fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
                where
                    E: de::Error,
                {
                    Ok(UniqueValue(serde_json::Value::String(value.to_owned())))
                }

                fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
                where
                    E: de::Error,
                {
                    Ok(UniqueValue(serde_json::Value::String(value)))
                }

                fn visit_none<E>(self) -> Result<Self::Value, E>
                where
                    E: de::Error,
                {
                    Ok(UniqueValue(serde_json::Value::Null))
                }

                fn visit_unit<E>(self) -> Result<Self::Value, E>
                where
                    E: de::Error,
                {
                    Ok(UniqueValue(serde_json::Value::Null))
                }

                fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
                where
                    A: SeqAccess<'de>,
                {
                    let mut values = Vec::new();
                    while let Some(value) = sequence.next_element::<UniqueValue>()? {
                        values.push(value.0);
                    }
                    Ok(UniqueValue(serde_json::Value::Array(values)))
                }

                fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
                where
                    A: MapAccess<'de>,
                {
                    let mut seen = HashSet::new();
                    let mut values = serde_json::Map::new();
                    while let Some(key) = map.next_key::<String>()? {
                        if !seen.insert(key.clone()) {
                            return Err(de::Error::custom(format!(
                                "duplicate JSON object key {key:?}"
                            )));
                        }
                        let value = map.next_value::<UniqueValue>()?;
                        values.insert(key, value.0);
                    }
                    Ok(UniqueValue(serde_json::Value::Object(values)))
                }
            }

            deserializer.deserialize_any(UniqueValueVisitor)
        }
    }

    let mut deserializer = serde_json::Deserializer::from_str(text);
    let value = UniqueValue::deserialize(&mut deserializer)?.0;
    deserializer.end()?;
    Ok(value)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::parse_json_without_duplicate_keys;

    #[test]
    fn distinct_keys_parse_as_before() {
        let value = parse_json_without_duplicate_keys(r#"{"a":1,"b":{"c":[true]}}"#).unwrap();
        assert_eq!(value["b"]["c"][0], serde_json::Value::Bool(true));
    }

    #[test]
    fn duplicate_keys_are_refused_at_every_depth() {
        for text in [
            r#"{"a":1,"a":2}"#,
            r#"{"a":{"b":1,"b":2}}"#,
            r#"{"a":[{"b":1,"b":2}]}"#,
        ] {
            let err = parse_json_without_duplicate_keys(text).unwrap_err();
            assert!(
                err.to_string().contains("duplicate JSON object key"),
                "{err}"
            );
        }
    }
}
