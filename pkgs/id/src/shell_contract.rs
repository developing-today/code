//! The versioned shell contract (F13.1).
//!
//! Every shell (the SSH terminal, `id world join`, the browser, a native
//! client) speaks the same frame protocol to a world host. The contract is
//! data, [`SHELL_CONTRACT_JSON`] (`contract/shell-contract.v1.json`): each
//! client and server frame with its fields, its class (journaled event,
//! ephemeral context, query, control, admin, delegation) and the session
//! phases it may be sent in. This module reads that file and checks frames and
//! whole session transcripts against it, so a new shell proves conformance by
//! running a check instead of reading host code.
//!
//! The check is strict about what is *sent*: a frame type, field, or phase
//! the contract does not name is a violation, on either side. Shells must
//! still ignore unknown server frames at run time, because later revisions
//! may add them.
//!
//! Layer 2 of the platform (the session protocol); the guest ABI beneath it
//! is F12.7, and transports (iroh, WebSocket, SSH exec) only move these
//! frames.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::OnceLock;

use serde::Deserialize;
use serde_json::Value;

/// The contract document, embedded at build time.
pub const SHELL_CONTRACT_JSON: &str = include_str!("../contract/shell-contract.v1.json");

/// The contract's wire version; it matches the `/id-world/1` ALPN.
pub const SHELL_CONTRACT_VERSION: u32 = 1;

/// A point in a session's lifecycle; the contract names which client frames
/// each one accepts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// The session's first frame.
    Opening,
    /// After an opening query frame.
    Query,
    /// During a module upload.
    Install,
    /// After a join.
    Joined,
    /// The host answers once and ends the session; nothing more may be sent.
    Closed,
}

impl Phase {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Opening => "opening",
            Self::Query => "query",
            Self::Install => "install",
            Self::Joined => "joined",
            Self::Closed => "closed",
        }
    }
}

/// Which side sent a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    /// The shell.
    Client,
    /// The world host.
    Server,
}

/// How a client frame relates to the journal.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FrameClass {
    /// Becomes one committed, journaled world event.
    Event,
    /// Connection-local presentation context; never journaled.
    Ephemeral,
    /// Read-only; never journaled.
    Query,
    /// Session lifecycle; never journaled itself.
    Control,
    /// Authority-bearing configuration; may write a journal entry.
    Admin,
    /// Mints a narrower capability; may write a journal entry.
    Delegation,
    /// A server frame carrying committed, projected world state.
    Durable,
    /// A server frame answering a request.
    Reply,
}

/// The JSON type of one field.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
pub enum FieldType {
    /// A JSON string.
    #[serde(rename = "string")]
    String,
    /// A string of an even number of hex digits.
    #[serde(rename = "hex")]
    Hex,
    /// A JSON boolean.
    #[serde(rename = "bool")]
    Bool,
    /// An integer in `0..=u16::MAX`.
    #[serde(rename = "u16")]
    U16,
    /// An integer in `0..=u32::MAX`.
    #[serde(rename = "u32")]
    U32,
    /// A non-negative integer.
    #[serde(rename = "u64")]
    U64,
    /// An array of strings.
    #[serde(rename = "string[]")]
    StringArray,
    /// Any JSON array.
    #[serde(rename = "array")]
    Array,
    /// Any JSON object.
    #[serde(rename = "object")]
    Object,
}

/// One field of a frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    /// Its JSON type.
    pub kind: FieldType,
    /// Whether it may be absent.
    pub optional: bool,
    /// Whether an optional field may also be `null`.
    pub nullable: bool,
    /// Smallest allowed integer.
    pub min: Option<u64>,
    /// Largest allowed integer.
    pub max: Option<u64>,
    /// Longest allowed string, in UTF-8 bytes.
    pub max_bytes: Option<usize>,
    /// The only allowed string values.
    pub allowed: Option<Vec<String>>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum RawField {
    Short(String),
    Full {
        #[serde(rename = "type")]
        kind: FieldType,
        #[serde(default)]
        optional: bool,
        nullable: Option<bool>,
        min: Option<u64>,
        max: Option<u64>,
        max_bytes: Option<usize>,
        #[serde(rename = "enum")]
        allowed: Option<Vec<String>>,
    },
}

