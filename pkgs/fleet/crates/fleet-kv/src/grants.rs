//! Group databases by explicit grant (code#13316 acceptance 4, design D014).
//!
//! Every central write lands on the shared primary unless it names a group.
//! A write that names a group goes to that group's own database only when a
//! `kv_grants` row `(group, database, registry, namespace, allowed ops,
//! policy_epoch)` on the shared primary authorizes it; anything else refuses
//! fail-closed:
//!
//! - a group with no grant rows at all: `unknown_group`;
//! - a granted group, but not for this namespace: `namespace_not_granted`;
//! - a granted namespace, but not for this operation: `op_not_granted`;
//! - a grant whose database this host cannot reach: `group_db_unprovisioned`.
//!
//! Grants are issued only through [`admit`], which checks the namespace is
//! defined, the requester belongs to the group and the group owns the
//! namespace. No session is given a database by default: with no grant,
//! nothing but the shared primary exists.
//!
//! Group databases hold inline bytes only. A value over the namespace's
//! inline threshold addressed to a group refuses with
//! `blob_requires_shared_primary` before anything is written; it is never
//! moved to the shared primary on the caller's behalf.

use std::time::Duration;

use crate::central::{
    self, CentralError, Intent, Namespace, Op, Pipeline, REGISTRY_ID, Receipt, Statement, Value,
};

/// One operation a grant can allow.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum GrantOp {
    Read,
    Write,
    Delete,
}

impl GrantOp {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
            Self::Delete => "delete",
        }
    }

    fn parse(word: &str) -> Option<Self> {
        Some(match word {
            "read" => Self::Read,
            "write" => Self::Write,
            "delete" => Self::Delete,
            _ => return None,
        })
    }

    /// The operation a CAS performs.
    pub fn of(op: &Op) -> Self {
        match op {
            Op::Set(_) => Self::Write,
            Op::Tombstone => Self::Delete,
        }
    }
}

/// One `kv_grants` row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Grant {
    pub group_id: String,
    pub database_name: String,
    pub registry_id: String,
    pub namespace: String,
    pub allowed_ops: Vec<GrantOp>,
    pub policy_epoch: u64,
}

impl Grant {
    pub fn allows(&self, op: GrantOp) -> bool {
        self.allowed_ops.contains(&op)
    }
}

/// A grant refusal, or a central failure while deciding one.
#[derive(Debug)]
pub enum GrantError {
    Central(CentralError),
    UnknownGroup(String),
    NamespaceNotGranted { group_id: String, namespace: String },
    OpNotGranted { group_id: String, op: GrantOp },
    NotAMember { group_id: String },
    NotNamespaceOwner { group_id: String, namespace: String },
    GroupDbUnprovisioned { database_name: String },
    BlobRequiresSharedPrimary { threshold: u64 },
    Malformed(&'static str),
}

impl GrantError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Central(error) => error.code(),
            Self::UnknownGroup(_) => "unknown_group",
            Self::NamespaceNotGranted { .. } => "namespace_not_granted",
            Self::OpNotGranted { .. } => "op_not_granted",
            Self::NotAMember { .. } => "not_a_group_member",
            Self::NotNamespaceOwner { .. } => "not_namespace_owner",
            Self::GroupDbUnprovisioned { .. } => "group_db_unprovisioned",
            Self::BlobRequiresSharedPrimary { .. } => "blob_requires_shared_primary",
            Self::Malformed(code) => *code,
        }
    }
}

impl std::fmt::Display for GrantError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Central(error) => write!(f, "{error}"),
            Self::UnknownGroup(group) => write!(f, "no grant names group `{group}`"),
            Self::NamespaceNotGranted {
                group_id,
                namespace,
            } => {
                write!(
                    f,
                    "group `{group_id}` has no grant for namespace `{namespace}`"
                )
            }
            Self::OpNotGranted { group_id, op } => {
                write!(
                    f,
                    "group `{group_id}`'s grant does not allow `{}`",
                    op.as_str()
                )
            }
            Self::NotAMember { group_id } => {
                write!(f, "the requester is not a member of group `{group_id}`")
            }
            Self::NotNamespaceOwner {
                group_id,
                namespace,
            } => {
                write!(f, "group `{group_id}` does not own namespace `{namespace}`")
            }
            Self::GroupDbUnprovisioned { database_name } => write!(
                f,
                "group database `{database_name}` is granted but not provisioned for this host"
            ),
            Self::BlobRequiresSharedPrimary { threshold } => write!(
                f,
                "group databases hold inline bytes only (at most {threshold} bytes here); \
                 blob placement needs the shared primary"
            ),
            Self::Malformed(code) => write!(f, "{code}"),
        }
    }
}

