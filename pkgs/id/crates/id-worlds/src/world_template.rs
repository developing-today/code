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
//! This slice covers the manifest format, the strip rules and the
//! export half. Import (fresh world identity, empty roles,
//! hash-verified program install through the ordinary D4 path) is the
//! next slice; the round-trip acceptance on #8282 closes over both.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::program_bundle::ProgramIdentity;

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
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TemplateError {
    /// A role has an empty name; it cannot be recreated on import.
    EmptyRoleName,
    /// A program reference has an empty content hash; it cannot be
    /// hash-verified on import.
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

#[cfg(test)]
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
