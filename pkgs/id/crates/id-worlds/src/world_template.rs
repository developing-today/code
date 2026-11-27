//! Template-world snapshot export: manifest + strip rules.
//!
//! First slice of issue #8282 (design #6278, decision D5): a world
//! snapshot as a starting point. A template carries world state (the
//! template-scoped part), installed programs as bundle references
//! (identity + content hash, never re-hosted binaries), and role
//! *definitions* — while member lists, credentials/secrets, per-user
//! data and grants are stripped by default and the stripping is
//! recorded in the manifest.
//!
//! The governing invariant (D5) is that a template never carries
//! another user's identity material. It is stated as property tests
//! below: [`export_template`] output contains no member, credential,
//! per-user-datum or grant material for any source, and the manifest
//! accounts for every stripped item.
//!
//! The first slice covered the manifest format, the strip rules and
//! the export half. The second adds [`import_template`]: a fresh world
//! identity, roles recreated empty, template-scoped state as the
//! starting state, and each program reference resolved to a verified
//! [`ProgramBundle`] the importer holds, ready for the ordinary D4
//! install path ([`crate::world_session`]'s `install_bundle`). A
//! reference the importer does not hold is a named, resumable gap
//! (v1 record §5), never a silent skip and never a network fetch.
//!
//! The third adds [`template_source_from_journal`]: the projection from
//! a durable world's journal to the [`TemplateSource`] export reads, so
//! a template can be taken from a real world on disk. Its retained
//! state is only the `kv` entries the author names.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::program_bundle::{ProgramBundle, ProgramIdentity};

/// Template bundle format version. Bump when [`TemplateManifest`]
/// gains or loses a field.
pub const TEMPLATE_FORMAT_VERSION: u32 = 1;

/// A role definition carried by a template: name and purpose only.
/// Members live outside the template — roles are recreated empty on
/// import (D5).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoleDefinition {
    /// Role name, unique within the template.
    pub name: String,
    /// What the role is for. Recorded; never adjudicated here.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
}

/// One role in the export source: its definition plus the live
/// membership, which export always strips.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourcedRole {
    /// The definition the template keeps.
    pub definition: RoleDefinition,
    /// Live member identities. Never leaves the exporter.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub members: Vec<String>,
}

/// One credential or secret in the export source. The whole record is
/// identity material: export strips id and value alike.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourcedCredential {
    /// Credential id (e.g. key name). Stripped.
    pub id: String,
    /// The secret itself. Stripped.
    pub secret: String,
}

/// One per-user datum in the export source. Owner-keyed values belong
/// to a user, never to a template: export strips them whole.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourcedUserDatum {
    /// Owning user identity. Stripped.
    pub owner: String,
    /// Datum key. Stripped with the value.
    pub key: String,
    /// Datum value. Stripped.
    pub value: String,
}

/// The serializable world shape [`export_template`] reads. It is data,
/// not the live runtime: callers project whatever world state the
/// template may carry into this struct, and everything not projected
/// is out of scope by construction.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TemplateSource {
    /// Installed programs as bundle references (identity + content
    /// hash). Retained.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub programs: Vec<ProgramIdentity>,
    /// Roles with live membership. Definitions retained, members
    /// stripped.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roles: Vec<SourcedRole>,
    /// Credentials and secrets. Stripped whole.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub credentials: Vec<SourcedCredential>,
    /// Per-user data. Stripped whole.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub per_user_data: Vec<SourcedUserDatum>,
    /// Grants (member lists, portal/schedule grants, capability
    /// delegations). Stripped; descriptions are recorded only as
    /// counts in the manifest, never as content.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub grants: Vec<String>,
    /// Template-scoped world state (e.g. the world's `kv` entries the
    /// template author chose to carry). Retained verbatim.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub state: BTreeMap<String, String>,
}

/// What export removed, as `"<category>:<count>"` entries in a fixed
/// order (members, credentials, per-user data, grants). Categories
/// with a zero count are omitted.
pub fn stripped_entries(source: &TemplateSource) -> Vec<String> {
    let member_count: usize = source.roles.iter().map(|r| r.members.len()).sum();
    let mut entries = Vec::new();
    if member_count > 0 {
        entries.push(format!("members:{member_count}"));
    }
    if !source.credentials.is_empty() {
        entries.push(format!("credentials:{}", source.credentials.len()));
    }
    if !source.per_user_data.is_empty() {
        entries.push(format!("per-user-data:{}", source.per_user_data.len()));
    }
    if !source.grants.is_empty() {
        entries.push(format!("grants:{}", source.grants.len()));
    }
    entries
}

/// What export kept, as `"<category>:<count>"` entries in a fixed
/// order (programs, roles, state keys). Zero-count categories omitted.
pub fn retained_entries(source: &TemplateSource) -> Vec<String> {
    let mut entries = Vec::new();
    if !source.programs.is_empty() {
        entries.push(format!("programs:{}", source.programs.len()));
    }
    if !source.roles.is_empty() {
        entries.push(format!("roles:{}", source.roles.len()));
    }
    if !source.state.is_empty() {
        entries.push(format!("state-keys:{}", source.state.len()));
    }
    entries
}

/// The template manifest: everything a template carries plus the
/// record of everything export removed. An importer needs no other
/// document to verify what it is installing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TemplateManifest {
    /// [`TEMPLATE_FORMAT_VERSION`] at export time.
    pub format: u32,
    /// Program bundle references (identity + content hash).
    pub programs: Vec<ProgramIdentity>,
    /// Role definitions, without members.
    pub roles: Vec<RoleDefinition>,
    /// Template-scoped state, retained verbatim.
    pub state: BTreeMap<String, String>,
    /// One `"<category>:<count>"` entry per stripped category.
    pub stripped: Vec<String>,
    /// One `"<category>:<count>"` entry per retained category.
    pub retained: Vec<String>,
}

/// A template bundle: the manifest is the whole bundle in this slice.
/// (A file envelope and signatures arrive with the import slice; the
/// manifest already carries everything the importer must verify.)
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TemplateBundle {
    /// The manifest: carried content plus strip/retain accounting.
    pub manifest: TemplateManifest,
}

/// Export failures: the source data itself is unfit, never the strip
/// rules (stripping is total by construction).
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum TemplateError {
    /// A role has an empty name; it cannot be recreated on import.
    #[error("a role has an empty name")]
    EmptyRoleName,
    /// A program reference has an empty content hash; it cannot be
    /// hash-verified on import.
    #[error("program {program} has an empty content hash")]
    EmptyProgramHash {
        /// `author/name` of the offending reference.
        program: String,
    },
}

