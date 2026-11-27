//! Fork extension (#5753): the `kdlx` extension declaration and its
//! lint.
//!
//! Per the #5753 design record (Q2), a document that uses this fork's
//! extension syntax declares itself with a `kdlx` node as its first
//! node:
//!
//! ```kdl
//! kdlx version=1 extensions="multi-annotation prop-key-annotation children-annotation"
//! ```
//!
//! The declaration is for other consumers, not for this parser: the
//! parser accepts extension syntax with or without it. A document
//! that uses extension syntax without declaring it (or that declares
//! tokens nobody defined) is a lint warning, never an error — see
//! [`KdlDocument::lint_kdlx`].

use std::fmt::Display;

use crate::{KdlDocument, KdlNode, KdlValue};

/// The name of the extension declaration node.
pub const KDLX_NODE_NAME: &str = "kdlx";

/// Extension token: more than one annotation on a single part.
pub const EXT_MULTI_ANNOTATION: &str = "multi-annotation";

/// Extension token: annotations on property keys.
pub const EXT_PROP_KEY_ANNOTATION: &str = "prop-key-annotation";

/// Extension token: annotations on a children block.
pub const EXT_CHILDREN_ANNOTATION: &str = "children-annotation";

/// All extension tokens this fork defines, in declaration order.
pub const KNOWN_EXTENSIONS: [&str; 3] = [
    EXT_MULTI_ANNOTATION,
    EXT_PROP_KEY_ANNOTATION,
    EXT_CHILDREN_ANNOTATION,
];

/// A parsed `kdlx` extension declaration (the document's first node).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KdlxDeclaration {
    /// The `version` property of the declaration node, if it held an
    /// integer.
    pub version: Option<i128>,
    /// The whitespace-separated tokens of the `extensions` property,
    /// in declaration order.
    pub extensions: Vec<String>,
}

/// A single `kdlx` lint finding. Lints are warnings: none of these
/// make a document invalid for this parser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KdlxLint {
    /// The document uses extension syntax but has no `kdlx`
    /// declaration node.
    MissingDeclaration {
        /// The extension tokens the document uses, in
        /// [`KNOWN_EXTENSIONS`] order.
        extensions: Vec<&'static str>,
    },
    /// The document declares `kdlx` but uses an extension it did not
    /// declare.
    UndeclaredExtension {
        /// The undeclared extension token.
        extension: &'static str,
    },
    /// The document declares an extension token this fork does not
    /// define.
    UnknownExtension {
        /// The unknown extension token.
        extension: String,
    },
}

impl Display for KdlxLint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            KdlxLint::MissingDeclaration { extensions } => write!(
                f,
                "document uses kdlx extensions ({}) without a `kdlx` declaration node",
                extensions.join(" ")
            ),
            KdlxLint::UndeclaredExtension { extension } => write!(
                f,
                "document uses kdlx extension `{extension}` that its `kdlx` declaration does not list"
            ),
            KdlxLint::UnknownExtension { extension } => write!(
                f,
                "`kdlx` declaration lists unknown extension `{extension}`"
            ),
        }
    }
}

impl KdlDocument {
    /// Parses this document's `kdlx` extension declaration, if its
    /// first node is a `kdlx` node. Returns `None` for spec-valid
    /// documents and for documents whose first node is anything else.
    pub fn kdlx_declaration(&self) -> Option<KdlxDeclaration> {
        let node = self.nodes().first()?;
        if node.name().value() != KDLX_NODE_NAME {
            return None;
        }
        let version = match node.get("version") {
            Some(KdlValue::Integer(i)) => Some(*i),
            _ => None,
        };
        let extensions = match node.get("extensions") {
            Some(KdlValue::String(s)) => s.split_whitespace().map(|t| t.to_string()).collect(),
            _ => Vec::new(),
        };
        Some(KdlxDeclaration {
            version,
            extensions,
        })
    }

