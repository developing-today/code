//! Library manifest (`#5969` §2 KDL profile) parsing and validation, with
//! the platform-requirement semantics `#5956` §1/§7 step 1 defines.
//!
//! A library declares the platform functions it uses in the `platform`
//! block of its manifest, by catalog name — an entry of
//! [`crate::world_caps::PRODUCTION_CATALOG_ENTRIES`], either a platform
//! module (`RocCap`) or a capability (`records.read`). This module is the
//! fail-closed front door for that declaration, mirroring
//! [`crate::world_manifest::WorldManifest::parse`]: unknown requirement
//! names, duplicate requirements, wildcard names, and version pins that
//! disagree with the compiling catalog are all refused at parse time.
//! An absent or empty `platform` block is a pure library — purity is the
//! empty requirement set, never a self-asserted label (`#5994` §5.5).
//!
//! Scope of this step (`#10027`, `#5956` §7 step 1): parse + validate
//! only. The transitive resolver (§7 step 2) and the compile-path import
//! gate (§7 step 3) consume [`LibraryManifest::requirements`] later;
//! there is no registry/upload code path in the tree yet for this
//! parser to wire into, so it is exposed as the `id-caps` seam those
//! consumers will call.

use anyhow::{Result, bail, ensure};
use std::collections::BTreeSet;

use crate::world_caps::PRODUCTION_CATALOG_ENTRIES;

/// The registry upload kinds a library manifest may declare (`#5969` §1).
/// `world` is deliberately not a variant: a world runs, a library is
/// depended on, and a manifest claiming kind `world` is refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LibraryKind {
    /// A precompiled library artifact, loaded as a DLL.
    DllLibrary,
    /// Library source, compiled into the consuming program.
    SourceLibrary,
    /// Nothing but a manifest pointing at an external URL (`#5969` §6).
    UrlLibrary,
}

impl LibraryKind {
    /// The KDL spelling of this kind.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::DllLibrary => "dll-library",
            Self::SourceLibrary => "source-library",
            Self::UrlLibrary => "url-library",
        }
    }

    fn parse(text: &str) -> Result<Self> {
        match text {
            "dll-library" => Ok(Self::DllLibrary),
            "source-library" => Ok(Self::SourceLibrary),
            "url-library" => Ok(Self::UrlLibrary),
            other => bail!(
                "library kind {other:?} is not a library kind (expected dll-library, source-library, or url-library)"
            ),
        }
    }
}

/// How a library is built for its consumers (`#5969` §5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuildMode {
    /// Built to be loaded as a DLL.
    Dll,
    /// Built to be compiled into another program.
    CompileIn,
    /// Both forms published under one manifest.
    Dual,
}

impl BuildMode {
    /// The KDL spelling of this build mode.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Dll => "dll",
            Self::CompileIn => "compile-in",
            Self::Dual => "dual",
        }
    }

    fn parse(text: &str) -> Result<Self> {
        match text {
            "dll" => Ok(Self::Dll),
            "compile-in" => Ok(Self::CompileIn),
            "dual" => Ok(Self::Dual),
            other => bail!(
                "build-mode {other:?} is not a build mode (expected dll, compile-in, or dual)"
            ),
        }
    }
}

/// One `requires` entry of a manifest `platform` block: a catalog name,
/// optionally pinned to the catalog version the library was written
/// against (`#5956` §1 — pins are optional and exact).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlatformRequirement {
    /// The catalog entry name (a platform module or a capability).
    pub name: String,
    /// The pinned catalog version, if the declaration pins one.
    pub version_pin: Option<u32>,
}

/// A `subtype` conformance declaration (`#5969` §4), carried structurally;
/// conformance checking against the pinned interface is registry work,
/// not this parser's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubtypeDecl {
    /// The published interface name (e.g. `chess/timed-game`).
    pub name: String,
    /// The interface version, if declared.
    pub version: Option<String>,
    /// The interface content hash, if declared.
    pub hash: Option<String>,
}

/// An `attach` entity-attachment declaration (`#5969` §3), carried
/// structurally; attachment semantics are registry work.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttachmentDecl {
    /// The entity the library attaches to (e.g. `genre:chess`).
    pub target: String,
    /// The attachment mode (`discover` by default, or `restrict`).
    pub mode: Option<String>,
}

