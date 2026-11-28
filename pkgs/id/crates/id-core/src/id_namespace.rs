//! Namespace declarations and `id://` URL → namespace selection
//! (#10981; design #9117 §4,
//! `doc/2026-11-17T04-55-00Z_design_id9117_id_uri_scheme.md`).
//!
//! A server keeps its own [`NamespaceRegistry`]. There is no global
//! registry: the same URL text resolving differently on different
//! servers is by design (S4.6, S4.7). Selection runs in the record's
//! order:
//!
//! 1. the URL's authority must be this registry's server; a URL for
//!    another server is refused, never looked up here as a guess;
//! 2. the first path segment names a namespace in this registry; an
//!    unknown name is a resolution error, with no fallback;
//! 3. the namespace's declared query parameters are the only ones
//!    allowed; an undeclared parameter is rejected, not ignored, so a
//!    typo fails loudly instead of naming a different object;
//! 4. the namespace's declared encoding turns the remaining segments
//!    into its core's address items, and its handler is what the caller
//!    dispatches to.
//!
//! New encodings and handlers are registry entries, not grammar changes
//! (§6): an [`Encoding::Extension`] this module cannot decode is a
//! resolution error on that server, and adding one never changes how
//! existing URLs parse.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;

use crate::id_uri::{IdUri, IdUriError, MAX_ID_URI_LEN};

/// The addressing core a namespace path encodes (§4 `core_type`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CoreType {
    /// The array-address core (#9118).
    ArrayAddress,
    /// A plain hierarchical path.
    Hierarchical,
    /// A content hash.
    Hash,
    /// A core registered by the server under this identifier.
    Extension(String),
}

/// How core addresses map to and from path segments (§4 `encoding`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Encoding {
    /// Human-readable UTF-8 text, percent-encoded where needed (the default).
    Text,
    /// base64url without padding (RFC 4648 §5) of each segment's bytes.
    B64,
    /// An encoding registered by the server under this identifier.
    Extension(String),
}

impl fmt::Display for Encoding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Text => f.write_str("text"),
            Self::B64 => f.write_str("b64"),
            Self::Extension(id) => f.write_str(id),
        }
    }
}

/// A handler identifier from the server's handler registry.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct HandlerId(pub String);

/// Who may perform an operation in a namespace.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Access {
    /// Anyone.
    Anyone,
    /// Nobody (for example, a read-only namespace's writes).
    Nobody,
    /// Whoever the server's named policy admits.
    Policy(String),
}

/// A namespace's resolution and write rules (§4 `rules`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamespaceRules {
    /// Who may resolve paths in the namespace.
    pub resolve: Access,
    /// Who may write paths in the namespace.
    pub write: Access,
    /// Whether text paths are case-sensitive. When false, `text`
    /// segments are compared lowercased. It has no effect on `b64`,
    /// whose alphabet is case-significant.
    pub case_sensitive_paths: bool,
    /// The query parameters the namespace declares; any other is rejected.
    pub query_params: BTreeSet<String>,
    /// A URL length limit lower than [`MAX_ID_URI_LEN`], if the namespace sets one.
    pub max_url_len: Option<usize>,
}

impl Default for NamespaceRules {
    fn default() -> Self {
        Self {
            resolve: Access::Anyone,
            write: Access::Nobody,
            case_sensitive_paths: true,
            query_params: BTreeSet::new(),
            max_url_len: None,
        }
    }
}

/// One namespace definition (§4, S4.18).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamespaceDeclaration {
    name: String,
    core_type: CoreType,
    handler: HandlerId,
    encoding: Encoding,
    rules: NamespaceRules,
}

/// Why a namespace declaration or registration was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DeclarationError {
    /// The name is not `ALPHA *( ALPHA / DIGIT / "-" )`.
    #[error("namespace name {0:?} is invalid")]
    BadName(String),
    /// The namespace tried to raise the URL length limit.
    #[error("namespace URL limit {limit} is over the {max}-byte maximum")]
    LimitTooHigh {
        /// The declared limit.
        limit: usize,
        /// The maximum a namespace may declare.
        max: usize,
    },
    /// The registry already has a namespace with this name.
    #[error("namespace {0:?} is already declared on this server")]
    Duplicate(String),
}

