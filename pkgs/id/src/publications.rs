//! Dynamic program publications (#2147, slice 1: the persisted registry).
//!
//! Issue #2147 makes publishing a runtime decision instead of a
//! launch-time one: while a node runs, an operator must be able to
//! publish and unpublish programs and pages — on new listeners and at
//! paths on an existing listener — without restarting the node. The
//! published set must persist so a restart restores it, and it must be
//! inspectable.
//!
//! This module is the registry that set lives in: the record type, the
//! placement rules, and the JSON persistence. It is deliberately the
//! part with no sockets in it. Later slices wire it to live listeners
//! (the runtime apply step) and to a CLI; this slice wires the boot
//! half — `id serve` loads the persisted registry at startup and
//! re-issues the boot warnings #2147 requires (see below).
//!
//! ## The path-override rule (#2147, binding on this module)
//!
//! A path binding on the same listener as the World Explorer OVERRIDES
//! the Explorer's own route where the paths overlap — allowed and
//! intended — but the server must warn loudly at bind time, and at
//! boot for persisted bindings, naming the shadowed route, unless an
//! explicit acknowledgement is set for that binding. So:
//!
//! - [`Registry::publish`] refuses a path mount that shadows an
//!   Explorer route when `acknowledge_shadow` is false, naming the
//!   shadowed route in the error.
//! - With the acknowledgement set, publish succeeds and returns the
//!   loud warning text for the caller to surface.
//! - [`Registry::boot_warnings`] re-derives the warnings at boot:
//!   every persisted path mount that shadows an Explorer route without
//!   an acknowledgement produces one. (An acknowledged mount stays
//!   quiet at boot too — the acknowledgement is part of the record.)
//!
//! Shadowing is a segment-boundary prefix relation in either
//! direction: the mount `/world` shadows the Explorer route `/world`
//! and `/world/anything`; the mount `/` shadows every route; a mount
//! at `/worlds` shadows nothing (no segment boundary).
//!
//! ## Persistence
//!
//! The registry is one JSON file, [`publications_file`]: the
//! `ID_PUBLICATIONS_FILE` override, else `publications.json` inside
//! the store directory ([`crate::STORE_PATH`]). Writes are atomic
//! (write a sibling temp file, then rename), so a crash mid-save
//! leaves the previous set intact. A missing file is an empty
//! registry, not an error; a malformed file is an error naming the
//! file — a serve that cannot read its persisted publications must
//! say so loudly rather than silently restore nothing.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};

/// Explorer route prefixes a path mount can shadow, as path prefixes
/// at segment granularity. Parameterised routes are named by their
/// static prefix (`/edit/{*name}` is `/edit`). Grounded in
/// `crate::web::routes` / `crate::web::explore` /
/// `crate::web::world_ws` at the time of writing; the registry takes
/// the route list as a parameter everywhere it matters, so callers
/// with a fresher list can pass their own — this constant is the
/// fallback for the boot path, which runs before any router exists.
pub const KNOWN_EXPLORER_ROUTES: &[&str] = &[
    "/",
    "/api",
    "/assets",
    "/blob",
    "/edit",
    "/explore",
    "/files",
    "/hash",
    "/invite",
    "/peers",
    "/settings",
    "/view",
    "/world",
    "/ws",
];

/// Where a published program is placed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Placement {
    /// A dedicated listener for this program alone, serving at `/`.
    Listener {
        /// The listener's socket address (port 0 = OS-assigned).
        addr: SocketAddr,
    },
    /// A path mount on the node's primary (Explorer) listener.
    PathMount {
        /// The canonical mount path (see [`canonicalize_mount_path`]).
        path: String,
    },
}

impl Placement {
    /// A stable one-line description for logs, warnings, and listings.
    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            Self::Listener { addr } => format!("listener {addr}"),
            Self::PathMount { path } => format!("path {path}"),
        }
    }
}

