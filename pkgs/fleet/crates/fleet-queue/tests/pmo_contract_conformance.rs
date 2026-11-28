//! CEN-07 real-adapter conformance (developing-today/code#9437; design
//! record `doc/2026-11-25T20-54-29Z_design_cen07_adapter_conformance_ci/`).
//!
//! Golden fixtures under `tests/fixtures/pmo-conformance/` are fed to
//! the code that serves each PM orchestration contract on main: the
//! `fleet.command/v1` envelope (`work::contracts`), command acceptance
//! (`work::operations`), the per-source activity cursor (`work::activity`),
//! the priority selector (`work::priority`) and
//! the managed body section policy (`work::authority`). Nothing here
//! is faked except the network edge, which these pure models do not
//! touch. `manifest.json` names the tests that cover each landed
//! family; gated families contribute no tests.
//!
//! The `acceptance` family also reads the projector's TypeScript
//! source read-only (`pm-stack/projector/src/`). A hermetic build that
//! carries only `pkgs/` cannot see it: the projector checks then print
//! `NOT RUN` and return, unless `PMO_CONFORMANCE_REQUIRE_PROJECTOR` is
//! set, as the conformance venue sets it, in which case a missing
//! source fails.

use std::cmp::Ordering;
use std::path::{Path, PathBuf};

use fleet_queue::work::activity::{ActivityRecord, ActivityState, dedupe_replay, refresh_activity};
use fleet_queue::work::authority::{
    BestEffortOptIn, ConditionalWriteCapability, Guarantee, MalformedMarkers, Refusal,
    TimeoutResolution, plan_update, reconcile_after_patch_timeout,
};
use fleet_queue::work::contracts::{
    Command, CommandResult, ContractError, WorkRef, canonical_json,
};
use fleet_queue::work::operations::{
    AcceptanceLedger, AuthorizationError, AuthorizationPolicy, Command as OpCommand, Grant,
    Principal, ResourceRef, Verb,
};
use fleet_queue::work::priority::{
    CandidateKey, PriorityError, PriorityProfile, PriorityValue, compare_candidates,
    order_candidates,
};
use serde_json::{Map, Value};

const REQUIRE_PROJECTOR_ENV: &str = "PMO_CONFORMANCE_REQUIRE_PROJECTOR";

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pmo-conformance")
}

/// The repository root: `pkgs/fleet/crates/fleet-queue` is four levels
/// below it.
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../..")
}

fn load(rel: &str) -> Value {
    let path = fixtures_dir().join(rel);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("read fixture {}: {err}", path.display()));
    serde_json::from_str(&text)
        .unwrap_or_else(|err| panic!("parse fixture {}: {err}", path.display()))
}

fn str_of<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key]
        .as_str()
        .unwrap_or_else(|| panic!("fixture field {key:?} is not a string: {value}"))
}

/// Apply a fixture negative to the golden object: `patch` fields are
/// set at the top level and `remove` fields are dropped.
fn patched(golden: &Value, case: &Value) -> Value {
    let mut object: Map<String, Value> = golden.as_object().expect("golden is an object").clone();
    if let Some(patch) = case.get("patch").and_then(Value::as_object) {
        for (key, value) in patch {
            object.insert(key.clone(), value.clone());
        }
    }
    if let Some(remove) = case.get("remove").and_then(Value::as_array) {
        for key in remove {
            object.remove(key.as_str().expect("remove names a field"));
        }
    }
    Value::Object(object)
}

fn canon(value: &Value) -> String {
    canonical_json(value).expect("fixture values are canonicalizable")
}

// ---------------------------------------------------------------------------
// command
// ---------------------------------------------------------------------------