impl NamespaceDeclaration {
    /// A declaration; the name is lowercased and checked against the
    /// namespace grammar, and a URL limit may only lower the default.
    pub fn new(
        name: &str,
        core_type: CoreType,
        handler: HandlerId,
        encoding: Encoding,
        rules: NamespaceRules,
    ) -> Result<Self, DeclarationError> {
        let lower = name.to_ascii_lowercase();
        let mut bytes = lower.bytes();
        let first_is_alpha = bytes.next().is_some_and(|b| b.is_ascii_alphabetic());
        if !first_is_alpha || !bytes.all(|b| b.is_ascii_alphanumeric() || b == b'-') {
            return Err(DeclarationError::BadName(name.to_owned()));
        }
        if let Some(limit) = rules.max_url_len
            && limit > MAX_ID_URI_LEN
        {
            return Err(DeclarationError::LimitTooHigh {
                limit,
                max: MAX_ID_URI_LEN,
            });
        }
        Ok(Self {
            name: lower,
            core_type,
            handler,
            encoding,
            rules,
        })
    }

    /// The namespace name, lowercase.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The addressing core.
    #[must_use]
    pub const fn core_type(&self) -> &CoreType {
        &self.core_type
    }

    /// The handler that resolves and writes paths here.
    #[must_use]
    pub const fn handler(&self) -> &HandlerId {
        &self.handler
    }

    /// The path encoding.
    #[must_use]
    pub const fn encoding(&self) -> &Encoding {
        &self.encoding
    }

    /// The resolution and write rules.
    #[must_use]
    pub const fn rules(&self) -> &NamespaceRules {
        &self.rules
    }
}

/// The server a registry belongs to: an `id://` authority.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ServerAuthority {
    host: String,
    port: Option<u16>,
}

impl ServerAuthority {
    /// The authority of `id://{authority}/…`, normalised the way the URL
    /// parser normalises it (lowercase, IDNA as punycode).
    pub fn parse(authority: &str) -> Result<Self, IdUriError> {
        let uri = IdUri::parse(&format!("id://{authority}/x"))?;
        Ok(Self::of(&uri))
    }

    /// The authority a URL names.
    #[must_use]
    pub fn of(uri: &IdUri) -> Self {
        Self {
            host: uri.host().to_owned(),
            port: uri.port(),
        }
    }
}

impl fmt::Display for ServerAuthority {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.host)?;
        if let Some(port) = self.port {
            write!(f, ":{port}")?;
        }
        Ok(())
    }
}

/// The remainder of a URL interpreted by its namespace's encoding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NamespacePath {
    /// `text` segments, lowercased when the namespace's paths are case-insensitive.
    Text(Vec<String>),
    /// `b64` segments decoded to bytes.
    Bytes(Vec<Vec<u8>>),
}

/// The result of selecting a URL's namespace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection<'r> {
    /// The namespace the URL names on this server.
    pub namespace: &'r NamespaceDeclaration,
    /// The handler to dispatch to.
    pub handler: &'r HandlerId,
    /// The namespace path under the declared encoding.
    pub path: NamespacePath,
    /// The query parameters, percent-decoded, in URL order.
    pub query: Vec<(String, String)>,
}

/// Why a URL did not resolve to a namespace on this server.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ResolveError {
    /// The URL is for another server; it is not looked up here.
    #[error("id URL is for server {url}, not {this}")]
    WrongServer {
        /// The server the URL names.
        url: ServerAuthority,
        /// This registry's server.
        this: ServerAuthority,
    },
    /// No namespace of that name on this server.
    #[error("no namespace {namespace:?} on server {server}")]
    UnknownNamespace {
        /// The namespace the URL names.
        namespace: String,
        /// This registry's server.
        server: ServerAuthority,
    },
    /// A query parameter the namespace does not declare.
    #[error("namespace {namespace:?} does not declare query parameter {param:?}")]
    UndeclaredQueryParameter {
        /// The namespace.
        namespace: String,
        /// The parameter, percent-decoded.
        param: String,
    },
    /// Longer than the namespace's own URL limit.
    #[error("id URL is {len} bytes, over namespace {namespace:?}'s {max}-byte limit")]
    TooLong {
        /// The namespace.
        namespace: String,
        /// The canonical URL length in bytes.
        len: usize,
        /// The namespace's limit.
        max: usize,
    },
    /// A segment that the namespace's encoding cannot decode.
    #[error("path segment {index} is not valid {encoding}")]
    BadSegment {
        /// The segment's index in the namespace path.
        index: usize,
        /// The namespace's encoding.
        encoding: Encoding,
    },
    /// The namespace declares an encoding this server cannot decode.
    #[error(
        "namespace {namespace:?} declares encoding {encoding}, which this server does not support"
    )]
    UnsupportedEncoding {
        /// The namespace.
        namespace: String,
        /// The declared encoding.
        encoding: Encoding,
    },
}