/// One published program: a world (program) target at one placement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Publication {
    /// The world/program name being published (the #2146 target model:
    /// world names are target identity).
    pub world: String,
    /// Where it is published.
    pub placement: Placement,
    /// The operator's explicit "I know this shadows an Explorer
    /// route" acknowledgement for path mounts. Meaningless (and
    /// ignored) for listener placements, which shadow nothing.
    #[serde(default)]
    pub acknowledge_shadow: bool,
}

/// The persisted set of publications.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Registry {
    /// Publications in publish order.
    #[serde(default)]
    pub publications: Vec<Publication>,
}

/// Canonicalise a path-mount path, or explain why it cannot be one.
///
/// Canonical form: starts with `/`, no empty / `.` / `..` segments,
/// no trailing slash except the root itself. `/` is a valid mount
/// (it shadows the whole Explorer — the publish rule handles that).
///
/// # Errors
///
/// Fails on a relative path or on `.` / `..` / empty segments.
pub fn canonicalize_mount_path(path: &str) -> Result<String> {
    ensure!(
        path.starts_with('/'),
        "mount path must start with '/': {path:?}"
    );
    let mut segments = Vec::new();
    for segment in path.split('/').skip(1) {
        match segment {
            "" if segments.is_empty() && path == "/" => {}
            "" => bail!("mount path has an empty segment: {path:?}"),
            "." | ".." => bail!("mount path must not contain '{segment}' segments: {path:?}"),
            _ => segments.push(segment),
        }
    }
    if segments.is_empty() {
        return Ok("/".to_string());
    }
    Ok(format!("/{}", segments.join("/")))
}

/// Normalise a mount path's spelling for comparison purposes:
/// collapse duplicate separators, drop `.` segments and any trailing
/// slash. Unlike [`canonicalize_mount_path`] this never fails — it
/// exists so records that bypassed `publish()` (hand-edited files,
/// direct `publications` pushes) still compare and shadow-match by
/// the placement they actually name, not by their raw spelling:
/// `//world` is the `/world` mount, and `/api/` is `/api`.
fn normalise_mount_spelling(path: &str) -> String {
    let segments: Vec<&str> = path
        .split('/')
        .filter(|segment| !segment.is_empty() && *segment != ".")
        .collect();
    if segments.is_empty() {
        "/".to_string()
    } else {
        format!("/{}", segments.join("/"))
    }
}

/// Whether two placements name the same placement. Path mounts
/// compare by normalised spelling, matching unpublish and
/// boot_warnings: a stored record that bypassed publish() (a
/// hand-edited file, a direct `publications` push) must still block
/// a canonical re-spelling of itself, or the one-program-per-
/// placement check is defeated by `//world` or `/api/`. Listeners
/// compare by exact SocketAddr equality.
fn same_placement(a: &Placement, b: &Placement) -> bool {
    match (a, b) {
        (Placement::PathMount { path: x }, Placement::PathMount { path: y }) => {
            normalise_mount_spelling(x) == normalise_mount_spelling(y)
        }
        _ => a == b,
    }
}

/// Whether mount path `mount` shadows Explorer route `route`:
/// a segment-boundary prefix relation in either direction.
fn shadows(mount: &str, route: &str) -> bool {
    mount == route
        || mount == "/"
        || route.starts_with(&format!("{mount}/"))
        || mount.starts_with(&format!("{route}/"))
}

/// The first Explorer route in `explorer_routes` that the canonical
/// mount path `mount_path` shadows, if any.
#[must_use]
pub fn shadowed_explorer_route(mount_path: &str, explorer_routes: &[&str]) -> Option<String> {
    explorer_routes
        .iter()
        .find(|route| shadows(mount_path, route))
        .map(|route| (*route).to_string())
}

/// The loud warning text for a shadowed route (#2147's wording duty:
/// warn loudly, naming the shadowed route).
fn shadow_warning(publication: &Publication, route: &str) -> String {
    let Placement::PathMount { path } = &publication.placement else {
        return String::new();
    };
    format!(
        "WARNING: publication of world '{}' at path {} shadows the World Explorer route {} \
         on this listener — the publication overrides the Explorer there",
        publication.world, path, route
    )
}

