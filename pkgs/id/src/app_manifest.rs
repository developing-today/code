//! App-folder manifests and host-side discovery.
//!
//! Symlink rule for app folders (resolve and contain): discovery
//! canonicalizes each candidate app folder and the apps root. A
//! candidate whose resolved path lies outside the canonicalized apps
//! root is reported as a named per-app error and is never registered,
//! so a symlinked folder cannot smuggle an external folder in as a
//! local app. A symlink whose resolved target stays inside the apps
//! root registers normally, under the symlink's own folder name.
//! Symlinks *inside* a real app folder are dispositioned in the
//! Phase 4 table below (issue #4203).
//!
//! # Phase 4 disposition table (issue #4203, audit of #4202)
//!
//! Every `AppManifest` field and every `discover_apps` behaviour,
//! dispositioned against current code as: gated by a named rule,
//! deliberately ungated with the reason, or routed to a named owner.
//! "Gated" means `parse_app_manifest` (parse) or `discover_apps`
//! (discovery) rejects or reports the item with a named error.
//!
//! | Item | Disposition |
//! | --- | --- |
//! | `app.id` | Gated (parse): kebab-case (`is_kebab_id`) and equal to the folder name. |
//! | `app.name` | Gated (parse): non-blank only. Deliberately ungated beyond that: a display string no consumer keys on; padding is stored verbatim. |
//! | `app.version` | Gated (parse): non-blank only. Deliberately ungated beyond that: no semver rule; no consumer parses it. |
//! | `app.description` | Deliberately ungated: optional free display text; absent is legal. |
//! | `program.kind` | Gated (parse): non-blank only. Vocabulary deliberately ungated: a free string with one checked-in value (`roc-world`); no enumeration until a second kind exists (required disposition 5). |
//! | `program.source` | Gated (parse): exactly one of `source`/`module_hash`; non-empty relative path, every component a plain name. Gated (discovery): the named path must exist in the folder. |
//! | `program.module_hash` | Gated (parse): exactly one of `source`/`module_hash`; 1 to 64 plain hex digits (the world-apply shape). |
//! | `interfaces.cli.commands` | Gated (parse): at least one of `cli`/`web` must be present; a present `cli` needs at least one command; the one entry rule (non-blank, unpadded, no duplicates). |
//! | `interfaces.web.mode` | Gated (parse): exactly `adapter` or `direct`. |
//! | `interfaces.web.route` | Gated (parse): the web-path shape rule (`is_safe_web_path`); a manifest whose route equals its own socket is rejected (route and socket both bind the path). Gated (discovery): unique across the discovered set in the single route/socket namespace — a route colliding with a registered app's route or socket (and vice versa) is rejected; first folder in sort order wins, the loser is a named error. Host-table collisions are gated at the binding layer (disposition 6). |
//! | `interfaces.web.socket` | Gated (parse): the web-path shape rule when present; a manifest whose socket equals its own route is rejected (route and socket both bind the path). Gated (discovery): unique across the discovered set in the single route/socket namespace — a socket colliding with a registered app's socket or route (and vice versa) is rejected, same first-wins rule. |
//! | `capabilities.request` | Gated (parse): the one entry rule. Vocabulary deliberately ungated: capability names are an open vocabulary; the gate guarantees only the exact-match shape (non-blank, unpadded, distinct). |
//! | `resources.files` | Gated (parse): the one entry rule plus folder confinement: non-empty, relative, every component a plain name, no empty/`.`/`..` segment (`is_confined_relative_path`); glob segments are ordinary names. Empty `files = []` is deliberately legal (required disposition 2): the section is optional and packages nothing, and no consumer in this crate reads the list as a defect. |
//! | Unknown fields, any level | Gated (parse): `deny_unknown_fields` on every struct; missing required fields fail the TOML parse. |
//! | Root read (discovery) | Gated: a root that cannot be canonicalized or read, missing included, is one named error entry naming the root, and zero apps; a readable empty root is zero apps and zero errors, so the three states stay distinguishable. |
//! | Entry filtering (discovery) | Immediate children only, no recursion. Non-directories and folders without `app.toml` are skipped silently, deliberately: not every folder is an app. An entry that errors while iterating is a named error, not a silent skip. |
//! | Per-app error isolation (discovery) | Gated: parse failure, containment failure, missing `program.source`, and duplicate route/socket each become a named per-app error and never stop the other folders from registering. |
//! | `program.source` existence check (discovery) | Existence only: `Path::exists` follows symlinks, including a symlink component inside the folder, and discovery never reads or executes through the path; see required disposition 1. |
//! | Sort order (discovery) | Folders are sorted by path before scanning; registration order is sort order, which is what makes first-wins for route/socket (and the structural `app.id` rule) deterministic. |
//!
//! ## The six required dispositions (#4203)
//!
//! 1. **Symlinks inside a real app folder.** Discovery-side rule:
//!    syntactic confinement only: `resources.files` entries and
//!    `program.source` are component-checked strings, and the source
//!    existence check follows symlinks without resolving or
//!    containing the target, because discovery opens neither path.
//!    Resolution confinement at read time belongs to the pack step,
//!    which is the named owner of the pack-time half: no pack step
//!    exists in this crate yet, and the one that is built (the
//!    packaging design's pack step) must resolve each named file
//!    inside the app folder before packaging or hashing it.
//! 2. **Empty `files = []`.** Deliberately legal; see the
//!    `resources.files` row. Recorded here so the posture is a
//!    decision, not an accident.
//! 3. **Route edge spellings.** Tightened in this audit:
//!    `is_safe_web_path` now rejects empty segments, so route `/`
//!    (no non-empty segment), a trailing slash (`/app/`), and
//!    doubled separators (`/a//b`) all fail the manifest gate for
//!    `route` and `socket` alike. Dot segments are rejected on the
//!    same rule (#5427): `/a/./b`, `/./a`, and `/a/.` normalize onto
//!    other paths, so they are alternate spellings, not new routes.
//!    `/` is additionally a host-registered route (disposition 6). Ordinary spellings
//!    (`/tic-tac-toe`, `/ws/world`) are unchanged.
//! 4. **App-`id` uniqueness.** Structural, no extra check added:
//!    `app.id` must equal the folder (entry) name, and one
//!    directory entry has one name, so two discovered apps cannot
//!    share an `id`. Two symlinks to one target cannot both
//!    register either: the manifest `id` can equal only one
//!    entry's name, and the other entry fails the id match with a
//!    named error.
//! 5. **`program.kind` vocabulary.** Disposition only; see the
//!    `program.kind` row. No enumeration without a second kind.
//! 6. **Uniqueness against the host's own routes.** Not a
//!    discovery gate: the host route table is owned by the binding
//!    layer, and the check lives there: `web::app_interface`
//!    rejects a declared route or socket that exactly or
//!    structurally collides with a host-registered route
//!    (`HOST_RESERVED_ROUTES`, issues #4779 and #4795), the layer
//!    the #2325 binding (PR #3894) created. Discovery keeps only
//!    the cross-app uniqueness it can see. No follow-up is filed:
//!    the gap #2325's scope left open was filed and landed as
//!    #4779/#4795, so the disposition is covered today.
//!
//! Consumer note (audit evidence): at audit time the only caller
//! of `discover_apps` is `commands::serve`, which logs each
//! discovered app's `id` and path and warns per error; no code in
//! this crate yet reads `resources.files`, executes
//! `program.source`, or binds a discovered app's route, which is
//! why the read/execute-time confinements above are assigned to
//! the steps that will.

