//! Typed mapping, mapping version 1 (design #5805, §5.1; slice #13761).
//!
//! `PostgREST` returns JSON, and JSON loses type information. The owner
//! therefore declares a Postgres type for every column of a collection
//! ([`PgType`]), and [`TypedDecoder`] maps each value under that declared
//! type, never under whatever the JSON happens to look like:
//!
//! | Declared type                       | Mapped to                                        |
//! | ----------------------------------- | ------------------------------------------------ |
//! | `boolean`                           | [`MappedValue::Boolean`]                         |
//! | `int2`, `int4`, `int8`              | [`MappedValue::Integer`], parsed from the number text, range-checked |
//! | `float4`, `float8`                  | [`MappedValue::Float64`]; `NaN` and infinities refused |
//! | `numeric`                           | tagged `pg.numeric`, the decimal text kept exactly |
//! | `text`, `varchar`, `char`, enums    | [`MappedValue::String`]; enum labels checked     |
//! | `bytea`                             | [`MappedValue::Bytes`], decoded from `\x` hex    |
//! | `timestamptz`                       | [`MappedValue::Timestamp`], epoch microseconds   |
//! | `date`, `time`, `timetz`, `interval`| tagged with their own type id                    |
//! | `json`, `jsonb`                     | a record or list only under a declared [`JsonShape`]; otherwise tagged, opaque |
//! | `uuid`                              | [`MappedValue::String`], canonical lowercase     |
//! | arrays                              | [`MappedValue::List`] of the element mapping     |
//! | `NULL`                              | [`MappedValue::Null`], distinct from an absent column |
//!
//! The invariants this module holds:
//!
//! - **Numbers stay text until the type is known (§9 invariant 5).** The
//!   body is read by the crate's own JSON reader, which keeps every number
//!   as its source text. An integer column is parsed from that text
//!   straight into `i64`; nothing passes through `f64`, so `int8` values
//!   near 2^63 survive exactly.
//! - **A mismatch is named, never shown (§9 invariant 6).** A value that
//!   does not fit its declared type is [`DecodeError::TypeMapping`] with
//!   the column and the declared type name, and nothing of the value.
//! - **Nothing undeclared passes through.** A row that carries a column
//!   the owner did not declare, or a JSON value that does not match its
//!   declared shape, is refused. Undeclared JSON is never flattened into
//!   records.
//! - **The mapping is versioned (§5.1, #5803 §3.5).** [`MAPPING_VERSION`]
//!   names these rules; a change to them is a new version.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

use crate::allowlist::CollectionAllowlist;
use crate::json::{self, Json};
use crate::query::OrderKey;
use crate::read::{DecodeError, PageDecoder};
use crate::value::Value;

/// The version of the rules in this module. It is stored with each
/// materialized snapshot and each live response; any change to the
/// mapping is a new version.
pub const MAPPING_VERSION: u32 = 1;

/// The type id of a `numeric` value.
pub const TYPE_NUMERIC: &str = "pg.numeric";
/// The type id of a `date` value.
pub const TYPE_DATE: &str = "pg.date";
/// The type id of a `time` value.
pub const TYPE_TIME: &str = "pg.time";
/// The type id of a `timetz` value.
pub const TYPE_TIMETZ: &str = "pg.timetz";
/// The type id of an `interval` value.
pub const TYPE_INTERVAL: &str = "pg.interval";
/// The type id of an undeclared `json` value.
pub const TYPE_JSON: &str = "pg.json";
/// The type id of an undeclared `jsonb` value.
pub const TYPE_JSONB: &str = "pg.jsonb";