#[test]
fn test_adapter_conformance_command_envelope_round_trips() {
    let cases = load("command/cases.json");
    let golden = &cases["envelope"];
    let text = golden.to_string();

    let command = Command::from_json(&text).expect("golden envelope parses and validates");
    assert_eq!(
        canon(&command.intent_dto()),
        str_of(&cases, "intent_canonical_json"),
        "intent DTO bytes differ from the golden JCS"
    );
    assert_eq!(
        command.compute_request_digest().unwrap(),
        str_of(&cases, "request_digest"),
        "request digest differs from the independently computed golden"
    );
    command.verify_request_digest().unwrap();

    // Serialize: the wire form carries exactly the golden fields.
    let wire: Value = serde_json::from_str(&command.to_json()).unwrap();
    assert_eq!(canon(&wire), canon(golden), "serialized envelope drifted");
    assert_eq!(Command::from_json(&command.to_json()).unwrap(), command);

    // A retry with a new operation id, delivery and dry-run mode keeps
    // the digest; a changed precondition changes it.
    let same = &cases["digest_invariants"]["same_intent"];
    let replay = Command::from_json(&same["envelope"].to_string()).unwrap();
    assert_eq!(
        replay.compute_request_digest().unwrap(),
        str_of(same, "request_digest")
    );
    replay.verify_request_digest().unwrap();
    let altered_case = &cases["digest_invariants"]["altered_precondition"];
    let altered = Command::from_json(&patched(golden, altered_case).to_string()).unwrap();
    assert_eq!(
        altered.compute_request_digest().unwrap(),
        str_of(altered_case, "request_digest")
    );
    assert_ne!(
        altered.compute_request_digest().unwrap(),
        command.compute_request_digest().unwrap()
    );
    assert_eq!(
        altered.verify_request_digest().unwrap_err().code(),
        "invalid_schema",
        "a carried digest that no longer matches the intent is refused"
    );

    // The result envelope round-trips and validates.
    let result_value = &cases["result"];
    let result: CommandResult = serde_json::from_value(result_value.clone()).unwrap();
    result.validate().unwrap();
    let result_wire: Value = serde_json::from_str(&result.to_json()).unwrap();
    assert_eq!(canon(&result_wire), canon(result_value));
    assert_eq!(result.outcome_parsed().unwrap().as_str(), "accepted");
    assert_eq!(
        result.work,
        WorkRef::new("fleet-local", "842").unwrap(),
        "pair identity survives the round trip"
    );

    // Every other negative is typed and never a partial parse.
    for case in cases["negatives"].as_array().unwrap() {
        let name = str_of(case, "name");
        let err = Command::from_json(&patched(golden, case).to_string())
            .expect_err(&format!("negative {name} must be refused"));
        assert_eq!(
            err.code(),
            str_of(case, "expected_error"),
            "negative {name}"
        );
        assert!(!err.mutates_canonical_state(), "negative {name}");
    }
}

#[test]
fn test_adapter_conformance_command_rejects_client_authorization() {
    let cases = load("command/cases.json");
    let case = cases["negatives"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "client_authorization_claim")
        .expect("fixture carries the client authorization negative");
    let presented = patched(&cases["envelope"], case);
    assert_eq!(presented["authorized"], Value::Bool(true));
    let err = Command::from_json(&presented.to_string()).unwrap_err();
    assert_eq!(err, ContractError::ClientSuppliedAuthorization);
    assert_eq!(err.code(), str_of(case, "expected_error"));

    // The claim is refused even when the rest of the envelope is
    // broken: it is checked before any other interpretation.
    let mut broken = presented.as_object().unwrap().clone();
    broken.remove("reason");
    broken.insert("schema_version".into(), Value::from("fleet.command/v9"));
    assert_eq!(
        Command::from_json(&Value::Object(broken).to_string()).unwrap_err(),
        ContractError::ClientSuppliedAuthorization
    );
}

#[test]
fn test_adapter_conformance_command_unknown_major_version_typed() {
    let cases = load("command/cases.json");
    let golden = &cases["envelope"];
    let case = cases["negatives"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "unknown_major_version")
        .expect("fixture carries the unknown command version negative");
    let presented = patched(golden, case);
    match Command::from_json(&presented.to_string()) {
        Err(ContractError::UnsupportedVersion { schema_version }) => {
            assert_eq!(schema_version, str_of(&presented, "schema_version"));
        }
        other => panic!("unknown command version must be typed, got {other:?}"),
    }

    for case in cases["result_negatives"].as_array().unwrap() {
        let name = str_of(case, "name");
        let value = patched(&cases["result"], case);
        let err = match serde_json::from_value::<CommandResult>(value) {
            Ok(result) => result
                .validate()
                .expect_err(&format!("result negative {name} must be refused")),
            Err(err) => panic!("result negative {name} must parse then fail typed: {err}"),
        };
        assert_eq!(
            err.code(),
            str_of(case, "expected_error"),
            "result negative {name}"
        );
    }
}

