//! The owner's collection allowlist (design §3.1, §4 layer 1).
//!
//! A collection is the grantable unit: the adapter exposes exactly the
//! collections listed here, with exactly the columns declared here.
//! A column that exists in the source but is not declared is *withheld*
//! — and, by the §6 collapse rule, indistinguishable from a column
//! that does not exist at all: both are `field_unsupported`.
//!
//! Registration and owner configuration (#13758, `registration.rs`)
//! checks these values, with the rest of the endpoint configuration,
//! against the live endpoint before any query runs.

use serde::{Deserialize, Serialize};

/// The per-collection row cap applied when the owner declares none
/// (design §3.1: "default 500 rows").
pub const DEFAULT_MAX_LIMIT: u32 = 500;

/// Query-parameter names PostgREST reads as directives or logic
/// operators. A filter or order column with one of these names would be
/// emitted as a parameter key PostgREST interprets as something else.
pub const RESERVED_PARAM_NAMES: &[&str] = &[
    "select",
    "order",
    "limit",
    "offset",
    "or",
    "and",
    "not",
    "columns",
    "on_conflict",
];

/// Whether `name` is a plain ASCII identifier (letters, digits,
/// underscore, not starting with a digit). Anything else would be
/// interpolated raw into a path or a query value.
pub fn is_plain_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphabetic() || first == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Whether `name` may be used as a filter or order column: a plain
/// identifier that is not a reserved PostgREST parameter name.
pub fn is_usable_field_name(name: &str) -> bool {
    is_plain_identifier(name) && !RESERVED_PARAM_NAMES.contains(&name)
}

/// One declared column of a collection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColumnDecl {
    /// The column name as PostgREST addresses it.
    pub name: String,
    /// Whether queries may filter on this column.
    pub filterable: bool,
    /// Whether queries may order by this column.
    pub orderable: bool,
}

/// The allowlist for one collection (one table or view).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CollectionAllowlist {
    /// The exposed schema the endpoint was registered with. Sent as
    /// the request's `Accept-Profile`; a request naming any other
    /// schema is refused by construction, since only this value is
    /// ever emitted.
    pub schema: String,
    /// The relation name (table or view) within `schema`.
    pub relation: String,
    /// The declared columns, in declaration order. Projection `*` and
    /// an empty projection both expand to this list, in this order —
    /// never to whatever the source currently has.
    pub columns: Vec<ColumnDecl>,
    /// The declared unique key: column names, all of which must be
    /// declared columns. Appended to every ordering as the final
    /// tiebreak so the ordering is total and keyset pagination is
    /// stable.
    pub unique_key: Vec<String>,
    /// The declared per-collection row cap. `None` applies
    /// [`DEFAULT_MAX_LIMIT`].
    pub max_limit: Option<u32>,
}

impl CollectionAllowlist {
    /// The row cap in force for this collection.
    pub fn effective_max_limit(&self) -> u32 {
        self.max_limit.unwrap_or(DEFAULT_MAX_LIMIT)
    }