/// A parsed, validated library manifest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LibraryManifest {
    /// Author-scoped local name; identity is the hash, not the name.
    pub name: String,
    /// Author-assigned version string.
    pub version: String,
    /// The upload kind (`#5969` §1).
    pub kind: LibraryKind,
    /// The build mode (`#5969` §5).
    pub build_mode: BuildMode,
    /// Content hash of the artifact / source closure / pinned bytes.
    pub hash: String,
    /// License string (vocabulary owned by the licensing children).
    pub license: String,
    /// The declared platform-requirement set, in declaration order.
    /// Empty means the library is pure.
    pub requirements: Vec<PlatformRequirement>,
    /// Declared subtype conformances (`#5969` §4).
    pub subtypes: Vec<SubtypeDecl>,
    /// Declared entity attachments (`#5969` §3).
    pub attachments: Vec<AttachmentDecl>,
    /// The external URL, for `url-library` manifests only (`#5969` §6).
    pub source_url: Option<String>,
}

impl LibraryManifest {
    /// Parse a KDL library manifest and validate it fail-closed:
    /// required fields present, kind/build-mode agreement (`#5969` §5),
    /// and every platform requirement naming a real catalog entry at a
    /// compatible version (`#5956` §1).
    ///
    /// # Errors
    ///
    /// Names the first offending field, requirement, or node.
    pub fn parse(text: &str) -> Result<Self> {
        let nodes = Parser::new(text).parse_document()?;
        ensure!(
            nodes.len() == 1 && nodes[0].name == "library",
            "a library manifest is exactly one `library` node"
        );
        let root = &nodes[0];
        let name = root
            .string_arg()
            .ok_or_else(|| anyhow::anyhow!("library node requires a name string argument"))?
            .to_string();
        let version = root
            .prop_string("version")
            .ok_or_else(|| anyhow::anyhow!("library {name:?} is missing its version property"))?
            .to_string();

        let mut kind = None;
        let mut build_mode = None;
        let mut hash = None;
        let mut license = None;
        let mut requirements = Vec::new();
        let mut subtypes = Vec::new();
        let mut attachments = Vec::new();
        let mut source_url = None;
        let mut platform_seen = false;

        for child in &root.children {
            match child.name.as_str() {
                "kind" => set_once(&mut kind, child, LibraryKind::parse)?,
                "build-mode" => set_once(&mut build_mode, child, BuildMode::parse)?,
                "hash" => set_string_once(&mut hash, child)?,
                "license" => set_string_once(&mut license, child)?,
                "platform" => {
                    ensure!(
                        !platform_seen,
                        "duplicate platform block in library {name:?}"
                    );
                    platform_seen = true;
                    requirements = parse_platform_block(child)?;
                }
                "subtype" => subtypes.push(parse_subtype(child)?),
                "attach" => attachments.push(parse_attach(child)?),
                "source" => {
                    ensure!(
                        source_url.is_none(),
                        "duplicate source node in library {name:?}"
                    );
                    source_url = Some(
                        child
                            .prop_string("url")
                            .ok_or_else(|| anyhow::anyhow!("source node requires a url property"))?
                            .to_string(),
                    );
                }
                other => bail!("unknown node {other:?} in library manifest {name:?}"),
            }
        }

        // Required fields (#5969 §2): a manifest without a content hash
        // is not a library — it is a claim about a library.
        let kind = kind.ok_or_else(|| anyhow::anyhow!("library {name:?} is missing kind"))?;
        let build_mode =
            build_mode.ok_or_else(|| anyhow::anyhow!("library {name:?} is missing build-mode"))?;
        let hash = hash.ok_or_else(|| anyhow::anyhow!("library {name:?} is missing hash"))?;
        let license =
            license.ok_or_else(|| anyhow::anyhow!("library {name:?} is missing license"))?;
        ensure!(!hash.is_empty(), "library {name:?} hash must not be empty");

        // Build mode must agree with kind (#5969 §5): dll mode goes with
        // a dll-library (or the DLL form of a dual), compile-in with a
        // source-library (or its source form); a url-library declares
        // the mode its pinned bytes take once fetched, so any mode goes.
        let agrees = match (kind, build_mode) {
            (LibraryKind::DllLibrary, BuildMode::Dll | BuildMode::Dual)
            | (LibraryKind::SourceLibrary, BuildMode::CompileIn | BuildMode::Dual)
            | (LibraryKind::UrlLibrary, _) => true,
            (LibraryKind::DllLibrary | LibraryKind::SourceLibrary, _) => false,
        };
        ensure!(
            agrees,
            "build-mode {:?} does not agree with kind {:?} (#5969 §5)",
            build_mode.as_str(),
            kind.as_str()
        );
        if kind == LibraryKind::UrlLibrary {
            ensure!(
                source_url.is_some(),
                "url-library {name:?} requires a source url (#5969 §6)"
            );
        }

        Ok(Self {
            name,
            version,
            kind,
            build_mode,
            hash,
            license,
            requirements,
            subtypes,
            attachments,
            source_url,
        })
    }

