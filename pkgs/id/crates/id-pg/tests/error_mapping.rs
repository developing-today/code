//! Model tests over the design's §6 table (design #5805, §6.1, §6.2;
//! verification plan §10): every source condition maps to its stable
//! code, the grantee collapse merges every data- or grant-dependent
//! outcome into one `not_available` body, and an unknown column and a
//! withheld column produce identical error bodies.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use id_pg::{
    Audience, CollectionAllowlist, ColumnDecl, ErrorCode, Failure, Query, Refusal, SelectItem,
    Translator, classify_response,
};

fn translator() -> Translator {
    Translator::new(vec![CollectionAllowlist {
        schema: "api".to_owned(),
        relation: "items".to_owned(),
        columns: vec![
            ColumnDecl {
                name: "id".to_owned(),
                filterable: true,
                orderable: true,
            },
            ColumnDecl {
                name: "title".to_owned(),
                filterable: true,
                orderable: true,
            },
        ],
        unique_key: vec!["id".to_owned()],
        max_limit: Some(10),
    }])
}

fn refusal_for(query: &Query) -> Refusal {
    translator().translate(query).unwrap_err()
}

fn projecting(column: &str) -> Query {
    let mut query = Query::read("items");
    query.select = vec![SelectItem::Column(column.to_owned())];
    query
}

const BOTH: [Audience; 2] = [Audience::Owner, Audience::Grantee];

/// The §6.1 table: (source condition, owner code, grantee code).
fn table() -> Vec<(Failure, ErrorCode, ErrorCode)> {
    vec![
        (
            Failure::Refused(Refusal::shape(id_pg::Feature::JoinUnsupported)),
            ErrorCode::QueryShapeRefused,
            ErrorCode::QueryShapeRefused,
        ),
        (
            Failure::Refused(Refusal::limit_exceeded()),
            ErrorCode::LimitExceeded,
            ErrorCode::LimitExceeded,
        ),
        (
            Failure::Refused(Refusal::not_available()),
            ErrorCode::NotAvailable,
            ErrorCode::NotAvailable,
        ),
        (
            Failure::Status(401),
            ErrorCode::CredentialUnavailable,
            ErrorCode::NotAvailable,
        ),
        (
            Failure::Status(403),
            ErrorCode::CredentialUnavailable,
            ErrorCode::NotAvailable,
        ),
        (
            Failure::Status(404),
            ErrorCode::NotAvailable,
            ErrorCode::NotAvailable,
        ),
        (
            Failure::Status(400),
            ErrorCode::NotAvailable,
            ErrorCode::NotAvailable,
        ),
        (
            Failure::Status(429),
            ErrorCode::RateLimited,
            ErrorCode::RateLimited,
        ),
        (
            Failure::CapacityLimit,
            ErrorCode::RateLimited,
            ErrorCode::RateLimited,
        ),
        (Failure::Deadline, ErrorCode::Timeout, ErrorCode::Timeout),
        (
            Failure::Connection,
            ErrorCode::AdapterUnavailable,
            ErrorCode::AdapterUnavailable,
        ),
        (
            Failure::Status(500),
            ErrorCode::AdapterUnavailable,
            ErrorCode::AdapterUnavailable,
        ),
        (
            Failure::Status(503),
            ErrorCode::AdapterUnavailable,
            ErrorCode::AdapterUnavailable,
        ),
        (
            Failure::Status(302),
            ErrorCode::AdapterUnavailable,
            ErrorCode::AdapterUnavailable,
        ),
        (
            Failure::NonJsonBody,
            ErrorCode::AdapterUnavailable,
            ErrorCode::AdapterUnavailable,
        ),
        (
            Failure::TypeMapping {
                column: "id".to_owned(),
                declared_type: "int8".to_owned(),
            },
            ErrorCode::TypeMappingFailed,
            ErrorCode::TypeMappingFailed,
        ),
    ]
}

#[test]
fn every_source_condition_maps_to_its_stable_code() {
    for (failure, owner, grantee) in table() {
        assert_eq!(failure.to_error(Audience::Owner).code, owner, "{failure:?}");
        assert_eq!(
            failure.to_error(Audience::Grantee).code,
            grantee,
            "{failure:?}"
        );
    }
}