impl Registry {
    /// Publish `publication`. Path mounts are canonicalised in place.
    ///
    /// Returns the loud shadow warning when the mount shadows an
    /// Explorer route and the operator acknowledged it; `None` when
    /// nothing is shadowed.
    ///
    /// # Errors
    ///
    /// Fails when the placement is already taken (one program per
    /// placement — a second publish must unpublish first, never
    /// silently replace), when the world is empty, when the mount
    /// path is not canonicalisable, or when the mount shadows an
    /// Explorer route without `acknowledge_shadow`.
    pub fn publish(
        &mut self,
        mut publication: Publication,
        explorer_routes: &[&str],
    ) -> Result<Option<String>> {
        ensure!(
            !publication.world.trim().is_empty(),
            "publication world name must not be empty"
        );
        if let Placement::PathMount { path } = &mut publication.placement {
            *path = canonicalize_mount_path(path)?;
        }
        ensure!(
            !self
                .publications
                .iter()
                .any(|existing| same_placement(&existing.placement, &publication.placement)),
            "placement already published: {} (unpublish it first)",
            publication.placement.describe()
        );
        let mut warning = None;
        if let Placement::PathMount { path } = &publication.placement {
            if let Some(route) = shadowed_explorer_route(path, explorer_routes) {
                ensure!(
                    publication.acknowledge_shadow,
                    "publishing world '{}' at path {} would shadow the World Explorer route {} \
                     — pass the shadow acknowledgement to publish anyway",
                    publication.world,
                    path,
                    route
                );
                warning = Some(shadow_warning(&publication, &route));
            }
        }
        self.publications.push(publication);
        Ok(warning)
    }

    /// Remove the publication at `placement`, returning it.
    ///
    /// # Errors
    ///
    /// Fails when nothing is published at that placement.
    pub fn unpublish(&mut self, placement: &Placement) -> Result<Publication> {
        // Compare placements by the mount they name, not by raw
        // spelling: a record stored under a non-canonical spelling
        // (it bypassed publish(), which canonicalises) must still be
        // removable by its canonical name.
        let wanted = match placement {
            Placement::PathMount { path } => {
                ensure!(
                    path.starts_with('/'),
                    "mount path must start with '/': {path:?}"
                );
                Placement::PathMount {
                    path: normalise_mount_spelling(path),
                }
            }
            listener => listener.clone(),
        };
        let position = self
            .publications
            .iter()
            .position(|existing| match &existing.placement {
                Placement::PathMount { path } => {
                    wanted
                        == Placement::PathMount {
                            path: normalise_mount_spelling(path),
                        }
                }
                listener => *listener == wanted,
            })
            .with_context(|| format!("nothing published at {}", wanted.describe()))?;
        Ok(self.publications.remove(position))
    }

    /// The boot warnings for this persisted set (#2147): one loud
    /// warning per path mount that shadows an Explorer route without
    /// an acknowledgement. Acknowledged mounts are quiet, exactly as
    /// the issue specifies.
    #[must_use]
    pub fn boot_warnings(&self, explorer_routes: &[&str]) -> Vec<String> {
        self.publications
            .iter()
            .filter(|publication| !publication.acknowledge_shadow)
            .filter_map(|publication| match &publication.placement {
                // Match on the normalised spelling: a stored `//world`
                // shadows the route `/world`, and the warning must
                // name that route — matching the raw string instead
                // names `/` (the only route `//world` textually
                // prefixes), which is the wrong route.
                Placement::PathMount { path } => {
                    shadowed_explorer_route(&normalise_mount_spelling(path), explorer_routes)
                        .map(|route| shadow_warning(publication, &route))
                }
                Placement::Listener { .. } => None,
            })
            .collect()
    }