// ---------------------------------------------------------------------------
// acceptance
// ---------------------------------------------------------------------------

fn policy_from(value: &Value) -> AuthorizationPolicy {
    let grants = value["grants"]
        .as_array()
        .unwrap()
        .iter()
        .map(|grant| Grant {
            principal: str_of(grant, "principal").to_string(),
            verbs: grant["verbs"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| Verb::parse(v.as_str().unwrap()).expect("fixture verb is canonical"))
                .collect(),
            repos: grant["repos"]
                .as_array()
                .unwrap()
                .iter()
                .map(|r| r.as_str().unwrap().to_string())
                .collect(),
        })
        .collect();
    AuthorizationPolicy { grants }
}

/// The Rust mapping of the projector's forwarded envelope onto
/// `operations::Command`. The actor is authenticated because the
/// projector only forwards from the HMAC-verified payload. `args` has
/// no slot on the Fleet side today (mismatch C1).
fn to_operation(envelope: &Value) -> OpCommand {
    OpCommand {
        idempotency_key: str_of(envelope, "idempotencyKey").to_string(),
        principal: Principal::authenticated(str_of(envelope, "actor")),
        verb: Verb::parse(str_of(envelope, "verb")).expect("forwarded verb is canonical"),
        resource: ResourceRef {
            repo: str_of(envelope, "repo").to_string(),
            number: envelope["issue"].as_u64().expect("issue is a number"),
        },
        delivery_id: str_of(envelope, "deliveryId").to_string(),
    }
}

fn authorization_code(err: AuthorizationError) -> &'static str {
    match err {
        AuthorizationError::Unauthenticated => "unauthenticated",
        AuthorizationError::NoGrant => "no_grant",
        AuthorizationError::VerbDenied => "verb_denied",
        AuthorizationError::ResourceDenied => "resource_denied",
    }
}

/// Render the projector's key template with one forwarded envelope's
/// own fields.
fn render_key(template: &str, envelope: &Value) -> String {
    template
        .replace("${repo}", str_of(envelope, "repo"))
        .replace("${issue}", &envelope["issue"].to_string())
        .replace("${comment.id}", &envelope["commentId"].to_string())
        .replace("${command.verb}", str_of(envelope, "verb"))
}

/// Read a projector source file, or report that the hermetic build
/// does not carry it. Returns `None` only when the venue did not
/// require the projector.
fn read_projector(rel: &str) -> Option<String> {
    let path = repo_root().join(rel);
    match std::fs::read_to_string(&path) {
        Ok(text) => Some(text),
        Err(err) => {
            let required = std::env::var(REQUIRE_PROJECTOR_ENV)
                .map(|v| !v.trim().is_empty())
                .unwrap_or(false);
            assert!(
                !required,
                "{REQUIRE_PROJECTOR_ENV} is set but {} is unreadable: {err}",
                path.display()
            );
            eprintln!(
                "NOT RUN: projector source {} is not in this tree ({err}); \
                 the cross-language check runs only where the projector is present",
                path.display()
            );
            None
        }
    }
}

