//! A signed, append-only log of directory changes, merged in a deterministic order.
//!
//! Each [`LogRecord`] wraps one [`DirectoryEntry`] with its author's Ed25519
//! key, a Lamport counter and a SHA-256 content hash, and carries the author's
//! signature over those. A [`Log`] holds only records that verify, so [`merge`]
//! is a set union keyed by `(lamport, author, hash)`: commutative, associative
//! and idempotent. [`apply`] replays the merged records through
//! [`Directory::apply`], so every replica holding the same records reaches the
//! same directory and refuses the same entries. This module does no network or
//! file I/O.

use std::collections::BTreeMap;

use anyhow::{Context, Result, ensure};
use ed25519_dalek::{Signature, Signer as _, SigningKey, Verifier as _, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::directory::{Directory, DirectoryEntry, hex_to_array};
use crate::world::hex_encode;

/// The order in which replicas apply records: Lamport counter, then author, then hash.
pub type MergeKey = (u64, String, String);

/// One signed change to the directory.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogRecord {
    /// Hex Ed25519 public key of the signer.
    pub author: String,
    /// One more than the highest counter the log held when the record was made.
    pub lamport: u64,
    /// Hex SHA-256 over the signed body.
    pub hash: String,
    /// The directory change.
    pub entry: DirectoryEntry,
    /// Hex Ed25519 signature over the signed body.
    pub signature: String,
}

#[derive(Serialize)]
struct SignedBody<'a> {
    author: &'a str,
    lamport: u64,
    entry: &'a DirectoryEntry,
}

fn body_bytes(author: &str, lamport: u64, entry: &DirectoryEntry) -> Result<Vec<u8>> {
    Ok(serde_json::to_vec(&SignedBody {
        author,
        lamport,
        entry,
    })?)
}

impl LogRecord {
    /// Sign `entry` as the author holding `key`, at `lamport`.
    ///
    /// # Errors
    ///
    /// Fails if the body cannot be serialized.
    pub fn sign(key: &SigningKey, lamport: u64, entry: DirectoryEntry) -> Result<Self> {
        let author = hex_encode(&key.verifying_key().to_bytes());
        let body = body_bytes(&author, lamport, &entry)?;
        let hash = hex_encode(&Sha256::digest(&body));
        let signature = hex_encode(&key.sign(&body).to_bytes());
        Ok(Self {
            author,
            lamport,
            hash,
            entry,
            signature,
        })
    }

    /// The position of this record in the merged order.
    #[must_use]
    pub fn key(&self) -> MergeKey {
        (self.lamport, self.author.clone(), self.hash.clone())
    }

    /// Check the hash against the content and the signature against the author key.
    ///
    /// # Errors
    ///
    /// Fails if the hash does not match the content, the author is not an
    /// Ed25519 key, or the signature does not verify.
    pub fn verify(&self) -> Result<()> {
        let body = body_bytes(&self.author, self.lamport, &self.entry)?;
        ensure!(
            hex_encode(&Sha256::digest(&body)) == self.hash,
            "record hash does not match its content"
        );
        let key = VerifyingKey::from_bytes(
            &hex_to_array::<32>(&self.author).context("malformed author key")?,
        )
        .context("author is not an Ed25519 key")?;
        let signature = Signature::from_bytes(
            &hex_to_array::<64>(&self.signature).context("malformed record signature")?,
        );
        key.verify(&body, &signature)
            .context("signature does not verify against the author key")
    }
}

/// A set of verified records, ordered by [`MergeKey`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Log {
    records: BTreeMap<MergeKey, LogRecord>,
}

impl Log {
    /// Add a record if it verifies. Returns whether it was new.
    ///
    /// # Errors
    ///
    /// Fails if the record does not verify; the log is unchanged.
    pub fn insert(&mut self, record: LogRecord) -> Result<bool> {
        record.verify()?;
        let key = record.key();
        if self.records.contains_key(&key) {
            return Ok(false);
        }
        self.records.insert(key, record);
        Ok(true)
    }

    /// Sign `entry` as `key`, one Lamport step past every record held, and add it.
    ///
    /// # Errors
    ///
    /// Fails if the counter overflows or the record cannot be signed.
    pub fn append(&mut self, key: &SigningKey, entry: DirectoryEntry) -> Result<LogRecord> {
        let highest = self
            .records
            .last_key_value()
            .map_or(0, |(order, _)| order.0);
        let lamport = highest.checked_add(1).context("lamport counter overflow")?;
        let record = LogRecord::sign(key, lamport, entry)?;
        self.insert(record.clone())?;
        Ok(record)
    }

