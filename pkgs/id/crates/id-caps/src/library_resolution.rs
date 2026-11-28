//! Transitive requirement resolution for library imports (`#5956` §4,
//! `#10027` §7 step 2).
//!
//! An import names a library one of the three ways `#5969`'s taxonomy
//! provides — registry, upload, or URL — and resolution always settles
//! identity by **manifest hash**: registry imports pin the hash the
//! name + version resolved to, uploads become registry records first
//! (there is no direct-from-upload path that skips the record), and URL
//! imports carry the pinned content hash of their `(URL, hash, version)`
//! identity. This module walks the dependency edges the registry
//! records for each manifest, unions the declared requirement sets of
//! the whole closure ([`crate::library_manifest::LibraryManifest`],
//! §7 step 1), refuses dependency cycles naming the cycle, and produces
//! the **resolution record** §4 defines: importing world, library
//! name + manifest hash, source kind, the transitive requirement set as
//! resolved, the platform surface checked against, and the outcome.
//!
//! Scope of this step: resolution and the record only. The §3 subset
//! checks and §5 diagnostics are the compile-path import gate (§7
//! step 3), which consumes the record this module produces; a record
//! whose outcome is [`ResolutionOutcome::Resolved`] says the closure
//! resolved, not that any world may import it. Refusals are record
//! outcomes, not errors: a refused import is exactly as auditable as
//! an accepted one, so [`resolve_import`] does not fail.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::library_manifest::{LibraryManifest, PlatformRequirement};

/// How the imported library was named (`#5969` taxonomy, `#5956` §4).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImportSource {
    /// Named by name + version in the registry; resolution pinned the
    /// manifest hash it resolved to.
    Registry,
    /// Uploaded, and therefore a registry record before it was
    /// imported — the registry case with an upload provenance.
    Upload,
    /// Named by URL; identity is the triple (URL, pinned content hash,
    /// version) per `#5969` §6.
    Url {
        /// The URL the library was named by.
        url: String,
    },
}

/// One import to resolve.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportRequest {
    /// The importing world/app the record is produced for.
    pub importing_world: String,
    /// The library name as imported.
    pub library_name: String,
    /// The library version as imported.
    pub library_version: String,
    /// The manifest hash resolution settled on — the identity every
    /// "which library was this" question is answered by.
    pub manifest_hash: String,
    /// How the library was named.
    pub source: ImportSource,
    /// For URL imports: whether the fetched bytes have been verified
    /// against the pinned hash. Until they are, the library's declared
    /// requirements participate as *declared-not-checked* (§4) and the
    /// record says so. Registry and upload imports are records already
    /// and ignore this flag.
    pub url_bytes_verified: bool,
}

/// The surface a resolution was checked against (§3): the platform
/// identity plus the catalog version the compiling world is built
/// against, recorded so a verdict can be re-derived later.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckedSurface {
    /// The selected platform's identity (e.g. its package path).
    pub platform: String,
    /// The `id-caps` catalog version the check ran against.
    pub catalog_version: String,
}

/// One registry record as the resolver needs it: the manifest, plus
/// the manifest hashes of the library's dependencies as the registry
/// recorded them at upload/registration time.
pub struct RegistryEntry<'a> {
    /// The library's parsed, validated manifest.
    pub manifest: &'a LibraryManifest,
    /// Manifest hashes of the library's dependencies, in registry
    /// record order.
    pub dependencies: &'a [String],
}

/// The registry view the resolver reads. There is no registry code
/// path in the tree yet (step 1's note); this trait is the seam it
/// will implement, keyed by manifest hash exactly as §4 requires.
pub trait ManifestRegistry {
    /// The registry entry recorded under a manifest hash, if any.
    fn entry(&self, manifest_hash: &str) -> Option<RegistryEntry<'_>>;
}

