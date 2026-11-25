//! Author-scoped local index as a journal projection (#13741).
//!
//! Slice 1 of the creator cross-world v2 outline
//! (`doc/2026-11-23T21-00-00Z_design_id6309_creator_cross_world_v2_outline.md`,
//! sections 1, 2, 4 and 6 item 1). It answers "what have I made, where,
//! and in what state" across the worlds one author holds on this machine
//! or peer, from local data only.
//!
//! # A projection, never a second source of truth
//!
//! The index is rebuilt from each world's own durable state (outline
//! section 4, question 1):
//!
//! - the world's journal, read once in the owner report's discipline
//!   (#6308 v1 step 1): header checked, lines bounded, a torn tail
//!   labelled rather than guessed past, any malformed line refusing the
//!   world whole;
//! - the world's records, whose #13742 draft / published facts name the
//!   program versions it holds and, for published ones, carry the
//!   signed #6278 manifest the description and tags come from.
//!
//! Only program versions signed by the index's author are listed; another
//! author's programs in the same world stay out. Nothing is fetched from
//! any server, so a peer with no network builds the same index as a
//! hosted node (outline section 3).
//!
//! # Build time and staleness
//!
//! Every index records when it was built and, per world, how many
//! journal bytes it read. [`AuthorIndex::staleness`] compares that with
//! the journals as they are now: a grown, added or removed world makes
//! the index stale, and [`fresh_or_rebuild`] then rebuilds it from the
//! worlds (the directory walk) and labels the answer with why. A missing,
//! unreadable or other-schema index is rebuilt the same way. An answer is
//! always either fresh or rebuilt now, never a stale guess.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, Read};

use serde::{Deserialize, Serialize};

use crate::program_bundle::{ProgramIdentity, parse_hex};
use crate::program_publish_state::ProgramPublishState;
use crate::world::{JournalEntry, MAX_JOURNAL_VERSION, Records, WorldEventKind};

/// Major version of the index's JSON shape. A stored index of any other
/// version is rebuilt, never read.
pub const AUTHOR_INDEX_SCHEMA_VERSION: u32 = 1;

/// The longest journal line the index reads, in bytes.
pub const MAX_JOURNAL_LINE_BYTES: usize = 4 * 1024 * 1024;

/// Why a world could not be indexed. The world is refused whole rather
/// than indexed in part as if complete.
#[derive(Debug, thiserror::Error)]
pub enum IndexError {
    /// The author key is not 32 bytes of lower-case hex.
    #[error("the index author must be 32 bytes of lower-case hex")]
    Author,
    /// The same world was added twice.
    #[error("world {0:?} is already in the index")]
    DuplicateWorld(String),
    /// The journal's first entry is not its `created` header.
    #[error("world {world_id:?}: the journal does not start with its header")]
    MissingHeader {
        /// The world.
        world_id: String,
    },
    /// The journal belongs to another world.
    #[error("world {world_id:?}: the journal belongs to world {found:?}")]
    WrongWorld {
        /// The world asked for.
        world_id: String,
        /// The world the journal names.
        found: String,
    },
    /// The journal's format version is not one this build reads.
    #[error("world {world_id:?}: journal version {version} is not supported")]
    UnsupportedJournalVersion {
        /// The world.
        world_id: String,
        /// The version found.
        version: u32,
    },
    /// A journal line is not a journal entry.
    #[error("world {world_id:?}: journal line {line} is malformed")]
    Malformed {
        /// The world.
        world_id: String,
        /// The 1-based line.
        line: u64,
    },
    /// A journal line is longer than [`MAX_JOURNAL_LINE_BYTES`].
    #[error(
        "world {world_id:?}: journal line {line} is longer than {MAX_JOURNAL_LINE_BYTES} bytes"
    )]
    OversizedLine {
        /// The world.
        world_id: String,
        /// The 1-based line.
        line: u64,
    },
    /// A publish-state record in the world is malformed or invalid.
    #[error("world {world_id:?}: {reason}")]
    Records {
        /// The world.
        world_id: String,
        /// What is wrong.
        reason: String,
    },
    /// Reading the journal failed.
    #[error("reading a journal: {0}")]
    Io(#[from] std::io::Error),
}

