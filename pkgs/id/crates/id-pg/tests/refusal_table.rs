//! Model tests over the design's §3.2 refusal table (verification
//! plan §10): each refused shape maps to its stable feature name, and
//! an undeclared column and a withheld column produce identical error
//! bodies (oracle: the §6 table).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use id_pg::{
    CollectionAllowlist, ColumnDecl, CountPref, Direction, FilterExpr, FilterLeaf, FilterOp,
    Method, OrderKey, Pagination, Query, Refusal, RefusalCode, SelectItem, Target, Translator,
    Value,
};

fn collection(max_limit: Option<u32>) -> CollectionAllowlist {
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
        max_limit,
    }
}

fn translator() -> Translator {
    Translator::new(vec![collection(Some(50))])
}

fn refusal(translator: &Translator, query: &Query) -> Refusal {
    translator
        .translate(query)
        .expect_err("query must be refused")
}

fn leaf(column: &str, op: FilterOp, value: Value) -> FilterExpr {
    FilterExpr::Leaf(FilterLeaf {
        column: column.to_string(),
        op,
        value,
    })
}

fn assert_shape(translator: &Translator, query: &Query, feature: &str) {
    let refusal = refusal(translator, query);
    assert_eq!(refusal.code, RefusalCode::QueryShapeRefused);
    assert_eq!(
        refusal.body(),
        format!("{{\"code\":\"query_shape_refused\",\"feature\":\"{feature}\"}}")
    );
}

#[test]
fn writes_are_refused_by_name() {
    for method in [Method::Post, Method::Patch, Method::Put, Method::Delete] {
        let mut query = Query::read("items");
        query.method = method;
        assert_shape(&translator(), &query, "write_unsupported");
    }
}

#[test]
fn rpc_is_refused_by_name() {
    let mut query = Query::read("items");
    query.target = Target::Rpc("do_anything".to_string());
    assert_shape(&translator(), &query, "rpc_unsupported");
}

#[test]
fn discovery_is_refused_by_name() {
    let mut query = Query::read("items");
    query.target = Target::Root;
    assert_shape(&translator(), &query, "discovery_unsupported");
}

#[test]
fn raw_sql_is_refused_by_name() {
    let mut query = Query::read("items");
    query.target = Target::Sql;
    assert_shape(&translator(), &query, "sql_unsupported");
}

#[test]
fn counts_are_refused_by_name() {
    for count in [CountPref::Exact, CountPref::Planned, CountPref::Estimated] {
        let mut query = Query::read("items");
        query.count = count;
        assert_shape(&translator(), &query, "count_unsupported");
    }
}

#[test]
fn offset_and_page_pagination_are_refused_by_name() {
    for pagination in [
        Pagination::Offset { offset: 100 },
        Pagination::Page { page: 3 },
    ] {
        let mut query = Query::read("items");
        query.pagination = pagination;
        assert_shape(&translator(), &query, "offset_unsupported");
    }
}

#[test]
fn boolean_trees_are_refused_by_name() {
    let eq = leaf("id", FilterOp::Eq, Value::Int(1));
    for filter in [
        FilterExpr::Or(vec![eq.clone(), eq.clone()]),
        FilterExpr::Not(Box::new(eq.clone())),
        FilterExpr::And(vec![
            eq,
            FilterExpr::Or(vec![leaf("id", FilterOp::Eq, Value::Int(2))]),
        ]),
    ] {
        let mut query = Query::read("items");
        query.filter = Some(filter);
        assert_shape(&translator(), &query, "boolean_unsupported");
    }
}

#[test]
fn embeds_are_refused_by_name() {
    let mut query = Query::read("items");
    query.select = vec![
        SelectItem::Column("id".to_string()),
        SelectItem::Embed {
            relation: "owner".to_string(),
            items: vec![SelectItem::All],
        },
    ];
    assert_shape(&translator(), &query, "join_unsupported");
}

#[test]
fn undeclared_and_unusable_fields_are_refused_by_name() {
    // Filtering on a column that was never declared.
    let mut query = Query::read("items");
    query.filter = Some(leaf("salary", FilterOp::Eq, Value::Int(1)));
    assert_shape(&translator(), &query, "field_unsupported");

    // Filtering on a declared column that is not filterable.
    let mut query = Query::read("items");
    query.filter = Some(leaf("body", FilterOp::Eq, Value::Text("x".to_string())));
    assert_shape(&translator(), &query, "field_unsupported");

    // Ordering on a declared column that is not orderable.
    let mut query = Query::read("items");
    query.order = vec![OrderKey {
        column: "body".to_string(),
        direction: Direction::Asc,
    }];
    assert_shape(&translator(), &query, "field_unsupported");

    // Projecting a column that was never declared.
    let mut query = Query::read("items");
    query.select = vec![SelectItem::Column("salary".to_string())];
    assert_shape(&translator(), &query, "field_unsupported");
}