impl std::error::Error for GrantError {}

impl From<CentralError> for GrantError {
    fn from(error: CentralError) -> Self {
        Self::Central(error)
    }
}

pub type Result<T> = std::result::Result<T, GrantError>;

fn text(value: &str) -> Value {
    Value::Text(value.to_string())
}

fn ops_text(ops: &[GrantOp]) -> String {
    let mut ops = ops.to_vec();
    ops.sort();
    ops.dedup();
    ops.iter()
        .map(|op| op.as_str())
        .collect::<Vec<_>>()
        .join(",")
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 63
        && name.starts_with(|c: char| c.is_ascii_alphabetic())
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// Every grant naming `group_id`.
pub fn grants(primary: &dyn Pipeline, group_id: &str) -> Result<Vec<Grant>> {
    let rows = primary
        .transaction(&[Statement::numbered(
            "SELECT group_id, database_name, registry_id, namespace, allowed_ops, policy_epoch
             FROM kv_grants WHERE group_id = ?1 ORDER BY namespace, database_name",
            &[text(group_id)],
        )])?
        .into_iter()
        .next()
        .map(|rows| rows.rows)
        .unwrap_or_default();
    Ok(rows
        .into_iter()
        .map(|row| Grant {
            group_id: row[0].as_str().unwrap_or_default().to_string(),
            database_name: row[1].as_str().unwrap_or_default().to_string(),
            registry_id: row[2].as_str().unwrap_or_default().to_string(),
            namespace: row[3].as_str().unwrap_or_default().to_string(),
            allowed_ops: row[4]
                .as_str()
                .unwrap_or_default()
                .split(',')
                .filter_map(GrantOp::parse)
                .collect(),
            policy_epoch: row[5].as_i64().unwrap_or(0).max(0) as u64,
        })
        .collect())
}

/// Who belongs to which group and which namespaces each group owns: the
/// admission policy's source of truth.
pub trait GroupDirectory {
    fn is_member(&self, group_id: &str, user_id: &str) -> bool;
    fn owns_namespace(&self, group_id: &str, namespace: &str) -> bool;
}

/// A request to give a group its own database for one namespace.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GrantRequest {
    pub requester: String,
    pub group_id: String,
    pub database_name: String,
    pub namespace: String,
    pub allowed_ops: Vec<GrantOp>,
}

/// Admits a grant request: the namespace must be defined in
/// `fleet-kv-central`, the requester must belong to the group and the group
/// must own the namespace. On success the grant row is written (its policy
/// epoch one past any earlier grant for the same group, database and
/// namespace) and returned. Any failed check writes nothing.
pub fn admit(
    primary: &dyn Pipeline,
    directory: &dyn GroupDirectory,
    request: &GrantRequest,
) -> Result<Grant> {
    if !valid_name(&request.group_id) {
        return Err(GrantError::Malformed("malformed_group"));
    }
    if !valid_name(&request.database_name) {
        return Err(GrantError::Malformed("malformed_database"));
    }
    if request.allowed_ops.is_empty() {
        return Err(GrantError::Malformed("malformed_ops"));
    }
    central::resolve(primary, REGISTRY_ID, &request.namespace)?;
    if !directory.is_member(&request.group_id, &request.requester) {
        return Err(GrantError::NotAMember {
            group_id: request.group_id.clone(),
        });
    }
    if !directory.owns_namespace(&request.group_id, &request.namespace) {
        return Err(GrantError::NotNamespaceOwner {
            group_id: request.group_id.clone(),
            namespace: request.namespace.clone(),
        });
    }
    let args = [
        text(&request.group_id),
        text(&request.database_name),
        text(REGISTRY_ID),
        text(&request.namespace),
        text(&ops_text(&request.allowed_ops)),
    ];
    primary.transaction(&[Statement::numbered(
        "INSERT INTO kv_grants
             (group_id, database_name, registry_id, namespace, allowed_ops, policy_epoch)
         VALUES (?1, ?2, ?3, ?4, ?5, 1)
         ON CONFLICT (group_id, database_name, registry_id, namespace) DO UPDATE SET
             allowed_ops = excluded.allowed_ops,
             policy_epoch = kv_grants.policy_epoch + 1",
        &args,
    )])?;
    grants(primary, &request.group_id)?
        .into_iter()
        .find(|grant| {
            grant.database_name == request.database_name && grant.namespace == request.namespace
        })
        .ok_or_else(|| GrantError::Central(CentralError::Store("the grant was not stored".into())))
}

