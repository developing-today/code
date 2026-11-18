//! The gallery read surface over a program registry (#10003; v3 design
//! `doc/2026-11-14T13-33-00Z_design_id_program_ecosystem_v3/`, G1, G3, G7).
//!
//! A [`Gallery`] is a [`Registry`] plus a full-text [`SearchIndex`] over the
//! records' signed discovery metadata (name, description, categories,
//! author key). The index is a cache, not a source: [`SearchIndex::rebuild`]
//! recreates it from registry records at any time, and it holds nothing a
//! record does not (G3). List and fetch are the registry's own operations,
//! unchanged (G1).
//!
//! Search order is deterministic and documented (G3): records whose name
//! equals the query come first, then records with a category equal to a
//! query word, then everything else; within each group the most recent
//! version comes first, then name, author and content hash break ties.
//! There is no engagement ranking.

use std::collections::BTreeMap;

use crate::program_bundle::{BundleManifest, BundleRecord};
use crate::program_registry::{ListQuery, Registry, RegistryError};

/// Most results one search returns, whatever the caller asks for.
pub const MAX_SEARCH_RESULTS: usize = 200;

/// Longest query text, in bytes.
pub const MAX_QUERY_BYTES: usize = 512;

/// One search request.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SearchQuery {
    /// Free text. Every word must appear in the record's name, description,
    /// categories or author key; empty text matches every record.
    pub text: String,
    /// Only records whose signed categories include this one.
    pub category: Option<String>,
    /// Most results wanted; capped at [`MAX_SEARCH_RESULTS`].
    pub limit: Option<usize>,
}

/// Why a search was refused.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SearchError {
    /// The query text is longer than [`MAX_QUERY_BYTES`].
    #[error("the query is {0} bytes; the limit is {MAX_QUERY_BYTES}")]
    QueryTooLong(usize),
}

#[derive(Clone, Debug)]
struct Indexed {
    record: BundleRecord,
    manifest: BundleManifest,
    text: String,
}

/// A rebuildable full-text index over verified records' discovery metadata.
#[derive(Clone, Debug, Default)]
pub struct SearchIndex {
    entries: BTreeMap<String, Indexed>,
}

impl SearchIndex {
    /// An index of `records`. A record that does not verify is left out: it
    /// is not a record anyone may serve (v2 §1).
    #[must_use]
    pub fn rebuild(records: impl IntoIterator<Item = BundleRecord>) -> Self {
        let mut index = Self::default();
        for record in records {
            index.insert(record);
        }
        index
    }

    /// Index one record, replacing any with the same content hash. Returns
    /// whether it verified and was indexed.
    pub fn insert(&mut self, record: BundleRecord) -> bool {
        let Ok(manifest) = record.verify() else {
            return false;
        };
        let text = searchable_text(&manifest);
        self.entries.insert(
            manifest.content_hash.clone(),
            Indexed {
                record,
                manifest,
                text,
            },
        );
        true
    }

    /// Drop the record with this content hash, if indexed.
    pub fn remove(&mut self, content_hash: &str) {
        self.entries.remove(content_hash);
    }

    /// How many records are indexed.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether nothing is indexed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The records matching `query`, in the documented order.
    ///
    /// # Errors
    ///
    /// The query text is too long.
    pub fn search(&self, query: &SearchQuery) -> Result<Vec<BundleRecord>, SearchError> {
        if query.text.len() > MAX_QUERY_BYTES {
            return Err(SearchError::QueryTooLong(query.text.len()));
        }
        let words = words(&query.text);
        let phrase = words.join(" ");
        let mut hits: Vec<(Rank, &Indexed)> = self
            .entries
            .values()
            .filter(|entry| {
                query
                    .category
                    .as_ref()
                    .is_none_or(|wanted| entry.manifest.categories.contains(wanted))
            })
            .filter(|entry| words.iter().all(|word| entry.text.contains(word.as_str())))
            .map(|entry| (rank(&entry.manifest, &phrase, &words), entry))
            .collect();
        hits.sort_by(|(a, _), (b, _)| a.cmp(b));
        let limit = query
            .limit
            .unwrap_or(MAX_SEARCH_RESULTS)
            .min(MAX_SEARCH_RESULTS);
        Ok(hits
            .into_iter()
            .take(limit)
            .map(|(_, entry)| entry.record.clone())
            .collect())
    }
}

/// A sort key: smaller sorts first.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Rank {
    /// 0 for an exact-name match, 1 otherwise.
    exact_name: u8,
    /// 0 for a category match, 1 otherwise.
    category: u8,
    /// The version, reversed so the most recent sorts first.
    newest: std::cmp::Reverse<u64>,
    name: String,
    author: String,
    content_hash: String,
}

fn rank(manifest: &BundleManifest, phrase: &str, words: &[String]) -> Rank {
    let exact_name = !phrase.is_empty() && manifest.name.to_lowercase() == phrase;
    let category = manifest
        .categories
        .iter()
        .any(|category| words.iter().any(|word| word == category));
    Rank {
        exact_name: u8::from(!exact_name),
        category: u8::from(!category),
        newest: std::cmp::Reverse(manifest.version),
        name: manifest.name.clone(),
        author: manifest.author.clone(),
        content_hash: manifest.content_hash.clone(),
    }
}

