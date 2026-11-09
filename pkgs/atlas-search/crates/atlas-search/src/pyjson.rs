//! Python-compatible JSON rendering for the CLI's output.
//! Source of the verbatim copy in pkgs/fleet's fleet-search crate,
//! ported in fleet step 5 (#5433): the output-parity contract lives
//! in these writers, so mirror every change there. Drift guard:
//! .github/scripts/check-fleet-search-parity.sh (#5920).
//!
//! The Python prints envelopes with
//! `json.dumps(out, indent=1, ensure_ascii=False)` and renders a
//! couple of `--text` fragments with the compact default dumps
//! (separators `", "` / `": "`, ensure_ascii=True). serde_json's
//! float formatting differs from CPython's repr in the small-value
//! range (serde prints `0.000087`, CPython prints `8.7e-05`), so
//! the writers here are hand-rolled over `serde_json::Value` with
//! CPython's float repr and escaping rules, making the binary's
//! stdout byte-identical to the Python's for the same envelope.

use std::fmt::Write as _;

use serde_json::Value as Json;

/// CPython `repr(float)` (== `str(float)`): shortest round-trip
/// digits, decimal notation for decimal exponents in [-4, 16),
/// scientific outside that range with a signed, at-least-two-digit
/// exponent.
pub fn py_float(x: f64) -> String {
    if x.is_nan() {
        return "NaN".to_string();
    }
    if x.is_infinite() {
        return if x > 0.0 {
            "Infinity".to_string()
        } else {
            "-Infinity".to_string()
        };
    }
    if x == 0.0 {
        return if x.is_sign_negative() {
            "-0.0".to_string()
        } else {
            "0.0".to_string()
        };
    }
    // Shortest digits + decimal exponent from Rust's scientific
    // formatting (also shortest round-trip): "8.7e-5", "1.74242e-1".
    let sci = format!("{:e}", x);
    let (mant, exp_s) = sci.split_once('e').expect("LowerExp always has e");
    let exp: i32 = exp_s.parse().expect("exponent parses");
    let digits: String = mant.chars().filter(|c| c.is_ascii_digit()).collect();
    let mut s = String::new();
    if x < 0.0 {
        s.push('-');
    }
    if (-4..16).contains(&exp) {
        if exp >= 0 {
            let int_len = (exp + 1) as usize;
            if digits.len() <= int_len {
                s.push_str(&digits);
                for _ in 0..(int_len - digits.len()) {
                    s.push('0');
                }
                s.push_str(".0");
            } else {
                s.push_str(&digits[..int_len]);
                s.push('.');
                s.push_str(&digits[int_len..]);
            }
        } else {
            s.push_str("0.");
            for _ in 0..(-exp - 1) {
                s.push('0');
            }
            s.push_str(&digits);
        }
    } else {
        s.push_str(&digits[..1]);
        if digits.len() > 1 {
            s.push('.');
            s.push_str(&digits[1..]);
        }
        s.push('e');
        s.push(if exp < 0 { '-' } else { '+' });
        let a = exp.abs();
        if a < 10 {
            s.push('0');
        }
        let _ = write!(s, "{a}");
    }
    s
}

