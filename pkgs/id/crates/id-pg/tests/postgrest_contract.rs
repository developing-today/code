//! Real PostgREST/PostgreSQL contract tests. Ignored in ordinary local runs;
//! CI opts in explicitly and missing ID_PG_POSTGREST_URL is a hard failure.
//! Oracles: design #5805 §5.1 and §6.1–§6.2, shared with postgrest_unit.rs.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::env;

use support::{CurlTransport, FAULTS, Fault, assert_fault, assert_healthy, start_non_json_server};

fn base_url() -> String {
    env::var("ID_PG_POSTGREST_URL").expect("CI contract runner must provide ID_PG_POSTGREST_URL")
}

#[test]
#[ignore = "requires the CI PostgreSQL + PostgREST fixture; run with --ignored"]
fn real_postgrest_maps_every_declared_pg_type() {
    let mut transport = CurlTransport::default();
    assert_healthy(&base_url(), &mut transport);
}

#[test]
#[ignore = "requires the CI PostgreSQL + PostgREST fixture; run with --ignored"]
fn real_postgrest_matches_the_shared_failure_inventory() {
    let base = base_url();
    for fault in FAULTS {
        let mut transport = CurlTransport::default();
        if matches!(fault, Fault::Unauthorized) {
            transport.invalid_jwt = true;
        }
        if matches!(fault, Fault::Deadline) {
            transport.timeout_seconds = 1;
        }
        let url = if matches!(fault, Fault::NonJson) {
            start_non_json_server()
        } else {
            base.clone()
        };
        assert_fault(&url, &mut transport, fault);
    }
}