/// One indexed world.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorldEntry {
    /// The world's id.
    pub world_id: String,
    /// Journal bytes read when the index was built; the staleness stamp.
    pub journal_bytes: u64,
    /// Journal entries read.
    pub journal_entries: u64,
    /// The journal's format version.
    pub journal_version: u32,
    /// The journal ended in an unterminated line, which was not read.
    pub torn_tail: bool,
    /// How many program installs the journal records.
    pub installs: u64,
    /// The module hash of the most recent install, if any.
    pub active_module: Option<String>,
    /// The latest join or revoke time the journal records, Unix
    /// milliseconds; `None` when it records none.
    pub last_activity_ms: Option<u64>,
}

/// One indexed program version of the author's.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProgramEntry {
    /// The world whose records hold the version's fact.
    pub world_id: String,
    /// The program version (D1 identity).
    pub program: ProgramIdentity,
    /// Its draft / published state.
    pub state: ProgramPublishState,
    /// The signed manifest's description; published versions only.
    pub description: Option<String>,
    /// The signed manifest's categories, used as tags; published
    /// versions only.
    pub tags: Vec<String>,
    /// The world's journal records an install of exactly this payload.
    pub installed: bool,
    /// This payload is the world's active module.
    pub active: bool,
}

/// The author's index: every world added, and the author's program
/// versions in them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthorIndex {
    /// [`AUTHOR_INDEX_SCHEMA_VERSION`].
    pub schema_version: u32,
    /// The author's Ed25519 public key, lower-case hex.
    pub author: String,
    /// When the index was built, Unix milliseconds.
    pub built_at_ms: u64,
    /// The worlds, ordered by id.
    pub worlds: Vec<WorldEntry>,
    /// The author's program versions, ordered by world, name, version.
    pub programs: Vec<ProgramEntry>,
}

/// Builds an [`AuthorIndex`] one world at a time.
#[derive(Clone, Debug)]
pub struct AuthorIndexBuilder {
    author: String,
    built_at_ms: u64,
    worlds: BTreeMap<String, WorldEntry>,
    programs: Vec<ProgramEntry>,
}

impl AuthorIndexBuilder {
    /// Start an index for `author` (lower-case hex public key), built at
    /// `built_at_ms`.
    ///
    /// # Errors
    ///
    /// The author key is not 32 bytes of lower-case hex.
    pub fn new(author: &str, built_at_ms: u64) -> Result<Self, IndexError> {
        if parse_hex::<32>(author).is_none() {
            return Err(IndexError::Author);
        }
        Ok(Self {
            author: author.to_owned(),
            built_at_ms,
            worlds: BTreeMap::new(),
            programs: Vec::new(),
        })
    }

    /// Add one world from its journal and records.
    ///
    /// # Errors
    ///
    /// Any [`IndexError`]: the world is refused whole, and the builder is
    /// unchanged.
    pub fn add_world<R: BufRead>(
        &mut self,
        world_id: &str,
        journal: R,
        records: &Records,
    ) -> Result<(), IndexError> {
        if self.worlds.contains_key(world_id) {
            return Err(IndexError::DuplicateWorld(world_id.to_owned()));
        }
        let (world, installed) = read_journal(world_id, journal)?;
        let facts = crate::world_records::program_publish_facts(records).map_err(|e| {
            IndexError::Records {
                world_id: world_id.to_owned(),
                reason: format!("{e:#}"),
            }
        })?;
        let mut programs = Vec::new();
        for fact in facts {
            if fact.program.author != self.author {
                continue;
            }
            let manifest = match &fact.publication {
                Some(record) => Some(record.verify().map_err(|e| IndexError::Records {
                    world_id: world_id.to_owned(),
                    reason: format!("publication record for {:?}: {e}", fact.program),
                })?),
                None => None,
            };
            let (description, tags) =
                manifest.map_or((None, Vec::new()), |m| (m.description, m.categories));
            let hash = &fact.program.content_hash;
            programs.push(ProgramEntry {
                world_id: world_id.to_owned(),
                installed: installed.contains(hash),
                active: world.active_module.as_ref() == Some(hash),
                program: fact.program,
                state: fact.state,
                description,
                tags,
            });
        }
        self.worlds.insert(world_id.to_owned(), world);
        self.programs.extend(programs);
        Ok(())
    }