/// Removes a grant. The group's database is no longer reachable through it.
pub fn revoke(
    primary: &dyn Pipeline,
    group_id: &str,
    database_name: &str,
    namespace: &str,
) -> Result<()> {
    primary.transaction(&[Statement::numbered(
        "DELETE FROM kv_grants
         WHERE group_id = ?1 AND database_name = ?2 AND registry_id = ?3 AND namespace = ?4",
        &[
            text(group_id),
            text(database_name),
            text(REGISTRY_ID),
            text(namespace),
        ],
    )])?;
    Ok(())
}

/// Where a central operation lands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Placement {
    SharedPrimary,
    Group(Grant),
}

/// Decides where an operation on `namespace` lands. With no group it is the
/// shared primary; with a group it is that group's database, only under a
/// grant that covers the namespace and the operation.
pub fn place(
    primary: &dyn Pipeline,
    group_id: Option<&str>,
    namespace: &str,
    op: GrantOp,
) -> Result<Placement> {
    let Some(group_id) = group_id else {
        return Ok(Placement::SharedPrimary);
    };
    let granted = grants(primary, group_id)?;
    if granted.is_empty() {
        return Err(GrantError::UnknownGroup(group_id.to_string()));
    }
    let grant = granted
        .into_iter()
        .find(|grant| grant.namespace == namespace && grant.registry_id == REGISTRY_ID)
        .ok_or_else(|| GrantError::NamespaceNotGranted {
            group_id: group_id.to_string(),
            namespace: namespace.to_string(),
        })?;
    if !grant.allows(op) {
        return Err(GrantError::OpNotGranted {
            group_id: group_id.to_string(),
            op,
        });
    }
    Ok(Placement::Group(grant))
}

/// Opens granted group databases. Provisioning a database is outside this
/// crate; a host reaches only the databases its provisioner knows.
pub trait Provisioner {
    /// The pipeline for `database_name`, or `None` when this host has none.
    fn database(&self, database_name: &str) -> Option<&dyn Pipeline>;
}

/// The default: no group database is provisioned anywhere.
pub struct NoGroupDatabases;

impl Provisioner for NoGroupDatabases {
    fn database(&self, _: &str) -> Option<&dyn Pipeline> {
        None
    }
}

/// A CAS addressed to `group_id`'s database (or the shared primary when
/// `group_id` is `None`). Group writes are checked against the namespace's
/// inline threshold before any database is touched.
pub fn write(
    primary: &dyn Pipeline,
    provisioner: &dyn Provisioner,
    group_id: Option<&str>,
    definition: &Namespace,
    intent: &Intent,
    sleep: &mut dyn FnMut(Duration),
) -> Result<Receipt> {
    let placement = place(
        primary,
        group_id,
        &intent.address.namespace,
        GrantOp::of(&intent.op),
    )?;
    let target = match &placement {
        Placement::SharedPrimary => primary,
        Placement::Group(grant) => {
            if let Op::Set(value) = &intent.op
                && value.len() as u64 > definition.blob_threshold
            {
                return Err(GrantError::BlobRequiresSharedPrimary {
                    threshold: definition.blob_threshold,
                });
            }
            provisioner.database(&grant.database_name).ok_or_else(|| {
                GrantError::GroupDbUnprovisioned {
                    database_name: grant.database_name.clone(),
                }
            })?
        }
    };
    Ok(central::cas_with_retry(target, intent, sleep)?)
}

