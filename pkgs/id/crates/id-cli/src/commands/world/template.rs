//! `id world template-export` and `template-create`: take a template
//! from a durable world on disk, and make a new world from one (#8282).
//! The projection, the strip rules and the import checks live in
//! `id_worlds::world_template`, and the world creation in
//! `id_worlds::world_store`; this module reads and writes the files and
//! reports the result.

use std::io::Write as _;
use std::path::Path;

use anyhow::{Context, Result};

/// Export the world in `dir` as a template bundle written to `out`.
///
/// `out` must not exist, so an export never overwrites a file. Only the
/// `kv` keys in `keep_state` are carried as template state.
pub(super) fn export_world_template(
    dir: &Path,
    out: &Path,
    keep_state: &[String],
    json: bool,
) -> Result<()> {
    let source = id_worlds::world_store::template_source_from_dir(dir, keep_state)?;
    let bundle = id_worlds::world_template::export_template(&source)?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(out)
        .with_context(|| format!("create {}", out.display()))?;
    let mut text = serde_json::to_vec_pretty(&bundle)?;
    text.push(b'\n');
    file.write_all(&text)?;
    file.sync_all()?;

    let manifest = &bundle.manifest;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "out": out.display().to_string(),
                "programs": manifest.programs,
                "state_keys": manifest.state.keys().collect::<Vec<_>>(),
                "retained": manifest.retained,
                "stripped": manifest.stripped,
            }))?
        );
    } else {
        println!("exported {}", out.display());
        for program in &manifest.programs {
            println!(
                "program: {}/{}@{} {}",
                program.author, program.name, program.version, program.content_hash
            );
        }
        for key in manifest.state.keys() {
            println!("state: {key}");
        }
        let stripped = if manifest.stripped.is_empty() {
            "nothing".to_owned()
        } else {
            manifest.stripped.join(", ")
        };
        println!("stripped: {stripped}");
    }
    Ok(())
}