    /// The finished index.
    #[must_use]
    pub fn finish(mut self) -> AuthorIndex {
        self.programs.sort_by(|a, b| {
            (&a.world_id, &a.program.name, a.program.version).cmp(&(
                &b.world_id,
                &b.program.name,
                b.program.version,
            ))
        });
        AuthorIndex {
            schema_version: AUTHOR_INDEX_SCHEMA_VERSION,
            author: self.author,
            built_at_ms: self.built_at_ms,
            worlds: self.worlds.into_values().collect(),
            programs: self.programs,
        }
    }
}

/// Read one journal: the world entry and every installed payload hash.
fn read_journal<R: BufRead>(
    world_id: &str,
    mut journal: R,
) -> Result<(WorldEntry, BTreeSet<String>), IndexError> {
    let mut world = WorldEntry {
        world_id: world_id.to_owned(),
        journal_bytes: 0,
        journal_entries: 0,
        journal_version: 0,
        torn_tail: false,
        installs: 0,
        active_module: None,
        last_activity_ms: None,
    };
    let mut installed = BTreeSet::new();
    let mut header = false;
    let mut buf = Vec::new();
    let mut line = 0_u64;
    let cap = u64::try_from(MAX_JOURNAL_LINE_BYTES)
        .unwrap_or(u64::MAX)
        .saturating_add(1);
    loop {
        line += 1;
        buf.clear();
        let read = journal.by_ref().take(cap).read_until(b'\n', &mut buf)?;
        if read == 0 {
            break;
        }
        let terminated = buf.last() == Some(&b'\n');
        if buf.len() > MAX_JOURNAL_LINE_BYTES || (!terminated && read as u64 == cap) {
            return Err(IndexError::OversizedLine {
                world_id: world_id.to_owned(),
                line,
            });
        }
        if !terminated {
            world.torn_tail = true;
            break;
        }
        world.journal_bytes = world.journal_bytes.saturating_add(read as u64);
        let text = buf.strip_suffix(b"\n").unwrap_or(&buf);
        if text.is_empty() {
            continue;
        }
        let entry: JournalEntry =
            serde_json::from_slice(text).map_err(|_parse_error| IndexError::Malformed {
                world_id: world_id.to_owned(),
                line,
            })?;
        world.journal_entries += 1;
        if !header {
            let JournalEntry::Created {
                world_id: found,
                version,
            } = entry
            else {
                return Err(IndexError::MissingHeader {
                    world_id: world_id.to_owned(),
                });
            };
            if found != world_id {
                return Err(IndexError::WrongWorld {
                    world_id: world_id.to_owned(),
                    found,
                });
            }
            if !(1..=MAX_JOURNAL_VERSION).contains(&version) {
                return Err(IndexError::UnsupportedJournalVersion {
                    world_id: world_id.to_owned(),
                    version,
                });
            }
            world.journal_version = version;
            header = true;
            continue;
        }
        match entry {
            JournalEntry::Issued {
                issued_at: Some(at),
                ..
            }
            | JournalEntry::Revoked {
                revoked_at: Some(at),
                ..
            } => {
                world.last_activity_ms = Some(world.last_activity_ms.map_or(at, |t| t.max(at)));
            }
            JournalEntry::Committed { event } | JournalEntry::System { event } => {
                if let WorldEventKind::ProgramInstalled { module_hash, .. } = event.kind {
                    world.installs += 1;
                    installed.insert(module_hash.clone());
                    world.active_module = Some(module_hash);
                }
            }
            _ => {}
        }
    }
    if !header {
        return Err(IndexError::MissingHeader {
            world_id: world_id.to_owned(),
        });
    }
    Ok((world, installed))
}

/// How an index compares with the journals as they are now.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Staleness {
    /// Worlds whose journal length differs from the index's stamp.
    pub changed: Vec<String>,
    /// Worlds that exist now but are not in the index.
    pub added: Vec<String>,
    /// Worlds in the index that no longer exist.
    pub removed: Vec<String>,
}

impl Staleness {
    /// Whether the index still matches every journal.
    #[must_use]
    pub const fn is_fresh(&self) -> bool {
        self.changed.is_empty() && self.added.is_empty() && self.removed.is_empty()
    }
}

/// Which programs to list and how to order them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IndexQuery {
    /// Case-insensitive text matched against the program's name, its
    /// world's id and its description.
    pub text: Option<String>,
    /// A tag the program must carry, matched case-insensitively.
    pub tag: Option<String>,
    /// The state the program must be in.
    pub state: Option<ProgramPublishState>,
    /// The world the program must be in.
    pub world_id: Option<String>,
    /// The order of the results.
    pub order: IndexOrder,
}

