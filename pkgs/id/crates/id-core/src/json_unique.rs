//! Checked JSON parsing: a duplicate object key is a parse error.
//!
//! Shared home (design #6520) for the duplicate-key refusal helper first
//! landed in `id`'s `world_session` for issue #6501 (PR #6508). Every
//! checked JSON gate in `pkgs/id` parses through this helper so no gate
//! ever validates, counts, stores, or forwards a collapsed (last-wins)
//! document.

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