#[cfg(test)]
use std::cell::Cell;
use std::collections::HashMap;
use std::fs;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

/// The strict, version-one description of one unpacked app folder.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppManifest {
    pub app: AppInfo,
    pub interfaces: Interfaces,
    #[serde(default)]
    pub program: Option<Program>,
    #[serde(default)]
    pub capabilities: Option<Capabilities>,
    #[serde(default)]
    pub resources: Option<Resources>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppInfo {
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Program {
    pub kind: String,
    #[serde(default)]
    pub source: Option<PathBuf>,
    #[serde(default)]
    pub module_hash: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Interfaces {
    #[serde(default)]
    pub cli: Option<CliInterface>,
    #[serde(default)]
    pub web: Option<WebInterface>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CliInterface {
    pub commands: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WebInterface {
    pub mode: String,
    pub route: String,
    #[serde(default)]
    pub socket: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Capabilities {
    #[serde(default)]
    pub request: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Resources {
    #[serde(default)]
    pub files: Vec<String>,
}

/// A manifest accepted by discovery, paired with its containing directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscoveredApp {
    pub path: PathBuf,
    pub manifest: AppManifest,
}

/// Results of scanning an apps root. A malformed app is reported without
/// preventing other app folders from being discovered.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AppDiscovery {
    pub apps: Vec<DiscoveredApp>,
    pub errors: Vec<String>,
}

/// Parse and validate a manifest against its containing folder name.
pub fn parse_app_manifest(source: &str, folder_name: &str) -> Result<AppManifest> {
    let manifest: AppManifest = toml::from_str(source).context("parse app.toml")?;
    ensure!(
        is_kebab_id(&manifest.app.id),
        "app.id must be a kebab-case identifier"
    );
    ensure!(
        manifest.app.id == folder_name,
        "app.id {:?} does not match folder name {:?}",
        manifest.app.id,
        folder_name
    );
    ensure!(
        !manifest.app.name.trim().is_empty(),
        "app.name must not be empty"
    );
    ensure!(
        !manifest.app.version.trim().is_empty(),
        "app.version must not be empty"
    );
    ensure!(
        manifest.interfaces.cli.is_some() || manifest.interfaces.web.is_some(),
        "at least one [interfaces.cli] or [interfaces.web] section is required"
    );
    if let Some(cli) = &manifest.interfaces.cli {
        ensure!(
            !cli.commands.is_empty(),
            "interfaces.cli.commands must list at least one command"
        );
        ensure_distinct_entries(&cli.commands, "interfaces.cli.commands")?;
    }
    if let Some(capabilities) = &manifest.capabilities {
        ensure_distinct_entries(&capabilities.request, "capabilities.request")?;
    }
    if let Some(resources) = &manifest.resources {
        ensure_distinct_entries(&resources.files, "resources.files")?;
        for entry in &resources.files {
            ensure!(
                is_confined_relative_path(entry),
                "resources.files entry {entry:?} must be a relative path inside the app folder"
            );
        }
    }
    if let Some(program) = &manifest.program {
        ensure!(
            !program.kind.trim().is_empty(),
            "program.kind must not be empty"
        );
        ensure!(
            program.source.is_some() ^ program.module_hash.is_some(),
            "[program] must set exactly one of source or module_hash"
        );
        if let Some(source) = &program.source {
            ensure!(
                !source.as_os_str().is_empty()
                    && source
                        .components()
                        .all(|part| matches!(part, Component::Normal(_))),
                "program.source must be a non-empty relative path inside the app folder"
            );
        }
        if let Some(hash) = &program.module_hash {
            // Same shape as the world-apply manifest's module_hash
            // (world_manifest.rs): 1 to 64 plain hex digits.
            let valid = !hash.is_empty()
                && hash.len() <= 64
                && hash.bytes().all(|byte| byte.is_ascii_hexdigit());
            ensure!(valid, "program.module_hash must be 1 to 64 hex digits");
        }
    }
    if let Some(web) = &manifest.interfaces.web {
        ensure!(
            matches!(web.mode.as_str(), "adapter" | "direct"),
            "interfaces.web.mode must be adapter or direct"
        );
        ensure!(
            is_safe_web_path(&web.route),
            "interfaces.web.route must be an absolute path without traversal, empty or dot segments, whitespace, ? or #"
        );
        if let Some(socket) = &web.socket {
            ensure!(
                is_safe_web_path(socket),
                "interfaces.web.socket must be an absolute path without traversal, empty or dot segments, whitespace, ? or #"
            );
            // One namespace at the binding layer (#6454): a section whose
            // route and socket name the same path can never bind, so parse
            // rejects it with the binding layer's own rule wording.
            ensure!(
                socket != &web.route,
                "[interfaces.web] route and socket both bind {:?}; they must differ",
                web.route
            );
        }
    }
    Ok(manifest)
}

#[derive(Default)]
struct WebBindingIndex {
    route_owners: HashMap<String, usize>,
    socket_owners: HashMap<String, usize>,
    #[cfg(test)]
    lookups: Cell<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WebBindingConflict<'a> {
    DuplicateRoute { owner_index: usize },
    RouteWithSocket { owner_index: usize },
    DuplicateSocket { owner_index: usize, socket: &'a str },
    SocketWithRoute { owner_index: usize, socket: &'a str },
}

impl WebBindingConflict<'_> {
    fn owner_index(self) -> usize {
        match self {
            Self::DuplicateRoute { owner_index }
            | Self::RouteWithSocket { owner_index }
            | Self::DuplicateSocket { owner_index, .. }
            | Self::SocketWithRoute { owner_index, .. } => owner_index,
        }
    }

    fn diagnostic(
        self,
        manifest_path: &Path,
        app_id: &str,
        web: &WebInterface,
        owner_id: &str,
    ) -> String {
        match self {
            Self::DuplicateRoute { .. } => format!(
                "{}: app {:?} duplicates interfaces.web.route {:?} already registered by app {:?}; app not registered",
                manifest_path.display(),
                app_id,
                web.route,
                owner_id
            ),
            Self::RouteWithSocket { .. } => format!(
                "{}: app {:?} route {:?} (interfaces.web.route) collides with interfaces.web.socket already registered by app {:?}; app not registered",
                manifest_path.display(),
                app_id,
                web.route,
                owner_id
            ),
            Self::DuplicateSocket { socket, .. } => format!(
                "{}: app {:?} duplicates interfaces.web.socket {:?} already registered by app {:?}; app not registered",
                manifest_path.display(),
                app_id,
                socket,
                owner_id
            ),
            Self::SocketWithRoute { socket, .. } => format!(
                "{}: app {:?} socket {:?} (interfaces.web.socket) collides with interfaces.web.route already registered by app {:?}; app not registered",
                manifest_path.display(),
                app_id,
                socket,
                owner_id
            ),
        }
    }
}

impl WebBindingIndex {
    fn route_owner(&self, route: &str) -> Option<usize> {
        #[cfg(test)]
        self.lookups.set(self.lookups.get() + 1);
        self.route_owners.get(route).copied()
    }

    fn socket_owner(&self, socket: &str) -> Option<usize> {
        #[cfg(test)]
        self.lookups.set(self.lookups.get() + 1);
        self.socket_owners.get(socket).copied()
    }

    fn first_conflict<'a>(&self, web: &'a WebInterface) -> Option<WebBindingConflict<'a>> {
        if let Some(owner_index) = self.route_owner(&web.route) {
            return Some(WebBindingConflict::DuplicateRoute { owner_index });
        }
        if let Some(owner_index) = self.socket_owner(&web.route) {
            return Some(WebBindingConflict::RouteWithSocket { owner_index });
        }
        if let Some(socket) = web.socket.as_deref() {
            if let Some(owner_index) = self.socket_owner(socket) {
                return Some(WebBindingConflict::DuplicateSocket {
                    owner_index,
                    socket,
                });
            }
            if let Some(owner_index) = self.route_owner(socket) {
                return Some(WebBindingConflict::SocketWithRoute {
                    owner_index,
                    socket,
                });
            }
        }
        None
    }

    fn register(&mut self, web: &WebInterface, owner_index: usize) {
        self.route_owners.insert(web.route.clone(), owner_index);
        if let Some(socket) = &web.socket {
            self.socket_owners.insert(socket.clone(), owner_index);
        }
    }

    #[cfg(test)]
    fn lookup_count(&self) -> usize {
        self.lookups.get()
    }
}

/// Scan immediate child folders for `app.toml`, isolating errors to each app.
pub fn discover_apps(root: impl AsRef<Path>) -> AppDiscovery {
    let root = root.as_ref();
    let mut discovery = AppDiscovery::default();
    // The canonicalized root is the containment boundary for the
    // resolve-and-contain symlink rule documented at the top of this
    // module. A root that cannot be canonicalized is reported exactly
    // like a root that cannot be read.
    let canonical_root = match fs::canonicalize(root) {
        Ok(path) => path,
        Err(error) => {
            discovery
                .errors
                .push(format!("{}: {error}", root.display()));
            return discovery;
        }
    };
    let mut web_bindings = WebBindingIndex::default();
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) => {
            discovery
                .errors
                .push(format!("{}: {error}", root.display()));
            return discovery;
        }
    };

    let mut folders = entries
        .filter_map(|entry| match entry {
            Ok(entry) => Some(entry.path()),
            Err(error) => {
                discovery
                    .errors
                    .push(format!("{}: {error}", root.display()));
                None
            }
        })
        .filter(|path| path.is_dir())
        .collect::<Vec<_>>();
    folders.sort();

    for folder in folders {
        let manifest_path = folder.join("app.toml");
        if !manifest_path.is_file() {
            continue;
        }
        // Resolve and contain: the candidate folder itself must resolve
        // to a path inside the canonicalized apps root. A resolution
        // failure or an outside resolution is a named per-app error and
        // the folder is not registered.
        let resolved = match fs::canonicalize(&folder) {
            Ok(path) => path,
            Err(error) => {
                discovery.errors.push(format!(
                    "{}: cannot resolve app folder {}: {error}",
                    folder.display(),
                    folder.display()
                ));
                continue;
            }
        };
        if !resolved.starts_with(&canonical_root) {
            discovery.errors.push(format!(
                "{}: app folder resolves to {} outside the apps root {} - symlinked app folders must resolve inside the apps root",
                folder.display(),
                resolved.display(),
                canonical_root.display()
            ));
            continue;
        }
        let folder_name = folder
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        let parsed = fs::read_to_string(&manifest_path)
            .with_context(|| format!("read {}", manifest_path.display()))
            .and_then(|source| parse_app_manifest(&source, folder_name));
        match parsed {
            Ok(manifest) => {
                if let Some(source) = manifest
                    .program
                    .as_ref()
                    .and_then(|program| program.source.as_ref())
                {
                    let source_path = folder.join(source);
                    if !source_path.exists() {
                        discovery.errors.push(format!(
                            "{}: program.source {} does not exist in app folder {}",
                            manifest_path.display(),
                            source.display(),
                            folder.display()
                        ));
                        continue;
                    }
                }
                if let Some(web) = manifest.interfaces.web.as_ref() {
                    if let Some(conflict) = web_bindings.first_conflict(web) {
                        let owner = &discovery.apps[conflict.owner_index()];
                        discovery.errors.push(conflict.diagnostic(
                            &manifest_path,
                            &manifest.app.id,
                            web,
                            &owner.manifest.app.id,
                        ));
                        continue;
                    }
                    web_bindings.register(web, discovery.apps.len());
                }
                discovery.apps.push(DiscoveredApp {
                    path: folder,
                    manifest,
                });
            }
            Err(error) => discovery
                .errors
                .push(format!("{}: {error:#}", manifest_path.display())),
        }
    }
    discovery
}

fn is_safe_web_path(path: &str) -> bool {
    path.starts_with('/')
        && !path.chars().any(char::is_whitespace)
        && !path.contains('?')
        && !path.contains('#')
        && !path.split('/').any(|segment| segment == "..")
        // Phase 4 edge spellings (#4203, disposition 3): no empty
        // segment, so `/` itself, a trailing slash, and doubled
        // separators are rejected, not just traversal segments.
        && !path.split('/').skip(1).any(|segment| segment.is_empty())
        // Dot segments (#5427): a `.` segment is the remaining RFC 3986
        // dot segment -- `/a/./b`, `/./a`, and `/a/.` pass the checks
        // above but normalize onto `/a/b`, `/a`, and `/a/`, so one
        // normalized path would pass as two distinct routes/sockets
        // past the exact-string uniqueness checks downstream.
        && !path.split('/').skip(1).any(|segment| segment == ".")
}

/// One consistent list-entry rule for manifest string lists
/// (`interfaces.cli.commands`, `capabilities.request`, `resources.files`):
/// every entry must be non-blank with no leading or trailing whitespace, and
/// no entry may repeat. Duplicate and blank comparisons use the trimmed form,
/// so a whitespace-padded entry can neither evade the duplicate gate nor be
/// stored verbatim — padded entries are rejected, not silently trimmed.
/// Rejections name the field: like the #4082 duplicate gate this extends, a
/// repeated or padded entry is an authoring mistake the gate should name
/// rather than silently absorb.
fn ensure_distinct_entries(entries: &[String], field: &str) -> Result<()> {
    let mut seen = std::collections::HashSet::new();
    for entry in entries {
        let trimmed = entry.trim();
        ensure!(
            !trimmed.is_empty(),
            "{field} must not contain empty entries"
        );
        ensure!(
            entry.as_str() == trimmed,
            "{field} must not contain entries with leading or trailing whitespace: {entry:?}"
        );
        ensure!(
            seen.insert(trimmed.to_owned()),
            "{field} must not contain duplicates"
        );
    }
    Ok(())
}

/// Folder confinement for `[resources].files` entries (#4059): the design
/// rule is that everything is relative to the app folder — no absolute
/// paths, no references outside the folder. The check mirrors the
/// `program.source` rule above: the entry must be non-empty and every
/// path component must be a plain name, so a root, `.` or `..` component
/// anywhere in the entry rejects it. Glob segments such as `**` are
/// ordinary components and stay legal (`assets/**`, `web/templates/**`).
fn is_confined_relative_path(entry: &str) -> bool {
    // Segment-level, not only component-level: `Path::components`
    // normalizes an internal `.` away (`a/./b` yields just `a`, `b`),
    // so the raw segments are checked as well. An empty segment rejects
    // doubled or trailing separators along with the empty entry itself.
    !entry.is_empty()
        && !Path::new(entry).is_absolute()
        && Path::new(entry)
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
        && entry
            .split('/')
            .all(|segment| !segment.is_empty() && segment != "." && segment != "..")
}

fn is_kebab_id(id: &str) -> bool {
    !id.is_empty()
        && !id.starts_with('-')
        && !id.ends_with('-')
        && !id.contains("--")
        && id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const MANIFEST: &str = r#"
[app]
id = "tic-tac-toe"
name = "Tic Tac Toe"
version = "0.1.0"

[program]
kind = "roc-world"
source = "program/"

[interfaces.web]
mode = "adapter"
route = "/tic-tac-toe"
"#;

    fn manifest_with_web(id: &str, route: &str, socket: Option<&str>) -> String {
        let socket = socket
            .map(|socket| format!("socket = {socket:?}\n"))
            .unwrap_or_default();
        format!(
            "[app]\nid = {id:?}\nname = {id:?}\nversion = \"0.1.0\"\n\n[program]\nkind = \"roc-world\"\nsource = \"program/\"\n\n[interfaces.web]\nmode = \"adapter\"\nroute = {route:?}\n{socket}"
        )
    }

    fn write_discoverable_app(root: &Path, folder: &str, manifest: &str) {
        let app = root.join(folder);
        fs::create_dir_all(app.join("program")).unwrap();
        fs::write(app.join("app.toml"), manifest).unwrap();
    }

    #[test]
    fn parses_app_manifest_and_matches_folder() {
        let manifest = parse_app_manifest(MANIFEST, "tic-tac-toe").unwrap();
        assert_eq!(manifest.app.id, "tic-tac-toe");
        assert_eq!(
            manifest.program.unwrap().source,
            Some(PathBuf::from("program/"))
        );
    }

    #[test]
    fn web_route_and_socket_must_be_safe_absolute_paths() {
        for route in ["/a/../b", "/a b", "/a?query", "/a#fragment"] {
            let source =
                MANIFEST.replace("route = \"/tic-tac-toe\"", &format!("route = {route:?}"));
            let error = parse_app_manifest(&source, "tic-tac-toe").unwrap_err();
            assert!(error.to_string().contains("interfaces.web.route"));
        }

        let absolute_url_socket = MANIFEST.replace(
            "route = \"/tic-tac-toe\"",
            "route = \"/tic-tac-toe\"\nsocket = \"https://example.test/socket\"",
        );
        let error = parse_app_manifest(&absolute_url_socket, "tic-tac-toe").unwrap_err();
        assert!(error.to_string().contains("interfaces.web.socket"));

        let valid_socket = MANIFEST.replace(
            "route = \"/tic-tac-toe\"",
            "route = \"/tic-tac-toe\"\nsocket = \"/ws/world\"",
        );
        assert!(parse_app_manifest(&valid_socket, "tic-tac-toe").is_ok());
    }

    #[test]
    fn web_route_edge_spellings_are_rejected() {
        // Phase 4 disposition 3 (#4203): `/`, a trailing slash, and
        // doubled separators fail the shape rule for route and socket.
        for route in ["/", "/tic-tac-toe/", "/tic-tac-toe//x", "//tic-tac-toe"] {
            let source =
                MANIFEST.replace("route = \"/tic-tac-toe\"", &format!("route = {route:?}"));
            let error = parse_app_manifest(&source, "tic-tac-toe").unwrap_err();
            assert!(
                error.to_string().contains("interfaces.web.route"),
                "route {route:?} rejection names the field: {error:#}"
            );
        }
        let trailing_socket = MANIFEST.replace(
            "route = \"/tic-tac-toe\"",
            "route = \"/tic-tac-toe\"\nsocket = \"/ws/world/\"",
        );
        let error = parse_app_manifest(&trailing_socket, "tic-tac-toe").unwrap_err();
        assert!(error.to_string().contains("interfaces.web.socket"));
    }

    #[test]
    fn test_web_route_dot_segments_are_rejected() {
        // #5427: a `.` segment is an alternate spelling of the path
        // without it (RFC 3986 remove_dot_segments), so it fails the
        // shape rule for route and socket alike, while a dotted name
        // that is not a whole segment stays legal.
        for route in ["/./tic-tac-toe", "/tic-tac-toe/./x", "/tic-tac-toe/."] {
            let source =
                MANIFEST.replace("route = \"/tic-tac-toe\"", &format!("route = {route:?}"));
            let error = parse_app_manifest(&source, "tic-tac-toe").unwrap_err();
            assert!(
                error.to_string().contains("interfaces.web.route"),
                "route {route:?} rejection names the field: {error:#}"
            );
        }
        let dot_socket = MANIFEST.replace(
            "route = \"/tic-tac-toe\"",
            "route = \"/tic-tac-toe\"\nsocket = \"/ws/./world\"",
        );
        let error = parse_app_manifest(&dot_socket, "tic-tac-toe").unwrap_err();
        assert!(error.to_string().contains("interfaces.web.socket"));

        let dotted_names = MANIFEST.replace(
            "route = \"/tic-tac-toe\"",
            "route = \"/tic-tac-toe\"\nsocket = \"/ws/world.min\"",
        );
        assert!(parse_app_manifest(&dotted_names, "tic-tac-toe").is_ok());
    }

    #[test]
    fn checked_in_tic_tac_toe_manifest_remains_valid() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("apps/tic-tac-toe/app.toml");
        let source = fs::read_to_string(path).unwrap();
        let manifest = parse_app_manifest(&source, "tic-tac-toe").unwrap();
        assert_eq!(manifest.interfaces.web.unwrap().route, "/tic-tac-toe");
    }

    #[test]
    fn discovery_keeps_first_duplicate_route_and_names_loser() {
        let root = tempfile::tempdir().unwrap();
        write_discoverable_app(
            root.path(),
            "a-first",
            &manifest_with_web("a-first", "/same", None),
        );
        write_discoverable_app(
            root.path(),
            "b-second",
            &manifest_with_web("b-second", "/same", None),
        );

        let discovery = discover_apps(root.path());

        assert_eq!(discovery.apps.len(), 1);
        assert_eq!(discovery.apps[0].manifest.app.id, "a-first");
        assert_eq!(discovery.errors.len(), 1);
        assert_eq!(
            discovery.errors[0],
            format!(
                "{}: app \"b-second\" duplicates interfaces.web.route \"/same\" already registered by app \"a-first\"; app not registered",
                root.path().join("b-second/app.toml").display()
            )
        );
    }

    #[test]
    fn discovery_keeps_first_duplicate_socket_and_names_loser() {
        let root = tempfile::tempdir().unwrap();
        write_discoverable_app(
            root.path(),
            "a-first",
            &manifest_with_web("a-first", "/first", Some("/ws/shared")),
        );
        write_discoverable_app(
            root.path(),
            "b-second",
            &manifest_with_web("b-second", "/second", Some("/ws/shared")),
        );

        let discovery = discover_apps(root.path());

        assert_eq!(discovery.apps.len(), 1);
        assert_eq!(discovery.apps[0].manifest.app.id, "a-first");
        assert_eq!(discovery.errors.len(), 1);
        assert_eq!(
            discovery.errors[0],
            format!(
                "{}: app \"b-second\" duplicates interfaces.web.socket \"/ws/shared\" already registered by app \"a-first\"; app not registered",
                root.path().join("b-second/app.toml").display()
            )
        );
    }

    #[test]
    fn parse_rejects_self_route_equal_socket() {
        // #6454 (X3): the binding layer rejects a single section whose
        // route and socket name the same path; parse rejects it too.
        let source = manifest_with_web("solo", "/same", Some("/same"));
        let error = parse_app_manifest(&source, "solo").unwrap_err();
        assert!(
            error.to_string().contains("both bind"),
            "self route==socket rejection names the rule: {error:#}"
        );
        assert!(error.to_string().contains("/same"));
    }

    #[test]
    fn discovery_rejects_route_colliding_with_registered_socket() {
        // #6454 (X1): a-first registers socket /shared; b-second's route
        // /shared collides in the single binding namespace.
        let root = tempfile::tempdir().unwrap();
        write_discoverable_app(
            root.path(),
            "a-first",
            &manifest_with_web("a-first", "/a", Some("/shared")),
        );
        write_discoverable_app(
            root.path(),
            "b-second",
            &manifest_with_web("b-second", "/shared", None),
        );

        let discovery = discover_apps(root.path());

        assert_eq!(discovery.apps.len(), 1);
        assert_eq!(discovery.apps[0].manifest.app.id, "a-first");
        assert_eq!(discovery.errors.len(), 1);
        assert_eq!(
            discovery.errors[0],
            format!(
                "{}: app \"b-second\" route \"/shared\" (interfaces.web.route) collides with interfaces.web.socket already registered by app \"a-first\"; app not registered",
                root.path().join("b-second/app.toml").display()
            )
        );
    }

    #[test]
    fn discovery_rejects_socket_colliding_with_registered_route() {
        // #6454 (X2): a-first registers route /shared; b-second's socket
        // /shared collides in the single binding namespace.
        let root = tempfile::tempdir().unwrap();
        write_discoverable_app(
            root.path(),
            "a-first",
            &manifest_with_web("a-first", "/shared", None),
        );
        write_discoverable_app(
            root.path(),
            "b-second",
            &manifest_with_web("b-second", "/b", Some("/shared")),
        );

        let discovery = discover_apps(root.path());

        assert_eq!(discovery.apps.len(), 1);
        assert_eq!(discovery.apps[0].manifest.app.id, "a-first");
        assert_eq!(discovery.errors.len(), 1);
        assert_eq!(
            discovery.errors[0],
            format!(
                "{}: app \"b-second\" socket \"/shared\" (interfaces.web.socket) collides with interfaces.web.route already registered by app \"a-first\"; app not registered",
                root.path().join("b-second/app.toml").display()
            )
        );
    }

    #[test]
    fn web_binding_index_uses_constant_lookups_with_large_registry() {
        let mut index = WebBindingIndex::default();
        for owner_index in 0..10_000 {
            let web = WebInterface {
                mode: "adapter".to_owned(),
                route: format!("/apps/{owner_index}"),
                socket: Some(format!("/ws/{owner_index}")),
            };
            index.register(&web, owner_index);
        }

        let candidate = WebInterface {
            mode: "adapter".to_owned(),
            route: "/apps/candidate".to_owned(),
            socket: Some("/ws/candidate".to_owned()),
        };
        assert_eq!(index.first_conflict(&candidate), None);
        // Oracle: each checked map access increments the test-only counter.
        // A route and socket candidate use exactly four lookups regardless
        // of the number of already-registered apps; there is no timing bound.
        assert_eq!(index.lookup_count(), 4);
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]
        /// #6454 Codex finding on PR #6470: the cross-kind uniqueness
        /// invariant needs generated coverage, not only fixed examples.
        /// Generate app sequences whose routes/sockets collide across
        /// kinds and compare discovery against a sorted, first-wins
        /// reference model over the single binding namespace.
        #[test]
        fn discovery_cross_kind_namespace_matches_first_wins_model(
            specs in proptest::collection::vec(
                (0usize..4, proptest::option::of(0usize..4)),
                0..9,
            )
        ) {
            const PATHS: [&str; 4] = ["/a", "/b", "/c", "/d"];
            let root = tempfile::tempdir().unwrap();
            for (index, (route, socket)) in specs.iter().enumerate() {
                let id = format!("app-{index:02}");
                write_discoverable_app(
                    root.path(),
                    &id,
                    &manifest_with_web(&id, PATHS[*route], (*socket).map(|s| PATHS[s])),
                );
            }
            // Reference model: folders scan in sort order (the
            // zero-padded ids sort in generation order); an app is
            // rejected when its route or socket is already claimed in
            // the shared namespace, or when route == socket (the parse
            // rejection); otherwise it claims both of its paths.
            let mut claimed: Vec<&str> = Vec::new();
            let mut accepted: Vec<String> = Vec::new();
            let mut rejected: Vec<String> = Vec::new();
            for (index, (route, socket)) in specs.iter().enumerate() {
                let id = format!("app-{index:02}");
                let route = PATHS[*route];
                let socket = (*socket).map(|s| PATHS[s]);
                let self_collision = socket == Some(route);
                let namespace_collision = claimed.contains(&route)
                    || socket.is_some_and(|s| claimed.contains(&s));
                if self_collision || namespace_collision {
                    rejected.push(id);
                } else {
                    claimed.push(route);
                    if let Some(s) = socket {
                        claimed.push(s);
                    }
                    accepted.push(id);
                }
            }
            let discovery = discover_apps(root.path());
            let found: Vec<&str> = discovery
                .apps
                .iter()
                .map(|app| app.manifest.app.id.as_str())
                .collect();
            prop_assert_eq!(
                found,
                accepted.iter().map(String::as_str).collect::<Vec<_>>()
            );
            // Rejection isolation: exactly one named error per rejected
            // app, in scan order, and no other errors.
            prop_assert_eq!(discovery.errors.len(), rejected.len());
            for (error, id) in discovery.errors.iter().zip(&rejected) {
                prop_assert!(
                    error.contains(id.as_str()),
                    "error names the rejected app {id}: {error}"
                );
            }
            // Namespace uniqueness across the registered set.
            let mut bound: Vec<&str> = Vec::new();
            for app in &discovery.apps {
                let web = app.manifest.interfaces.web.as_ref().unwrap();
                bound.push(web.route.as_str());
                if let Some(socket) = &web.socket {
                    bound.push(socket.as_str());
                }
            }
            let mut unique = bound.clone();
            unique.sort_unstable();
            unique.dedup();
            prop_assert_eq!(bound.len(), unique.len());
        }
    }

    #[test]
    fn discovery_rejects_self_route_equal_socket() {
        // #6454 (X3) at discovery: the app is not registered and the
        // named parse error surfaces as its per-app error.
        let root = tempfile::tempdir().unwrap();
        write_discoverable_app(
            root.path(),
            "a-solo",
            &manifest_with_web("a-solo", "/same", Some("/same")),
        );

        let discovery = discover_apps(root.path());

        assert_eq!(discovery.apps.len(), 0);
        assert_eq!(discovery.errors.len(), 1);
        assert!(discovery.errors[0].contains("both bind"));
    }

    #[test]
    fn discovery_registers_distinct_cross_kind_paths() {
        // #6454 (C3 control): route/socket pairs that share no path
        // across kinds register fully.
        let root = tempfile::tempdir().unwrap();
        write_discoverable_app(
            root.path(),
            "a-first",
            &manifest_with_web("a-first", "/a", Some("/ws/a")),
        );
        write_discoverable_app(
            root.path(),
            "b-second",
            &manifest_with_web("b-second", "/b", Some("/ws/b")),
        );

        let discovery = discover_apps(root.path());

        assert_eq!(discovery.apps.len(), 2);
        assert!(discovery.errors.is_empty());
    }

    #[test]
    fn rejects_unknown_fields_at_nested_levels() {
        let unknown_app = MANIFEST.replace(
            "name = \"Tic Tac Toe\"",
            "name = \"Tic Tac Toe\"\nextra = true",
        );
        assert!(parse_app_manifest(&unknown_app, "tic-tac-toe").is_err());
        let unknown_web = MANIFEST.replace(
            "route = \"/tic-tac-toe\"",
            "route = \"/tic-tac-toe\"\nextra = true",
        );
        assert!(parse_app_manifest(&unknown_web, "tic-tac-toe").is_err());
    }

    #[test]
    fn rejects_folder_id_mismatch_and_unsafe_source() {
        assert!(parse_app_manifest(MANIFEST, "other-app").is_err());
        let unsafe_source = MANIFEST.replace("program/", "../outside");
        assert!(parse_app_manifest(&unsafe_source, "tic-tac-toe").is_err());
    }

    #[test]
    fn rejects_blank_name_and_version() {
        let blank_name = MANIFEST.replace("name = \"Tic Tac Toe\"", "name = \"  \"");
        assert!(parse_app_manifest(&blank_name, "tic-tac-toe").is_err());
        let blank_version = MANIFEST.replace("version = \"0.1.0\"", "version = \"\"");
        assert!(parse_app_manifest(&blank_version, "tic-tac-toe").is_err());
    }

    const CLI_MANIFEST: &str = r#"
