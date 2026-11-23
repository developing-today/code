//! pmo_operator_docs_match_registered_help_inventory — DOC-01
//! (developing-today/code#10728).
//!
//! Oracle: the operator/contributor contract in `pkgs/fleet/README.md`
//! must stay consistent with the registered help inventory in this
//! crate (`DOMAINS`), must carry the DOC-01 honesty-contract strings,
//! and must not claim what the surface never guarantees. Designed
//! spellings (`fleet work …`, `fleet dependency …`) are allowed only
//! inside the README section that explicitly marks them "not yet in
//! the registry".

use std::fs;
use std::path::PathBuf;

const DESIGNED_HEADING: &str = "### Designed spellings — not yet in the registry";
const DESIGNED_END: &str = "\n### ";

/// The real `fleet queue` dispatch surface
/// (`crates/fleet-queue/src/client.rs`; the registry's queue
/// `subcommands` array is descriptive and narrower — see
/// `docs/verbs.md`, which records the same).
const QUEUE_VERBS: &[&str] = &[
    "validate",
    "list",
    "enqueue",
    "claim",
    "release",
    "lane",
    "bind",
    "heartbeat",
    "checkpoint",
    "park",
    "wake",
    "block",
    "unblock",
    "dep-recheck",
    "quarantine-recheck",
    "cancel",
    "revive",
    "fail",
    "complete",
    "verify-disposition",
    "timeline",
    "history",
    "models",
    "health",
    "signal",
    "fallback",
    "dispatch",
    "session",
    "surfaces",
    "outbox",
    "stack",
];

/// Root built-ins the multicall dispatch answers directly
/// (`crates/fleet/src/main.rs`).
const BUILTINS: &[&str] = &["init", "session", "version", "capabilities", "help"];

fn readme() -> String {
    let path: PathBuf = [env!("CARGO_MANIFEST_DIR"), "..", "..", "README.md"]
        .iter()
        .collect();
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// Every `` `…` `` code span in the README.
fn code_spans(readme: &str) -> Vec<String> {
    let mut spans = Vec::new();
    let mut rest = readme;
    while let Some(open) = rest.find('`') {
        let after_open = &rest[open + 1..];
        if let Some(close) = after_open.find('`') {
            spans.push(after_open[..close].to_string());
            rest = &after_open[close + 1..];
        } else {
            break;
        }
    }
    spans
}

/// `fleet <first> [second]` from a code span, if the span is such a
/// command (single words only — flags and `<placeholders>` are
/// skipped by the token grammar).
fn fleet_command(span: &str) -> Option<(String, Option<String>)> {
    let mut words = span.split_whitespace();
    if words.next()? != "fleet" {
        return None;
    }
    let word = |w: &str| {
        let mut chars = w.chars();
        matches!(chars.next(), Some('a'..='z'))
            && chars.all(|c| matches!(c, 'a'..='z' | '0'..='9' | '-'))
    };
    let first = words.next()?;
    if !word(first) {
        return None;
    }
    let second = words.next().filter(|w| word(w)).map(str::to_string);
    Some((first.to_string(), second))
}

fn designed_slice(readme: &str) -> (usize, usize) {
    let start = readme
        .find(DESIGNED_HEADING)
        .expect("README must carry the designed-spellings heading");
    let end = readme[start + DESIGNED_HEADING.len()..]
        .find(DESIGNED_END)
        .map(|offset| start + DESIGNED_HEADING.len() + offset)
        .unwrap_or(readme.len());
    (start, end)
}

#[test]
fn pmo_operator_docs_match_registered_help_inventory() {
    let readme = readme();

    // 1. Every registered domain appears in the README's inventory
    //    paragraph as a `fleet <name>` code span.
    for domain in fleet_registry::DOMAINS {
        let span = format!("`fleet {}`", domain.name);
        assert!(
            readme.contains(&span),
            "README inventory must list registered domain `{}` as {}",
            domain.name,
            span
        );
    }
    for builtin in BUILTINS {
        let span = format!("`fleet {}`", builtin);
        assert!(
            readme.contains(&span),
            "README inventory must list built-in `{}` as {}",
            builtin,
            span
        );
    }

    // 2. The DOC-01 required contract strings (the issue's done check).
    for required in [
        "fleet dependency add",
        "fleet work complete",
        "private by default",
        "no exactly-once guarantee",
        "Queue Table Improvements",
    ] {
        assert!(
            readme.contains(required),
            "README operator contract must contain {required:?}"
        );
    }

    // 3. Shipped examples must match the real dispatch surface, and
    //    no code span may name an unregistered domain. The designed
    //    spellings are exempt only inside their explicitly marked
    //    section.
    let (designed_start, designed_end) = designed_slice(&readme);
    let designed = ["work", "dependency"];
    for span in code_spans(&readme) {
        let Some((first, second)) = fleet_command(&span) else {
            continue;
        };
        if designed.contains(&first.as_str()) {
            // Confined by the check below, which scans everything
            // outside the marked section.
            continue;
        }
        let registered = fleet_registry::DOMAINS.iter().any(|d| d.name == first);
        assert!(
            registered || BUILTINS.contains(&first.as_str()),
            "code span `{span}` names `{first}`, which is neither a registered domain nor a built-in"
        );
        if first == "queue" {
            if let Some(verb) = second {
                assert!(
                    QUEUE_VERBS.contains(&verb.as_str()),
                    "code span `{span}` presents queue verb `{verb}`, which the client does not dispatch"
                );
            }
        }
    }
    // Designed spellings appear in their marked section and NOWHERE
    // else, in prose or in code spans.
    let outside = format!("{}{}", &readme[..designed_start], &readme[designed_end..]);
    for word in designed {
        let needle = format!("fleet {word}");
        assert!(
            readme[designed_start..designed_end].contains(&needle),
            "designed spelling `{needle}` must appear in its marked section"
        );
        assert!(
            !outside.contains(&needle),
            "designed spelling `{needle}` may appear only under {DESIGNED_HEADING:?}"
        );
    }

    // 4. Negative coverage — claims the docs must never make
    //    (DOC-01; phrased so a denial also avoids the pattern).
    for forbidden in [
        "parent blocks",
        "author field proves",
        "author proves",
        "optimistic locking",
        "exactly-once delivery",
        "exactly-once completion",
        "guarantees exactly-once",
    ] {
        assert!(
            !readme.contains(forbidden),
            "README must not claim {forbidden:?}"
        );
    }
}
