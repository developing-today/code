//! The central statement catalog: every statement this crate sends to the
//! central primary, with what each positional argument means.
//!
//! The hosted kv door (`pkgs/fleet/hosted-worker`, route
//! `POST /v1/kv/v2/pipeline`) never forwards SQL it did not get from here: it
//! reads a JSON copy of this catalog (`src/kv-catalog.json` in that package),
//! refuses any statement whose text is not in it, checks the argument count,
//! and uses the argument roles to bind the request to the authenticated
//! caller:
//!
//! - `scope` (with `registry` and `namespace`): the scope must be the
//!   namespace's shared scope, or a private scope that belongs to the caller
//!   ([`crate::central::private_scope_prefix`]).
//! - `owner` and `writer`: must be the caller's tenancy binding id.
//! - class `admin` (namespace definitions, grants): only for a caller holding
//!   the `kv-admin` role, or a service token scoped to the `kv-admin` door.
//!
//! The migrations ride along so the door can bring the primary's schema up
//! to date itself; no client ever runs DDL through it.
//!
//! The JSON copy must equal [`document`]: a test compares them, and
//! `FLEET_KV_WRITE_CATALOG=1 cargo test -p fleet-kv central_catalog` rewrites
//! the copy after a statement changes.

use crate::{archive, blob_central, central, grants};

/// What a statement may do, for the door's authorization.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    Read,
    Write,
    Admin,
}

impl Class {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
            Self::Admin => "admin",
        }
    }
}

/// The 1-based positions (`?N`) of the arguments the door checks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Roles {
    pub scope: Option<u8>,
    pub registry: Option<u8>,
    pub namespace: Option<u8>,
    pub owner: Option<u8>,
    pub writer: Option<u8>,
}

const ADDRESSED: Roles = Roles {
    scope: Some(1),
    registry: Some(2),
    namespace: Some(3),
    owner: None,
    writer: None,
};

const CAS: Roles = Roles {
    writer: Some(8),
    ..ADDRESSED
};

const PARTITION: Roles = Roles {
    scope: None,
    registry: None,
    namespace: None,
    owner: Some(1),
    writer: None,
};

const NONE: Roles = Roles {
    scope: None,
    registry: None,
    namespace: None,
    owner: None,
    writer: None,
};

/// One statement the client may send.
#[derive(Clone, Copy, Debug)]
pub struct Entry {
    pub name: &'static str,
    pub class: Class,
    pub sql: &'static str,
    pub roles: Roles,
}

impl Entry {
    /// How many arguments the statement takes: its highest `?N`.
    pub fn args(&self) -> usize {
        central::highest_parameter(self.sql)
    }
}

/// The query the door runs to learn a namespace's scope before it forwards
/// a statement addressed to it.
pub const DOOR_NAMESPACE_SCOPE: &str = "SELECT scope_kind, shared_scope_id FROM kv_namespaces
         WHERE registry_id = ?1 AND namespace = ?2";

/// The query the upload door runs to learn the partition a namespace places
/// its blobs in. No row: the upload is refused (no default partition).
pub const DOOR_NAMESPACE_PLACEMENT: &str = "SELECT jurisdiction, tier FROM kv_namespace_placement
         WHERE registry_id = ?1 AND namespace = ?2";