/// Make a new world in `dir` from the template file at `template`.
///
/// Every `--bundle` file is decoded and verified before anything is
/// written; the template's references resolve against them, and the
/// world store installs the held ones and records the rest as gaps.
#[cfg(feature = "sandbox")]
pub(super) async fn create_world_from_template_file(
    template: &Path,
    dir: &Path,
    name: Option<&str>,
    bundles: &[std::path::PathBuf],
    trust_keys: &[String],
    seed: u64,
    json: bool,
) -> Result<()> {
    let world_id = match name {
        Some(name) => name.to_owned(),
        None => dir
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::to_owned)
            .context("the destination directory has no name; pass --name")?,
    };
    id_worlds::world_hub::validate_world_name(&world_id)?;
    let bytes = std::fs::read(template).with_context(|| format!("read {}", template.display()))?;
    let template_bundle: id_worlds::world_template::TemplateBundle = serde_json::from_slice(&bytes)
        .with_context(|| format!("{} is not a template file", template.display()))?;
    let mut held = Vec::new();
    for path in bundles {
        let bytes = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
        let bundle = id_worlds::program_bundle::ProgramBundle::decode(&bytes)
            .with_context(|| format!("{} is not a valid program bundle", path.display()))?;
        held.push(bundle);
    }
    let mut trusted = Vec::new();
    for key in trust_keys {
        let bytes = id_worlds::world_session::decode_hex(key)
            .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
            .with_context(|| format!("trust key {key:?} is not 64 hex characters"))?;
        trusted.push(bytes);
    }
    let import = id_worlds::world_template::import_template(&template_bundle, &world_id, &held)?;
    let world =
        id_worlds::world_store::create_world_from_template(dir, &import, trusted, seed).await?;

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "world_id": world.world_id,
                "dir": dir.display().to_string(),
                "installed": world.installed,
                "gaps": world.gaps,
                "state_keys": world.state_keys,
            }))?
        );
    } else {
        println!("created {} as {}", dir.display(), world.world_id);
        for hash in &world.installed {
            println!("installed: {hash}");
        }
        for gap in &world.gaps {
            println!(
                "gap: {}/{}@{} {} (no bundle given; recorded in {})",
                gap.author,
                gap.name,
                gap.version,
                gap.content_hash,
                id_worlds::world_store::TEMPLATE_GAPS_FILE
            );
        }
        for key in &world.state_keys {
            println!("state: {key}");
        }
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use id_worlds::world::{JOURNAL_VERSION, JournalEntry};
    use id_worlds::world_template::TemplateBundle;

    fn world_dir() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let mut bytes = Vec::new();
        for entry in [
            JournalEntry::Created {
                world_id: "plaza".to_owned(),
                version: JOURNAL_VERSION,
            },
            JournalEntry::PortalGrantSet {
                peer: "north".to_owned(),
                granted: true,
            },
            JournalEntry::KvSet {
                key: "layout".to_owned(),
                value: "1".to_owned(),
            },
            JournalEntry::KvSet {
                key: "scratch".to_owned(),
                value: "2".to_owned(),
            },
        ] {
            bytes.extend(serde_json::to_vec(&entry).unwrap());
            bytes.push(b'\n');
        }
        std::fs::write(dir.path().join("journal.jsonl"), bytes).unwrap();
        dir
    }

    #[test]
    fn export_writes_the_kept_state_and_counts_the_stripped_grants() {
        let dir = world_dir();
        let out = dir.path().join("plaza.template.json");
        export_world_template(dir.path(), &out, &["layout".to_owned()], true).unwrap();

        let bundle: TemplateBundle = serde_json::from_slice(&std::fs::read(&out).unwrap()).unwrap();
        assert_eq!(bundle.manifest.state.keys().collect::<Vec<_>>(), ["layout"]);
        assert_eq!(bundle.manifest.stripped, ["grants:1"]);
        let text = std::fs::read_to_string(&out).unwrap();
        assert!(!text.contains("north") && !text.contains("scratch"));
    }

    #[test]
    fn export_never_overwrites_and_writes_nothing_on_refusal() {
        let dir = world_dir();
        let out = dir.path().join("taken.json");
        std::fs::write(&out, b"mine").unwrap();
        assert!(export_world_template(dir.path(), &out, &[], false).is_err());
        assert_eq!(std::fs::read(&out).unwrap(), b"mine");

        let refused = dir.path().join("refused.json");
        assert!(
            export_world_template(dir.path(), &refused, &["absent".to_owned()], false).is_err()
        );
        assert!(!refused.exists());
    }

    #[cfg(feature = "sandbox")]
    #[tokio::test]
    async fn a_template_file_and_its_bundle_make_a_world_that_exports_the_same_template() {
        use ed25519_dalek::SigningKey;
        use id_worlds::program_bundle::{ProgramSpec, export};
        use id_worlds::world_template::{TemplateSource, export_template};

        let author = SigningKey::from_bytes(&[61; 32]);
        let wasm = include_bytes!("../../../../../examples/roc-counter/counter.wasm").to_vec();
        let bundle = export(
            wasm,
            ProgramSpec {
                name: "counter",
                version: 1,
                licence: None,
                description: None,
                categories: &[],
            },
            &author,
        )
        .unwrap();
        let source = TemplateSource {
            programs: vec![bundle.identity()],
            roles: Vec::new(),
            credentials: Vec::new(),
            per_user_data: Vec::new(),
            grants: Vec::new(),
            state: [("layout".to_owned(), "1".to_owned())].into(),
        };
        let files = tempfile::tempdir().unwrap();
        let template = files.path().join("counter.template.json");
        let bundle_file = files.path().join("counter.bundle");
        std::fs::write(
            &template,
            serde_json::to_vec(&export_template(&source).unwrap()).unwrap(),
        )
        .unwrap();
        std::fs::write(&bundle_file, bundle.to_bytes()).unwrap();

        let key: String = author
            .verifying_key()
            .to_bytes()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        let world = files.path().join("counter-copy");
        create_world_from_template_file(
            &template,
            &world,
            None,
            std::slice::from_ref(&bundle_file),
            &[key],
            1,
            true,
        )
        .await
        .unwrap();

        let out = files.path().join("again.json");
        export_world_template(&world, &out, &["layout".to_owned()], true).unwrap();
        let again: TemplateBundle = serde_json::from_slice(&std::fs::read(&out).unwrap()).unwrap();
        assert_eq!(again.manifest.programs, source.programs);
        assert_eq!(again.manifest.state, source.state);
    }

    #[cfg(feature = "sandbox")]
    #[tokio::test]
    async fn a_malformed_trust_key_or_world_name_writes_nothing() {
        let dir = world_dir();
        let template = dir.path().join("plaza.template.json");
        export_world_template(dir.path(), &template, &[], false).unwrap();
        let target = dir.path().join("copy");
        for (name, keys) in [
            (None, vec!["zz".to_owned()]),
            (Some("Not A Name"), Vec::new()),
        ] {
            assert!(
                create_world_from_template_file(&template, &target, name, &[], &keys, 1, false)
                    .await
                    .is_err()
            );
            assert!(!target.exists());
        }
    }
}
