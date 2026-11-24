//! Typed literal values carried by the query object.

use serde::{Deserialize, Serialize};

use crate::refusal::{Feature, Refusal};

/// A literal value in a filter leaf or a keyset cursor.
///
/// Values are data, never PostgREST text: the translator serializes them
/// at translation time (design §3.1). Integers are carried as `i64` and
/// serialized from the integer itself, never through `f64`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Value {
    /// SQL `NULL`. Only expressible through the `is.null` operator; a
    /// comparison against `Null` is refused (see crate docs).
    Null,
    /// A boolean literal.
    Bool(bool),
    /// A 64-bit integer literal.
    Int(i64),
    /// A 64-bit float literal. Non-finite values are refused at
    /// translation time.
    Float(f64),
    /// A text literal.
    Text(String),
    /// A list of literals; only valid as the value of an `in` filter.
    List(Vec<Value>),
}

impl Value {
    /// Serialize a scalar literal to its PostgREST filter text.
    ///
    /// `Null` and `List` have no scalar literal form and are refused as
    /// `field_unsupported`; a non-finite float is refused the same way
    /// (it has no Postgres literal the adapter may send).
    pub(crate) fn literal(&self) -> Result<String, Refusal> {
        match self {
            Value::Bool(b) => Ok(b.to_string()),
            Value::Int(i) => Ok(i.to_string()),
            Value::Float(f) => {
                if f.is_finite() {
                    Ok(f.to_string())
                } else {
                    Err(Refusal::shape(Feature::FieldUnsupported))
                }
            }
            Value::Text(s) => Ok(s.clone()),
            Value::Null | Value::List(_) => Err(Refusal::shape(Feature::FieldUnsupported)),
        }
    }

    /// Quote a serialized literal for use inside a PostgREST list
    /// context — an `in.(...)` list or the adapter-generated keyset
    /// `or=(...)` expression — where commas and parentheses are syntax.
    /// Values containing a comma, parenthesis, double quote, or
    /// whitespace are double-quoted with backslash escapes, per
    /// PostgREST's quoting rules.
    pub(crate) fn quoted_literal(&self) -> Result<String, Refusal> {
        let raw = self.literal()?;
        if raw.is_empty()
            || raw
                .chars()
                .any(|c| c == ',' || c == '(' || c == ')' || c == '"' || c.is_whitespace())
        {
            Ok(format!(
                "\"{}\"",
                raw.replace('\\', "\\\\").replace('"', "\\\"")
            ))
        } else {
            Ok(raw)
        }
    }
}