/// Snapshot a world projection into a template bundle, stripping all
/// identity material and recording the stripping in the manifest.
///
/// The bundle contains no member identity, no credential id or
/// secret, no per-user owner/key/value and no grant content for any
/// source — see the property tests. What the bundle *does* record is
/// the count per stripped category, so an importer (or auditor) can
/// check the manifest against the source world's own census.
pub fn export_template(source: &TemplateSource) -> Result<TemplateBundle, TemplateError> {
    for role in &source.roles {
        if role.definition.name.is_empty() {
            return Err(TemplateError::EmptyRoleName);
        }
    }
    for program in &source.programs {
        if program.content_hash.is_empty() {
            return Err(TemplateError::EmptyProgramHash {
                program: format!("{}/{}", program.author, program.name),
            });
        }
    }
    let manifest = TemplateManifest {
        format: TEMPLATE_FORMAT_VERSION,
        programs: source.programs.clone(),
        roles: source.roles.iter().map(|r| r.definition.clone()).collect(),
        state: source.state.clone(),
        stripped: stripped_entries(source),
        retained: retained_entries(source),
    };
    Ok(TemplateBundle { manifest })
}

/// The refusal name an unresolved template reference carries.
///
/// The name is from the v1 record's §6 taxonomy. A gap is not an
/// import failure: the world is created and the gap is recorded so a
/// later [`TemplateImport::resolve_gaps`] can finish it.
pub const UNRESOLVED_REFERENCE: &str = "template_reference_unresolved";

/// Stripped categories in the order [`stripped_entries`] writes them.
const STRIPPED_CATEGORIES: [&str; 4] = ["members", "credentials", "per-user-data", "grants"];

/// Why a template was not imported. Each refusal names one check, so a
/// CLI, a log or a UI can present it identically (v1 record §6).
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum TemplateImportError {
    /// The manifest was written by a format this build does not read.
    #[error("template refused at the format check: format {found}, this build reads {supported}")]
    UnsupportedFormat {
        /// The manifest's `format`.
        found: u32,
        /// [`TEMPLATE_FORMAT_VERSION`].
        supported: u32,
    },
    /// The caller supplied no identity for the new world.
    #[error("template refused at the world_id check: the new world identity is empty")]
    EmptyWorldId,
    /// A role definition has an empty name.
    #[error("template refused at the roles check: a role has an empty name")]
    EmptyRoleName,
    /// Two role definitions share a name, so they cannot both be
    /// recreated.
    #[error("template refused at the roles check: role {name} is defined twice")]
    DuplicateRole {
        /// The repeated role name.
        name: String,
    },
    /// A program reference has an empty content hash.
    #[error("template refused at the programs check: {program} has no content hash")]
    EmptyProgramHash {
        /// `author/name` of the reference.
        program: String,
    },
    /// The same program reference appears twice.
    #[error("template refused at the programs check: {program} is referenced twice")]
    DuplicateProgram {
        /// `author/name@version` of the reference.
        program: String,
    },
    /// The manifest's accounting does not describe its own content.
    #[error("template refused at the accounting check: {0}")]
    Accounting(String),
    /// The importer holds a bundle with the reference's author, name
    /// and version but a different content hash. Installing it would
    /// put a different program under the template's name.
    #[error(
        "template refused at the provenance_violation check: {program} is {template_hash} in the template and {held_hash} in the held bundle"
    )]
    ProvenanceViolation {
        /// `author/name@version` of the reference.
        program: String,
        /// The hash the template names.
        template_hash: String,
        /// The hash of the bundle the importer holds.
        held_hash: String,
    },
}

impl TemplateImportError {
    /// The stable name of the check that failed.
    #[must_use]
    pub const fn check(&self) -> &'static str {
        match self {
            Self::UnsupportedFormat { .. } => "format",
            Self::EmptyWorldId => "world_id",
            Self::EmptyRoleName | Self::DuplicateRole { .. } => "roles",
            Self::EmptyProgramHash { .. } | Self::DuplicateProgram { .. } => "programs",
            Self::Accounting(_) => "accounting",
            Self::ProvenanceViolation { .. } => "provenance_violation",
        }
    }
}

/// A world instantiated from a template, before its programs install.
///
/// Nothing here came from another user: the roles are definitions only
/// and start with no members, and the state is the template-scoped
/// state the exporter chose to carry.
#[derive(Clone, Debug)]
pub struct TemplateImport {
    /// The new world's identity, minted by the caller's world
    /// machinery. Never the source world's.
    pub world_id: String,
    /// Role definitions to recreate, each with no members.
    pub roles: Vec<RoleDefinition>,
    /// The starting state, verbatim from the template.
    pub state: BTreeMap<String, String>,
    /// Verified bundles to install through the D4 path, in template
    /// order. Each bundle's identity equals its template reference,
    /// content hash included.
    pub installs: Vec<ProgramBundle>,
    /// References the importer does not hold yet, in template order.
    /// Each is a [`UNRESOLVED_REFERENCE`] gap.
    pub gaps: Vec<ProgramIdentity>,
}

impl TemplateImport {
    /// Resolve recorded gaps against bundles the importer now holds.
    /// Resolved references move to [`Self::installs`]; the rest stay
    /// gaps. Returns how many were resolved.
    ///
    /// # Errors
    ///
    /// [`TemplateImportError::ProvenanceViolation`] when a held bundle
    /// has a gap's author, name and version under another hash. Nothing
    /// changes in that case.
    pub fn resolve_gaps(&mut self, held: &[ProgramBundle]) -> Result<usize, TemplateImportError> {
        let mut found = Vec::new();
        for (index, reference) in self.gaps.iter().enumerate() {
            if let Some(bundle) = resolve(reference, held)? {
                found.push((index, bundle));
            }
        }
        let resolved = found.len();
        for (index, bundle) in found.into_iter().rev() {
            self.gaps.remove(index);
            self.installs.push(bundle);
        }
        Ok(resolved)
    }
}

fn program_label(program: &ProgramIdentity) -> String {
    format!("{}/{}@{}", program.author, program.name, program.version)
}

/// Find the held bundle for `reference`. A bundle matches only on the
/// whole identity, content hash included; [`ProgramBundle`] can only be
/// held after its payload hashed to that content hash, so a match is
/// hash-verified.
fn resolve(
    reference: &ProgramIdentity,
    held: &[ProgramBundle],
) -> Result<Option<ProgramBundle>, TemplateImportError> {
    if let Some(bundle) = held.iter().find(|b| b.identity() == *reference) {
        return Ok(Some(bundle.clone()));
    }
    let conflict = held
        .iter()
        .map(ProgramBundle::identity)
        .find(|identity| identity.same_channel(reference) && identity.version == reference.version);
    match conflict {
        Some(identity) => Err(TemplateImportError::ProvenanceViolation {
            program: program_label(reference),
            template_hash: reference.content_hash.clone(),
            held_hash: identity.content_hash,
        }),
        None => Ok(None),
    }
}