#[test]
fn grantee_errors_carry_no_diagnostics() {
    for (failure, _, _) in table() {
        let error = failure.to_error(Audience::Grantee);
        assert_eq!(error.detail, None, "{failure:?}");
        assert!(!error.body().contains("detail"), "{failure:?}");
        assert!(!error.body().contains("HTTP"), "{failure:?}");
    }
}

#[test]
fn owner_sees_why_a_credential_was_refused() {
    let error = Failure::Status(401).to_error(Audience::Owner);
    assert_eq!(
        error.body(),
        r#"{"code":"credential_unavailable","detail":"source returned HTTP 401"}"#
    );
}

#[test]
fn every_grant_or_data_dependent_outcome_collapses_to_one_body() {
    // An unlisted relation (refused before the network), a relation the
    // source does not have (404), a refused credential (401/403), and any
    // other 4xx all look the same to a grantee (§6.2).
    let unlisted = refusal_for(&Query::read("secrets"));
    let expected = unlisted.body();
    assert_eq!(expected, r#"{"code":"not_available"}"#);
    for failure in [
        Failure::Refused(unlisted),
        Failure::Status(404),
        Failure::Status(401),
        Failure::Status(403),
        Failure::Status(400),
        Failure::Status(406),
        Failure::Status(416),
    ] {
        assert_eq!(
            failure.to_error(Audience::Grantee).body(),
            expected,
            "{failure:?}"
        );
    }
}

#[test]
fn unknown_and_withheld_columns_produce_identical_bodies() {
    // `secret` exists in the source but is not declared (withheld);
    // `nonexistent` does not exist at all. The adapter cannot tell them
    // apart, and neither can the grantee or the owner.
    let withheld = refusal_for(&projecting("secret"));
    let unknown = refusal_for(&projecting("nonexistent"));
    for audience in BOTH {
        let a = Failure::Refused(withheld.clone()).to_error(audience);
        let b = Failure::Refused(unknown.clone()).to_error(audience);
        assert_eq!(a.body(), b.body());
        assert_eq!(
            a.body(),
            r#"{"code":"query_shape_refused","feature":"field_unsupported"}"#
        );
    }
}

#[test]
fn query_text_refusals_are_shown_unchanged() {
    for refusal in [
        refusal_for(&projecting("nonexistent")),
        Refusal::limit_exceeded(),
        Refusal::shape(id_pg::Feature::CountUnsupported),
    ] {
        for audience in BOTH {
            assert_eq!(
                Failure::Refused(refusal.clone()).to_error(audience).body(),
                refusal.body()
            );
        }
    }
}

#[test]
fn type_mapping_names_column_and_type_but_not_the_value() {
    let failure = Failure::TypeMapping {
        column: "id".to_owned(),
        declared_type: "int8".to_owned(),
    };
    for audience in BOTH {
        assert_eq!(
            failure.to_error(audience).body(),
            r#"{"code":"type_mapping_failed","column":"id","type":"int8"}"#
        );
    }
}

#[test]
fn a_non_json_success_is_an_error_not_an_empty_result() {
    assert_eq!(classify_response(200, true), Ok(()));
    assert_eq!(classify_response(206, true), Ok(()));
    assert_eq!(classify_response(200, false), Err(Failure::NonJsonBody));
    assert_eq!(classify_response(404, true), Err(Failure::Status(404)));
    assert_eq!(classify_response(500, false), Err(Failure::Status(500)));
    assert_eq!(classify_response(301, false), Err(Failure::Status(301)));
}

#[test]
fn status_mapping_is_total_and_never_succeeds() {
    // Every non-2xx status maps to an error for both audiences, and a
    // grantee never sees a code that depends on the grant.
    for status in 100..=599_u16 {
        if (200..=299).contains(&status) {
            continue;
        }
        let grantee = Failure::Status(status).to_error(Audience::Grantee);
        assert_ne!(grantee.code, ErrorCode::CredentialUnavailable, "{status}");
        assert_eq!(grantee.detail, None, "{status}");
        let owner = Failure::Status(status).to_error(Audience::Owner);
        assert!(owner.detail.is_some(), "{status}");
    }
}