/// Why a resolution was refused. Each variant names its subject
/// exhaustively — a refusal that cannot say what it refused is not
/// auditable (§4/§5 posture, applied at resolution time).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResolutionRefusal {
    /// The dependency graph has a cycle; resolution refuses it and
    /// names the cycle (§1: "Cycles in the dependency graph are
    /// refused at resolution time, naming the cycle"). The path runs
    /// from the first occurrence of the repeated manifest back to it,
    /// as `name (hash)` entries.
    Cycle {
        /// The cycle path, first and last entries the same manifest.
        cycle: Vec<String>,
    },
    /// A manifest hash in the closure has no registry record: the
    /// dependency it names cannot be identified, so nothing about it
    /// can be checked.
    UnknownManifest {
        /// The hash with no registry record.
        hash: String,
    },
    /// Two manifests in the closure pin the same catalog entry to
    /// different versions. One compiling catalog carries one version
    /// of an entry, so no resolution can satisfy both pins; the
    /// conflict is refused here rather than silently resolved to
    /// either pin.
    PinConflict {
        /// The catalog entry name pinned inconsistently.
        name: String,
        /// The distinct pinned versions found, ascending.
        pins: Vec<u32>,
    },
}

/// The outcome recorded on a resolution record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResolutionOutcome {
    /// The closure resolved; the record's requirement set is the
    /// library's transitive set as resolved.
    Resolved,
    /// Resolution refused, with the reason named.
    Refused(ResolutionRefusal),
}

/// One manifest of the resolved closure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClosureEntry {
    /// The library's manifest name.
    pub name: String,
    /// Its manifest hash — its identity.
    pub hash: String,
}

/// The resolution record of §4. It is what makes a build reproducible
/// and a refusal auditable: one hash settles which library, the
/// closure settles which libraries it pulled in, the requirement set
/// is the transitive set as resolved, and the checked surface settles
/// what the resolution was derived against.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolutionRecord {
    /// The importing world/app.
    pub importing_world: String,
    /// The imported library's name.
    pub library_name: String,
    /// The imported library's version as imported.
    pub library_version: String,
    /// The manifest hash resolution settled on.
    pub manifest_hash: String,
    /// How the library was named.
    pub source: ImportSource,
    /// True when the requirements in this record are
    /// declared-not-checked: a URL import whose bytes have not yet
    /// been fetched and verified against the pin (§4).
    pub declared_only: bool,
    /// The resolved dependency closure, root first, in first-visit
    /// order. On a cycle refusal this holds the partial closure
    /// visited before the cycle was found.
    pub closure: Vec<ClosureEntry>,
    /// The transitive requirement set as resolved, sorted by catalog
    /// name, with version pins merged (an unpinned declaration accepts
    /// any version and merges into a pinned one; two different pins
    /// are a [`ResolutionRefusal::PinConflict`]). Empty on refusal.
    pub transitive_requirements: Vec<PlatformRequirement>,
    /// The surface the resolution was checked against, when the
    /// caller supplied one.
    pub checked_surface: Option<CheckedSurface>,
    /// The outcome: resolved, or refused with the reason named.
    pub outcome: ResolutionOutcome,
}

/// Resolve one import against a registry view and produce its
/// resolution record. Never fails: refusals are outcomes on the
/// record, so every import attempt leaves the same auditable shape.
#[must_use]
pub fn resolve_import(
    registry: &dyn ManifestRegistry,
    request: &ImportRequest,
    checked_surface: Option<CheckedSurface>,
) -> ResolutionRecord {
    let mut walk = Walk {
        registry,
        state: HashMap::new(),
        stack: Vec::new(),
        closure: Vec::new(),
        pins: BTreeMap::new(),
    };
    let outcome = match walk.visit(&request.manifest_hash) {
        Ok(()) => match walk.merged_requirements() {
            Ok(requirements) => {
                return ResolutionRecord {
                    importing_world: request.importing_world.clone(),
                    library_name: request.library_name.clone(),
                    library_version: request.library_version.clone(),
                    manifest_hash: request.manifest_hash.clone(),
                    source: request.source.clone(),
                    declared_only: declared_only(request),
                    closure: walk.closure,
                    transitive_requirements: requirements,
                    checked_surface,
                    outcome: ResolutionOutcome::Resolved,
                };
            }
            Err(refusal) => ResolutionOutcome::Refused(refusal),
        },
        Err(refusal) => ResolutionOutcome::Refused(refusal),
    };
    ResolutionRecord {
        importing_world: request.importing_world.clone(),
        library_name: request.library_name.clone(),
        library_version: request.library_version.clone(),
        manifest_hash: request.manifest_hash.clone(),
        source: request.source.clone(),
        declared_only: declared_only(request),
        closure: walk.closure,
        transitive_requirements: Vec::new(),
        checked_surface,
        outcome,
    }
}

