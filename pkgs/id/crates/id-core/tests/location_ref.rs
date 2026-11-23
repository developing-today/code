//! Golden-vector and property tests for typed location values
//! (slice A9115-locator-types, issue #10200; design A-9115.2,
//! statements S3.21, S3.22, S3.26–S3.30).
//!
//! Oracle: **invariant** — typed enum reference model; no coercion
//! between locator kinds and no scope-less locator accepted —
//! checked over generated inputs, with the exact failure
//! classification asserted for unknown-kind, malformed, empty-field
//! and missing-scope inputs.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use id_core::location_ref::{
    DriveLocator, LaptopLocator, LocationError, LocationRef, LocatorKind, R2Locator, TypedLocator,
};
use id_core::namespace_definition::NamespaceId;
use proptest::prelude::*;

// ---------- golden vectors ----------

#[test]
fn golden_tagged_forms() {
    let r2 =
        TypedLocator::from(R2Locator::new("acct123", "atlas-blobs", "chunks/ab/cdef").unwrap());
    assert_eq!(
        r2.to_tagged_string(),
        "r2/acct123/atlas-blobs/chunks/ab/cdef"
    );
    assert_eq!(r2.kind(), LocatorKind::R2);
    assert_eq!(
        TypedLocator::parse_tagged(&r2.to_tagged_string()).unwrap(),
        r2
    );

    let laptop = TypedLocator::from(LaptopLocator::new("amd-6", "/home/user/id/file.bin").unwrap());
    assert_eq!(
        laptop.to_tagged_string(),
        "laptop/amd-6/home/user/id/file.bin"
    );
    assert_eq!(laptop.kind(), LocatorKind::Laptop);
    assert_eq!(
        TypedLocator::parse_tagged(&laptop.to_tagged_string()).unwrap(),
        laptop
    );

    let drive = TypedLocator::from(
        DriveLocator::new(
            "team-drive",
            "1AbC_dEf",
            "https://drive.google.com/file/d/1AbC_dEf/view",
        )
        .unwrap(),
    );
    assert_eq!(
        drive.to_tagged_string(),
        "drive/team-drive/1AbC_dEf/https://drive.google.com/file/d/1AbC_dEf/view"
    );
    assert_eq!(drive.kind(), LocatorKind::Drive);
    assert_eq!(
        TypedLocator::parse_tagged(&drive.to_tagged_string()).unwrap(),
        drive
    );
}

// ---------- failure classification ----------

#[test]
fn unknown_kind_tag_is_unsupported_not_guessed() {
    // A perfectly R2-shaped value under an unknown tag must not be
    // read as R2 (or anything else).
    assert_eq!(
        TypedLocator::parse_tagged("s3/acct123/atlas-blobs/chunks/ab/cdef"),
        Err(LocationError::UnsupportedLocatorKind {
            tag: "s3".to_string()
        })
    );
    assert_eq!(
        TypedLocator::parse_tagged("path/home/user/id/file.bin"),
        Err(LocationError::UnsupportedLocatorKind {
            tag: "path".to_string()
        })
    );
}

#[test]
fn malformed_tagged_forms_are_classified() {
    assert_eq!(
        TypedLocator::parse_tagged("r2only"),
        Err(LocationError::MalformedTaggedForm {
            problem: "missing kind tag"
        })
    );
    assert!(matches!(
        TypedLocator::parse_tagged("r2/acct123"),
        Err(LocationError::MalformedTaggedForm { .. })
    ));
    assert!(matches!(
        TypedLocator::parse_tagged("laptop/amd-6"),
        Err(LocationError::MalformedTaggedForm { .. })
    ));
    assert!(matches!(
        TypedLocator::parse_tagged("drive/team-drive"),
        Err(LocationError::MalformedTaggedForm { .. })
    ));
}

#[test]
fn invalid_fields_are_classified_by_field() {
    assert_eq!(
        R2Locator::new("", "bucket", "key"),
        Err(LocationError::EmptyField { field: "account" })
    );
    assert_eq!(
        R2Locator::new("acct", "", "key"),
        Err(LocationError::EmptyField { field: "bucket" })
    );
    assert_eq!(
        R2Locator::new("acct", "bucket", ""),
        Err(LocationError::EmptyField { field: "key" })
    );
    // A slash inside a segment field would corrupt the tagged form.
    assert!(matches!(
        R2Locator::new("acct", "buck/et", "key"),
        Err(LocationError::InvalidField {
            field: "bucket",
            ..
        })
    ));
    // A laptop locator with a relative path is not a location.
    assert!(matches!(
        LaptopLocator::new("amd-6", "home/user/file.bin"),
        Err(LocationError::InvalidField { field: "path", .. })
    ));
    assert_eq!(
        LaptopLocator::new("amd-6", ""),
        Err(LocationError::EmptyField { field: "path" })
    );
    // Drive URLs are https; anything else is a different kind of
    // string, not a Drive locator.
    assert!(matches!(
        DriveLocator::new("team", "file1", "http://drive.google.com/file/d/file1"),
        Err(LocationError::InvalidField { field: "url", .. })
    ));
    assert_eq!(
        DriveLocator::new("team", "", "https://drive.google.com/file/d/file1"),
        Err(LocationError::EmptyField { field: "file_id" })
    );
}