/// Every statement the client sends to the central primary.
pub fn entries() -> Vec<Entry> {
    use Class::{Admin, Read, Write};
    let entry = |name, class, sql, roles| Entry {
        name,
        class,
        sql,
        roles,
    };
    vec![
        entry("cas.receipt_insert", Write, central::RECEIPT_INSERT, CAS),
        entry("cas.conflict_insert", Write, central::CONFLICT_INSERT, CAS),
        entry("cas.version_insert", Write, central::VERSION_INSERT, CAS),
        entry("cas.head_upsert", Write, central::HEAD_UPSERT, CAS),
        entry("cas.receipt_read", Read, central::RECEIPT_READ, CAS),
        entry("head.read", Read, central::HEAD_READ, ADDRESSED),
        entry("history.read", Read, central::HISTORY_READ, ADDRESSED),
        entry("conflicts.read", Read, central::CONFLICTS_READ, ADDRESSED),
        entry("heads.read", Read, central::HEADS_READ, ADDRESSED),
        entry(
            "head.read_enveloped",
            Read,
            central::HEAD_ENVELOPED_READ,
            ADDRESSED,
        ),
        entry(
            "heads.read_enveloped",
            Read,
            central::HEADS_ENVELOPED_READ,
            ADDRESSED,
        ),
        entry("listing.read", Read, central::LISTING_READ, ADDRESSED),
        entry("namespace.define", Admin, central::NAMESPACE_UPSERT, NONE),
        entry("namespace.epoch", Read, central::NAMESPACE_EPOCH_READ, NONE),
        entry("namespaces.read", Read, central::NAMESPACES_READ, NONE),
        entry("placement.upsert", Admin, central::PLACEMENT_UPSERT, NONE),
        entry("placement.delete", Admin, central::PLACEMENT_DELETE, NONE),
        entry("placements.read", Read, central::PLACEMENTS_READ, NONE),
        entry("grants.read", Read, grants::GRANTS_READ, NONE),
        entry("grants.upsert", Admin, grants::GRANT_UPSERT, NONE),
        entry("grants.delete", Admin, grants::GRANT_DELETE, NONE),
        entry("archive.clear", Write, archive::ARCHIVE_CLEAR, PARTITION),
        entry("archive.insert", Write, archive::ARCHIVE_INSERT, PARTITION),
        entry(
            "archive.checkpoint_upsert",
            Write,
            archive::CHECKPOINT_UPSERT,
            PARTITION,
        ),
        entry(
            "archive.checkpoint_read",
            Read,
            archive::CHECKPOINT_READ,
            PARTITION,
        ),
        entry("archive.read", Read, archive::ARCHIVE_READ, PARTITION),
        entry(
            "blob.cas_receipt_insert",
            Write,
            central::BLOB_RECEIPT_INSERT,
            CAS,
        ),
        entry(
            "blob.cas_conflict_insert",
            Write,
            central::BLOB_CONFLICT_INSERT,
            CAS,
        ),
        entry(
            "blob.cas_version_insert",
            Write,
            central::BLOB_VERSION_INSERT,
            CAS,
        ),
        entry("blob.ref_insert", Write, blob_central::BLOB_REF_INSERT, CAS),
        entry("blob.publish", Admin, blob_central::BLOB_PUBLISH, NONE),
        entry(
            "blob.published_read",
            Admin,
            blob_central::BLOB_PUBLISHED_READ,
            NONE,
        ),
        entry(
            "blob.roots_read",
            Admin,
            blob_central::BLOB_ROOTS_READ,
            NONE,
        ),
        entry("blobs.read", Admin, blob_central::BLOBS_READ, NONE),
        entry(
            "blob.deletion_insert",
            Admin,
            blob_central::BLOB_DELETION_INSERT,
            NONE,
        ),
        entry("blob.delete", Admin, blob_central::BLOB_DELETE, NONE),
        entry(
            "blob.deletion_read",
            Admin,
            blob_central::BLOB_DELETION_READ,
            NONE,
        ),
        entry(
            "blob.candidates_clear",
            Admin,
            blob_central::BLOB_CANDIDATES_CLEAR,
            NONE,
        ),
        entry(
            "blob.candidates_mark",
            Admin,
            blob_central::BLOB_CANDIDATES_MARK,
            NONE,
        ),
        entry(
            "blob.candidates_due",
            Admin,
            blob_central::BLOB_CANDIDATES_DUE,
            NONE,
        ),
        entry(
            "blob.candidate_drop",
            Admin,
            blob_central::BLOB_CANDIDATE_DROP,
            NONE,
        ),
        entry(
            "blob.purge_claim",
            Admin,
            blob_central::BLOB_PURGE_CLAIM,
            NONE,
        ),
        entry(
            "blob.purged_mark",
            Admin,
            blob_central::BLOB_PURGED_MARK,
            NONE,
        ),
        entry(
            "blob.purge_release",
            Admin,
            blob_central::BLOB_PURGE_RELEASE,
            NONE,
        ),
    ]
}

/// The catalog entry whose text is exactly `sql`.
pub fn find(sql: &str) -> Option<Entry> {
    entries().into_iter().find(|entry| entry.sql == sql)
}

/// True when `sql` is a catalog statement, a migration statement or the
/// migration bookkeeping: everything that may reach a primary.
pub fn is_known(sql: &str) -> bool {
    find(sql).is_some()
        || sql == DOOR_NAMESPACE_SCOPE
        || sql == DOOR_NAMESPACE_PLACEMENT
        || [
            central::MIGRATIONS_TABLE,
            central::MIGRATIONS_READ,
            central::MIGRATION_RECORD,
        ]
        .contains(&sql)
        || central::MIGRATIONS.iter().any(|(_, source)| {
            central::migration_statements(source)
                .iter()
                .any(|s| s == sql)
        })
}