    /// The records in merged order.
    pub fn records(&self) -> impl Iterator<Item = &LogRecord> {
        self.records.values()
    }
}

/// The union of two logs. Both are sets of verified records, so the result is
/// the same whichever side is first.
#[must_use]
pub fn merge(local: &Log, remote: &Log) -> Log {
    let mut merged = local.clone();
    for record in remote.records.values() {
        merged
            .records
            .entry(record.key())
            .or_insert_with(|| record.clone());
    }
    merged
}

/// A line of log text that was not loaded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Skipped {
    /// The 1-based line number.
    pub line: usize,
    /// Why the line was not loaded.
    pub reason: SkipReason,
}

/// Why a line of log text was not loaded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SkipReason {
    /// The final line ends without a newline and is not a complete record,
    /// as after a crash during a write.
    Truncated,
    /// The line is not a record.
    Malformed(String),
    /// The record does not verify.
    Unverified(String),
}

/// The result of reading log text: the verified records and what was set aside.
#[derive(Debug, Default)]
pub struct Loaded {
    /// The records that verified.
    pub log: Log,
    /// Lines that were not loaded, in file order.
    pub skipped: Vec<Skipped>,
}

/// Read a log written by [`to_jsonl`]. A truncated final line is reported in
/// `skipped` and does not stop the load; a record that fails verification is
/// reported and never enters the log.
#[must_use]
pub fn load(text: &str) -> Loaded {
    let mut loaded = Loaded::default();
    let final_line = text
        .lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(index, _)| index + 1)
        .max();
    let ends_clean = text.ends_with('\n');
    for (index, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let number = index + 1;
        match serde_json::from_str::<LogRecord>(line) {
            Ok(record) => {
                if let Err(error) = loaded.log.insert(record) {
                    loaded.skipped.push(Skipped {
                        line: number,
                        reason: SkipReason::Unverified(format!("{error:#}")),
                    });
                }
            }
            Err(error) => {
                let reason = if Some(number) == final_line && !ends_clean {
                    SkipReason::Truncated
                } else {
                    SkipReason::Malformed(error.to_string())
                };
                loaded.skipped.push(Skipped {
                    line: number,
                    reason,
                });
            }
        }
    }
    loaded
}

/// The log as JSON lines in merged order, one record per line.
///
/// # Errors
///
/// Fails if a record cannot be serialized.
pub fn to_jsonl(log: &Log) -> Result<String> {
    let mut text = String::new();
    for record in log.records() {
        text.push_str(&serde_json::to_string(record)?);
        text.push('\n');
    }
    Ok(text)
}

/// A record the directory refused while the log was applied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refused {
    /// The refused record's content hash.
    pub hash: String,
    /// Why the directory refused it.
    pub reason: String,
}

/// What applying a log did to a directory.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Applied {
    /// How many records the directory accepted.
    pub applied: usize,
    /// The records it refused, in merged order.
    pub refused: Vec<Refused>,
}