/// A column's declared Postgres type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum PgType {
    /// `boolean`.
    Boolean,
    /// `smallint` / `int2`.
    Int2,
    /// `integer` / `int4`.
    Int4,
    /// `bigint` / `int8`.
    Int8,
    /// `real` / `float4`.
    Float4,
    /// `double precision` / `float8`.
    Float8,
    /// `numeric` / `decimal`.
    Numeric,
    /// `text`.
    Text,
    /// `varchar`.
    Varchar,
    /// `char` / `bpchar`.
    Char,
    /// A Postgres enum with its declared labels.
    Enum {
        /// The declared labels. A value outside them fails its type.
        labels: Vec<String>,
    },
    /// `bytea`.
    Bytea,
    /// `timestamptz`.
    Timestamptz,
    /// `date`.
    Date,
    /// `time`.
    Time,
    /// `timetz`.
    Timetz,
    /// `interval`.
    Interval,
    /// `json`, with an optional declared shape.
    Json {
        /// The owner-declared shape. Without one the value is tagged and
        /// opaque.
        shape: Option<JsonShape>,
    },
    /// `jsonb`, with an optional declared shape.
    Jsonb {
        /// The owner-declared shape. Without one the value is tagged and
        /// opaque.
        shape: Option<JsonShape>,
    },
    /// `uuid`.
    Uuid,
    /// An array of the element type.
    Array {
        /// The element type; it must itself be mapped.
        element: Box<Self>,
    },
}

impl PgType {
    /// The type's name, as reported in a `type_mapping_failed` error.
    pub fn name(&self) -> String {
        match self {
            Self::Boolean => "boolean".to_owned(),
            Self::Int2 => "int2".to_owned(),
            Self::Int4 => "int4".to_owned(),
            Self::Int8 => "int8".to_owned(),
            Self::Float4 => "float4".to_owned(),
            Self::Float8 => "float8".to_owned(),
            Self::Numeric => "numeric".to_owned(),
            Self::Text => "text".to_owned(),
            Self::Varchar => "varchar".to_owned(),
            Self::Char => "char".to_owned(),
            Self::Enum { .. } => "enum".to_owned(),
            Self::Bytea => "bytea".to_owned(),
            Self::Timestamptz => "timestamptz".to_owned(),
            Self::Date => "date".to_owned(),
            Self::Time => "time".to_owned(),
            Self::Timetz => "timetz".to_owned(),
            Self::Interval => "interval".to_owned(),
            Self::Json { .. } => "json".to_owned(),
            Self::Jsonb { .. } => "jsonb".to_owned(),
            Self::Uuid => "uuid".to_owned(),
            Self::Array { element } => format!("{}[]", element.name()),
        }
    }

    fn validate(&self, column: &str) -> Result<(), String> {
        match self {
            Self::Enum { labels } => {
                if labels.is_empty() {
                    return Err(format!("column {column:?}: an enum needs declared labels"));
                }
                let unique: BTreeSet<&String> = labels.iter().collect();
                if unique.len() != labels.len() {
                    return Err(format!("column {column:?}: enum labels repeat"));
                }
                Ok(())
            }
            Self::Json { shape: Some(shape) } | Self::Jsonb { shape: Some(shape) } => {
                shape.validate(column)
            }
            Self::Array { element } => element.validate(column),
            _ => Ok(()),
        }
    }
}

/// The owner-declared shape of a `json` or `jsonb` column.
///
/// Only a declared shape lets JSON become a record or a list (§5.1).
/// A value must match the shape exactly: a record carries every declared
/// field and no other, and `null` is accepted only where the shape says
/// [`JsonShape::Nullable`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum JsonShape {
    /// A JSON boolean.
    Boolean,
    /// A JSON number that is a 64-bit integer.
    Integer,
    /// A finite JSON number.
    Float64,
    /// A JSON string.
    String,
    /// `null` or the inner shape.
    Nullable {
        /// The shape of a non-null value.
        inner: Box<Self>,
    },
    /// A JSON array of the element shape.
    List {
        /// The shape of every element.
        element: Box<Self>,
    },
    /// A JSON object with exactly these fields.
    Record {
        /// The declared fields, in the order the mapped record keeps.
        fields: Vec<FieldShape>,
    },
}

/// One declared field of a [`JsonShape::Record`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldShape {
    /// The field name.
    pub name: String,
    /// The field's shape.
    pub shape: JsonShape,
}

