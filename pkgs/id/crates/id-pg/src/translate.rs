//! The translator: query object + allowlist → PostgREST request, or a
//! refusal by name (design §3).
//!
//! The check order implements the precedence documented in the crate
//! root: method, target, count, pagination kind, boolean structure,
//! embeds, relation, fields, operators, values, cursor, limit.

use crate::allowlist::CollectionAllowlist;
use crate::query::{
    CountPref, Direction, FilterExpr, FilterLeaf, FilterOp, Method, OrderKey, Pagination, Query,
    SelectItem, Target,
};
use crate::refusal::{Feature, Refusal};
use crate::value::Value;

/// A PostgREST request, as data. The transport (#13763) turns this
/// into an actual HTTP call; nothing here performs I/O.
///
/// The request is always a `GET` against one relation in the
/// endpoint's one exposed schema. There is no field for a body, a
/// write method, or a `Prefer` header, so a write or a count cannot
/// be expressed in the output any more than in the rules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostgrestRequest {
    /// The HTTP method: always `"GET"`.
    pub method: &'static str,
    /// The request path: `/` followed by the relation name.
    pub path: String,
    /// The value for the `Accept-Profile` header: the collection's
    /// one exposed schema (design §2, schema pinning).
    pub accept_profile: String,
    /// The query parameters, in emission order: `select`, one
    /// parameter per filter leaf, the keyset `or` parameter when
    /// paging, `order`, and `limit`.
    pub params: Vec<(String, String)>,
}

/// The query translator, holding the owner's collection allowlists.
#[derive(Debug, Clone)]
pub struct Translator {
    collections: Vec<CollectionAllowlist>,
}

impl Translator {
    /// Build a translator over the owner's allowlist of collections.
    /// The adapter exposes exactly these collections; a query naming
    /// any other relation is `not_available`.
    pub fn new(collections: Vec<CollectionAllowlist>) -> Self {
        Translator { collections }
    }

    /// Translate one query. Every §3.2 refusal is returned before any
    /// request exists; a successful result is the whole request.
    pub fn translate(&self, query: &Query) -> Result<PostgrestRequest, Refusal> {
        // 1. Method: anything but GET is a write.
        if query.method != Method::Get {
            return Err(Refusal::shape(Feature::WriteUnsupported));
        }
        // 2. Target: only a collection read is a v1 shape.
        let relation = match &query.target {
            Target::Collection(name) => name,
            Target::Rpc(_) => return Err(Refusal::shape(Feature::RpcUnsupported)),
            Target::Root => return Err(Refusal::shape(Feature::DiscoveryUnsupported)),
            Target::Sql => return Err(Refusal::shape(Feature::SqlUnsupported)),
        };
        // 3. Counts disclose the size of a withheld set.
        if query.count != CountPref::None {
            return Err(Refusal::shape(Feature::CountUnsupported));
        }
        // 4. Pagination kind: keyset only.
        match &query.pagination {
            Pagination::Offset { .. } | Pagination::Page { .. } => {
                return Err(Refusal::shape(Feature::OffsetUnsupported));
            }
            Pagination::None | Pagination::Keyset { .. } => {}
        }
        // 5. Filter structure: a flat conjunction of leaves.
        let mut leaves: Vec<&FilterLeaf> = Vec::new();
        if let Some(filter) = &query.filter {
            flatten_filter(filter, &mut leaves)?;
        }
        // 6. Embeds are joins, wherever they nest in the projection.
        if select_contains_embed(&query.select) {
            return Err(Refusal::shape(Feature::JoinUnsupported));
        }
        // 7. The relation must be one the owner listed. Unknown and
        // unlisted collapse to the same outcome.
        let collection = self
            .collections
            .iter()
            .find(|c| &c.relation == relation)
            .ok_or_else(Refusal::not_available)?;
        // 8. Fields: projection, filter, and order columns must be
        // declared for the use they are put to. An undeclared column
        // and a withheld column are the same refusal (§6.1).
        let projection = resolve_projection(collection, &query.select)?;
        for leaf in &leaves {
            if !collection.is_filterable(&leaf.column) {
                return Err(Refusal::shape(Feature::FieldUnsupported));
            }
        }
        for key in &query.order {
            if !collection.is_orderable(&key.column) {
                return Err(Refusal::shape(Feature::FieldUnsupported));
            }
        }
        // 9. Operators: the text-pattern family is refused by name.
        for leaf in &leaves {
            if leaf.op.is_text_pattern() {
                return Err(Refusal::shape(Feature::TextPatternUnsupported));
            }
        }
        // 10. Values and rendering of the filter parameters.
        let mut params: Vec<(String, String)> = vec![("select".to_string(), projection.join(","))];
        for leaf in &leaves {
            params.push((leaf.column.clone(), render_leaf_value(leaf)?));
        }
        // 11. Ordering and the keyset cursor over the effective
        // ordering (total by construction: see `effective_ordering`).
        let ordering = effective_ordering(collection, &query.order);
        if let Pagination::Keyset { cursor } = &query.pagination {
            params.push(("or".to_string(), keyset_or(&ordering, cursor)?));
        }
        params.push(("order".to_string(), render_order(&ordering)));
        // 12. Limit: above the cap is refused, never clamped.
        let cap = collection.effective_max_limit();
        if let Some(limit) = query.limit {
            if limit > cap {
                return Err(Refusal::limit_exceeded());
            }
        }
        params.push(("limit".to_string(), query.limit.unwrap_or(cap).to_string()));
        Ok(PostgrestRequest {
            method: "GET",
            path: format!("/{relation}"),
            accept_profile: collection.schema.clone(),
            params,
        })
    }
}