/// The catalog as the JSON document the hosted door reads.
#[cfg(feature = "http")]
pub fn document() -> serde_json::Value {
    use serde_json::json;
    let role = |index: Option<u8>| index.map_or(serde_json::Value::Null, |index| json!(index));
    json!({
        "format": "fleet-kv-door-catalog/1",
        "source": "pkgs/fleet/crates/fleet-kv/src/catalog.rs",
        "admin_role": "kv-admin",
        "admin_door": "kv-admin",
        "door_namespace_scope": DOOR_NAMESPACE_SCOPE,
        "door_namespace_placement": DOOR_NAMESPACE_PLACEMENT,
        "statements": entries().iter().map(|entry| json!({
            "name": entry.name,
            "class": entry.class.as_str(),
            "args": entry.args(),
            "scope": role(entry.roles.scope),
            "registry": role(entry.roles.registry),
            "namespace": role(entry.roles.namespace),
            "owner": role(entry.roles.owner),
            "writer": role(entry.roles.writer),
            "sql": entry.sql,
        })).collect::<Vec<_>>(),
        "migrations": {
            "table": central::MIGRATIONS_TABLE,
            "read": central::MIGRATIONS_READ,
            "record": central::MIGRATION_RECORD,
            "versions": central::MIGRATIONS.iter().map(|(version, source)| json!({
                "version": version,
                "statements": central::migration_statements(source),
            })).collect::<Vec<_>>(),
        },
    })
}

/// The JSON copy's text: pretty-printed with a trailing newline. The copy is
/// checked by value, so its key order may differ.
#[cfg(feature = "http")]
pub fn document_text() -> String {
    let mut text = serde_json::to_string_pretty(&document()).unwrap_or_default();
    text.push('\n');
    text
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::central::{Pipeline, SqlitePrimary, Statement, Value};
    use std::collections::HashSet;

    #[test]
    fn central_catalog_names_and_texts_are_unique() {
        let entries = entries();
        let names: HashSet<_> = entries.iter().map(|entry| entry.name).collect();
        let texts: HashSet<_> = entries.iter().map(|entry| entry.sql).collect();
        assert_eq!(names.len(), entries.len());
        assert_eq!(texts.len(), entries.len());
    }

    #[test]
    fn central_catalog_roles_fall_inside_the_arguments() {
        for entry in entries() {
            let roles = entry.roles;
            for index in [
                roles.scope,
                roles.registry,
                roles.namespace,
                roles.owner,
                roles.writer,
            ]
            .into_iter()
            .flatten()
            {
                assert!(
                    usize::from(index) <= entry.args(),
                    "{} names ?{index} but takes {}",
                    entry.name,
                    entry.args()
                );
            }
            // A scope is only checkable with its namespace beside it.
            assert_eq!(
                roles.scope.is_some(),
                roles.registry.is_some(),
                "{}",
                entry.name
            );
            assert_eq!(
                roles.scope.is_some(),
                roles.namespace.is_some(),
                "{}",
                entry.name
            );
            // Every write is bound to a caller: by scope, owner or writer.
            if entry.class == Class::Write {
                assert!(
                    roles.scope.is_some() || roles.owner.is_some(),
                    "{} writes unbound",
                    entry.name
                );
            }
        }
    }

    #[test]
    fn central_catalog_statements_prepare_on_a_migrated_primary() {
        let primary = SqlitePrimary::in_memory().unwrap();
        for sql in entries()
            .iter()
            .map(|entry| entry.sql)
            .chain([DOOR_NAMESPACE_SCOPE])
        {
            let args = vec![Value::Null; central::highest_parameter(sql)];
            // Prepared and run inside a transaction the test then drops: a
            // statement that names a missing table or column fails here.
            let result = primary.transaction(&[Statement {
                sql: sql.to_string(),
                args,
            }]);
            if let Err(error) = result {
                // NOT NULL / CHECK failures prove the statement prepared.
                let text = error.to_string();
                assert!(
                    text.contains("constraint") || text.contains("NOT NULL"),
                    "{sql}: {text}"
                );
            }
        }
    }

    #[cfg(feature = "http")]
    #[test]
    fn central_catalog_matches_the_hosted_door_copy() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../hosted-worker/src/kv-catalog.json");
        let expected = document_text();
        if std::env::var_os("FLEET_KV_WRITE_CATALOG").is_some() {
            std::fs::write(&path, &expected).expect("write the door catalog");
            println!("===BEGIN kv-catalog.json===\n{expected}===END kv-catalog.json===");
            return;
        }
        // Compared as JSON values, not text: whether serde_json keeps key
        // order depends on which workspace crates are in the build.
        let actual = std::fs::read_to_string(&path).unwrap_or_default();
        let parsed: Option<serde_json::Value> = serde_json::from_str(&actual).ok();
        if parsed.as_ref() != Some(&document()) {
            println!("===BEGIN kv-catalog.json===\n{expected}===END kv-catalog.json===");
            panic!(
                "{} is stale: rerun with FLEET_KV_WRITE_CATALOG=1 (the expected text is above)",
                path.display()
            );
        }
    }
}