/// The cross-language check (design record §3.3): the projector's key
/// template and envelope field list match the fixture, every forwarded
/// envelope's key is that template applied to its own fields, and no
/// projector entry point attaches a Fleet sink. Returns whether the
/// projector source was read.
fn check_projector_contract(cases: &Value) -> bool {
    let template = str_of(cases, "key_template");
    let fields: Vec<&str> = cases["forwarded_envelope_fields"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f.as_str().unwrap())
        .collect();

    // Fixture-internal: keys follow the template and envelopes carry
    // exactly the field list.
    for delivery in cases["deliveries"].as_array().unwrap() {
        let envelope = &delivery["forwarded"];
        if envelope.is_null() {
            continue;
        }
        let name = str_of(delivery, "name");
        assert_eq!(
            str_of(envelope, "idempotencyKey"),
            render_key(template, envelope),
            "delivery {name}: key is not the template applied to its fields"
        );
        let keys: Vec<&str> = envelope
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        let mut sorted_keys = keys.clone();
        sorted_keys.sort_unstable();
        let mut sorted_fields = fields.clone();
        sorted_fields.sort_unstable();
        assert_eq!(
            sorted_keys, sorted_fields,
            "delivery {name}: envelope fields"
        );
    }

    let Some(source) = read_projector(str_of(cases, "projector_source")) else {
        return false;
    };

    // Key template: the template literal assigned to idempotencyKey.
    let marker = "idempotencyKey: `";
    let start = source
        .find(marker)
        .expect("projector builds idempotencyKey from a template literal")
        + marker.len();
    let end = start + source[start..].find('`').expect("template literal closes");
    assert_eq!(
        &source[start..end],
        template,
        "the projector's key template changed; update the fixture and the Rust mapping together"
    );
    assert_eq!(
        source.matches(marker).count(),
        1,
        "the projector builds idempotencyKey in more than one place"
    );

    // Envelope fields, in declaration order.
    let open = "export interface OperationEnvelope {";
    let body_start = source
        .find(open)
        .expect("projector declares OperationEnvelope")
        + open.len();
    let body_end = body_start + source[body_start..].find("\n}").expect("interface closes");
    let declared: Vec<&str> = source[body_start..body_end]
        .lines()
        .map(str::trim)
        .filter(|line| !line.starts_with("/*") && !line.starts_with('*') && !line.starts_with("//"))
        .filter_map(|line| {
            line.split_once(':')
                .map(|(name, _)| name.trim_end_matches('?'))
        })
        .filter(|name| !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric()))
        .collect();
    assert_eq!(
        declared, fields,
        "the projector's OperationEnvelope fields changed; update the fixture and the Rust mapping together"
    );

    // Inert default wiring: only the file that declares the optional
    // dependency names the sink.
    let wiring = &cases["inert_default_wiring"];
    let token = str_of(wiring, "token");
    let only = str_of(wiring, "only_file");
    let dir = repo_root().join(str_of(wiring, "projector_src_dir"));
    let mut namers = Vec::new();
    for entry in std::fs::read_dir(&dir).expect("projector src directory is readable") {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("ts") {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        if text.contains(token) {
            namers.push(path.file_name().unwrap().to_string_lossy().into_owned());
        }
    }
    namers.sort();
    assert_eq!(
        namers,
        vec![only.to_string()],
        "a projector file other than {only} names {token}: a sink may be attached by default"
    );
    true
}