[app]
id = "tic-tac-toe"
name = "Tic Tac Toe"
version = "0.1.0"

[interfaces.cli]
commands = ["play"]
"#;

    #[test]
    fn cli_interface_must_offer_a_real_command() {
        // A valid CLI-only manifest parses.
        assert!(parse_app_manifest(CLI_MANIFEST, "tic-tac-toe").is_ok());
        // Zero commands do not satisfy the at-least-one-interface rule.
        let no_commands = CLI_MANIFEST.replace("commands = [\"play\"]", "commands = []");
        assert!(parse_app_manifest(&no_commands, "tic-tac-toe").is_err());
        // Blank command names are rejected.
        let blank_command =
            CLI_MANIFEST.replace("commands = [\"play\"]", "commands = [\"play\", \"  \"]");
        assert!(parse_app_manifest(&blank_command, "tic-tac-toe").is_err());
    }

    #[test]
    fn rejects_duplicate_and_padded_cli_commands() {
        // Exact duplicate commands are rejected, naming the field.
        let duplicate =
            CLI_MANIFEST.replace("commands = [\"play\"]", "commands = [\"play\", \"play\"]");
        let error = parse_app_manifest(&duplicate, "tic-tac-toe").unwrap_err();
        assert!(
            error.to_string().contains("interfaces.cli.commands"),
            "duplicate error names the field: {error:#}"
        );
        // A whitespace-padded entry cannot evade the duplicate gate.
        let padded_duplicate =
            CLI_MANIFEST.replace("commands = [\"play\"]", "commands = [\"play\", \" play \"]");
        let error = parse_app_manifest(&padded_duplicate, "tic-tac-toe").unwrap_err();
        assert!(
            error.to_string().contains("interfaces.cli.commands"),
            "padded-duplicate error names the field: {error:#}"
        );
        // A lone padded entry is rejected rather than stored verbatim.
        let padded = CLI_MANIFEST.replace("commands = [\"play\"]", "commands = [\" play \"]");
        assert!(parse_app_manifest(&padded, "tic-tac-toe").is_err());
    }

    #[test]
    fn rejects_padded_capability_requests() {
        // A padded duplicate cannot evade the #4082 duplicate gate.
        let padded_duplicate =
            format!("{MANIFEST}\n[capabilities]\nrequest = [\"store.read\", \" store.read \"]\n");
        let error = parse_app_manifest(&padded_duplicate, "tic-tac-toe").unwrap_err();
        assert!(
            error.to_string().contains("capabilities.request"),
            "padded-duplicate error names the field: {error:#}"
        );
        // A lone padded entry is rejected rather than stored untrimmed.
        let padded = format!("{MANIFEST}\n[capabilities]\nrequest = [\" store.read \"]\n");
        assert!(parse_app_manifest(&padded, "tic-tac-toe").is_err());
        // The unpadded baseline still parses.
        let valid = format!("{MANIFEST}\n[capabilities]\nrequest = [\"store.read\"]\n");
        assert!(parse_app_manifest(&valid, "tic-tac-toe").is_ok());
    }

    #[test]
    fn rejects_blank_and_duplicate_capability_requests() {
        // Duplicate rule: duplicates are REJECTED, not deduplicated — a
        // repeated request entry is an authoring mistake the gate names.
        let valid = format!("{MANIFEST}\n[capabilities]\nrequest = [\"store.read\"]\n");
        assert!(parse_app_manifest(&valid, "tic-tac-toe").is_ok());
        let blank = format!("{MANIFEST}\n[capabilities]\nrequest = [\"\", \"store.read\"]\n");
        assert!(parse_app_manifest(&blank, "tic-tac-toe").is_err());
        let duplicate =
            format!("{MANIFEST}\n[capabilities]\nrequest = [\"store.read\", \"store.read\"]\n");
        assert!(parse_app_manifest(&duplicate, "tic-tac-toe").is_err());
    }

    #[test]
    fn rejects_blank_duplicate_and_padded_resource_files() {
        for files in [
            r#"["", "assets/**"]"#,
            r#"["  ", "assets/**"]"#,
            r#"["assets/**", "assets/**"]"#,
            r#"["assets/**", " assets/**"]"#,
        ] {
            let source = format!("{MANIFEST}\n[resources]\nfiles = {files}\n");
            let error = parse_app_manifest(&source, "tic-tac-toe").unwrap_err();
            assert!(
                error.to_string().contains("resources.files"),
                "resource-file validation error names the field: {error:#}"
            );
        }
    }

    #[test]
    fn rejects_unconfined_resource_files() {
        // Folder confinement (#4059): [resources].files entries are paths
        // the pack step will read, so absolute paths, parent traversal
        // anywhere in the entry, and dot paths are rejected with an error
        // naming the offending entry.
        for entry in [
            "/etc/passwd",
            "../../etc/passwd",
            "../secret",
            "a/../../b",
            ".",
            "./assets",
            "a/./b",
            "",
        ] {
            let source = format!("{MANIFEST}\n[resources]\nfiles = [\"{entry}\"]\n");
            let error = parse_app_manifest(&source, "tic-tac-toe").unwrap_err();
            let message = error.to_string();
            assert!(
                message.contains("resources.files") && message.contains(entry),
                "rejection of {entry:?} names the field and the entry: {message}"
            );
        }
        // Confined entries — plain files, nested paths, and the design's
        // glob shapes — stay accepted.
        for entry in ["assets/logo.png", "web/templates/index.html", "assets/**"] {
            let source = format!("{MANIFEST}\n[resources]\nfiles = [\"{entry}\"]\n");
            assert!(
                parse_app_manifest(&source, "tic-tac-toe").is_ok(),
                "confined entry {entry:?} is accepted"
            );
        }
    }

    #[test]
    fn accepts_design_resource_files_and_empty_optional_list() {
        let design_example =
            format!("{MANIFEST}\n[resources]\nfiles = [\"assets/**\", \"web/templates/**\"]\n");
        assert!(parse_app_manifest(&design_example, "tic-tac-toe").is_ok());

        // The resources section is optional, and an explicitly empty files
        // list remains legal.
        let empty = format!("{MANIFEST}\n[resources]\nfiles = []\n");
        assert!(parse_app_manifest(&empty, "tic-tac-toe").is_ok());
    }

    #[test]
    fn module_hash_must_be_bounded_plain_hex() {
        let with_hash = |hash: &str| {
            MANIFEST.replace(
                "source = \"program/\"",
                &format!("module_hash = \"{hash}\""),
            )
        };
        // Non-hex text is not a hash.
        assert!(parse_app_manifest(&with_hash("not-a-hash"), "tic-tac-toe").is_err());
        // Empty and over-64-digit hashes are rejected (world-apply rule).
        assert!(parse_app_manifest(&with_hash(""), "tic-tac-toe").is_err());
        assert!(parse_app_manifest(&with_hash(&"a".repeat(65)), "tic-tac-toe").is_err());
        // A 64-digit hex hash is accepted.
        assert!(parse_app_manifest(&with_hash(&"a".repeat(64)), "tic-tac-toe").is_ok());
        assert!(parse_app_manifest(&with_hash("0123456789abcdef"), "tic-tac-toe").is_ok());
    }

    #[test]
    fn discovery_reports_missing_root() {
        let parent = tempfile::tempdir().unwrap();
        let missing_root = parent.path().join("missing-apps-root");

        let discovery = discover_apps(&missing_root);

        assert!(discovery.apps.is_empty());
        assert_eq!(discovery.errors.len(), 1);
        assert!(discovery.errors[0].contains(&missing_root.display().to_string()));
    }

    #[test]
    fn discovery_reports_bad_app_and_keeps_valid_app() {
        let root = tempfile::tempdir().unwrap();
        let valid = root.path().join("tic-tac-toe");
        let invalid = root.path().join("mismatched");
        fs::create_dir_all(&valid).unwrap();
        fs::create_dir_all(&invalid).unwrap();
        fs::create_dir_all(valid.join("program")).unwrap();
        fs::write(valid.join("app.toml"), MANIFEST).unwrap();
        fs::write(invalid.join("app.toml"), MANIFEST).unwrap();

        let discovery = discover_apps(root.path());
        assert_eq!(discovery.apps.len(), 1);
        assert_eq!(discovery.apps[0].manifest.app.id, "tic-tac-toe");
        assert_eq!(discovery.errors.len(), 1);
        assert!(discovery.errors[0].contains("does not match folder name"));
    }

    #[test]
    fn discovery_rejects_app_whose_program_source_is_missing() {
        let root = tempfile::tempdir().unwrap();
        let ghost = root.path().join("tic-tac-toe");
        fs::create_dir_all(&ghost).unwrap();
        fs::write(ghost.join("app.toml"), MANIFEST).unwrap();

        let discovery = discover_apps(root.path());
        assert_eq!(discovery.apps.len(), 0);
        assert_eq!(discovery.errors.len(), 1);
        assert!(discovery.errors[0].contains("program.source"));
        assert!(discovery.errors[0].contains("program/"));
        assert!(discovery.errors[0].contains("does not exist"));
    }
    #[cfg(unix)]
    #[test]
    fn discovery_rejects_symlinked_app_folder_pointing_outside_root() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let external_app = outside.path().join("tic-tac-toe");
        fs::create_dir_all(external_app.join("program")).unwrap();
        fs::write(external_app.join("app.toml"), MANIFEST).unwrap();
        symlink(&external_app, root.path().join("tic-tac-toe")).unwrap();

        let discovery = discover_apps(root.path());

        assert!(
            discovery.apps.is_empty(),
            "outside symlink must not register: {:?}",
            discovery.apps
        );
        assert_eq!(discovery.errors.len(), 1);
        assert!(
            discovery.errors[0].contains("tic-tac-toe"),
            "error names the app folder: {}",
            discovery.errors[0]
        );
        assert!(
            discovery.errors[0].contains("outside the apps root"),
            "error names the containment rule: {}",
            discovery.errors[0]
        );
    }

    #[cfg(unix)]
    #[test]
    fn discovery_registers_symlinked_app_folder_resolving_inside_root() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        // The real app is nested under the root but is not an immediate
        // child, so the only discovery path to it is through the symlink.
        let stored = root.path().join("stored").join("tic-tac-toe");
        fs::create_dir_all(stored.join("program")).unwrap();
        fs::write(stored.join("app.toml"), MANIFEST).unwrap();
        symlink(&stored, root.path().join("tic-tac-toe")).unwrap();

        let discovery = discover_apps(root.path());

        assert_eq!(
            discovery.errors.len(),
            0,
            "in-root symlink errors: {:?}",
            discovery.errors
        );
        assert_eq!(discovery.apps.len(), 1);
        assert_eq!(discovery.apps[0].manifest.app.id, "tic-tac-toe");
    }
}