/// One server's namespace registry.
#[derive(Debug, Clone)]
pub struct NamespaceRegistry {
    server: ServerAuthority,
    namespaces: BTreeMap<String, NamespaceDeclaration>,
}

impl NamespaceRegistry {
    /// An empty registry for `server`.
    #[must_use]
    pub const fn new(server: ServerAuthority) -> Self {
        Self {
            server,
            namespaces: BTreeMap::new(),
        }
    }

    /// The server this registry belongs to.
    #[must_use]
    pub const fn server(&self) -> &ServerAuthority {
        &self.server
    }

    /// Adds a namespace; a name already declared here is refused.
    pub fn declare(&mut self, declaration: NamespaceDeclaration) -> Result<(), DeclarationError> {
        if self.namespaces.contains_key(declaration.name()) {
            return Err(DeclarationError::Duplicate(declaration.name));
        }
        self.namespaces
            .insert(declaration.name.clone(), declaration);
        Ok(())
    }

    /// The namespace declared under `name`, if any.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&NamespaceDeclaration> {
        self.namespaces.get(&name.to_ascii_lowercase())
    }

    /// Selects the namespace a URL names on this server and interprets
    /// its path and query under that namespace's declarations. The
    /// fragment is ignored: it never reaches resolution.
    pub fn select(&self, uri: &IdUri) -> Result<Selection<'_>, ResolveError> {
        let url_server = ServerAuthority::of(uri);
        if url_server != self.server {
            return Err(ResolveError::WrongServer {
                url: url_server,
                this: self.server.clone(),
            });
        }
        let namespace =
            self.namespaces
                .get(uri.namespace())
                .ok_or_else(|| ResolveError::UnknownNamespace {
                    namespace: uri.namespace().to_owned(),
                    server: self.server.clone(),
                })?;
        let name = namespace.name.clone();

        if let Some(max) = namespace.rules.max_url_len {
            let len = uri.without_fragment().to_string().len();
            if len > max {
                return Err(ResolveError::TooLong {
                    namespace: name,
                    len,
                    max,
                });
            }
        }

        let query = parse_query(uri.query());
        if let Some((param, _)) = query
            .iter()
            .find(|(param, _)| !namespace.rules.query_params.contains(param))
        {
            return Err(ResolveError::UndeclaredQueryParameter {
                namespace: name,
                param: param.clone(),
            });
        }

        let path = match &namespace.encoding {
            Encoding::Text => {
                let segments = uri.text_segments().map_err(|err| match err {
                    IdUriError::InvalidUtf8 { index } => ResolveError::BadSegment {
                        index,
                        encoding: Encoding::Text,
                    },
                    _other => ResolveError::BadSegment {
                        index: 0,
                        encoding: Encoding::Text,
                    },
                })?;
                if namespace.rules.case_sensitive_paths {
                    NamespacePath::Text(segments)
                } else {
                    NamespacePath::Text(segments.iter().map(|s| s.to_lowercase()).collect())
                }
            }
            Encoding::B64 => NamespacePath::Bytes(
                uri.segment_bytes()
                    .iter()
                    .enumerate()
                    .map(|(index, raw)| {
                        URL_SAFE_NO_PAD
                            .decode(raw)
                            .map_err(|_err| ResolveError::BadSegment {
                                index,
                                encoding: Encoding::B64,
                            })
                    })
                    .collect::<Result<_, _>>()?,
            ),
            other @ Encoding::Extension(_) => {
                return Err(ResolveError::UnsupportedEncoding {
                    namespace: name,
                    encoding: other.clone(),
                });
            }
        };

        Ok(Selection {
            namespace,
            handler: &namespace.handler,
            path,
            query,
        })
    }
}