/// Feed the log through [`Directory::apply`] in merged order.
///
/// Pass a directory without a journal: an applied entry would otherwise be
/// journaled. A refused record is skipped, and every replica holding the same
/// records refuses the same ones.
pub fn apply(directory: &mut Directory, log: &Log) -> Applied {
    let mut outcome = Applied::default();
    for record in log.records() {
        match directory.apply(&record.entry) {
            Ok(()) => outcome.applied += 1,
            Err(error) => outcome.refused.push(Refused {
                hash: record.hash.clone(),
                reason: format!("{error:#}"),
            }),
        }
    }
    outcome
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    struct Draw(u64);

    impl Draw {
        fn draw(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }

        fn below(&mut self, bound: usize) -> usize {
            usize::try_from(self.draw() % u64::try_from(bound).unwrap()).unwrap()
        }

        fn shuffle<T>(&mut self, items: &mut [T]) {
            for index in (1..items.len()).rev() {
                items.swap(index, self.below(index + 1));
            }
        }
    }

    fn key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    fn public(seed: u8) -> String {
        hex_encode(&key(seed).verifying_key().to_bytes())
    }

    /// A fixed mix of valid and conflicting entries from four authors. Some
    /// refuse on apply, depending on order, and the replicas must agree on which.
    fn entries(rng: &mut Draw) -> Vec<(u8, DirectoryEntry)> {
        let mut out = Vec::new();
        for _ in 0..60 {
            let author = 1 + u8::try_from(rng.below(4)).unwrap();
            let other = 1 + u8::try_from((usize::from(author) + rng.below(3)) % 4).unwrap();
            let entry = match rng.below(4) {
                0 => DirectoryEntry::AccountKeyed {
                    id: public(author),
                    name: format!("user {author}"),
                },
                1 => DirectoryEntry::AccountVerified { id: public(author) },
                2 => DirectoryEntry::AccountKeyAdded {
                    id: public(author),
                    key: public(other),
                },
                _ => DirectoryEntry::AccountKeyed {
                    id: public(other),
                    name: format!("user {other}"),
                },
            };
            out.push((author, entry));
        }
        out
    }

    fn records(rng: &mut Draw) -> Vec<LogRecord> {
        entries(rng)
            .into_iter()
            .map(|(author, entry)| {
                let lamport = 1 + u64::try_from(rng.below(6)).unwrap();
                LogRecord::sign(&key(author), lamport, entry).unwrap()
            })
            .collect()
    }

    fn log_of(records: &[LogRecord]) -> Log {
        let mut log = Log::default();
        for record in records {
            log.insert(record.clone()).unwrap();
        }
        log
    }

    fn sample(rng: &mut Draw, records: &[LogRecord]) -> Log {
        let mut log = Log::default();
        for record in records {
            if rng.below(2) == 0 {
                log.insert(record.clone()).unwrap();
            }
        }
        log
    }

    fn rebuild(log: &Log) -> (Directory, Applied) {
        let mut directory = Directory::new();
        let applied = apply(&mut directory, log);
        (directory, applied)
    }

    #[test]
    fn merge_is_commutative_associative_and_idempotent() {
        for seed in 1..=25_u64 {
            let mut rng = Draw(seed * 0x9E37_79B9 + 1);
            let all = records(&mut rng);
            let a = sample(&mut rng, &all);
            let b = sample(&mut rng, &all);
            let c = sample(&mut rng, &all);
            assert_eq!(merge(&a, &b), merge(&b, &a), "commutative, seed {seed}");
            assert_eq!(
                merge(&merge(&a, &b), &c),
                merge(&a, &merge(&b, &c)),
                "associative, seed {seed}"
            );
            assert_eq!(merge(&a, &a), a, "idempotent, seed {seed}");
            let ab = merge(&a, &b);
            assert_eq!(merge(&ab, &b), ab, "re-merging a subset adds nothing");
        }
    }

    #[test]
    fn replicas_with_the_same_records_converge_in_any_order() {
        let mut rng = Draw(0x00C0_FFEE);
        let all = records(&mut rng);
        let baseline = log_of(&all);
        let (expected, expected_applied) = rebuild(&baseline);
        assert!(
            expected_applied.applied > 0,
            "the fixture should apply something"
        );
        assert!(
            !expected_applied.refused.is_empty(),
            "the fixture should refuse something"
        );
        for _ in 0..10 {
            let mut shuffled = all.clone();
            rng.shuffle(&mut shuffled);
            let replica = log_of(&shuffled);
            assert_eq!(replica, baseline);
            let (directory, applied) = rebuild(&replica);
            assert_eq!(directory, expected);
            assert_eq!(applied, expected_applied);
        }
    }

    #[test]
    fn split_replicas_merge_to_the_full_log_in_any_grouping() {
        let mut rng = Draw(0xBEEF);
        let all = records(&mut rng);
        let full = log_of(&all);
        let a = sample(&mut rng, &all);
        let b = sample(&mut rng, &all);
        let missing: Vec<LogRecord> = all
            .iter()
            .filter(|r| !a.records().any(|x| x == *r) && !b.records().any(|x| x == *r))
            .cloned()
            .collect();
        let c = log_of(&missing);
        let one = merge(&merge(&a, &b), &c);
        let two = merge(&c, &merge(&b, &a));
        assert_eq!(one, two);
        assert_eq!(one, full);
        let (from_merge, _) = rebuild(&one);
        let (from_full, _) = rebuild(&full);
        assert_eq!(from_merge, from_full);
    }

    #[test]
    fn a_record_with_a_bad_signature_is_rejected_and_changes_nothing() {
        let mut log = Log::default();
        let good = LogRecord::sign(
            &key(1),
            1,
            DirectoryEntry::AccountKeyed {
                id: public(1),
                name: "Ann".to_owned(),
            },
        )
        .unwrap();
        log.insert(good).unwrap();
        let before = log.clone();

        let mut altered = LogRecord::sign(
            &key(2),
            2,
            DirectoryEntry::AccountKeyed {
                id: public(2),
                name: "Bo".to_owned(),
            },
        )
        .unwrap();
        altered.entry = DirectoryEntry::AccountKeyed {
            id: public(2),
            name: "Mallory".to_owned(),
        };
        assert!(log.insert(altered).is_err());
        assert_eq!(log, before);

        let mut forged = LogRecord::sign(
            &key(2),
            2,
            DirectoryEntry::AccountKeyed {
                id: public(2),
                name: "Bo".to_owned(),
            },
        )
        .unwrap();
        forged.author = public(3);
        let body = body_bytes(&forged.author, forged.lamport, &forged.entry).unwrap();
        forged.hash = hex_encode(&Sha256::digest(&body));
        let error = forged.verify().unwrap_err();
        assert!(format!("{error:#}").contains("signature"), "{error:#}");
        assert!(log.insert(forged).is_err());
        assert_eq!(log, before);

        let (directory, _) = rebuild(&before);
        let (with_forgery, _) = rebuild(&log);
        assert_eq!(directory, with_forgery);
    }

    #[test]
    fn a_tampered_line_is_reported_and_leaves_the_state_as_if_it_were_absent() {
        let mut rng = Draw(7);
        let all = records(&mut rng);
        let clean = log_of(&all);
        let lines: Vec<String> = to_jsonl(&clean)
            .unwrap()
            .lines()
            .map(str::to_owned)
            .collect();
        let target = lines
            .iter()
            .position(|line| line.contains("user "))
            .unwrap();
        let mut tampered = lines.clone();
        tampered[target] = tampered[target].replacen("user ", "usr ", 1);
        let loaded = load(&format!("{}\n", tampered.join("\n")));
        assert_eq!(
            loaded.skipped,
            vec![Skipped {
                line: target + 1,
                reason: SkipReason::Unverified("record hash does not match its content".to_owned()),
            }]
        );
        let victim = load(&lines[target]).log.records().next().unwrap().clone();
        let without: Vec<LogRecord> = all.iter().filter(|r| **r != victim).cloned().collect();
        assert_eq!(loaded.log, log_of(&without));
        assert_eq!(rebuild(&loaded.log), rebuild(&log_of(&without)));
    }

    #[test]
    fn jsonl_round_trips() {
        let mut rng = Draw(11);
        let log = log_of(&records(&mut rng));
        let text = to_jsonl(&log).unwrap();
        let loaded = load(&text);
        assert!(loaded.skipped.is_empty());
        assert_eq!(loaded.log, log);
    }

    #[test]
    fn a_truncated_final_line_is_tolerated_and_reported() {
        let mut rng = Draw(19);
        let log = log_of(&records(&mut rng));
        let text = to_jsonl(&log).unwrap();
        let cut = text.len() - 20;
        let truncated = &text[..cut];
        assert!(!truncated.ends_with('\n'));
        let loaded = load(truncated);
        let last = truncated.lines().count();
        assert_eq!(
            loaded.skipped,
            vec![Skipped {
                line: last,
                reason: SkipReason::Truncated,
            }]
        );
        assert_eq!(loaded.log.records().count(), log.records().count() - 1);
        for record in loaded.log.records() {
            assert!(log.records().any(|known| known == record));
        }
    }

    #[test]
    fn malformed_middle_lines_are_reported_not_fatal() {
        let mut rng = Draw(23);
        let log = log_of(&records(&mut rng));
        let text = to_jsonl(&log).unwrap();
        let mut lines: Vec<&str> = text.lines().collect();
        lines.insert(1, "{not json");
        let damaged = format!("{}\n", lines.join("\n"));
        let loaded = load(&damaged);
        assert_eq!(loaded.log, log);
        assert_eq!(loaded.skipped.len(), 1);
        assert_eq!(loaded.skipped[0].line, 2);
        assert!(matches!(loaded.skipped[0].reason, SkipReason::Malformed(_)));
    }

    #[test]
    fn empty_text_loads_an_empty_log() {
        let loaded = load("");
        assert_eq!(loaded.log, Log::default());
        assert!(loaded.skipped.is_empty());
    }

    #[test]
    fn append_orders_by_lamport_from_the_log_head() {
        let mut log = Log::default();
        let ann = key(1);
        let first = log
            .append(
                &ann,
                DirectoryEntry::AccountKeyed {
                    id: public(1),
                    name: "Ann".to_owned(),
                },
            )
            .unwrap();
        let second = log
            .append(&ann, DirectoryEntry::AccountVerified { id: public(1) })
            .unwrap();
        assert_eq!(first.lamport, 1);
        assert_eq!(second.lamport, 2);
        let (directory, applied) = rebuild(&log);
        assert_eq!(applied.applied, 2);
        assert!(directory.account(&public(1)).unwrap().verified);
    }
}