impl JsonShape {
    fn validate(&self, column: &str) -> Result<(), String> {
        match self {
            Self::Boolean | Self::Integer | Self::Float64 | Self::String => Ok(()),
            Self::Nullable { inner } => inner.validate(column),
            Self::List { element } => element.validate(column),
            Self::Record { fields } => {
                let mut seen = BTreeSet::new();
                for field in fields {
                    if !seen.insert(field.name.as_str()) {
                        return Err(format!(
                            "column {column:?}: JSON shape field {:?} repeats",
                            field.name
                        ));
                    }
                    field.shape.validate(column)?;
                }
                Ok(())
            }
        }
    }
}

/// A value after the typed mapping.
#[derive(Debug, Clone, PartialEq)]
pub enum MappedValue {
    /// SQL `NULL`.
    Null,
    /// A boolean.
    Boolean(bool),
    /// A 64-bit integer.
    Integer(i64),
    /// A finite 64-bit float.
    Float64(f64),
    /// UTF-8 text.
    String(String),
    /// Raw bytes.
    Bytes(Vec<u8>),
    /// An instant, in microseconds since the Unix epoch (UTC).
    Timestamp {
        /// Microseconds since 1970-01-01T00:00:00Z.
        micros: i64,
    },
    /// A value kept under its own type id, with a text payload. Native
    /// ordering and arithmetic are not promised.
    Tagged {
        /// The type id, such as [`TYPE_NUMERIC`].
        type_id: String,
        /// The value's text, exactly as the source sent it (re-serialized
        /// compactly for opaque JSON).
        payload: String,
    },
    /// A list.
    List(Vec<Self>),
    /// A record, fields in declared order.
    Record(Vec<(String, Self)>),
}

/// One decoded row: the columns the page carried, in source order.
///
/// A column the query did not project is absent, which is not the same
/// as [`MappedValue::Null`].
#[derive(Debug, Clone, PartialEq)]
pub struct TypedRow {
    /// Column name → mapped value, in the order the source sent them.
    pub columns: Vec<(String, MappedValue)>,
}

impl TypedRow {
    /// The value of `column`, or `None` when the row does not carry it.
    pub fn get(&self, column: &str) -> Option<&MappedValue> {
        self.columns
            .iter()
            .find_map(|(name, value)| (name == column).then_some(value))
    }
}

/// The typed mapping for one collection: a [`PageDecoder`] that reads a
/// `PostgREST` page body under the owner's declared column types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypedDecoder {
    relation: String,
    types: BTreeMap<String, PgType>,
}

impl TypedDecoder {
    /// Build the decoder for `collection`.
    ///
    /// Every declared column needs a declared type, and every typed
    /// column must be a declared column; an enum needs at least one
    /// label and no repeated label; a JSON shape may not repeat a field.
    /// Anything else is refused with the reason named.
    pub fn new(
        collection: &CollectionAllowlist,
        types: BTreeMap<String, PgType>,
    ) -> Result<Self, String> {
        for column in &collection.columns {
            let ty = types
                .get(&column.name)
                .ok_or_else(|| format!("declared column {:?} has no declared type", column.name))?;
            ty.validate(&column.name)?;
        }
        if let Some(extra) = types.keys().find(|name| !collection.is_declared(name)) {
            return Err(format!("typed column {extra:?} is not a declared column"));
        }
        Ok(Self {
            relation: collection.relation.clone(),
            types,
        })
    }

    /// The relation this decoder maps.
    pub fn relation(&self) -> &str {
        &self.relation
    }

    /// The mapping version the decoder applies: [`MAPPING_VERSION`].
    pub const fn mapping_version(&self) -> u32 {
        MAPPING_VERSION
    }

    /// The declared type of `column`.
    pub fn declared_type(&self, column: &str) -> Option<&PgType> {
        self.types.get(column)
    }

    /// Map one JSON value under `column`'s declared type.
    fn map_column(&self, column: &str, value: &Json) -> Result<MappedValue, DecodeError> {
        let ty = self.types.get(column).ok_or(DecodeError::MalformedRow)?;
        map_value(ty, value).ok_or_else(|| mapping_failed(column, ty))
    }

    fn decode_row(&self, item: Json) -> Result<TypedRow, DecodeError> {
        let Json::Object(members) = item else {
            return Err(DecodeError::NotJson);
        };
        let mut columns = Vec::with_capacity(members.len());
        for (name, value) in members {
            let mapped = self.map_column(&name, &value)?;
            columns.push((name, mapped));
        }
        Ok(TypedRow { columns })
    }
}