/// `a=1&b&c=2` → `[("a","1"),("b",""),("c","2")]`, percent-decoded.
fn parse_query(query: Option<&str>) -> Vec<(String, String)> {
    let Some(query) = query.filter(|q| !q.is_empty()) else {
        return Vec::new();
    };
    query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
            (decode_lossless(name), decode_lossless(value))
        })
        .collect()
}

/// Percent-decodes `s`. The URL parser has already validated every
/// escape; a decoded sequence that is not UTF-8 stays percent-encoded
/// rather than being replaced.
fn decode_lossless(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while let Some(&b) = bytes.get(i) {
        let pair = bytes
            .get(i + 1..i + 3)
            .and_then(|p| std::str::from_utf8(p).ok());
        match pair.and_then(|p| u8::from_str_radix(p, 16).ok()) {
            Some(value) if b == b'%' => {
                out.push(value);
                i += 3;
            }
            _ => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8(out).unwrap_or_else(|_err| s.to_owned())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn handler(id: &str) -> HandlerId {
        HandlerId(id.to_owned())
    }

    fn declare(
        name: &str,
        handler_id: &str,
        encoding: Encoding,
        rules: NamespaceRules,
    ) -> NamespaceDeclaration {
        NamespaceDeclaration::new(
            name,
            CoreType::Hierarchical,
            handler(handler_id),
            encoding,
            rules,
        )
        .unwrap()
    }

    fn server(authority: &str) -> NamespaceRegistry {
        NamespaceRegistry::new(ServerAuthority::parse(authority).unwrap())
    }

    fn uri(s: &str) -> IdUri {
        IdUri::parse(s).unwrap()
    }

    // Oracle: the design's selection rule (§4) — authority picks the
    // server, the first segment picks a namespace in that server's own
    // registry, and that namespace's declarations interpret the rest.

    #[test]
    fn two_servers_resolve_the_same_text_differently_without_error() {
        let mut a = server("a.example");
        a.declare(declare(
            "docs",
            "files",
            Encoding::Text,
            NamespaceRules::default(),
        ))
        .unwrap();
        let mut a_prime = server("a.example");
        a_prime
            .declare(declare(
                "docs",
                "wiki",
                Encoding::Text,
                NamespaceRules::default(),
            ))
            .unwrap();
        let url = uri("id://a.example/docs/readme");
        assert_eq!(a.select(&url).unwrap().handler, &handler("files"));
        assert_eq!(a_prime.select(&url).unwrap().handler, &handler("wiki"));
    }

    #[test]
    fn a_url_for_another_server_is_refused_not_guessed() {
        let mut here = server("here.example");
        here.declare(declare(
            "docs",
            "files",
            Encoding::Text,
            NamespaceRules::default(),
        ))
        .unwrap();
        assert!(matches!(
            here.select(&uri("id://there.example/docs/readme")),
            Err(ResolveError::WrongServer { .. })
        ));
        assert!(matches!(
            here.select(&uri("id://here.example:8443/docs/readme")),
            Err(ResolveError::WrongServer { .. })
        ));
    }

    #[test]
    fn unknown_namespace_is_a_resolution_error() {
        let reg = server("h");
        assert_eq!(
            reg.select(&uri("id://h/nope/x")),
            Err(ResolveError::UnknownNamespace {
                namespace: "nope".into(),
                server: ServerAuthority::parse("h").unwrap(),
            })
        );
    }

    #[test]
    fn undeclared_query_parameters_are_rejected_and_declared_ones_pass() {
        let mut reg = server("h");
        let rules = NamespaceRules {
            query_params: BTreeSet::from(["rev".to_owned()]),
            ..NamespaceRules::default()
        };
        reg.declare(declare("docs", "files", Encoding::Text, rules))
            .unwrap();
        let ok = reg.select(&uri("id://h/docs/a?rev=3")).unwrap();
        assert_eq!(ok.query, [("rev".to_owned(), "3".to_owned())]);
        assert_eq!(
            reg.select(&uri("id://h/docs/a?rev=3&revv=4")),
            Err(ResolveError::UndeclaredQueryParameter {
                namespace: "docs".into(),
                param: "revv".into(),
            })
        );
    }

    #[test]
    fn b64_and_text_namespaces_coexist_on_one_server() {
        let mut reg = server("h");
        reg.declare(declare(
            "notes",
            "notes",
            Encoding::Text,
            NamespaceRules::default(),
        ))
        .unwrap();
        reg.declare(declare(
            "blobs",
            "blobs",
            Encoding::B64,
            NamespaceRules::default(),
        ))
        .unwrap();
        let bytes = [0u8, 0xFF, 0x10, b'/'];
        let encoded = URL_SAFE_NO_PAD.encode(bytes);
        let blob = reg
            .select(&uri(&format!("id://h/blobs/{encoded}")))
            .unwrap();
        assert_eq!(blob.path, NamespacePath::Bytes(vec![bytes.to_vec()]));
        let note = reg.select(&uri("id://h/notes/a%2Fb/c")).unwrap();
        assert_eq!(
            note.path,
            NamespacePath::Text(vec!["a/b".into(), "c".into()])
        );
        assert_eq!(
            reg.select(&uri("id://h/blobs/not*base64")),
            Err(ResolveError::BadSegment {
                index: 0,
                encoding: Encoding::B64,
            })
        );
    }

    #[test]
    fn text_paths_fold_case_only_when_the_namespace_says_so() {
        let mut reg = server("h");
        reg.declare(declare(
            "exact",
            "x",
            Encoding::Text,
            NamespaceRules::default(),
        ))
        .unwrap();
        let folding = NamespaceRules {
            case_sensitive_paths: false,
            ..NamespaceRules::default()
        };
        reg.declare(declare("loose", "y", Encoding::Text, folding))
            .unwrap();
        assert_eq!(
            reg.select(&uri("id://h/exact/ReadMe")).unwrap().path,
            NamespacePath::Text(vec!["ReadMe".into()])
        );
        assert_eq!(
            reg.select(&uri("id://h/loose/ReadMe")).unwrap().path,
            NamespacePath::Text(vec!["readme".into()])
        );
    }

    #[test]
    fn invalid_utf8_under_text_and_unknown_encodings_are_errors() {
        let mut reg = server("h");
        reg.declare(declare(
            "notes",
            "notes",
            Encoding::Text,
            NamespaceRules::default(),
        ))
        .unwrap();
        reg.declare(declare(
            "custom",
            "c",
            Encoding::Extension("base32".into()),
            NamespaceRules::default(),
        ))
        .unwrap();
        assert_eq!(
            reg.select(&uri("id://h/notes/ok/%FF")),
            Err(ResolveError::BadSegment {
                index: 1,
                encoding: Encoding::Text,
            })
        );
        assert!(matches!(
            reg.select(&uri("id://h/custom/x")),
            Err(ResolveError::UnsupportedEncoding { .. })
        ));
    }

    #[test]
    fn a_namespace_may_lower_the_url_limit_but_not_raise_it() {
        let mut reg = server("h");
        let short = NamespaceRules {
            max_url_len: Some(20),
            ..NamespaceRules::default()
        };
        reg.declare(declare("tiny", "t", Encoding::Text, short))
            .unwrap();
        assert!(reg.select(&uri("id://h/tiny/abc")).is_ok());
        assert!(matches!(
            reg.select(&uri("id://h/tiny/abcdefghijk")),
            Err(ResolveError::TooLong { max: 20, .. })
        ));
        let high = NamespaceRules {
            max_url_len: Some(MAX_ID_URI_LEN + 1),
            ..NamespaceRules::default()
        };
        assert!(matches!(
            NamespaceDeclaration::new("big", CoreType::Hash, handler("b"), Encoding::B64, high),
            Err(DeclarationError::LimitTooHigh { .. })
        ));
    }

    #[test]
    fn declarations_check_names_and_refuse_duplicates() {
        assert!(matches!(
            NamespaceDeclaration::new(
                "9x",
                CoreType::Hash,
                handler("h"),
                Encoding::Text,
                NamespaceRules::default()
            ),
            Err(DeclarationError::BadName(_))
        ));
        let mut reg = server("h");
        reg.declare(declare(
            "Docs",
            "a",
            Encoding::Text,
            NamespaceRules::default(),
        ))
        .unwrap();
        assert_eq!(reg.get("DOCS").unwrap().name(), "docs");
        assert_eq!(
            reg.declare(declare(
                "docs",
                "b",
                Encoding::Text,
                NamespaceRules::default()
            )),
            Err(DeclarationError::Duplicate("docs".into()))
        );
    }
}