impl TryFrom<RawField> for Field {
    type Error = String;

    fn try_from(raw: RawField) -> Result<Self, String> {
        match raw {
            RawField::Short(text) => {
                let (name, optional) = text
                    .strip_suffix('?')
                    .map_or((text.as_str(), false), |name| (name, true));
                let kind = serde_json::from_value(Value::from(name))
                    .map_err(|_| format!("unknown field type `{name}`"))?;
                Ok(Self {
                    kind,
                    optional,
                    nullable: optional,
                    min: None,
                    max: None,
                    max_bytes: None,
                    allowed: None,
                })
            }
            RawField::Full {
                kind,
                optional,
                nullable,
                min,
                max,
                max_bytes,
                allowed,
            } => Ok(Self {
                kind,
                optional,
                nullable: nullable.unwrap_or(optional),
                min,
                max,
                max_bytes,
                allowed,
            }),
        }
    }
}

/// One frame type.
#[derive(Clone, Debug)]
pub struct FrameSpec {
    /// How it relates to the journal.
    pub class: FrameClass,
    /// Phases a client may send it in (empty for server frames).
    pub phases: Vec<Phase>,
    /// The phase an opening frame of this type moves the session to.
    pub opens: Option<Phase>,
    /// Keys that may not appear anywhere in the frame.
    pub forbidden_keys: Vec<String>,
    /// Its fields, by name.
    pub fields: BTreeMap<String, Field>,
}

#[derive(Deserialize)]
struct RawFrame {
    class: FrameClass,
    #[serde(default)]
    phases: Vec<Phase>,
    opens: Option<Phase>,
    #[serde(default)]
    forbidden_keys: Vec<String>,
    fields: BTreeMap<String, RawField>,
}

#[derive(Deserialize)]
struct RawProjection {
    redacted_keys: Vec<String>,
}

#[derive(Deserialize)]
struct RawContract {
    contract: String,
    version: u32,
    revision: u32,
    max_frame_bytes: usize,
    routing_fields: Vec<String>,
    transports: BTreeMap<String, Value>,
    client_frames: BTreeMap<String, RawFrame>,
    server_frames: BTreeMap<String, RawFrame>,
    projection: RawProjection,
}

/// The parsed shell contract.
#[derive(Clone, Debug)]
pub struct ShellContract {
    /// Contract name (`id-shell`).
    pub name: String,
    /// Wire version.
    pub version: u32,
    /// Additive revision within the version.
    pub revision: u32,
    /// Largest frame either side may send.
    pub max_frame_bytes: usize,
    /// Fields any client frame may carry for routing (`world`).
    pub routing_fields: Vec<String>,
    /// Transport envelopes, by name.
    pub transports: BTreeMap<String, Value>,
    /// Frames a shell may send.
    pub client_frames: BTreeMap<String, FrameSpec>,
    /// Frames a host may send.
    pub server_frames: BTreeMap<String, FrameSpec>,
    /// Keys the client projection redacts to zero in every server frame.
    pub redacted_keys: Vec<String>,
}

fn frames(raw: BTreeMap<String, RawFrame>) -> Result<BTreeMap<String, FrameSpec>, String> {
    raw.into_iter()
        .map(|(name, frame)| {
            let fields = frame
                .fields
                .into_iter()
                .map(|(field, raw)| {
                    Field::try_from(raw)
                        .map(|spec| (field.clone(), spec))
                        .map_err(|error| format!("{name}.{field}: {error}"))
                })
                .collect::<Result<_, _>>()?;
            Ok((
                name,
                FrameSpec {
                    class: frame.class,
                    phases: frame.phases,
                    opens: frame.opens,
                    forbidden_keys: frame.forbidden_keys,
                    fields,
                },
            ))
        })
        .collect()
}

