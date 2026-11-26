//! Offline contract layer. Oracle: the shared §10 case inventory and §5.1/§6 outcomes.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use id_pg::{HttpResponse, PostgrestRequest, RegisteredEndpoint, Transport, TransportError};
use support::{FAULTS, Fault, HEALTHY_BODY, assert_fault, assert_healthy, response};

struct FixtureTransport {
    fault: Option<Fault>,
}

impl Transport for FixtureTransport {
    fn get(
        &mut self,
        _endpoint: &RegisteredEndpoint,
        _request: &PostgrestRequest,
    ) -> Result<HttpResponse, TransportError> {
        match self.fault {
            Some(fault) => fault.response(),
            None => Ok(response(
                200,
                "application/json; charset=utf-8",
                HEALTHY_BODY.as_bytes(),
            )),
        }
    }
}

#[test]
fn injectable_transport_maps_every_declared_pg_type() {
    let mut transport = FixtureTransport { fault: None };
    assert_healthy("http://127.0.0.1", &mut transport);
}

#[test]
fn injectable_transport_matches_the_shared_failure_inventory() {
    for fault in FAULTS {
        let mut transport = FixtureTransport { fault: Some(fault) };
        assert_fault("http://127.0.0.1", &mut transport, fault);
    }
}