    /// Check that every name this allowlist would hand to the
    /// translator is safe to interpolate into a PostgREST request.
    ///
    /// Rejected, with the reason named:
    ///
    /// - a relation that is not a plain identifier — the relation is
    ///   interpolated raw into the request path (`/{relation}`);
    /// - a declared column name that is not a usable field name (not
    ///   a plain identifier, or a reserved PostgREST parameter name
    ///   such as `or` or `select`) — declared names are joined raw
    ///   into the `select` value, and filter/order columns also
    ///   become query-parameter keys, so a name containing `,`, `(`,
    ///   `)`, `"`, or whitespace would change the shape of the
    ///   `select` list the source parses;
    /// - an empty `unique_key` — the key is the tiebreak that makes
    ///   every ordering total; with no key there is no stable
    ///   ordering, so keyset pagination cannot keep its contract
    ///   (#14795);
    /// - a `unique_key` entry that is not one of the declared
    ///   columns — the key is appended to every ordering and rendered
    ///   into the `order` parameter and the keyset `or` group.
    ///
    /// Registration (#13758) calls this when an allowlist is built,
    /// so a bad configuration is refused at construction and never
    /// reaches a request.
    pub fn validate(&self) -> Result<(), String> {
        if !is_plain_identifier(&self.relation) {
            return Err(format!(
                "relation {:?} is not a plain identifier",
                self.relation
            ));
        }
        for column in &self.columns {
            if !is_usable_field_name(&column.name) {
                return Err(format!(
                    "declared column {:?} is not a usable field name",
                    column.name
                ));
            }
        }
        if self.unique_key.is_empty() {
            return Err(
                "unique_key is empty: a collection needs a declared unique key \
                 so its ordering is total and keyset pagination is stable"
                    .to_string(),
            );
        }
        for key in &self.unique_key {
            if !self.is_declared(key) {
                return Err(format!("unique_key entry {key:?} is not a declared column"));
            }
        }
        Ok(())
    }

    /// Look up a declared column by name.
    pub fn column(&self, name: &str) -> Option<&ColumnDecl> {
        self.columns.iter().find(|c| c.name == name)
    }

    /// Whether `name` is a declared column at all.
    pub fn is_declared(&self, name: &str) -> bool {
        self.column(name).is_some()
    }

    /// Whether `name` may be filtered on (declared and filterable).
    pub fn is_filterable(&self, name: &str) -> bool {
        self.column(name).is_some_and(|c| c.filterable)
    }

    /// Whether `name` may be ordered by (declared and orderable).
    pub fn is_orderable(&self, name: &str) -> bool {
        self.column(name).is_some_and(|c| c.orderable)
    }

    /// The declared column names, in declaration order.
    pub fn declared_column_names(&self) -> Vec<&str> {
        self.columns.iter().map(|c| c.name.as_str()).collect()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn collection() -> CollectionAllowlist {
        CollectionAllowlist {
            schema: "public".to_string(),
            relation: "items".to_string(),
            columns: vec![
                ColumnDecl {
                    name: "id".to_string(),
                    filterable: true,
                    orderable: true,
                },
                ColumnDecl {
                    name: "title".to_string(),
                    filterable: true,
                    orderable: true,
                },
            ],
            unique_key: vec!["id".to_string()],
            max_limit: None,
        }
    }

    fn with_column(name: &str) -> CollectionAllowlist {
        let mut collection = collection();
        collection.columns.push(ColumnDecl {
            name: name.to_string(),
            filterable: true,
            orderable: true,
        });
        collection
    }

    #[test]
    fn validate_accepts_plain_declared_columns() {
        assert_eq!(collection().validate(), Ok(()));
    }

    #[test]
    fn validate_refuses_column_with_comma() {
        let collection = with_column("a,b");
        assert!(collection.validate().unwrap_err().contains("a,b"));
    }

    #[test]
    fn validate_refuses_reserved_column_name() {
        let collection = with_column("or");
        assert!(collection.validate().unwrap_err().contains("\"or\""));
    }

    #[test]
    fn validate_refuses_non_identifier_relation() {
        let mut collection = collection();
        collection.relation = "items/x".to_string();
        assert!(collection.validate().is_err());
    }

    #[test]
    fn validate_refuses_empty_unique_key() {
        // Oracle: #14795 — keyset pagination is only stable when the
        // effective ordering is total, and the unique key is the
        // tiebreak that makes it total.
        let mut collection = collection();
        collection.unique_key = vec![];
        assert!(collection.validate().unwrap_err().contains("unique_key"));
    }

    #[test]
    fn validate_refuses_undeclared_unique_key_entry() {
        let mut collection = collection();
        collection.unique_key = vec!["missing".to_string()];
        assert!(collection.validate().unwrap_err().contains("missing"));
    }
}
