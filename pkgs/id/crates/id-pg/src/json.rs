//! A small JSON reader that keeps every number as its source text
//! (design #5805, §5.1 and §9 invariant 5; slice #13761).
//!
//! `PostgREST` returns JSON, and a general JSON parser reads numbers
//! through `f64`, which rounds integers above 2^53. The typed mapping
//! must parse each number only once its declared column type is known,
//! so this reader stops at the syntax: a number is kept as the exact
//! text the source sent, checked against the JSON number grammar and
//! nothing more.
//!
//! The reader is strict. It refuses anything RFC 8259 does not allow
//! (trailing commas, comments, leading zeros, bare `NaN` or
//! `Infinity`, control characters in strings, trailing data), a body
//! that is not UTF-8, an escape that names a lone surrogate (it has no
//! UTF-8 form), and nesting deeper than [`MAX_DEPTH`]. An object keeps
//! its members in source order and refuses a repeated key, so a row
//! can never carry two values for one column.

use std::fmt::Write as _;

/// The deepest nesting the reader accepts. A deeper body is refused,
/// so a hostile body cannot exhaust the stack.
pub const MAX_DEPTH: usize = 128;

/// One parsed JSON value, with numbers kept as text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Json {
    /// `null`.
    Null,
    /// `true` or `false`.
    Bool(bool),
    /// A number, exactly as the source wrote it.
    Number(String),
    /// A string, unescaped.
    String(String),
    /// An array.
    Array(Vec<Self>),
    /// An object, members in source order, keys unique.
    Object(Vec<(String, Self)>),
}

impl Json {
    /// Write this value back as compact JSON text. Numbers are written
    /// exactly as they were read; strings are re-escaped minimally.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        self.write_text(&mut out);
        out
    }

    fn write_text(&self, out: &mut String) {
        match self {
            Self::Null => out.push_str("null"),
            Self::Bool(true) => out.push_str("true"),
            Self::Bool(false) => out.push_str("false"),
            Self::Number(text) => out.push_str(text),
            Self::String(text) => write_string(text, out),
            Self::Array(items) => {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    item.write_text(out);
                }
                out.push(']');
            }
            Self::Object(members) => {
                out.push('{');
                for (i, (key, value)) in members.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write_string(key, out);
                    out.push(':');
                    value.write_text(out);
                }
                out.push('}');
            }
        }
    }
}

