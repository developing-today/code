//! Used-capability derivation (#10923): the static used-set of a Roc
//! world or library, implemented from the spec
//! `doc/2026-11-11T02-55-00Z_design_used-capability-derivation/` (#5998).
//!
//! The scan closes over the world's Roc sources (§4 step 1), builds the
//! reference graph over every definition in the closure (step 2), marks
//! what the platform's entry points can reach (step 3) and collects the
//! catalog names that reachable definitions place in a capability
//! position (step 4):
//!
//! - (a) a string literal passed where a closure definition takes a
//!   parameter named `cap` — the `RocCap.issue(session, "chat.say")`
//!   position;
//! - (b) the `value` of a `{ key: "cap", value: … }` entry of a request
//!   document under construction (`RocCap.chat_request`);
//! - (c) each element of a `{ key: "subscribe", value: [...] }` list,
//!   normalised with [`Subscription::parse`];
//! - (d) the `cap` fields of `requests` entries and the `subscribe`
//!   entries of a whole wants document written as one string literal.
//!
//! # How the over-approximation rules are realised
//!
//! Every reference to a definition becomes a graph edge, whether it is
//! applied or taken as a value. R1 is the base case; R2 and R3 (a
//! capability-naming function or local function taken as a value) are
//! the value-position edges, which make the function reachable and so
//! contribute every name its body can reach — the sound direction the
//! spec requires. R5 is the edge to a value definition such as
//! `ops = { read: … }` reached through `ops.read`.
//!
//! R4: the platform exposes no runtime-loading API, so the dynamic
//! construct the scan can meet is a capability position filled by a
//! computed (non-literal) name. Forwarding the enclosing definition's
//! own `cap` parameter is not computed: the literal is counted at the
//! forwarding definition's call sites through rule (a).
//!
//! Two front-end limits, both resolved toward over-approximation:
//! static-dispatch method calls (`x.name(…)`) cannot be resolved to a
//! type here, so each takes an edge to every nominal-type member called
//! `name` in the closure; and a pattern that destructures a field named
//! like a top-level definition takes an edge to that definition. A
//! reference this front end cannot resolve is an error, never a skip:
//! a missed edge would understate the used-set while looking exact.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;
use std::path::{Component, Path, PathBuf};

use serde_json::Value;

use crate::world_caps::{Subscription, UNKNOWN_CAP_USAGE_KEY, WANTS_VERSION, is_known};

/// Where the scan reads Roc sources from. Tests use an in-memory map;
/// the tool reads the file system.
pub trait RocSources {
    /// The text of the file at `path`, or `None` when it does not exist.
    fn read(&self, path: &Path) -> Option<String>;
}

/// The file system.
#[derive(Debug, Clone, Copy, Default)]
pub struct FsSources;

impl RocSources for FsSources {
    fn read(&self, path: &Path) -> Option<String> {
        std::fs::read_to_string(path).ok()
    }
}

impl RocSources for BTreeMap<PathBuf, String> {
    fn read(&self, path: &Path) -> Option<String> {
        self.get(path).cloned()
    }
}

/// The derived used-set (§6.1). Built only through [`UsedCaps::new`],
/// which enforces the spec's invariants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsedCaps {
    used: BTreeSet<String>,
    certified: bool,
    entry_points: BTreeSet<String>,
    partial: bool,
    unreachable_refs: BTreeSet<String>,
}

impl UsedCaps {
    /// Build a record, refusing one that breaks a §6.1 invariant: every
    /// name is a catalog name or `<unknown>`, `<unknown>` only when
    /// uncertified, a partial record never certified, and at least one
    /// entry point.
    pub fn new(
        used: BTreeSet<String>,
        certified: bool,
        entry_points: BTreeSet<String>,
        partial: bool,
        unreachable_refs: BTreeSet<String>,
    ) -> Result<Self, UsedCapsError> {
        let unreachable_refs: BTreeSet<String> = unreachable_refs
            .into_iter()
            .filter(|name| !used.contains(name))
            .collect();
        let invalid = |detail: String| Err(UsedCapsError::InvalidRecord(detail));
        if let Some(bad) = used
            .iter()
            .find(|name| name.as_str() != UNKNOWN_CAP_USAGE_KEY && !is_known(name))
        {
            return invalid(format!("used name {bad:?} is not a catalog name"));
        }
        if certified && used.contains(UNKNOWN_CAP_USAGE_KEY) {
            return invalid(format!("{UNKNOWN_CAP_USAGE_KEY} in a certified record"));
        }
        if certified && partial {
            return invalid("a partial-scope record cannot be certified".to_owned());
        }
        if entry_points.is_empty() {
            return invalid("a record names at least one entry point".to_owned());
        }
        if let Some(bad) = unreachable_refs.iter().find(|name| !is_known(name)) {
            return invalid(format!("unreachable ref {bad:?} is not a catalog name"));
        }
        Ok(Self {
            used,
            certified,
            entry_points,
            partial,
            unreachable_refs,
        })
    }

    /// Catalog names, plus `<unknown>` when an R4 construct was reachable.
    #[must_use]
    pub const fn used(&self) -> &BTreeSet<String> {
        &self.used
    }

    /// False iff an R4 construct was reachable or the scope is partial.
    #[must_use]
    pub const fn certified(&self) -> bool {
        self.certified
    }

    /// The module-qualified entry points the scan was seeded with.
    #[must_use]
    pub const fn entry_points(&self) -> &BTreeSet<String> {
        &self.entry_points
    }

    /// True iff the entry points are not the platform's whole declared
    /// export set (§3).
    #[must_use]
    pub const fn partial(&self) -> bool {
        self.partial
    }

    /// Diagnostic only (§6.2): catalog names referenced only from
    /// unreachable code. Never diffed.
    #[must_use]
    pub const fn unreachable_refs(&self) -> &BTreeSet<String> {
        &self.unreachable_refs
    }

    /// The §6.2 registry record for `subject` (for example
    /// `worlds/roc-chat`), as a JSON value with a fixed key order.
    #[must_use]
    pub fn to_record(&self, subject: &str) -> Value {
        let mut map = serde_json::Map::new();
        map.insert("v".to_owned(), Value::from(WANTS_VERSION));
        map.insert("subject".to_owned(), Value::from(subject));
        map.insert("used".to_owned(), strings(&self.used));
        map.insert("certified".to_owned(), Value::from(self.certified));
        map.insert("entry_points".to_owned(), strings(&self.entry_points));
        map.insert(
            "scope".to_owned(),
            Value::from(if self.partial {
                "partial"
            } else {
                "whole-world"
            }),
        );
        if !self.unreachable_refs.is_empty() {
            map.insert(
                "unreachable_refs".to_owned(),
                strings(&self.unreachable_refs),
            );
        }
        Value::Object(map)
    }

    /// The record serialised (§4 step 6: identical inputs give
    /// byte-identical output).
    #[must_use]
    pub fn to_record_string(&self, subject: &str) -> String {
        self.to_record(subject).to_string()
    }

    /// Parse a §6.2 record. Duplicate keys are refused (#6520), as is a
    /// record that breaks an invariant. Returns the subject and the set.
    pub fn parse_record(text: &str) -> Result<(String, Self), UsedCapsError> {
        let bad = |detail: &str| UsedCapsError::InvalidRecord(detail.to_owned());
        let value = id_core::json_unique::parse_json_without_duplicate_keys(text)
            .map_err(|e| UsedCapsError::InvalidRecord(e.to_string()))?;
        let obj = value.as_object().ok_or_else(|| bad("not a JSON object"))?;
        if obj.get("v").and_then(Value::as_u64) != Some(WANTS_VERSION) {
            return Err(bad("unsupported or missing version"));
        }
        let subject = obj
            .get("subject")
            .and_then(Value::as_str)
            .ok_or_else(|| bad("missing subject"))?
            .to_owned();
        let set = |key: &str| -> Result<BTreeSet<String>, UsedCapsError> {
            match obj.get(key) {
                None => Ok(BTreeSet::new()),
                Some(Value::Array(items)) => items
                    .iter()
                    .map(|item| {
                        item.as_str()
                            .map(str::to_owned)
                            .ok_or_else(|| bad("set entries must be strings"))
                    })
                    .collect(),
                Some(_) => Err(bad("sets must be arrays")),
            }
        };
        let certified = obj
            .get("certified")
            .and_then(Value::as_bool)
            .ok_or_else(|| bad("missing certified"))?;
        let partial = match obj.get("scope").and_then(Value::as_str) {
            Some("whole-world") => false,
            Some("partial") => true,
            _ => return Err(bad("scope must be whole-world or partial")),
        };
        let record = Self::new(
            set("used")?,
            certified,
            set("entry_points")?,
            partial,
            set("unreachable_refs")?,
        )?;
        Ok((subject, record))
    }

