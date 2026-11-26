//! Property tests over the query object (verification plan §10):
//! for every generated query in the supported set, the translated
//! request uses only declared columns and operators, and the
//! pagination key is total. Oracle: the §3 allowlist.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;

use id_pg::{
    CollectionAllowlist, ColumnDecl, CountPref, Direction, FilterExpr, FilterLeaf, FilterOp,
    Method, OrderKey, Pagination, Query, SelectItem, Target, Translator, Value, effective_ordering,
};
use proptest::prelude::*;
use proptest::sample;
use proptest::strategy::BoxedStrategy;

const POOL: [&str; 6] = ["id", "title", "body", "score", "created_at", "owner"];

fn collection_from_masks(
    mask: u32,
    filterable_mask: u32,
    orderable_mask: u32,
    max_limit: Option<u32>,
) -> CollectionAllowlist {
    let columns: Vec<ColumnDecl> = POOL
        .iter()
        .enumerate()
        .filter(|(i, _)| mask & (1 << i) != 0)
        .map(|(i, name)| ColumnDecl {
            name: (*name).to_string(),
            filterable: filterable_mask & (1 << i) != 0,
            orderable: orderable_mask & (1 << i) != 0,
        })
        .collect();
    let unique_key: Vec<String> = columns.iter().take(2).map(|c| c.name.clone()).collect();
    CollectionAllowlist {
        schema: "public".to_string(),
        relation: "items".to_string(),
        columns,
        unique_key,
        max_limit,
    }
}

fn arb_collection() -> impl Strategy<Value = CollectionAllowlist> {
    (
        1u32..64u32,
        any::<u32>(),
        any::<u32>(),
        prop_oneof![Just(None), (1u32..=600u32).prop_map(Some)],
    )
        .prop_map(|(mask, f, o, cap)| collection_from_masks(mask, f, o, cap))
}

fn arb_text() -> impl Strategy<Value = String> {
    proptest::collection::vec(
        sample::select(vec!['a', 'b', 'Z', '0', ' ', ',', '"', '(', ')', '-', '_']),
        0..=10,
    )
    .prop_map(|chars| chars.into_iter().collect())
}

fn arb_scalar() -> impl Strategy<Value = Value> {
    prop_oneof![
        any::<bool>().prop_map(Value::Bool),
        (-1000i64..1000i64).prop_map(Value::Int),
        (-1.0e6f64..1.0e6f64).prop_map(Value::Float),
        arb_text().prop_map(Value::Text),
    ]
}

fn arb_op_value() -> impl Strategy<Value = (FilterOp, Value)> {
    prop_oneof![
        sample::select(vec![
            FilterOp::Eq,
            FilterOp::Neq,
            FilterOp::Lt,
            FilterOp::Lte,
            FilterOp::Gt,
            FilterOp::Gte,
        ])
        .prop_flat_map(|op| (Just(op), arb_scalar())),
        Just((FilterOp::IsNull, Value::Null)),
        proptest::collection::vec(arb_scalar(), 1..=4)
            .prop_map(|list| (FilterOp::In, Value::List(list))),
    ]
}

fn arb_leaf(filterable: Vec<String>) -> impl Strategy<Value = FilterExpr> {
    (sample::select(filterable), arb_op_value())
        .prop_map(|(column, (op, value))| FilterExpr::Leaf(FilterLeaf { column, op, value }))
}

fn arb_filter(filterable: Vec<String>) -> BoxedStrategy<Option<FilterExpr>> {
    if filterable.is_empty() {
        return Just(None).boxed();
    }
    prop_oneof![
        Just(None),
        arb_leaf(filterable.clone()).prop_map(Some),
        proptest::collection::vec(arb_leaf(filterable), 1..=3)
            .prop_map(|leaves| Some(FilterExpr::And(leaves))),
    ]
    .boxed()
}

fn arb_select(declared: Vec<String>) -> BoxedStrategy<Vec<SelectItem>> {
    prop_oneof![
        Just(Vec::new()),
        Just(vec![SelectItem::All]),
        proptest::collection::vec(sample::select(declared).prop_map(SelectItem::Column), 1..=3),
    ]
    .boxed()
}

fn arb_order(orderable: Vec<String>) -> BoxedStrategy<Vec<OrderKey>> {
    if orderable.is_empty() {
        return Just(Vec::new()).boxed();
    }
    proptest::collection::vec((sample::select(orderable), any::<bool>()), 0..=3)
        .prop_map(|pairs| {
            pairs
                .into_iter()
                .map(|(column, asc)| OrderKey {
                    column,
                    direction: if asc { Direction::Asc } else { Direction::Desc },
                })
                .collect()
        })
        .boxed()
}

fn arb_pagination(effective_len: usize) -> impl Strategy<Value = Pagination> {
    prop_oneof![
        Just(Pagination::None),
        proptest::collection::vec(arb_scalar(), effective_len..=effective_len)
            .prop_map(|cursor| Pagination::Keyset { cursor }),
    ]
}

fn arb_limit(cap: u32) -> impl Strategy<Value = Option<u32>> {
    prop_oneof![Just(None), (0u32..=cap).prop_map(Some)]
}

