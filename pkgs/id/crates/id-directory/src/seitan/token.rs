//! Seitan V2 tokens. This mirrors `generateIKey` and `ParseIKeyV2FromString` in keybase/client `go/teams`.

use std::fmt;

use rand::RngExt as _;

/// Keybase's 30-symbol alphabet (`KBase30EncodeStd`).
pub const ALPHABET: &[u8; 30] = b"abcdefghjkmnpqrsuvwxyz23456789";
/// The length of a full V2 token, including the '+'.
pub const TOKEN_LEN: usize = 18;
/// The index of the '+' in a V2 token. V1 tokens put it at index 5.
pub const PLUS_INDEX: usize = 6;

/// A lowercase Seitan V2 token. The token is the secret. Its `Debug` output is redacted.
#[derive(Clone, PartialEq, Eq)]
pub struct Token(String);

/// Why a typed token was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenError {
    /// The token is not 18 bytes long. The field is the length given.
    Length(usize),
    /// The token has a non-ASCII character.
    NotAscii,
    /// The '+' is not at index 6.
    PlusPosition,
    /// A symbol is not in the alphabet.
    Symbol(char),
}

impl fmt::Display for TokenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Length(n) => write!(f, "a Seitan token is {TOKEN_LEN} characters, got {n}"),
            Self::NotAscii => f.write_str("a Seitan token is ASCII"),
            Self::PlusPosition => {
                write!(f, "character {} of a Seitan token is '+'", PLUS_INDEX + 1)
            }
            Self::Symbol(c) => write!(f, "{c:?} is not in the Seitan token alphabet"),
        }
    }
}

impl std::error::Error for TokenError {}

impl Token {
    /// Draws 17 symbols from the thread RNG. Rejection sampling on 5 bits keeps the 30 symbols uniform, so the
    /// token carries about 83 bits.
    #[must_use]
    pub fn generate() -> Self {
        let mut rng = rand::rng();
        let mut out = String::with_capacity(TOKEN_LEN);
        let mut byte = [0u8; 1];
        while out.len() < TOKEN_LEN {
            if out.len() == PLUS_INDEX {
                out.push('+');
                continue;
            }
            rng.fill(&mut byte);
            if let Some(&symbol) = ALPHABET.get(usize::from(byte[0] & 0x1f)) {
                out.push(char::from(symbol));
            }
        }
        Self(out)
    }

    /// Parses a token that a person typed or pasted. Surrounding whitespace and case are ignored. Keybase checks
    /// only the length and the '+'. This also rejects symbols outside the alphabet, which the generator never
    /// emits, so a typo fails here with the symbol named.
    ///
    /// # Errors
    ///
    /// Fails if the token is not ASCII, not 18 bytes, has its '+' in the wrong place, or has a symbol outside
    /// the alphabet.
    pub fn parse(input: &str) -> Result<Self, TokenError> {
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
                (false, _) => return Err(TokenError::Symbol(char::from(c))),
            }
        }
        Ok(Self(lower))
    }

    /// The token as it is typed, with the '+'. Show it to the joiner only. Do not log it.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Token(<redacted>)")
    }
}