    /// The §6.3 pairing against the world's stable declared surface.
    #[must_use]
    pub fn pair_with_declared(&self, declared: &BTreeSet<String>) -> CapPairing {
        let under_declared = self
            .used
            .iter()
            .filter(|name| name.as_str() != UNKNOWN_CAP_USAGE_KEY && !declared.contains(*name))
            .cloned()
            .collect();
        let over_declared = self
            .certified
            .then(|| declared.difference(&self.used).cloned().collect());
        CapPairing {
            under_declared,
            unknown_use: self.used.contains(UNKNOWN_CAP_USAGE_KEY),
            over_declared,
            partial: self.partial,
        }
    }
}

fn strings(set: &BTreeSet<String>) -> Value {
    Value::Array(set.iter().map(|s| Value::from(s.as_str())).collect())
}

/// A used-set paired with a stable declaration (§6.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapPairing {
    /// `used − declared`, without `<unknown>`. Always reportable.
    pub under_declared: BTreeSet<String>,
    /// Whether the scan reached an R4 construct (reported on its own line).
    pub unknown_use: bool,
    /// `declared − used`, only for a certified record; `None` when the
    /// record is uncertified or partial and the side is suppressed.
    pub over_declared: Option<BTreeSet<String>>,
    /// Whether the record covers only part of the world's entry points.
    pub partial: bool,
}

/// Why a scan produced no record (§7). Every variant names the
/// offending artifact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UsedCapsError {
    /// A source file is missing or cannot be parsed.
    Unparseable {
        /// The file.
        file: PathBuf,
        /// What went wrong.
        detail: String,
    },
    /// An import does not resolve inside the scanned packages.
    UnresolvedImport {
        /// The importing file.
        file: PathBuf,
        /// The import as written.
        import: String,
    },
    /// A reference names no definition in the closure.
    UnresolvedReference {
        /// The definition holding the reference.
        definition: String,
        /// The reference as written.
        reference: String,
    },
    /// No entry point resolved, or a requested one does not exist.
    EntryPoints(String),
    /// A capability-position literal is not a catalog name: the world
    /// and the platform disagree on the catalog.
    VersionSkew {
        /// The definition holding the literal.
        definition: String,
        /// The literal (a whole subscription spec when it is one).
        literal: String,
    },
    /// A serialized record is malformed or breaks an invariant.
    InvalidRecord(String),
}

impl fmt::Display for UsedCapsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unparseable { file, detail } => {
                write!(f, "cannot parse {}: {detail}", file.display())
            }
            Self::UnresolvedImport { file, import } => {
                write!(f, "{}: import {import} does not resolve", file.display())
            }
            Self::UnresolvedReference {
                definition,
                reference,
            } => write!(
                f,
                "{definition}: reference {reference} names no definition in the closure"
            ),
            Self::EntryPoints(detail) => write!(f, "entry points: {detail}"),
            Self::VersionSkew {
                definition,
                literal,
            } => write!(
                f,
                "{definition}: capability {literal:?} is not in the platform catalog (version skew)"
            ),
            Self::InvalidRecord(detail) => write!(f, "invalid used-caps record: {detail}"),
        }
    }
}

impl std::error::Error for UsedCapsError {}

// ---------------------------------------------------------------------------
// Lexer

#[derive(Debug, Clone, PartialEq, Eq)]
enum Tok {
    /// An identifier, possibly dotted (`RocCap.issue`, `model.cap`).
    Ident(String),
    /// `.name` after an expression: field access or a method.
    DotIdent(String),
    /// A string literal: its text when it has no interpolation. The
    /// interpolated expressions follow as ordinary tokens.
    Str(Option<String>),
    /// A number or character literal.
    Lit,
    /// Punctuation or an operator.
    Op(&'static str),
}

#[derive(Debug, Clone)]
struct Token {
    tok: Tok,
    line: usize,
    col: usize,
    line_start: bool,
}

struct Lexer<'a> {
    chars: Vec<char>,
    pos: usize,
    line: usize,
    col: usize,
    last_line: usize,
    out: Vec<Token>,
    file: &'a Path,
}

const OPS: &[&str] = &[
    "...", "..", "==", "!=", "=>", "->", "<-", "|>", ":=", "<=", ">=", "&&", "||", "//", "??", "(",
    ")", "{", "}", "[", "]", ",", ":", "|", "=", ".", "+", "-", "*", "/", "%", "<", ">", "!", "&",
    "^", "?", "\\", "@", "$", ";", "~",
];

const fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}

const fn is_ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

impl<'a> Lexer<'a> {
    fn new(text: &str, file: &'a Path) -> Self {
        Self {
            chars: text.chars().collect(),
            pos: 0,
            line: 1,
            col: 0,
            last_line: 0,
            out: Vec::new(),
            file,
        }
    }

    fn err(&self, detail: &str) -> UsedCapsError {
        UsedCapsError::Unparseable {
            file: self.file.to_path_buf(),
            detail: format!("line {}: {detail}", self.line),
        }
    }

    fn peek(&self, off: usize) -> Option<char> {
        self.chars.get(self.pos + off).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.chars.get(self.pos).copied()?;
        self.pos += 1;
        if c == '\n' {
            self.line += 1;
            self.col = 0;
        } else {
            self.col += 1;
        }
        Some(c)
    }

    fn push(&mut self, tok: Tok, line: usize, col: usize) {
        let line_start = line != self.last_line;
        self.last_line = line;
        self.out.push(Token {
            tok,
            line,
            col,
            line_start,
        });
    }

    fn ident_run(&mut self) -> String {
        let mut s = String::new();
        while let Some(c) = self.peek(0) {
            if is_ident_char(c) {
                s.push(c);
                self.bump();
            } else {
                break;
            }
        }
        if self.peek(0) == Some('!') && self.peek(1) != Some('=') {
            s.push('!');
            self.bump();
        }
        s
    }

    /// Lex until the input ends, or (when `close_on_brace`) until the
    /// `}` that closes an interpolation opened by the caller.
    fn run(&mut self, close_on_brace: bool) -> Result<(), UsedCapsError> {
        let mut depth = 0usize;
        while let Some(c) = self.peek(0) {
            let (line, col) = (self.line, self.col);
            if c.is_whitespace() {
                self.bump();
            } else if c == '#' {
                while let Some(c) = self.peek(0) {
                    if c == '\n' {
                        break;
                    }
                    self.bump();
                }
            } else if c == '"' {
                self.string(line, col)?;
            } else if c == '\'' {
                self.bump();
                loop {
                    match self.bump() {
                        Some('\\') => {
                            self.bump();
                        }
                        Some('\'') => break,
                        Some(_) => {}
                        None => return Err(self.err("unterminated character literal")),
                    }
                }
                self.push(Tok::Lit, line, col);
            } else if c.is_ascii_digit() {
                while let Some(c) = self.peek(0) {
                    let more = is_ident_char(c)
                        || (c == '.' && self.peek(1).is_some_and(|d| d.is_ascii_digit()));
                    if !more {
                        break;
                    }
                    self.bump();
                }
                self.push(Tok::Lit, line, col);
            } else if is_ident_start(c) {
                let mut name = self.ident_run();
                while self.peek(0) == Some('.') && self.peek(1).is_some_and(is_ident_start) {
                    self.bump();
                    name.push('.');
                    name.push_str(&self.ident_run());
                }
                self.push(Tok::Ident(name), line, col);
            } else if c == '.'
                && self.peek(1).is_some_and(is_ident_start)
                && self.out.last().is_some_and(|t| {
                    matches!(t.tok, Tok::Op(")" | "]" | "}") | Tok::Str(_) | Tok::Lit)
                })
            {
                self.bump();
                let name = self.ident_run();
                self.push(Tok::DotIdent(name), line, col);
            } else {
                let op = OPS
                    .iter()
                    .find(|op| {
                        op.chars()
                            .enumerate()
                            .all(|(i, oc)| self.peek(i) == Some(oc))
                    })
                    .copied()
                    .ok_or_else(|| self.err(&format!("unexpected character {c:?}")))?;
                if close_on_brace && op == "}" && depth == 0 {
                    self.bump();
                    return Ok(());
                }
                match op {
                    "{" | "(" | "[" => depth += 1,
                    "}" | ")" | "]" => depth = depth.saturating_sub(1),
                    _ => {}
                }
                for _ in 0..op.chars().count() {
                    self.bump();
                }
                self.push(Tok::Op(op), line, col);
            }
        }
        if close_on_brace {
            Err(self.err("unterminated string interpolation"))
        } else {
            Ok(())
        }
    }

