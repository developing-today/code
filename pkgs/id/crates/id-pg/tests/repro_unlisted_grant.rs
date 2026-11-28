use id_pg::{
    CollectionAllowlist, ColumnDecl, EndpointConfig, EndpointProbe, ObjectKind, Privilege,
    RoleGrant, register,
};

#[test]
fn registration_accepts_select_on_unlisted_relation() {
    // Oracle: design §4 layer 2 promises SELECT only on exposed views,
    // independent of the adapter allowlist (layer 1).
    let config = EndpointConfig {
        alias: "source".into(),
        base_url: "https://example.test".into(),
        schema: "exposed".into(),
        credential_ref: "keyring:test".into(),
        collections: vec![CollectionAllowlist {
            schema: "exposed".into(),
            relation: "items".into(),
            columns: vec![ColumnDecl { name: "id".into(), filterable: true, orderable: true }],
            unique_key: vec!["id".into()],
            max_limit: None,
        }],
    };
    let probe = EndpointProbe {
        db_schemas: vec!["exposed".into()],
        role_grants: vec![
            RoleGrant { kind: ObjectKind::Schema, object: "exposed".into(), privilege: Privilege::Usage },
            RoleGrant { kind: ObjectKind::Relation, object: "items".into(), privilege: Privilege::Select },
            RoleGrant { kind: ObjectKind::Relation, object: "secrets".into(), privilege: Privilege::Select },
        ],
    };
    let endpoint = register(config, &probe).expect("reproduces acceptance of the overprivileged role");
    assert_eq!(endpoint.collections().len(), 1);
    assert_eq!(endpoint.collections()[0].relation, "items");
}