    /// True when the library declares no platform requirements — the
    /// pure case, which is the empty set, never a label (`#5994` §5.5).
    #[must_use]
    pub fn is_pure(&self) -> bool {
        self.requirements.is_empty()
    }

    /// The declared requirement names as a set.
    #[must_use]
    pub fn requirement_names(&self) -> BTreeSet<&str> {
        self.requirements.iter().map(|r| r.name.as_str()).collect()
    }
}

/// Set a typed field from a single-string-argument node, refusing a
/// duplicate node or a missing/extra argument.
fn set_once<T>(slot: &mut Option<T>, node: &Node, parse: fn(&str) -> Result<T>) -> Result<()> {
    ensure!(slot.is_none(), "duplicate {} node", node.name);
    let value = node
        .string_arg()
        .ok_or_else(|| anyhow::anyhow!("{} node requires one string argument", node.name))?;
    *slot = Some(parse(value)?);
    Ok(())
}

/// Set a string field from a single-string-argument node, refusing a
/// duplicate node or a missing argument.
fn set_string_once(slot: &mut Option<String>, node: &Node) -> Result<()> {
    ensure!(slot.is_none(), "duplicate {} node", node.name);
    let value = node
        .string_arg()
        .ok_or_else(|| anyhow::anyhow!("{} node requires one string argument", node.name))?;
    *slot = Some(value.to_string());
    Ok(())
}

/// Parse and validate a `platform` block: only `requires` children,
/// each naming a catalog entry exactly (`#5956` §1).
fn parse_platform_block(node: &Node) -> Result<Vec<PlatformRequirement>> {
    let mut requirements = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for child in &node.children {
        ensure!(
            child.name == "requires",
            "unknown node {:?} in platform block (only requires is defined)",
            child.name
        );
        let name = child
            .string_arg()
            .ok_or_else(|| anyhow::anyhow!("requires node requires one string argument"))?
            .to_string();
        ensure!(
            !name.contains('*'),
            "requirement {name:?} is a wildcard; requirements are exact catalog names (#5956 §1)"
        );
        ensure!(
            seen.insert(name.clone()),
            "duplicate requirement {name:?} in platform block"
        );
        let entry = PRODUCTION_CATALOG_ENTRIES
            .iter()
            .find(|e| e.name == name)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "requirement {name:?} names no catalog entry; unknown names fail closed (#5956 §1)"
                )
            })?;
        let version_pin = child.prop_int("version")?;
        if let Some(pin) = version_pin {
            ensure!(
                pin == entry.version,
                "requirement {name:?} pins catalog version {pin} but the catalog carries version {}",
                entry.version
            );
        }
        requirements.push(PlatformRequirement { name, version_pin });
    }
    Ok(requirements)
}

/// Parse a `subtype` node: name argument plus optional version/hash props.
fn parse_subtype(node: &Node) -> Result<SubtypeDecl> {
    let name = node
        .string_arg()
        .ok_or_else(|| anyhow::anyhow!("subtype node requires one string argument"))?
        .to_string();
    Ok(SubtypeDecl {
        name,
        version: node.prop_string("version").map(str::to_string),
        hash: node.prop_string("hash").map(str::to_string),
    })
}

/// Parse an `attach` node: target argument plus optional mode prop.
fn parse_attach(node: &Node) -> Result<AttachmentDecl> {
    let target = node
        .string_arg()
        .ok_or_else(|| anyhow::anyhow!("attach node requires one string argument"))?
        .to_string();
    Ok(AttachmentDecl {
        target,
        mode: node.prop_string("mode").map(str::to_string),
    })
}

/// One node of the KDL subset this profile uses.
#[derive(Debug)]
struct Node {
    name: String,
    args: Vec<Value>,
    props: Vec<(String, Value)>,
    children: Vec<Node>,
}

impl Node {
    fn string_arg(&self) -> Option<&str> {
        match self.args.as_slice() {
            [Value::Str(s)] => Some(s),
            _ => None,
        }
    }