#[test]
fn test_adapter_conformance_distinguishes_projection_ack_from_fleet_acceptance() {
    let cases = load("acceptance/cases.json");
    let policy = policy_from(&cases["policy"]);
    let mut ledger = AcceptanceLedger::new();

    for delivery in cases["deliveries"].as_array().unwrap() {
        let name = str_of(delivery, "name");
        let expected = &delivery["expected"];
        let payload = &delivery["payload"];
        let body = payload["comment"]["body"].as_str().unwrap();
        let forwarded = &delivery["forwarded"];

        // Projection application is recorded independently of the
        // command: every delivery here was applied by the projector.
        assert_eq!(
            delivery["projection_applied"],
            Value::Bool(true),
            "delivery {name}"
        );

        let before = ledger.clone();
        if forwarded.is_null() {
            // A projection ACK with no forwarded envelope is never
            // acceptance: nothing reaches the ledger.
            assert!(
                !body.trim_start().starts_with("/fleet"),
                "delivery {name}: a command comment must forward an envelope"
            );
            assert_eq!(expected["accepted"], Value::Bool(false), "delivery {name}");
            assert_eq!(ledger, before, "delivery {name}");
        } else {
            // The forwarded envelope comes from the signed payload.
            assert_eq!(
                str_of(forwarded, "deliveryId"),
                str_of(delivery, "delivery_id")
            );
            assert_eq!(str_of(forwarded, "event"), str_of(delivery, "event"));
            assert_eq!(
                forwarded["commentId"], payload["comment"]["id"],
                "delivery {name}"
            );
            assert_eq!(forwarded["actor"], payload["comment"]["user"]["login"]);
            assert_eq!(forwarded["repo"], payload["repository"]["full_name"]);
            assert_eq!(forwarded["issue"], payload["issue"]["number"]);
            assert!(
                body.trim_start().starts_with("/fleet"),
                "delivery {name}: only a leading /fleet forwards"
            );
            assert!(
                !str_of(forwarded, "idempotencyKey").contains(str_of(delivery, "delivery_id")),
                "delivery {name}: the delivery id must never be part of the key"
            );

            let command = to_operation(forwarded);
            match ledger.accept(&policy, &command) {
                Ok(outcome) => {
                    assert_eq!(expected["accepted"], Value::Bool(true), "delivery {name}");
                    assert_eq!(
                        Value::Bool(outcome.duplicate),
                        expected["duplicate"],
                        "delivery {name}"
                    );
                    assert_eq!(
                        Value::from(outcome.receipt.sequence),
                        expected["sequence"],
                        "delivery {name}"
                    );
                    assert_eq!(outcome.receipt.idempotency_key, command.idempotency_key);
                    if outcome.duplicate {
                        assert_eq!(ledger, before, "delivery {name}: a repeat changes nothing");
                    } else {
                        let effect = ledger.effects().last().unwrap();
                        assert_eq!(effect.idempotency_key, command.idempotency_key);
                        assert_eq!(
                            effect.delivery_id,
                            str_of(delivery, "delivery_id"),
                            "delivery {name}: the effect carries its delivery as provenance"
                        );
                    }
                }
                Err(err) => {
                    assert_eq!(expected["accepted"], Value::Bool(false), "delivery {name}");
                    assert_eq!(
                        authorization_code(err),
                        str_of(expected, "error"),
                        "delivery {name}"
                    );
                    assert_eq!(ledger, before, "delivery {name}: a refusal records nothing");
                    assert!(ledger.receipt(&command.idempotency_key).is_none());
                }
            }
        }
        assert_eq!(
            Value::from(ledger.effects().len()),
            expected["effects_after"],
            "delivery {name}: effect count"
        );
    }

    // The first delivery, not the redelivery, carried the accepted
    // attempt.
    let first = &cases["deliveries"][0];
    let key = str_of(&first["forwarded"], "idempotencyKey");
    let effect = ledger
        .effects()
        .iter()
        .find(|e| e.idempotency_key == key)
        .expect("the first command was accepted");
    assert_eq!(effect.delivery_id, str_of(first, "delivery_id"));

    // An unauthenticated actor is refused before any grant.
    let mut unauthenticated = to_operation(&first["forwarded"]);
    unauthenticated.principal = Principal::unauthenticated("octocat");
    unauthenticated.idempotency_key.push_str(":unauthenticated");
    let before = ledger.clone();
    assert_eq!(
        ledger.accept(&policy, &unauthenticated).unwrap_err(),
        AuthorizationError::Unauthenticated
    );
    assert_eq!(ledger, before);

    // Design record §4 step 5: the cross-language check.
    check_projector_contract(&cases);
}

#[test]
fn test_adapter_conformance_projector_key_template_matches_fixture() {
    let cases = load("acceptance/cases.json");
    if check_projector_contract(&cases) {
        eprintln!("projector key template, envelope fields and inert wiring match the fixture");
    }
}

// ---------------------------------------------------------------------------
// selector
// ---------------------------------------------------------------------------

