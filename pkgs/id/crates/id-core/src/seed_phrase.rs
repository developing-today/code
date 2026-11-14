//! BIP39-style seed-phrase encoding of the identity key (#2381).
//!
//! The phrase is a direct encoding of the key's 32 bytes, not a KDF
//! input: BIP39's entropy + checksum layout (256 bits of entropy, 8 bits
//! of SHA-256 checksum, 24 words of 11 bits each) over the official
//! English wordlist, embedded verbatim in `seed_wordlist.txt`. Anyone
//! holding the phrase holds the key — it is a backup of the secret
//! itself, with zero server involvement, and handling guidance lives in
//! the `key export-seed` command's output, not here.
//!
//! Decoding is strict because a restore must never silently produce a
//! different key: unknown words, a word count that is not a BIP39
//! length, and a checksum mismatch are all errors, and the checksum is
//! verified before any entropy byte is trusted.

use anyhow::{Result, bail};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

/// The official BIP39 English wordlist, one word per line, in order.
const WORDLIST_TEXT: &str = include_str!("seed_wordlist.txt");

/// Entropy lengths BIP39 defines, in bytes (128–256 bits, step 32).
const ENTROPY_LENGTHS: [usize; 5] = [16, 20, 24, 28, 32];

fn wordlist() -> Vec<&'static str> {
    WORDLIST_TEXT.lines().collect()
}

/// Encode entropy as a seed phrase: the entropy bits followed by the
/// first `entropy_bits / 32` bits of its SHA-256, split into 11-bit
/// word indices.
pub fn encode(entropy: &[u8]) -> Result<String> {
    if !ENTROPY_LENGTHS.contains(&entropy.len()) {
        bail!(
            "seed phrase entropy must be one of {ENTROPY_LENGTHS:?} bytes, got {}",
            entropy.len()
        );
    }
    let words = wordlist();
    debug_assert_eq!(words.len(), 2048);
    let checksum_bits = entropy.len() / 4;
    let digest = Sha256::digest(entropy);
    let mut bits = BitWriter::default();
    for byte in entropy {
        bits.push(*byte as u32, 8);
    }
    bits.push((digest[0] >> (8 - checksum_bits)) as u32, checksum_bits);
    let word_count = bits.len() / 11;
    let mut phrase = String::new();
    for i in 0..word_count {
        if i > 0 {
            phrase.push(' ');
        }
        phrase.push_str(words[bits.word(i)]);
    }
    Ok(phrase)
}

/// Decode a seed phrase back to its entropy, verifying every word
/// against the wordlist and the trailing checksum against SHA-256.
/// Input is normalized the way a person pastes: any whitespace runs
/// separate words, and case is folded (the wordlist is lowercase).
pub fn decode(phrase: &str) -> Result<Zeroizing<Vec<u8>>> {
    let words = wordlist();
    let tokens: Vec<&str> = phrase.split_whitespace().collect();
    let total_bits = tokens.len() * 11;
    // BIP39 phrases encode entropy + entropy/32 checksum bits, so the
    // total bit count is a multiple of 33 within 132..=264.
    if tokens.is_empty() || total_bits % 33 != 0 || !(132..=264).contains(&total_bits) {
        bail!(
            "seed phrase must have 12, 15, 18, 21, or 24 words, got {}",
            tokens.len()
        );
    }
    let mut bits = BitWriter::default();
    for (position, token) in tokens.iter().enumerate() {
        let lowered = token.to_lowercase();
        let index = words.binary_search(&lowered.as_str()).map_err(|_| {
            anyhow::anyhow!(
                "seed phrase word {} is not in the wordlist: {token:?}",
                position + 1
            )
        })?;
        bits.push(index as u32, 11);
    }
    let entropy_bits = total_bits * 32 / 33;
    let entropy_len = entropy_bits / 8;
    let mut entropy = vec![0u8; entropy_len];
    for (i, byte) in entropy.iter_mut().enumerate() {
        *byte = bits.byte(i);
    }
    let checksum_bits = total_bits - entropy_bits;
    let expected = (Sha256::digest(&entropy)[0] >> (8 - checksum_bits)) as u32;
    let actual = bits.bits(entropy_bits, checksum_bits);
    if actual != expected {
        bail!("seed phrase checksum mismatch: a word is wrong or out of order");
    }
    Ok(Zeroizing::new(entropy))
}

/// Encode the identity key (32 bytes) as its 24-word seed phrase.
pub fn encode_key(key: &[u8; 32]) -> Result<String> {
    encode(key)
}

