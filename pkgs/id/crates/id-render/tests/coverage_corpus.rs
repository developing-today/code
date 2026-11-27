#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Coverage gates for the semantic vocabulary and assertion-role corpus.

use std::{collections::BTreeSet, fs, path::PathBuf};

use id_render::{
    compose::{AssertionInput, ComposeData, Specificity, TargetSpec},
    compose_document,
    lower::lower_web,
    lower_native::lower_native,
    native_table, parse,
    vocabulary::REGISTRY,
    web_table,
};

fn coverage_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/conformance/coverage")
}

fn assertion_cases() -> BTreeSet<(String, String)> {
    fs::read_to_string(coverage_dir().join("assertion-roles.txt"))
        .expect("read assertion-role corpus")
        .lines()
        .filter_map(|line| {
            let line = line.split('#').next().unwrap_or_default().trim();
            if line.is_empty() {
                return None;
            }
            let mut fields = line.split_whitespace();
            Some((fields.next()?.to_string(), fields.next()?.to_string()))
        })
        .collect()
}

#[test]
fn every_registered_element_has_a_parser_corpus_case() {
    let dir = coverage_dir();
    let source = fs::read_to_string(dir.join("vocabulary-elements.id"))
        .expect("read vocabulary-element corpus");
    let document = parse(&source).expect("vocabulary-element corpus parses");
    let mut names = BTreeSet::new();
    let mut pending = vec![&document.root];
    while let Some(element) = pending.pop() {
        names.insert(element.name.clone());
        for child in &element.children {
            match child {
                id_render::Node::Element(child) => pending.push(child),
                id_render::Node::Raw(_) => {
                    names.insert("raw".to_string());
                }
                id_render::Node::Text(_) => {}
            }
        }
    }

    let missing = REGISTRY
        .iter()
        .map(|spec| spec.name)
        .filter(|name| !names.contains(*name))
        .collect::<Vec<_>>();
    assert!(
        missing.is_empty(),
        "vocabulary elements without corpus cases: {missing:?}"
    );
}

#[test]
fn every_web_and_native_assertion_role_has_a_corpus_case() {
    let cases = assertion_cases();
    let web_roles = web_table::ROLE_ROWS
        .iter()
        .map(|row| (row.property.to_string(), row.token.to_string()))
        .chain([
            ("density".to_string(), "comfortable".to_string()),
            ("density".to_string(), "compact".to_string()),
        ])
        .chain((0..=8).flat_map(|n| {
            ["gap", "pad"].map(move |property| (property.to_string(), format!("space-{n}")))
        }))
        .collect::<BTreeSet<_>>();
    let native_roles = native_table::ROLE_ROWS
        .iter()
        .map(|row| (row.property.to_string(), row.token.to_string()))
        .chain((0..=8).flat_map(|n| {
            ["gap", "pad"].map(move |property| (property.to_string(), format!("space-{n}")))
        }))
        .collect::<BTreeSet<_>>();

    assert_eq!(
        cases, web_roles,
        "web assertion roles and corpus cases differ"
    );
    assert_eq!(
        cases, native_roles,
        "native assertion roles and corpus cases differ"
    );
}

#[test]
fn assertion_role_corpus_reaches_each_lowering_target() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/conformance/target-goldens");
    let source = fs::read_to_string(dir.join("basic.id")).expect("read target corpus source");
    let document = id_render::parse_and_validate(&source).expect("target corpus validates");
    let target = |name: &str| TargetSpec {
        name: name.to_string(),
        form_factor: "desktop".to_string(),
        capabilities: BTreeSet::new(),
    };

    for (property, token) in assertion_cases() {
        let data = ComposeData {
            assertions: vec![AssertionInput {
                element_path: "document/page[0]/button[0]".to_string(),
                property: property.clone(),
                token: token.clone(),
                specificity: Specificity::Instance,
            }],
            ..ComposeData::default()
        };

        let web = compose_document(&document, &target("web"), &data).expect("compose web");
        let (_, web_report) = lower_web(&document, &web).expect("lower web assertion case");
        assert!(
            web_report.defaulted.is_empty()
                && web_report.mapped.len() == 1
                && web_report.mapped[0].property == property
                && web_report.mapped[0].token == token,
            "web assertion case was not mapped: {property}={token}"
        );

        for platform in native_table::PLATFORMS {
            let native = compose_document(&document, &target(platform), &data)
                .expect("compose native target");
            let (_, report) =
                lower_native(&document, &native).expect("lower native assertion case");
            let is_platform_default = native_table::role_row(&property, &token)
                .is_some_and(|row| row.rendering_on(platform).is_none());
            if is_platform_default {
                assert!(
                    report.mapped.is_empty()
                        && report.defaulted.len() == 1
                        && report.defaulted[0].property == property
                        && report.defaulted[0].token == token,
                    "native platform-default case was not reported: {property}={token} on {platform}"
                );
            } else {
                assert!(
                    report.defaulted.is_empty()
                        && report.mapped.len() == 1
                        && report.mapped[0].property == property
                        && report.mapped[0].token == token,
                    "native assertion case was not mapped: {property}={token} on {platform}"
                );
            }
        }
    }
}
