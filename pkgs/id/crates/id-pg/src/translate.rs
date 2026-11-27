//! The translator: query object + allowlist → PostgREST request, or a
//! refusal by name (design §3).
//!
//! The check order implements the precedence documented in the crate
//! root: method, target, count, pagination kind, boolean structure,
//! embeds, relation, fields, operators, values, cursor, limit.

use crate::allowlist::{CollectionAllowlist, is_plain_identifier, is_usable_field_name};
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
    /// Ordering columns the query's projection left out. They are
    /// appended to `select` only so the reader can build the keyset
    /// cursor from each row; the reader removes them from the rows it
    /// returns, so the caller sees exactly the projection it asked
    /// for (#15010). Empty for a star projection, which already
    /// carries every declared column.
    pub cursor_only: Vec<String>,
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
    ///
    /// This constructor trusts that the collections were validated
    /// when they were built (registration, #13758); `translate`
    /// still refuses any declared name that is not a usable field
    /// name before it can reach a request. Prefer
    /// [`Translator::try_new`] when the allowlists come straight
    /// from configuration.
    pub fn new(collections: Vec<CollectionAllowlist>) -> Self {
        Translator { collections }
    }

    /// Build a translator, refusing the whole set when any
    /// collection's allowlist fails
    /// [`CollectionAllowlist::validate`]: a relation that is not a
    /// plain identifier, a declared column that is not a usable
    /// field name, an empty unique key, or a unique-key entry that
    /// is not declared. A bad
    /// configuration is refused here, at construction, so it never
    /// reaches a request.
    pub fn try_new(collections: Vec<CollectionAllowlist>) -> Result<Self, String> {
        for collection in &collections {
            collection.validate()?;
        }
        Ok(Translator { collections })
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
        if !is_plain_identifier(relation) {
            return Err(Refusal::not_available());
        }
        let collection = self
            .collections
            .iter()
            .find(|c| &c.relation == relation)
            .ok_or_else(Refusal::not_available)?;
        // 7b. A collection with an empty unique key has no total
        // ordering, so it cannot serve a stable read: without this
        // guard the legacy unchecked constructor path would emit an
        // empty `order=` and an empty keyset `or=()` (#14795). The
        // refusal collapses to `not_available`, like an unlisted
        // relation, so no configuration detail leaks.
        if collection.unique_key.is_empty() {
            return Err(Refusal::not_available());
        }
        // 8. Fields: projection, filter, and order columns must be
        // declared for the use they are put to. An undeclared column
        // and a withheld column are the same refusal (§6.1). A filter
        // column also becomes a query-parameter key, so a reserved
        // PostgREST parameter name or a non-identifier is refused here.
        let projection = resolve_projection(collection, &query.select)?;
        for leaf in &leaves {
            if !collection.is_filterable(&leaf.column) || !is_usable_field_name(&leaf.column) {
                return Err(Refusal::shape(Feature::FieldUnsupported));
            }
        }
        for key in &query.order {
            if !collection.is_orderable(&key.column) || !is_usable_field_name(&key.column) {
                return Err(Refusal::shape(Feature::FieldUnsupported));
            }
        }
        // The unique key is appended to every ordering by
        // `effective_ordering`, unchecked by the loops above; a key
        // column that is not a usable field name would be rendered
        // raw into the `order` parameter and the keyset `or` group.
        for key in &collection.unique_key {
            if !is_usable_field_name(key) {
                return Err(Refusal::shape(Feature::FieldUnsupported));
            }
        }
        // 9. Operators: the text-pattern family is refused by name.
        for leaf in &leaves {
            if leaf.op.is_text_pattern() {
                return Err(Refusal::shape(Feature::TextPatternUnsupported));
            }
        }
        // 10. Values and rendering of the filter parameters. The
        // effective ordering (total by construction: see
        // `effective_ordering`) decides which columns each row must
        // carry for its keyset cursor; any the projection left out are
        // fetched as cursor-only columns (#15010).
        let ordering = effective_ordering(collection, &query.order);
        let (sent, cursor_only) = with_cursor_columns(projection, &ordering);
        let mut params: Vec<(String, String)> = vec![("select".to_string(), sent.join(","))];
        for leaf in &leaves {
            params.push((leaf.column.clone(), render_leaf_value(leaf)?));
        }
        // 11. Ordering and the keyset cursor over the effective
        // ordering.
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
            cursor_only,
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

/// The columns to send in `select`: the resolved projection, then
/// every effective-ordering column it does not already name, in
/// ordering order. The second list is those appended columns alone,
/// which the reader strips from returned rows. Ordering columns are
/// declared (checked in step 8 and by the allowlist's unique-key
/// rule), so appending one never projects an undeclared column.
fn with_cursor_columns(
    mut projection: Vec<String>,
    ordering: &[OrderKey],
) -> (Vec<String>, Vec<String>) {
    let mut cursor_only = Vec::new();
    for key in ordering {
        if !projection.contains(&key.column) {
            projection.push(key.column.clone());
            cursor_only.push(key.column.clone());
        }
    }
    (projection, cursor_only)
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

/// Expand a star (or empty) projection to the declared columns,
/// refusing when any declared name is not a usable field name: the
/// names are joined raw into the `select` value, so a declared
/// column such as `a,b` or `x(y)` would change the shape of the
/// select list the source parses. Defense in depth behind
/// [`CollectionAllowlist::validate`], for allowlists that were
/// never validated at construction.
fn expand_declared(collection: &CollectionAllowlist) -> Result<Vec<String>, Refusal> {
    let columns = declared_columns(collection);
    if columns.iter().any(|name| !is_usable_field_name(name)) {
        return Err(Refusal::shape(Feature::FieldUnsupported));
    }
    Ok(columns)
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
        return expand_declared(collection);
    }
    let mut columns: Vec<String> = Vec::new();
    for item in select {
        match item {
            SelectItem::Column(name) => {
                if !collection.is_declared(name) || !is_usable_field_name(name) {
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
            SelectItem::All => return expand_declared(collection),
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
///
/// NULL placement is pinned explicitly to PostgreSQL's defaults:
/// ascending keys sort nulls last, descending keys sort them first.
/// Pinning the defaults keeps the server's row order exactly as before
/// (a bare `asc`/`desc` already meant this) while giving the keyset
/// continuation one fixed oracle to match (#16132).
fn render_order(ordering: &[OrderKey]) -> String {
    ordering
        .iter()
        .map(|key| {
            let (direction, placement) = match key.direction {
                Direction::Asc => ("asc", "nullslast"),
                Direction::Desc => ("desc", "nullsfirst"),
            };
            format!("{}.{direction}.{placement}", key.column)
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
    let Some(first) = ordering.first() else {
        return Err(Refusal::shape(Feature::FieldUnsupported));
    };
    let mut branches: Vec<String> = Vec::new();
    for j in 0..ordering.len() {
        let mut prefix: Vec<String> = Vec::with_capacity(j + 1);
        for (key, value) in ordering.iter().zip(cursor.iter()).take(j) {
            prefix.push(match value {
                Value::Null => format!("{}.is.null", key.column),
                _ => format!("{}.eq.{}", key.column, value.quoted_literal()?),
            });
        }
        let key = &ordering[j];
        let steps: Vec<String> = match (&cursor[j], key.direction) {
            // #16132: `NULL` sorts as one bucket per the pinned placement
            // (`ASC` puts it last, `DESC` first), and every `NULL` in one
            // bucket compares equal. A plain `eq` prefix already excludes
            // `NULL` rows, since `NULL = v` is unknown.
            //
            // - `ASC` (nulls last), non-`NULL` cursor: rows after the
            //   cursor are greater values plus the whole `NULL` bucket.
            // - `ASC`, `NULL` cursor: nothing sorts after the last bucket.
            // - `DESC` (nulls first), non-`NULL` cursor: rows after the
            //   cursor are smaller values alone (`NULL` rows sort before
            //   it, not after).
            // - `DESC`, `NULL` cursor: every non-`NULL` row sorts after
            //   the first bucket.
            (Value::Null, Direction::Asc) => Vec::new(),
            (Value::Null, Direction::Desc) => vec![format!("{}.not.is.null", key.column)],
            (value, Direction::Asc) => vec![
                format!("{}.gt.{}", key.column, value.quoted_literal()?),
                format!("{}.is.null", key.column),
            ],
            (value, Direction::Desc) => {
                vec![format!("{}.lt.{}", key.column, value.quoted_literal()?)]
            }
        };
        for step in steps {
            let mut branch = prefix.clone();
            branch.push(step);
            branches.push(if branch.len() == 1 {
                branch.pop().expect("one branch part")
            } else {
                format!("and({})", branch.join(","))
            });
        }
    }
    // A cursor that is `NULL` in every ascending key sits at the very
    // end of the ordering, so its continuation is empty. Emit it as a
    // contradiction on the first ordering column (which always exists:
    // the effective ordering ends with the unique key) rather than as
    // an empty `or=()`, which PostgREST would reject as malformed.
    if branches.is_empty() {
        return Ok(format!(
            "and({}.is.null,{}.not.is.null)",
            first.column, first.column
        ));
    }
    Ok(format!("({})", branches.join(",")))
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
    fn projection_without_the_unique_key_fetches_it_cursor_only() {
        // Oracle: #15010 — the reader builds the keyset cursor from
        // every effective-ordering column, so a projection that leaves
        // the unique key out still has it sent, marked cursor-only.
        let mut query = Query::read("items");
        query.select = vec![SelectItem::Column("title".to_string())];
        let request = translator().translate(&query).unwrap();
        assert_eq!(param(&request, "select"), Some("title,id"));
        assert_eq!(param(&request, "order"), Some("id.asc.nullslast"));
        assert_eq!(request.cursor_only, vec!["id".to_string()]);
    }

    #[test]
    fn projection_without_an_order_key_fetches_it_cursor_only() {
        let mut query = Query::read("items");
        query.select = vec![SelectItem::Column("body".to_string())];
        query.order = vec![OrderKey {
            column: "title".to_string(),
            direction: Direction::Desc,
        }];
        let request = translator().translate(&query).unwrap();
        assert_eq!(param(&request, "select"), Some("body,title,id"));
        assert_eq!(
            param(&request, "order"),
            Some("title.desc.nullsfirst,id.asc.nullslast")
        );
        assert_eq!(
            request.cursor_only,
            vec!["title".to_string(), "id".to_string()]
        );
    }

    #[test]
    fn projection_naming_every_ordering_column_has_no_cursor_only_columns() {
        let mut query = Query::read("items");
        query.select = vec![
            SelectItem::Column("title".to_string()),
            SelectItem::Column("id".to_string()),
        ];
        let request = translator().translate(&query).unwrap();
        assert_eq!(param(&request, "select"), Some("title,id"));
        assert_eq!(request.cursor_only, [] as [std::string::String; 0]);
        let star = translator().translate(&Query::read("items")).unwrap();
        assert_eq!(star.cursor_only, [] as [std::string::String; 0]);
    }

    #[test]
    fn plain_read_expands_to_declared_columns_and_defaults() {
        let request = translator().translate(&Query::read("items")).unwrap();
        assert_eq!(request.method, "GET");
        assert_eq!(request.path, "/items");
        assert_eq!(request.accept_profile, "public");
        assert_eq!(param(&request, "select"), Some("id,title,body"));
        assert_eq!(param(&request, "order"), Some("id.asc.nullslast"));
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
        assert_eq!(
            param(&request, "order"),
            Some("title.desc.nullsfirst,id.asc.nullslast")
        );
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
            Some("(title.lt.m,and(title.eq.m,id.gt.42),and(title.eq.m,id.is.null))")
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

    fn leaf_query(column: &str) -> Query {
        let mut query = Query::read("items");
        query.filter = Some(FilterExpr::Leaf(FilterLeaf {
            column: column.to_string(),
            op: FilterOp::Eq,
            value: Value::Int(1),
        }));
        query
    }

    fn collection_with_columns(names: &[&str]) -> Translator {
        let mut collection = collection();
        collection.columns = names
            .iter()
            .map(|name| ColumnDecl {
                name: name.to_string(),
                filterable: true,
                orderable: true,
            })
            .collect();
        Translator::new(vec![collection])
    }

    #[test]
    fn filter_on_reserved_parameter_name_is_refused() {
        let translator = collection_with_columns(&["id", "or", "select"]);
        for name in ["or", "select"] {
            let refusal = translator.translate(&leaf_query(name)).unwrap_err();
            assert_eq!(refusal, Refusal::shape(Feature::FieldUnsupported));
        }
    }

    #[test]
    fn order_on_reserved_parameter_name_is_refused() {
        let translator = collection_with_columns(&["id", "limit"]);
        let mut query = Query::read("items");
        query.order = vec![OrderKey {
            column: "limit".to_string(),
            direction: Direction::Asc,
        }];
        let refusal = translator.translate(&query).unwrap_err();
        assert_eq!(refusal, Refusal::shape(Feature::FieldUnsupported));
    }

    #[test]
    fn filter_on_non_identifier_column_is_refused() {
        let translator = collection_with_columns(&["id", "a,b"]);
        let refusal = translator.translate(&leaf_query("a,b")).unwrap_err();
        assert_eq!(refusal, Refusal::shape(Feature::FieldUnsupported));
    }

    #[test]
    fn unsafe_relation_name_collapses_to_not_available() {
        let mut collection = collection();
        collection.relation = "items/x".to_string();
        let translator = Translator::new(vec![collection]);
        let refusal = translator.translate(&Query::read("items/x")).unwrap_err();
        assert_eq!(refusal, Refusal::not_available());
    }

    #[test]
    fn plain_filter_column_still_translates() {
        let request = translator().translate(&leaf_query("title")).unwrap();
        assert_eq!(param(&request, "title"), Some("eq.1"));
    }

    #[test]
    fn star_projection_refuses_unsafe_declared_column() {
        let translator = collection_with_columns(&["id", "x(y)"]);
        let refusal = translator.translate(&Query::read("items")).unwrap_err();
        assert_eq!(refusal, Refusal::shape(Feature::FieldUnsupported));
    }

    #[test]
    fn explicit_projection_refuses_unsafe_declared_column() {
        let translator = collection_with_columns(&["id", "x(y)"]);
        let mut query = Query::read("items");
        query.select = vec![SelectItem::Column("x(y)".to_string())];
        let refusal = translator.translate(&query).unwrap_err();
        assert_eq!(refusal, Refusal::shape(Feature::FieldUnsupported));
    }

    #[test]
    fn try_new_refuses_unvalidated_allowlist() {
        let mut raw = collection();
        raw.columns = vec![ColumnDecl {
            name: "a,b".to_string(),
            filterable: true,
            orderable: true,
        }];
        assert!(Translator::try_new(vec![raw]).is_err());
    }

    #[test]
    fn try_new_accepts_valid_allowlist() {
        assert!(Translator::try_new(vec![collection()]).is_ok());
    }

    #[test]
    fn try_new_refuses_empty_unique_key() {
        // Oracle: #14795 acceptance — the translator boundary refuses
        // an allowlist whose unique key is empty.
        let mut raw = collection();
        raw.unique_key = vec![];
        assert!(Translator::try_new(vec![raw]).is_err());
    }

    #[test]
    fn empty_unique_key_never_produces_empty_order_or_keyset_or() {
        // Oracle: #14795 acceptance — even through the legacy
        // unchecked `Translator::new` path, a keyless collection
        // produces no request at all (so no `order=` and no
        // `or=()`), for both a plain read and a keyset read.
        let mut raw = collection();
        raw.unique_key = vec![];
        let translator = Translator::new(vec![raw]);
        let refusal = translator.translate(&Query::read("items")).unwrap_err();
        assert_eq!(refusal, Refusal::not_available());
        let mut keyset = Query::read("items");
        keyset.pagination = Pagination::Keyset { cursor: vec![] };
        let refusal = translator.translate(&keyset).unwrap_err();
        assert_eq!(refusal, Refusal::not_available());
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

    #[test]
    fn order_pins_postgres_null_placement() {
        // Oracle: PostgreSQL defaults — ASC sorts NULLS LAST, DESC
        // sorts NULLS FIRST. The adapter pins them explicitly so the
        // server's row order and the continuation share one oracle.
        let mut query = Query::read("items");
        query.order = vec![
            OrderKey {
                column: "title".to_string(),
                direction: Direction::Asc,
            },
            OrderKey {
                column: "id".to_string(),
                direction: Direction::Desc,
            },
        ];
        let request = translator().translate(&query).unwrap();
        assert_eq!(
            param(&request, "order"),
            Some("title.asc.nullslast,id.desc.nullsfirst")
        );
    }

    fn score_collection() -> CollectionAllowlist {
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
                    name: "score".to_string(),
                    filterable: true,
                    orderable: true,
                },
            ],
            unique_key: vec!["id".to_string()],
            max_limit: Some(50),
        }
    }

    fn score_translator() -> Translator {
        Translator::new(vec![score_collection()])
    }

    fn score_request(order: Vec<OrderKey>, cursor: Vec<Value>) -> PostgrestRequest {
        let mut query = Query::read("items");
        query.order = order;
        query.pagination = Pagination::Keyset { cursor };
        score_translator().translate(&query).unwrap()
    }

    fn asc(column: &str) -> OrderKey {
        OrderKey {
            column: column.to_string(),
            direction: Direction::Asc,
        }
    }

    fn desc(column: &str) -> OrderKey {
        OrderKey {
            column: column.to_string(),
            direction: Direction::Desc,
        }
    }

    #[test]
    fn keyset_continuation_after_non_null_asc_includes_the_null_bucket() {
        // #16132 defect case: a cursor of (score=10, id=1) used to emit
        // only `score.gt.10`, silently dropping every NULL-score row and
        // claiming completeness.
        let request = score_request(
            vec![asc("score"), asc("id")],
            vec![Value::Int(10), Value::Int(1)],
        );
        assert_eq!(
            param(&request, "or"),
            Some(
                "(score.gt.10,score.is.null,\
                 and(score.eq.10,id.gt.1),and(score.eq.10,id.is.null))"
            )
        );
    }

    #[test]
    fn keyset_continuation_after_null_asc_stays_inside_the_null_bucket() {
        // A NULL cursor names the NULL bucket: only NULL-score rows with
        // greater tiebreak ids follow.
        let request = score_request(
            vec![asc("score"), asc("id")],
            vec![Value::Null, Value::Int(5)],
        );
        assert_eq!(
            param(&request, "or"),
            Some("(and(score.is.null,id.gt.5),and(score.is.null,id.is.null))")
        );
    }

    #[test]
    fn keyset_continuation_desc_pages_around_the_null_bucket() {
        // DESC is NULLS FIRST: after (score=10, id=1) come only smaller
        // scores — NULL rows sort before the cursor, not after it.
        let request = score_request(
            vec![desc("score"), asc("id")],
            vec![Value::Int(10), Value::Int(1)],
        );
        assert_eq!(
            param(&request, "or"),
            Some("(score.lt.10,and(score.eq.10,id.gt.1),and(score.eq.10,id.is.null))")
        );
        // After (NULL, 5): every non-NULL row sorts after the first
        // bucket, plus the greater NULL-bucket ids.
        let request = score_request(
            vec![desc("score"), asc("id")],
            vec![Value::Null, Value::Int(5)],
        );
        assert_eq!(
            param(&request, "or"),
            Some(
                "(score.not.is.null,\
                 and(score.is.null,id.gt.5),and(score.is.null,id.is.null))"
            )
        );
    }

    #[test]
    fn keyset_continuation_past_the_final_null_bucket_is_empty_not_invalid() {
        // An all-NULL ascending cursor is the end of the ordering; the
        // continuation matches nothing but stays valid PostgREST rather
        // than an empty `or=()` or a refusal.
        let request = score_request(
            vec![asc("score"), asc("id")],
            vec![Value::Null, Value::Null],
        );
        assert_eq!(
            param(&request, "or"),
            Some("and(score.is.null,score.not.is.null)")
        );
    }

    // #16132 regression harness: a PostgREST-lite row store. It sorts
    // rows per the request's `order=` value and filters per its `or=`
    // value under PostgreSQL comparison semantics (three-valued logic,
    // NULL buckets per the pinned placement) — exactly what the real
    // source does with the emitted strings. The oracle is tuple order:
    // walking pages of size 1 must surface every row exactly once, in
    // full database order, ending on an honestly complete empty page.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct LiteRow {
        id: i64,
        score: Option<i64>,
    }

    fn lite_value(row: &LiteRow, column: &str) -> Option<i64> {
        match column {
            "id" => Some(row.id),
            "score" => row.score,
            _ => panic!("undeclared column {column}"),
        }
    }

    /// Split a comma list at depth zero, so `and(a,b),c` yields its two
    /// top-level branches.
    fn split_top_level(value: &str) -> Vec<&str> {
        let mut parts = Vec::new();
        let mut depth = 0usize;
        let mut start = 0usize;
        for (i, c) in value.char_indices() {
            match c {
                '(' => depth += 1,
                ')' => depth -= 1,
                ',' if depth == 0 => {
                    parts.push(&value[start..i]);
                    start = i + 1;
                }
                _ => {}
            }
        }
        parts.push(&value[start..]);
        parts
    }

    fn lite_predicate(row: &LiteRow, predicate: &str) -> bool {
        let (column, rest) = predicate.split_once('.').expect("predicate names a column");
        let value = lite_value(row, column);
        if rest == "is.null" {
            return value.is_none();
        }
        if rest == "not.is.null" {
            return value.is_some();
        }
        let (op, raw) = rest
            .split_once('.')
            .expect("comparison has an operator and a value");
        let literal: i64 = raw.parse().expect("integer literal");
        match (op, value) {
            // SQL: any comparison against NULL is unknown, never true.
            (_, None) => false,
            (op, Some(seen)) => match op {
                "eq" => seen == literal,
                "gt" => seen > literal,
                "lt" => seen < literal,
                _ => panic!("unexpected operator {op}"),
            },
        }
    }

    fn lite_branch(row: &LiteRow, branch: &str) -> bool {
        let inner = branch
            .strip_prefix("and(")
            .and_then(|s| s.strip_suffix(')'))
            .unwrap_or(branch);
        split_top_level(inner)
            .iter()
            .all(|predicate| lite_predicate(row, predicate))
    }

    fn lite_sort(rows: &mut [LiteRow], order: &str) {
        let mut keys: Vec<(&str, bool, bool)> = Vec::new();
        for term in order.split(',') {
            let mut parts = term.split('.');
            let column = parts.next().expect("order term names a column");
            let direction = parts.next().expect("order term has a direction");
            let placement = parts.next().expect("order term pins null placement");
            assert!(parts.next().is_none(), "order term has no extra: {term}");
            keys.push((
                column,
                match direction {
                    "asc" => true,
                    "desc" => false,
                    _ => panic!("unexpected direction {direction}"),
                },
                match placement {
                    "nullslast" => true,
                    "nullsfirst" => false,
                    _ => panic!("unexpected placement {placement}"),
                },
            ));
        }
        rows.sort_by(|a, b| {
            keys.iter()
                .map(|(column, ascending, nulls_last)| {
                    // NULL placement is absolute (NULLS FIRST means
                    // first regardless of direction); only value
                    // comparisons reverse with the direction.
                    match (lite_value(a, column), lite_value(b, column)) {
                        (None, None) => std::cmp::Ordering::Equal,
                        (None, _) => {
                            if *nulls_last {
                                std::cmp::Ordering::Greater
                            } else {
                                std::cmp::Ordering::Less
                            }
                        }
                        (_, None) => {
                            if *nulls_last {
                                std::cmp::Ordering::Less
                            } else {
                                std::cmp::Ordering::Greater
                            }
                        }
                        (Some(x), Some(y)) => {
                            let ord = x.cmp(&y);
                            if *ascending { ord } else { ord.reverse() }
                        }
                    }
                })
                .find(|ord| *ord != std::cmp::Ordering::Equal)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    }

    /// Walk the table one row per page through translated requests,
    /// returning the visited ids and the page count (including the
    /// final empty page that marks the result complete).
    fn lite_walk(order: Vec<OrderKey>, select: Vec<SelectItem>) -> (Vec<i64>, usize) {
        let table = vec![
            LiteRow {
                id: 1,
                score: Some(10),
            },
            LiteRow { id: 2, score: None },
            LiteRow {
                id: 3,
                score: Some(10),
            },
            LiteRow { id: 4, score: None },
            LiteRow {
                id: 5,
                score: Some(3),
            },
        ];
        let effective = effective_ordering(&score_collection(), &order);
        let mut seen = Vec::new();
        let mut pages = 0usize;
        let mut pagination = Pagination::None;
        loop {
            let mut query = Query::read("items");
            query.select = select.clone();
            query.order = order.clone();
            query.pagination = pagination.clone();
            query.limit = Some(1);
            let request = score_translator()
                .translate(&query)
                .expect("walk query translates");
            let order_param = param(&request, "order")
                .expect("walk request carries order")
                .to_string();
            let filter = param(&request, "or").map(str::to_string);
            let mut rows = table.clone();
            lite_sort(&mut rows, &order_param);
            if let Some(condition) = filter {
                let group = condition
                    .strip_prefix('(')
                    .and_then(|s| s.strip_suffix(')'))
                    .expect("keyset condition is parenthesized");
                rows.retain(|row| {
                    split_top_level(group)
                        .iter()
                        .any(|branch| lite_branch(row, branch))
                });
            }
            pages += 1;
            match rows.first() {
                // A short page ends the walk complete, like the reader.
                None => return (seen, pages),
                Some(row) => {
                    seen.push(row.id);
                    pagination = Pagination::Keyset {
                        cursor: effective
                            .iter()
                            .map(|key| match key.column.as_str() {
                                "id" => Value::Int(row.id),
                                "score" => row.score.map_or(Value::Null, Value::Int),
                                column => panic!("undeclared column {column}"),
                            })
                            .collect(),
                    };
                }
            }
        }
    }

    #[test]
    fn keyset_walk_over_nullable_ordering_is_lossless_asc() {
        // Page size 1 across NULL and non-NULL scores, duplicate
        // scores, and multiple NULL rows: concatenated pages must equal
        // full database order exactly once, ending complete.
        let (seen, pages) = lite_walk(vec![asc("score")], vec![]);
        assert_eq!(seen, vec![5, 1, 3, 2, 4]);
        assert_eq!(pages, 6);
    }

    #[test]
    fn keyset_walk_over_nullable_ordering_is_lossless_desc() {
        let (seen, pages) = lite_walk(vec![desc("score")], vec![]);
        assert_eq!(seen, vec![2, 4, 1, 3, 5]);
        assert_eq!(pages, 6);
    }

    #[test]
    fn keyset_walk_with_projected_out_ordering_column_is_lossless() {
        // The ordering column is projected out, so it travels only as a
        // cursor-only column (#15010); the walk must still be lossless.
        let mut query = Query::read("items");
        query.select = vec![SelectItem::Column("id".to_string())];
        query.order = vec![asc("score")];
        let request = score_translator().translate(&query).unwrap();
        assert_eq!(param(&request, "select"), Some("id,score"));
        assert_eq!(request.cursor_only, vec!["score".to_string()]);
        let (seen, pages) = lite_walk(
            vec![asc("score")],
            vec![SelectItem::Column("id".to_string())],
        );
        assert_eq!(seen, vec![5, 1, 3, 2, 4]);
        assert_eq!(pages, 6);
    }
}
