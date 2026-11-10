//! Seitan V2 tokens. Mirrors `generateIKey` and `ParseIKeyV2FromString` in keybase/client `go/teams`.

use std::fmt;

/// Keybase's 30-symbol alphabet (`KBase30EncodeStd`).
pub const ALPHABET: &[u8; 30] = b"abcdefghjkmnpqrsuvwxyz23456789";
/// Length of a full V2 token, including the '+'.
pub const TOKEN_LEN: usize = 18;
/// Index of the '+' in a V2 token. (V1 tokens use index 5.)
pub const PLUS_INDEX: usize = 6;

/// A lowercase Seitan V2 token. Debug output is redacted.
#[derive(Clone, PartialEq, Eq)]
pub struct Token(String);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenError {
    Length(usize),
    NotAscii,
    PlusPosition,
    Symbol(char),
}

impl fmt::Display for TokenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TokenError::Length(n) => write!(f, "token must be {TOKEN_LEN} characters, got {n}"),
            TokenError::NotAscii => f.write_str("token must be ASCII"),
            TokenError::PlusPosition => write!(f, "character {} must be '+'", PLUS_INDEX + 1),
            TokenError::Symbol(c) => write!(f, "{c:?} is not in the token alphabet"),
        }
    }
}

impl std::error::Error for TokenError {}

impl Token {
    /// Draws 17 symbols from the OS RNG. Rejection sampling on 5 bits keeps the 30 symbols
    /// uniform, so the token carries about 83.4 bits.
    pub fn generate() -> Result<Token, getrandom::Error> {
        let mut out = String::with_capacity(TOKEN_LEN);
        while out.len() < TOKEN_LEN {
            if out.len() == PLUS_INDEX {
                out.push('+');
                continue;
            }
            let mut byte = [0u8; 1];
            getrandom::fill(&mut byte)?;
            if let Some(&symbol) = ALPHABET.get((byte[0] & 0x1f) as usize) {
                out.push(symbol as char);
            }
        }
        Ok(Token(out))
    }

    /// Parses a token that a person typed or pasted. Keybase checks only the length and the '+'.
    /// This also rejects symbols outside the alphabet, which the generator never emits, so a typo
    /// fails here with a clear message. Case and surrounding whitespace are ignored.
    pub fn parse(input: &str) -> Result<Token, TokenError> {
        let s = input.trim();
        if !s.is_ascii() {
            return Err(TokenError::NotAscii);
        }
        if s.len() != TOKEN_LEN {
            return Err(TokenError::Length(s.len()));
        }
        let lower = s.to_ascii_lowercase();
        for (i, c) in lower.bytes().enumerate() {
            match (i == PLUS_INDEX, c) {
                (true, b'+') => {}
                (true, _) => return Err(TokenError::PlusPosition),
                (false, _) if ALPHABET.contains(&c) => {}
                (false, _) => return Err(TokenError::Symbol(c as char)),
            }
        }
        Ok(Token(lower))
    }

    /// The token as typed, with the '+'. Show it to the joiner. Do not log it.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Token(<redacted>)")
    }
}
