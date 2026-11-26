//! `id gallery`: offline administration of a gallery directory (#10005).

use anyhow::{Context as _, bail};
use serde::Serialize;

use id_worlds::program_gallery::Gallery;
use id_worlds::program_gallery_admission::AdmissionConfig;
use id_worlds::program_registry::AdmitAll;

use crate::cli::{GalleryArgs, GalleryCommand};

/// Run a gallery administration command.
///
/// # Errors
///
/// The directory is missing or does not load, or the gallery refuses the
/// step.
pub fn cmd_gallery(args: GalleryArgs) -> anyhow::Result<()> {
    if !args.dir.is_dir() {
        bail!("{} is not a gallery directory", args.dir.display());
    }
    let mut gallery = Gallery::open_dir(&args.dir, AdmitAll)
        .with_context(|| format!("opening the gallery in {}", args.dir.display()))?;
    match args.command {
        GalleryCommand::Remove {
            content_hash,
            reason,
            note,
        } => print(&gallery.remove(&content_hash, reason, now(), note)?),
        GalleryCommand::Refuse {
            content_hash,
            reason,
            note,
        } => print(&gallery.refuse(&content_hash, reason, now(), note)?),
        GalleryCommand::Reinstate { content_hash, note } => {
            print(&gallery.reinstate(&content_hash, now(), note)?)
        }
        GalleryCommand::Approve { content_hash } => print(&gallery.approve(&content_hash)?),
        GalleryCommand::Pending => print(&gallery.pending()),
        GalleryCommand::Transparency => print(gallery.admission().list().entries()),
        GalleryCommand::Posture { posture } => {
            let config = AdmissionConfig {
                posture,
                ..gallery.admission().config().clone()
            };
            gallery.set_admission_config(config)?;
            print(gallery.admission().config())
        }
        GalleryCommand::ResponseHours { reason, hours } => {
            let mut config = gallery.admission().config().clone();
            match hours {
                Some(hours) => config.response_hours.insert(reason, hours),
                None => config.response_hours.remove(&reason),
            };
            gallery.set_admission_config(config)?;
            print(gallery.admission().config())
        }
        GalleryCommand::Tag {
            program,
            author,
            tags,
        } => {
            let mut overlay = gallery.overlay().clone();
            let (map, key) = match (program, author) {
                (Some(program), _) => (&mut overlay.programs, program),
                (None, Some(author)) => (&mut overlay.authors, author),
                (None, None) => bail!("name a --program or an --author"),
            };
            if tags.is_empty() {
                map.remove(&key);
            } else {
                map.insert(key, tags);
            }
            gallery.set_overlay(overlay)?;
            print(gallery.overlay())
        }
    }
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

fn print(value: &(impl Serialize + ?Sized)) -> anyhow::Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use id_worlds::program_bundle::{ProgramSpec, export};
    use id_worlds::program_gallery_admission::{AdmissionPosture, ReasonClass};
    use id_worlds::program_registry::ListQuery;

    use super::*;

    fn run(dir: &std::path::Path, command: GalleryCommand) -> anyhow::Result<()> {
        cmd_gallery(GalleryArgs {
            dir: dir.to_path_buf(),
            command,
        })
    }

    #[test]
    fn removal_review_and_tags_are_stored_for_the_server() {
        let dir = tempfile::tempdir().unwrap();
        let chess = export(
            b"chess".to_vec(),
            ProgramSpec {
                name: "chess",
                version: 1,
                licence: None,
                description: Some("Board game."),
                categories: &["board"],
            },
            &ed25519_dalek::SigningKey::from_bytes(&[3; 32]),
        )
        .unwrap();
        let hash = chess.manifest().content_hash.clone();
        Gallery::open_dir(dir.path(), AdmitAll)
            .unwrap()
            .publish(&chess.to_bytes())
            .unwrap();

        run(
            dir.path(),
            GalleryCommand::Remove {
                content_hash: hash.clone(),
                reason: ReasonClass::Spam,
                note: None,
            },
        )
        .unwrap();
        let served = Gallery::open_dir(dir.path(), AdmitAll).unwrap();
        assert_eq!(
            served.list(&ListQuery::default()),
            [] as [id_worlds::program_bundle::BundleRecord; 0]
        );
        assert_eq!(served.admission().list().len(), 1);

        run(
            dir.path(),
            GalleryCommand::Reinstate {
                content_hash: hash.clone(),
                note: Some("mistake".to_owned()),
            },
        )
        .unwrap();
        run(
            dir.path(),
            GalleryCommand::Posture {
                posture: AdmissionPosture::Reviewed,
            },
        )
        .unwrap();
        run(
            dir.path(),
            GalleryCommand::ResponseHours {
                reason: ReasonClass::Malware,
                hours: Some(4),
            },
        )
        .unwrap();
        run(
            dir.path(),
            GalleryCommand::Tag {
                program: Some(hash.clone()),
                author: None,
                tags: vec!["classic".to_owned()],
            },
        )
        .unwrap();

        let served = Gallery::open_dir(dir.path(), AdmitAll).unwrap();
        assert_eq!(
            served.list(&ListQuery::default()),
            vec![chess.record()],
            "reinstated records stay listed under review"
        );
        assert_eq!(
            served.admission().config().posture,
            AdmissionPosture::Reviewed
        );
        assert_eq!(
            served.admission().config().response_hours[&ReasonClass::Malware],
            4
        );
        assert_eq!(served.overlay().programs[&hash], vec!["classic".to_owned()]);
        assert!(
            run(dir.path(), GalleryCommand::Approve { content_hash: hash }).is_err(),
            "it is not pending"
        );
        assert!(run(&dir.path().join("missing"), GalleryCommand::Transparency).is_err());
    }
}