    /// Lints this document's extension use against its `kdlx`
    /// declaration (see the module docs). Returns an empty vec for
    /// spec-valid documents and for fully declared extension use.
    pub fn lint_kdlx(&self) -> Vec<KdlxLint> {
        let used = used_extensions(self.nodes());
        let mut lints = Vec::new();
        match self.kdlx_declaration() {
            None => {
                if !used.is_empty() {
                    lints.push(KdlxLint::MissingDeclaration { extensions: used });
                }
            }
            Some(decl) => {
                for extension in used {
                    if !decl.extensions.iter().any(|e| e == extension) {
                        lints.push(KdlxLint::UndeclaredExtension { extension });
                    }
                }
                for token in &decl.extensions {
                    if !KNOWN_EXTENSIONS.contains(&token.as_str()) {
                        lints.push(KdlxLint::UnknownExtension {
                            extension: token.clone(),
                        });
                    }
                }
            }
        }
        lints
    }
}

/// Collects the extension tokens a set of nodes uses, in
/// [`KNOWN_EXTENSIONS`] order.
fn used_extensions(nodes: &[KdlNode]) -> Vec<&'static str> {
    let mut multi = false;
    let mut prop_key = false;
    let mut children = false;
    fn walk(nodes: &[KdlNode], multi: &mut bool, prop_key: &mut bool, children: &mut bool) {
        for node in nodes {
            *multi |= node.annotations().len() > 1;
            *children |= !node.children_annotations().is_empty();
            for entry in node.entries() {
                *multi |= entry.annotations().len() > 1;
                *prop_key |= !entry.key_annotations().is_empty();
            }
            if let Some(doc) = node.children() {
                walk(doc.nodes(), multi, prop_key, children);
            }
        }
    }
    walk(nodes, &mut multi, &mut prop_key, &mut children);
    let mut used = Vec::new();
    if multi {
        used.push(EXT_MULTI_ANNOTATION);
    }
    if prop_key {
        used.push(EXT_PROP_KEY_ANNOTATION);
    }
    if children {
        used.push(EXT_CHILDREN_ANNOTATION);
    }
    used
}

#[cfg(test)]
mod test {
    use super::*;

    fn parse(doc: &str) -> KdlDocument {
        doc.parse().expect("test document should parse")
    }

    #[test]
    fn declaration_parses() {
        let doc = parse(
            "kdlx version=1 extensions=\"multi-annotation prop-key-annotation children-annotation\"\nnode 1\n",
        );
        let decl = doc.kdlx_declaration().expect("declaration present");
        assert_eq!(decl.version, Some(1));
        assert_eq!(
            decl.extensions,
            vec![
                "multi-annotation",
                "prop-key-annotation",
                "children-annotation"
            ]
        );
        assert_eq!(doc.lint_kdlx(), vec![]);
    }

    #[test]
    fn spec_document_has_no_declaration_or_lints() {
        let doc = parse("(ty)node (a)1 key=(b)2 {\n    child\n}\n");
        assert_eq!(doc.kdlx_declaration(), None);
        assert_eq!(doc.lint_kdlx(), vec![]);
    }

    #[test]
    fn undeclared_use_is_a_lint_not_an_error() {
        let doc = parse("(a)(b)node (k)key=1 (ann) {\n    child\n}\n");
        assert_eq!(
            doc.lint_kdlx(),
            vec![KdlxLint::MissingDeclaration {
                extensions: vec![
                    EXT_MULTI_ANNOTATION,
                    EXT_PROP_KEY_ANNOTATION,
                    EXT_CHILDREN_ANNOTATION
                ],
            }]
        );
    }

    #[test]
    fn partially_declared_use_lints_the_gap() {
        let doc = parse("kdlx version=1 extensions=\"multi-annotation\"\n(a)(b)node (k)key=1\n");
        assert_eq!(
            doc.lint_kdlx(),
            vec![KdlxLint::UndeclaredExtension {
                extension: EXT_PROP_KEY_ANNOTATION,
            }]
        );
    }

    #[test]
    fn unknown_token_lints() {
        let doc = parse("kdlx version=1 extensions=\"multi-annotation teleportation\"\nnode\n");
        assert_eq!(
            doc.lint_kdlx(),
            vec![KdlxLint::UnknownExtension {
                extension: "teleportation".into(),
            }]
        );
    }

    #[test]
    fn fully_declared_use_is_clean() {
        let doc = parse(
            "kdlx version=1 extensions=\"multi-annotation prop-key-annotation children-annotation\"\n(a)(b)node (k)key=(v)1 (ann) {\n    child\n}\n",
        );
        assert_eq!(doc.lint_kdlx(), vec![]);
    }
}
