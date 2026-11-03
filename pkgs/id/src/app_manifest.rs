//! App-folder manifests and host-side discovery.
//!
//! Symlink rule for app folders (resolve and contain): discovery
//! canonicalizes each candidate app folder and the apps root. A
//! candidate whose resolved path lies outside the canonicalized apps
//! root is reported as a named per-app error and is never registered,
//! so a symlinked folder cannot smuggle an external folder in as a
//! local app. A symlink whose resolved target stays inside the apps
//! root registers normally, under the symlink's own folder name.
//! Symlinks *inside* a real app folder are explicitly out of scope
//! here; they are owned by the Phase 4 audit (issue #4203).

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
            "interfaces.web.route must be an absolute path without traversal segments, whitespace, ? or #"
        );
        if let Some(socket) = &web.socket {
            ensure!(
                is_safe_web_path(socket),
                "interfaces.web.socket must be an absolute path without traversal segments, whitespace, ? or #"
            );
        }
    }
    Ok(manifest)
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
                    if let Some(existing) = discovery.apps.iter().find(|app| {
                        app.manifest
                            .interfaces
                            .web
                            .as_ref()
                            .map(|existing| existing.route.as_str())
                            == Some(web.route.as_str())
                    }) {
                        discovery.errors.push(format!(
                            "{}: app {:?} duplicates interfaces.web.route {:?} already registered by app {:?}; app not registered",
                            manifest_path.display(),
                            manifest.app.id,
                            web.route,
                            existing.manifest.app.id
                        ));
                        continue;
                    }
                    if let Some(socket) = &web.socket {
                        if let Some(existing) = discovery.apps.iter().find(|app| {
                            app.manifest
                                .interfaces
                                .web
                                .as_ref()
                                .and_then(|existing| existing.socket.as_deref())
                                == Some(socket.as_str())
                        }) {
                            discovery.errors.push(format!(
                                "{}: app {:?} duplicates interfaces.web.socket {:?} already registered by app {:?}; app not registered",
                                manifest_path.display(),
                                manifest.app.id,
                                socket,
                                existing.manifest.app.id
                            ));
                            continue;
                        }
                    }
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
        assert!(discovery.errors[0].contains("b-second"));
        assert!(discovery.errors[0].contains("interfaces.web.route"));
        assert!(discovery.errors[0].contains("/same"));
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
        assert!(discovery.errors[0].contains("b-second"));
        assert!(discovery.errors[0].contains("interfaces.web.socket"));
        assert!(discovery.errors[0].contains("/ws/shared"));
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
