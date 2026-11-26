//! `id world template-export`: take a template from a durable world on
//! disk (#8282). The projection and the strip rules live in
//! `id_worlds::world_template`; this module reads the world, writes
//! the bundle and reports what was kept and what was stripped.

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
}
