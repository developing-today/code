//! Stage 1 — parse (#5855 §2.1).
//!
//! A hand-rolled parser for the HTML-shaped authoring syntax of
//! decision record #5849 §2. Parse errors are fatal, located, and
//! never recovered by guessing: the first error stops the parse.
//! Unknown elements are parse errors listing the nearest registry
//! names (#5849 §3.2) — a tag outside the registry is never treated
//! as raw HTML.

use crate::vocabulary;
use crate::{BuildError, Document, Element, Node, RawRegion, Span};

/// A cursor over the source with 1-based line/column tracking.
#[derive(Debug)]
pub(crate) struct Parser {
    chars: Vec<char>,
    pos: usize,
    line: usize,
    col: usize,
}

impl Parser {
    pub(crate) fn new(source: &str) -> Self {
        Self {
            chars: source.chars().collect(),
            pos: 0,
            line: 1,
            col: 1,
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn peek_at(&self, offset: usize) -> Option<char> {
        self.chars.get(self.pos + offset).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += 1;
        if c == '\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
        Some(c)
    }

    fn span(&self) -> Span {
        Span {
            line: self.line,
            col: self.col,
        }
    }

    fn err(&self, message: impl Into<String>) -> BuildError {
        BuildError::new(message, self.line, self.col)
    }

    fn starts_with(&self, s: &str) -> bool {
        s.chars()
            .enumerate()
            .all(|(i, c)| self.peek_at(i) == Some(c))
    }

    /// Parses the whole source: misc (whitespace/comments), exactly
    /// one `<document>` root element, misc, EOF.
    pub(crate) fn parse_document(mut self) -> Result<Document, BuildError> {
        self.skip_misc()?;
        let root = self.parse_element()?;
        if root.name != "document" {
            return Err(BuildError::new(
                format!(
                    "source must be a single <document> element, found <{}>",
                    root.name
                ),
                root.span.line,
                root.span.col,
            ));
        }
        self.skip_misc()?;
        if self.peek().is_some() {
            return Err(self.err("unexpected content after the <document> element"));
        }
        Ok(Document { root })
    }

    /// Skips whitespace and comments between elements (#5849 §2.1:
    /// whitespace between elements is insignificant; comments are
    /// stripped at build time).
    fn skip_misc(&mut self) -> Result<(), BuildError> {
        loop {
            match self.peek() {
                Some(c) if c.is_whitespace() => {
                    self.bump();
                }
                Some('<') if self.starts_with("<!--") => {
                    self.parse_comment()?;
                }
                _ => return Ok(()),
            }
        }
    }

    /// Consumes one `<!-- ... -->` comment. Comments may not contain
    /// elements (#5849 §2.1).
    fn parse_comment(&mut self) -> Result<(), BuildError> {
        let start = self.span();
        for _ in 0..4 {
            self.bump();
        }
        loop {
            match self.peek() {
                None => {
                    return Err(BuildError::new(
                        "unterminated comment",
                        start.line,
                        start.col,
                    ));
                }
                Some('-') if self.starts_with("-->") => {
                    for _ in 0..3 {
                        self.bump();
                    }
                    return Ok(());
                }
                Some('<') => {
                    return Err(self.err("comments may not contain elements (#5849 §2.1)"));
                }
                Some(_) => {
                    self.bump();
                }
            }
        }
    }

    /// Reads a name (element or attribute): lowercase letters, digits,
    /// and hyphens.
    fn read_name(&mut self) -> String {
        let mut name = String::new();
        while let Some(c) = self.peek() {
            if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' {
                name.push(c);
                self.bump();
            } else {
                break;
            }
        }
        name
    }

    fn skip_inline_whitespace(&mut self) {
        while matches!(
            self.peek(),
            Some(' ') | Some('\t') | Some('\r') | Some('\n')
        ) {
            self.bump();
        }
    }

    /// Parses one element starting at `<`.
    fn parse_element(&mut self) -> Result<Element, BuildError> {
        let span = self.span();
        if self.peek() != Some('<') {
            return Err(self.err("expected an element"));
        }
        self.bump();
        let name = self.read_name();
        if name.is_empty() {
            return Err(BuildError::new(
                "expected an element name after `<`",
                span.line,
                span.col,
            ));
        }
        let Some(_spec) = vocabulary::lookup(&name) else {
            let near = vocabulary::nearest_names(&name);
            let hints = near
                .iter()
                .map(|s| format!("<{}> ({})", s.name, s.purpose))
                .collect::<Vec<_>>()
                .join(", ");
            return Err(BuildError::new(
                format!(
                    "unknown element <{name}>: not in the #5849 registry, and never treated as raw HTML; nearest registry names: {hints}"
                ),
                span.line,
                span.col,
            ));
        };

        // Attributes until `>` or `/>`.
        let mut attrs: Vec<(String, Option<String>)> = Vec::new();
        let self_closing = loop {
            self.skip_inline_whitespace();
            match self.peek() {
                Some('>') => {
                    self.bump();
                    break false;
                }
                Some('/') if self.peek_at(1) == Some('>') => {
                    self.bump();
                    self.bump();
                    break true;
                }
                Some(c) if c.is_ascii_lowercase() => {
                    let attr_span = self.span();
                    let attr = self.read_name();
                    if attrs.iter().any(|(n, _)| n == &attr) {
                        return Err(BuildError::new(
                            format!("duplicate attribute `{attr}` on <{name}>"),
                            attr_span.line,
                            attr_span.col,
                        ));
                    }
                    self.skip_inline_whitespace();
                    let value = if self.peek() == Some('=') {
                        self.bump();
                        self.skip_inline_whitespace();
                        if self.peek() != Some('"') {
                            return Err(BuildError::new(
                                format!(
                                    "attribute `{attr}` on <{name}>: values are always double-quoted (#5849 §2.1)"
                                ),
                                attr_span.line,
                                attr_span.col,
                            ));
                        }
                        self.bump();
                        let mut v = String::new();
                        loop {
                            match self.bump() {
                                None => {
                                    return Err(BuildError::new(
                                        format!(
                                            "unterminated value of attribute `{attr}` on <{name}>"
                                        ),
                                        attr_span.line,
                                        attr_span.col,
                                    ));
                                }
                                Some('"') => break,
                                Some(c) => v.push(c),
                            }
                        }
                        Some(v)
                    } else {
                        None
                    };
                    attrs.push((attr, value));
                }
                Some(_) => {
                    return Err(self.err(format!("unexpected character in <{name}> open tag")));
                }
                None => {
                    return Err(BuildError::new(
                        format!("unterminated <{name}> open tag"),
                        span.line,
                        span.col,
                    ));
                }
            }
        };

        if self_closing {
            return Ok(Element {
                name,
                attrs,
                children: Vec::new(),
                span,
            });
        }

        if name == "raw" {
            let kind = attrs
                .iter()
                .find(|(n, _)| n == "kind")
                .and_then(|(_, v)| v.clone())
                .unwrap_or_default();
            let content = self.read_raw_content(span)?;
            return Ok(Element {
                name,
                attrs,
                children: vec![Node::Raw(RawRegion {
                    kind,
                    content,
                    span,
                })],
                span,
            });
        }

        let children = self.parse_children(&name, span)?;
        Ok(Element {
            name,
            attrs,
            children,
            span,
        })
    }

    /// Captures a `<raw>` region's verbatim content up to `</raw>`
    /// (#5849 §3.1), kept as one raw node so no inner markup is ever
    /// parsed as vocabulary.
    fn read_raw_content(&mut self, span: Span) -> Result<String, BuildError> {
        let mut content = String::new();
        loop {
            if self.peek().is_none() {
                return Err(BuildError::new(
                    "unclosed <raw> region",
                    span.line,
                    span.col,
                ));
            }
            if self.starts_with("</raw>") {
                for _ in 0..6 {
                    self.bump();
                }
                return Ok(content);
            }
            if let Some(c) = self.bump() {
                content.push(c);
            }
        }
    }

    /// Parses children until the parent's matching close tag.
    fn parse_children(&mut self, parent: &str, parent_span: Span) -> Result<Vec<Node>, BuildError> {
        let mut children = Vec::new();
        loop {
            match self.peek() {
                None => {
                    return Err(BuildError::new(
                        format!("unclosed <{parent}> element"),
                        parent_span.line,
                        parent_span.col,
                    ));
                }
                Some('<') if self.starts_with("<!--") => {
                    self.parse_comment()?;
                }
                Some('<') if self.peek_at(1) == Some('/') => {
                    let close_span = self.span();
                    self.bump();
                    self.bump();
                    let close = self.read_name();
                    self.skip_inline_whitespace();
                    if self.peek() != Some('>') {
                        return Err(self.err("expected `>` to finish a close tag"));
                    }
                    self.bump();
                    if close != parent {
                        return Err(BuildError::new(
                            format!(
                                "mismatched close tag </{close}> for <{parent}> opened at {}:{}",
                                parent_span.line, parent_span.col
                            ),
                            close_span.line,
                            close_span.col,
                        ));
                    }
                    return Ok(children);
                }
                Some('<') => {
                    let el = self.parse_element()?;
                    children.push(Node::Element(el));
                }
                Some(_) => {
                    let mut text = String::new();
                    while let Some(c) = self.peek() {
                        if c == '<' {
                            break;
                        }
                        text.push(c);
                        self.bump();
                    }
                    // Whitespace inside text collapses to single
                    // spaces and the ends are trimmed (#5849 §2.1);
                    // whitespace-only runs between elements vanish.
                    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
                    if !collapsed.is_empty() {
                        children.push(Node::Text(collapsed));
                    }
                }
            }
        }
    }
}