    fn string(&mut self, line: usize, col: usize) -> Result<(), UsedCapsError> {
        let triple = self.peek(1) == Some('"') && self.peek(2) == Some('"');
        for _ in 0..if triple { 3 } else { 1 } {
            self.bump();
        }
        let mut text = String::new();
        let mut interpolated = false;
        // Interpolated expressions are lexed into their own buffer and
        // appended after the string token, so the string stays one token.
        let mut inner: Vec<Token> = Vec::new();
        loop {
            let Some(c) = self.peek(0) else {
                return Err(self.err("unterminated string literal"));
            };
            if triple && c == '"' && self.peek(1) == Some('"') && self.peek(2) == Some('"') {
                for _ in 0..3 {
                    self.bump();
                }
                break;
            }
            if !triple && c == '"' {
                self.bump();
                break;
            }
            if !triple && c == '\n' {
                return Err(self.err("newline in string literal"));
            }
            if c == '$' && self.peek(1) == Some('{') {
                self.bump();
                self.bump();
                interpolated = true;
                let saved = std::mem::take(&mut self.out);
                let saved_last = self.last_line;
                self.run(true)?;
                inner.extend(std::mem::replace(&mut self.out, saved));
                self.last_line = saved_last;
                continue;
            }
            self.bump();
            if c != '\\' {
                text.push(c);
                continue;
            }
            match self.bump() {
                Some('n') => text.push('\n'),
                Some('t') => text.push('\t'),
                Some('r') => text.push('\r'),
                Some('u') if self.peek(0) == Some('(') => {
                    self.bump();
                    let mut hex = String::new();
                    while let Some(h) = self.bump() {
                        if h == ')' {
                            break;
                        }
                        hex.push(h);
                    }
                    let ch = u32::from_str_radix(&hex, 16)
                        .ok()
                        .and_then(char::from_u32)
                        .ok_or_else(|| self.err("bad unicode escape"))?;
                    text.push(ch);
                }
                Some(other) => text.push(other),
                None => return Err(self.err("unterminated string literal")),
            }
        }
        self.push(Tok::Str((!interpolated).then_some(text)), line, col);
        for mut t in inner {
            t.line_start = false;
            self.out.push(t);
        }
        Ok(())
    }
}

fn lex(text: &str, file: &Path) -> Result<Vec<Token>, UsedCapsError> {
    let mut lexer = Lexer::new(text, file);
    lexer.run(false)?;
    Ok(lexer.out)
}

// ---------------------------------------------------------------------------
// Modules and definitions

/// One definition: a top-level value/function, or a member of a nominal
/// type's `.{ … }` block.
#[derive(Debug)]
struct Def {
    name: String,
    /// The body tokens after `=`.
    body: Vec<Token>,
    /// Lambda parameter names by position (`""` for a pattern).
    params: Vec<String>,
    /// Whether this is a member of a nominal type block (the targets of
    /// static-dispatch method calls).
    member: bool,
}

#[derive(Debug)]
struct Module {
    path: PathBuf,
    stem: String,
    /// The header keyword (`app`, `package`, `platform`), if any.
    kind: Option<String>,
    header: Vec<Token>,
    /// Imported module alias -> module path.
    imports: BTreeMap<String, PathBuf>,
    /// Bare names brought in with `exposing [...]` -> module path.
    exposed_names: BTreeMap<String, PathBuf>,
    /// Nominal type names declared here (they alias the module itself).
    types: BTreeSet<String>,
    defs: BTreeMap<String, Def>,
}

fn qname(stem: &str, def: &str) -> String {
    format!("{stem}.{def}")
}

fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

fn sibling(file: &Path, rel: &str) -> PathBuf {
    normalize(&file.parent().unwrap_or_else(|| Path::new("")).join(rel))
}

fn is_lower_ident(s: &str) -> bool {
    s.chars()
        .next()
        .is_some_and(|c| c.is_ascii_lowercase() || c == '_')
}

fn is_upper_ident(s: &str) -> bool {
    s.chars().next().is_some_and(|c| c.is_ascii_uppercase())
}

fn op(t: Option<&Token>, want: &str) -> bool {
    t.is_some_and(|t| matches!(t.tok, Tok::Op(o) if o == want))
}

fn ident(t: Option<&Token>) -> Option<&str> {
    match t.map(|t| &t.tok) {
        Some(Tok::Ident(s)) => Some(s),
        _ => None,
    }
}

/// The index just past the bracket that closes the one at `open`.
fn close_of(tokens: &[Token], open: usize) -> usize {
    let mut depth = 0usize;
    for (i, t) in tokens.iter().enumerate().skip(open) {
        match t.tok {
            Tok::Op("(" | "[" | "{") => depth += 1,
            Tok::Op(")" | "]" | "}") => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return i + 1;
                }
            }
            _ => {}
        }
    }
    tokens.len()
}

