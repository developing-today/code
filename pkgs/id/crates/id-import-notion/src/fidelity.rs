//! Persisted fidelity report generation (slice 5, #13525, design #6436
//! §6.4).
//!
//! Every import ends with a user-visible report, persisted with the
//! manifest, containing at minimum: objects by outcome (clean /
//! degraded / failed / skipped) with counts and per-item reasons for
//! every non-clean item; every §2.5 degradation class with counts;
//! externalised links and relations; missing resources; comments
//! coverage; access-metadata observations; re-import moves; and the
//! acquisition degradations of §1.3. The report's invariant is that
//! **every source object in the inventory is accounted for exactly
//! once** — [`generate_report`] fails rather than producing a report
//! that does not reconcile, and a report whose counts do not reconcile
//! is a failed import, whatever the logs say.

use std::collections::BTreeMap;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::{
    Manifest, Outcome, Stage,
    content::{Externalised, MissingFile, ReferenceSurface},
    rows::{ExternalisedReason, RelationType},
};

/// One inventory object whose outcome is not clean (§6.4: every
/// non-clean item carries its reason).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct NonCleanItem {
    pub object_id: String,
    pub kind: String,
    pub stage: Stage,
    pub outcome: Outcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// One link or mention kept verbatim because its target is outside the
/// import set (§3), with the page that carries it.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExternalisedLinkItem {
    pub page_id: String,
    pub from_block_id: String,
    pub target_id: String,
    pub surface: ReferenceSurface,
}

/// One relation value recorded as provenance instead of a typed
/// reference (§2.2), with the row page that carries it.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExternalisedRelationItem {
    pub row_page_id: String,
    pub database_id: String,
    pub data_source_id: String,
    pub property_name: String,
    pub reference_type: RelationType,
    pub target_page_id: String,
    pub reason: ExternalisedReason,
}

/// One file that could not be fetched during the run (§4): the page is
/// degraded, never aborted, and the reason never contains the expiring
/// file URL. Converter reasons are stripped of URLs at the source;
/// [`generate_report`] re-checks here as a second gate.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MissingFileItem {
    pub page_id: String,
    pub block_id: String,
    pub reason: String,
}

/// The persisted fidelity report (design §6.4). Every collection is
/// sorted (`BTreeMap`, ordered pushes), so two runs over an unchanged
/// corpus produce identical reports (§8 check 3).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct FidelityReport {
    pub workspace_id: String,
    pub notion_version: String,
    /// Every inventoried object is accounted for exactly once:
    /// `clean + degraded + failed + skipped == total_objects`.
    pub total_objects: usize,
    pub clean: usize,
    pub degraded: usize,
    pub failed: usize,
    pub skipped: usize,
    pub non_clean: Vec<NonCleanItem>,
    /// Degraded block type -> blocks of that type (§2.5 accounting).
    pub degradation_classes: BTreeMap<String, usize>,
    /// `<block type>.<attribute>` -> how many times it was dropped.
    pub degradation_attributes: BTreeMap<String, usize>,
    /// Degraded block type -> verbatim fallbacks (§2.5 safety net).
    pub verbatim_fallbacks: BTreeMap<String, usize>,
    /// Synced-block duplicates whose source is outside the import set.
    pub synced_externalised: usize,
    pub externalised_links: Vec<ExternalisedLinkItem>,
    pub externalised_relations: Vec<ExternalisedRelationItem>,
    pub missing_files: Vec<MissingFileItem>,
    /// Pages whose comments converted (possibly to zero claims).
    pub comments_covered: Vec<String>,
    /// Pages whose comments the acquisition path could not see (§2.4).
    pub comments_unavailable: Vec<String>,
    /// Pages with no comments conversion recorded yet.
    pub comments_not_converted: Vec<String>,
    /// Total comment claims across covered pages.
    pub comment_claims: usize,
    /// Export paths the report must name (§1.3): every match that is
    /// not UUID-exact, and every title conflict.
    pub export_uncertain_matches: Vec<String>,
    /// Pages carrying degraded export-only identity (§1.3, §2.5).
    pub export_only_identities: Vec<String>,
    pub api_acquisition: bool,
    pub export_acquisition: bool,
    pub degraded_mode: bool,
    /// Top-level shares visible to the integration (§7 provenance).
    pub visible_shares: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_edition: Option<String>,
    /// Data-source moves observed during re-import (§6.1).
    pub moved_data_sources: Vec<crate::reimport::MovedDataSource>,
}

impl FidelityReport {
    /// The §6.4 invariant: every source object accounted for exactly
    /// once.
    pub fn reconciles(&self) -> bool {
        self.clean + self.degraded + self.failed + self.skipped == self.total_objects
    }
}