/// Whether the request's requirements participate as
/// declared-not-checked: only an unverified URL import (§4).
fn declared_only(request: &ImportRequest) -> bool {
    matches!(request.source, ImportSource::Url { .. }) && !request.url_bytes_verified
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mark {
    Visiting,
    Done,
}

struct Walk<'r> {
    registry: &'r dyn ManifestRegistry,
    state: HashMap<String, Mark>,
    /// Hashes on the current DFS path, for cycle naming.
    stack: Vec<String>,
    closure: Vec<ClosureEntry>,
    /// Requirement name → every version pin declared for it in the
    /// closure (`None` = unpinned).
    pins: BTreeMap<String, BTreeSet<Option<u32>>>,
}

impl Walk<'_> {
    fn visit(&mut self, hash: &str) -> Result<(), ResolutionRefusal> {
        match self.state.get(hash) {
            Some(Mark::Done) => return Ok(()),
            Some(Mark::Visiting) => {
                // Name the cycle: the stack from this hash's first
                // occurrence, closed by the hash itself.
                let start = self
                    .stack
                    .iter()
                    .position(|h| h.as_str() == hash)
                    .unwrap_or(0);
                let mut cycle: Vec<String> = self.stack[start..]
                    .iter()
                    .map(|h| self.describe(h))
                    .collect();
                cycle.push(self.describe(hash));
                return Err(ResolutionRefusal::Cycle { cycle });
            }
            None => {}
        }
        let entry =
            self.registry
                .entry(hash)
                .ok_or_else(|| ResolutionRefusal::UnknownManifest {
                    hash: hash.to_string(),
                })?;
        self.state.insert(hash.to_string(), Mark::Visiting);
        self.stack.push(hash.to_string());
        self.closure.push(ClosureEntry {
            name: entry.manifest.name.clone(),
            hash: hash.to_string(),
        });
        for requirement in &entry.manifest.requirements {
            self.pins
                .entry(requirement.name.clone())
                .or_default()
                .insert(requirement.version_pin);
        }
        let dependencies: Vec<String> = entry.dependencies.to_vec();
        for dependency in &dependencies {
            self.visit(dependency)?;
        }
        self.stack.pop();
        self.state.insert(hash.to_string(), Mark::Done);
        Ok(())
    }

    fn describe(&self, hash: &str) -> String {
        match self.registry.entry(hash) {
            Some(entry) => format!("{} ({hash})", entry.manifest.name),
            None => format!("<unknown> ({hash})"),
        }
    }

    /// Merge the closure's declared pins into the transitive
    /// requirement set: one entry per catalog name, sorted by name,
    /// pinned when any declaration pins it — and refused when two
    /// declarations pin it differently.
    fn merged_requirements(&self) -> Result<Vec<PlatformRequirement>, ResolutionRefusal> {
        let mut merged = Vec::with_capacity(self.pins.len());
        for (name, pins) in &self.pins {
            let distinct: Vec<u32> = pins.iter().copied().flatten().collect();
            if distinct.len() > 1 {
                return Err(ResolutionRefusal::PinConflict {
                    name: name.clone(),
                    pins: distinct,
                });
            }
            merged.push(PlatformRequirement {
                name: name.clone(),
                version_pin: distinct.first().copied(),
            });
        }
        Ok(merged)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::library_manifest::{BuildMode, LibraryKind};

    fn manifest(name: &str, hash: &str, requirements: &[(&str, Option<u32>)]) -> LibraryManifest {
        LibraryManifest {
            name: name.to_string(),
            version: "1.0.0".to_string(),
            kind: LibraryKind::SourceLibrary,
            build_mode: BuildMode::CompileIn,
            hash: hash.to_string(),
            license: "MIT".to_string(),
            requirements: requirements
                .iter()
                .map(|(name, pin)| PlatformRequirement {
                    name: (*name).to_string(),
                    version_pin: *pin,
                })
                .collect(),
            subtypes: Vec::new(),
            attachments: Vec::new(),
            migrations: Vec::new(),
            source_url: None,
        }
    }

    #[derive(Default)]
    struct TestRegistry {
        manifests: HashMap<String, LibraryManifest>,
        dependencies: HashMap<String, Vec<String>>,
    }

    impl TestRegistry {
        fn add(&mut self, manifest: LibraryManifest, dependencies: &[&str]) {
            self.dependencies.insert(
                manifest.hash.clone(),
                dependencies.iter().map(ToString::to_string).collect(),
            );
            self.manifests.insert(manifest.hash.clone(), manifest);
        }
    }

    impl ManifestRegistry for TestRegistry {
        fn entry(&self, manifest_hash: &str) -> Option<RegistryEntry<'_>> {
            let manifest = self.manifests.get(manifest_hash)?;
            let dependencies = self
                .dependencies
                .get(manifest_hash)
                .map_or(&[][..], Vec::as_slice);
            Some(RegistryEntry {
                manifest,
                dependencies,
            })
        }
    }

    fn request(hash: &str) -> ImportRequest {
        ImportRequest {
            importing_world: "world:test".to_string(),
            library_name: "root".to_string(),
            library_version: "1.0.0".to_string(),
            manifest_hash: hash.to_string(),
            source: ImportSource::Registry,
            url_bytes_verified: false,
        }
    }

    fn requirement_names(record: &ResolutionRecord) -> Vec<&str> {
        record
            .transitive_requirements
            .iter()
            .map(|r| r.name.as_str())
            .collect()
    }

    /// Oracle (§1): a library's requirement set is its own declared
    /// set union the declared sets of its dependencies, recursively —
    /// a diamond dependency contributes its requirements once.
    #[test]
    fn transitive_requirements_union_across_a_diamond() {
        let mut registry = TestRegistry::default();
        registry.add(
            manifest("root", "h-root", &[("RocCap", None)]),
            &["h-left", "h-right"],
        );
        registry.add(
            manifest("left", "h-left", &[("records.read", None)]),
            &["h-base"],
        );
        registry.add(
            manifest("right", "h-right", &[("time.now", Some(1))]),
            &["h-base"],
        );
        registry.add(manifest("base", "h-base", &[("chat.say", None)]), &[]);

        let record = resolve_import(&registry, &request("h-root"), None);
        assert_eq!(record.outcome, ResolutionOutcome::Resolved);
        assert_eq!(
            requirement_names(&record),
            ["RocCap", "chat.say", "records.read", "time.now"]
        );
        assert_eq!(record.closure.len(), 4, "diamond base counted once");
        assert_eq!(record.closure[0].hash, "h-root", "root first");
    }

    /// Oracle (§1): cycles in the dependency graph are refused at
    /// resolution time, naming the cycle.
    #[test]
    fn cycle_is_refused_and_named() {
        let mut registry = TestRegistry::default();
        registry.add(manifest("a", "h-a", &[]), &["h-b"]);
        registry.add(manifest("b", "h-b", &[]), &["h-a"]);

        let record = resolve_import(&registry, &request("h-a"), None);
        match &record.outcome {
            ResolutionOutcome::Refused(ResolutionRefusal::Cycle { cycle }) => {
                assert_eq!(cycle.len(), 3, "a → b → a: {cycle:?}");
                assert!(cycle[0].contains("h-a") && cycle[2].contains("h-a"));
                assert!(cycle[1].contains("h-b"));
            }
            other => panic!("expected a cycle refusal, got {other:?}"),
        }
        assert_eq!(
            record.transitive_requirements,
            [] as [crate::library_manifest::PlatformRequirement; 0]
        );
    }

    /// Oracle (§1): a library that depends on itself is the smallest
    /// cycle and is refused the same way.
    #[test]
    fn self_cycle_is_refused() {
        let mut registry = TestRegistry::default();
        registry.add(manifest("selfish", "h-self", &[]), &["h-self"]);
        let record = resolve_import(&registry, &request("h-self"), None);
        assert!(matches!(
            record.outcome,
            ResolutionOutcome::Refused(ResolutionRefusal::Cycle { .. })
        ));
    }

    /// Oracle (§4): identity is always a hash — a hash with no
    /// registry record names nothing, and the refusal names the hash.
    #[test]
    fn unknown_dependency_hash_is_refused_and_named() {
        let mut registry = TestRegistry::default();
        registry.add(manifest("root", "h-root", &[]), &["h-missing"]);
        let record = resolve_import(&registry, &request("h-root"), None);
        assert_eq!(
            record.outcome,
            ResolutionOutcome::Refused(ResolutionRefusal::UnknownManifest {
                hash: "h-missing".to_string()
            })
        );
    }

    /// Oracle (§1): pins are exact — two different pins for one
    /// catalog entry cannot both be satisfied by one catalog, so the
    /// resolution refuses and names the entry and both pins.
    #[test]
    fn conflicting_version_pins_across_the_closure_are_refused() {
        let mut registry = TestRegistry::default();
        registry.add(manifest("root", "h-root", &[]), &["h-a", "h-b"]);
        registry.add(manifest("a", "h-a", &[("time.now", Some(1))]), &[]);
        registry.add(manifest("b", "h-b", &[("time.now", Some(2))]), &[]);
        let record = resolve_import(&registry, &request("h-root"), None);
        assert_eq!(
            record.outcome,
            ResolutionOutcome::Refused(ResolutionRefusal::PinConflict {
                name: "time.now".to_string(),
                pins: vec![1, 2]
            })
        );
    }

    /// Oracle (§1): an unpinned requirement accepts any catalog
    /// version, so it merges into a pinned declaration of the same
    /// entry instead of conflicting with it.
    #[test]
    fn unpinned_and_pinned_declarations_merge_to_the_pin() {
        let mut registry = TestRegistry::default();
        registry.add(manifest("root", "h-root", &[("time.now", None)]), &["h-a"]);
        registry.add(manifest("a", "h-a", &[("time.now", Some(3))]), &[]);
        let record = resolve_import(&registry, &request("h-root"), None);
        assert_eq!(record.outcome, ResolutionOutcome::Resolved);
        assert_eq!(
            record.transitive_requirements,
            [PlatformRequirement {
                name: "time.now".to_string(),
                version_pin: Some(3)
            }]
        );
    }

    /// Oracle (§1/§5994 §5.5): a pure closure — no platform blocks
    /// anywhere — resolves with the empty requirement set.
    #[test]
    fn pure_closure_resolves_with_the_empty_requirement_set() {
        let mut registry = TestRegistry::default();
        registry.add(manifest("root", "h-root", &[]), &["h-leaf"]);
        registry.add(manifest("leaf", "h-leaf", &[]), &[]);
        let record = resolve_import(&registry, &request("h-root"), None);
        assert_eq!(record.outcome, ResolutionOutcome::Resolved);
        assert_eq!(
            record.transitive_requirements,
            [] as [crate::library_manifest::PlatformRequirement; 0]
        );
        assert!(!record.declared_only);
    }

    /// Oracle (§4): until a URL library's bytes are fetched and
    /// verified against the pin, its requirements participate as
    /// declared-not-checked and the record displays that state.
    #[test]
    fn unverified_url_import_records_declared_only() {
        let mut registry = TestRegistry::default();
        registry.add(manifest("root", "h-root", &[("RocCap", None)]), &[]);
        let mut url_request = request("h-root");
        url_request.source = ImportSource::Url {
            url: "https://example.invalid/root.kdl".to_string(),
        };
        let unverified = resolve_import(&registry, &url_request, None);
        assert!(unverified.declared_only);
        url_request.url_bytes_verified = true;
        let verified = resolve_import(&registry, &url_request, None);
        assert!(!verified.declared_only);
    }

    /// Oracle (§3/§4): the record carries the platform surface
    /// checked against — platform identity plus catalog version — so
    /// a verdict can be re-derived later.
    #[test]
    fn resolution_record_carries_the_checked_surface() {
        let mut registry = TestRegistry::default();
        registry.add(manifest("root", "h-root", &[]), &[]);
        let surface = CheckedSurface {
            platform: "roc-world/platform".to_string(),
            catalog_version: "2026-11".to_string(),
        };
        let record = resolve_import(&registry, &request("h-root"), Some(surface.clone()));
        assert_eq!(record.checked_surface, Some(surface));
        assert_eq!(record.importing_world, "world:test");
        assert_eq!(record.manifest_hash, "h-root");
    }
}
