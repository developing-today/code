//! `id gallery`: administer the gallery directory that
//! `id serve --web --gallery-dir` serves (#10005; v3 design G2, G6).

use std::path::PathBuf;

use clap::{Args, Subcommand};
use id_worlds::program_gallery_admission::{AdmissionPosture, ReasonClass};

/// Arguments for `id gallery`.
#[derive(Args, Debug)]
pub struct GalleryArgs {
    /// The gallery directory: the one `id serve --gallery-dir` serves. A
    /// running server reads it when it starts, so restart it to serve a
    /// change.
    #[arg(long, env = "ID_GALLERY_DIR")]
    pub dir: PathBuf,
    /// What to do.
    #[command(subcommand)]
    pub command: GalleryCommand,
}

/// Subcommands for `id gallery`.
#[derive(Subcommand, Debug)]
pub enum GalleryCommand {
    /// Stop listing a record here and write the removal to the public
    /// transparency list. The bundle file is kept, and the record stays
    /// valid on other hosts.
    Remove {
        /// The record's content hash.
        content_hash: String,
        /// Reason class: `illegal_content`, `malware`, `spam` or `policy`.
        #[arg(long)]
        reason: ReasonClass,
        /// A short public note for the transparency list.
        #[arg(long)]
        note: Option<String>,
    },
    /// Refuse a record waiting for review, publicly.
    Refuse {
        /// The record's content hash.
        content_hash: String,
        /// Reason class: `illegal_content`, `malware`, `spam` or `policy`.
        #[arg(long)]
        reason: ReasonClass,
        /// A short public note for the transparency list.
        #[arg(long)]
        note: Option<String>,
    },
    /// Reverse a removal or refusal (an appeal upheld) and list the record
    /// again; the reinstatement goes on the transparency list.
    Reinstate {
        /// The record's content hash.
        content_hash: String,
        /// A short public note for the transparency list.
        #[arg(long)]
        note: Option<String>,
    },
    /// List a record waiting for review.
    Approve {
        /// The record's content hash.
        content_hash: String,
    },
    /// Show the records waiting for review.
    Pending,
    /// Show the transparency list.
    Transparency,
    /// Set the admission posture: `open` lists every verified record until
    /// it is removed, `reviewed` lists a record once it is approved.
    Posture {
        /// open or reviewed.
        posture: AdmissionPosture,
    },
    /// State, or with no hours withdraw, the response time published for
    /// one reason class.
    ResponseHours {
        /// Reason class: `illegal_content`, `malware`, `spam` or `policy`.
        reason: ReasonClass,
        /// Hours; leave out to withdraw the statement.
        hours: Option<u32>,
    },
    /// Set the operator's own tags for a program or an author (none clears
    /// them). Tags are operator speech, served apart from signed records.
    Tag {
        /// A program's content hash.
        #[arg(long, required_unless_present = "author", conflicts_with = "author")]
        program: Option<String>,
        /// An author's public key.
        #[arg(long)]
        author: Option<String>,
        /// The tags, in the portable form (lower-case letters, digits, `-`).
        tags: Vec<String>,
    },
}