/// Decode a seed phrase that must carry a full 32-byte identity key.
pub fn decode_key(phrase: &str) -> Result<Zeroizing<[u8; 32]>> {
    let entropy = decode(phrase)?;
    if entropy.len() != 32 {
        bail!(
            "seed phrase carries {} bytes; an identity key needs 32 (a 24-word phrase)",
            entropy.len()
        );
    }
    let mut key = Zeroizing::new([0u8; 32]);
    key.copy_from_slice(&entropy);
    Ok(key)
}

/// Big-endian bit accumulator for the 11-bit word packing. At most 264
/// bits are ever held, so a fixed array is the whole state.
struct BitWriter {
    bytes: [u8; 33],
    len: usize,
}

impl Default for BitWriter {
    fn default() -> Self {
        Self {
            bytes: [0; 33],
            len: 0,
        }
    }
}

impl BitWriter {
    fn push(&mut self, value: u32, count: usize) {
        for i in 0..count {
            let bit = (value >> (count - 1 - i)) & 1;
            let pos = self.len;
            if bit == 1 {
                self.bytes[pos / 8] |= 1 << (7 - (pos % 8));
            }
            self.len += 1;
        }
    }

    fn len(&self) -> usize {
        self.len
    }

    fn bits(&self, start: usize, count: usize) -> u32 {
        let mut value = 0u32;
        for i in 0..count {
            let pos = start + i;
            let bit = (self.bytes[pos / 8] >> (7 - (pos % 8))) & 1;
            value = (value << 1) | bit as u32;
        }
        value
    }

    fn byte(&self, index: usize) -> u8 {
        self.bits(index * 8, 8) as u8
    }

