//! Exit proof for #1061 (F12.4, slice 2): a provider added through the
//! provider registry alone.
//!
//! The probe provider lives in `id-caps`'s `test-provider` feature, which the
//! root package enables through its dev-dependency. Nothing under
//! `pkgs/id/src/world/` (dispatch, actor, core) learns about it: it is one
//! manifest entry, and the surfaces below pick it up from the registry.
//!
//! The three surfaces the issue asks for:
//!
//! 1. the capability catalog the caps report prints,
//! 2. the default-ceiling selection (a registered provider must not widen
//!    authority by itself, #838),
//! 3. the compile gate, which refuses an ungranted platform-module import.

use std::collections::BTreeSet;

use id::world_caps::{
    CATALOG, CATALOG_ENTRIES, CLIENT_CATALOG_ENTRIES, CLIENT_FILE_READ_NAME, CLIENT_NOTIFY_NAME,
    CapLedger, Decision, DefaultCeiling, PROVIDER_REGISTRY, ProviderSide, TEST_PROVIDER_NAME,
    catalog_names, default_ceiling_names,
};
use id::world_compile::{CompileSpec, check_imports};

fn spec_importing(module: &str) -> CompileSpec {
    CompileSpec {
        files: vec![(
            "main.roc".to_owned(),
            format!("import {module}\n\nmain = \"ok\"\n"),
        )],
        seed: 0,
        native: false,
    }
}

#[test]
#[allow(clippy::expect_used)]
fn the_probe_is_visible_in_the_capability_catalog() {
    let names = catalog_names();
    assert!(
        names.contains(TEST_PROVIDER_NAME),
        "the test provider must be in the catalog the caps report prints, got {names:?}"
    );

    let manifest = PROVIDER_REGISTRY
        .get(TEST_PROVIDER_NAME)
        .expect("the registry resolves the test provider by name");
    assert_eq!(manifest.version, 1);
    assert_eq!(manifest.default_ceiling, DefaultCeiling::Excluded);

    // Registration appended it; the compatibility view of production names is
    // untouched, so no existing catalog consumer changes shape.
    assert_eq!(
        CATALOG_ENTRIES.last().map(|entry| entry.name),
        Some(TEST_PROVIDER_NAME)
    );
    assert!(!CATALOG.contains(&TEST_PROVIDER_NAME));

    // The assembled registry decomposes exactly, in order: the legacy
    // CATALOG's production entries, then the declared client segment
    // (CLIENT_CATALOG_ENTRIES), then the test probe — and nothing else
    // (#11016). The invariant is the decomposition against the declared
    // segment, not a bare count: registry growth happens by declaring a
    // segment, never by an entry appearing unaccounted for.
    let production = CATALOG.len();
    let client = CLIENT_CATALOG_ENTRIES.len();
    assert_eq!(production + client + 1, CATALOG_ENTRIES.len());

    let production_names: Vec<&str> = CATALOG_ENTRIES
        .iter()
        .take(production)
        .map(|entry| entry.name)
        .collect();
    assert_eq!(
        production_names.as_slice(),
        CATALOG,
        "the assembled registry must open with the legacy catalog, in order"
    );

    let assembled_client_names: Vec<&str> = CATALOG_ENTRIES
        .iter()
        .skip(production)
        .take(client)
        .map(|entry| entry.name)
        .collect();
    let declared_client_names: Vec<&str> = CLIENT_CATALOG_ENTRIES
        .iter()
        .map(|entry| entry.name)
        .collect();
    assert_eq!(
        assembled_client_names, declared_client_names,
        "the client segment of the assembled registry must be exactly the declared client entries"
    );

    // No duplicates and no unaccounted extras: the registry's name set
    // is exactly the union of the three declared parts.
    let assembled_names: Vec<&str> = CATALOG_ENTRIES.iter().map(|entry| entry.name).collect();
    let unique: BTreeSet<&str> = assembled_names.iter().copied().collect();
    assert_eq!(
        unique.len(),
        assembled_names.len(),
        "registry names must be unique"
    );
    let expected: BTreeSet<&str> = CATALOG
        .iter()
        .copied()
        .chain(declared_client_names.iter().copied())
        .chain([TEST_PROVIDER_NAME])
        .collect();
    assert_eq!(unique, expected);
}

