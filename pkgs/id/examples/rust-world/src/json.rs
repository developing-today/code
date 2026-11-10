//! Minimal JSON mirroring `RocJson.roc`: dependency-free decode/encode
//! with the same contract points the examples rely on — number lexemes
//! are preserved as strings (no implicit float conversion), decode is
//! bounded (at most 65,536 bytes and 64 nested arrays/objects), object
//! `get` returns the FIRST matching key, and encoding validates number
//! lexemes instead of emitting malformed JSON.

pub const MAX_DECODE_BYTES: usize = 65_536;
pub const MAX_DEPTH: usize = 64;

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    /// The exact source lexeme, e.g. "-12.50e3".
    Number(String),
    Str(String),
    Array(Vec<Value>),
    /// Insertion-ordered pairs; duplicate keys are preserved.
    Object(Vec<(String, Value)>),
}

impl Value {
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Object(pairs) => pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Number(lexeme) => lexeme.parse::<i64>().ok(),
            _ => None,
        }
    }

    pub fn as_u64(&self) -> Option<u64> {
        match self {
            Value::Number(lexeme) => lexeme.parse::<u64>().ok(),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Value::Array(items) => Some(items),
            _ => None,
        }
    }
}

/// A valid JSON number lexeme: -?(0|[1-9][0-9]*)(\.[0-9]+)?([eE][+-]?[0-9]+)?
pub fn valid_number_lexeme(text: &str) -> bool {
    let b = text.as_bytes();
    let mut i = 0;
    if i < b.len() && b[i] == b'-' {
        i += 1;
    }
    if i >= b.len() {
        return false;
    }
    if b[i] == b'0' {
        i += 1;
    } else if b[i].is_ascii_digit() {
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
    } else {
        return false;
    }
    if i < b.len() && b[i] == b'.' {
        i += 1;
        if i >= b.len() || !b[i].is_ascii_digit() {
            return false;
        }
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
    }
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        i += 1;
        if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
            i += 1;
        }
        if i >= b.len() || !b[i].is_ascii_digit() {
            return false;
        }
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
    }
    i == b.len()
}