/// The effective ordering for a query: the query's ordering keys
/// (deduplicated by column, first occurrence wins) with the
/// collection's declared unique key appended, ascending, as the final
/// tiebreak. The result is always total — it ends with the unique key
/// — which is what makes keyset pagination stable (design §3.1).
pub fn effective_ordering(collection: &CollectionAllowlist, order: &[OrderKey]) -> Vec<OrderKey> {
    let mut effective: Vec<OrderKey> = Vec::new();
    for key in order {
        if !effective.iter().any(|k| k.column == key.column) {
            effective.push(key.clone());
        }
    }
    for column in &collection.unique_key {
        if !effective.iter().any(|k| &k.column == column) {
            effective.push(OrderKey {
                column: column.clone(),
                direction: Direction::Asc,
            });
        }
    }
    effective
}

/// Flatten a filter expression into its leaves. Only conjunctions
/// flatten; `Or` and `Not` are `boolean_unsupported` wherever they
/// appear in the tree.
fn flatten_filter<'a>(expr: &'a FilterExpr, out: &mut Vec<&'a FilterLeaf>) -> Result<(), Refusal> {
    match expr {
        FilterExpr::Leaf(leaf) => {
            out.push(leaf);
            Ok(())
        }
        FilterExpr::And(parts) => {
            for part in parts {
                flatten_filter(part, out)?;
            }
            Ok(())
        }
        FilterExpr::Or(_) | FilterExpr::Not(_) => Err(Refusal::shape(Feature::BooleanUnsupported)),
    }
}

/// Whether any element of the projection is an embed. Embeds nest
/// only inside embeds, so an embed at any depth means a top-level
/// embed exists.
fn select_contains_embed(items: &[SelectItem]) -> bool {
    items
        .iter()
        .any(|item| matches!(item, SelectItem::Embed { .. }))
}

/// The collection's declared columns as owned strings, in
/// declaration order.
fn declared_columns(collection: &CollectionAllowlist) -> Vec<String> {
    collection
        .declared_column_names()
        .into_iter()
        .map(str::to_string)
        .collect()
}

/// Resolve the projection to the declared column list to send.
///
/// An empty projection, or one containing `All`, expands to the
/// collection's declared columns in declaration order — the star is
/// never forwarded, so a column added to the source later is not
/// exposed by accident. An explicit list keeps the query's order,
/// deduplicated; every name must be declared.
fn resolve_projection(
    collection: &CollectionAllowlist,
    select: &[SelectItem],
) -> Result<Vec<String>, Refusal> {
    if select.is_empty() || select.iter().any(|item| matches!(item, SelectItem::All)) {
        return Ok(declared_columns(collection));
    }
    let mut columns: Vec<String> = Vec::new();
    for item in select {
        match item {
            SelectItem::Column(name) => {
                if !collection.is_declared(name) {
                    return Err(Refusal::shape(Feature::FieldUnsupported));
                }
                if !columns.contains(name) {
                    columns.push(name.clone());
                }
            }
            // Embeds were refused before this point; a stray one is
            // still a join, never a column.
            SelectItem::Embed { .. } => {
                return Err(Refusal::shape(Feature::JoinUnsupported));
            }
            // Handled by the early return above; expand again rather
            // than assume.
            SelectItem::All => return Ok(declared_columns(collection)),
        }
    }
    Ok(columns)
}

/// Render one filter leaf's value as a PostgREST parameter value:
/// `op.value`, `in.(...)`, or `is.null`.
fn render_leaf_value(leaf: &FilterLeaf) -> Result<String, Refusal> {
    match leaf.op {
        FilterOp::IsNull => Ok("is.null".to_string()),
        FilterOp::In => {
            let Value::List(elements) = &leaf.value else {
                return Err(Refusal::shape(Feature::FieldUnsupported));
            };
            if elements.is_empty() {
                return Err(Refusal::shape(Feature::FieldUnsupported));
            }
            let mut rendered = Vec::with_capacity(elements.len());
            for element in elements {
                rendered.push(element.quoted_literal()?);
            }
            Ok(format!("in.({})", rendered.join(",")))
        }
        op => {
            let token = op
                .postgrest_token()
                .ok_or_else(|| Refusal::shape(Feature::TextPatternUnsupported))?;
            Ok(format!("{token}.{}", leaf.value.literal()?))
        }
    }
}

