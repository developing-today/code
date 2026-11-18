//! Checked JSON decode for peer input (#8794, contract in
//! `id_core::json_unique`).
//!
//! Every frame or body this crate receives from a peer and then checks,
//! stores, forwards or acts on is a gate, so a duplicated object key is a
//! parse error rather than a last-wins collapse. The lossless form the
//! contract prescribes: validate the original bytes with
//! [`parse_json_without_duplicate_keys`], discard its value, and run the
//! typed decode over the same bytes, so nothing the typed decode used to
//! accept (full-width integers included) is lost.

use anyhow::{Context as _, Result};
use serde::de::DeserializeOwned;

use id_core::json_unique::parse_json_without_duplicate_keys;

/// Decodes peer-supplied JSON into `T`, refusing any duplicated object key at
/// any depth. The error names the repeated key.
pub(crate) fn from_slice_unique<T: DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    let text = std::str::from_utf8(bytes).context("JSON is not valid UTF-8")?;
    parse_json_without_duplicate_keys(text)?;
    Ok(serde_json::from_str(text)?)
}

/// Test support: `json` (an object) with its first key written twice, and
/// that key, so each gate's regression test can feed a real body of its own
/// type with one duplicated key.
#[cfg(test)]
#[allow(clippy::expect_used)]
pub(crate) fn with_duplicated_first_key(json: &[u8]) -> (Vec<u8>, String) {
    let value: serde_json::Value = serde_json::from_slice(json).expect("test JSON parses");
    let (key, first) = value
        .as_object()
        .and_then(|object| object.iter().next())
        .expect("test JSON is a non-empty object");
    let mut out = format!("{{{}:{},", serde_json::Value::from(key.as_str()), first).into_bytes();
    out.extend_from_slice(&json[1..]);
    (out, key.clone())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::from_slice_unique;

    #[derive(Debug, PartialEq, Eq, serde::Deserialize)]
    struct Wide {
        id: String,
        amount: u128,
    }

    #[test]
    fn a_duplicated_key_is_refused_by_name_at_any_depth() {
        let top = from_slice_unique::<serde_json::Value>(br#"{"a":1,"a":2}"#).unwrap_err();
        assert!(format!("{top:#}").contains(r#""a""#), "{top:#}");
        let nested =
            from_slice_unique::<serde_json::Value>(br#"{"outer":{"b":1,"b":2}}"#).unwrap_err();
        assert!(format!("{nested:#}").contains(r#""b""#), "{nested:#}");
    }

    #[test]
    fn full_width_integers_still_decode() {
        let wide: Wide =
            from_slice_unique(br#"{"id":"x","amount":340282366920938463463374607431768211455}"#)
                .unwrap();
        assert_eq!(wide.amount, u128::MAX);
    }

    #[test]
    fn invalid_utf8_is_refused() {
        assert!(from_slice_unique::<serde_json::Value>(b"\"\xff\"").is_err());
    }
}