fn mapping_failed(column: &str, ty: &PgType) -> DecodeError {
    DecodeError::TypeMapping {
        column: column.to_owned(),
        declared_type: ty.name(),
    }
}

impl PageDecoder for TypedDecoder {
    type Row = TypedRow;

    fn decode(&self, body: &[u8]) -> Result<Vec<TypedRow>, DecodeError> {
        let Json::Array(items) = json::parse(body).map_err(|json::NotJson| DecodeError::NotJson)?
        else {
            return Err(DecodeError::NotJson);
        };
        items
            .into_iter()
            .map(|item| self.decode_row(item))
            .collect()
    }

    /// The keyset cursor of `row`. Each ordering column must be in the
    /// row. A value becomes the literal the translator sends back:
    /// integers, floats, booleans and text as themselves, a timestamp as
    /// UTC ISO-8601 with microseconds, `bytea` as `\x` hex, and a tagged
    /// scalar as its payload. Opaque JSON, records and lists have no
    /// ordering literal and fail their declared type.
    fn cursor(&self, row: &TypedRow, ordering: &[OrderKey]) -> Result<Vec<Value>, DecodeError> {
        ordering
            .iter()
            .map(|key| {
                let value = row.get(&key.column).ok_or(DecodeError::MalformedRow)?;
                let ty = self
                    .types
                    .get(&key.column)
                    .ok_or(DecodeError::MalformedRow)?;
                cursor_value(value).ok_or_else(|| mapping_failed(&key.column, ty))
            })
            .collect()
    }
}

fn cursor_value(value: &MappedValue) -> Option<Value> {
    Some(match value {
        MappedValue::Null => Value::Null,
        MappedValue::Boolean(b) => Value::Bool(*b),
        MappedValue::Integer(i) => Value::Int(*i),
        MappedValue::Float64(f) => Value::Float(*f),
        MappedValue::String(s) => Value::Text(s.clone()),
        MappedValue::Bytes(bytes) => Value::Text(format!("\\x{}", hex(bytes))),
        MappedValue::Timestamp { micros } => Value::Text(format_timestamp(*micros)?),
        MappedValue::Tagged { type_id, payload } => {
            if type_id == TYPE_JSON || type_id == TYPE_JSONB {
                return None;
            }
            Value::Text(payload.clone())
        }
        MappedValue::List(_) | MappedValue::Record(_) => return None,
    })
}

/// Map one value under a declared type, or `None` when it does not fit.
fn map_value(ty: &PgType, value: &Json) -> Option<MappedValue> {
    if *value == Json::Null {
        return Some(MappedValue::Null);
    }
    Some(match (ty, value) {
        (PgType::Boolean, Json::Bool(b)) => MappedValue::Boolean(*b),
        (PgType::Int2, Json::Number(text)) => MappedValue::Integer(parse_integer(
            text,
            i64::from(i16::MIN),
            i64::from(i16::MAX),
        )?),
        (PgType::Int4, Json::Number(text)) => MappedValue::Integer(parse_integer(
            text,
            i64::from(i32::MIN),
            i64::from(i32::MAX),
        )?),
        (PgType::Int8, Json::Number(text)) => {
            MappedValue::Integer(parse_integer(text, i64::MIN, i64::MAX)?)
        }
        (PgType::Float4 | PgType::Float8, Json::Number(text)) => {
            MappedValue::Float64(parse_float(text)?)
        }
        (PgType::Numeric, Json::Number(text)) => tagged(TYPE_NUMERIC, text.clone()),
        (PgType::Text | PgType::Varchar | PgType::Char, Json::String(s)) => {
            MappedValue::String(s.clone())
        }
        (PgType::Enum { labels }, Json::String(s)) => {
            if !labels.contains(s) {
                return None;
            }
            MappedValue::String(s.clone())
        }
        (PgType::Bytea, Json::String(s)) => MappedValue::Bytes(parse_bytea(s)?),
        (PgType::Timestamptz, Json::String(s)) => MappedValue::Timestamp {
            micros: parse_timestamptz(s)?,
        },
        (PgType::Date, Json::String(s)) => tagged(TYPE_DATE, s.clone()),
        (PgType::Time, Json::String(s)) => tagged(TYPE_TIME, s.clone()),
        (PgType::Timetz, Json::String(s)) => tagged(TYPE_TIMETZ, s.clone()),
        (PgType::Interval, Json::String(s)) => tagged(TYPE_INTERVAL, s.clone()),
        (PgType::Json { shape: None }, json) => tagged(TYPE_JSON, json.to_text()),
        (PgType::Jsonb { shape: None }, json) => tagged(TYPE_JSONB, json.to_text()),
        (PgType::Json { shape: Some(shape) } | PgType::Jsonb { shape: Some(shape) }, json) => {
            map_shape(shape, json)?
        }
        (PgType::Uuid, Json::String(s)) => MappedValue::String(parse_uuid(s)?),
        (PgType::Array { element }, Json::Array(items)) => MappedValue::List(
            items
                .iter()
                .map(|item| map_value(element, item))
                .collect::<Option<Vec<_>>>()?,
        ),
        _ => return None,
    })
}