#[test]
fn test_adapter_conformance_selector_profile_golden() {
    let cases = load("selector/cases.json");
    let golden = &cases["profile"];

    let profile =
        PriorityProfile::from_json(&golden.to_string()).expect("golden profile validates");
    assert_eq!(
        profile,
        PriorityProfile::queue_v1(),
        "the queue profile drifted from the fixture"
    );
    let wire: Value = serde_json::from_str(&profile.to_json()).unwrap();
    assert_eq!(canon(&wire), canon(golden), "serialized profile drifted");

    for request in cases["requests"].as_array().unwrap() {
        let input = request["input"].as_str();
        let got = profile.resolve_request(input);
        match (request.get("expected_value"), got) {
            (Some(value), Ok(resolved)) => {
                assert_eq!(Value::from(resolved.value), *value, "request {input:?}");
                assert_eq!(resolved.profile_id, profile.profile_id);
                assert_eq!(resolved.profile_version, profile.version);
                profile.check(&resolved).unwrap();
            }
            (None, Err(err)) => {
                assert_eq!(
                    err.code(),
                    str_of(request, "expected_error"),
                    "request {input:?}"
                );
            }
            (expected, got) => panic!("request {input:?}: expected {expected:?}, got {got:?}"),
        }
    }

    for case in cases["invalid_profiles"].as_array().unwrap() {
        let name = str_of(case, "name");
        let err = PriorityProfile::from_json(&patched(golden, case).to_string())
            .expect_err(&format!("invalid profile {name} must be refused"));
        assert_eq!(
            err.code(),
            str_of(case, "expected_error"),
            "invalid profile {name}"
        );
    }

    // Claim order.
    let ordering = &cases["ordering"];
    let candidates: Vec<CandidateKey> = ordering["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| CandidateKey {
            work: WorkRef::new(
                str_of(&c["work"], "authority_id"),
                str_of(&c["work"], "work_id"),
            )
            .unwrap(),
            urgency: c["priority"]
                .as_i64()
                .map(|value| profile.validate_value(value).unwrap()),
            peer_rank: c["peer_rank"].as_i64(),
            created_at: c["created_at"].as_i64().unwrap(),
        })
        .collect();
    let ordered = order_candidates(candidates.clone()).unwrap();
    let ids: Vec<&str> = ordered.iter().map(|c| c.work.work_id.as_str()).collect();
    let expected: Vec<&str> = ordering["expected_order"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(ids, expected, "claim order");
    for pair in ordered.windows(2) {
        assert_eq!(
            compare_candidates(&pair[0], &pair[1]).unwrap(),
            Ordering::Less
        );
    }

    // Values from another profile version are never compared.
    let mixed = &cases["mixed_profile"];
    let mut foreign = candidates[1].clone();
    foreign.urgency = Some(PriorityValue {
        profile_id: profile.profile_id.clone(),
        profile_version: str_of(mixed, "other_profile_version").to_string(),
        value: 40,
    });
    let mut set = candidates.clone();
    set.push(foreign);
    let err: PriorityError = order_candidates(set).unwrap_err();
    assert_eq!(err.code(), str_of(mixed, "expected_error"));
}

// ---------------------------------------------------------------------------
// authority.managed_section
// ---------------------------------------------------------------------------

fn capability_from(value: &Value) -> ConditionalWriteCapability {
    match value {
        Value::String(s) if s == "unverified" => ConditionalWriteCapability::Unverified,
        Value::Object(map) => ConditionalWriteCapability::Verified {
            evidence: map["verified"].as_str().unwrap().to_string(),
        },
        other => panic!("unknown capability fixture {other}"),
    }
}

fn opt_in_from(value: &Value) -> Option<BestEffortOptIn> {
    if value.is_null() {
        return None;
    }
    Some(BestEffortOptIn {
        approved_by: str_of(value, "approved_by").to_string(),
        acknowledges_human_clobber_risk: value["acknowledges_human_clobber_risk"]
            .as_bool()
            .unwrap(),
    })
}

fn guarantee_code(guarantee: Guarantee) -> &'static str {
    match guarantee {
        Guarantee::Unsupported => "unsupported",
        Guarantee::BestEffortNoCas => "best_effort_no_cas",
        Guarantee::VerifiedConditionalWrite => "verified_conditional_write",
    }
}

fn refusal_codes(refusal: Refusal) -> (&'static str, Option<&'static str>) {
    match refusal {
        Refusal::DeniedUnsupported => ("denied_unsupported", None),
        Refusal::OutsideBytesAltered => ("outside_bytes_altered", None),
        Refusal::ConcurrentEdit => ("concurrent_edit", None),
        Refusal::MalformedMarkers(markers) => (
            "malformed_markers",
            Some(match markers {
                MalformedMarkers::MissingStart => "missing_start",
                MalformedMarkers::MissingEnd => "missing_end",
                MalformedMarkers::DuplicateMarkers => "duplicate_markers",
                MalformedMarkers::EndBeforeStart => "end_before_start",
            }),
        ),
    }
}

fn timeout_code(resolution: TimeoutResolution) -> &'static str {
    match resolution {
        TimeoutResolution::Applied => "applied",
        TimeoutResolution::NotApplied => "not_applied",
        TimeoutResolution::Diverged => "diverged",
        TimeoutResolution::StillUnknown => "still_unknown",
    }
}