    fn prop_string(&self, key: &str) -> Option<&str> {
        self.props.iter().find_map(|(k, v)| {
            if k == key {
                if let Value::Str(s) = v {
                    Some(s.as_str())
                } else {
                    None
                }
            } else {
                None
            }
        })
    }

    fn prop_int(&self, key: &str) -> Result<Option<u32>> {
        for (k, v) in &self.props {
            if k == key {
                return match v {
                    Value::Int(i) => {
                        Ok(Some(u32::try_from(*i).map_err(|_| {
                            anyhow::anyhow!("property {key} must not be negative")
                        })?))
                    }
                    _ => bail!("property {key} must be an integer"),
                };
            }
        }
        Ok(None)
    }
}

/// A scalar value in the KDL subset: string or integer.
#[derive(Debug)]
enum Value {
    Str(String),
    Int(i64),
}

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Ident(String),
    Str(String),
    Int(i64),
    Eq,
    LBrace,
    RBrace,
    Semi,
    Newline,
    Eof,
}

/// A hand-rolled parser for the KDL subset the `#5969` §2 profile uses:
/// identifiers, string/integer values, `key=value` properties, nested
/// blocks, `//` line comments, and newline/`;` node terminators. There
/// is no KDL crate in the workspace today; this subset covers exactly
/// the manifest profile and fails closed on anything outside it.
struct Parser {
    toks: Vec<Tok>,
    pos: usize,
}

impl Parser {
    fn new(text: &str) -> Self {
        Self {
            toks: lex(text),
            pos: 0,
        }
    }

    fn peek(&self) -> &Tok {
        &self.toks[self.pos]
    }

    fn next_tok(&mut self) -> Tok {
        let tok = self.toks[self.pos].clone();
        if self.pos + 1 < self.toks.len() {
            self.pos += 1;
        }
        tok
    }

    fn parse_document(mut self) -> Result<Vec<Node>> {
        let nodes = self.parse_nodes(true)?;
        ensure!(
            matches!(self.peek(), Tok::Eof),
            "trailing tokens after manifest"
        );
        Ok(nodes)
    }

    fn skip_terminators(&mut self) {
        while matches!(self.peek(), Tok::Semi | Tok::Newline) {
            self.next_tok();
        }
    }

    fn parse_nodes(&mut self, top: bool) -> Result<Vec<Node>> {
        let mut nodes = Vec::new();
        loop {
            self.skip_terminators();
            match self.peek() {
                Tok::Eof => {
                    ensure!(top, "unterminated block: missing }}");
                    break;
                }
                Tok::RBrace => {
                    ensure!(!top, "unmatched }} at top level");
                    break;
                }
                Tok::Ident(_) => nodes.push(self.parse_node()?),
                other => bail!("expected a node name, found {other:?}"),
            }
        }
        Ok(nodes)
    }

    fn parse_node(&mut self) -> Result<Node> {
        let name = match self.next_tok() {
            Tok::Ident(s) => s,
            other => bail!("expected a node name, found {other:?}"),
        };
        let mut node = Node {
            name,
            args: Vec::new(),
            props: Vec::new(),
            children: Vec::new(),
        };
        loop {
            match self.peek().clone() {
                Tok::Str(_) | Tok::Int(_) => match self.next_tok() {
                    Tok::Str(s) => node.args.push(Value::Str(s)),
                    Tok::Int(i) => node.args.push(Value::Int(i)),
                    other => bail!("expected a value, found {other:?}"),
                },
                Tok::Ident(key) => {
                    // A bare identifier in argument position is a
                    // property key only when followed by `=`; otherwise
                    // this node's terminator was missed — fail closed.
                    ensure!(
                        matches!(self.toks.get(self.pos + 1), Some(Tok::Eq)),
                        "bare identifier {key:?} in node {:?}; properties need key=value",
                        node.name
                    );
                    self.next_tok();
                    self.next_tok();
                    let value = match self.next_tok() {
                        Tok::Str(s) => Value::Str(s),
                        Tok::Int(i) => Value::Int(i),
                        other => bail!(
                            "property {key} requires a string or integer value, found {other:?}"
                        ),
                    };
                    node.props.push((key, value));
                }
                Tok::LBrace => {
                    self.next_tok();
                    node.children = self.parse_nodes(false)?;
                    match self.next_tok() {
                        Tok::RBrace => {}
                        other => bail!("expected }} closing node {:?}, found {other:?}", node.name),
                    }
                    break;
                }
                Tok::Semi | Tok::Newline | Tok::Eof | Tok::RBrace => break,
                other => bail!("unexpected token {other:?} in node {:?}", node.name),
            }
        }
        Ok(node)
    }
}