/// Generate a (collection, query) pair where the query is entirely
/// inside the §3.1 supported set for that collection.
fn arb_supported() -> impl Strategy<Value = (CollectionAllowlist, Query)> {
    arb_collection().prop_flat_map(|collection| {
        let declared: Vec<String> = collection.columns.iter().map(|c| c.name.clone()).collect();
        let filterable: Vec<String> = collection
            .columns
            .iter()
            .filter(|c| c.filterable)
            .map(|c| c.name.clone())
            .collect();
        let orderable: Vec<String> = collection
            .columns
            .iter()
            .filter(|c| c.orderable)
            .map(|c| c.name.clone())
            .collect();
        (
            Just(collection),
            arb_select(declared),
            arb_filter(filterable),
            arb_order(orderable),
        )
            .prop_flat_map(|(collection, select, filter, order)| {
                let effective_len = effective_ordering(&collection, &order).len();
                let cap = collection.effective_max_limit();
                (
                    Just(collection),
                    Just(select),
                    Just(filter),
                    Just(order),
                    arb_pagination(effective_len),
                    arb_limit(cap),
                )
            })
            .prop_map(|(collection, select, filter, order, pagination, limit)| {
                let query = Query {
                    method: Method::Get,
                    target: Target::Collection(collection.relation.clone()),
                    select,
                    filter,
                    order,
                    pagination,
                    limit,
                    count: CountPref::None,
                };
                (collection, query)
            })
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    #[test]
    fn supported_queries_translate_within_the_allowlist(
        (collection, query) in arb_supported(),
    ) {
        let translator = Translator::new(vec![collection.clone()]);
        let request = translator
            .translate(&query)
            .expect("a generated supported query must translate");

        // One GET against the one relation, in the one schema.
        prop_assert_eq!(request.method, "GET");
        prop_assert_eq!(&request.path, &format!("/{}", collection.relation));
        prop_assert_eq!(&request.accept_profile, &collection.schema);

        let declared: BTreeSet<&str> = collection
            .declared_column_names()
            .into_iter()
            .collect();
        let filterable: BTreeSet<&str> = collection
            .columns
            .iter()
            .filter(|c| c.filterable)
            .map(|c| c.name.as_str())
            .collect();

        let mut order_columns: Vec<String> = Vec::new();
        for (key, value) in &request.params {
            match key.as_str() {
                "select" => {
                    // Only declared columns are ever projected.
                    prop_assert!(!value.is_empty());
                    for name in value.split(',') {
                        prop_assert!(declared.contains(name), "undeclared select: {name}");
                    }
                }
                "order" => {
                    for term in value.split(',') {
                        let (column, direction) = term
                            .rsplit_once('.')
                            .expect("order term is column.direction");
                        prop_assert!(declared.contains(column));
                        prop_assert!(direction == "asc" || direction == "desc");
                        order_columns.push(column.to_string());
                    }
                }
                "limit" => {
                    let limit: u32 = value.parse().expect("limit is a number");
                    prop_assert!(limit <= collection.effective_max_limit());
                }
                "or" => {
                    // The keyset condition is parenthesized and
                    // mentions every effective ordering column.
                    prop_assert!(value.starts_with('(') && value.ends_with(')'));
                    for key in effective_ordering(&collection, &query.order) {
                        prop_assert!(
                            value.contains(&format!("{}.", key.column)),
                            "cursor condition misses ordering column {}",
                            key.column
                        );
                    }
                }
                column => {
                    // A filter parameter: declared + filterable column,
                    // closed operator set, never an offset.
                    prop_assert!(filterable.contains(column), "filter on {column}");
                    prop_assert!(
                        value.starts_with("eq.")
                            || value.starts_with("neq.")
                            || value.starts_with("lt.")
                            || value.starts_with("lte.")
                            || value.starts_with("gt.")
                            || value.starts_with("gte.")
                            || value.starts_with("in.(")
                            || value.as_str() == "is.null",
                        "unsupported operator rendering: {value}"
                    );
                }
            }
        }

        // Totality: the emitted ordering is exactly the effective
        // ordering, it has no duplicate columns, and it contains
        // every unique-key column — so the tuple order is total.
        // (A unique-key column the query already orders by keeps its
        // earlier position; it is not appended a second time.)
        let effective = effective_ordering(&collection, &query.order);
        let effective_columns: Vec<String> =
            effective.iter().map(|k| k.column.clone()).collect();
        prop_assert_eq!(&order_columns, &effective_columns);
        let distinct: BTreeSet<&String> = order_columns.iter().collect();
        prop_assert_eq!(distinct.len(), order_columns.len());
        for column in &collection.unique_key {
            prop_assert!(order_columns.contains(column));
        }

        // Determinism: the same query translates to the same request.
        let again = translator.translate(&query).expect("still translates");
        prop_assert_eq!(&request, &again);
    }

    #[test]
    fn undeclared_filter_column_is_always_field_unsupported(
        collection in arb_collection(),
        outsider in sample::select(vec!["salary", "secret", "ssn", "password"]),
    ) {
        // None of the outsider names is in the generator's column
        // pool, so each is undeclared for every generated collection.
        let mut query = Query::read(&collection.relation);
        query.filter = Some(FilterExpr::Leaf(FilterLeaf {
            column: outsider.to_string(),
            op: FilterOp::Eq,
            value: Value::Int(1),
        }));
        let translator = Translator::new(vec![collection]);
        let refusal = translator.translate(&query).expect_err("must be refused");
        prop_assert_eq!(
            refusal.body(),
            "{\"code\":\"query_shape_refused\",\"feature\":\"field_unsupported\"}"
        );
    }
}