#[test]
fn text_patterns_are_refused_by_name() {
    for op in [
        FilterOp::Like,
        FilterOp::ILike,
        FilterOp::Match,
        FilterOp::Fts,
        FilterOp::PlFts,
        FilterOp::Wfts,
    ] {
        let mut query = Query::read("items");
        query.filter = Some(leaf("title", op, Value::Text("abc".to_string())));
        assert_shape(&translator(), &query, "text_pattern_unsupported");
    }
}

#[test]
fn limit_above_cap_is_refused_not_clamped() {
    let mut query = Query::read("items");
    query.limit = Some(51);
    let over = refusal(&translator(), &query);
    assert_eq!(over.code, RefusalCode::LimitExceeded);
    assert_eq!(
        over.body(),
        "{\"code\":\"limit_exceeded\",\"feature\":\"limit_exceeded\"}"
    );

    // At the cap is fine.
    let mut query = Query::read("items");
    query.limit = Some(50);
    assert!(translator().translate(&query).is_ok());

    // The default cap is 500 when the owner declares none.
    let default_cap = Translator::new(vec![collection(None)]);
    let mut query = Query::read("items");
    query.limit = Some(501);
    let over_default = refusal(&default_cap, &query);
    assert_eq!(over_default.code, RefusalCode::LimitExceeded);
    let mut query = Query::read("items");
    query.limit = Some(500);
    assert!(default_cap.translate(&query).is_ok());
}

#[test]
fn unknown_relation_collapses_to_not_available() {
    let query = Query::read("secrets");
    let refusal = refusal(&translator(), &query);
    assert_eq!(refusal.code, RefusalCode::NotAvailable);
    assert_eq!(refusal.feature, None);
    assert_eq!(refusal.body(), "{\"code\":\"not_available\"}");
}

#[test]
fn withheld_and_unknown_columns_produce_identical_bodies() {
    // The translator only ever sees the allowlist, so a column the
    // owner withheld from it and a column that does not exist at all
    // are the same input — and §6.2 requires the same body. The body
    // must not even carry the column's name.
    let mut withheld = Query::read("items");
    withheld.filter = Some(leaf("salary", FilterOp::Eq, Value::Int(1)));
    let mut unknown = Query::read("items");
    unknown.filter = Some(leaf("never_existed", FilterOp::Eq, Value::Int(1)));
    let withheld_body = refusal(&translator(), &withheld).body();
    let unknown_body = refusal(&translator(), &unknown).body();
    assert_eq!(withheld_body, unknown_body);
    assert!(!withheld_body.contains("salary"));
    assert!(!unknown_body.contains("never_existed"));
}

#[test]
fn refusal_precedence_is_deterministic() {
    // Method is checked before target: a POST to an RPC target is a
    // write refusal, not an RPC refusal.
    let mut query = Query::read("items");
    query.method = Method::Post;
    query.target = Target::Rpc("f".to_string());
    assert_shape(&translator(), &query, "write_unsupported");

    // Target is checked before count.
    let mut query = Query::read("items");
    query.target = Target::Rpc("f".to_string());
    query.count = CountPref::Exact;
    assert_shape(&translator(), &query, "rpc_unsupported");
}

#[test]
fn a_fully_supported_query_translates() {
    let mut query = Query::read("items");
    query.select = vec![
        SelectItem::Column("id".to_string()),
        SelectItem::Column("title".to_string()),
    ];
    query.filter = Some(FilterExpr::And(vec![
        leaf("id", FilterOp::Gte, Value::Int(10)),
        leaf(
            "title",
            FilterOp::In,
            Value::List(vec![
                Value::Text("a".to_string()),
                Value::Text("b".to_string()),
            ]),
        ),
    ]));
    query.order = vec![OrderKey {
        column: "title".to_string(),
        direction: Direction::Asc,
    }];
    query.pagination = Pagination::Keyset {
        cursor: vec![Value::Text("a".to_string()), Value::Int(10)],
    };
    query.limit = Some(25);
    let request = translator().translate(&query).unwrap();
    assert_eq!(request.method, "GET");
    assert_eq!(request.path, "/items");
    assert_eq!(request.accept_profile, "public");
    assert!(
        request
            .params
            .iter()
            .any(|(k, v)| k == "select" && v == "id,title")
    );
    assert!(
        request
            .params
            .iter()
            .any(|(k, v)| k == "order" && v == "title.asc.nullslast,id.asc.nullslast")
    );
    assert!(
        request
            .params
            .iter()
            .any(|(k, v)| k == "limit" && v == "25")
    );
}
