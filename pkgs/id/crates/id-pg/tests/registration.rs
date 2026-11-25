//! Enforcement tests for endpoint registration (design #5805, §2, §4,
//! §9 invariant 9, verification plan §10): a role with extra privileges
//! is refused at registration, and so is an endpoint whose `db-schemas`
//! lists more than the one exposed schema.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use id_pg::{
    CollectionAllowlist, ColumnDecl, EndpointConfig, EndpointProbe, ObjectKind, Privilege,
    RegistrationError, RoleGrant, register,
};

fn collection(relation: &str) -> CollectionAllowlist {
    CollectionAllowlist {
        schema: "api".to_string(),
        relation: relation.to_string(),
        columns: vec![
            ColumnDecl {
                name: "id".to_string(),
                filterable: true,
                orderable: true,
            },
            ColumnDecl {
                name: "title".to_string(),
                filterable: true,
                orderable: false,
            },
        ],
        unique_key: vec!["id".to_string()],
        max_limit: Some(100),
    }
}

fn config() -> EndpointConfig {
    EndpointConfig {
        alias: "books-db".to_string(),
        base_url: "https://pg.example.org/rest".to_string(),
        schema: "api".to_string(),
        credential_ref: "secret://pg-books-reader".to_string(),
        collections: vec![collection("items"), collection("authors")],
    }
}

fn grant(kind: ObjectKind, object: &str, privilege: Privilege) -> RoleGrant {
    RoleGrant {
        kind,
        object: object.to_string(),
        privilege,
    }
}

fn read_only_probe() -> EndpointProbe {
    EndpointProbe {
        db_schemas: vec!["api".to_string()],
        role_grants: vec![
            grant(ObjectKind::Schema, "api", Privilege::Usage),
            grant(ObjectKind::Relation, "items", Privilege::Select),
            grant(ObjectKind::Relation, "authors", Privilege::Select),
        ],
    }
}

fn err(config: EndpointConfig, probe: &EndpointProbe) -> RegistrationError {
    register(config, probe).unwrap_err()
}

#[test]
fn read_only_single_schema_endpoint_registers() {
    let endpoint = register(config(), &read_only_probe()).unwrap();
    assert_eq!(endpoint.alias(), "books-db");
    assert_eq!(endpoint.schema(), "api");
    assert_eq!(endpoint.collections().len(), 2);
    assert_eq!(endpoint.credential_ref(), "secret://pg-books-reader");
}

#[test]
fn registered_translator_serves_only_registered_collections() {
    use id_pg::{Query, RefusalCode};
    let translator = register(config(), &read_only_probe()).unwrap().translator();
    let request = translator.translate(&Query::read("items")).unwrap();
    assert_eq!(request.accept_profile, "api");
    assert_eq!(request.path, "/items");
    let refusal = translator.translate(&Query::read("secrets")).unwrap_err();
    assert_eq!(refusal.code, RefusalCode::NotAvailable);
}

#[test]
fn extra_db_schema_is_refused() {
    let mut probe = read_only_probe();
    probe.db_schemas.push("internal".to_string());
    probe.db_schemas.push("auth".to_string());
    assert_eq!(
        err(config(), &probe),
        RegistrationError::ExtraSchemas {
            schemas: vec!["auth".to_string(), "internal".to_string()],
        }
    );
}

#[test]
fn unserved_exposed_schema_is_refused() {
    let mut probe = read_only_probe();
    probe.db_schemas = vec!["public".to_string()];
    assert_eq!(
        err(config(), &probe),
        RegistrationError::ExposedSchemaNotServed
    );
    probe.db_schemas.clear();
    assert_eq!(
        err(config(), &probe),
        RegistrationError::ExposedSchemaNotServed
    );
}

#[test]
fn repeated_exposed_schema_is_not_extra() {
    let mut probe = read_only_probe();
    probe.db_schemas.push("api".to_string());
    assert!(register(config(), &probe).is_ok());
}

#[test]
fn write_and_execute_privileges_are_refused() {
    for (kind, object, privilege) in [
        (ObjectKind::Relation, "items", Privilege::Insert),
        (ObjectKind::Relation, "items", Privilege::Update),
        (ObjectKind::Relation, "items", Privilege::Delete),
        (ObjectKind::Relation, "items", Privilege::Truncate),
        (ObjectKind::Relation, "items", Privilege::Trigger),
        (ObjectKind::Relation, "items", Privilege::References),
        (ObjectKind::Relation, "items", Privilege::Maintain),
        (ObjectKind::Function, "refresh_items", Privilege::Execute),
        (ObjectKind::Sequence, "items_id_seq", Privilege::Usage),
        (ObjectKind::Schema, "api", Privilege::Create),
        (ObjectKind::Other, "item_kind", Privilege::Usage),
    ] {
        let mut probe = read_only_probe();
        probe.role_grants.push(grant(kind, object, privilege));
        assert_eq!(
            err(config(), &probe),
            RegistrationError::ForbiddenPrivilege {
                object: object.to_string(),
                privilege,
            },
            "{kind:?} {object} {privilege:?} must be refused"
        );
    }
}