/// How [`AuthorIndex::search`] orders its results.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum IndexOrder {
    /// By program name, then version, then world.
    #[default]
    Name,
    /// By world, then program name, then version.
    World,
    /// Most recently active world first (worlds with no recorded
    /// activity last), then program name, then version.
    RecentActivity,
}

impl AuthorIndex {
    /// The index as JSON, for a caller that keeps it between runs.
    ///
    /// # Errors
    ///
    /// The index cannot be encoded.
    pub fn to_json(&self) -> serde_json::Result<String> {
        serde_json::to_string(self)
    }

    /// A stored index, if it is this schema version and well formed.
    #[must_use]
    pub fn from_json(json: &str) -> Option<Self> {
        let value: serde_json::Value = serde_json::from_str(json).ok()?;
        let version = value.get("schema_version")?.as_u64()?;
        if version != u64::from(AUTHOR_INDEX_SCHEMA_VERSION) {
            return None;
        }
        serde_json::from_value(value).ok()
    }

    /// Compare the index with each world's current journal length
    /// (world id to bytes; a journal whose last line is unterminated
    /// counts only its complete lines).
    #[must_use]
    pub fn staleness(&self, current: &BTreeMap<String, u64>) -> Staleness {
        let mut staleness = Staleness::default();
        let indexed: BTreeSet<&str> = self.worlds.iter().map(|w| w.world_id.as_str()).collect();
        for world in &self.worlds {
            match current.get(&world.world_id) {
                None => staleness.removed.push(world.world_id.clone()),
                Some(&bytes) if bytes != world.journal_bytes => {
                    staleness.changed.push(world.world_id.clone());
                }
                Some(_) => {}
            }
        }
        for world_id in current.keys() {
            if !indexed.contains(world_id.as_str()) {
                staleness.added.push(world_id.clone());
            }
        }
        staleness
    }

    /// The world entry for `world_id`.
    #[must_use]
    pub fn world(&self, world_id: &str) -> Option<&WorldEntry> {
        self.worlds.iter().find(|w| w.world_id == world_id)
    }

    /// The author's program versions matching `query`, in its order.
    #[must_use]
    pub fn search(&self, query: &IndexQuery) -> Vec<&ProgramEntry> {
        let text = query.text.as_deref().map(str::to_lowercase);
        let tag = query.tag.as_deref().map(str::to_lowercase);
        let mut hits: Vec<&ProgramEntry> = self
            .programs
            .iter()
            .filter(|p| query.state.is_none_or(|state| p.state == state))
            .filter(|p| {
                query
                    .world_id
                    .as_deref()
                    .is_none_or(|world| p.world_id == world)
            })
            .filter(|p| {
                tag.as_deref()
                    .is_none_or(|tag| p.tags.iter().any(|t| t.to_lowercase() == tag))
            })
            .filter(|p| {
                text.as_deref().is_none_or(|text| {
                    p.program.name.to_lowercase().contains(text)
                        || p.world_id.to_lowercase().contains(text)
                        || p.description
                            .as_deref()
                            .is_some_and(|d| d.to_lowercase().contains(text))
                })
            })
            .collect();
        match query.order {
            IndexOrder::Name => hits.sort_by(|a, b| {
                (&a.program.name, a.program.version, &a.world_id).cmp(&(
                    &b.program.name,
                    b.program.version,
                    &b.world_id,
                ))
            }),
            IndexOrder::World => hits.sort_by(|a, b| {
                (&a.world_id, &a.program.name, a.program.version).cmp(&(
                    &b.world_id,
                    &b.program.name,
                    b.program.version,
                ))
            }),
            IndexOrder::RecentActivity => {
                let activity = |p: &ProgramEntry| {
                    self.world(&p.world_id)
                        .and_then(|w| w.last_activity_ms)
                        .map_or(0, |t| t.saturating_add(1))
                };
                hits.sort_by(|a, b| {
                    activity(b)
                        .cmp(&activity(a))
                        .then_with(|| a.program.name.cmp(&b.program.name))
                        .then_with(|| a.program.version.cmp(&b.program.version))
                });
            }
        }
        hits
    }

    /// The author's program versions grouped by world.
    #[must_use]
    pub fn by_world(&self) -> BTreeMap<&str, Vec<&ProgramEntry>> {
        let mut groups: BTreeMap<&str, Vec<&ProgramEntry>> = self
            .worlds
            .iter()
            .map(|w| (w.world_id.as_str(), Vec::new()))
            .collect();
        for program in &self.programs {
            groups
                .entry(program.world_id.as_str())
                .or_default()
                .push(program);
        }
        groups
    }
}