#[test]
fn test_adapter_conformance_authority_managed_section_golden() {
    let cases = load("authority/managed_section/cases.json");
    let baseline = str_of(&cases, "baseline_body");
    let new_section = str_of(&cases, "new_section");

    for case in cases["plans"].as_array().unwrap() {
        let name = str_of(case, "name");
        let expected = &case["expected"];
        let capability = capability_from(&case["capability"]);
        let opt_in = opt_in_from(&case["best_effort_opt_in"]);
        let current = str_of(case, "current_body");
        match plan_update(&capability, opt_in.as_ref(), baseline, current, new_section) {
            Ok(plan) => {
                assert_eq!(
                    guarantee_code(plan.guarantee),
                    str_of(expected, "guarantee"),
                    "plan {name}"
                );
                assert_eq!(
                    plan.guarantee.label(),
                    str_of(expected, "label"),
                    "plan {name}"
                );
                assert_eq!(
                    plan.body,
                    str_of(expected, "body"),
                    "plan {name}: body bytes"
                );
            }
            Err(refusal) => {
                let (code, markers) = refusal_codes(refusal);
                assert_eq!(code, str_of(expected, "refusal"), "plan {name}");
                assert_eq!(
                    markers,
                    expected.get("markers").and_then(Value::as_str),
                    "plan {name}: marker detail"
                );
            }
        }
    }

    // The planned body is what a timed-out PATCH would have written.
    let planned = cases["plans"]
        .as_array()
        .unwrap()
        .iter()
        .find_map(|c| c["expected"]["body"].as_str())
        .expect("a planning case carries the planned body");
    for case in cases["timeouts"].as_array().unwrap() {
        let name = str_of(case, "name");
        let resolution =
            reconcile_after_patch_timeout(baseline, new_section, case["current_body"].as_str());
        assert_eq!(
            timeout_code(resolution),
            str_of(case, "expected"),
            "timeout {name}"
        );
        if timeout_code(resolution) == "applied" {
            assert_eq!(case["current_body"].as_str(), Some(planned));
        }
    }
}

// ---------------------------------------------------------------------------
// cursor
// ---------------------------------------------------------------------------

/// Build one HYD-06 observation from fixture wire JSON.
fn activity_record(value: &Value) -> ActivityRecord {
    serde_json::from_value(value.clone()).expect("fixture record is an ActivityRecord")
}

fn record_ids(records: &[ActivityRecord]) -> Vec<&str> {
    records.iter().map(|r| r.native_id.as_str()).collect()
}

fn expected_ids(value: &Value) -> Vec<&str> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().expect("expected id is a string"))
        .collect()
}

#[test]
fn test_adapter_conformance_cursor_refresh_distinguishes_edit_from_creation() {
    let cases = load("cursor/cases.json");
    let refresh = &cases["refresh"];
    let committed: Vec<ActivityRecord> = refresh["committed"]
        .as_array()
        .unwrap()
        .iter()
        .map(activity_record)
        .collect();
    let refetch: Vec<ActivityRecord> = refresh["refetch"]
        .as_array()
        .unwrap()
        .iter()
        .map(activity_record)
        .collect();
    let outcome = refresh_activity(&committed, &refetch);
    let expected = &refresh["expected"];

    // An edit behind the creation watermark is exactly one edit,
    // never a second creation of the same native ID.
    assert_eq!(record_ids(&outcome.edits), expected_ids(&expected["edits"]));
    assert_eq!(
        outcome.edits,
        vec![activity_record(&expected["edit_record"])],
        "the edit carries the new revision, digest and clocks"
    );

    // Only genuinely new native IDs are creations; the duplicate
    // refetch row reports once.
    assert_eq!(
        record_ids(&outcome.creations),
        expected_ids(&expected["creations"])
    );

    // Minimized-first-observation and Present-to-Deleted are
    // retained tombstones, not drops and not creations.
    assert_eq!(
        record_ids(&outcome.tombstones),
        expected_ids(&expected["tombstones"])
    );

    // Re-observation with equal revision, digest and state changes
    // nothing; absence with no tombstone is a reported gap, never
    // an inferred delete.
    assert_eq!(outcome.unchanged_ids, expected_ids(&expected["unchanged"]));
    assert_eq!(
        outcome.visibility_gaps,
        expected_ids(&expected["visibility_gaps"])
    );
    assert!(!outcome.creations.iter().any(|r| r.native_id == "c8"));
    assert!(!outcome.edits.iter().any(|r| r.native_id == "c8"));
    assert!(!outcome.tombstones.iter().any(|r| r.native_id == "c8"));
}