fn tagged(type_id: &str, payload: String) -> MappedValue {
    MappedValue::Tagged {
        type_id: type_id.to_owned(),
        payload,
    }
}

/// Map a JSON value under a declared shape, or `None` when it does not
/// fit.
fn map_shape(shape: &JsonShape, value: &Json) -> Option<MappedValue> {
    Some(match (shape, value) {
        (JsonShape::Nullable { .. }, Json::Null) => MappedValue::Null,
        (JsonShape::Nullable { inner }, json) => map_shape(inner, json)?,
        (JsonShape::Boolean, Json::Bool(b)) => MappedValue::Boolean(*b),
        (JsonShape::Integer, Json::Number(text)) => {
            MappedValue::Integer(parse_integer(text, i64::MIN, i64::MAX)?)
        }
        (JsonShape::Float64, Json::Number(text)) => MappedValue::Float64(parse_float(text)?),
        (JsonShape::String, Json::String(s)) => MappedValue::String(s.clone()),
        (JsonShape::List { element }, Json::Array(items)) => MappedValue::List(
            items
                .iter()
                .map(|item| map_shape(element, item))
                .collect::<Option<Vec<_>>>()?,
        ),
        (JsonShape::Record { fields }, Json::Object(members)) => {
            if members.len() != fields.len() {
                return None;
            }
            let mut record = Vec::with_capacity(fields.len());
            for field in fields {
                let member = members
                    .iter()
                    .find_map(|(name, value)| (*name == field.name).then_some(value))?;
                record.push((field.name.clone(), map_shape(&field.shape, member)?));
            }
            MappedValue::Record(record)
        }
        _ => return None,
    })
}

/// Parse an integer from its JSON number text, never through `f64`.
/// A fraction or exponent is refused even when its value is whole: the
/// source sent a number that is not an integer literal.
fn parse_integer(text: &str, min: i64, max: i64) -> Option<i64> {
    let digits = text.strip_prefix('-').unwrap_or(text);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let value = text.parse::<i64>().ok()?;
    (min..=max).contains(&value).then_some(value)
}

/// Parse a float from its JSON number text; a value that overflows to an
/// infinity is refused.
fn parse_float(text: &str) -> Option<f64> {
    text.parse::<f64>().ok().filter(|f| f.is_finite())
}