fn write_string(text: &str, out: &mut String) {
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if u32::from(c) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Why a body is not JSON. The reader reports no position or content,
/// so nothing from the body can reach an error (§9 invariant 6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotJson;

/// Parse one complete JSON text. Surrounding whitespace is allowed;
/// anything else after the value is refused.
pub fn parse(body: &[u8]) -> Result<Json, NotJson> {
    let text = std::str::from_utf8(body).map_err(|_| NotJson)?;
    let mut reader = Reader {
        bytes: text.as_bytes(),
        pos: 0,
    };
    reader.skip_whitespace();
    let value = reader.value(0)?;
    reader.skip_whitespace();
    if reader.pos == reader.bytes.len() {
        Ok(value)
    } else {
        Err(NotJson)
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl Reader<'_> {
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn bump(&mut self) -> Option<u8> {
        let byte = self.peek()?;
        self.pos += 1;
        Some(byte)
    }

    fn expect(&mut self, byte: u8) -> Result<(), NotJson> {
        if self.bump() == Some(byte) {
            Ok(())
        } else {
            Err(NotJson)
        }
    }

    fn skip_whitespace(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.pos += 1;
        }
    }

    fn literal(&mut self, word: &[u8], value: Json) -> Result<Json, NotJson> {
        let end = self.pos.checked_add(word.len()).ok_or(NotJson)?;
        if self.bytes.get(self.pos..end) == Some(word) {
            self.pos = end;
            Ok(value)
        } else {
            Err(NotJson)
        }
    }

    fn value(&mut self, depth: usize) -> Result<Json, NotJson> {
        match self.peek().ok_or(NotJson)? {
            b'n' => self.literal(b"null", Json::Null),
            b't' => self.literal(b"true", Json::Bool(true)),
            b'f' => self.literal(b"false", Json::Bool(false)),
            b'"' => self.string().map(Json::String),
            b'[' => self.array(depth),
            b'{' => self.object(depth),
            b'-' | b'0'..=b'9' => self.number().map(Json::Number),
            _ => Err(NotJson),
        }
    }

    fn array(&mut self, depth: usize) -> Result<Json, NotJson> {
        let depth = depth + 1;
        if depth > MAX_DEPTH {
            return Err(NotJson);
        }
        self.expect(b'[')?;
        let mut items = Vec::new();
        self.skip_whitespace();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(Json::Array(items));
        }
        loop {
            self.skip_whitespace();
            items.push(self.value(depth)?);
            self.skip_whitespace();
            match self.bump() {
                Some(b',') => {}
                Some(b']') => return Ok(Json::Array(items)),
                _ => return Err(NotJson),
            }
        }
    }

    fn object(&mut self, depth: usize) -> Result<Json, NotJson> {
        let depth = depth + 1;
        if depth > MAX_DEPTH {
            return Err(NotJson);
        }
        self.expect(b'{')?;
        let mut members: Vec<(String, Json)> = Vec::new();
        self.skip_whitespace();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(Json::Object(members));
        }
        loop {
            self.skip_whitespace();
            if self.peek() != Some(b'"') {
                return Err(NotJson);
            }
            let key = self.string()?;
            if members.iter().any(|(existing, _)| *existing == key) {
                return Err(NotJson);
            }
            self.skip_whitespace();
            self.expect(b':')?;
            self.skip_whitespace();
            let value = self.value(depth)?;
            members.push((key, value));
            self.skip_whitespace();
            match self.bump() {
                Some(b',') => {}
                Some(b'}') => return Ok(Json::Object(members)),
                _ => return Err(NotJson),
            }
        }
    }

    fn digits(&mut self) -> usize {
        let start = self.pos;
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.pos += 1;
        }
        self.pos - start
    }

    /// `-? (0 | [1-9][0-9]*) (. [0-9]+)? ([eE] [+-]? [0-9]+)?`, kept as
    /// text.
    fn number(&mut self) -> Result<String, NotJson> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        match self.peek() {
            Some(b'0') => {
                self.pos += 1;
                if matches!(self.peek(), Some(b'0'..=b'9')) {
                    return Err(NotJson);
                }
            }
            Some(b'1'..=b'9') => {
                self.digits();
            }
            _ => return Err(NotJson),
        }
        if self.peek() == Some(b'.') {
            self.pos += 1;
            if self.digits() == 0 {
                return Err(NotJson);
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.pos += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            if self.digits() == 0 {
                return Err(NotJson);
            }
        }
        let text = std::str::from_utf8(&self.bytes[start..self.pos]).map_err(|_| NotJson)?;
        Ok(text.to_owned())
    }

    fn hex4(&mut self) -> Result<u32, NotJson> {
        let mut code = 0_u32;
        for _ in 0..4 {
            let digit = char::from(self.bump().ok_or(NotJson)?)
                .to_digit(16)
                .ok_or(NotJson)?;
            code = code * 16 + digit;
        }
        Ok(code)
    }

    fn string(&mut self) -> Result<String, NotJson> {
        self.expect(b'"')?;
        let mut out = String::new();
        loop {
            let start = self.pos;
            while let Some(byte) = self.peek() {
                if byte == b'"' || byte == b'\\' || byte < 0x20 {
                    break;
                }
                self.pos += 1;
            }
            // The input is valid UTF-8 and the run stops only at ASCII
            // bytes, so the run is a whole number of characters.
            out.push_str(std::str::from_utf8(&self.bytes[start..self.pos]).map_err(|_| NotJson)?);
            match self.bump().ok_or(NotJson)? {
                b'"' => return Ok(out),
                b'\\' => {
                    let c = match self.bump().ok_or(NotJson)? {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'/' => '/',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'u' => self.unicode_escape()?,
                        _ => return Err(NotJson),
                    };
                    out.push(c);
                }
                // A raw control character inside a string.
                _ => return Err(NotJson),
            }
        }
    }

    /// The character named by a `\u` escape (the `\u` already read),
    /// joining a surrogate pair. A lone surrogate is refused.
    fn unicode_escape(&mut self) -> Result<char, NotJson> {
        let first = self.hex4()?;
        let code = match first {
            0xD800..=0xDBFF => {
                self.expect(b'\\')?;
                self.expect(b'u')?;
                let second = self.hex4()?;
                if !(0xDC00..=0xDFFF).contains(&second) {
                    return Err(NotJson);
                }
                0x10000 + ((first - 0xD800) << 10) + (second - 0xDC00)
            }
            0xDC00..=0xDFFF => return Err(NotJson),
            code => code,
        };
        char::from_u32(code).ok_or(NotJson)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn ok(text: &str) -> Json {
        parse(text.as_bytes()).unwrap()
    }

    fn bad(text: &str) {
        assert_eq!(parse(text.as_bytes()), Err(NotJson), "{text:?} parsed");
    }

    #[test]
    fn numbers_keep_their_source_text() {
        // Oracle: §9 invariant 5 — no integer through f64. 2^53 + 1 and
        // i64::MAX both round in f64; the text must survive exactly.
        for text in [
            "9007199254740993",
            "9223372036854775807",
            "-9223372036854775808",
            "0",
            "-0",
            "1.50",
            "1e400",
            "12345678901234567890.0000000001",
        ] {
            assert_eq!(ok(text), Json::Number(text.to_owned()));
        }
    }

    #[test]
    fn refuses_what_rfc_8259_refuses() {
        for text in [
            "",
            "01",
            "1.",
            ".5",
            "-",
            "1e",
            "+1",
            "NaN",
            "Infinity",
            "-Infinity",
            "[1,]",
            "{\"a\":1,}",
            "[1 2]",
            "{\"a\" 1}",
            "{a:1}",
            "'x'",
            "\"\u{1}\"",
            "[1] x",
            "tru",
            "\"\\x\"",
            "// c\n1",
        ] {
            bad(text);
        }
    }

    #[test]
    fn refuses_a_repeated_key() {
        bad(r#"{"id":1,"id":2}"#);
    }

    #[test]
    fn refuses_lone_surrogates_and_bad_utf8() {
        bad(r#""\ud800""#);
        bad(r#""\udc00""#);
        bad(r#""\ud800\u0041""#);
        assert_eq!(parse(b"\"\xff\""), Err(NotJson));
        assert_eq!(
            ok(r#""\ud83d\ude00""#),
            Json::String("\u{1f600}".to_owned())
        );
    }

    #[test]
    fn refuses_nesting_past_the_limit() {
        let deep = "[".repeat(MAX_DEPTH + 1) + &"]".repeat(MAX_DEPTH + 1);
        bad(&deep);
        let fine = "[".repeat(MAX_DEPTH) + &"]".repeat(MAX_DEPTH);
        assert!(parse(fine.as_bytes()).is_ok());
    }

    #[test]
    fn objects_keep_source_order_and_round_trip() {
        let value = ok(r#" { "b" : [true, null, -1.5e3], "a" : "x\"\n\u00e9" } "#);
        let Json::Object(members) = &value else {
            panic!("not an object");
        };
        assert_eq!(members[0].0, "b");
        assert_eq!(members[1].0, "a");
        assert_eq!(
            value.to_text(),
            "{\"b\":[true,null,-1.5e3],\"a\":\"x\\\"\\n\u{e9}\"}"
        );
        assert_eq!(ok(&value.to_text()), value);
    }
}
