//! App-folder manifests and host-side discovery.

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
        manifest.interfaces.cli.is_some() || manifest.interfaces.web.is_some(),
        "at least one [interfaces.cli] or [interfaces.web] section is required"
    );
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
            ensure!(
                !hash.trim().is_empty(),
                "program.module_hash must not be empty"
            );
        }
    }
    if let Some(web) = &manifest.interfaces.web {
        ensure!(
            matches!(web.mode.as_str(), "adapter" | "direct"),
            "interfaces.web.mode must be adapter or direct"
        );
        ensure!(
            web.route.starts_with('/'),
            "interfaces.web.route must start with /"
        );
    }
    Ok(manifest)
}

/// Scan immediate child folders for `app.toml`, isolating errors to each app.
pub fn discover_apps(root: impl AsRef<Path>) -> AppDiscovery {
    let root = root.as_ref();
    let mut discovery = AppDiscovery::default();
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return discovery,
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
        let folder_name = folder
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        let parsed = fs::read_to_string(&manifest_path)
            .with_context(|| format!("read {}", manifest_path.display()))
            .and_then(|source| parse_app_manifest(&source, folder_name));
        match parsed {
            Ok(manifest) => discovery.apps.push(DiscoveredApp {
                path: folder,
                manifest,
            }),
            Err(error) => discovery
                .errors
                .push(format!("{}: {error:#}", manifest_path.display())),
        }
    }
    discovery
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
    fn discovery_reports_bad_app_and_keeps_valid_app() {
        let root = tempfile::tempdir().unwrap();
        let valid = root.path().join("tic-tac-toe");
        let invalid = root.path().join("mismatched");
        fs::create_dir_all(&valid).unwrap();
        fs::create_dir_all(&invalid).unwrap();
        fs::write(valid.join("app.toml"), MANIFEST).unwrap();
        fs::write(invalid.join("app.toml"), MANIFEST).unwrap();

        let discovery = discover_apps(root.path());
        assert_eq!(discovery.apps.len(), 1);
        assert_eq!(discovery.apps[0].manifest.app.id, "tic-tac-toe");
        assert_eq!(discovery.errors.len(), 1);
        assert!(discovery.errors[0].contains("does not match folder name"));
    }
}
