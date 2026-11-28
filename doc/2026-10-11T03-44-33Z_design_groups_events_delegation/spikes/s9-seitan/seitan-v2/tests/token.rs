use std::collections::HashSet;

use proptest::prelude::*;
use seitan_v2::derive;
use seitan_v2::token::{ALPHABET, PLUS_INDEX, TOKEN_LEN};
use seitan_v2::{Token, TokenError};

#[test]
fn generated_tokens_have_the_v2_shape() {
    for _ in 0..1000 {
        let token = Token::generate().unwrap();
        let s = token.as_str().as_bytes();
        assert_eq!(s.len(), TOKEN_LEN);
        assert_eq!(s[PLUS_INDEX], b'+');
        for (i, c) in s.iter().enumerate() {
            if i != PLUS_INDEX {
                assert!(
                    ALPHABET.contains(c),
                    "symbol {:?} at {i} is outside the alphabet",
                    *c as char
                );
            }
        }
    }
}

#[test]
fn generated_tokens_cover_the_whole_alphabet() {
    // Keybase's coverage test: 500 tokens give 8,500 symbols, so a missing symbol is about e^-288 likely.
    let mut seen = HashSet::new();
    for _ in 0..500 {
        seen.extend(Token::generate().unwrap().as_str().bytes());
    }
    for c in ALPHABET {
        assert!(seen.contains(c), "symbol {:?} never generated", *c as char);
    }
}

#[test]
fn generated_tokens_do_not_repeat() {
    let tokens: HashSet<String> = (0..2000)
        .map(|_| Token::generate().unwrap().as_str().to_owned())
        .collect();
    assert_eq!(tokens.len(), 2000);
}

#[test]
fn parse_lowercases_and_ignores_surrounding_space() {
    let typed = Token::parse("  4UYWZA+B3CGA7RD6YC\n").unwrap();
    assert_eq!(typed.as_str(), "4uywza+b3cga7rd6yc");
}

#[test]
fn parse_rejects_each_malformed_shape_with_a_reason() {
    assert_eq!(
        Token::parse("4uywza+b3cga7rd6y"),
        Err(TokenError::Length(17))
    );
    assert_eq!(
        Token::parse("4uywza+b3cga7rd6yca"),
        Err(TokenError::Length(19))
    );
    assert_eq!(
        Token::parse("4uywzab+3cga7rd6yc"),
        Err(TokenError::PlusPosition)
    );
    assert_eq!(
        Token::parse("4uywza-b3cga7rd6yc"),
        Err(TokenError::PlusPosition)
    );
    assert_eq!(
        Token::parse("4uywzl+b3cga7rd6yc"),
        Err(TokenError::Symbol('l'))
    );
    assert_eq!(
        Token::parse("4uywza+b3cga7rd6y0"),
        Err(TokenError::Symbol('0'))
    );
    assert_eq!(
        Token::parse("4uywza+b3cga7rd6yé"),
        Err(TokenError::NotAscii)
    );
}

#[test]
fn debug_output_does_not_show_the_token_or_the_stretched_key() {
    let token = Token::parse("4uywza+b3cga7rd6yc").unwrap();
    assert_eq!(format!("{token:?}"), "Token(<redacted>)");
    assert_eq!(
        format!("{:?}", derive::stretch(&token)),
        "SiKey(<redacted>)"
    );
}

fn token_string() -> impl Strategy<Value = String> {
    proptest::collection::vec(proptest::sample::select(ALPHABET.to_vec()), TOKEN_LEN - 1).prop_map(
        |symbols| {
            let (head, tail) = symbols.split_at(PLUS_INDEX);
            let text = |s: &[u8]| s.iter().map(|&c| c as char).collect::<String>();
            format!("{}+{}", text(head), text(tail))
        },
    )
}

proptest! {
    #[test]
    fn any_case_gives_the_same_token_and_the_same_invite_id(s in token_string()) {
        let upper = s.to_ascii_uppercase();
        let lower_token = Token::parse(&s).unwrap();
        let upper_token = Token::parse(&upper).unwrap();
        prop_assert_eq!(&lower_token, &upper_token);
        prop_assert_eq!(
            derive::invite_id(&derive::stretch(&lower_token)),
            derive::invite_id(&derive::stretch(&upper_token))
        );
    }
}