#[test]
fn scope_less_references_are_rejected() {
    let locator = TypedLocator::from(LaptopLocator::new("amd-6", "/srv/id/blob").unwrap());
    let namespace = NamespaceId::new("main").unwrap();
    for blank in ["", " ", "   ", "\t"] {
        assert_eq!(
            LocationRef::new(locator.clone(), namespace.clone(), blank),
            Err(LocationError::MissingTenantScope),
            "blank tenant scope {blank:?} must reject"
        );
    }
    let reference = LocationRef::new(locator.clone(), namespace.clone(), "tenant-7").unwrap();
    assert_eq!(reference.locator(), &locator);
    assert_eq!(reference.namespace(), &namespace);
    assert_eq!(reference.tenant_scope(), "tenant-7");
    assert_eq!(reference.kind(), LocatorKind::Laptop);
}

// ---------- generated-input properties ----------

fn segment() -> impl Strategy<Value = String> {
    "[a-z0-9][a-z0-9._-]{0,30}"
}

fn object_key() -> impl Strategy<Value = String> {
    "[a-z0-9][a-z0-9/._-]{0,50}"
}

fn laptop_path() -> impl Strategy<Value = String> {
    "/[a-z0-9._-]{0,20}(/[a-z0-9._-]{0,20}){0,3}"
}

fn drive_url() -> impl Strategy<Value = String> {
    "https://[a-z0-9][a-z0-9.-]{0,30}(/[a-z0-9/._-]{0,40})?"
}

fn locator() -> impl Strategy<Value = TypedLocator> {
    prop_oneof![
        (segment(), segment(), object_key())
            .prop_map(|(a, b, k)| TypedLocator::from(R2Locator::new(a, b, k).unwrap())),
        (segment(), laptop_path())
            .prop_map(|(h, p)| TypedLocator::from(LaptopLocator::new(h, p).unwrap())),
        (segment(), segment(), drive_url())
            .prop_map(|(a, f, u)| TypedLocator::from(DriveLocator::new(a, f, u).unwrap())),
    ]
}

proptest! {
    /// The tagged form round-trips exactly and the kind survives:
    /// a value's kind is intrinsic, never re-derived from strings.
    #[test]
    fn tagged_form_round_trips_and_preserves_kind(value in locator()) {
        let text = value.to_tagged_string();
        let parsed = TypedLocator::parse_tagged(&text).unwrap();
        prop_assert_eq!(&parsed, &value);
        prop_assert_eq!(parsed.kind(), value.kind());
        prop_assert!(text.starts_with(value.kind().tag()));
    }

    /// No coercion between kinds: re-tagging a value with another
    /// kind's tag never yields the original value back — it either
    /// rejects or yields a value of the *named* kind.
    #[test]
    fn retagging_never_coerces(value in locator()) {
        let text = value.to_tagged_string();
        let rest = text.split_once('/').unwrap().1;
        for other in [LocatorKind::R2, LocatorKind::Laptop, LocatorKind::Drive] {
            if other == value.kind() {
                continue;
            }
            let retagged = format!("{}/{rest}", other.tag());
            if let Ok(parsed) = TypedLocator::parse_tagged(&retagged) {
                prop_assert_eq!(parsed.kind(), other);
                prop_assert_ne!(parsed, value.clone());
            }
        }
    }

    /// Every generated non-blank tenant scope is accepted and read
    /// back exactly; the scope is part of the reference, not an
    /// optional decoration.
    #[test]
    fn scoped_references_round_trip(value in locator(), scope in segment()) {
        let namespace = NamespaceId::new("main").unwrap();
        let reference = LocationRef::new(value.clone(), namespace.clone(), scope.clone()).unwrap();
        prop_assert_eq!(reference.locator(), &value);
        prop_assert_eq!(reference.namespace(), &namespace);
        prop_assert_eq!(reference.tenant_scope(), scope.as_str());
    }

    /// Blank scopes of any generated whitespace shape reject with
    /// exactly MissingTenantScope.
    #[test]
    fn blank_scopes_reject_as_missing(value in locator(), blank in "\\s{0,6}") {
        let namespace = NamespaceId::new("main").unwrap();
        prop_assert_eq!(
            LocationRef::new(value, namespace, blank),
            Err(LocationError::MissingTenantScope)
        );
    }
}
