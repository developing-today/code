#!/usr/bin/env python3
"""Validate research-matrix artifacts structurally; this does not prove their claims."""
import argparse
import datetime as dt
import json
import re
import sys
from pathlib import Path

SCOPE_RE = re.compile(r"[a-z0-9][a-z0-9-]*\Z")
EVIDENCE_STATES = {"supported", "unsupported", "unknown"}
DECISIONS = {"adopt", "defer", "adopt-none", "inconclusive"}
EXPERIMENT_STATES = {"planned", "unmeasured", "measured", "failed"}


def timestamp(value):
    try:
        parsed = dt.datetime.fromisoformat(value.replace("Z", "+00:00"))
        return parsed.tzinfo is not None
    except (AttributeError, TypeError, ValueError):
        return False


def strings(value):
    return isinstance(value, list) and all(isinstance(x, str) and x.strip() for x in value)


def validate_original30(report):
    errors = []
    status = report.get("status")
    if not isinstance(status, str) or status not in {"recovered", "unrecovered"}:
        errors.append("original-30 status must be recovered or unrecovered")
    if not timestamp(report.get("checked_at")):
        errors.append("checked_at must be an ISO-8601 timestamp with timezone")
    bounded = report.get("bounded_scan")
    if not isinstance(bounded, dict) or bounded.get("complete") is not True or not bounded.get("scope") or not strings(bounded.get("methods")):
        errors.append("bounded_scan requires complete=true, scope, and nonempty methods")
    log = report.get("search_log")
    if not isinstance(log, list) or not log:
        errors.append("search_log must document at least one checked source")
    else:
        for index, entry in enumerate(log):
            if not isinstance(entry, dict) or not text_fields(entry, ("source", "result")) or not timestamp(entry.get("checked_at")):
                errors.append(f"search_log[{index}] requires source, result, and timezone-bearing checked_at")
    items = report.get("original_items")
    if not isinstance(items, list):
        errors.append("original_items must be an array")
        items = []
    if status == "recovered":
        if len(items) != 30:
            errors.append("recovered requires exactly 30 source-backed original_items")
        ids = []
        for index, item in enumerate(items):
            if not isinstance(item, dict) or not text_fields(item, ("id", "title", "source_ref")):
                errors.append(f"original_items[{index}] requires id, title, and source_ref")
            else:
                ids.append(item["id"])
        if len(ids) != len(set(ids)):
            errors.append("original item IDs must be unique")
    elif status == "unrecovered":
        if items:
            errors.append("unrecovered result must not fabricate original_items")
        if not isinstance(report.get("reason"), str) or not report["reason"].strip():
            errors.append("unrecovered result requires a bounded-scan disposition reason")
    return errors


def text_fields(record, keys):
    return all(isinstance(record.get(key), str) and record[key].strip() for key in keys)