/// A read of one head, placed the same way as [`write`].
pub fn read(
    primary: &dyn Pipeline,
    provisioner: &dyn Provisioner,
    group_id: Option<&str>,
    address: &central::Address,
) -> Result<Option<central::Head>> {
    let placement = place(primary, group_id, &address.namespace, GrantOp::Read)?;
    let target = match &placement {
        Placement::SharedPrimary => primary,
        Placement::Group(grant) => provisioner.database(&grant.database_name).ok_or_else(|| {
            GrantError::GroupDbUnprovisioned {
                database_name: grant.database_name.clone(),
            }
        })?,
    };
    Ok(central::head(target, address)?)
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::central::{Address, IdempotencyKey, SqlitePrimary, define_namespace, head};
    use std::collections::HashMap;

    struct Directory;

    impl GroupDirectory for Directory {
        fn is_member(&self, group_id: &str, user_id: &str) -> bool {
            matches!((group_id, user_id), ("repo-code", "drew" | "agent-1"))
        }
        fn owns_namespace(&self, group_id: &str, namespace: &str) -> bool {
            group_id == "repo-code" && namespace == "build"
        }
    }

    struct Databases(HashMap<String, SqlitePrimary>);

    impl Provisioner for Databases {
        fn database(&self, database_name: &str) -> Option<&dyn Pipeline> {
            self.0.get(database_name).map(|db| db as &dyn Pipeline)
        }
    }

    fn setup() -> (SqlitePrimary, Databases, Namespace) {
        let primary = SqlitePrimary::in_memory().unwrap();
        let mut build = Namespace::shared("build", "build-cache");
        build.blob_threshold = 8;
        define_namespace(&primary, &build).unwrap();
        define_namespace(&primary, &Namespace::shared("status", "fleet-status")).unwrap();
        let databases = Databases(HashMap::from([(
            "code-build".to_string(),
            SqlitePrimary::in_memory().unwrap(),
        )]));
        (primary, databases, build)
    }

    fn request(requester: &str, namespace: &str, ops: &[GrantOp]) -> GrantRequest {
        GrantRequest {
            requester: requester.into(),
            group_id: "repo-code".into(),
            database_name: "code-build".into(),
            namespace: namespace.into(),
            allowed_ops: ops.to_vec(),
        }
    }

    fn intent(namespace: &str, key: &str, op: Op, id: &str) -> Intent {
        let scope = if namespace == "build" {
            "build-cache"
        } else {
            "fleet-status"
        };
        Intent {
            address: Address::new(scope, namespace, key),
            expected_version: 0,
            op,
            idempotency: IdempotencyKey {
                writer: "agent-1".into(),
                host_incarnation: "host".into(),
                outbox_id: id.into(),
            },
        }
    }

    fn no_sleep() -> impl FnMut(Duration) {
        |_| {}
    }

    #[test]
    fn central_grants_admission_is_fail_closed() {
        let (primary, _, _) = setup();
        let ops = [GrantOp::Read, GrantOp::Write];
        assert_eq!(
            admit(&primary, &Directory, &request("stranger", "build", &ops))
                .unwrap_err()
                .code(),
            "not_a_group_member"
        );
        assert_eq!(
            admit(&primary, &Directory, &request("drew", "status", &ops))
                .unwrap_err()
                .code(),
            "not_namespace_owner"
        );
        assert_eq!(
            admit(&primary, &Directory, &request("drew", "nowhere", &ops))
                .unwrap_err()
                .code(),
            "unknown_namespace"
        );
        let mut bad = request("drew", "build", &ops);
        bad.group_id = "Not a group".into();
        assert_eq!(
            admit(&primary, &Directory, &bad).unwrap_err().code(),
            "malformed_group"
        );
        // None of the refusals wrote a grant.
        assert!(grants(&primary, "repo-code").unwrap().is_empty());
        let grant = admit(&primary, &Directory, &request("drew", "build", &ops)).unwrap();
        assert_eq!(grant.allowed_ops, vec![GrantOp::Read, GrantOp::Write]);
        assert_eq!(grant.policy_epoch, 1);
        // Re-admitting changes the ops and bumps the epoch.
        let widened = admit(
            &primary,
            &Directory,
            &request(
                "drew",
                "build",
                &[GrantOp::Delete, GrantOp::Read, GrantOp::Write],
            ),
        )
        .unwrap();
        assert_eq!(widened.policy_epoch, 2);
        assert!(widened.allows(GrantOp::Delete));
    }

    #[test]
    fn central_grants_no_grant_means_shared_primary_and_unknown_group_refuses() {
        let (primary, databases, build) = setup();
        assert_eq!(
            place(&primary, None, "build", GrantOp::Write).unwrap(),
            Placement::SharedPrimary
        );
        let receipt = write(
            &primary,
            &databases,
            None,
            &build,
            &intent("build", "k", Op::Set(b"v".to_vec()), "1"),
            &mut no_sleep(),
        )
        .unwrap();
        assert!(matches!(
            receipt.outcome,
            central::Outcome::Committed { version: 1 }
        ));
        assert!(
            head(&primary, &Address::new("build-cache", "build", "k"))
                .unwrap()
                .is_some()
        );
        // The group's database was never touched.
        let group_db = databases.database("code-build").unwrap();
        assert!(
            head(group_db, &Address::new("build-cache", "build", "k"))
                .unwrap()
                .is_none()
        );
        let refused = write(
            &primary,
            &databases,
            Some("repo-code"),
            &build,
            &intent("build", "k2", Op::Set(b"v".to_vec()), "2"),
            &mut no_sleep(),
        )
        .unwrap_err();
        assert_eq!(refused.code(), "unknown_group");
    }

    #[test]
    fn central_grants_group_writes_land_only_in_the_granted_database() {
        let (primary, databases, build) = setup();
        admit(
            &primary,
            &Directory,
            &request("drew", "build", &[GrantOp::Read, GrantOp::Write]),
        )
        .unwrap();
        write(
            &primary,
            &databases,
            Some("repo-code"),
            &build,
            &intent("build", "k", Op::Set(b"group".to_vec()), "1"),
            &mut no_sleep(),
        )
        .unwrap();
        let address = Address::new("build-cache", "build", "k");
        let in_group = read(&primary, &databases, Some("repo-code"), &address)
            .unwrap()
            .unwrap();
        assert_eq!(in_group.value, b"group");
        assert!(head(&primary, &address).unwrap().is_none());
        // Not granted: another namespace, or an op the grant lacks.
        assert_eq!(
            place(&primary, Some("repo-code"), "status", GrantOp::Write)
                .unwrap_err()
                .code(),
            "namespace_not_granted"
        );
        assert_eq!(
            write(
                &primary,
                &databases,
                Some("repo-code"),
                &build,
                &intent("build", "k", Op::Tombstone, "2"),
                &mut no_sleep(),
            )
            .unwrap_err()
            .code(),
            "op_not_granted"
        );
        // Revoked: the group is unknown again.
        revoke(&primary, "repo-code", "code-build", "build").unwrap();
        assert_eq!(
            read(&primary, &databases, Some("repo-code"), &address)
                .unwrap_err()
                .code(),
            "unknown_group"
        );
    }

    #[test]
    fn central_grants_group_databases_are_inline_only() {
        let (primary, databases, build) = setup();
        admit(
            &primary,
            &Directory,
            &request("drew", "build", &[GrantOp::Write]),
        )
        .unwrap();
        let refused = write(
            &primary,
            &databases,
            Some("repo-code"),
            &build,
            &intent("build", "big", Op::Set(vec![0; 9]), "1"),
            &mut no_sleep(),
        )
        .unwrap_err();
        assert_eq!(refused.code(), "blob_requires_shared_primary");
        let address = Address::new("build-cache", "build", "big");
        assert!(
            head(databases.database("code-build").unwrap(), &address)
                .unwrap()
                .is_none()
        );
        // Never moved to the shared primary on the caller's behalf.
        assert!(head(&primary, &address).unwrap().is_none());
        // At the threshold it is inline and fine.
        write(
            &primary,
            &databases,
            Some("repo-code"),
            &build,
            &intent("build", "fits", Op::Set(vec![0; 8]), "2"),
            &mut no_sleep(),
        )
        .unwrap();
    }

    #[test]
    fn central_grants_unprovisioned_database_refuses() {
        let (primary, _, build) = setup();
        admit(
            &primary,
            &Directory,
            &request("drew", "build", &[GrantOp::Write]),
        )
        .unwrap();
        let refused = write(
            &primary,
            &NoGroupDatabases,
            Some("repo-code"),
            &build,
            &intent("build", "k", Op::Set(b"v".to_vec()), "1"),
            &mut no_sleep(),
        )
        .unwrap_err();
        assert_eq!(refused.code(), "group_db_unprovisioned");
    }
}