/// The retained accounting a manifest's own content implies.
fn retained_for(manifest: &TemplateManifest) -> Vec<String> {
    let mut entries = Vec::new();
    if !manifest.programs.is_empty() {
        entries.push(format!("programs:{}", manifest.programs.len()));
    }
    if !manifest.roles.is_empty() {
        entries.push(format!("roles:{}", manifest.roles.len()));
    }
    if !manifest.state.is_empty() {
        entries.push(format!("state-keys:{}", manifest.state.len()));
    }
    entries
}

/// Stripped content is absent by design, so only the shape of its
/// accounting can be checked: known categories, positive counts, each
/// at most once and in the order export writes them.
fn check_stripped(stripped: &[String]) -> Result<(), TemplateImportError> {
    let mut next = 0;
    for entry in stripped {
        let refuse = || {
            TemplateImportError::Accounting(format!(
                "stripped entry {entry:?} is not one export writes"
            ))
        };
        let (category, raw) = entry.split_once(':').ok_or_else(refuse)?;
        let position = STRIPPED_CATEGORIES[next..]
            .iter()
            .position(|known| *known == category)
            .ok_or_else(refuse)?;
        let count: u64 = raw
            .parse()
            .map_err(|_not_a_count: std::num::ParseIntError| refuse())?;
        if count == 0 || count.to_string() != raw {
            return Err(refuse());
        }
        next += position + 1;
    }
    Ok(())
}

/// Instantiate a new world from a template bundle (D5; v1 record §5).
///
/// `world_id` is the fresh identity the caller's world machinery
/// minted for the new world. `held` is every bundle the importer has
/// at hand; only bundles the template references are used, so a held
/// bundle can never ride into the world unreferenced. Each reference
/// resolves to a held bundle with the identical identity, content hash
/// included, or is recorded as a gap.
///
/// The manifest is checked before anything is resolved: its format,
/// role names (non-empty, unique), program references (hashed,
/// unique), its retained accounting against its own content and the
/// shape of its stripped accounting.
///
/// # Errors
///
/// The first check the template fails, as a [`TemplateImportError`].
pub fn import_template(
    bundle: &TemplateBundle,
    world_id: &str,
    held: &[ProgramBundle],
) -> Result<TemplateImport, TemplateImportError> {
    let manifest = &bundle.manifest;
    if manifest.format != TEMPLATE_FORMAT_VERSION {
        return Err(TemplateImportError::UnsupportedFormat {
            found: manifest.format,
            supported: TEMPLATE_FORMAT_VERSION,
        });
    }
    if world_id.is_empty() {
        return Err(TemplateImportError::EmptyWorldId);
    }
    let mut names = std::collections::BTreeSet::new();
    for role in &manifest.roles {
        if role.name.is_empty() {
            return Err(TemplateImportError::EmptyRoleName);
        }
        if !names.insert(role.name.as_str()) {
            return Err(TemplateImportError::DuplicateRole {
                name: role.name.clone(),
            });
        }
    }
    for (index, program) in manifest.programs.iter().enumerate() {
        if program.content_hash.is_empty() {
            return Err(TemplateImportError::EmptyProgramHash {
                program: format!("{}/{}", program.author, program.name),
            });
        }
        if manifest.programs[..index].contains(program) {
            return Err(TemplateImportError::DuplicateProgram {
                program: program_label(program),
            });
        }
    }
    let implied = retained_for(manifest);
    if manifest.retained != implied {
        return Err(TemplateImportError::Accounting(format!(
            "retained says {:?}, the carried content is {implied:?}",
            manifest.retained
        )));
    }
    check_stripped(&manifest.stripped)?;

    let mut installs = Vec::new();
    let mut gaps = Vec::new();
    for reference in &manifest.programs {
        match resolve(reference, held)? {
            Some(bundle) => installs.push(bundle),
            None => gaps.push(reference.clone()),
        }
    }
    Ok(TemplateImport {
        world_id: world_id.to_owned(),
        roles: manifest.roles.clone(),
        state: manifest.state.clone(),
        installs,
        gaps,
    })
}

/// Why a world's journal cannot be projected into a [`TemplateSource`].
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum TemplateProjectionError {
    /// The journal does not open with its `Created` header.
    #[error("the journal has no Created header")]
    MissingHeader,
    /// The active program was not installed from a program bundle. A
    /// template carries bundle references, never re-hosted binaries
    /// (D5), so there is nothing it could name.
    #[error(
        "the active program {module_hash} was not installed from a program bundle, so a template cannot reference it"
    )]
    ProgramNotFromBundle {
        /// The active module's hash.
        module_hash: String,
    },
    /// The install record names a bundle whose content hash is not the
    /// installed module's hash.
    #[error("the active program {module_hash} records bundle content {bundle_hash}")]
    ProvenanceMismatch {
        /// The active module's hash.
        module_hash: String,
        /// The content hash the bundle provenance names.
        bundle_hash: String,
    },
    /// The author asked to keep a state key the world does not hold.
    #[error("state key {key:?} is not in the world's kv store")]
    UnknownStateKey {
        /// The requested key.
        key: String,
    },
}

/// The active program's hash and provenance, or `None` when no module
/// is active (an empty hash means none).
fn active_program(
    entries: &[crate::world::JournalEntry],
) -> Option<(&str, Option<&crate::world::ModuleProvenance>)> {
    use crate::world::{JournalEntry, WorldEventKind};
    let mut active = None;
    for entry in entries {
        match entry {
            JournalEntry::Checkpoint {
                module_hash,
                provenance,
                ..
            } => active = Some((module_hash.as_str(), provenance.as_ref())),
            JournalEntry::Committed { event } => {
                if let WorldEventKind::ProgramInstalled {
                    module_hash,
                    provenance,
                    ..
                } = &event.kind
                {
                    active = Some((module_hash.as_str(), provenance.as_ref()));
                }
            }
            _ => {}
        }
    }
    active.filter(|(hash, _)| !hash.is_empty())
}