/// Where an index answer came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IndexStatus {
    /// The stored index matched every journal.
    Fresh,
    /// The index was rebuilt from the worlds now, for this reason.
    Rebuilt(RebuildReason),
}

/// Why an index was rebuilt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RebuildReason {
    /// No stored index.
    Missing,
    /// The stored index was unreadable, or another schema version.
    Unreadable,
    /// The stored index was for another author.
    OtherAuthor,
    /// The stored index no longer matched the journals.
    Stale(Staleness),
}

/// Use the stored index when it is fresh; otherwise rebuild it with
/// `rebuild` (normally a walk over the author's world directories) and
/// say why. Never answers from a stale index.
///
/// # Errors
///
/// The rebuild failed.
pub fn fresh_or_rebuild<E>(
    stored: Option<&str>,
    author: &str,
    current: &BTreeMap<String, u64>,
    rebuild: impl FnOnce() -> Result<AuthorIndex, E>,
) -> Result<(AuthorIndex, IndexStatus), E> {
    let reason = match stored {
        None => RebuildReason::Missing,
        Some(json) => match AuthorIndex::from_json(json) {
            None => RebuildReason::Unreadable,
            Some(index) if index.author != author => RebuildReason::OtherAuthor,
            Some(index) => {
                let staleness = index.staleness(current);
                if staleness.is_fresh() {
                    return Ok((index, IndexStatus::Fresh));
                }
                RebuildReason::Stale(staleness)
            }
        },
    };
    Ok((rebuild()?, IndexStatus::Rebuilt(reason)))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use ed25519_dalek::SigningKey;

    use super::*;
    use crate::program_bundle::{ProgramBundle, ProgramSpec, export};
    use crate::program_publish_state::{ProgramPublishFact, record_fact};
    use crate::world_session::encode_hex;

    fn me() -> SigningKey {
        SigningKey::from_bytes(&[7_u8; 32])
    }

    fn someone_else() -> SigningKey {
        SigningKey::from_bytes(&[9_u8; 32])
    }

    fn my_key() -> String {
        encode_hex(&me().verifying_key().to_bytes())
    }

    fn bundle(
        by: &SigningKey,
        name: &str,
        version: u64,
        payload: &[u8],
        description: Option<&str>,
        categories: &[&str],
    ) -> ProgramBundle {
        export(
            payload.to_vec(),
            ProgramSpec {
                name,
                version,
                licence: None,
                description,
                categories,
            },
            by,
        )
        .unwrap()
    }

    fn published(records: &mut Records, bundle: &ProgramBundle) {
        record_fact(
            records,
            &ProgramPublishFact::published(bundle.identity(), bundle.record()).unwrap(),
        )
        .unwrap();
    }

    fn draft(records: &mut Records, bundle: &ProgramBundle) {
        record_fact(records, &ProgramPublishFact::draft(bundle.identity())).unwrap();
    }

    fn header(world_id: &str) -> String {
        format!(
            "{}\n",
            serde_json::json!({"entry": "created", "world_id": world_id, "version": MAX_JOURNAL_VERSION})
        )
    }

    fn install(sequence: u64, module_hash: &str) -> String {
        format!(
            "{}\n",
            serde_json::json!({
                "entry": "committed",
                "event": {
                    "sequence": sequence,
                    "participant_id": 0,
                    "kind": {"kind": "program_installed", "data": {"module_hash": module_hash}}
                }
            })
        )
    }

    fn joined(id: u64, at: u64) -> String {
        format!(
            "{}\n",
            serde_json::json!({
                "entry": "issued",
                "participant": {"id": id, "display_name": format!("p{id}")},
                "digest": "00".repeat(32),
                "scopes": 1,
                "issued_at": at
            })
        )
    }

    /// Two worlds: a quiz hall with my quiz v1 (published, installed and
    /// active), my quiz v2 (draft) and someone else's program, and a
    /// gallery with my published painting program.
    struct Fixture {
        quiz_v1: ProgramBundle,
        quiz_v2: ProgramBundle,
        paint: ProgramBundle,
        hall_journal: String,
        hall_records: Records,
        gallery_journal: String,
        gallery_records: Records,
    }

    fn fixture() -> Fixture {
        let quiz_v1 = bundle(
            &me(),
            "quiz",
            1,
            b"quiz v1",
            Some("A pub quiz"),
            &["games", "trivia"],
        );
        let quiz_v2 = bundle(&me(), "quiz", 2, b"quiz v2", None, &[]);
        let theirs = bundle(&someone_else(), "chess", 1, b"chess", None, &["games"]);
        let paint = bundle(
            &me(),
            "paint",
            3,
            b"paint v3",
            Some("Collaborative canvas"),
            &["art"],
        );

        let mut hall_records = Records::new();
        published(&mut hall_records, &quiz_v1);
        draft(&mut hall_records, &quiz_v2);
        published(&mut hall_records, &theirs);
        let hall_journal = [
            header("quiz-hall"),
            joined(1, 1_000),
            install(1, &quiz_v1.identity().content_hash),
            joined(2, 5_000),
        ]
        .concat();

        let mut gallery_records = Records::new();
        published(&mut gallery_records, &paint);
        let gallery_journal = [header("gallery"), joined(1, 9_000)].concat();

        Fixture {
            quiz_v1,
            quiz_v2,
            paint,
            hall_journal,
            hall_records,
            gallery_journal,
            gallery_records,
        }
    }

    fn build(f: &Fixture, at: u64) -> AuthorIndex {
        let mut builder = AuthorIndexBuilder::new(&my_key(), at).unwrap();
        builder
            .add_world("quiz-hall", f.hall_journal.as_bytes(), &f.hall_records)
            .unwrap();
        builder
            .add_world("gallery", f.gallery_journal.as_bytes(), &f.gallery_records)
            .unwrap();
        builder.finish()
    }

    fn current(f: &Fixture) -> BTreeMap<String, u64> {
        BTreeMap::from([
            ("quiz-hall".to_owned(), f.hall_journal.len() as u64),
            ("gallery".to_owned(), f.gallery_journal.len() as u64),
        ])
    }

    fn names(hits: &[&ProgramEntry]) -> Vec<String> {
        hits.iter()
            .map(|p| format!("{}@{}", p.program.name, p.program.version))
            .collect()
    }

    #[test]
    fn the_index_lists_only_the_authors_programs_with_their_states() {
        let f = fixture();
        let index = build(&f, 42);
        assert_eq!(index.schema_version, AUTHOR_INDEX_SCHEMA_VERSION);
        assert_eq!(index.built_at_ms, 42);
        assert_eq!(index.author, my_key());
        assert_eq!(
            names(&index.search(&IndexQuery::default())),
            vec!["paint@3", "quiz@1", "quiz@2"]
        );
        let quiz_v1 = index
            .programs
            .iter()
            .find(|p| p.program == f.quiz_v1.identity())
            .unwrap();
        assert_eq!(quiz_v1.state, ProgramPublishState::Published);
        assert_eq!(quiz_v1.description.as_deref(), Some("A pub quiz"));
        assert_eq!(quiz_v1.tags, vec!["games", "trivia"]);
        assert!(quiz_v1.installed && quiz_v1.active);
        let quiz_v2 = index
            .programs
            .iter()
            .find(|p| p.program == f.quiz_v2.identity())
            .unwrap();
        assert_eq!(quiz_v2.state, ProgramPublishState::Draft);
        assert_eq!(quiz_v2.description, None);
        assert!(quiz_v2.tags.is_empty());
        assert!(!quiz_v2.installed && !quiz_v2.active);
        assert!(index.programs.iter().all(|p| p.program.author == my_key()));
    }

    #[test]
    fn world_entries_record_journal_coverage_installs_and_activity() {
        let f = fixture();
        let index = build(&f, 42);
        let hall = index.world("quiz-hall").unwrap();
        assert_eq!(hall.journal_bytes, f.hall_journal.len() as u64);
        assert_eq!(hall.journal_entries, 4);
        assert_eq!(hall.journal_version, MAX_JOURNAL_VERSION);
        assert_eq!(hall.installs, 1);
        assert_eq!(
            hall.active_module.as_deref(),
            Some(f.quiz_v1.identity().content_hash.as_str())
        );
        assert_eq!(hall.last_activity_ms, Some(5_000));
        assert!(!hall.torn_tail);
        assert_eq!(
            index.world("gallery").unwrap().last_activity_ms,
            Some(9_000)
        );
    }

    #[test]
    fn search_filters_by_text_tag_state_and_world() {
        let f = fixture();
        let index = build(&f, 42);
        let query = |q: IndexQuery| names(&index.search(&q));
        assert_eq!(
            query(IndexQuery {
                text: Some("CANVAS".to_owned()),
                ..IndexQuery::default()
            }),
            vec!["paint@3"]
        );
        assert_eq!(
            query(IndexQuery {
                text: Some("hall".to_owned()),
                ..IndexQuery::default()
            }),
            vec!["quiz@1", "quiz@2"]
        );
        assert_eq!(
            query(IndexQuery {
                tag: Some("TRIVIA".to_owned()),
                ..IndexQuery::default()
            }),
            vec!["quiz@1"]
        );
        // Someone else's "games" program is not the author's.
        assert_eq!(
            query(IndexQuery {
                tag: Some("games".to_owned()),
                ..IndexQuery::default()
            }),
            vec!["quiz@1"]
        );
        assert_eq!(
            query(IndexQuery {
                state: Some(ProgramPublishState::Draft),
                ..IndexQuery::default()
            }),
            vec!["quiz@2"]
        );
        assert_eq!(
            query(IndexQuery {
                world_id: Some("gallery".to_owned()),
                ..IndexQuery::default()
            }),
            vec!["paint@3"]
        );
        assert!(
            query(IndexQuery {
                text: Some("nothing like this".to_owned()),
                ..IndexQuery::default()
            })
            .is_empty()
        );
    }

    #[test]
    fn search_orders_by_name_world_or_recent_activity() {
        let f = fixture();
        let index = build(&f, 42);
        let order = |order| {
            names(&index.search(&IndexQuery {
                order,
                ..IndexQuery::default()
            }))
        };
        assert_eq!(order(IndexOrder::Name), vec!["paint@3", "quiz@1", "quiz@2"]);
        assert_eq!(
            order(IndexOrder::World),
            vec!["paint@3", "quiz@1", "quiz@2"]
        );
        // The gallery's last join (9000) is later than the hall's (5000).
        assert_eq!(
            order(IndexOrder::RecentActivity),
            vec!["paint@3", "quiz@1", "quiz@2"]
        );
        let groups = index.by_world();
        assert_eq!(names(&groups["quiz-hall"]), vec!["quiz@1", "quiz@2"]);
        assert_eq!(names(&groups["gallery"]), vec!["paint@3"]);
    }

    #[test]
    fn a_grown_added_or_removed_world_makes_the_index_stale() {
        let f = fixture();
        let index = build(&f, 42);
        let mut now = current(&f);
        assert!(index.staleness(&now).is_fresh());

        now.insert("quiz-hall".to_owned(), f.hall_journal.len() as u64 + 10);
        now.insert("new-world".to_owned(), 100);
        now.remove("gallery");
        let staleness = index.staleness(&now);
        assert_eq!(staleness.changed, vec!["quiz-hall"]);
        assert_eq!(staleness.added, vec!["new-world"]);
        assert_eq!(staleness.removed, vec!["gallery"]);
        assert!(!staleness.is_fresh());
    }

    #[test]
    fn a_fresh_stored_index_is_used_and_anything_else_is_rebuilt_and_labelled() {
        let f = fixture();
        let stored = build(&f, 42).to_json().unwrap();
        let now = current(&f);
        let rebuild = || Ok::<_, IndexError>(build(&f, 99));

        let (index, status) = fresh_or_rebuild(Some(&stored), &my_key(), &now, rebuild).unwrap();
        assert_eq!(status, IndexStatus::Fresh);
        assert_eq!(index.built_at_ms, 42);

        let (index, status) = fresh_or_rebuild(None, &my_key(), &now, rebuild).unwrap();
        assert_eq!(status, IndexStatus::Rebuilt(RebuildReason::Missing));
        assert_eq!(index.built_at_ms, 99);

        let (_, status) = fresh_or_rebuild(Some("{not json"), &my_key(), &now, rebuild).unwrap();
        assert_eq!(status, IndexStatus::Rebuilt(RebuildReason::Unreadable));

        let other_schema = stored.replace(
            &format!("\"schema_version\":{AUTHOR_INDEX_SCHEMA_VERSION}"),
            "\"schema_version\":999",
        );
        assert_ne!(other_schema, stored);
        let (_, status) = fresh_or_rebuild(Some(&other_schema), &my_key(), &now, rebuild).unwrap();
        assert_eq!(status, IndexStatus::Rebuilt(RebuildReason::Unreadable));

        let theirs = encode_hex(&someone_else().verifying_key().to_bytes());
        let (_, status) = fresh_or_rebuild(Some(&stored), &theirs, &now, || {
            Ok::<_, IndexError>(build(&f, 99))
        })
        .unwrap();
        assert_eq!(status, IndexStatus::Rebuilt(RebuildReason::OtherAuthor));

        let mut grown = now;
        grown.insert("gallery".to_owned(), 1);
        let (index, status) = fresh_or_rebuild(Some(&stored), &my_key(), &grown, rebuild).unwrap();
        assert_eq!(
            status,
            IndexStatus::Rebuilt(RebuildReason::Stale(Staleness {
                changed: vec!["gallery".to_owned()],
                ..Staleness::default()
            }))
        );
        assert_eq!(index.built_at_ms, 99);
    }

    #[test]
    fn a_torn_tail_is_labelled_and_not_read() {
        let f = fixture();
        let torn = format!("{}{{\"entry\":\"issu", f.gallery_journal);
        let mut builder = AuthorIndexBuilder::new(&my_key(), 1).unwrap();
        builder
            .add_world("gallery", torn.as_bytes(), &f.gallery_records)
            .unwrap();
        let index = builder.finish();
        let gallery = index.world("gallery").unwrap();
        assert!(gallery.torn_tail);
        assert_eq!(gallery.journal_entries, 2);
        assert_eq!(gallery.journal_bytes, f.gallery_journal.len() as u64);
    }

    #[test]
    fn bad_journals_and_records_refuse_the_world_whole() {
        let f = fixture();
        let mut builder = AuthorIndexBuilder::new(&my_key(), 1).unwrap();

        let no_header = joined(1, 1);
        assert!(matches!(
            builder.add_world("gallery", no_header.as_bytes(), &f.gallery_records),
            Err(IndexError::MissingHeader { .. })
        ));
        assert!(matches!(
            builder.add_world("gallery", b"".as_slice(), &f.gallery_records),
            Err(IndexError::MissingHeader { .. })
        ));
        assert!(matches!(
            builder.add_world("gallery", f.hall_journal.as_bytes(), &f.gallery_records),
            Err(IndexError::WrongWorld { .. })
        ));
        let malformed = format!("{}not json\n", f.gallery_journal);
        assert!(matches!(
            builder.add_world("gallery", malformed.as_bytes(), &f.gallery_records),
            Err(IndexError::Malformed { line: 3, .. })
        ));
        let future = format!(
            "{}\n",
            serde_json::json!({"entry": "created", "world_id": "gallery", "version": MAX_JOURNAL_VERSION + 1})
        );
        assert!(matches!(
            builder.add_world("gallery", future.as_bytes(), &f.gallery_records),
            Err(IndexError::UnsupportedJournalVersion { .. })
        ));
        let mut bad_records = f.gallery_records.clone();
        bad_records.insert(
            format!("{}broken", crate::program_publish_state::RECORD_KEY_PREFIX),
            serde_json::json!({"not": "a fact"}),
        );
        assert!(matches!(
            builder.add_world("gallery", f.gallery_journal.as_bytes(), &bad_records),
            Err(IndexError::Records { .. })
        ));

        // Every refusal left the builder unchanged; the good world still adds.
        builder
            .add_world("gallery", f.gallery_journal.as_bytes(), &f.gallery_records)
            .unwrap();
        assert!(matches!(
            builder.add_world("gallery", f.gallery_journal.as_bytes(), &f.gallery_records),
            Err(IndexError::DuplicateWorld(_))
        ));
        let index = builder.finish();
        assert_eq!(
            names(&index.search(&IndexQuery::default())),
            vec!["paint@3"]
        );
        assert_eq!(index.programs[0].program, f.paint.identity());
    }

    #[test]
    fn a_bad_author_key_is_refused() {
        assert!(matches!(
            AuthorIndexBuilder::new("not hex", 1),
            Err(IndexError::Author)
        ));
    }

    #[test]
    fn the_index_round_trips_through_json() {
        let f = fixture();
        let index = build(&f, 42);
        let json = index.to_json().unwrap();
        assert_eq!(AuthorIndex::from_json(&json).unwrap(), index);
    }
}