#[test]
fn insecure_and_unsafe_base_urls_are_refused() {
    for (url, expected) in [
        ("http://pg.example.org", RegistrationError::InsecureBaseUrl),
        ("ftp://pg.example.org", RegistrationError::InsecureBaseUrl),
        (
            "https://reader:pw@pg.example.org",
            RegistrationError::BaseUrlUserinfo,
        ),
        (
            "https://pg.example.org/?apikey=x",
            RegistrationError::BaseUrlQueryOrFragment,
        ),
        (
            "https://pg.example.org/#frag",
            RegistrationError::BaseUrlQueryOrFragment,
        ),
        (
            "pg.example.org",
            RegistrationError::MalformedBaseUrl("missing scheme"),
        ),
        (
            "https:///rest",
            RegistrationError::MalformedBaseUrl("missing host"),
        ),
        (
            "https://pg.example.org:notaport",
            RegistrationError::MalformedBaseUrl("invalid port"),
        ),
        (
            "https://pg.example.org /x",
            RegistrationError::MalformedBaseUrl("contains whitespace or control characters"),
        ),
        (
            "http://127.0.0.1.evil.example",
            RegistrationError::InsecureBaseUrl,
        ),
        (
            "http://localhost.evil.example",
            RegistrationError::InsecureBaseUrl,
        ),
    ] {
        let mut config = config();
        config.base_url = url.to_string();
        assert_eq!(err(config, &read_only_probe()), expected, "{url}");
    }
}

#[test]
fn plain_http_is_accepted_only_on_loopback() {
    for url in [
        "http://localhost:3000",
        "http://LOCALHOST",
        "http://127.0.0.1:3000/",
        "http://127.8.9.10",
        "http://[::1]:3000",
        "https://pg.example.org",
        "HTTPS://pg.example.org:443/rest/v1",
    ] {
        let mut config = config();
        config.base_url = url.to_string();
        assert!(register(config, &read_only_probe()).is_ok(), "{url}");
    }
}

#[test]
fn owner_config_errors_are_refused_before_the_probe_is_read() {
    // A bad config is refused even when the probe would also fail, so a
    // transport can call `validate` before it sends the credential.
    let probe = EndpointProbe::default();

    let mut c = config();
    c.alias = "books db".to_string();
    assert_eq!(err(c, &probe), RegistrationError::InvalidAlias);

    let mut c = config();
    c.schema = "api;drop".to_string();
    assert_eq!(err(c, &probe), RegistrationError::InvalidSchema);

    let mut c = config();
    c.credential_ref = "  ".to_string();
    assert_eq!(err(c, &probe), RegistrationError::MissingCredentialRef);

    let mut c = config();
    c.collections.clear();
    assert_eq!(err(c, &probe), RegistrationError::NoCollections);

    let mut c = config();
    c.collections[1].schema = "public".to_string();
    assert_eq!(
        err(c, &probe),
        RegistrationError::CollectionSchemaMismatch {
            relation: "authors".to_string()
        }
    );

    let mut c = config();
    c.collections.push(collection("items"));
    assert_eq!(
        err(c, &probe),
        RegistrationError::DuplicateCollection {
            relation: "items".to_string()
        }
    );

    let mut c = config();
    c.collections[0].max_limit = Some(0);
    assert_eq!(
        err(c, &probe),
        RegistrationError::ZeroLimit {
            relation: "items".to_string()
        }
    );

    let mut c = config();
    c.collections[0].columns[1].name = "or".to_string();
    assert!(matches!(
        err(c, &probe),
        RegistrationError::InvalidCollection { relation, .. } if relation == "items"
    ));
}

#[test]
fn errors_never_carry_the_credential_reference() {
    let mut c = config();
    c.credential_ref = "secret://do-not-leak".to_string();
    c.base_url = "http://pg.example.org".to_string();
    let mut probe = read_only_probe();
    probe.db_schemas.push("internal".to_string());
    for error in [err(c.clone(), &probe), err(config(), &probe), {
        let mut probe = read_only_probe();
        probe
            .role_grants
            .push(grant(ObjectKind::Function, "f", Privilege::Execute));
        err(c, &probe)
    }] {
        assert!(!error.to_string().contains("do-not-leak"), "{error}");
    }
}

#[test]
fn owner_config_deserializes_and_rejects_unknown_fields() {
    let json = r#"{
        "alias": "books-db",
        "base_url": "https://pg.example.org",
        "schema": "api",
        "credential_ref": "secret://pg-books-reader",
        "collections": [{
            "schema": "api",
            "relation": "items",
            "columns": [{"name": "id", "filterable": true, "orderable": true}],
            "unique_key": ["id"],
            "max_limit": null
        }]
    }"#;
    let config: EndpointConfig = serde_json::from_str(json).unwrap();
    assert_eq!(config.collections.len(), 1);
    let probe: EndpointProbe = serde_json::from_str(
        r#"{"db_schemas": ["api"], "role_grants": [
            {"kind": "schema", "object": "api", "privilege": "USAGE"},
            {"kind": "relation", "object": "items", "privilege": "SELECT"}
        ]}"#,
    )
    .unwrap();
    assert!(register(config, &probe).is_ok());

    let with_password = json.replace(
        "\"schema\": \"api\",\n        \"credential_ref\"",
        "\"schema\": \"api\",\n        \"password\": \"x\",\n        \"credential_ref\"",
    );
    assert!(serde_json::from_str::<EndpointConfig>(&with_password).is_err());
}