    /// Save the registry atomically: write a sibling temp file, then
    /// rename over `path`, so a crash mid-save keeps the previous set.
    ///
    /// # Errors
    ///
    /// Fails on any I/O or serialisation error, with the target file
    /// named in the context.
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        let json = serde_json::to_string_pretty(self).context("serialising publications")?;
        let temp = path.with_extension("json.tmp");
        std::fs::write(&temp, json).with_context(|| format!("writing {}", temp.display()))?;
        std::fs::rename(&temp, path).with_context(|| format!("replacing {}", path.display()))?;
        Ok(())
    }

    /// Load the registry from `path`. A missing file is an empty
    /// registry; a malformed one is an error naming the file. A file
    /// that parses but violates the invariants `publish()` enforces
    /// (empty world name, non-canonical mount path, duplicate
    /// placement) is likewise an error naming the file — see
    /// `validate_persisted`.
    ///
    /// # Errors
    ///
    /// Fails when the file exists but cannot be read, parsed, or
    /// validated.
    pub fn load(path: &Path) -> Result<Self> {
        // Read directly and treat only NotFound as an empty registry:
        // probing with `exists()` first would collapse permission and
        // metadata failures into "no registry", silently restoring
        // nothing — the fail-loud contract above (the same pattern
        // `GroupLog::open` uses).
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("reading publications from {}", path.display()));
            }
        };
        let registry: Self = serde_json::from_str(&text)
            .with_context(|| format!("parsing publications from {}", path.display()))?;
        registry
            .validate_persisted()
            .with_context(|| format!("validating publications from {}", path.display()))?;
        Ok(registry)
    }

    /// The load-side counterpart of [`Registry::publish`]'s
    /// invariants. `save()` only ever writes sets `publish()` built,
    /// so a persisted record that violates them is hand-edited or
    /// corrupt — and restoring it verbatim would resurrect exactly
    /// what `publish()` refuses: an empty world name, a mount path
    /// stored under a non-canonical spelling (which the boot shadow
    /// check then misjudges and `unpublish` cannot name), or the
    /// same placement twice. Fail loudly instead, per the module's
    /// persistence contract. The shadow gate itself is *not*
    /// re-checked here: an unacknowledged shadow mount is a boot
    /// warning ([`Registry::boot_warnings`]), not a corrupt record.
    fn validate_persisted(&self) -> Result<()> {
        let mut seen: Vec<&Placement> = Vec::new();
        for publication in &self.publications {
            ensure!(
                !publication.world.trim().is_empty(),
                "publication world name must not be empty"
            );
            if let Placement::PathMount { path } = &publication.placement {
                let canonical = canonicalize_mount_path(path)?;
                ensure!(
                    canonical == *path,
                    "mount path is not stored in canonical form: {path:?} \
                     (canonical form is {canonical:?})"
                );
            }
            ensure!(
                !seen.contains(&&publication.placement),
                "duplicate publication placement: {}",
                publication.placement.describe()
            );
            seen.push(&publication.placement);
        }
        Ok(())
    }
}