/// Project a durable world's journal into the [`TemplateSource`] that
/// [`export_template`] reads.
///
/// What each part of the source holds:
///
/// - **programs:** the active program's bundle identity. A program
///   installed any other way is refused, because a template names
///   bundles and never carries binaries.
/// - **roles:** the latest D5 template-role definition set. Membership is
///   deliberately absent; `id-caps` access-control role configuration is a
///   separate surface and is not exported as template metadata.
/// - **credentials:** every capability record, live or revoked, once
///   per participant. Export strips them.
/// - **per-user data:** every author-key binding, once per participant.
///   Export strips them.
/// - **grants:** each portal grant and schedule grant that is in force
///   at the end of the journal. Export strips them.
/// - **state:** the world's `kv` entries named in `keep_state`, with
///   their stored JSON encodings. Nothing else of the world's state
///   (program snapshot, blobs, schedules, capability settings) is
///   template-scoped, so none of it is projected.
///
/// # Errors
///
/// A journal with no `Created` header, an active program that did not
/// come from a bundle (or whose provenance names other content), and a
/// `keep_state` key the world does not hold.
pub fn template_source_from_journal(
    entries: &[crate::world::JournalEntry],
    keep_state: &[String],
) -> Result<TemplateSource, TemplateProjectionError> {
    use crate::world::{JournalEntry, ModuleProvenance};
    if !matches!(entries.first(), Some(JournalEntry::Created { .. })) {
        return Err(TemplateProjectionError::MissingHeader);
    }

    let mut programs = Vec::new();
    if let Some((module_hash, provenance)) = active_program(entries) {
        let Some(ModuleProvenance::Bundle(bundle)) = provenance else {
            return Err(TemplateProjectionError::ProgramNotFromBundle {
                module_hash: module_hash.to_owned(),
            });
        };
        if bundle.program.content_hash != module_hash {
            return Err(TemplateProjectionError::ProvenanceMismatch {
                module_hash: module_hash.to_owned(),
                bundle_hash: bundle.program.content_hash.clone(),
            });
        }
        programs.push(bundle.program.clone());
    }

    let mut credentials = BTreeMap::new();
    let mut author_keys = BTreeMap::new();
    let mut portal = BTreeMap::new();
    let mut schedule = BTreeMap::new();
    let mut kv = BTreeMap::new();
    let mut role_definitions = Vec::new();
    for entry in entries {
        match entry {
            JournalEntry::Issued {
                participant,
                digest,
                ..
            } => {
                credentials.insert(participant.id, digest.clone());
            }
            JournalEntry::AuthorKeyBound {
                participant_id,
                author,
            } => {
                author_keys
                    .entry(*participant_id)
                    .or_insert_with(|| author.clone());
            }
            JournalEntry::PortalGrantSet { peer, granted } => {
                portal.insert(peer.clone(), *granted);
            }
            JournalEntry::ScheduleGrantSet {
                peer,
                send,
                receive,
            } => {
                schedule.insert(peer.clone(), *send || *receive);
            }
            JournalEntry::KvSet { key, value } => {
                kv.insert(key.clone(), value.clone());
            }
            JournalEntry::KvDeleted { key } => {
                kv.remove(key);
            }
            JournalEntry::TemplateRolesSet { roles } => {
                role_definitions.clone_from(roles);
            }
            _ => {}
        }
    }

    let mut state = BTreeMap::new();
    for key in keep_state {
        let value = kv
            .get(key)
            .ok_or_else(|| TemplateProjectionError::UnknownStateKey { key: key.clone() })?;
        state.insert(key.clone(), value.clone());
    }

    let in_force = |grants: BTreeMap<String, bool>, kind: &str| {
        grants
            .into_iter()
            .filter(|(_, live)| *live)
            .map(|(peer, _)| format!("{kind}:{peer}"))
            .collect::<Vec<_>>()
    };
    let mut grants = in_force(portal, "portal");
    grants.extend(in_force(schedule, "schedule"));

    Ok(TemplateSource {
        programs,
        roles: role_definitions
            .into_iter()
            .map(|definition| SourcedRole {
                definition,
                members: Vec::new(),
            })
            .collect(),
        credentials: credentials
            .into_iter()
            .map(|(id, digest)| SourcedCredential {
                id: id.to_string(),
                secret: digest,
            })
            .collect(),
        per_user_data: author_keys
            .into_iter()
            .map(|(id, author)| SourcedUserDatum {
                owner: id.to_string(),
                key: "author-key".to_owned(),
                value: author,
            })
            .collect(),
        grants,
        state,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// Every string value in an exported bundle, read off the struct
    /// (not out of JSON, where field names would pollute the set):
    /// program refs, role definitions, state keys and values, and the
    /// manifest accounting. If export ever smuggled a member name,
    /// secret or grant into any field, the explained-strings property
    /// below fails.
    fn bundle_strings(bundle: &TemplateBundle) -> Vec<String> {
        let manifest = &bundle.manifest;
        let mut out = Vec::new();
        for program in &manifest.programs {
            out.push(program.author.clone());
            out.push(program.name.clone());
            out.push(program.content_hash.clone());
        }
        for role in &manifest.roles {
            out.push(role.name.clone());
            if !role.description.is_empty() {
                out.push(role.description.clone());
            }
        }
        for (k, v) in &manifest.state {
            out.push(k.clone());
            out.push(v.clone());
        }
        out.extend(manifest.stripped.iter().cloned());
        out.extend(manifest.retained.iter().cloned());
        out
    }

    fn program(author: &str, name: &str, hash: &str) -> ProgramIdentity {
        ProgramIdentity {
            author: author.to_owned(),
            name: name.to_owned(),
            version: 1,
            content_hash: hash.to_owned(),
        }
    }

    /// The #8282 acceptance fixture in miniature: members, a
    /// credential, per-user data and grants all present in the
    /// source world.
    fn fixture_source() -> TemplateSource {
        TemplateSource {
            programs: vec![program("alice", "plaza", "abc123")],
            roles: vec![
                SourcedRole {
                    definition: RoleDefinition {
                        name: "admin".to_owned(),
                        description: "world admins".to_owned(),
                    },
                    members: vec!["alice".to_owned(), "bob".to_owned()],
                },
                SourcedRole {
                    definition: RoleDefinition {
                        name: "guest".to_owned(),
                        description: String::new(),
                    },
                    members: vec!["mallory".to_owned()],
                },
            ],
            credentials: vec![SourcedCredential {
                id: "api-key".to_owned(),
                secret: "supersecret-value-9f2c".to_owned(),
            }],
            per_user_data: vec![SourcedUserDatum {
                owner: "bob".to_owned(),
                key: "draft".to_owned(),
                value: "bob-private-draft-77".to_owned(),
            }],
            grants: vec!["portal:world-x:peer-y".to_owned()],
            state: [("motd".to_owned(), "hello template".to_owned())]
                .into_iter()
                .collect(),
        }
    }

    #[test]
    fn export_strips_identity_material_and_accounts_for_it() {
        let bundle = export_template(&fixture_source()).expect("fixture exports");
        let strings = bundle_strings(&bundle);

        // None of the fixture's identity material appears as a value
        // anywhere in the exported bundle, under any field name.
        // ("alice" doubles as the retained program's author, which is
        // why this compares exact values: the member position is
        // gone, the author position is kept.)
        for leaked in [
            "bob",
            "mallory",
            "api-key",
            "supersecret-value-9f2c",
            "bob-private-draft-77",
            "portal:world-x:peer-y",
        ] {
            assert!(
                !strings.iter().any(|s| s == leaked),
                "template bundle leaks identity material: {leaked}"
            );
        }
        // "alice" is both a stripped member and the retained
        // program's author: it appears exactly once, in the author
        // position — proving the member position is gone.
        assert_eq!(strings.iter().filter(|s| *s == "alice").count(), 1);
        assert_eq!(bundle.manifest.programs[0].author, "alice");

        // The manifest accounts for every stripped item by category.
        assert_eq!(
            bundle.manifest.stripped,
            vec![
                "members:3".to_owned(),
                "credentials:1".to_owned(),
                "per-user-data:1".to_owned(),
                "grants:1".to_owned(),
            ]
        );
        // ... and for everything retained.
        assert_eq!(
            bundle.manifest.retained,
            vec![
                "programs:1".to_owned(),
                "roles:2".to_owned(),
                "state-keys:1".to_owned(),
            ]
        );

        // Roles survive as definitions only; programs keep the hash
        // the importer will verify; template-scoped state is verbatim.
        assert_eq!(
            bundle.manifest.roles,
            vec![
                RoleDefinition {
                    name: "admin".to_owned(),
                    description: "world admins".to_owned(),
                },
                RoleDefinition {
                    name: "guest".to_owned(),
                    description: String::new(),
                },
            ]
        );
        assert_eq!(bundle.manifest.programs[0].content_hash, "abc123");
        assert_eq!(
            bundle.manifest.state.get("motd").map(String::as_str),
            Some("hello template")
        );
        assert_eq!(bundle.manifest.format, TEMPLATE_FORMAT_VERSION);
    }

    #[test]
    fn manifest_round_trips_through_json() {
        let bundle = export_template(&fixture_source()).expect("fixture exports");
        let wire = serde_json::to_string(&bundle.manifest).expect("manifest serializes");
        let back: TemplateManifest = serde_json::from_str(&wire).expect("manifest parses");
        assert_eq!(back, bundle.manifest);
    }

    #[test]
    fn empty_source_exports_to_empty_manifest() {
        let source = TemplateSource {
            programs: Vec::new(),
            roles: Vec::new(),
            credentials: Vec::new(),
            per_user_data: Vec::new(),
            grants: Vec::new(),
            state: BTreeMap::new(),
        };
        let bundle = export_template(&source).expect("empty source exports");
        assert_eq!(bundle.manifest.stripped, [] as [String; 0]);
        assert_eq!(bundle.manifest.retained, [] as [String; 0]);
        assert_eq!(bundle.manifest.roles, [] as [RoleDefinition; 0]);
    }

    #[test]
    fn empty_role_name_is_rejected() {
        let mut source = fixture_source();
        source.roles[0].definition.name.clear();
        assert_eq!(export_template(&source), Err(TemplateError::EmptyRoleName));
    }

    #[test]
    fn empty_program_hash_is_rejected() {
        let mut source = fixture_source();
        source.programs[0].content_hash.clear();
        assert_eq!(
            export_template(&source),
            Err(TemplateError::EmptyProgramHash {
                program: "alice/plaza".to_owned(),
            })
        );
    }

    mod import {
        use super::*;
        use crate::program_bundle::{ProgramSpec, export};
        use ed25519_dalek::SigningKey;

        const PAYLOAD: &[u8] = b"\0asm\x01\0\0\0 template program";

        fn alice() -> SigningKey {
            SigningKey::from_bytes(&[7; 32])
        }

        fn signed(name: &str, version: u64, payload: &[u8]) -> ProgramBundle {
            let spec = ProgramSpec {
                name,
                version,
                licence: None,
                description: None,
                categories: &[],
            };
            export(payload.to_vec(), spec, &alice()).expect("fixture bundle signs")
        }

        /// The #8282 fixture world W with real, signed programs.
        fn world_w(programs: &[&ProgramBundle]) -> TemplateSource {
            let mut source = fixture_source();
            source.programs = programs.iter().map(|b| b.identity()).collect();
            source
        }

        /// Export a source, write the manifest to its wire form and
        /// read it back, as a file round trip would.
        fn through_the_wire(source: &TemplateSource) -> TemplateBundle {
            let bundle = export_template(source).expect("source exports");
            let wire = serde_json::to_vec(&bundle).expect("bundle serializes");
            serde_json::from_slice(&wire).expect("bundle parses")
        }

        fn identities(import: &TemplateImport) -> Vec<ProgramIdentity> {
            import
                .installs
                .iter()
                .map(ProgramBundle::identity)
                .collect()
        }

        /// The #8282 acceptance: W -> template -> W'. W' has the same
        /// programs (hash-verified), the same role definitions with no
        /// members, the template-scoped state and its own identity, and
        /// none of W's members, credentials, per-user data or grants.
        #[test]
        fn round_trip_builds_a_fresh_world_without_identity_material() {
            let chess = signed("chess", 3, PAYLOAD);
            let board = signed("board", 1, b"\0asm\x01\0\0\0 board");
            let source = world_w(&[&chess, &board]);
            let template = through_the_wire(&source);

            let import = import_template(&template, "world-w-prime", &[board, chess])
                .expect("template imports");

            assert_ne!(import.world_id, "world-w");
            assert_eq!(import.world_id, "world-w-prime");
            assert_eq!(identities(&import), source.programs);
            for (installed, reference) in import.installs.iter().zip(&source.programs) {
                assert_eq!(
                    crate::world_session::module_hash(installed.payload()),
                    reference.content_hash,
                    "the install payload hashes to the template reference"
                );
            }
            assert!(import.gaps.is_empty());
            assert_eq!(
                import.roles,
                source
                    .roles
                    .iter()
                    .map(|r| r.definition.clone())
                    .collect::<Vec<_>>()
            );
            assert_eq!(import.state, source.state);

            let mut carried: Vec<String> = vec![import.world_id.clone()];
            for role in &import.roles {
                carried.push(role.name.clone());
                carried.push(role.description.clone());
            }
            for (k, v) in &import.state {
                carried.push(k.clone());
                carried.push(v.clone());
            }
            for leaked in [
                "bob",
                "mallory",
                "api-key",
                "supersecret-value-9f2c",
                "bob-private-draft-77",
                "portal:world-x:peer-y",
            ] {
                assert!(
                    !carried.iter().any(|s| s == leaked),
                    "imported world carries identity material: {leaked}"
                );
            }
        }

        #[test]
        fn a_held_bundle_the_template_does_not_name_is_not_installed() {
            let chess = signed("chess", 3, PAYLOAD);
            let stowaway = signed("stowaway", 1, b"\0asm\x01\0\0\0 stowaway");
            let template = through_the_wire(&world_w(&[&chess]));
            let import =
                import_template(&template, "w2", &[stowaway, chess.clone()]).expect("imports");
            assert_eq!(identities(&import), vec![chess.identity()]);
        }

        #[test]
        fn an_unresolved_reference_is_a_named_gap_that_resumes() {
            let chess = signed("chess", 3, PAYLOAD);
            let board = signed("board", 1, b"\0asm\x01\0\0\0 board");
            let source = world_w(&[&chess, &board]);
            let template = through_the_wire(&source);

            let mut import = import_template(&template, "w2", std::slice::from_ref(&chess))
                .expect("a gap is not a refusal");
            assert_eq!(identities(&import), vec![chess.identity()]);
            assert_eq!(import.gaps, vec![board.identity()]);
            assert_eq!(UNRESOLVED_REFERENCE, "template_reference_unresolved");

            assert_eq!(import.resolve_gaps(&[]), Ok(0));
            assert_eq!(import.gaps.len(), 1);
            assert_eq!(import.resolve_gaps(std::slice::from_ref(&board)), Ok(1));
            assert!(import.gaps.is_empty());
            assert_eq!(
                identities(&import),
                vec![chess.identity(), board.identity()]
            );
        }

        #[test]
        fn same_version_under_another_hash_is_a_provenance_violation() {
            let chess = signed("chess", 3, PAYLOAD);
            let impostor = signed("chess", 3, b"\0asm\x01\0\0\0 not the same program");
            let template = through_the_wire(&world_w(&[&chess]));

            let refusal = import_template(&template, "w2", std::slice::from_ref(&impostor))
                .expect_err("a different program under the template's name is refused");
            assert_eq!(refusal.check(), "provenance_violation");
            assert_eq!(
                refusal,
                TemplateImportError::ProvenanceViolation {
                    program: format!("{}/chess@3", chess.identity().author),
                    template_hash: chess.identity().content_hash,
                    held_hash: impostor.identity().content_hash,
                }
            );

            // The real bundle wins when both are held.
            let import = import_template(&template, "w2", &[impostor.clone(), chess.clone()])
                .expect("the exact bundle resolves");
            assert_eq!(identities(&import), vec![chess.identity()]);

            // Resuming a gap applies the same rule and changes nothing on refusal.
            let mut gapped = import_template(&template, "w2", &[]).expect("imports with a gap");
            assert_eq!(
                gapped.resolve_gaps(&[impostor]).map_err(|e| e.check()),
                Err("provenance_violation")
            );
            assert_eq!(gapped.gaps, vec![chess.identity()]);
            assert!(gapped.installs.is_empty());
        }

        #[test]
        fn a_later_version_of_the_same_program_is_a_gap_not_a_substitute() {
            let chess = signed("chess", 3, PAYLOAD);
            let newer = signed("chess", 4, b"\0asm\x01\0\0\0 chess four");
            let template = through_the_wire(&world_w(&[&chess]));
            let import = import_template(&template, "w2", &[newer]).expect("imports");
            assert!(import.installs.is_empty());
            assert_eq!(import.gaps, vec![chess.identity()]);
        }

        #[test]
        fn the_manifest_is_checked_before_anything_resolves() {
            let chess = signed("chess", 3, PAYLOAD);
            let good = through_the_wire(&world_w(&[&chess]));
            let held = [chess.clone()];
            let check = |bundle: &TemplateBundle, world: &str| {
                import_template(bundle, world, &held)
                    .map(|_| ())
                    .map_err(|e| e.check())
            };

            assert_eq!(check(&good, ""), Err("world_id"));

            let mut t = good.clone();
            t.manifest.format = TEMPLATE_FORMAT_VERSION + 1;
            assert_eq!(check(&t, "w2"), Err("format"));

            let mut t = good.clone();
            t.manifest.roles[1].name = t.manifest.roles[0].name.clone();
            assert_eq!(
                import_template(&t, "w2", &held).map(|_| ()),
                Err(TemplateImportError::DuplicateRole {
                    name: "admin".to_owned()
                })
            );

            let mut t = good.clone();
            t.manifest.roles[0].name.clear();
            assert_eq!(check(&t, "w2"), Err("roles"));

            let mut t = good.clone();
            t.manifest.programs[0].content_hash.clear();
            assert_eq!(check(&t, "w2"), Err("programs"));

            let mut t = good.clone();
            t.manifest.programs.push(chess.identity());
            t.manifest.retained[0] = "programs:2".to_owned();
            assert_eq!(check(&t, "w2"), Err("programs"));

            let mut t = good.clone();
            t.manifest
                .state
                .insert("extra".to_owned(), "unaccounted".to_owned());
            assert_eq!(check(&t, "w2"), Err("accounting"));

            for bad in [
                vec!["members:0".to_owned()],
                vec!["members:01".to_owned()],
                vec!["passwords:1".to_owned()],
                vec!["grants:1".to_owned(), "members:2".to_owned()],
                vec!["members:1".to_owned(), "members:1".to_owned()],
                vec!["members".to_owned()],
            ] {
                let mut t = good.clone();
                t.manifest.stripped = bad.clone();
                assert_eq!(check(&t, "w2"), Err("accounting"), "{bad:?}");
            }

            assert_eq!(check(&good, "w2"), Ok(()));
        }

        #[test]
        fn an_empty_template_imports_to_an_empty_world() {
            let source = TemplateSource {
                programs: Vec::new(),
                roles: Vec::new(),
                credentials: Vec::new(),
                per_user_data: Vec::new(),
                grants: Vec::new(),
                state: BTreeMap::new(),
            };
            let import = import_template(&through_the_wire(&source), "w2", &[]).expect("imports");
            assert!(import.roles.is_empty() && import.state.is_empty());
            assert!(import.installs.is_empty() && import.gaps.is_empty());
        }

        proptest::proptest! {
            /// Whatever the source world strips, its own manifest always
            /// passes the import checks.
            #[test]
            fn every_exported_manifest_passes_the_import_checks(source in property::arb_unique_source()) {
                let template = through_the_wire(&source);
                let import = import_template(&template, "w2", &[]).expect("exported manifests import");
                proptest::prop_assert_eq!(import.gaps, source.programs);
                proptest::prop_assert_eq!(import.state, source.state);
            }
        }
    }

    mod projection {
        use super::*;
        use crate::program_bundle::AbiDescriptor;
        use crate::world::{
            BundleProvenance, InstallPolicy, JournalEntry, ModuleProvenance, Participant,
            WorldEvent, WorldEventKind,
        };

        fn created() -> JournalEntry {
            JournalEntry::Created {
                world_id: "plaza".to_owned(),
                version: 1,
            }
        }

        fn issued(id: u64) -> JournalEntry {
            JournalEntry::Issued {
                participant: Participant {
                    id,
                    display_name: format!("player{id}"),
                },
                digest: format!("{id:064x}"),
                scopes: 1,
                parent: None,
                expires_at: None,
                uses: None,
                used: 0,
                subject: Some(format!("account{id}")),
                issued_at: None,
            }
        }

        fn identity(content_hash: &str) -> ProgramIdentity {
            ProgramIdentity {
                author: "ab".repeat(32),
                name: "board".to_owned(),
                version: 3,
                content_hash: content_hash.to_owned(),
            }
        }

        fn from_bundle(program: ProgramIdentity) -> ModuleProvenance {
            ModuleProvenance::Bundle(BundleProvenance {
                schema_version: 1,
                program,
                abi: AbiDescriptor::plaza_v1(),
                lineage: Vec::new(),
                licence: None,
                bundle_signature: "cd".repeat(64),
                policy: InstallPolicy::Pinned,
            })
        }

        fn installed(module_hash: &str, provenance: Option<ModuleProvenance>) -> JournalEntry {
            JournalEntry::Committed {
                event: WorldEvent {
                    sequence: 1,
                    participant_id: 1,
                    kind: WorldEventKind::ProgramInstalled {
                        module_hash: module_hash.to_owned(),
                        seed: 9,
                        provenance,
                        trust: None,
                    },
                },
            }
        }

        fn kv_set(key: &str, value: &str) -> JournalEntry {
            JournalEntry::KvSet {
                key: key.to_owned(),
                value: value.to_owned(),
            }
        }

        fn checkpoint(module_hash: &str, provenance: Option<ModuleProvenance>) -> JournalEntry {
            JournalEntry::Checkpoint {
                sequence: 4,
                module_hash: module_hash.to_owned(),
                seed: 9,
                provenance,
                execution_profile: None,
                caps_ceiling: None,
                schedules: None,
                kv: None,
                blobs: None,
                transfers: None,
                lease: None,
                snapshot: String::new(),
                events: Vec::new(),
                admin_history: None,
                open_presence: None,
            }
        }

        fn keep(keys: &[&str]) -> Vec<String> {
            keys.iter().map(|key| (*key).to_owned()).collect()
        }

        #[test]
        fn journal_roles_export_as_definitions_without_member_identity() {
            let role_entry: JournalEntry = serde_json::from_value(serde_json::json!({
                "entry": "template_roles_set",
                "roles": [
                    { "name": "editor", "description": "Can edit shared notes" },
                    { "name": "reader", "description": "Can read shared notes" }
                ]
            }))
            .unwrap();
            let entries = vec![created(), issued(1), issued(2), role_entry];

            let source = template_source_from_journal(&entries, &[]).unwrap();
            let bundle = export_template(&source).unwrap();

            assert!(source.roles.iter().all(|role| role.members.is_empty()));
            assert_eq!(
                bundle.manifest.roles,
                vec![
                    RoleDefinition {
                        name: "editor".to_owned(),
                        description: "Can edit shared notes".to_owned(),
                    },
                    RoleDefinition {
                        name: "reader".to_owned(),
                        description: "Can read shared notes".to_owned(),
                    },
                ]
            );
            let exported = serde_json::to_string(&bundle).unwrap();
            for identity in ["player1", "player2", "account1", "account2"] {
                assert!(
                    !exported.contains(identity),
                    "{identity} leaked into {exported}"
                );
            }
        }

        #[test]
        fn a_bundle_world_exports_without_identity_material() {
            let hash = "11".repeat(32);
            let author_key = "ef".repeat(32);
            let entries = vec![
                created(),
                issued(1),
                issued(2),
                JournalEntry::Revoked {
                    participant_id: 2,
                    by: None,
                    revoked_at: None,
                },
                installed(&hash, Some(from_bundle(identity(&hash)))),
                JournalEntry::AuthorKeyBound {
                    participant_id: 1,
                    author: author_key.clone(),
                },
                JournalEntry::PortalGrantSet {
                    peer: "north".to_owned(),
                    granted: true,
                },
                JournalEntry::PortalGrantSet {
                    peer: "south".to_owned(),
                    granted: true,
                },
                JournalEntry::PortalGrantSet {
                    peer: "south".to_owned(),
                    granted: false,
                },
                JournalEntry::ScheduleGrantSet {
                    peer: "east".to_owned(),
                    send: true,
                    receive: false,
                },
                kv_set("layout", r#"{"cells":9}"#),
                kv_set("private_note", r#""keep out""#),
            ];

            let source = template_source_from_journal(&entries, &keep(&["layout"])).unwrap();
            assert_eq!(source.programs, vec![identity(&hash)]);
            assert_eq!(source.credentials.len(), 2);
            assert_eq!(source.per_user_data.len(), 1);
            assert_eq!(source.grants, vec!["portal:north", "schedule:east"]);
            assert_eq!(
                source.state,
                BTreeMap::from([("layout".to_owned(), r#"{"cells":9}"#.to_owned())])
            );

            let bundle = export_template(&source).unwrap();
            assert_eq!(
                bundle.manifest.stripped,
                vec!["credentials:2", "per-user-data:1", "grants:2"]
            );
            assert_eq!(bundle.manifest.retained, vec!["programs:1", "state-keys:1"]);
            let text = serde_json::to_string(&bundle).unwrap();
            for leaked in [
                format!("{:064x}", 1),
                author_key,
                "player1".to_owned(),
                "account1".to_owned(),
                "private_note".to_owned(),
                "north".to_owned(),
                "east".to_owned(),
            ] {
                assert!(!text.contains(&leaked), "{leaked} leaked into {text}");
            }

            let import = import_template(&bundle, "copy", &[]).unwrap();
            assert_eq!(import.gaps, vec![identity(&hash)]);
            assert_eq!(import.state, source.state);
        }

        #[test]
        fn the_latest_install_or_checkpoint_is_the_active_program() {
            let first = "11".repeat(32);
            let second = "22".repeat(32);
            let entries = vec![
                created(),
                installed(&first, Some(from_bundle(identity(&first)))),
                installed(&second, Some(from_bundle(identity(&second)))),
            ];
            let source = template_source_from_journal(&entries, &[]).unwrap();
            assert_eq!(source.programs, vec![identity(&second)]);

            let compacted = vec![
                created(),
                kv_set("layout", "1"),
                checkpoint(&first, Some(from_bundle(identity(&first)))),
            ];
            let source = template_source_from_journal(&compacted, &keep(&["layout"])).unwrap();
            assert_eq!(source.programs, vec![identity(&first)]);
            assert_eq!(source.state.get("layout").map(String::as_str), Some("1"));
        }

        #[test]
        fn a_program_not_from_a_bundle_is_refused() {
            let hash = "11".repeat(32);
            for provenance in [None, Some(ModuleProvenance::Artifact)] {
                let entries = vec![created(), installed(&hash, provenance)];
                assert_eq!(
                    template_source_from_journal(&entries, &[]).unwrap_err(),
                    TemplateProjectionError::ProgramNotFromBundle {
                        module_hash: hash.clone()
                    }
                );
            }
        }

        #[test]
        fn provenance_naming_other_content_is_refused() {
            let hash = "11".repeat(32);
            let other = "22".repeat(32);
            let entries = vec![
                created(),
                installed(&hash, Some(from_bundle(identity(&other)))),
            ];
            assert_eq!(
                template_source_from_journal(&entries, &[]).unwrap_err(),
                TemplateProjectionError::ProvenanceMismatch {
                    module_hash: hash,
                    bundle_hash: other,
                }
            );
        }

        #[test]
        fn a_deleted_or_absent_state_key_is_refused() {
            let entries = vec![
                created(),
                kv_set("layout", "1"),
                JournalEntry::KvDeleted {
                    key: "layout".to_owned(),
                },
            ];
            for key in ["layout", "missing"] {
                assert_eq!(
                    template_source_from_journal(&entries, &keep(&[key])).unwrap_err(),
                    TemplateProjectionError::UnknownStateKey {
                        key: key.to_owned()
                    }
                );
            }
        }

        #[test]
        fn a_world_with_no_program_projects_no_reference() {
            let entries = vec![created(), installed("", None), kv_set("layout", "1")];
            let source = template_source_from_journal(&entries, &[]).unwrap();
            assert!(source.programs.is_empty());
            assert!(source.state.is_empty());
        }

        #[test]
        fn a_journal_without_its_header_is_refused() {
            for entries in [Vec::new(), vec![kv_set("layout", "1")]] {
                assert_eq!(
                    template_source_from_journal(&entries, &[]).unwrap_err(),
                    TemplateProjectionError::MissingHeader
                );
            }
        }
    }

    mod property {
        use super::*;
        use proptest::prelude::*;

        fn arb_program() -> impl Strategy<Value = ProgramIdentity> {
            ("[a-z0-9]{1,8}", "[a-z0-9]{1,8}", "[a-f0-9]{8,16}").prop_map(
                |(author, name, content_hash)| ProgramIdentity {
                    author,
                    name,
                    version: 1,
                    content_hash,
                },
            )
        }

        fn arb_source() -> impl Strategy<Value = TemplateSource> {
            (
                prop::collection::vec(arb_program(), 0..3),
                prop::collection::vec(
                    ("[a-z]{1,8}", 0..3usize).prop_flat_map(|(name, members)| {
                        (
                            Just(RoleDefinition {
                                name,
                                description: String::new(),
                            }),
                            prop::collection::vec("[a-z]{1,8}", members..=members),
                        )
                            .prop_map(|(definition, members)| {
                                SourcedRole {
                                    definition,
                                    members,
                                }
                            })
                    }),
                    0..3,
                ),
                prop::collection::vec("[a-z]{1,6}", 0..3),
                prop::collection::vec("[a-z]{1,6}", 0..3),
                prop::collection::vec("[a-z]{1,6}", 0..3),
                prop::collection::hash_map("[a-z]{1,6}", "[a-z]{1,6}", 0..3),
            )
                .prop_map(
                    |(programs, roles, credentials, per_user_data, grants, state)| TemplateSource {
                        programs,
                        roles,
                        credentials: credentials
                            .into_iter()
                            .map(|id| SourcedCredential {
                                id,
                                secret: "s".to_owned(),
                            })
                            .collect(),
                        per_user_data: per_user_data
                            .into_iter()
                            .map(|owner| SourcedUserDatum {
                                owner,
                                key: "k".to_owned(),
                                value: "v".to_owned(),
                            })
                            .collect(),
                        grants,
                        state: state.into_iter().collect(),
                    },
                )
        }

        /// [`arb_source`] with unique role names and program
        /// references, which import requires and a real world has.
        pub(super) fn arb_unique_source() -> impl Strategy<Value = TemplateSource> {
            arb_source().prop_map(|mut source| {
                let mut seen = std::collections::BTreeSet::new();
                source
                    .roles
                    .retain(|r| seen.insert(r.definition.name.clone()));
                let mut programs: Vec<ProgramIdentity> = Vec::new();
                for p in source.programs {
                    if !programs.contains(&p) {
                        programs.push(p);
                    }
                }
                source.programs = programs;
                source
            })
        }

        proptest! {
            /// D5 invariant, positional form: every string value in
            /// the bundle is *explained* — it comes from the retained
            /// projection (program refs, role definitions, state) or
            /// from the manifest accounting itself. No member,
            /// credential, per-user-datum or grant value reaches the
            /// bundle in any position, whatever the source holds —
            /// including collisions (a member named like a state key
            /// changes the multiset count, so equality still fails).
            #[test]
            fn export_carries_only_explained_strings(source in arb_source()) {
                let bundle = export_template(&source).expect("generated role names are non-empty");
                let mut actual = bundle_strings(&bundle);
                actual.sort();
                let mut expected: Vec<String> = Vec::new();
                for program in &source.programs {
                    expected.push(program.author.clone());
                    expected.push(program.name.clone());
                    expected.push(program.content_hash.clone());
                }
                for role in &source.roles {
                    expected.push(role.definition.name.clone());
                    if !role.definition.description.is_empty() {
                        expected.push(role.definition.description.clone());
                    }
                }
                for (k, v) in &source.state {
                    expected.push(k.clone());
                    expected.push(v.clone());
                }
                expected.extend(stripped_entries(&source));
                expected.extend(retained_entries(&source));
                expected.sort();
                prop_assert_eq!(actual, expected);
            }

            /// The manifest's stripped accounting covers every
            /// stripped category present in the source, exactly.
            #[test]
            fn stripped_accounting_is_exact(source in arb_source()) {
                let bundle = export_template(&source).expect("generated role names are non-empty");
                prop_assert_eq!(bundle.manifest.stripped, stripped_entries(&source));
                prop_assert_eq!(bundle.manifest.retained, retained_entries(&source));
            }

            /// Export preserves the carried content verbatim:
            /// programs (hash included), role definitions and state.
            #[test]
            fn carried_content_is_verbatim(source in arb_source()) {
                let bundle = export_template(&source).expect("generated role names are non-empty");
                prop_assert_eq!(bundle.manifest.programs, source.programs);
                prop_assert_eq!(
                    bundle.manifest.roles,
                    source.roles.iter().map(|r| r.definition.clone()).collect::<Vec<_>>()
                );
                prop_assert_eq!(bundle.manifest.state, source.state);
            }
        }
    }
}