def validate_matrix(report):
    errors = []
    if report.get("report_kind") != "research-matrix":
        errors.append("report_kind must be research-matrix")
    scope = report.get("research_scope")
    if not isinstance(scope, str) or not SCOPE_RE.fullmatch(scope) or scope == "original-30-provenance-recovery":
        errors.append("research_scope must be a non-original lowercase kebab-case tag")
    if not timestamp(report.get("checked_at")):
        errors.append("checked_at must be an ISO-8601 timestamp with timezone")
    if not isinstance(report.get("question"), str) or not report["question"].strip():
        errors.append("question must be nonempty")

    sources = report.get("source_ledger")
    if not isinstance(sources, list) or not sources:
        errors.append("source_ledger must contain dated source records")
        sources = []
    source_ids = set()
    for i, source in enumerate(sources):
        if not isinstance(source, dict) or not text_fields(source, ("id", "reference")):
            errors.append(f"source_ledger[{i}] requires id and reference")
            continue
        if source["id"] in source_ids:
            errors.append(f"duplicate source_ledger id {source['id']!r}")
        source_ids.add(source["id"])
        if not timestamp(source.get("accessed_at")):
            errors.append(f"source {source['id']}: accessed_at must include timezone")
        if not timestamp(source.get("published_at")) and not (isinstance(source.get("undated_reason"), str) and source["undated_reason"].strip()):
            errors.append(f"source {source['id']}: provide published_at or undated_reason")

    options = report.get("options")
    if not isinstance(options, list) or len(options) < 2:
        errors.append("options must contain at least two alternatives")
        options = []
    option_ids = []
    for i, option in enumerate(options):
        if not isinstance(option, dict) or not text_fields(option, ("id", "label")):
            errors.append(f"options[{i}] requires id and label")
        else:
            option_ids.append(option["id"])
    if len(option_ids) != len(set(option_ids)):
        errors.append("option IDs must be unique")

    criteria = report.get("criteria")
    if not isinstance(criteria, list) or not criteria:
        errors.append("criteria must be a nonempty list")
        criteria = []
    criterion_ids = []
    for i, criterion in enumerate(criteria):
        if not isinstance(criterion, dict) or not text_fields(criterion, ("id", "question")):
            errors.append(f"criteria[{i}] requires id and question")
        else:
            criterion_ids.append(criterion["id"])
    if len(criterion_ids) != len(set(criterion_ids)):
        errors.append("criterion IDs must be unique")

    experiments = report.get("experiments", [])
    if not isinstance(experiments, list):
        errors.append("experiments must be a list")
        experiments = []
    experiment_by_id = {}
    for i, experiment in enumerate(experiments):
        if (not isinstance(experiment, dict) or not text_fields(experiment, ("id", "method"))
            or not isinstance(experiment.get("status"), str) or experiment.get("status") not in EXPERIMENT_STATES):
            errors.append(f"experiments[{i}] requires id, method, and a supported status")
            continue
        eid = experiment["id"]
        if eid in experiment_by_id:
            errors.append(f"duplicate experiment id {eid!r}")
        experiment_by_id[eid] = experiment
        if not timestamp(experiment.get("checked_at")):
            errors.append(f"experiment {eid}: checked_at must include timezone")
        measurements = experiment.get("measurements", [])
        if not isinstance(measurements, list):
            errors.append(f"experiment {eid}: measurements must be a list")
            measurements = []
        if experiment["status"] == "measured":
            if not measurements or not experiment.get("evidence_ref"):
                errors.append(f"measured experiment {eid} needs measurements and evidence_ref")
            for j, measurement in enumerate(measurements):
                if (not isinstance(measurement, dict) or not text_fields(measurement, ("metric", "unit"))
                    or type(measurement.get("value")) not in {int, float}):
                    errors.append(f"measured experiment {eid} measurement[{j}] needs metric, numeric value, and unit")
        elif measurements:
            errors.append(f"unmeasured/planned experiment {eid} must not assert measurements")
        if experiment.get("evidence_ref") and (not isinstance(experiment["evidence_ref"], str) or experiment["evidence_ref"] not in source_ids):
            errors.append(f"experiment {eid}: evidence_ref does not resolve in source_ledger")

    assessments = report.get("assessments")
    if not isinstance(assessments, list):
        errors.append("assessments must be a list")
        assessments = []
    seen_pairs = set()
    experiment_refs = set()
    for i, assessment in enumerate(assessments):
        if not isinstance(assessment, dict) or not text_fields(assessment, ("option_id", "criterion_id", "status", "reason")):
            errors.append(f"assessments[{i}] requires option_id, criterion_id, status, and reason")
            continue
        pair = (assessment["option_id"], assessment["criterion_id"])
        if pair in seen_pairs:
            errors.append(f"duplicate assessment for option/criterion {pair!r}")
        seen_pairs.add(pair)
        if pair[0] not in option_ids or pair[1] not in criterion_ids:
            errors.append(f"assessment {pair!r} refers to unknown option or criterion")
        if assessment["status"] not in EVIDENCE_STATES:
            errors.append(f"assessment {pair!r} status must be supported, unsupported, or unknown")
        refs = assessment.get("evidence_refs")
        if not strings(refs):
            errors.append(f"assessment {pair!r} needs nonempty evidence_refs")
        else:
            for ref in refs:
                if ref not in source_ids:
                    errors.append(f"assessment {pair!r} evidence ref {ref!r} is absent from source_ledger")
        empirical = assessment.get("empirical_claim", False)
        if not isinstance(empirical, bool):
            errors.append(f"assessment {pair!r} empirical_claim must be boolean")
            empirical = False
        if empirical:
            exp_ref = assessment.get("experiment_ref")
            experiment = experiment_by_id.get(exp_ref) if isinstance(exp_ref, str) else None
            if not experiment or experiment.get("status") != "measured" or not experiment.get("measurements"):
                errors.append(f"assessment {pair!r} empirical claim needs a measured experiment_ref and measurements")
            elif experiment.get("evidence_ref") not in source_ids:
                errors.append(f"assessment {pair!r} empirical evidence must resolve to source_ledger")
            experiment_refs.add(exp_ref)
        elif assessment.get("experiment_ref"):
            exp_ref = assessment["experiment_ref"]
            if not isinstance(exp_ref, str) or exp_ref not in experiment_by_id:
                errors.append(f"assessment {pair!r} experiment_ref is unknown")
            else:
                experiment_refs.add(exp_ref)

    expected_pairs = {(option, criterion) for option in option_ids for criterion in criterion_ids}
    if seen_pairs != expected_pairs:
        errors.append(f"assessment matrix incomplete: {len(expected_pairs-seen_pairs)} missing option/criterion pairs")
    for eid, experiment in experiment_by_id.items():
        if experiment.get("status") == "measured" and eid not in experiment_refs:
            errors.append(f"measured experiment {eid} is not referenced by an assessment")

    decision = report.get("decision")
    if (not isinstance(decision, dict) or not isinstance(decision.get("outcome"), str)
        or decision.get("outcome") not in DECISIONS or not text_fields(decision, ("reason",))):
        errors.append("decision needs outcome adopt|defer|adopt-none|inconclusive and a reason")
    if not strings(report.get("limitations")):
        errors.append("limitations must be a nonempty string list")
    return errors


def validate_report(report):
    errors = []
    if not isinstance(report, dict):
        return ["report must be a JSON object"]
    if type(report.get("schema_version")) is not int or report.get("schema_version") != 1:
        errors.append("schema_version must be 1")
    scope = report.get("research_scope")
    if scope == "original-30-provenance-recovery":
        return errors + validate_original30(report)
    return errors + validate_matrix(report)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    parser.add_argument("--input", required=True, type=Path, help="future report/fixture JSON artifact")
    args = parser.parse_args(argv)  # Full strict parse before reading.
    try:
        report = json.loads(args.input.read_text())
    except (OSError, json.JSONDecodeError) as exc:
        print(json.dumps({"valid": False, "errors": [str(exc)]}, sort_keys=True))
        return 1
    errors = validate_report(report)
    print(json.dumps({"valid": not errors, "errors": errors}, sort_keys=True))
    return 0 if not errors else 1


if __name__ == "__main__":
    raise SystemExit(main())