/// Tokenize the KDL subset. Newlines are significant (node terminators).
fn lex(text: &str) -> Vec<Tok> {
    let bytes = text.as_bytes();
    let mut toks = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        match b {
            b' ' | b'\t' | b'\r' => i += 1,
            b'\n' => {
                toks.push(Tok::Newline);
                i += 1;
            }
            b';' => {
                toks.push(Tok::Semi);
                i += 1;
            }
            b'{' => {
                toks.push(Tok::LBrace);
                i += 1;
            }
            b'}' => {
                toks.push(Tok::RBrace);
                i += 1;
            }
            b'=' => {
                toks.push(Tok::Eq);
                i += 1;
            }
            b'/' if i + 1 < bytes.len() && bytes[i + 1] == b'/' => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            b'"' => {
                let (s, next) = lex_string(text, i);
                toks.push(Tok::Str(s));
                i = next;
            }
            b'0'..=b'9' | b'-' => {
                let start = i;
                i += 1;
                while i < bytes.len() && bytes[i].is_ascii_digit() {
                    i += 1;
                }
                let text_int = &text[start..i];
                if let Ok(v) = text_int.parse::<i64>() {
                    toks.push(Tok::Int(v));
                }
            }
            _ if b.is_ascii_alphabetic() || b == b'_' || b == b'-' => {
                let start = i;
                while i < bytes.len()
                    && (bytes[i].is_ascii_alphanumeric()
                        || matches!(bytes[i], b'_' | b'-' | b'.' | b'/'))
                {
                    i += 1;
                }
                toks.push(Tok::Ident(text[start..i].to_string()));
            }
            _ => i += 1,
        }
    }
    toks.push(Tok::Eof);
    toks
}

/// Lex a `"…"` string starting at `start` (the opening quote), returning
/// the decoded string and the index just past the closing quote.
/// Supports the common escapes (`\"`, `\\`, `\n`, `\t`, `\r`); an
/// unterminated string yields what was scanned (the parser then fails
/// closed on the missing terminator structure).
fn lex_string(text: &str, start: usize) -> (String, usize) {
    let bytes = text.as_bytes();
    let mut out = String::new();
    let mut i = start + 1;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => return (out, i + 1),
            b'\\' if i + 1 < bytes.len() => {
                match bytes[i + 1] {
                    b'n' => out.push('\n'),
                    b't' => out.push('\t'),
                    b'r' => out.push('\r'),
                    other => out.push(other as char),
                }
                i += 2;
            }
            _ => {
                // Copy one UTF-8 character.
                let ch_len = utf8_len(bytes[i]);
                out.push_str(&text[i..i + ch_len]);
                i += ch_len;
            }
        }
    }
    (out, i)
}