/// The lower-cased words of `text`: runs of letters, digits and `-`.
fn words(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !(c.is_alphanumeric() || c == '-'))
        .filter(|word| !word.is_empty())
        .map(str::to_owned)
        .collect()
}

fn searchable_text(manifest: &BundleManifest) -> String {
    let mut text = manifest.name.to_lowercase();
    if let Some(description) = &manifest.description {
        text.push('\n');
        text.push_str(&description.to_lowercase());
    }
    for category in &manifest.categories {
        text.push('\n');
        text.push_str(category);
    }
    text.push('\n');
    text.push_str(&manifest.author);
    text
}

/// A registry with its search index kept in step.
#[derive(Debug)]
pub struct Gallery {
    registry: Registry,
    index: SearchIndex,
}

impl Gallery {
    /// Serve `registry`, indexing every record it lists.
    #[must_use]
    pub fn new(registry: Registry) -> Self {
        let index = SearchIndex::rebuild(registry.list(&ListQuery::default()));
        Self { registry, index }
    }

    /// The registry behind this gallery.
    #[must_use]
    pub const fn registry(&self) -> &Registry {
        &self.registry
    }

    /// The current search index.
    #[must_use]
    pub const fn index(&self) -> &SearchIndex {
        &self.index
    }

    /// Throw the index away and rebuild it from the registry's records.
    pub fn rebuild_index(&mut self) {
        self.index = SearchIndex::rebuild(self.registry.list(&ListQuery::default()));
    }

    /// Publish through the registry and index the new record.
    ///
    /// # Errors
    ///
    /// Whatever [`Registry::publish`] refuses.
    pub fn publish(&mut self, bytes: &[u8]) -> Result<BundleRecord, RegistryError> {
        let record = self.registry.publish(bytes)?;
        self.index.insert(record.clone());
        Ok(record)
    }

    /// Withdraw on the author's signed request and drop it from the index.
    ///
    /// # Errors
    ///
    /// Whatever [`Registry::unpublish`] refuses.
    pub fn unpublish(
        &mut self,
        content_hash: &str,
        signature: &str,
    ) -> Result<BundleRecord, RegistryError> {
        let record = self.registry.unpublish(content_hash, signature)?;
        self.index.remove(content_hash);
        Ok(record)
    }

    /// Delist on the operator's authority and drop it from the index.
    ///
    /// # Errors
    ///
    /// Whatever [`Registry::remove`] refuses.
    pub fn remove(&mut self, content_hash: &str) -> Result<BundleRecord, RegistryError> {
        let record = self.registry.remove(content_hash)?;
        self.index.remove(content_hash);
        Ok(record)
    }

    /// The registry's list operation, unchanged.
    #[must_use]
    pub fn list(&self, query: &ListQuery) -> Vec<BundleRecord> {
        self.registry.list(query)
    }

    /// The registry's fetch operation, unchanged.
    #[must_use]
    pub fn fetch(&self, content_hash: &str) -> Option<Vec<u8>> {
        self.registry.fetch(content_hash)
    }

