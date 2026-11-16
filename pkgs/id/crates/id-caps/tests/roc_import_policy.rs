//! The `#5994` §4 compile-fixture matrix for the Roc library import
//! gate (`#5956` §7 step 4, `#10027`), executed against the real
//! pipeline: manifest parse (step 1) → transitive resolution (step 2)
//! → the import gate (step 3).
//!
//! Cases live in `pkgs/id/examples/roc-import-policy/fixtures.json`;
//! each pins its diagnostic — the subreason and the exact excess set —
//! as its oracle. A refusal for any other reason is a failing fixture,
//! per the §4 contract. The two Roc-compile negative fixtures of the
//! matrix (missing module, wrong signature) are the `roc-cap-demo`
//! tests, already wired through `roc-test.sh`; this suite covers the
//! gate-level rows.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;

use id_caps::library_gate::{GateInputs, GateOutcome, check_import};
use id_caps::library_manifest::{BuildMode, LibraryKind, LibraryManifest, PlatformRequirement};
use id_caps::library_resolution::{
    CheckedSurface, ImportRequest, ImportSource, ManifestRegistry, RegistryEntry, resolve_import,
};
use id_caps::world_caps::PROVIDER_REGISTRY;
use serde_json::Value;

struct TestRegistry<'m> {
    entries: HashMap<String, RegistryEntry<'m>>,
}

impl ManifestRegistry for TestRegistry<'_> {
    fn entry(&self, manifest_hash: &str) -> Option<RegistryEntry<'_>> {
        self.entries.get(manifest_hash).map(|e| RegistryEntry {
            manifest: e.manifest,
            dependencies: e.dependencies,
        })
    }
}

fn str_set(value: &Value) -> BTreeSet<String> {
    value
        .as_array()
        .expect("a name set is an array")
        .iter()
        .map(|v| v.as_str().expect("names are strings").to_string())
        .collect()
}

fn manifest_from_record(record: &Value) -> LibraryManifest {
    let kind = match record["kind"].as_str().expect("kind") {
        "source-library" => LibraryKind::SourceLibrary,
        other => panic!("fixture uses unhandled kind {other}"),
    };
    let build_mode = match record["build_mode"].as_str().expect("build_mode") {
        "compile-in" => BuildMode::CompileIn,
        other => panic!("fixture uses unhandled build-mode {other}"),
    };
    LibraryManifest {
        name: record["name"].as_str().expect("name").to_string(),
        version: record["version"].as_str().expect("version").to_string(),
        kind,
        build_mode,
        hash: record["hash"].as_str().expect("hash").to_string(),
        license: record["license"].as_str().expect("license").to_string(),
        requirements: record["requirements"]
            .as_array()
            .expect("requirements")
            .iter()
            .map(|r| PlatformRequirement {
                name: r["name"].as_str().expect("requirement name").to_string(),
                version_pin: r["version_pin"].as_u64().map(|v| v as u32),
            })
            .collect(),
        subtypes: Vec::new(),
        attachments: Vec::new(),
        source_url: None,
    }
}