/// Length in bytes of the UTF-8 character starting with lead byte `b`.
fn utf8_len(b: u8) -> usize {
    if b < 0x80 {
        1
    } else if b >> 5 == 0b110 {
        2
    } else if b >> 4 == 0b1110 {
        3
    } else {
        4
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    /// The `#5956` §1 example manifest shape, on the `#5969` §2 profile.
    const GREETER: &str = r##"
library "greeter" version="1.0.0" {
    kind "source-library"
    build-mode "compile-in"
    hash "blake3:9f2"
    license "MIT"
    platform {
        requires "RocCap"
        requires "records.read"
        requires "time.now" version=1
    }
}
"##;

    #[test]
    fn parses_the_design_example() {
        // Oracle: #5969 §2 profile + #5956 §1 example — the requirement
        // set is exactly the three declared catalog names, with the
        // version pin carried on time.now only.
        let m = LibraryManifest::parse(GREETER).unwrap();
        assert_eq!(m.name, "greeter");
        assert_eq!(m.kind, LibraryKind::SourceLibrary);
        assert_eq!(m.build_mode, BuildMode::CompileIn);
        assert_eq!(
            m.requirement_names(),
            BTreeSet::from(["RocCap", "records.read", "time.now"])
        );
        let pinned: Vec<Option<u32>> = m.requirements.iter().map(|r| r.version_pin).collect();
        assert_eq!(pinned, vec![None, None, Some(1)]);
        assert!(!m.is_pure());
    }

    #[test]
    fn absent_platform_block_is_pure() {
        // Oracle: #5956 §1 / #5994 §5.5 — purity is the empty set; an
        // absent platform block parses to zero requirements.
        let text = r#"
library "pure-lib" version="0.1.0" {
    kind "dll-library"
    build-mode "dll"
    hash "blake3:abc"
    license "MIT"
}
"#;
        let m = LibraryManifest::parse(text).unwrap();
        assert!(m.is_pure());
        assert_eq!(m.requirements, [] as [library_manifest::PlatformRequirement; 0]);
    }

    #[test]
    fn unknown_requirement_fails_closed() {
        // Oracle: #5956 §1 — a requires naming no catalog entry is
        // refused, the WorldManifest::parse posture.
        let text = GREETER.replace("records.read", "records.bogus");
        let err = LibraryManifest::parse(&text).unwrap_err().to_string();
        assert!(
            err.contains("records.bogus"),
            "error names the excess: {err}"
        );
        assert!(err.contains("no catalog entry"), "{err}");
    }

    #[test]
    fn wildcard_requirement_is_refused() {
        // Oracle: #5956 §1 — exact names, no wildcards.
        let text = GREETER.replace("records.read", "records.*");
        let err = LibraryManifest::parse(&text).unwrap_err().to_string();
        assert!(err.contains("wildcard"), "{err}");
    }

    #[test]
    fn duplicate_requirement_is_refused() {
        // Oracle: #5956 §7 step 1 — duplicate checks are part of the
        // validator's named job.
        let text = GREETER.replace(
            "requires \"time.now\" version=1",
            "requires \"time.now\" version=1\n        requires \"RocCap\"",
        );
        let err = LibraryManifest::parse(&text).unwrap_err().to_string();
        assert!(err.contains("duplicate requirement"), "{err}");
        assert!(err.contains("RocCap"), "{err}");
    }

    #[test]
    fn version_pin_mismatch_is_named() {
        // Oracle: #5956 §1 — a pinned requirement fails if the
        // compiling catalog carries a different version, with the
        // mismatch named. Every catalog entry is version 1 today.
        let text = GREETER.replace("version=1", "version=2");
        let err = LibraryManifest::parse(&text).unwrap_err().to_string();
        assert!(err.contains("time.now"), "{err}");
        assert!(err.contains("pins catalog version 2"), "{err}");
        assert!(err.contains("catalog carries version 1"), "{err}");
    }

    #[test]
    fn missing_hash_is_refused() {
        // Oracle: #5969 §2 — a manifest without a content hash is not
        // a library; the registry refuses it.
        let text = GREETER.replace("    hash \"blake3:9f2\"\n", "");
        let err = LibraryManifest::parse(&text).unwrap_err().to_string();
        assert!(err.contains("missing hash"), "{err}");
    }

    #[test]
    fn kind_build_mode_disagreement_is_refused() {
        // Oracle: #5969 §5 — build mode must agree with kind
        // (dll mode goes with dll-library / dual form only).
        let text = GREETER.replace("build-mode \"compile-in\"", "build-mode \"dll\"");
        let err = LibraryManifest::parse(&text).unwrap_err().to_string();
        assert!(err.contains("does not agree"), "{err}");
    }

    #[test]
    fn unknown_child_node_fails_closed() {
        // Oracle: fail-closed posture mirroring WorldManifest::parse —
        // a node the profile does not define is refused, not ignored.
        let text = GREETER.replace("license \"MIT\"", "license \"MIT\"\n    bogus \"x\"");
        let err = LibraryManifest::parse(&text).unwrap_err().to_string();
        assert!(err.contains("unknown node"), "{err}");
        assert!(err.contains("bogus"), "{err}");
    }

    #[test]
    fn url_library_requires_source_url() {
        // Oracle: #5969 §6 — a url-library is a manifest pointing at an
        // external URL; without the URL there is nothing to pin.
        let text = r#"
library "remote" version="1.0.0" {
    kind "url-library"
    build-mode "dll"
    hash "blake3:def"
    license "MIT"
}
"#;
        let err = LibraryManifest::parse(text).unwrap_err().to_string();
        assert!(err.contains("source url"), "{err}");
        let with_url = text.replace(
            "license \"MIT\"",
            "license \"MIT\"\n    source url=\"https://example.invalid/remote.kdl\"",
        );
        let m = LibraryManifest::parse(&with_url).unwrap();
        assert_eq!(
            m.source_url.as_deref(),
            Some("https://example.invalid/remote.kdl")
        );
    }
}