    fn word(&self, index: usize) -> usize {
        self.bits(index * 11, 11) as usize
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn wordlist_is_the_official_2048() {
        let words = wordlist();
        assert_eq!(words.len(), 2048);
        assert_eq!(words[0], "abandon");
        assert_eq!(words[2047], "zoo");
        // BIP39 invariant: the first four letters identify a word.
        let mut prefixes = std::collections::HashSet::new();
        for word in &words {
            assert!(
                prefixes.insert(word[..4.min(word.len())].to_string()),
                "duplicate 4-letter prefix at {word}"
            );
        }
    }

    #[test]
    fn bip39_official_vectors() {
        // Published BIP39 test vectors (english, no passphrase — the
        // phrase encodes the entropy itself).
        let zero32 = [0u8; 32];
        let phrase = encode(&zero32).unwrap();
        let expected_zero = std::iter::repeat_n("abandon", 23)
            .chain(["art"])
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(phrase, expected_zero);
        assert_eq!(&decode(&phrase).unwrap()[..], &zero32[..]);

        let ff32 = [0xffu8; 32];
        let phrase = encode(&ff32).unwrap();
        let expected_ff = std::iter::repeat_n("zoo", 23)
            .chain(["vote"])
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(phrase, expected_ff);
        assert_eq!(&decode(&phrase).unwrap()[..], &ff32[..]);

        // 16-byte vector: 0x7f * 16 → legal winner thank year … (first
        // words only are pinned here; the round trip pins the rest).
        let mid = [0x7fu8; 16];
        let phrase = encode(&mid).unwrap();
        assert!(phrase.starts_with("legal winner thank year "), "{phrase}");
        assert_eq!(&decode(&phrase).unwrap()[..], &mid[..]);
    }

    #[test]
    fn round_trips_a_key() {
        let key = [0x42u8; 32];
        let phrase = encode_key(&key).unwrap();
        assert_eq!(phrase.split_whitespace().count(), 24);
        assert_eq!(&decode_key(&phrase).unwrap()[..], &key[..]);
    }

    #[test]
    fn decode_normalizes_case_and_whitespace() {
        let key = [7u8; 32];
        let phrase = encode_key(&key).unwrap();
        let messy = format!("  {} \n", phrase.to_uppercase().replace(' ', "  "));
        assert_eq!(&decode_key(&messy).unwrap()[..], &key[..]);
    }

    #[test]
    fn rejects_a_checksum_mismatch() {
        let phrase = encode_key(&[3u8; 32]).unwrap();
        let mut words: Vec<&str> = phrase.split_whitespace().collect();
        // Swap the last word for the list's first: still a real word,
        // so only the checksum can catch it.
        words[23] = if words[23] == "abandon" {
            "ability"
        } else {
            "abandon"
        };
        assert!(decode_key(&words.join(" ")).is_err());
    }

    #[test]
    fn rejects_unknown_words_and_bad_counts() {
        assert!(decode("notaword abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon").is_err());
        assert!(decode("abandon abandon abandon").is_err());
        assert!(decode("").is_err());
        // A 12-word phrase decodes as entropy but is not an identity key.
        let short = encode(&[9u8; 16]).unwrap();
        assert!(decode_key(&short).is_err());
    }

    mod property {
        use super::*;
        use proptest::prelude::*;

        fn entropy_strategy() -> impl Strategy<Value = Vec<u8>> {
            (0..ENTROPY_LENGTHS.len())
                .prop_flat_map(|i| prop::collection::vec(any::<u8>(), ENTROPY_LENGTHS[i]))
        }

        proptest! {
            /// Oracle: decode is the exact inverse of encode at every BIP39
            /// entropy length — generated entropy of any supported length
            /// round-trips byte-for-byte, at the word count the BIP39 layout
            /// predicts ((entropy bits + entropy/32 checksum bits) / 11).
            #[test]
            fn generated_entropy_round_trips_at_every_length(
                entropy in entropy_strategy(),
            ) {
                let phrase = encode(&entropy).unwrap();
                prop_assert_eq!(
                    phrase.split_whitespace().count(),
                    (entropy.len() * 8 + entropy.len() / 4) / 11
                );
                prop_assert_eq!(&decode(&phrase).unwrap()[..], &entropy[..]);
            }

            /// Oracle: an arbitrary 32-byte identity key round-trips through
            /// its 24-word phrase unchanged — restore produces the same key.
            #[test]
            fn generated_keys_round_trip_through_their_phrase(key in any::<[u8; 32]>()) {
                let phrase = encode_key(&key).unwrap();
                prop_assert_eq!(phrase.split_whitespace().count(), 24);
                prop_assert_eq!(&decode_key(&phrase).unwrap()[..], &key[..]);
            }

            /// Oracle: flipping only a checksum bit of the last word leaves
            /// the entropy bits untouched, so the checksum oracle must
            /// reject the phrase — a wrong or retyped word never restores
            /// a different key silently.
            #[test]
            fn a_checksum_bit_flip_is_always_rejected(key in any::<[u8; 32]>()) {
                let phrase = encode_key(&key).unwrap();
                let words = wordlist();
                let mut tokens: Vec<&str> = phrase.split_whitespace().collect();
                let last = words
                    .binary_search(&tokens[23])
                    .expect("encoded word is in the wordlist");
                // The last word of a 24-word phrase carries 3 entropy bits
                // in its high bits and the 8 checksum bits below them, so
                // flipping bit 0 changes the checksum field only.
                tokens[23] = words[last ^ 1];
                prop_assert!(decode_key(&tokens.join(" ")).is_err());
            }

            /// Oracle: a generated token that is not a wordlist word is
            /// rejected wherever it lands in an otherwise valid phrase.
            #[test]
            fn an_unknown_word_is_always_rejected(
                key in any::<[u8; 32]>(),
                position in 0..24usize,
                token in "[a-z]{3,10}",
            ) {
                prop_assume!(!wordlist().contains(&token.as_str()));
                let phrase = encode_key(&key).unwrap();
                let mut tokens: Vec<&str> = phrase.split_whitespace().collect();
                tokens[position] = token.as_str();
                prop_assert!(decode_key(&tokens.join(" ")).is_err());
            }

            /// Oracle: word counts outside the BIP39 set are rejected no
            /// matter the words — only 12/15/18/21/24-word phrases decode.
            #[test]
            fn a_non_bip39_word_count_is_always_rejected(count in 1..=30usize) {
                prop_assume!(![12, 15, 18, 21, 24].contains(&count));
                let phrase = std::iter::repeat_n("abandon", count)
                    .collect::<Vec<_>>()
                    .join(" ");
                prop_assert!(decode(&phrase).is_err());
            }

            /// Oracle: a valid phrase carrying fewer than 32 bytes decodes
            /// as entropy but is never an identity key — decode_key rejects
            /// every generated sub-key-length phrase.
            #[test]
            fn a_short_phrase_is_never_a_key(
                entropy in (0..ENTROPY_LENGTHS.len() - 1).prop_flat_map(|i| {
                    prop::collection::vec(any::<u8>(), ENTROPY_LENGTHS[i])
                }),
            ) {
                let phrase = encode(&entropy).unwrap();
                prop_assert!(decode(&phrase).is_ok());
                prop_assert!(decode_key(&phrase).is_err());
            }
        }
    }

    #[test]
    fn encode_rejects_non_bip39_entropy_lengths() {
        assert!(encode(&[0u8; 10]).is_err());
        assert!(encode(&[0u8; 33]).is_err());
    }
}