/// Render the `order` parameter value over the effective ordering.
fn render_order(ordering: &[OrderKey]) -> String {
    ordering
        .iter()
        .map(|key| {
            let direction = match key.direction {
                Direction::Asc => "asc",
                Direction::Desc => "desc",
            };
            format!("{}.{direction}", key.column)
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// Build the keyset cursor condition as a PostgREST `or=(...)`
/// parameter value: the lexicographic "strictly after the cursor
/// tuple" condition over the effective ordering.
///
/// This `or` group is generated by the adapter from the declared
/// ordering and the cursor alone; it is not a user boolean expression
/// (those are `boolean_unsupported`). For ordering keys
/// `k0..kn` with cursor values `v0..vn`, the terms are
/// `k0 <cmp0> v0`, `and(k0.eq.v0, k1 <cmp1> v1)`, … where `<cmp>` is
/// `gt` for ascending keys and `lt` for descending ones.
fn keyset_or(ordering: &[OrderKey], cursor: &[Value]) -> Result<String, Refusal> {
    if cursor.len() != ordering.len() {
        return Err(Refusal::shape(Feature::FieldUnsupported));
    }
    let mut terms: Vec<String> = Vec::with_capacity(ordering.len());
    for j in 0..ordering.len() {
        let mut parts: Vec<String> = Vec::with_capacity(j + 1);
        for (i, key) in ordering.iter().enumerate().take(j) {
            parts.push(format!("{}.eq.{}", key.column, cursor[i].quoted_literal()?));
        }
        let cmp = match ordering[j].direction {
            Direction::Asc => "gt",
            Direction::Desc => "lt",
        };
        parts.push(format!(
            "{}.{cmp}.{}",
            ordering[j].column,
            cursor[j].quoted_literal()?
        ));
        if parts.len() == 1 {
            terms.push(parts.remove(0));
        } else {
            terms.push(format!("and({})", parts.join(",")));
        }
    }
    Ok(format!("({})", terms.join(",")))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::allowlist::ColumnDecl;
    use crate::query::FilterExpr;

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
                ColumnDecl {
                    name: "body".to_string(),
                    filterable: false,
                    orderable: false,
                },
            ],
            unique_key: vec!["id".to_string()],
            max_limit: Some(50),
        }
    }

    fn translator() -> Translator {
        Translator::new(vec![collection()])
    }

    fn param<'a>(request: &'a PostgrestRequest, key: &str) -> Option<&'a str> {
        request
            .params
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    #[test]
    fn plain_read_expands_to_declared_columns_and_defaults() {
        let request = translator().translate(&Query::read("items")).unwrap();
        assert_eq!(request.method, "GET");
        assert_eq!(request.path, "/items");
        assert_eq!(request.accept_profile, "public");
        assert_eq!(param(&request, "select"), Some("id,title,body"));
        assert_eq!(param(&request, "order"), Some("id.asc"));
        assert_eq!(param(&request, "limit"), Some("50"));
    }

    #[test]
    fn tiebreak_appended_after_declared_ordering() {
        let mut query = Query::read("items");
        query.order = vec![OrderKey {
            column: "title".to_string(),
            direction: Direction::Desc,
        }];
        let request = translator().translate(&query).unwrap();
        assert_eq!(param(&request, "order"), Some("title.desc,id.asc"));
    }

    #[test]
    fn keyset_cursor_renders_lexicographic_or() {
        let mut query = Query::read("items");
        query.order = vec![OrderKey {
            column: "title".to_string(),
            direction: Direction::Desc,
        }];
        query.pagination = Pagination::Keyset {
            cursor: vec![Value::Text("m".to_string()), Value::Int(42)],
        };
        let request = translator().translate(&query).unwrap();
        assert_eq!(
            param(&request, "or"),
            Some("(title.lt.m,and(title.eq.m,id.gt.42))")
        );
    }

    #[test]
    fn in_list_quotes_text_with_commas() {
        let mut query = Query::read("items");
        query.filter = Some(FilterExpr::Leaf(FilterLeaf {
            column: "title".to_string(),
            op: FilterOp::In,
            value: Value::List(vec![
                Value::Text("a,b".to_string()),
                Value::Text("plain".to_string()),
            ]),
        }));
        let request = translator().translate(&query).unwrap();
        assert_eq!(param(&request, "title"), Some("in.(\"a,b\",plain)"));
    }

    #[test]
    fn is_null_renders_without_value() {
        let mut query = Query::read("items");
        query.filter = Some(FilterExpr::Leaf(FilterLeaf {
            column: "title".to_string(),
            op: FilterOp::IsNull,
            value: Value::Null,
        }));
        let request = translator().translate(&query).unwrap();
        assert_eq!(param(&request, "title"), Some("is.null"));
    }
}