impl ShellContract {
    /// Parse a contract document.
    ///
    /// # Errors
    ///
    /// Fails if the document is not a valid contract.
    pub fn parse(text: &str) -> Result<Self, String> {
        let raw: RawContract = serde_json::from_str(text).map_err(|error| error.to_string())?;
        Ok(Self {
            name: raw.contract,
            version: raw.version,
            revision: raw.revision,
            max_frame_bytes: raw.max_frame_bytes,
            routing_fields: raw.routing_fields,
            transports: raw.transports,
            client_frames: frames(raw.client_frames)?,
            server_frames: frames(raw.server_frames)?,
            redacted_keys: raw.projection.redacted_keys,
        })
    }

    /// The embedded version 1 contract.
    ///
    /// # Errors
    ///
    /// Fails only if the embedded document is broken, which its tests rule out.
    pub fn embedded() -> Result<&'static Self, String> {
        static CONTRACT: OnceLock<Result<ShellContract, String>> = OnceLock::new();
        CONTRACT
            .get_or_init(|| Self::parse(SHELL_CONTRACT_JSON))
            .as_ref()
            .map_err(Clone::clone)
    }

    /// Check one client frame sent in `phase`, returning its type.
    ///
    /// # Errors
    ///
    /// Returns what the frame got wrong.
    pub fn check_client_frame<'v>(
        &self,
        frame: &'v Value,
        phase: Phase,
    ) -> Result<&'v str, String> {
        let (kind, spec) = self.lookup(frame, &self.client_frames, "client")?;
        if !spec.phases.contains(&phase) {
            return Err(format!(
                "`{kind}` may not be sent in the {} phase",
                phase.as_str()
            ));
        }
        for key in &spec.forbidden_keys {
            if names_key(frame, key) {
                return Err(format!("`{kind}` may not carry `{key}` anywhere"));
            }
        }
        check_fields(kind, frame, spec, &self.routing_fields)?;
        Ok(kind)
    }

    /// Check one server frame, returning its type.
    ///
    /// # Errors
    ///
    /// Returns what the frame got wrong, including a leaked seed.
    pub fn check_server_frame<'v>(&self, frame: &'v Value) -> Result<&'v str, String> {
        let (kind, spec) = self.lookup(frame, &self.server_frames, "server")?;
        check_fields(kind, frame, spec, &[])?;
        for key in &self.redacted_keys {
            if let Some(value) = nonzero_key(frame, key) {
                return Err(format!(
                    "`{kind}` leaks `{key}` ({value}) past the client projection"
                ));
            }
        }
        Ok(kind)
    }

    fn lookup<'v, 's>(
        &self,
        frame: &'v Value,
        table: &'s BTreeMap<String, FrameSpec>,
        side: &str,
    ) -> Result<(&'v str, &'s FrameSpec), String> {
        let encoded = frame.to_string().len();
        if encoded > self.max_frame_bytes {
            return Err(format!(
                "frame of {encoded} bytes exceeds {}",
                self.max_frame_bytes
            ));
        }
        let Some(object) = frame.as_object() else {
            return Err("a frame must be a JSON object".to_owned());
        };
        let Some(kind) = object.get("type").and_then(Value::as_str) else {
            return Err("a frame needs a string `type`".to_owned());
        };
        let spec = table
            .get(kind)
            .ok_or_else(|| format!("`{kind}` is not a {side} frame in the contract"))?;
        Ok((kind, spec))
    }
}

fn check_fields(
    kind: &str,
    frame: &Value,
    spec: &FrameSpec,
    routing: &[String],
) -> Result<(), String> {
    let Some(object) = frame.as_object() else {
        return Err("a frame must be a JSON object".to_owned());
    };
    for (key, value) in object {
        if key == "type" {
            continue;
        }
        if routing.iter().any(|name| name == key) {
            if !(value.is_null() || value.is_string()) {
                return Err(format!("routing field `{key}` must be a string"));
            }
            continue;
        }
        if !spec.fields.contains_key(key) {
            return Err(format!("`{kind}` has no field `{key}`"));
        }
    }
    for (name, field) in &spec.fields {
        match object.get(name) {
            None if field.optional => {}
            Some(Value::Null) if field.nullable => {}
            None | Some(Value::Null) => {
                return Err(format!("`{kind}` needs field `{name}`"));
            }
            Some(value) => {
                check_value(field, value).map_err(|error| format!("`{kind}.{name}` {error}"))?;
            }
        }
    }
    Ok(())
}