struct Parser<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn ws(&mut self) {
        while self.pos < self.bytes.len()
            && matches!(self.bytes[self.pos], b' ' | b'\t' | b'\n' | b'\r')
        {
            self.pos += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn value(&mut self, depth: usize) -> Result<Value, String> {
        if depth > MAX_DEPTH {
            return Err("json: nesting deeper than 64".into());
        }
        self.ws();
        match self.peek() {
            Some(b'n') => self.literal("null", Value::Null),
            Some(b't') => self.literal("true", Value::Bool(true)),
            Some(b'f') => self.literal("false", Value::Bool(false)),
            Some(b'"') => Ok(Value::Str(self.string()?)),
            Some(b'[') => {
                self.pos += 1;
                let mut items = Vec::new();
                self.ws();
                if self.peek() == Some(b']') {
                    self.pos += 1;
                    return Ok(Value::Array(items));
                }
                loop {
                    items.push(self.value(depth + 1)?);
                    self.ws();
                    match self.peek() {
                        Some(b',') => self.pos += 1,
                        Some(b']') => {
                            self.pos += 1;
                            return Ok(Value::Array(items));
                        }
                        _ => return Err("json: expected ',' or ']' in array".into()),
                    }
                }
            }
            Some(b'{') => {
                self.pos += 1;
                let mut pairs = Vec::new();
                self.ws();
                if self.peek() == Some(b'}') {
                    self.pos += 1;
                    return Ok(Value::Object(pairs));
                }
                loop {
                    self.ws();
                    let key = self.string()?;
                    self.ws();
                    if self.peek() != Some(b':') {
                        return Err("json: expected ':' in object".into());
                    }
                    self.pos += 1;
                    let val = self.value(depth + 1)?;
                    pairs.push((key, val));
                    self.ws();
                    match self.peek() {
                        Some(b',') => self.pos += 1,
                        Some(b'}') => {
                            self.pos += 1;
                            return Ok(Value::Object(pairs));
                        }
                        _ => return Err("json: expected ',' or '}' in object".into()),
                    }
                }
            }
            Some(c) if c == b'-' || c.is_ascii_digit() => {
                let start = self.pos;
                while self.pos < self.bytes.len()
                    && matches!(self.bytes[self.pos], b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9')
                {
                    self.pos += 1;
                }
                let lexeme = std::str::from_utf8(&self.bytes[start..self.pos])
                    .map_err(|_| "json: bad number")?
                    .to_string();
                if !valid_number_lexeme(&lexeme) {
                    return Err(format!("json: invalid number lexeme {lexeme:?}"));
                }
                Ok(Value::Number(lexeme))
            }
            _ => Err("json: unexpected input".into()),
        }
    }

    fn literal(&mut self, text: &str, value: Value) -> Result<Value, String> {
        if self.bytes[self.pos..].starts_with(text.as_bytes()) {
            self.pos += text.len();
            Ok(value)
        } else {
            Err(format!("json: expected {text}"))
        }
    }

    fn string(&mut self) -> Result<String, String> {
        if self.peek() != Some(b'"') {
            return Err("json: expected string".into());
        }
        self.pos += 1;
        let mut out = String::new();
        loop {
            let c = self.peek().ok_or("json: unterminated string")?;
            self.pos += 1;
            match c {
                b'"' => return Ok(out),
                b'\\' => {
                    let e = self.peek().ok_or("json: bad escape")?;
                    self.pos += 1;
                    match e {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{0008}'),
                        b'f' => out.push('\u{000C}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let hex = self.bytes.get(self.pos..self.pos + 4)
                                .ok_or("json: bad \\u escape")?;
                            let text = std::str::from_utf8(hex).map_err(|_| "json: bad \\u")?;
                            let code = u32::from_str_radix(text, 16)
                                .map_err(|_| "json: bad \\u escape")?;
                            self.pos += 4;
                            out.push(char::from_u32(code).unwrap_or('\u{FFFD}'));
                        }
                        _ => return Err("json: bad escape".into()),
                    }
                }
                _ => {
                    // Copy the full UTF-8 sequence for this scalar.
                    let start = self.pos - 1;
                    let len = utf8_len(self.bytes[start]);
                    let end = (start + len).min(self.bytes.len());
                    out.push_str(
                        std::str::from_utf8(&self.bytes[start..end])
                            .map_err(|_| "json: invalid utf-8")?,
                    );
                    self.pos = end;
                }
            }
        }
    }
}

fn utf8_len(first: u8) -> usize {
    if first < 0x80 {
        1
    } else if first >> 5 == 0b110 {
        2
    } else if first >> 4 == 0b1110 {
        3
    } else if first >> 3 == 0b11110 {
        4
    } else {
        1
    }
}

/// Decode with RocJson's bounds: > 65,536 bytes or > 64 nesting is an
/// explicit error, never a partial value.
pub fn decode(text: &str) -> Result<Value, String> {
    if text.len() > MAX_DECODE_BYTES {
        return Err("json: input longer than 65536 bytes".into());
    }
    let mut parser = Parser {
        bytes: text.as_bytes(),
        pos: 0,
    };
    let value = parser.value(0)?;
    parser.ws();
    if parser.pos != parser.bytes.len() {
        return Err("json: trailing input".into());
    }
    Ok(value)
}

/// Encode, validating number lexemes (an invalid one is an error, as in
/// RocJson, rather than malformed output).
pub fn encode(value: &Value) -> Result<String, String> {
    let mut out = String::new();
    encode_into(value, &mut out)?;
    Ok(out)
}

fn encode_into(value: &Value, out: &mut String) -> Result<(), String> {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(true) => out.push_str("true"),
        Value::Bool(false) => out.push_str("false"),
        Value::Number(lexeme) => {
            if !valid_number_lexeme(lexeme) {
                return Err(format!("json: invalid number lexeme {lexeme:?}"));
            }
            out.push_str(lexeme);
        }
        Value::Str(s) => out.push_str(&quote(s)),
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                encode_into(item, out)?;
            }
            out.push(']');
        }
        Value::Object(pairs) => {
            out.push('{');
            for (i, (key, val)) in pairs.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&quote(key));
                out.push(':');
                encode_into(val, out)?;
            }
            out.push('}');
        }
    }
    Ok(())
}

/// JSON string quoting, byte-compatible with `Screen.json_quote` for the
/// escapes the screen document uses.
pub fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_preserves_number_lexemes_and_first_key_wins() {
        let v = decode("{\"a\": -12.50e3, \"a\": 2, \"b\": [true, null, \"x\"]}").unwrap();
        assert_eq!(v.get("a"), Some(&Value::Number("-12.50e3".into())));
        assert_eq!(v.get("b").and_then(|b| b.as_array()).map(|a| a.len()), Some(3));
        assert_eq!(v.get("missing"), None);
    }

    #[test]
    fn encode_validates_numbers_and_round_trips() {
        let v = decode("{\"count\": 3}").unwrap();
        assert_eq!(encode(&v).unwrap(), "{\"count\":3}");
        assert!(encode(&Value::Number("01".into())).is_err());
        assert!(encode(&Value::Number("1.5e2".into())).is_ok());
        assert_eq!(quote("a\"\\\n"), "\"a\\\"\\\\\\n\"");
    }

    #[test]
    fn decode_bounds_are_explicit_errors() {
        let big = format!("\"{}\"", "x".repeat(MAX_DECODE_BYTES));
        assert!(decode(&big).is_err());
        let deep = "[".repeat(MAX_DEPTH + 2);
        assert!(decode(&deep).is_err());
        assert!(decode("{").is_err());
        assert!(decode("1 2").is_err());
    }
}