/// Split `tokens` (the inside of a bracket pair) at depth-0 commas. The
/// commas of a lambda's `|a, b|` parameter list do not split.
fn split_commas(tokens: &[Token]) -> Vec<&[Token]> {
    let mut parts = Vec::new();
    let mut depth = 0usize;
    let mut in_params = false;
    let mut start = 0;
    for (i, t) in tokens.iter().enumerate() {
        match t.tok {
            Tok::Op("(" | "[" | "{") => depth += 1,
            Tok::Op(")" | "]" | "}") => depth = depth.saturating_sub(1),
            Tok::Op("|") if depth == 0 => in_params = !in_params,
            Tok::Op(",") if depth == 0 && !in_params => {
                parts.push(&tokens[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    if start < tokens.len() {
        parts.push(&tokens[start..]);
    }
    parts
}

/// Whether the item starting at `i` (a line-start token) begins a new
/// definition, annotation, type declaration or import.
fn starts_item(tokens: &[Token], i: usize) -> bool {
    match ident(tokens.get(i)) {
        Some("import" | "expect") => true,
        Some(name) if !name.contains('.') => {
            op(tokens.get(i + 1), "=") || op(tokens.get(i + 1), ":") || op(tokens.get(i + 1), ":=")
        }
        _ => false,
    }
}

/// Lambda parameter names when `body` starts with `|a, b|`.
fn lambda_params(body: &[Token]) -> Vec<String> {
    if !op(body.first(), "|") {
        return Vec::new();
    }
    let Some(end) = body.iter().skip(1).position(|t| t.tok == Tok::Op("|")) else {
        return Vec::new();
    };
    split_commas(&body[1..=end])
        .into_iter()
        .map(|part| match part {
            [one] => ident(Some(one)).unwrap_or("").to_owned(),
            _ => String::new(),
        })
        .collect()
}

fn parse_module(path: &Path, text: &str) -> Result<Module, UsedCapsError> {
    let tokens = lex(text, path)?;
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_owned();
    let mut module = Module {
        path: path.to_path_buf(),
        stem,
        kind: None,
        header: Vec::new(),
        imports: BTreeMap::new(),
        exposed_names: BTreeMap::new(),
        types: BTreeSet::new(),
        defs: BTreeMap::new(),
    };
    let mut i = 0;
    if let Some(kw @ ("app" | "package" | "platform" | "module")) = ident(tokens.first()) {
        module.kind = Some(kw.to_owned());
        i = 1;
        while i < tokens.len() && !(tokens[i].line_start && tokens[i].col == 0) {
            i += 1;
        }
        module.header = tokens[..i].to_vec();
    }
    let unparseable = |at: &Token, detail: &str| UsedCapsError::Unparseable {
        file: path.to_path_buf(),
        detail: format!("line {}: {detail}", at.line),
    };
    while i < tokens.len() {
        let t = &tokens[i];
        if !(t.line_start && t.col == 0) {
            return Err(unparseable(t, "expected a top-level item"));
        }
        // The extent of this item: up to the next depth-0 column-0 item.
        let mut end = i + 1;
        let mut depth = 0isize;
        while end < tokens.len() {
            let u = &tokens[end];
            if depth <= 0 && u.line_start && u.col == 0 && starts_item(&tokens, end) {
                break;
            }
            match u.tok {
                Tok::Op("(" | "[" | "{") => depth += 1,
                Tok::Op(")" | "]" | "}") => depth -= 1,
                _ => {}
            }
            end += 1;
        }
        let item = &tokens[i..end];
        match (ident(item.first()), item.get(1).map(|t| &t.tok)) {
            (Some("import"), _) => module_import(&mut module, item)?,
            (Some(name), Some(Tok::Op("="))) if is_lower_ident(name) => {
                let body = item[2..].to_vec();
                module.defs.insert(
                    name.to_owned(),
                    Def {
                        name: name.to_owned(),
                        params: lambda_params(&body),
                        body,
                        member: false,
                    },
                );
            }
            (Some(name), Some(Tok::Op(":="))) if is_upper_ident(name) => {
                module.types.insert(name.to_owned());
                nominal_members(&mut module, item, path)?;
            }
            // Annotations, type aliases and inline tests (never entry points).
            (Some(_), Some(Tok::Op(":"))) | (Some("expect"), _) => {}
            _ => return Err(unparseable(t, "unrecognised top-level item")),
        }
        i = end;
    }
    Ok(module)
}

/// The members of a nominal type's `.{ … }` block.
fn nominal_members(module: &mut Module, item: &[Token], path: &Path) -> Result<(), UsedCapsError> {
    // Find `.` `{` at depth 0 after the backing type.
    let mut depth = 0usize;
    let mut open = None;
    for (k, t) in item.iter().enumerate().skip(2) {
        match t.tok {
            Tok::Op("(" | "[" | "{") => {
                if depth == 0 && k > 0 && op(item.get(k - 1), ".") && t.tok == Tok::Op("{") {
                    open = Some(k);
                    break;
                }
                depth += 1;
            }
            Tok::Op(")" | "]" | "}") => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    let Some(open) = open else {
        return Ok(());
    };
    let close = close_of(item, open) - 1;
    let block = &item[open + 1..close];
    let Some(first) = block.first() else {
        return Ok(());
    };
    let indent = first.col;
    let mut k = 0;
    while k < block.len() {
        let t = &block[k];
        if !(t.line_start && t.col == indent) {
            return Err(UsedCapsError::Unparseable {
                file: path.to_path_buf(),
                detail: format!("line {}: expected a member of the type block", t.line),
            });
        }
        let mut end = k + 1;
        let mut depth = 0isize;
        while end < block.len() {
            let u = &block[end];
            if depth <= 0 && u.line_start && u.col <= indent {
                break;
            }
            match u.tok {
                Tok::Op("(" | "[" | "{") => depth += 1,
                Tok::Op(")" | "]" | "}") => depth -= 1,
                _ => {}
            }
            end += 1;
        }
        let member = &block[k..end];
        if let (Some(name), true) = (ident(member.first()), op(member.get(1), "="))
            && is_lower_ident(name)
        {
            let body = member[2..].to_vec();
            module.defs.insert(
                name.to_owned(),
                Def {
                    name: name.to_owned(),
                    params: lambda_params(&body),
                    body,
                    member: true,
                },
            );
        }
        k = end;
    }
    Ok(())
}

fn module_import(module: &mut Module, item: &[Token]) -> Result<(), UsedCapsError> {
    let Some(target) = ident(item.get(1)) else {
        return Err(UsedCapsError::Unparseable {
            file: module.path.clone(),
            detail: format!("line {}: malformed import", item[0].line),
        });
    };
    // Resolved later, once the package header is known; record the raw form.
    let alias = if ident(item.get(2)) == Some("as") {
        ident(item.get(3)).unwrap_or(target).to_owned()
    } else {
        target.rsplit('.').next().unwrap_or(target).to_owned()
    };
    module.imports.insert(alias, PathBuf::from(target));
    if let Some(k) = item.iter().position(|t| ident(Some(t)) == Some("exposing"))
        && op(item.get(k + 1), "[")
    {
        let end = close_of(item, k + 1);
        for t in &item[k + 2..end.saturating_sub(1)] {
            if let Tok::Ident(name) = &t.tok {
                module
                    .exposed_names
                    .insert(name.clone(), PathBuf::from(target));
            }
        }
    }
    Ok(())
}

/// `alias: [platform] "path"` entries of a header's package record:
/// the braces after the exposed list of an app or package, or after
/// `packages` in a platform header. The flag marks a `platform` entry.
fn header_package_entries(header: &[Token]) -> Vec<(String, String, bool)> {
    let open = if ident(header.first()) == Some("platform") {
        header
            .iter()
            .position(|t| ident(Some(t)) == Some("packages"))
            .map(|k| k + 1)
            .filter(|k| op(header.get(*k), "{"))
    } else {
        header
            .iter()
            .position(|t| t.tok == Tok::Op("["))
            .map(|k| close_of(header, k))
            .filter(|k| op(header.get(*k), "{"))
    };
    let Some(open) = open else {
        return Vec::new();
    };
    let end = close_of(header, open);
    split_commas(&header[open + 1..end - 1])
        .into_iter()
        .filter_map(|part| {
            let alias = ident(part.first())?;
            let path = part.iter().find_map(|t| match &t.tok {
                Tok::Str(Some(s)) => Some(s.clone()),
                _ => None,
            })?;
            let platform = part.iter().any(|t| ident(Some(t)) == Some("platform"));
            Some((alias.to_owned(), path, platform))
        })
        .collect()
}

fn header_packages(header: &[Token]) -> Vec<(String, String)> {
    header_package_entries(header)
        .into_iter()
        .map(|(alias, path, _)| (alias, path))
        .collect()
}

/// The names in the first `[ … ]` of a header (`app [program]`,
/// `package [Greeter]`).
fn header_exposes(header: &[Token]) -> Vec<String> {
    let Some(open) = header.iter().position(|t| t.tok == Tok::Op("[")) else {
        return Vec::new();
    };
    let end = close_of(header, open);
    header[open + 1..end - 1]
        .iter()
        .filter_map(|t| ident(Some(t)).map(str::to_owned))
        .collect()
}

/// A platform header's required program fields: the record after
/// `for <name> :` in its `requires` block.
fn platform_required_fields(header: &[Token]) -> Option<(String, Vec<String>)> {
    let k = header.iter().position(|t| ident(Some(t)) == Some("for"))?;
    let value = ident(header.get(k + 1))?.to_owned();
    if !op(header.get(k + 2), ":") || !op(header.get(k + 3), "{") {
        return None;
    }
    let end = close_of(header, k + 3);
    // `update : model, U64, Str -> model` splits at its argument commas;
    // only parts of the form `name :` start a field.
    let fields = split_commas(&header[k + 4..end - 1])
        .into_iter()
        .filter(|part| op(part.get(1), ":"))
        .filter_map(|part| {
            ident(part.first())
                .filter(|n| is_lower_ident(n))
                .map(str::to_owned)
        })
        .collect();
    Some((value, fields))
}

// ---------------------------------------------------------------------------
// The scan

struct Closure {
    modules: Vec<Module>,
    by_path: BTreeMap<PathBuf, usize>,
}

/// Node id: (module index, definition name).
type Node = (usize, String);

fn read_source(sources: &dyn RocSources, path: &Path) -> Result<String, UsedCapsError> {
    sources
        .read(path)
        .ok_or_else(|| UsedCapsError::Unparseable {
            file: path.to_path_buf(),
            detail: "file not found".to_owned(),
        })
}

/// Parse the source closure of `entry` (§4 step 1): the entry module,
/// everything it imports, recursively, and — for a package root — its
/// exposed modules. Each module resolves `alias.Name` imports through
/// the header of the package it belongs to.
fn load_closure(sources: &dyn RocSources, entry: &Path) -> Result<Closure, UsedCapsError> {
    let entry = normalize(entry);
    let mut closure = Closure {
        modules: Vec::new(),
        by_path: BTreeMap::new(),
    };
    // Package root -> its alias map (alias -> root file of the target).
    let mut roots: BTreeMap<PathBuf, BTreeMap<String, PathBuf>> = BTreeMap::new();
    let mut queue: VecDeque<(PathBuf, PathBuf)> = VecDeque::from([(entry.clone(), entry)]);
    while let Some((path, root)) = queue.pop_front() {
        if closure.by_path.contains_key(&path) {
            continue;
        }
        if !roots.contains_key(&root) {
            let header = parse_module(&root, &read_source(sources, &root)?)?.header;
            let aliases = header_packages(&header)
                .into_iter()
                .map(|(alias, rel)| (alias, sibling(&root, &rel)))
                .collect();
            roots.insert(root.clone(), aliases);
        }
        let mut module = parse_module(&path, &read_source(sources, &path)?)?;
        if closure.modules.is_empty() && module.kind.as_deref() == Some("package") {
            for name in header_exposes(&module.header) {
                queue.push_back((sibling(&path, &format!("{name}.roc")), root.clone()));
            }
        }
        let mut resolved = BTreeMap::new();
        for (alias, raw) in std::mem::take(&mut module.imports) {
            let raw = raw.to_string_lossy().into_owned();
            let unresolved = || UsedCapsError::UnresolvedImport {
                file: path.clone(),
                import: raw.clone(),
            };
            let (target, target_root) = match raw.split_once('.') {
                Some((pkg, name)) => {
                    let pkg_root = roots
                        .get(&root)
                        .and_then(|aliases| aliases.get(pkg))
                        .cloned()
                        .ok_or_else(unresolved)?;
                    (sibling(&pkg_root, &format!("{name}.roc")), pkg_root)
                }
                None => (sibling(&path, &format!("{raw}.roc")), root.clone()),
            };
            if sources.read(&target).is_none() {
                return Err(unresolved());
            }
            queue.push_back((target.clone(), target_root));
            resolved.insert(alias, target);
        }
        for (name, raw) in std::mem::take(&mut module.exposed_names) {
            let raw = raw.to_string_lossy().into_owned();
            let alias = raw.rsplit('.').next().unwrap_or_default();
            if let Some(target) = resolved.get(alias) {
                module.exposed_names.insert(name, target.clone());
            }
        }
        module.imports = resolved;
        closure.by_path.insert(path, closure.modules.len());
        closure.modules.push(module);
    }
    Ok(closure)
}

/// What one reachable definition contributes.
#[derive(Default)]
struct Facts {
    edges: BTreeSet<Node>,
    names: BTreeSet<String>,
    dynamic: bool,
}

struct Scanner<'c> {
    closure: &'c Closure,
    /// Method name -> nominal members with that name.
    members: BTreeMap<String, Vec<Node>>,
}

impl<'c> Scanner<'c> {
    fn new(closure: &'c Closure) -> Self {
        let mut members: BTreeMap<String, Vec<Node>> = BTreeMap::new();
        for (m, module) in closure.modules.iter().enumerate() {
            for def in module.defs.values().filter(|d| d.member) {
                members
                    .entry(def.name.clone())
                    .or_default()
                    .push((m, def.name.clone()));
            }
        }
        Self { closure, members }
    }

    fn label(&self, node: &Node) -> String {
        qname(&self.closure.modules[node.0].stem, &node.1)
    }

    /// Resolve a dotted identifier to the definition it names, if any.
    /// `Ok(None)` is a local, a builtin, a field access or a tag.
    fn resolve(&self, m: usize, name: &str) -> Result<Option<Node>, String> {
        let module = &self.closure.modules[m];
        let mut segs = name.split('.');
        let head = segs.next().unwrap_or_default();
        if is_upper_ident(head) {
            let Some(member) = segs.next() else {
                return Ok(None);
            };
            if !is_lower_ident(member) {
                return Ok(None); // a tag or a type
            }
            let target = if module.types.contains(head) {
                Some(m)
            } else {
                module
                    .imports
                    .get(head)
                    .and_then(|p| self.closure.by_path.get(p).copied())
            };
            let Some(target) = target else {
                return Ok(None); // a builtin module (List, Str, …)
            };
            let member = member.trim_end_matches('!');
            let bare = member.to_owned();
            return if self.closure.modules[target].defs.contains_key(&bare) {
                Ok(Some((target, bare)))
            } else {
                Err(name.to_owned())
            };
        }
        if module.defs.contains_key(head) {
            return Ok(Some((m, head.to_owned())));
        }
        if let Some(path) = module.exposed_names.get(head)
            && let Some(&target) = self.closure.by_path.get(path)
            && self.closure.modules[target].defs.contains_key(head)
        {
            return Ok(Some((target, head.to_owned())));
        }
        Ok(None)
    }

    fn facts(&self, node: &Node) -> Result<Facts, UsedCapsError> {
        let module = &self.closure.modules[node.0];
        let def = &module.defs[&node.1];
        let label = self.label(node);
        let body = &def.body;
        let own_cap_param = def.params.iter().any(|p| p == "cap");
        let mut facts = Facts::default();
        let unresolved = |reference: String| UsedCapsError::UnresolvedReference {
            definition: label.clone(),
            reference,
        };
        let mut use_name = |facts: &mut Facts, name: String| -> Result<(), UsedCapsError> {
            if is_known(&name) {
                facts.names.insert(name);
                Ok(())
            } else {
                Err(UsedCapsError::VersionSkew {
                    definition: label.clone(),
                    literal: name,
                })
            }
        };
        // A capability-position expression: a literal, a forwarded `cap`
        // parameter, or a computed name (R4).
        let cap_expr = |expr: &[Token]| -> Option<Option<String>> {
            let inner = match expr {
                [f, open, rest @ .., close]
                    if ident(Some(f)).is_some_and(|f| f.ends_with(".String"))
                        && open.tok == Tok::Op("(")
                        && close.tok == Tok::Op(")") =>
                {
                    rest
                }
                other => other,
            };
            match inner {
                [
                    Token {
                        tok: Tok::Str(Some(s)),
                        ..
                    },
                ] => Some(Some(s.clone())),
                [one] if own_cap_param && ident(Some(one)) == Some("cap") => Some(None),
                _ => None,
            }
        };

        for (k, t) in body.iter().enumerate() {
            match &t.tok {
                Tok::Ident(name) => {
                    // A record label or local annotation is not a reference.
                    if !name.contains('.') && op(body.get(k + 1), ":") {
                        continue;
                    }
                    let applied = op(body.get(k + 1), "(");
                    let head_lower = is_lower_ident(name);
                    if head_lower && applied && name.contains('.') {
                        let method = name.rsplit('.').next().unwrap_or_default();
                        self.method_edges(method, &mut facts);
                    }
                    let Some(target) = self.resolve(node.0, name).map_err(unresolved)? else {
                        continue;
                    };
                    // (a): literals passed where the callee takes `cap`.
                    if applied && !(head_lower && name.contains('.')) {
                        let callee = &self.closure.modules[target.0].defs[&target.1];
                        if callee.params.iter().any(|p| p == "cap") {
                            let end = close_of(body, k + 1);
                            let args = split_commas(&body[k + 2..end - 1]);
                            for (idx, arg) in args.iter().enumerate() {
                                if callee.params.get(idx).map(String::as_str) != Some("cap") {
                                    continue;
                                }
                                match cap_expr(arg) {
                                    Some(Some(lit)) => use_name(&mut facts, lit)?,
                                    Some(None) => {}
                                    None => facts.dynamic = true,
                                }
                            }
                        }
                    }
                    facts.edges.insert(target);
                }
                Tok::DotIdent(method) if op(body.get(k + 1), "(") => {
                    self.method_edges(method, &mut facts);
                }
                Tok::Op("{") => {
                    let end = close_of(body, k);
                    Self::document_entry(
                        &body[k + 1..end - 1],
                        &cap_expr,
                        &mut facts,
                        &mut use_name,
                    )?;
                }
                Tok::Str(Some(text)) => {
                    Self::serialized_document(text, &mut facts, &mut use_name)?;
                }
                _ => {}
            }
        }
        Ok(facts)
    }

    fn method_edges(&self, method: &str, facts: &mut Facts) {
        if let Some(targets) = self.members.get(method.trim_end_matches('!')) {
            facts.edges.extend(targets.iter().cloned());
        }
    }

    /// (b) and (c): a `{ key: "cap" | "subscribe", value: … }` record.
    fn document_entry(
        inside: &[Token],
        cap_expr: &dyn Fn(&[Token]) -> Option<Option<String>>,
        facts: &mut Facts,
        use_name: &mut dyn FnMut(&mut Facts, String) -> Result<(), UsedCapsError>,
    ) -> Result<(), UsedCapsError> {
        let mut key = None;
        let mut value: Option<&[Token]> = None;
        for field in split_commas(inside) {
            match (ident(field.first()), op(field.get(1), ":")) {
                (Some("key"), true) => {
                    if let [
                        Token {
                            tok: Tok::Str(Some(s)),
                            ..
                        },
                    ] = &field[2..]
                    {
                        key = Some(s.as_str());
                    }
                }
                (Some("value"), true) => value = Some(&field[2..]),
                _ => {}
            }
        }
        let Some(value) = value else {
            return Ok(());
        };
        match key {
            Some("cap") => match cap_expr(value) {
                Some(Some(lit)) => use_name(facts, lit)?,
                Some(None) => {}
                None => facts.dynamic = true,
            },
            Some("subscribe") => {
                // `X.Array([ … ])` or `[ … ]`.
                let list = match value {
                    [f, open, rest @ .., close]
                        if ident(Some(f)).is_some_and(|f| f.ends_with(".Array"))
                            && open.tok == Tok::Op("(")
                            && close.tok == Tok::Op(")") =>
                    {
                        rest
                    }
                    other => other,
                };
                match list {
                    [open, items @ .., close]
                        if open.tok == Tok::Op("[") && close.tok == Tok::Op("]") =>
                    {
                        for item in split_commas(items) {
                            match cap_expr(item) {
                                Some(Some(spec)) => {
                                    let cap = Subscription::parse(&spec).cap().to_owned();
                                    use_name(facts, cap)?;
                                }
                                Some(None) => {}
                                None => facts.dynamic = true,
                            }
                        }
                    }
                    _ => facts.dynamic = true,
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// (d): a whole wants document written as one string literal.
    fn serialized_document(
        text: &str,
        facts: &mut Facts,
        use_name: &mut dyn FnMut(&mut Facts, String) -> Result<(), UsedCapsError>,
    ) -> Result<(), UsedCapsError> {
        if !text.trim_start().starts_with('{') {
            return Ok(());
        }
        let Ok(Value::Object(doc)) = serde_json::from_str::<Value>(text) else {
            return Ok(());
        };
        if !doc.contains_key("requests") && !doc.contains_key("subscribe") {
            return Ok(());
        }
        if let Some(Value::Array(requests)) = doc.get("requests") {
            for request in requests {
                match request.get("cap") {
                    Some(Value::String(cap)) => use_name(facts, cap.clone())?,
                    _ => use_name(facts, request.to_string())?,
                }
            }
        }
        if let Some(Value::Array(subscribe)) = doc.get("subscribe") {
            for spec in subscribe {
                match spec {
                    Value::String(spec) => {
                        use_name(facts, Subscription::parse(spec).cap().to_owned())?;
                    }
                    other => use_name(facts, other.to_string())?,
                }
            }
        }
        Ok(())
    }
}

/// The platform's declared exports for an app (the fields of its
/// required program record), resolved to definitions; for a package,
/// every definition of its exposed modules.
fn declared_entry_points(
    closure: &Closure,
    sources: &dyn RocSources,
    scanner: &Scanner<'_>,
) -> Result<BTreeMap<String, Node>, UsedCapsError> {
    let app = &closure.modules[0];
    let mut out = BTreeMap::new();
    match app.kind.as_deref() {
        Some("app") => {
            let pf_rel = header_package_entries(&app.header)
                .into_iter()
                .find_map(|(_, path, platform)| platform.then_some(path))
                .ok_or_else(|| {
                    UsedCapsError::EntryPoints(format!("{} names no platform", app.path.display()))
                })?;
            let pf_path = sibling(&app.path, &pf_rel);
            let text = read_source(sources, &pf_path)?;
            let tokens = lex(&text, &pf_path)?;
            let (value, fields) = platform_required_fields(&tokens).ok_or_else(|| {
                UsedCapsError::EntryPoints(format!(
                    "{} declares no required program record",
                    pf_path.display()
                ))
            })?;
            if !header_exposes(&app.header).contains(&value) {
                return Err(UsedCapsError::EntryPoints(format!(
                    "{} does not provide {value}",
                    app.path.display()
                )));
            }
            let program = app.defs.get(&value).ok_or_else(|| {
                UsedCapsError::EntryPoints(format!(
                    "{} has no {value} definition",
                    app.path.display()
                ))
            })?;
            let record = match program.body.as_slice() {
                [open, inside @ .., close]
                    if open.tok == Tok::Op("{") && close.tok == Tok::Op("}") =>
                {
                    inside
                }
                _ => {
                    return Err(UsedCapsError::EntryPoints(format!(
                        "{}.{value} is not a record literal",
                        app.stem
                    )));
                }
            };
            let mut given: BTreeMap<String, Option<Node>> = BTreeMap::new();
            for part in split_commas(record) {
                let Some(field) = ident(part.first()) else {
                    continue;
                };
                let expr = if op(part.get(1), ":") {
                    &part[2..]
                } else {
                    &part[..1]
                };
                let target = match expr {
                    [one] => ident(Some(one))
                        .map(|name| scanner.resolve(0, name))
                        .transpose()
                        .map_err(|reference| UsedCapsError::UnresolvedReference {
                            definition: qname(&app.stem, &value),
                            reference,
                        })?
                        .flatten(),
                    _ => None,
                };
                given.insert(field.to_owned(), target);
            }
            for field in fields {
                let target = given.get(&field).ok_or_else(|| {
                    UsedCapsError::EntryPoints(format!(
                        "{}.{value} does not set the required field {field}",
                        app.stem
                    ))
                })?;
                // A field set to an inline expression seeds at the whole
                // program record (an over-approximation).
                let node = target.clone().unwrap_or_else(|| (0, value.clone()));
                out.insert(scanner.label(&node), node);
            }
        }
        Some("package") => {
            for name in header_exposes(&app.header) {
                let path = sibling(&app.path, &format!("{name}.roc"));
                let m =
                    *closure
                        .by_path
                        .get(&path)
                        .ok_or_else(|| UsedCapsError::UnresolvedImport {
                            file: app.path.clone(),
                            import: name.clone(),
                        })?;
                // A type module exposes its nominal type's members; its
                // top-level definitions are private to it.
                for def in closure.modules[m].defs.values().filter(|d| d.member) {
                    let node = (m, def.name.clone());
                    out.insert(scanner.label(&node), node);
                }
            }
        }
        _ => {
            return Err(UsedCapsError::EntryPoints(format!(
                "{} is neither an app nor a package",
                app.path.display()
            )));
        }
    }
    Ok(out)
}

/// Derive the used-set of the world or library whose entry module is
/// `entry` (§3, §4).
///
/// `entry_points`, when given, overrides the default
/// seed (the platform's declared exports) with module-qualified names
/// such as `main.init`; a seed other than the full export set yields a
/// partial-scope, uncertified record.
pub fn derive_used_caps(
    sources: &dyn RocSources,
    entry: &Path,
    entry_points: Option<&[String]>,
) -> Result<UsedCaps, UsedCapsError> {
    let closure = load_closure(sources, entry)?;
    let scanner = Scanner::new(&closure);
    let declared = declared_entry_points(&closure, sources, &scanner)?;
    let seeds: BTreeMap<String, Node> = match entry_points {
        None => declared.clone(),
        Some(names) => {
            let mut seeds = BTreeMap::new();
            for name in names {
                let (stem, def) = name.rsplit_once('.').ok_or_else(|| {
                    UsedCapsError::EntryPoints(format!("{name:?} is not module-qualified"))
                })?;
                let node = closure
                    .modules
                    .iter()
                    .enumerate()
                    .find(|(_, m)| m.stem == stem && m.defs.contains_key(def))
                    .map(|(m, _)| (m, def.to_owned()))
                    .ok_or_else(|| {
                        UsedCapsError::EntryPoints(format!("{name} names no definition"))
                    })?;
                seeds.insert(scanner.label(&node), node);
            }
            seeds
        }
    };
    if seeds.is_empty() {
        return Err(UsedCapsError::EntryPoints(
            "no entry point resolved".to_owned(),
        ));
    }
    let partial = seeds.keys().ne(declared.keys());

    // Reach (§4 step 3) and collect (step 4).
    let mut seen: BTreeSet<Node> = BTreeSet::new();
    let mut queue: VecDeque<Node> = seeds.values().cloned().collect();
    let mut used = BTreeSet::new();
    let mut dynamic = false;
    while let Some(node) = queue.pop_front() {
        if !seen.insert(node.clone()) {
            continue;
        }
        let facts = scanner.facts(&node)?;
        used.extend(facts.names);
        dynamic |= facts.dynamic;
        queue.extend(facts.edges.into_iter().filter(|n| !seen.contains(n)));
    }
    // Diagnostic: known names named only from unreachable definitions.
    // Dead code is never an error, so a failure there is ignored.
    let mut unreachable = BTreeSet::new();
    for (m, module) in closure.modules.iter().enumerate() {
        for name in module.defs.keys() {
            let node = (m, name.clone());
            if !seen.contains(&node)
                && let Ok(facts) = scanner.facts(&node)
            {
                unreachable.extend(facts.names);
            }
        }
    }
    if dynamic {
        used.insert(UNKNOWN_CAP_USAGE_KEY.to_owned());
    }
    UsedCaps::new(
        used,
        !dynamic && !partial,
        seeds.into_keys().collect(),
        partial,
        unreachable,
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    // A small platform in the shape of examples/roc-world: a program
    // record with three handlers, a JSON module and a capability helper
    // module whose builders embed catalog names (§2).
    const PLATFORM: &str = r#"platform ""
    requires {
        [Model : model] for program : {
            init : U64 -> model,
            update : model, U64, Str -> model,
            wants : model -> Str,
        }
    }
    exposes [Cap, Json]
    packages {}
    provides { "roc_init": init_for_host }

import Cap
import Json

init_for_host = |seed| Box.box((program.init)(seed))
"#;

    const JSON: &str = r#"## JSON values.
Json := [Null, String(Str), Array(List(Json)), Object(List({ key : Str, value : Json }))].{
	encode : Json -> Str
	encode = |value| match value {
		String(text) => text
		_ => ""
	}
}
"#;

    const CAP: &str = r#"import Json
Cap := { next : U64 }.{
	empty : Cap
	empty = { next: 0 }

	issue : Cap, Str -> { session : Cap, id : Str }
	issue = |session, cap| { session: session, id: cap }

	chat_request = |id| Json.Object([
		{ key: "id", value: Json.String(id) },
		{ key: "cap", value: Json.String("chat.say") },
	])

	read_request = |id| Json.Object([{ key: "id", value: Json.String(id) }, { key: "cap", value: Json.String("records.read") }])

	at_request = |id| Json.Object([{ key: "cap", value: Json.String("schedule.at") }])

	loc_request = |id| Json.Object([{ key: "cap", value: Json.String("loc.position") }])

	now_request = |id| Json.Object([{ key: "cap", value: Json.String("time.now") }])

	request = |id, cap| Json.Object([{ key: "id", value: Json.String(id) }, { key: "cap", value: Json.String(cap) }])
}
"#;

    const HEADER: &str = "app [program] { pf: platform \"../pf/platform.roc\" }\n\nimport pf.Cap\nimport pf.Json\n\nprogram = { init, update, wants }\n\n";

    fn tree(app: &str) -> BTreeMap<PathBuf, String> {
        BTreeMap::from([
            (PathBuf::from("/fx/pf/platform.roc"), PLATFORM.to_owned()),
            (PathBuf::from("/fx/pf/Json.roc"), JSON.to_owned()),
            (PathBuf::from("/fx/pf/Cap.roc"), CAP.to_owned()),
            (PathBuf::from("/fx/app/main.roc"), format!("{HEADER}{app}")),
        ])
    }

    /// An app with the given handler bodies; `update` and `wants` default
    /// to capability-free definitions.
    fn app(defs: &str) -> String {
        let mut out = defs.to_owned();
        if !defs.contains("\nupdate =") && !defs.starts_with("update =") {
            out.push_str("\nupdate = |model, _pid, _ev| model\n");
        }
        if !defs.contains("\nwants =") && !defs.starts_with("wants =") {
            out.push_str("\nwants = |_model| \"{}\"\n");
        }
        out
    }

    fn scan(defs: &str) -> Result<UsedCaps, UsedCapsError> {
        derive_used_caps(&tree(&app(defs)), Path::new("/fx/app/main.roc"), None)
    }

    fn names(list: &[&str]) -> BTreeSet<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    // §9.1
    #[test]
    fn direct_use_through_a_transitive_helper() {
        let used =
            scan("init = |_seed| helper(0)\n\nhelper = |n| Cap.read_request(\"r0\")\n").unwrap();
        assert_eq!(used.used(), &names(&["records.read"]));
        assert!(used.certified());
        assert!(!used.partial());
        assert_eq!(
            used.entry_points(),
            &names(&["main.init", "main.update", "main.wants"])
        );
    }

    // §9.2
    #[test]
    fn subscription_style_names_are_not_special_cased() {
        let used = scan("init = |_seed| Cap.at_request(\"r0\")\n").unwrap();
        assert_eq!(used.used(), &names(&["schedule.at"]));
        assert!(used.certified());
    }

    // §9.3, R2
    #[test]
    fn a_request_builder_passed_as_a_value_counts() {
        let used = scan("init = |_seed| keep(Cap.chat_request)\n\nkeep = |_f| 0\n").unwrap();
        assert_eq!(used.used(), &names(&["chat.say"]));
        assert!(used.certified());
    }

    // §9.4, R3
    #[test]
    fn a_local_helper_passed_as_a_value_counts() {
        let used = scan(
            "init = |_seed| keep(locate)\n\nkeep = |_f| 0\n\nlocate = |id| Cap.loc_request(id)\n",
        )
        .unwrap();
        assert_eq!(used.used(), &names(&["loc.position"]));
        assert!(used.certified());
    }

    // §9.5
    #[test]
    fn dead_code_is_not_used_and_is_reported_apart() {
        let used = scan("init = |_seed| 0\n\nunused = |id| Cap.now_request(id)\n").unwrap();
        assert!(used.used().is_empty());
        assert!(used.certified());
        assert!(used.unreachable_refs().contains("time.now"));
        let record = used.to_record("worlds/dead");
        assert!(record["unreachable_refs"].as_array().is_some());
    }

    // §9.6, R4
    #[test]
    fn a_computed_capability_name_marks_unknown_and_uncertifies() {
        let used = scan(
            "init = |seed| {\n\tread = Cap.read_request(\"r0\")\n\tCap.issue(Cap.empty, pick(seed))\n}\n\npick = |n| if n == 0 { \"chat.say\" } else { \"time.now\" }\n",
        )
        .unwrap();
        assert_eq!(
            used.used(),
            &names(&["records.read", UNKNOWN_CAP_USAGE_KEY])
        );
        assert!(!used.certified());
        assert!(!used.partial());
    }

    #[test]
    fn a_computed_cap_field_in_a_document_is_dynamic() {
        let used = scan(
            "init = |seed| Json.Object([{ key: \"cap\", value: Json.String(name(seed)) }])\n\nname = |_n| \"chat.say\"\n",
        )
        .unwrap();
        assert!(used.used().contains(UNKNOWN_CAP_USAGE_KEY));
        assert!(!used.certified());
    }

    #[test]
    fn forwarding_the_cap_parameter_is_not_dynamic() {
        // `request` forwards its own `cap` parameter into the document;
        // the literal is counted where `request` is called (position a).
        let used = scan(
            "init = |_seed| my_issue(Cap.empty, \"chat.say\")\n\nmy_issue = |session, cap| {\n\tissued = Cap.issue(session, cap)\n\tCap.request(issued.id, cap)\n}\n",
        )
        .unwrap();
        assert_eq!(used.used(), &names(&["chat.say"]));
        assert!(used.certified());
    }

    #[test]
    fn a_lambda_argument_does_not_shift_the_cap_position() {
        let used = scan(
            "init = |_seed| twice(|a, b| a, \"chat.say\")\n\ntwice = |f, cap| Cap.issue(Cap.empty, cap)\n",
        )
        .unwrap();
        assert_eq!(used.used(), &names(&["chat.say"]));
        assert!(used.certified());
    }

    // §9.7
    #[test]
    fn a_literal_outside_the_catalog_is_version_skew() {
        let err = scan("init = |_seed| Cap.issue(Cap.empty, \"chat.shout\")\n").unwrap_err();
        assert_eq!(
            err,
            UsedCapsError::VersionSkew {
                definition: "main.init".to_owned(),
                literal: "chat.shout".to_owned(),
            }
        );
        assert!(err.to_string().contains("chat.shout"));
    }

    // §9.8
    #[test]
    fn pairing_reports_both_sides_and_suppresses_over_declared_when_uncertified() {
        let declared = names(&["chat.say", "time.now"]);
        let certified = UsedCaps::new(
            names(&["chat.say", "records.read"]),
            true,
            names(&["main.init"]),
            false,
            BTreeSet::new(),
        )
        .unwrap();
        let pairing = certified.pair_with_declared(&declared);
        assert_eq!(pairing.under_declared, names(&["records.read"]));
        assert_eq!(pairing.over_declared, Some(names(&["time.now"])));
        assert!(!pairing.unknown_use);

        let uncertified = UsedCaps::new(
            names(&["chat.say", "records.read", UNKNOWN_CAP_USAGE_KEY]),
            false,
            names(&["main.init"]),
            false,
            BTreeSet::new(),
        )
        .unwrap();
        let pairing = uncertified.pair_with_declared(&declared);
        assert_eq!(pairing.under_declared, names(&["records.read"]));
        assert_eq!(pairing.over_declared, None);
        assert!(pairing.unknown_use);
    }

    // §9.9, R5
    #[test]
    fn a_value_definition_aggregate_counts_through_field_access() {
        let used =
            scan("init = |_seed| ops.read(\"r0\")\n\nops = { read: Cap.read_request }\n").unwrap();
        assert_eq!(used.used(), &names(&["records.read"]));
        assert!(used.certified());
    }

    // §9.10
    #[test]
    fn a_proper_subset_of_entry_points_is_a_partial_record() {
        let tree = tree(&app(
            "init = |_seed| 0\n\nupdate = |model, _pid, _ev| {\n\tdoc = Cap.chat_request(\"r0\")\n\tmodel\n}\n",
        ));
        let used = derive_used_caps(
            &tree,
            Path::new("/fx/app/main.roc"),
            Some(&["main.init".to_owned()]),
        )
        .unwrap();
        assert!(used.used().is_empty());
        assert!(!used.certified());
        assert!(used.partial());
        assert_eq!(used.entry_points(), &names(&["main.init"]));
        let record = used.to_record("worlds/partial");
        assert_eq!(record["scope"], "partial");
        let pairing = used.pair_with_declared(&names(&["chat.say"]));
        assert_eq!(pairing.over_declared, None);
        assert!(pairing.partial);

        // The whole export set, given explicitly, is not partial.
        let all = ["main.init", "main.update", "main.wants"].map(str::to_owned);
        let whole = derive_used_caps(&tree, Path::new("/fx/app/main.roc"), Some(&all)).unwrap();
        assert!(!whole.partial());
        assert!(whole.certified());
        assert_eq!(whole.used(), &names(&["chat.say"]));
    }

    // §9.11
    #[test]
    fn the_request_protocol_counts_the_issue_literal_and_the_builder() {
        let send = "init = |_seed| send(Cap.empty)\n\nsend = |session| {\n\tissued = Cap.issue(session, \"chat.say\")\n\tCap.chat_request(issued.id)\n}\n";
        let used = scan(send).unwrap();
        assert_eq!(used.used(), &names(&["chat.say"]));
        assert!(used.certified());

        let unreachable = send.replace("init = |_seed| send(Cap.empty)", "init = |_seed| 0");
        let used = scan(&unreachable).unwrap();
        assert!(used.used().is_empty());
        assert!(used.unreachable_refs().contains("chat.say"));
    }

    // §9.12
    #[test]
    fn a_serialized_wants_document_is_read_like_parse_wants() {
        let used = scan(
            "init = |_seed| 0\n\nwants = |_model| \"{\\\"v\\\":1,\\\"subscribe\\\":[\\\"players\\\"],\\\"requests\\\":[]}\"\n",
        )
        .unwrap();
        assert_eq!(used.used(), &names(&["players"]));
        assert!(used.certified());

        let err = scan(
            "init = |_seed| 0\n\nwants = |_model| \"{\\\"v\\\":1,\\\"subscribe\\\":[\\\"time.tick:soon\\\"],\\\"requests\\\":[]}\"\n",
        )
        .unwrap_err();
        assert_eq!(
            err,
            UsedCapsError::VersionSkew {
                definition: "main.wants".to_owned(),
                literal: "time.tick:soon".to_owned(),
            }
        );
    }

    #[test]
    fn a_subscription_list_is_normalised_by_the_runtime_parser() {
        let used = scan(
            "init = |_seed| 0\n\nwants = |_model| Json.encode(Json.Object([\n\t{ key: \"v\", value: Json.Null },\n\t{ key: \"subscribe\", value: Json.Array([Json.String(\"players\"), Json.String(\"time.tick:5000\")]) },\n]))\n",
        )
        .unwrap();
        assert_eq!(used.used(), &names(&["players", "time.tick"]));
        assert!(used.certified());

        let err = scan(
            "init = |_seed| 0\n\nwants = |_model| Json.Object([{ key: \"subscribe\", value: Json.Array([Json.String(\"players:extra\")]) }])\n",
        )
        .unwrap_err();
        assert!(
            matches!(err, UsedCapsError::VersionSkew { ref literal, .. } if literal == "players:extra")
        );
    }

    #[test]
    fn interpolated_references_stay_in_the_graph() {
        let used =
            scan("init = |_seed| \"doc ${Json.encode(Cap.read_request(\"r0\"))}\"\n").unwrap();
        assert_eq!(used.used(), &names(&["records.read"]));
    }

    #[test]
    fn records_are_byte_identical_across_runs() {
        let defs = "init = |_seed| helper(0)\n\nhelper = |n| Cap.read_request(\"r0\")\n";
        let a = scan(defs).unwrap().to_record_string("worlds/fixture");
        let b = scan(defs).unwrap().to_record_string("worlds/fixture");
        assert_eq!(a, b);
        assert_eq!(
            a,
            r#"{"certified":true,"entry_points":["main.init","main.update","main.wants"],"scope":"whole-world","subject":"worlds/fixture","unreachable_refs":["chat.say","loc.position","schedule.at","time.now"],"used":["records.read"],"v":1}"#
        );
    }

    #[test]
    fn records_round_trip_and_refuse_duplicate_keys_and_bad_invariants() {
        let used = scan("init = |_seed| Cap.read_request(\"r0\")\n").unwrap();
        let text = used.to_record_string("worlds/fixture");
        let (subject, parsed) = UsedCaps::parse_record(&text).unwrap();
        assert_eq!(subject, "worlds/fixture");
        assert_eq!(parsed, used);

        let dup = r#"{"v":1,"subject":"s","used":[],"used":["chat.say"],"certified":true,"entry_points":["main.init"],"scope":"whole-world"}"#;
        assert!(UsedCaps::parse_record(dup).is_err());
        let unknown_certified = r#"{"v":1,"subject":"s","used":["<unknown>"],"certified":true,"entry_points":["main.init"],"scope":"whole-world"}"#;
        assert!(UsedCaps::parse_record(unknown_certified).is_err());
        let partial_certified = r#"{"v":1,"subject":"s","used":[],"certified":true,"entry_points":["main.init"],"scope":"partial"}"#;
        assert!(UsedCaps::parse_record(partial_certified).is_err());
        let not_catalog = r#"{"v":1,"subject":"s","used":["chat.shout"],"certified":true,"entry_points":["main.init"],"scope":"whole-world"}"#;
        assert!(UsedCaps::parse_record(not_catalog).is_err());
    }

    #[test]
    fn input_errors_name_the_artifact() {
        let mut bad_import = tree(&app("init = |_seed| 0\n"));
        bad_import.insert(
            PathBuf::from("/fx/app/main.roc"),
            format!("{HEADER}import pf.Missing\n\n{}", app("init = |_seed| 0\n")),
        );
        assert!(matches!(
            derive_used_caps(&bad_import, Path::new("/fx/app/main.roc"), None),
            Err(UsedCapsError::UnresolvedImport { ref import, .. }) if import == "pf.Missing"
        ));

        let tree = tree(&app("init = |_seed| 0\n"));
        assert!(matches!(
            derive_used_caps(&tree, Path::new("/fx/app/main.roc"), Some(&[])),
            Err(UsedCapsError::EntryPoints(_))
        ));
        assert!(matches!(
            derive_used_caps(
                &tree,
                Path::new("/fx/app/main.roc"),
                Some(&["main.nope".to_owned()])
            ),
            Err(UsedCapsError::EntryPoints(_))
        ));
        assert!(matches!(
            derive_used_caps(&tree, Path::new("/fx/app/missing.roc"), None),
            Err(UsedCapsError::Unparseable { .. })
        ));
        assert!(matches!(
            scan("init = |_seed| Cap.no_such_member(0)\n"),
            Err(UsedCapsError::UnresolvedReference { ref reference, .. }) if reference == "Cap.no_such_member"
        ));
    }

    // The scan over the real example worlds, whose expected sets are
    // read off their sources by hand.
    fn example(rel: &str) -> UsedCaps {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples")
            .join(rel);
        derive_used_caps(&FsSources, &path, None).unwrap()
    }

    #[test]
    fn example_worlds_derive_their_hand_computed_sets() {
        // roc-chat issues `chat.history` and `chat.say` (RocCap.issue)
        // and builds both documents through RocCap's builders.
        let chat = example("roc-chat/main.roc");
        assert_eq!(chat.used(), &names(&["chat.history", "chat.say"]));
        assert!(chat.certified());
        // territory's wants is one serialized document subscribing to players.
        assert_eq!(example("territory/main.roc").used(), &names(&["players"]));
        // The cap-demo app reaches records.read only through its library.
        assert_eq!(
            example("roc-cap-demo/main.roc").used(),
            &names(&["records.read"])
        );
        // The library on its own: every Greeter member is an entry point.
        let lib = example("roc-cap-demo/lib/main.roc");
        assert_eq!(lib.used(), &names(&["records.read"]));
        assert!(lib.entry_points().contains("Greeter.ask_for_names"));
        // Worlds that use no capability derive the empty, certified set.
        for world in [
            "roc-counter/main.roc",
            "roc-grid/main.roc",
            "roc-screen/main.roc",
        ] {
            let used = example(world);
            assert!(used.used().is_empty(), "{world}");
            assert!(used.certified(), "{world}");
        }
    }
}