/// Generate the §6.4 fidelity report from a manifest. Fails when the
/// manifest's progress does not reconcile to the inventory (a report
/// that does not reconcile is a failed import), and when any persisted
/// reason leaks a Notion file URL.
pub fn generate_report(manifest: &Manifest) -> Result<FidelityReport> {
    manifest.validate_inventory_reconciles()?;

    let mut report = FidelityReport {
        workspace_id: manifest.workspace_id.clone(),
        notion_version: manifest.notion_version.clone(),
        total_objects: manifest.inventory.objects.len(),
        api_acquisition: manifest.acquisition.api,
        export_acquisition: manifest.acquisition.export,
        degraded_mode: manifest.acquisition.degraded_mode,
        visible_shares: manifest.inventory.visible_shares.clone(),
        workspace_edition: manifest.inventory.workspace_edition.clone(),
        moved_data_sources: manifest.moved_data_sources.clone(),
        ..FidelityReport::default()
    };

    // Outcomes: one row per inventory object, non-clean rows named.
    let mut non_clean: Vec<NonCleanItem> = Vec::new();
    for (id, object) in manifest.inventory.objects.iter() {
        let progress = manifest
            .objects
            .get(id)
            .with_context(|| format!("object {id} is not in the manifest inventory"))?;
        match progress.outcome {
            Outcome::Clean => report.clean += 1,
            Outcome::Degraded => report.degraded += 1,
            Outcome::Failed => report.failed += 1,
            Outcome::Skipped => report.skipped += 1,
        }
        if progress.outcome != Outcome::Clean {
            non_clean.push(NonCleanItem {
                object_id: id.clone(),
                kind: object.kind.clone(),
                stage: progress.stage.clone(),
                outcome: progress.outcome.clone(),
                reason: progress.reason.clone(),
            });
        }
    }
    non_clean.sort_by(|a, b| a.object_id.cmp(&b.object_id));
    report.non_clean = non_clean;

    // Degradation ledgers (§2.5): summed per class across pages.
    for losses in manifest.page_losses.values() {
        bump_all(&mut report.degradation_classes, &losses.per_class);
        bump_all(&mut report.degradation_attributes, &losses.per_attribute);
        bump_all(&mut report.verbatim_fallbacks, &losses.verbatim_fallback);
        report.synced_externalised += losses.synced_externalised;
    }

    // Externalised links and missing files (§3, §4), per page.
    for (page_id, items) in manifest.page_externalised.iter() {
        for item in items {
            let item: &Externalised = item;
            report.externalised_links.push(ExternalisedLinkItem {
                page_id: page_id.clone(),
                from_block_id: item.from_block_id.clone(),
                target_id: item.target_id.clone(),
                surface: item.surface,
            });
        }
    }
    report
        .externalised_links
        .sort_by(|a, b| (&a.page_id, &a.from_block_id).cmp(&(&b.page_id, &b.from_block_id)));
    for (page_id, files) in manifest.page_missing_files.iter() {
        for file in files {
            let file: &MissingFile = file;
            if file.reason.contains("notion.so") || file.reason.contains("expiry_time") {
                bail!(
                    "missing-file reason for block {} leaks a file URL",
                    file.block_id
                );
            }
            report.missing_files.push(MissingFileItem {
                page_id: page_id.clone(),
                block_id: file.block_id.clone(),
                reason: file.reason.clone(),
            });
        }
    }
    report
        .missing_files
        .sort_by(|a, b| (&a.page_id, &a.block_id).cmp(&(&b.page_id, &b.block_id)));

    // Externalised relations (§2.2), per row page.
    for record in manifest.row_records.values() {
        for relation in &record.externalised {
            report
                .externalised_relations
                .push(ExternalisedRelationItem {
                    row_page_id: record.page_id.clone(),
                    database_id: record.database_id.clone(),
                    data_source_id: record.data_source_id.clone(),
                    property_name: relation.property_name.clone(),
                    reference_type: relation.reference_type.clone(),
                    target_page_id: relation.target_page_id.clone(),
                    reason: relation.reason,
                });
        }
    }
    report.externalised_relations.sort_by(|a, b| {
        (&a.row_page_id, &a.property_name).cmp(&(&b.row_page_id, &b.property_name))
    });

    // Comments coverage (§2.4).
    for object in manifest.inventory.objects.values() {
        if object.kind != "page" {
            continue;
        }
        match manifest.comments_coverage.get(&object.id) {
            Some(crate::comments::CommentsCoverage::Covered) => {
                report.comments_covered.push(object.id.clone());
            }
            Some(crate::comments::CommentsCoverage::Unavailable) => {
                report.comments_unavailable.push(object.id.clone());
            }
            None => report.comments_not_converted.push(object.id.clone()),
        }
    }
    report.comments_covered.sort();
    report.comments_unavailable.sort();
    report.comments_not_converted.sort();
    report.comment_claims = manifest.comment_claims.values().map(Vec::len).sum();

    // Acquisition degradations (§1.3): every uncertain export match is
    // named, and export-only identity is headlined.
    let mut uncertain: Vec<String> = manifest
        .export_matches
        .iter()
        .filter(|(_, entry)| entry.needs_naming())
        .map(|(path, _)| path.clone())
        .collect();
    uncertain.sort();
    report.export_uncertain_matches = uncertain;
    let mut degraded: Vec<String> = manifest.degraded_identities.keys().cloned().collect();
    degraded.sort();
    report.export_only_identities = degraded;

    if !report.reconciles() {
        bail!(
            "fidelity report does not reconcile to inventory: {} accounted of {} objects",
            report.clean + report.degraded + report.failed + report.skipped,
            report.total_objects
        );
    }
    Ok(report)
}

fn bump_all(into: &mut BTreeMap<String, usize>, from: &BTreeMap<String, usize>) {
    for (key, count) in from {
        *into.entry(key.clone()).or_default() += count;
    }
}