    /// Search the index.
    ///
    /// # Errors
    ///
    /// The query text is too long.
    pub fn search(&self, query: &SearchQuery) -> Result<Vec<BundleRecord>, SearchError> {
        self.index.search(query)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use ed25519_dalek::SigningKey;

    use super::*;
    use crate::program_bundle::{ProgramBundle, ProgramSpec, export};
    use crate::program_registry::{AdmitAll, sign_unpublish};

    fn alice() -> SigningKey {
        SigningKey::from_bytes(&[1; 32])
    }

    fn bob() -> SigningKey {
        SigningKey::from_bytes(&[2; 32])
    }

    fn program(
        author: &SigningKey,
        name: &str,
        version: u64,
        description: &str,
        categories: &[&str],
    ) -> ProgramBundle {
        export(
            format!("{name} v{version}").into_bytes(),
            ProgramSpec {
                name,
                version,
                licence: None,
                description: Some(description),
                categories,
            },
            author,
        )
        .unwrap()
    }

    fn names(records: &[BundleRecord]) -> Vec<(String, u64)> {
        records
            .iter()
            .map(|record| {
                let manifest = record.verify().unwrap();
                (manifest.name, manifest.version)
            })
            .collect()
    }

    fn gallery() -> Gallery {
        let mut gallery = Gallery::new(Registry::in_memory(AdmitAll));
        for bundle in [
            program(&alice(), "chess", 1, "The classic board game.", &["board"]),
            program(
                &alice(),
                "chess",
                2,
                "The classic board game, faster.",
                &["board"],
            ),
            program(
                &bob(),
                "chess-puzzles",
                3,
                "Daily chess problems.",
                &["puzzle"],
            ),
            program(
                &bob(),
                "go",
                1,
                "Stones on a grid; a board classic.",
                &["board"],
            ),
            program(&alice(), "board", 1, "A blank board to draw on.", &["art"]),
            program(&bob(), "racer", 5, "Arcade driving.", &["arcade"]),
        ] {
            gallery.publish(&bundle.to_bytes()).unwrap();
        }
        gallery
    }

    fn search(gallery: &Gallery, text: &str) -> Vec<(String, u64)> {
        names(
            &gallery
                .search(&SearchQuery {
                    text: text.to_owned(),
                    ..SearchQuery::default()
                })
                .unwrap(),
        )
    }

    #[test]
    fn exact_name_comes_first_then_newest_version() {
        assert_eq!(
            search(&gallery(), "chess"),
            vec![
                ("chess".to_owned(), 2),
                ("chess".to_owned(), 1),
                ("chess-puzzles".to_owned(), 3),
            ]
        );
    }

    #[test]
    fn exact_name_then_category_match_then_the_rest() {
        assert_eq!(
            search(&gallery(), "Board"),
            vec![
                ("board".to_owned(), 1),
                ("chess".to_owned(), 2),
                ("chess".to_owned(), 1),
                ("go".to_owned(), 1),
            ],
            "the program named board, then the board category newest first"
        );
    }

    #[test]
    fn every_word_must_match_and_description_counts() {
        assert_eq!(
            search(&gallery(), "classic faster"),
            vec![("chess".to_owned(), 2)]
        );
        assert_eq!(search(&gallery(), "stones"), vec![("go".to_owned(), 1)]);
        assert!(search(&gallery(), "nothing-like-this").is_empty());
    }

    #[test]
    fn empty_text_lists_everything_newest_first_and_the_limit_caps_it() {
        let gallery = gallery();
        let all = search(&gallery, "");
        assert_eq!(all.len(), 6);
        assert_eq!(all[0], ("racer".to_owned(), 5));
        let two = gallery
            .search(&SearchQuery {
                limit: Some(2),
                ..SearchQuery::default()
            })
            .unwrap();
        assert_eq!(names(&two), all[..2].to_vec());
    }

    #[test]
    fn a_category_filter_keeps_only_that_signed_category() {
        let gallery = gallery();
        let board = gallery
            .search(&SearchQuery {
                category: Some("board".to_owned()),
                ..SearchQuery::default()
            })
            .unwrap();
        assert_eq!(
            names(&board),
            vec![
                ("chess".to_owned(), 2),
                ("chess".to_owned(), 1),
                ("go".to_owned(), 1),
            ]
        );
        let author = alice().verifying_key();
        let hex = crate::world_session::encode_hex(&author.to_bytes());
        assert_eq!(
            search(&gallery, &hex).len(),
            3,
            "the author key is searchable"
        );
    }

    #[test]
    fn the_order_does_not_depend_on_publish_order() {
        let records = gallery().list(&ListQuery::default());
        let forward = SearchIndex::rebuild(records.clone());
        let backward = SearchIndex::rebuild(records.into_iter().rev());
        for text in ["", "chess", "board", "classic"] {
            let query = SearchQuery {
                text: text.to_owned(),
                ..SearchQuery::default()
            };
            assert_eq!(
                forward.search(&query).unwrap(),
                backward.search(&query).unwrap()
            );
        }
    }

    #[test]
    fn the_index_rebuilds_from_records_alone_and_tracks_withdrawals() {
        let mut gallery = gallery();
        let before = search(&gallery, "board");
        gallery.rebuild_index();
        assert_eq!(search(&gallery, "board"), before);
        let rebuilt = SearchIndex::rebuild(gallery.list(&ListQuery::default()));
        assert_eq!(rebuilt.len(), gallery.index().len());

        let go = gallery
            .list(&ListQuery {
                author: None,
                name: Some("go".to_owned()),
            })
            .remove(0);
        let hash = go.verify().unwrap().content_hash;
        gallery
            .unpublish(&hash, &sign_unpublish(&hash, &bob()))
            .unwrap();
        assert!(search(&gallery, "stones").is_empty());
        let racer = gallery
            .list(&ListQuery {
                author: None,
                name: Some("racer".to_owned()),
            })
            .remove(0)
            .verify()
            .unwrap()
            .content_hash;
        gallery.remove(&racer).unwrap();
        assert!(search(&gallery, "arcade").is_empty());
        assert_eq!(gallery.index().len(), 4);
    }

    #[test]
    fn an_unverifiable_record_is_never_indexed() {
        let record = program(&alice(), "chess", 1, "x", &[]).record();
        let forged = BundleRecord {
            manifest: record.manifest.replace("\"x\"", "\"y\""),
            ..record
        };
        let index = SearchIndex::rebuild([forged]);
        assert!(index.is_empty());
    }

    #[test]
    fn an_overlong_query_is_refused() {
        let error = gallery()
            .search(&SearchQuery {
                text: "a".repeat(MAX_QUERY_BYTES + 1),
                ..SearchQuery::default()
            })
            .unwrap_err();
        assert_eq!(error, SearchError::QueryTooLong(MAX_QUERY_BYTES + 1));
    }
}