/// A JSON string literal the way CPython's encoder writes it.
/// With `ensure_ascii`, non-ASCII is `\uXXXX`-escaped (surrogate
/// pairs above the BMP), as the compact dumps in render_text does;
/// without it, only the mandatory escapes apply (the envelope
/// dumps passes ensure_ascii=False).
pub fn py_string(s: &str, ensure_ascii: bool) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c if ensure_ascii && (c as u32) >= 0x7f => {
                if (c as u32) > 0xffff {
                    let v = (c as u32) - 0x10000;
                    let _ = write!(
                        out,
                        "\\u{:04x}\\u{:04x}",
                        0xd800 + (v >> 10),
                        0xdc00 + (v & 0x3ff)
                    );
                } else {
                    let _ = write!(out, "\\u{:04x}", c as u32);
                }
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn write_number(out: &mut String, n: &serde_json::Number) {
    if let Some(i) = n.as_i64() {
        let _ = write!(out, "{i}");
    } else if let Some(u) = n.as_u64() {
        let _ = write!(out, "{u}");
    } else if let Some(f) = n.as_f64() {
        out.push_str(&py_float(f));
    }
}

fn write_value(out: &mut String, v: &Json, pretty: bool, level: usize) {
    match v {
        Json::Null => out.push_str("null"),
        Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Json::Number(n) => write_number(out, n),
        Json::String(s) => out.push_str(&py_string(s, !pretty)),
        Json::Array(items) => {
            if items.is_empty() {
                out.push_str("[]");
                return;
            }
            if pretty {
                out.push_str("[\n");
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push_str(",\n");
                    }
                    out.push_str(&" ".repeat(level + 1));
                    write_value(out, item, pretty, level + 1);
                }
                out.push('\n');
                out.push_str(&" ".repeat(level));
                out.push(']');
            } else {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    write_value(out, item, pretty, level);
                }
                out.push(']');
            }
        }
        Json::Object(map) => {
            if map.is_empty() {
                out.push_str("{}");
                return;
            }
            if pretty {
                out.push_str("{\n");
                for (i, (k, val)) in map.iter().enumerate() {
                    if i > 0 {
                        out.push_str(",\n");
                    }
                    out.push_str(&" ".repeat(level + 1));
                    out.push_str(&py_string(k, false));
                    out.push_str(": ");
                    write_value(out, val, pretty, level + 1);
                }
                out.push('\n');
                out.push_str(&" ".repeat(level));
                out.push('}');
            } else {
                out.push('{');
                for (i, (k, val)) in map.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    out.push_str(&py_string(k, true));
                    out.push_str(": ");
                    write_value(out, val, pretty, level);
                }
                out.push('}');
            }
        }
    }
}

/// `json.dumps(v, indent=1, ensure_ascii=False)` — the envelope
/// print (the caller adds print's trailing newline).
pub fn dumps_pretty(v: &Json) -> String {
    let mut s = String::new();
    write_value(&mut s, v, true, 0);
    s
}

/// `json.dumps(v)` with default separators and ensure_ascii=True
/// — the `--text` header's filters fragment.
pub fn dumps_compact(v: &Json) -> String {
    let mut s = String::new();
    write_value(&mut s, v, false, 0);
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn float_repr_matches_cpython() {
        let cases = [
            (0.090909, "0.090909"),
            (0.174242, "0.174242"),
            (1.0, "1.0"),
            (2.0, "2.0"),
            (0.5, "0.5"),
            (10.0, "10.0"),
            (0.0001, "0.0001"),
            (0.000087, "8.7e-05"),
            (0.000001, "1e-06"),
            (1e16, "1e+16"),
            (1234567890123456.0, "1234567890123456.0"),
            (0.3, "0.3"),
            (0.0, "0.0"),
            (-0.25, "-0.25"),
            (100.0, "100.0"),
        ];
        for (x, want) in cases {
            assert_eq!(py_float(x), want, "py_float({x})");
        }
    }

    #[test]
    fn pretty_shape() {
        let v = json!({"a": 1, "b": [1.5, "x"], "c": {}, "d": []});
        assert_eq!(
            dumps_pretty(&v),
            "{\n \"a\": 1,\n \"b\": [\n  1.5,\n  \"x\"\n ],\n \"c\": {},\n \"d\": []\n}"
        );
    }

    #[test]
    fn compact_escapes_del_like_cpython() {
        // CPython's ensure_ascii encoder escapes everything outside
        // 0x20..=0x7e, so DEL (0x7f) becomes \u007f even though it is
        // ASCII; with ensure_ascii=False it stays raw (#5404).
        assert_eq!(py_string("\u{7f}", true), "\"\\u007f\"");
        assert_eq!(py_string("\u{7f}", false), "\"\u{7f}\"");
        assert_eq!(py_string("~", true), "\"~\"");
        assert_eq!(py_string("\u{80}", true), "\"\\u0080\"");
        let v = json!({"k": "x\u{7f}y"});
        assert_eq!(dumps_compact(&v), "{\"k\": \"x\\u007fy\"}");
    }

    #[test]
    fn compact_escapes_non_ascii() {
        let v = json!({"k": "café"});
        assert_eq!(dumps_compact(&v), "{\"k\": \"caf\\u00e9\"}");
        assert_eq!(dumps_pretty(&v), "{\n \"k\": \"café\"\n}");
    }
}