/// The file the persisted publications live in: the
/// `ID_PUBLICATIONS_FILE` override, else `publications.json` inside
/// the store directory.
#[must_use]
pub fn publications_file() -> PathBuf {
    std::env::var_os("ID_PUBLICATIONS_FILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(crate::STORE_PATH).join("publications.json"))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn listener(addr: &str) -> Publication {
        Publication {
            world: "arena".to_string(),
            placement: Placement::Listener {
                addr: addr.parse().unwrap(),
            },
            acknowledge_shadow: false,
        }
    }

    fn mount(path: &str, acknowledge_shadow: bool) -> Publication {
        Publication {
            world: "arena".to_string(),
            placement: Placement::PathMount {
                path: path.to_string(),
            },
            acknowledge_shadow,
        }
    }

    #[test]
    fn canonical_paths() {
        assert_eq!(canonicalize_mount_path("/").unwrap(), "/");
        assert_eq!(canonicalize_mount_path("/arena").unwrap(), "/arena");
        assert_eq!(
            canonicalize_mount_path("/games/arena").unwrap(),
            "/games/arena"
        );
        assert!(canonicalize_mount_path("arena").is_err());
        assert!(canonicalize_mount_path("/games//arena").is_err());
        assert!(canonicalize_mount_path("/games/../arena").is_err());
        assert!(canonicalize_mount_path("/games/./arena").is_err());
        assert!(canonicalize_mount_path("/arena/").is_err());
    }

    #[test]
    fn shadow_detection_is_segment_boundaried() {
        let routes = KNOWN_EXPLORER_ROUTES;
        assert_eq!(
            shadowed_explorer_route("/world", routes).as_deref(),
            Some("/world")
        );
        assert_eq!(shadowed_explorer_route("/", routes).as_deref(), Some("/"));
        assert_eq!(
            shadowed_explorer_route("/api", routes).as_deref(),
            Some("/api")
        );
        assert_eq!(
            shadowed_explorer_route("/assets", routes).as_deref(),
            Some("/assets")
        );
        assert_eq!(
            shadowed_explorer_route("/assets/app.js", routes).as_deref(),
            Some("/assets")
        );
        assert_eq!(shadowed_explorer_route("/worlds", routes), None);
        assert_eq!(shadowed_explorer_route("/arena", routes), None);
    }

    #[test]
    fn publish_rejects_unacknowledged_shadow_and_warns_on_acknowledged() {
        let mut registry = Registry::default();
        let err = registry
            .publish(mount("/world", false), KNOWN_EXPLORER_ROUTES)
            .unwrap_err();
        assert!(format!("{err:#}").contains("/world"), "{err:#}");
        assert!(registry.publications.is_empty());

        let warning = registry
            .publish(mount("/world", true), KNOWN_EXPLORER_ROUTES)
            .unwrap();
        let warning = warning.expect("acknowledged shadow must warn");
        assert!(warning.contains("WARNING"), "{warning}");
        assert!(warning.contains("/world"), "{warning}");
        assert_eq!(registry.publications.len(), 1);
    }

    #[test]
    fn publish_allows_unshadowed_mount_and_listener_without_warning() {
        let mut registry = Registry::default();
        assert!(
            registry
                .publish(mount("/arena", false), KNOWN_EXPLORER_ROUTES)
                .unwrap()
                .is_none()
        );
        assert!(
            registry
                .publish(listener("127.0.0.1:4001"), KNOWN_EXPLORER_ROUTES)
                .unwrap()
                .is_none()
        );
        assert_eq!(registry.publications.len(), 2);
    }

    #[test]
    fn publish_rejects_a_taken_placement() {
        let mut registry = Registry::default();
        registry
            .publish(mount("/arena", false), KNOWN_EXPLORER_ROUTES)
            .unwrap();
        let mut other = mount("/arena", false);
        other.world = "other".to_string();
        assert!(registry.publish(other, KNOWN_EXPLORER_ROUTES).is_err());
        assert_eq!(registry.publications.len(), 1);
    }

    #[test]
    fn publish_rejects_a_duplicate_of_a_trailing_slash_stored_spelling() {
        // #6459: the stored record bypassed publish() (direct push),
        // so its spelling was never canonicalised; the canonical
        // re-spelling names the same placement and must be refused.
        let mut registry = Registry::default();
        registry.publications.push(mount("/arena/", false));
        let mut other = mount("/arena", false);
        other.world = "other".to_string();
        let err = registry.publish(other, KNOWN_EXPLORER_ROUTES).unwrap_err();
        assert!(
            format!("{err:#}").contains("placement already published"),
            "{err:#}"
        );
        assert_eq!(registry.publications.len(), 1);
    }

    #[test]
    fn publish_rejects_a_duplicate_of_a_doubled_separator_stored_spelling() {
        let mut registry = Registry::default();
        registry.publications.push(mount("//arena", false));
        let mut other = mount("/arena", false);
        other.world = "other".to_string();
        assert!(registry.publish(other, KNOWN_EXPLORER_ROUTES).is_err());
        assert_eq!(registry.publications.len(), 1);
    }

    #[test]
    fn publish_rejects_a_duplicate_shadow_mount_despite_acknowledgement() {
        // The duplicate check fires before shadow handling: an
        // acknowledged shadow does not entitle a second program to
        // a placement a non-canonical stored record already holds.
        let mut registry = Registry::default();
        registry.publications.push(mount("/world/", false));
        let err = registry
            .publish(mount("/world", true), KNOWN_EXPLORER_ROUTES)
            .unwrap_err();
        assert!(
            format!("{err:#}").contains("placement already published"),
            "{err:#}"
        );
        assert_eq!(registry.publications.len(), 1);
    }

    #[test]
    fn publish_allows_a_genuinely_different_mount_beside_a_stored_spelling() {
        let mut registry = Registry::default();
        registry.publications.push(mount("/api/", false));
        assert!(
            registry
                .publish(mount("/arena", false), KNOWN_EXPLORER_ROUTES)
                .unwrap()
                .is_none()
        );
        assert_eq!(registry.publications.len(), 2);
    }

    #[test]
    fn unpublish_removes_by_placement_and_errors_when_absent() {
        let mut registry = Registry::default();
        registry
            .publish(mount("/arena", false), KNOWN_EXPLORER_ROUTES)
            .unwrap();
        let removed = registry
            .unpublish(&Placement::PathMount {
                path: "/arena".to_string(),
            })
            .unwrap();
        assert_eq!(removed.world, "arena");
        assert!(registry.publications.is_empty());
        assert!(
            registry
                .unpublish(&Placement::PathMount {
                    path: "/arena".to_string(),
                })
                .is_err()
        );
    }

    #[test]
    fn save_load_round_trips_and_missing_file_is_empty() {
        let dir = std::env::temp_dir().join(format!("id-publications-test-{}", std::process::id()));
        let file = dir.join("publications.json");
        let missing = Registry::load(&file).unwrap();
        assert!(missing.publications.is_empty());

        let mut registry = Registry::default();
        registry
            .publish(mount("/world", true), KNOWN_EXPLORER_ROUTES)
            .unwrap();
        registry
            .publish(listener("127.0.0.1:4001"), KNOWN_EXPLORER_ROUTES)
            .unwrap();
        registry.save(&file).unwrap();
        let loaded = Registry::load(&file).unwrap();
        assert_eq!(loaded, registry);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_fails_loud_on_unreadable_path_not_just_missing() {
        // A directory is not a missing file: load must error, not
        // silently restore an empty registry.
        let dir = std::env::temp_dir();
        assert!(Registry::load(&dir).is_err());
    }

    #[test]
    fn boot_warnings_only_for_unacknowledged_shadows() {
        let mut registry = Registry::default();
        registry
            .publish(mount("/world", true), KNOWN_EXPLORER_ROUTES)
            .unwrap();
        registry
            .publish(mount("/arena", false), KNOWN_EXPLORER_ROUTES)
            .unwrap();
        assert!(registry.boot_warnings(KNOWN_EXPLORER_ROUTES).is_empty());

        // A record persisted outside publish() (hand-edited, or routes
        // changed since) still warns at boot.
        registry.publications.push(mount("/api", false));
        let warnings = registry.boot_warnings(KNOWN_EXPLORER_ROUTES);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("/api"), "{}", warnings[0]);
    }

    fn write_registry_file(name: &str, registry: &Registry) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "id-publications-test-{}-{name}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("publications.json");
        std::fs::write(&file, serde_json::to_string(registry).unwrap()).unwrap();
        file
    }

    #[test]
    fn load_rejects_a_duplicate_placement_naming_the_file() {
        let mut first = mount("/arena", false);
        first.world = "one".to_string();
        let mut second = mount("/arena", false);
        second.world = "two".to_string();
        let file = write_registry_file(
            "dup",
            &Registry {
                publications: vec![first, second],
            },
        );
        let err = Registry::load(&file).unwrap_err();
        let text = format!("{err:#}");
        assert!(text.contains("duplicate"), "{text}");
        assert!(
            text.contains(&file.display().to_string()),
            "error must name the file: {text}"
        );
    }

    #[test]
    fn load_rejects_an_empty_world_name() {
        let mut record = mount("/arena", false);
        record.world = "  ".to_string();
        let file = write_registry_file(
            "empty-world",
            &Registry {
                publications: vec![record],
            },
        );
        let err = Registry::load(&file).unwrap_err();
        assert!(format!("{err:#}").contains("world name"), "{err:#}");
    }

    #[test]
    fn load_rejects_a_non_canonical_mount_path() {
        for spelling in ["/api/", "//world", "/games//arena"] {
            let file = write_registry_file(
                "non-canonical",
                &Registry {
                    publications: vec![mount(spelling, false)],
                },
            );
            assert!(
                Registry::load(&file).is_err(),
                "stored spelling {spelling} must not load"
            );
        }
    }

    fn write_raw_registry_file(name: &str, raw: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "id-publications-test-{}-{name}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("publications.json");
        std::fs::write(&file, raw).unwrap();
        file
    }

    #[test]
    fn load_refuses_duplicate_json_keys_naming_the_file() {
        // #6504: a key spelled twice must be refused outright, never
        // collapsed (last wins) before `validate_persisted` runs — an
        // invalid first spelling would otherwise be silently restored
        // as the valid second one. Each case hides an invalid value
        // first; the controls below prove each shape loads or fails on
        // its own merits.
        let cases: [(&str, &str, &str); 3] = [
            (
                "dup-publications",
                "publications",
                r#"{"publications":[{"world":"","placement":{"kind":"path_mount","path":"//bad"}}],
                    "publications":[{"world":"arena","placement":{"kind":"path_mount","path":"/arena"}}]}"#,
            ),
            (
                "dup-world",
                "world",
                r#"{"publications":[{"world":"","world":"arena",
                    "placement":{"kind":"path_mount","path":"/arena"}}]}"#,
            ),
            (
                "dup-path",
                "path",
                r#"{"publications":[{"world":"arena",
                    "placement":{"kind":"path_mount","path":"//bad","path":"/good"}}]}"#,
            ),
        ];
        for (name, key, raw) in cases {
            let file = write_raw_registry_file(name, raw);
            let err = Registry::load(&file)
                .expect_err(&format!("duplicate `{key}` must be refused ({name})"));
            let text = format!("{err:#}");
            assert!(
                text.contains("duplicate field"),
                "{name}: error must say duplicate field: {text}"
            );
            assert!(
                text.contains(&format!("`{key}`")),
                "{name}: error must name the key `{key}`: {text}"
            );
            assert!(
                text.contains(&file.display().to_string()),
                "{name}: error must name the file: {text}"
            );
        }
    }

    #[test]
    fn load_accepts_the_single_spelling_of_each_duplicate_key_case() {
        // Controls for the duplicate-key test: the same records with
        // each key spelled once (the valid spelling) load.
        let raw = r#"{"publications":[{"world":"arena",
            "placement":{"kind":"path_mount","path":"/arena"}}]}"#;
        let file = write_raw_registry_file("single-spelling", raw);
        let loaded = Registry::load(&file).unwrap();
        assert_eq!(loaded.publications, vec![mount("/arena", false)]);
    }

    #[test]
    fn boot_warning_names_the_route_for_any_spelling() {
        // Stored outside publish(): `//world` is the `/world` mount,
        // and the warning must name `/world` — matching the raw
        // string names `/` instead.
        let mut registry = Registry::default();
        registry.publications.push(mount("//world", false));
        let warnings = registry.boot_warnings(KNOWN_EXPLORER_ROUTES);
        assert_eq!(warnings.len(), 1);
        assert!(
            warnings[0].contains("route /world"),
            "warning must name the shadowed route: {}",
            warnings[0]
        );
    }

    #[test]
    fn unpublish_by_canonical_name_removes_a_non_canonically_stored_record() {
        let mut registry = Registry::default();
        registry.publications.push(mount("/api/", false));
        let removed = registry
            .unpublish(&Placement::PathMount {
                path: "/api".to_string(),
            })
            .unwrap();
        assert_eq!(removed.world, "arena");
        assert!(registry.publications.is_empty());
    }
}