#[test]
#[allow(clippy::expect_used)]
fn the_client_segment_is_exactly_the_declared_client_providers() {
    // Every client-side entry in the assembled registry comes from the
    // declared client segment (its position is asserted in
    // the_probe_is_visible_in_the_capability_catalog); here, its
    // contents: the two F13.7 client providers, by name, client-side,
    // excluded from every default ceiling, and absent from the legacy
    // host CATALOG view.
    let client_side: Vec<_> = CATALOG_ENTRIES
        .iter()
        .filter(|entry| entry.side == ProviderSide::Client)
        .collect();
    assert_eq!(
        client_side.len(),
        CLIENT_CATALOG_ENTRIES.len(),
        "every client-side registry entry must come from the declared client segment"
    );
    for entry in CLIENT_CATALOG_ENTRIES {
        assert_eq!(entry.side, ProviderSide::Client, "{}", entry.name);
        assert_eq!(
            entry.default_ceiling,
            DefaultCeiling::Excluded,
            "{}",
            entry.name
        );
        assert!(
            !CATALOG.contains(&entry.name),
            "{} must not join the legacy host CATALOG view",
            entry.name
        );
        assert!(
            !default_ceiling_names().contains(entry.name),
            "{} must stay out of every default ceiling",
            entry.name
        );
    }

    assert_eq!(
        CLIENT_CATALOG_ENTRIES.len(),
        2,
        "F13.7 declares exactly two client providers so far"
    );
    let declared_names: Vec<&str> = CLIENT_CATALOG_ENTRIES
        .iter()
        .map(|entry| entry.name)
        .collect();
    assert_eq!(
        declared_names,
        [CLIENT_FILE_READ_NAME, CLIENT_NOTIFY_NAME],
        "the declared client segment is client.file.read then client.notify, in order"
    );
    for name in [CLIENT_FILE_READ_NAME, CLIENT_NOTIFY_NAME] {
        let declared = CLIENT_CATALOG_ENTRIES
            .iter()
            .find(|entry| entry.name == name)
            .expect("the client segment declares the provider");
        let manifest = PROVIDER_REGISTRY
            .get(name)
            .expect("the registry resolves the client provider by name");
        assert_eq!(manifest, declared);
        assert_eq!(manifest.version, 1);
    }
}

#[test]
fn the_probe_stays_out_of_the_default_ceiling_until_a_ceiling_names_it() {
    let defaults = default_ceiling_names();
    assert!(
        !defaults.contains(TEST_PROVIDER_NAME),
        "an Excluded manifest default must keep the provider out of every default ceiling"
    );

    // Granted, but the ceiling is still the default: refused as outside it.
    let granted = BTreeSet::from([TEST_PROVIDER_NAME.to_owned()]);
    let mut ledger = CapLedger::new(granted, false);
    assert!(
        matches!(ledger.decide(TEST_PROVIDER_NAME), Decision::Deny(_)),
        "the default ceiling must refuse the probe even when it is granted"
    );

    // The same manifest is usable once an explicit ceiling names it.
    let mut explicit = defaults;
    explicit.insert(TEST_PROVIDER_NAME.to_owned());
    ledger.set_allowed(explicit);
    assert_eq!(ledger.decide(TEST_PROVIDER_NAME), Decision::Allow);
}

#[test]
fn the_compile_gate_knows_the_probe_and_refuses_it_until_granted() {
    let modules = vec![TEST_PROVIDER_NAME.to_owned()];
    let spec = spec_importing(TEST_PROVIDER_NAME);

    // Ungranted: the gate refuses the import.
    let refused = check_imports(&spec, &modules, &[]);
    assert!(
        refused.is_err(),
        "an ungranted import of the registered probe must be refused"
    );

    // Granted through an explicit ceiling that names it: the gate accepts it.
    let mut ceiling = default_ceiling_names();
    ceiling.insert(TEST_PROVIDER_NAME.to_owned());
    let mut ledger = CapLedger::new(ceiling.clone(), false);
    ledger.set_allowed(ceiling);
    assert_eq!(ledger.decide(TEST_PROVIDER_NAME), Decision::Allow);

    let granted: Vec<String> = ledger.effective().into_iter().collect();
    assert!(
        check_imports(&spec, &modules, &granted).is_ok(),
        "a ceiling-granted import of the registered probe must pass the gate"
    );
}