#[test]
fn roc_import_policy_fixture_matrix() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/roc-import-policy/fixtures.json");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("fixtures at {}: {e}", path.display()));
    let doc: Value = serde_json::from_str(&text).expect("fixtures are valid JSON");
    let standard_surface = str_set(&doc["standard_surface"]);
    let catalog_version = doc["catalog_version"].as_str().expect("catalog_version");

    let cases = doc["cases"].as_array().expect("cases");
    assert!(cases.len() >= 10, "the matrix covers its rows");
    for case in cases {
        let name = case["case"].as_str().expect("case name");

        // Manifests: KDL text goes through the step-1 parser; a
        // `record` manifest is built directly, standing for a manifest
        // parsed against a different catalog vintage (parse-time
        // validation is catalog-relative; the gate re-checks against
        // the compiling catalog, #5956 §3).
        let mut manifests: Vec<LibraryManifest> = Vec::new();
        let mut deps: Vec<Vec<String>> = Vec::new();
        for m in case["manifests"].as_array().expect("manifests") {
            if let Some(kdl) = m.get("kdl") {
                manifests.push(
                    LibraryManifest::parse(kdl.as_str().expect("kdl"))
                        .unwrap_or_else(|e| panic!("case {name}: manifest parses: {e}")),
                );
            } else {
                manifests.push(manifest_from_record(&m["record"]));
            }
            deps.push(
                m["depends_on"]
                    .as_array()
                    .expect("depends_on")
                    .iter()
                    .map(|d| d.as_str().expect("dep hash").to_string())
                    .collect(),
            );
        }
        let mut entries: HashMap<String, RegistryEntry<'_>> = HashMap::new();
        for (manifest, dep) in manifests.iter().zip(deps.iter()) {
            entries.insert(
                manifest.hash.clone(),
                RegistryEntry {
                    manifest,
                    dependencies: dep,
                },
            );
        }
        let registry = TestRegistry { entries };

        let import = &case["import"];
        let source = match &import["source"] {
            Value::String(s) if s == "registry" => ImportSource::Registry,
            Value::Object(o) => ImportSource::Url {
                url: o["url"].as_str().expect("url").to_string(),
            },
            other => panic!("case {name}: unhandled source {other}"),
        };
        let request = ImportRequest {
            importing_world: import["world"].as_str().expect("world").to_string(),
            library_name: import["name"].as_str().expect("name").to_string(),
            library_version: import["version"].as_str().expect("version").to_string(),
            manifest_hash: import["hash"].as_str().expect("hash").to_string(),
            source,
            url_bytes_verified: import["url_bytes_verified"]
                .as_bool()
                .expect("url_bytes_verified"),
        };
        let platform = &case["platform"];
        let surface = if platform.get("surface_from").is_some() {
            standard_surface.clone()
        } else {
            str_set(&platform["surface"])
        };
        let record = resolve_import(
            &registry,
            &request,
            Some(CheckedSurface {
                platform: platform["identity"].as_str().expect("identity").to_string(),
                catalog_version: catalog_version.to_string(),
            }),
        );
        let verdict = check_import(
            &record,
            &PROVIDER_REGISTRY,
            &GateInputs {
                platform_surface: surface,
                world_requested: str_set(&case["world_requested"]),
                discovered_used: str_set(&case["discovered_used"]),
                require_checked: case["require_checked"].as_bool().expect("require_checked"),
            },
        );

        let expected = &case["expected"];
        match expected["outcome"].as_str().expect("expected outcome") {
            "accepted" => assert_eq!(
                verdict.outcome,
                GateOutcome::Accepted,
                "case {name} ({})",
                case["matrix"].as_str().unwrap_or("")
            ),
            "refused" => {
                let GateOutcome::Refused(failures) = &verdict.outcome else {
                    panic!("case {name}: expected a refusal, got {:?}", verdict.outcome);
                };
                let got: Vec<(String, Vec<String>)> = failures
                    .iter()
                    .map(|f| {
                        (
                            f.subreason.as_str().to_string(),
                            f.excess.iter().map(|e| e.name.clone()).collect(),
                        )
                    })
                    .collect();
                let want: Vec<(String, Vec<String>)> = expected["failures"]
                    .as_array()
                    .expect("expected failures")
                    .iter()
                    .map(|f| {
                        (
                            f["subreason"].as_str().expect("subreason").to_string(),
                            f["excess"]
                                .as_array()
                                .expect("excess")
                                .iter()
                                .map(|n| n.as_str().expect("excess name").to_string())
                                .collect(),
                        )
                    })
                    .collect();
                assert_eq!(got, want, "case {name}: diagnostic is the oracle");
                assert!(
                    verdict.diagnostic.is_some(),
                    "case {name}: a refusal carries its diagnostic"
                );
            }
            other => panic!("case {name}: unknown expected outcome {other}"),
        }
    }
}