#[test]
fn test_adapter_conformance_cursor_replay_dedupes_revision_not_creation() {
    let cases = load("cursor/cases.json");
    let replay = &cases["replay"];
    let seen: Vec<(String, String)> = replay["seen"]
        .as_array()
        .unwrap()
        .iter()
        .map(|pair| {
            (
                pair[0].as_str().expect("seen key is a string").to_string(),
                pair[1].as_str().expect("seen key is a string").to_string(),
            )
        })
        .collect();
    let delivery: Vec<ActivityRecord> = replay["delivery"]
        .as_array()
        .unwrap()
        .iter()
        .map(activity_record)
        .collect();
    let fresh = dedupe_replay(&seen, &delivery);

    // Reordered redelivery of committed (native ID, revision) pairs
    // commits nothing; the same ID under a new revision passes
    // through exactly once as a change, never a second creation.
    assert_eq!(fresh, vec![activity_record(&replay["expected_fresh"])]);
}

#[test]
fn test_adapter_conformance_cursor_rejects_empty_identity() {
    assert!(ActivityRecord::new("", "r1", "d", "t", "o", "u", ActivityState::Present).is_err());
    assert!(ActivityRecord::new("c1", "", "d", "t", "o", "u", ActivityState::Present).is_err());
}

// ---------------------------------------------------------------------------
// manifest
// ---------------------------------------------------------------------------

/// Every test compiled into this target, from the harness's own list.
fn listed_tests() -> Vec<String> {
    let exe = std::env::current_exe().expect("test binary path");
    let output = std::process::Command::new(exe)
        .args(["--list", "--format", "terse"])
        .output()
        .expect("list this target's tests");
    assert!(output.status.success(), "--list failed: {output:?}");
    String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .filter_map(|line| line.strip_suffix(": test"))
        .map(str::to_string)
        .collect()
}

#[test]
fn test_adapter_conformance_manifest_matches_targets() {
    let manifest = load("manifest.json");
    let listed = listed_tests();
    assert!(!listed.is_empty(), "the harness listed no tests");
    let mut attributed: Vec<String> = manifest["harness_tests"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t.as_str().unwrap().to_string())
        .collect();

    let mut ids = Vec::new();
    for family in manifest["families"].as_array().unwrap() {
        let id = str_of(family, "id");
        assert!(!ids.contains(&id), "family {id} listed twice");
        ids.push(id);
        let tests: Vec<&str> = family["tests"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t.as_str().unwrap())
            .collect();
        match str_of(family, "status") {
            "landed" => {
                assert!(!tests.is_empty(), "landed family {id} names no tests");
                let fixtures = fixtures_dir().join(str_of(family, "fixtures"));
                assert!(
                    fixtures.join("cases.json").is_file() && fixtures.join("README.md").is_file(),
                    "landed family {id} has no fixtures at {}",
                    fixtures.display()
                );
                for test in &tests {
                    assert!(
                        listed.iter().any(|l| l == test),
                        "landed family {id} names {test}, which this target does not contain"
                    );
                    assert!(
                        !attributed.iter().any(|a| a == test),
                        "{test} is counted for more than one family"
                    );
                    attributed.push(test.to_string());
                }
                if let Some(files) = family.get("projector_tests").and_then(Value::as_array) {
                    for file in files {
                        let rel = file.as_str().unwrap();
                        if read_projector(rel).is_some() {
                            assert!(rel.ends_with(".test.ts"), "{rel} is not a bun test file");
                        }
                    }
                }
            }
            "gated" => {
                assert!(
                    tests.is_empty(),
                    "gated family {id} must contribute no tests"
                );
                assert!(
                    family["gating_issue"].as_u64().is_some_and(|n| n > 0),
                    "gated family {id} names no gating issue"
                );
                assert!(
                    family.get("fixtures").is_none(),
                    "gated family {id} has fixtures"
                );
            }
            other => panic!("family {id} has unknown status {other:?}"),
        }
    }

    // Every test in this target belongs to exactly one family or to the
    // harness, so nothing passes uncounted.
    for test in &listed {
        assert!(
            attributed.iter().any(|a| a == test),
            "{test} is in the target but no manifest family counts it"
        );
    }
}