fn check_value(field: &Field, value: &Value) -> Result<(), String> {
    let integer_max = match field.kind {
        FieldType::U16 => Some(u64::from(u16::MAX)),
        FieldType::U32 => Some(u64::from(u32::MAX)),
        FieldType::U64 => Some(u64::MAX),
        _ => None,
    };
    if let Some(type_max) = integer_max {
        let Some(number) = value.as_u64() else {
            return Err("must be a non-negative integer".to_owned());
        };
        let max = field.max.unwrap_or(type_max).min(type_max);
        let min = field.min.unwrap_or(0);
        if number < min || number > max {
            return Err(format!("must be in {min}..={max}, got {number}"));
        }
        return Ok(());
    }
    match field.kind {
        FieldType::String | FieldType::Hex => {
            let Some(text) = value.as_str() else {
                return Err("must be a string".to_owned());
            };
            if field.kind == FieldType::Hex
                && (!text.len().is_multiple_of(2) || !text.bytes().all(|b| b.is_ascii_hexdigit()))
            {
                return Err("must be an even number of hex digits".to_owned());
            }
            if let Some(max) = field.max_bytes.filter(|max| text.len() > *max) {
                return Err(format!("is {} bytes, more than {max}", text.len()));
            }
            if let Some(allowed) = field
                .allowed
                .as_ref()
                .filter(|allowed| !allowed.iter().any(|word| word == text))
            {
                return Err(format!("must be one of {}", allowed.join(", ")));
            }
            Ok(())
        }
        FieldType::Bool if value.is_boolean() => Ok(()),
        FieldType::Bool => Err("must be a boolean".to_owned()),
        FieldType::StringArray => match value.as_array() {
            Some(items) if items.iter().all(Value::is_string) => Ok(()),
            _ => Err("must be an array of strings".to_owned()),
        },
        FieldType::Array if value.is_array() => Ok(()),
        FieldType::Array => Err("must be an array".to_owned()),
        FieldType::Object if value.is_object() => Ok(()),
        FieldType::Object => Err("must be an object".to_owned()),
        FieldType::U16 | FieldType::U32 | FieldType::U64 => Ok(()),
    }
}

fn names_key(value: &Value, key: &str) -> bool {
    match value {
        Value::Object(map) => map
            .iter()
            .any(|(name, inner)| name == key || names_key(inner, key)),
        Value::Array(items) => items.iter().any(|item| names_key(item, key)),
        _ => false,
    }
}

/// The first value of `key`, anywhere in `value`, that is not zero.
fn nonzero_key<'v>(value: &'v Value, key: &str) -> Option<&'v Value> {
    match value {
        Value::Object(map) => map.iter().find_map(|(name, inner)| {
            if name == key && inner.as_u64() != Some(0) && !inner.is_null() {
                Some(inner)
            } else {
                nonzero_key(inner, key)
            }
        }),
        Value::Array(items) => items.iter().find_map(|item| nonzero_key(item, key)),
        _ => None,
    }
}

/// One contract violation in a transcript.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Violation {
    /// Zero-based position of the frame in the transcript.
    pub index: usize,
    /// Who sent it.
    pub side: Side,
    /// What was wrong.
    pub message: String,
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let side = match self.side {
            Side::Client => "client",
            Side::Server => "server",
        };
        write!(f, "frame {} ({side}): {}", self.index, self.message)
    }
}

/// Checks one session's frames in order, following its phase.
#[derive(Debug)]
pub struct Conformance<'c> {
    contract: &'c ShellContract,
    phase: Phase,
    index: usize,
    violations: Vec<Violation>,
}

impl<'c> Conformance<'c> {
    /// Start checking a new session.
    #[must_use]
    pub const fn new(contract: &'c ShellContract) -> Self {
        Self {
            contract,
            phase: Phase::Opening,
            index: 0,
            violations: Vec::new(),
        }
    }

