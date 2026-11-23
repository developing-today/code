//! Generation / partition naming and slugs (spec §§5.1, 5.2, 24.4).
//!
//! The structured form is authoritative — `{generation: 4,
//! partition: "b"}` — and the human/machine spellings derive from it:
//! machine id `g4b`, subject-CN tail `B G4` (the partition sits before
//! the generation, and the CN ends in `G<integer>`, §24.4).

use crate::error::{CertError, Result};
use serde::{Deserialize, Serialize};

/// A generation + partition coordinate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenerationPartition {
    /// Generation number, >= 1 (`g1` is the initial generation).
    pub generation: u32,
    /// Partition label, lowercase in structured form: `a`..`z`, then
    /// `aa`, `ab`, ... (§5.2: partitions are extensible past `z`).
    pub partition: String,
}

impl GenerationPartition {
    pub fn new(generation: u32, partition_label: &str) -> Result<Self> {
        if generation == 0 {
            return Err(CertError::usage("generation must be >= 1"));
        }
        let p = partition_label.trim().to_ascii_lowercase();
        if p.is_empty() || !p.chars().all(|c| c.is_ascii_lowercase()) {
            return Err(CertError::usage(format!(
                "partition label '{partition_label}' must be letters (A, B, ..., AA)"
            )));
        }
        Ok(GenerationPartition {
            generation,
            partition: p,
        })
    }

    /// The initial coordinate: generation 1, partition A.
    pub fn initial() -> Self {
        GenerationPartition {
            generation: 1,
            partition: "a".to_string(),
        }
    }

    /// Machine id: `g1a`, `g4b` (§5.2).
    pub fn machine_id(&self) -> String {
        format!("g{}{}", self.generation, self.partition)
    }

    /// Subject-CN tail (§24.4): partition before generation, CN ends
    /// in `G<integer>` — e.g. `A G1`.
    pub fn cn_tail(&self) -> String {
        format!("{} G{}", self.partition.to_ascii_uppercase(), self.generation)
    }

    /// Parse a machine id like `g4b` (case-insensitive).
    pub fn parse_machine_id(input: &str) -> Result<Self> {
        let text = input.trim().to_ascii_lowercase();
        let digits: String = text
            .strip_prefix('g')
            .ok_or_else(|| CertError::usage(format!("generation id '{input}' must start with 'g'")))?
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        if digits.is_empty() {
            return Err(CertError::usage(format!(
                "generation id '{input}' has no generation number"
            )));
        }
        let partition = &text[1 + digits.len()..];
        let generation: u32 = digits
            .parse()
            .map_err(|_| CertError::usage(format!("generation id '{input}': bad number")))?;
        GenerationPartition::new(generation, partition)
    }
}

/// Partition label for a zero-based index: 0→`a`, 25→`z`, 26→`aa`.
pub fn partition_label(index: usize) -> String {
    let mut n = index + 1;
    let mut out = String::new();
    while n > 0 {
        n -= 1;
        out.push((b'a' + (n % 26) as u8) as char);
        n /= 26;
    }
    out.chars().rev().collect()
}

/// Parse a comma-separated partition list (`A,B`) into coordinates at
/// one generation. Labels are case-insensitive; duplicates are an
/// error (two partitions with one label would collide).
pub fn parse_partitions(input: &str, generation: u32) -> Result<Vec<GenerationPartition>> {
    let mut out = Vec::new();
    for part in input.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let gp = GenerationPartition::new(generation, part)?;
        if out.contains(&gp) {
            return Err(CertError::usage(format!(
                "duplicate partition '{}'",
                part.to_ascii_uppercase()
            )));
        }
        out.push(gp);
    }
    if out.is_empty() {
        return Err(CertError::usage("empty partition list"));
    }
    Ok(out)
}

/// Canonical slug for a display name (§5.2): lowercase ASCII,
/// hyphenated, punctuation stripped, length-limited. The original
/// display text is always retained separately in metadata — a slug is
/// never the identity, only a path component.
pub fn slugify(name: &str) -> Result<String> {
    let mut out = String::new();
    let mut last_dash = true; // suppresses a leading dash
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        return Err(CertError::usage(format!(
            "name '{name}' produces an empty slug; give a name with letters or digits"
        )));
    }
    if out.len() > 64 {
        out.truncate(64);
        while out.ends_with('-') {
            out.pop();
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn machine_id_round_trip() {
        let gp = GenerationPartition::parse_machine_id("g4b").unwrap();
        assert_eq!(gp.generation, 4);
        assert_eq!(gp.partition, "b");
        assert_eq!(gp.machine_id(), "g4b");
        assert_eq!(gp.cn_tail(), "B G4");
        assert_eq!(
            GenerationPartition::initial().machine_id(),
            "g1a"
        );
        assert!(GenerationPartition::parse_machine_id("4b").is_err());
        assert!(GenerationPartition::parse_machine_id("g0a").is_err());
        assert!(GenerationPartition::parse_machine_id("g1").is_err());
    }

    #[test]
    fn partition_labels_extend_past_z() {
        assert_eq!(partition_label(0), "a");
        assert_eq!(partition_label(25), "z");
        assert_eq!(partition_label(26), "aa");
        assert_eq!(partition_label(27), "ab");
    }

    #[test]
    fn partition_lists() {
        let ps = parse_partitions("A,B", 1).unwrap();
        assert_eq!(ps.len(), 2);
        assert_eq!(ps[1].machine_id(), "g1b");
        assert!(parse_partitions("A,A", 1).is_err());
        assert!(parse_partitions("", 1).is_err());
    }

    #[test]
    fn slugs() {
        assert_eq!(slugify("Internal Web").unwrap(), "internal-web");
        assert_eq!(slugify("developing.today LLC").unwrap(), "developing-today-llc");
        assert_eq!(slugify("  Base  ").unwrap(), "base");
        assert!(slugify("!!!").is_err());
    }
}