/// Decode `PostgREST`'s `\x`-hex `bytea` encoding.
fn parse_bytea(text: &str) -> Option<Vec<u8>> {
    let hex = text.strip_prefix("\\x")?;
    if hex.len() % 2 != 0 {
        return None;
    }
    hex.as_bytes()
        .chunks(2)
        .map(|pair| {
            let high = char::from(pair[0]).to_digit(16)?;
            let low = char::from(pair[1]).to_digit(16)?;
            u8::try_from(high * 16 + low).ok()
        })
        .collect()
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// Check a UUID's 8-4-4-4-12 hex form and return it in canonical
/// lowercase.
fn parse_uuid(text: &str) -> Option<String> {
    let groups: Vec<&str> = text.split('-').collect();
    let lengths = [8, 4, 4, 4, 12];
    if groups.len() != lengths.len()
        || groups
            .iter()
            .zip(lengths)
            .any(|(group, len)| group.len() != len || !group.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return None;
    }
    Some(text.to_ascii_lowercase())
}

const MICROS_PER_SECOND: i64 = 1_000_000;
const SECONDS_PER_DAY: i64 = 86_400;

/// Days from 1970-01-01 to the given proleptic Gregorian date.
const fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month_index = (month + 9) % 12;
    let day_of_year = (153 * month_index + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// The proleptic Gregorian date `days` after 1970-01-01.
const fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let days = days + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    (if month <= 2 { year + 1 } else { year }, month, day)
}

const fn is_leap(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

const fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        2 if is_leap(year) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// A fixed run of ASCII digits read from the front of `text`.
fn take_digits<'a>(text: &mut &'a str, count: usize) -> Option<i64> {
    let head = text.get(..count)?;
    if !head.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    *text = &text[count..];
    head.parse().ok()
}

fn take_char(text: &mut &str, c: char) -> Option<()> {
    *text = text.strip_prefix(c)?;
    Some(())
}

/// Parse a `timestamptz` as `PostgREST` writes it
/// (`YYYY-MM-DDTHH:MM:SS[.ffffff](Z|±HH[:MM[:SS]])`) into microseconds
/// since the Unix epoch. Years outside 0001–9999, `BC` dates,
/// `infinity`, more than six fraction digits and out-of-range fields are
/// refused.
fn parse_timestamptz(text: &str) -> Option<i64> {
    let mut rest = text;
    let year = take_digits(&mut rest, 4)?;
    take_char(&mut rest, '-')?;
    let month = take_digits(&mut rest, 2)?;
    take_char(&mut rest, '-')?;
    let day = take_digits(&mut rest, 2)?;
    take_char(&mut rest, 'T')?;
    let hour = take_digits(&mut rest, 2)?;
    take_char(&mut rest, ':')?;
    let minute = take_digits(&mut rest, 2)?;
    take_char(&mut rest, ':')?;
    let second = take_digits(&mut rest, 2)?;
    let mut fraction = 0_i64;
    if let Some(after) = rest.strip_prefix('.') {
        let len = after.bytes().take_while(u8::is_ascii_digit).count();
        if len == 0 || len > 6 {
            return None;
        }
        rest = after;
        let digits = take_digits(&mut rest, len)?;
        let scale = 10_i64.pow(u32::try_from(6 - len).ok()?);
        fraction = digits * scale;
    }
    let offset_seconds = if rest == "Z" {
        0
    } else {
        let sign = match rest.chars().next()? {
            '+' => 1,
            '-' => -1,
            _ => return None,
        };
        rest = &rest[1..];
        let hours = take_digits(&mut rest, 2)?;
        let mut minutes = 0;
        let mut seconds = 0;
        if take_char(&mut rest, ':').is_some() {
            minutes = take_digits(&mut rest, 2)?;
            if take_char(&mut rest, ':').is_some() {
                seconds = take_digits(&mut rest, 2)?;
            }
        }
        if !rest.is_empty() || hours > 15 || minutes > 59 || seconds > 59 {
            return None;
        }
        sign * (hours * 3600 + minutes * 60 + seconds)
    };
    if year == 0
        || !(1..=12).contains(&month)
        || day < 1
        || day > days_in_month(year, month)
        || hour > 23
        || minute > 59
        || second > 59
    {
        return None;
    }
    let local_seconds =
        days_from_civil(year, month, day) * SECONDS_PER_DAY + hour * 3600 + minute * 60 + second;
    (local_seconds - offset_seconds)
        .checked_mul(MICROS_PER_SECOND)?
        .checked_add(fraction)
}

/// Write microseconds since the epoch as UTC ISO-8601 with six fraction
/// digits and a `Z`, the literal a keyset cursor sends back.
fn format_timestamp(micros: i64) -> Option<String> {
    let seconds = micros.div_euclid(MICROS_PER_SECOND);
    let fraction = micros.rem_euclid(MICROS_PER_SECOND);
    let days = seconds.div_euclid(SECONDS_PER_DAY);
    let of_day = seconds.rem_euclid(SECONDS_PER_DAY);
    let (year, month, day) = civil_from_days(days);
    if !(1..=9999).contains(&year) {
        return None;
    }
    Some(format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{fraction:06}Z",
        of_day / 3600,
        of_day % 3600 / 60,
        of_day % 60
    ))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn integers_are_range_checked_from_text() {
        assert_eq!(parse_integer("32767", -32768, 32767), Some(32767));
        assert_eq!(parse_integer("32768", -32768, 32767), None);
        assert_eq!(
            parse_integer("9223372036854775807", i64::MIN, i64::MAX),
            Some(i64::MAX)
        );
        assert_eq!(
            parse_integer("9223372036854775808", i64::MIN, i64::MAX),
            None
        );
        assert_eq!(parse_integer("1.0", i64::MIN, i64::MAX), None);
        assert_eq!(parse_integer("1e3", i64::MIN, i64::MAX), None);
        assert_eq!(parse_integer("-0", i64::MIN, i64::MAX), Some(0));
    }

    #[test]
    fn timestamps_convert_offsets_to_utc_micros() {
        assert_eq!(parse_timestamptz("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_timestamptz("1970-01-01T00:00:00+00:00"), Some(0));
        assert_eq!(parse_timestamptz("1970-01-01T01:00:00+01"), Some(0));
        assert_eq!(
            parse_timestamptz("1969-12-31T23:59:59.999999+00:00"),
            Some(-1)
        );
        assert_eq!(
            parse_timestamptz("2026-11-25T06:00:00.123456+05:30"),
            Some(
                days_from_civil(2026, 11, 25) * 86_400_000_000 + 6 * 3_600_000_000 + 123_456
                    - 19_800_000_000
            )
        );
        assert_eq!(
            parse_timestamptz("1883-11-18T12:00:00-04:56:02"),
            Some(
                (days_from_civil(1883, 11, 18) * 86_400 + 12 * 3600 + (4 * 3600 + 56 * 60 + 2))
                    * 1_000_000
            )
        );
        for bad in [
            "2026-02-29T00:00:00Z",
            "2026-13-01T00:00:00Z",
            "2026-01-01T24:00:00Z",
            "2026-01-01T00:00:00",
            "2026-01-01 00:00:00Z",
            "2026-01-01T00:00:00.1234567Z",
            "infinity",
            "0000-01-01T00:00:00Z",
            "2026-01-01T00:00:00+0530",
        ] {
            assert_eq!(parse_timestamptz(bad), None, "{bad}");
        }
        assert!(parse_timestamptz("2024-02-29T00:00:00Z").is_some());
    }

    #[test]
    fn timestamps_round_trip_through_the_cursor_literal() {
        for micros in [0, -1, 1_795_996_800_123_456, -62_135_596_800_000_000] {
            let text = format_timestamp(micros).unwrap();
            assert_eq!(parse_timestamptz(&text), Some(micros), "{text}");
        }
        assert_eq!(
            format_timestamp(1_000_001).unwrap(),
            "1970-01-01T00:00:01.000001Z"
        );
    }

    #[test]
    fn civil_dates_round_trip() {
        for days in [-719_162, -1, 0, 1, 10_957, 20_782, 2_932_896] {
            let (y, m, d) = civil_from_days(days);
            assert_eq!(days_from_civil(y, m, d), days);
        }
    }

    #[test]
    fn bytea_and_uuid_forms() {
        assert_eq!(parse_bytea("\\x00ff10"), Some(vec![0, 255, 16]));
        assert_eq!(parse_bytea("\\x"), Some(vec![]));
        assert_eq!(parse_bytea("\\x0"), None);
        assert_eq!(parse_bytea("\\xzz"), None);
        assert_eq!(parse_bytea("00ff"), None);
        assert_eq!(
            parse_uuid("A0EEBC99-9C0B-4EF8-BB6D-6BB9BD380A11").as_deref(),
            Some("a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11")
        );
        assert_eq!(parse_uuid("a0eebc999c0b4ef8bb6d6bb9bd380a11"), None);
        assert_eq!(parse_uuid("g0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11"), None);
    }
}