    /// The session's current phase.
    #[must_use]
    pub const fn phase(&self) -> Phase {
        self.phase
    }

    /// Record a frame the shell sent.
    pub fn client(&mut self, frame: &Value) {
        if self.phase == Phase::Closed {
            self.violate(
                Side::Client,
                "the host already ended this session".to_owned(),
            );
        } else {
            match self.contract.check_client_frame(frame, self.phase) {
                Ok(kind) => {
                    let opens = self
                        .contract
                        .client_frames
                        .get(kind)
                        .and_then(|spec| spec.opens);
                    self.phase = match (self.phase, kind) {
                        (Phase::Opening, _) => opens.unwrap_or(Phase::Closed),
                        (Phase::Query, "join") => Phase::Joined,
                        (Phase::Install, "install_end") => Phase::Closed,
                        (phase, _) => phase,
                    };
                }
                Err(message) => self.violate(Side::Client, message),
            }
        }
        self.index += 1;
    }

    /// Record a frame the host sent.
    pub fn server(&mut self, frame: &Value) {
        if let Err(message) = self.contract.check_server_frame(frame) {
            self.violate(Side::Server, message);
        }
        // Before a join, an error ends the session.
        if frame.get("type").and_then(Value::as_str) == Some("error")
            && matches!(self.phase, Phase::Opening | Phase::Query | Phase::Install)
        {
            self.phase = Phase::Closed;
        }
        self.index += 1;
    }

    fn violate(&mut self, side: Side, message: String) {
        self.violations.push(Violation {
            index: self.index,
            side,
            message,
        });
    }

    /// Every violation so far.
    #[must_use]
    pub fn violations(&self) -> &[Violation] {
        &self.violations
    }

    /// Finish, returning every violation.
    #[must_use]
    pub fn finish(self) -> Vec<Violation> {
        self.violations
    }
}

/// One line of a recorded transcript: `{"from": "client"|"server", "frame": {...}}`.
#[derive(Debug, Deserialize)]
pub struct TranscriptLine {
    /// Who sent the frame.
    pub from: Side,
    /// The frame.
    pub frame: Value,
}

/// Check a JSON-lines transcript of one session.
///
/// # Errors
///
/// Fails if a line is not a transcript entry; contract violations are
/// returned, not errors.
pub fn check_transcript(contract: &ShellContract, text: &str) -> anyhow::Result<Vec<Violation>> {
    let mut check = Conformance::new(contract);
    for (number, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let entry: TranscriptLine = serde_json::from_str(line)
            .map_err(|error| anyhow::anyhow!("transcript line {}: {error}", number + 1))?;
        match entry.from {
            Side::Client => check.client(&entry.frame),
            Side::Server => check.server(&entry.frame),
        }
    }
    Ok(check.finish())
}

/// The smallest frame of `kind` the contract accepts: every required field
/// with a valid value, `capability` fields set to `capability`.
#[cfg(test)]
pub(crate) fn sample_client_frame(contract: &ShellContract, kind: &str, capability: &str) -> Value {
    let mut frame = serde_json::Map::new();
    frame.insert("type".to_owned(), Value::from(kind));
    for (name, field) in contract
        .client_frames
        .get(kind)
        .map(|spec| &spec.fields)
        .into_iter()
        .flatten()
        .filter(|(_, field)| !field.optional)
    {
        let value = match field.kind {
            FieldType::String if name == "capability" => Value::from(capability),
            FieldType::String => field
                .allowed
                .as_ref()
                .and_then(|allowed| allowed.first())
                .map_or_else(|| Value::from("x"), |word| Value::from(word.as_str())),
            FieldType::Hex => Value::from("00"),
            FieldType::Bool => Value::from(false),
            FieldType::U16 | FieldType::U32 | FieldType::U64 => Value::from(field.min.unwrap_or(1)),
            FieldType::StringArray | FieldType::Array => Value::Array(Vec::new()),
            FieldType::Object => Value::Object(serde_json::Map::new()),
        };
        frame.insert(name.clone(), value);
    }
    Value::Object(frame)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests;
