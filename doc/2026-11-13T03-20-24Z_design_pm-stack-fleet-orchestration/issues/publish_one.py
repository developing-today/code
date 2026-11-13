#!/usr/bin/env python3
"""Prepare/check one-issue PM orchestration batch entry; writes require explicit flags."""

from __future__ import annotations

import argparse
import fcntl
import hashlib
import json
import os
import re
import subprocess
import sys
import tempfile
import time
import unicodedata
from datetime import datetime, timezone
from pathlib import Path


SCRIPT_PATH = Path(os.path.abspath(__file__))
PACKAGE = SCRIPT_PATH.parent.parent
ISSUES = SCRIPT_PATH.parent
SPECS = ISSUES / "specs.jsonl"
INDEX = ISSUES / "index.json"
CODE_REPO = "developing-today/code"
QUEUE = "repo-health-code-pilot"
LABELS = ["app/fleet", "app/pm-stack", "kind/design", "status:to-do"]
REQUIRED_FIELDS = (
    "key", "kind", "parent", "title", "requirements", "requirement_refs", "mapping_gaps", "design_expansion_support", "command_keys", "source_refs", "design_refs",
    "goal", "enables", "owned_files", "readonly_consumes", "hard_blockers", "hard_blocker_reasons", "parent_gate_resolutions", "runtime_issue_gates", "recommended_after", "runtime_gates", "manual_support", "external_requirement_refs",
    "first_failing_test", "oracle", "done_command", "negative_tests", "acceptance",
    "compatibility", "rollback", "version", "horizon", "cost_risk", "labels", "bodypath",
)
VALID_KINDS = {"implementation", "contract", "validation", "migration", "documentation", "research"}
CONTRACT_KEY_ALIASES = {
    "DST-CMD-01": ["DST-01", "CLI-02"],
    "DST-INTENT-01": ["REL-03"],
    "DST-SNAPSHOT-01": ["DST-05"],
    "DST-RECON-01": ["DST-05"],
    "DST-ORDER-01": ["PRI-03"],
    "PRI-PROFILE-01": ["PRI-01"],
    "PRI-CMP-01": ["PRI-03"],
    "PRI-PRESSURE-01": ["PRI-04"],
    "PRI-TRIAGE-01": ["PRI-05a"],
    "DST-REASON-01": ["DST-01"],
    "DEP-EDGE-01": ["DEP-02"],
    "DEP-01": ["CEN-04"],
    "AUTH-OP-01": ["AUTH-01"],
    "DSP-02": ["CLI-02"],
    "DSP-03": ["REL-03"],
    "CEN-03": ["#9241"],
    "QTI-01": ["#9241"],
}
DOMAIN_PARENT_IDS={"#9196","#9198","#9199","#9200","#9201","#9203","#9204","#9206","#9207","#9208","#9209"}
NONBLOCKING_ISSUE_GATES={
    "#9028":"runtime gate: reconcile the current PM PostgREST producer bypass before canonical reservation/cutover; offline contracts and fixtures may proceed",
    "#6533":"separate live launch-and-bind acceptance; never duplicate its live proof in a mocked/design slice",
    "#9178":"upstream claim/manual/takeover/resume semantics must be verified before mutation/cutover; offline compatibility models may proceed",
    "#9179":"upstream park/resume safety semantics must be verified before mutation/cutover; offline fixtures may proceed",
    "#9241":"bounded QTI search receipt is complete but original pointer remains unlocated; compatibility review may proceed from the receipt without waiting for issue closure",
    "#3424":"existing durable-task-model compatibility evidence; do not duplicate its research/intake issue",
    "#8575":"existing one-home-per-fact design evidence; consume, do not duplicate",
    "#8579":"existing schema-audit evidence; consume, do not duplicate",
    "#8602":"existing merged schema-audit PR evidence; consume, do not duplicate",
    "#8187":"closed launch-on-handoff baseline; not new live acceptance",
    "#2668":"existing staged-launch design evidence; not a duplicate issue",
}
GRANULAR_PARENT_GATES={
    "AUTH-01":{"#9206":"CLI-02","#9200":"PRI-01","#9198":"CEN-04"},
    "AUTH-02":{"#9209":"REL-03","#9198":"CEN-04"},
    "AUTH-03":{"#9198":"CEN-04"},
    "AUTH-05":{"#9206":"CLI-02","#9207":"HYD-06"},
    "CEN-07":{"#9200":"PRI-01","#9203":"REL-03","#9207":"HYD-06","#9208":"AUTH-01","#9209":"REL-03"},
    "DEP-02":{"#9206":"CLI-02","#9198":"CEN-04"},
    "DEP-04":{"#9206":"CLI-02","#9198":"CEN-04"},
    "DSP-04":{"#9200":"PRI-01","#9206":"CLI-02"},
    "DSP-05":{"#9206":"CLI-02"},
    "DSP-08":{"#9200":"PRI-01"},
    "DSP-10":{"#9196":None},
    "DST-03":{"#9209":"REL-03"},
    "DST-04":{"#9208":"AUTH-01"},
    "DST-07":{"#9209":"REL-03"},
    "HYD-01":{"#9198":"CEN-04","#9206":"CLI-02"},
    "HYD-02":{"#9206":"CLI-03"},
    "HYD-03":{"#9198":"CEN-04"},
    "HYD-04":{"#9206":"CLI-03","#9208":"AUTH-01","#9198":"CEN-04"},
    "HYD-05":{"#9198":"CEN-04","#9209":"REL-03"},
    "HYD-06":{"#9208":"JRN-01","#9206":"CLI-05"},
    "HYD-07":{"#9209":"REL-03","#9206":"CLI-05"},
    "HYD-08":{"#9206":"CLI-13","#9203":"REL-03"},
    "HYD-09":{"#9198":"CEN-04"},
    "JRN-01":{"#9198":"CEN-04","#9209":"REL-03"},
    "JRN-02":{"#9206":"CLI-02"},
    "JRN-03":{"#9206":"CLI-05","#9207":"HYD-01","#9209":"REL-03"},
    "MOB-02":{"#9200":"PRI-01"},
    "MOB-05":{"#9200":"PRI-01"},
    "MOB-06":{"#9200":"PRI-01"},
    "MOB-07":{"#9203":"DSP-05"},
    "MOB-08":{"#9208":"DEP-02","#9207":"HYD-06"},
    "PRI-08":{"#9198":"CONTEXT_ONLY"},
    "PRI-04":{"#9208":"DEP-03"},
    "REL-01":{"#9198":"CEN-04","#9206":"CLI-01"},
    "REL-03":{"#9206":"CLI-02","#9198":"CEN-04"},
    "REL-05":{"#9208":"AUTH-02"},
    "REL-06":{"#9208":"AUTH-02","#9198":"CEN-04"},
    "REL-07":{"#9207":"HYD-06","#9208":"AUTH-01","#9196":None},
    "REL-08":{"#9206":"CLI-13","#9203":None},
    "REL-09":{"#9207":"HYD-06","#9203":"REL-03","#9208":"AUTH-01"},
    "REL-10M":{"#9198":"CEN-04"},
    "REL-10H":{"#9198":"CEN-04"},
    "SEL-01":{"#9199":"PRI-01","#9200":"DST-01"},
    "SEL-02":{"#9209":"REL-02"},
    "SEL-04":{"#9209":"REL-03"},
    "JRN-04":{"#9198":"CONTEXT_ONLY"},
    "VAL-01":{"#9198":"CEN-04"},
}
OWNED_PATH_OVERRIDES={
    "CLI-01":["pkgs/fleet/crates/fleet-queue/src/work/lifecycle.rs"],
    "CLI-02":["pkgs/fleet/crates/fleet-queue/src/work/cli.rs"],
    "CLI-03":["pkgs/fleet/crates/fleet-queue/src/work/lifecycle.rs"],
    "CLI-04":["pkgs/fleet/crates/fleet-queue/src/work/selection.rs","pkgs/fleet/crates/fleet-queue/src/work/lifecycle.rs"],
    "CLI-05":["pkgs/fleet/crates/fleet-queue/src/work/lifecycle.rs","pkgs/fleet/crates/fleet-queue/src/work/journal.rs"],
    "CLI-12":["pkgs/fleet/crates/fleet-queue/tests/work_link_contract.rs"],
    "DST-01":["pkgs/fleet/crates/fleet-queue/src/work/contracts.rs"],
    "DST-03":["pkgs/fleet/crates/fleet-queue/src/work/distribution.rs"],
    "DST-04":["pkgs/fleet/crates/fleet-queue/src/work/distribution.rs","pkgs/fleet/crates/fleet-queue/src/work/contracts.rs","pm-stack/projector/test/priority-webhook.test.ts"],
    "DST-06":["pkgs/fleet/crates/fleet-queue/src/work/distribution.rs"],
    "DST-07":["pkgs/fleet/crates/fleet-queue/src/work/distribution.rs"],
    "DEP-02":["pkgs/fleet/crates/fleet-queue/src/work/relations.rs"],
    "DEP-03":["pkgs/fleet/crates/fleet-queue/src/work/relations.rs","pkgs/fleet/crates/fleet-queue/src/turso_queue.rs"],
    "DEP-04":["pkgs/fleet/crates/fleet-queue/src/work/relations.rs","pkgs/fleet/crates/fleet-queue/tests/dependency_github_relations.rs"],
    "PRI-01":["pkgs/fleet/crates/fleet-queue/src/work/priority.rs","pm-stack/dispatch/src/validate.ts","pm-stack/dispatch/test/validate.test.ts"],
    "PRI-02":["pkgs/fleet/crates/fleet-queue/src/work/priority.rs"],
    "PRI-03":["pkgs/fleet/crates/fleet-queue/src/work/priority.rs","pkgs/fleet/crates/fleet-queue/src/turso_queue.rs"],
    "PRI-04":["pkgs/fleet/crates/fleet-queue/src/work/priority.rs","pkgs/fleet/crates/fleet-queue/src/work/relations.rs"],
    "PRI-05a":["pkgs/fleet/crates/fleet-queue/src/work/priority.rs"],
    "PRI-05b":["pkgs/fleet/crates/fleet-queue/src/work/priority.rs"],
    "PRI-05c":["pkgs/fleet/crates/fleet-queue/src/work/priority.rs"],
    "PRI-05d":["pkgs/fleet/crates/fleet-queue/src/work/priority.rs"],
    "PRI-07":["pkgs/fleet/crates/fleet-queue/src/work/priority.rs"],
    "SEL-01":["pkgs/fleet/crates/fleet-queue/src/work/selection.rs"],
    "SEL-02":["pkgs/fleet/crates/fleet-queue/src/work/selection.rs","pkgs/fleet/crates/fleet-queue/src/turso_queue.rs"],
    "SEL-03":["pkgs/fleet/crates/fleet-queue/src/work/selection.rs"],
    "SEL-04":["pkgs/fleet/crates/fleet-queue/src/work/selection.rs"],
    "SEL-05":["pkgs/fleet/crates/fleet-queue/src/work/selection.rs"],
    "SEL-06":["pkgs/fleet/crates/fleet-queue/src/work/selection.rs"],
    "SEL-07":["pkgs/fleet/crates/fleet-queue/src/work/selection.rs"],
    "AUTH-01":["pkgs/fleet/crates/fleet-queue/src/work/authority.rs"],
    "AUTH-02":["pkgs/fleet/crates/fleet-queue/src/work/authority.rs"],
    "AUTH-03":["pkgs/fleet/crates/fleet-queue/src/work/authority.rs"],
    "AUTH-04":["pkgs/fleet/crates/fleet-queue/src/work/authority.rs"],
    "AUTH-05":["pkgs/fleet/crates/fleet-queue/src/work/authority.rs"],
    "JRN-01":["pkgs/fleet/crates/fleet-queue/src/work/journal.rs"],
    "JRN-02":["pkgs/fleet/crates/fleet-queue/src/work/journal.rs","pkgs/fleet/crates/fleet-queue/src/work/authority.rs"],
    "JRN-03":["pkgs/fleet/crates/fleet-queue/src/work/journal.rs"],
    "JRN-04":["doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/research/full-history-replay-retention.md"],
    "HYD-01":["pkgs/fleet/crates/fleet-queue/src/work/context.rs"],
    "HYD-02":["pkgs/fleet/crates/fleet-queue/src/work/context.rs"],
    "HYD-03":["pkgs/fleet/crates/fleet-queue/src/work/context.rs","pkgs/fleet/crates/fleet-queue/src/turso_queue.rs"],
    "HYD-04":["pkgs/fleet/crates/fleet-queue/src/work/context.rs"],
    "HYD-05":["pkgs/fleet/crates/fleet-queue/src/work/activity.rs","pkgs/fleet/crates/fleet-queue/src/turso_queue.rs"],
    "HYD-06":["pkgs/fleet/crates/fleet-queue/src/work/activity.rs"],
    "HYD-07":["pkgs/fleet/crates/fleet-queue/src/work/activity.rs"],
    "HYD-08":["pkgs/fleet/crates/fleet-queue/src/work/context.rs","pkgs/fleet/crates/fleet-queue/src/work/activity.rs"],
    "HYD-09":["pkgs/fleet/crates/fleet-queue/src/work/context.rs"],
    "MOB-00":["pm-stack/aggregator/package.json","pm-stack/aggregator/bun.lock","pm-stack/aggregator/playwright.config.ts","pm-stack/aggregator/test/browser/mobile-work-view.spec.ts",".github/workflows/pm-aggregator-tests.yml"],
    "MOB-01":["pm-stack/aggregator/src/model.ts","pm-stack/aggregator/src/work-projection.ts","pm-stack/aggregator/test/work-projection.test.ts"],
    "MOB-02":["pm-stack/aggregator/src/work-lifecycle-policy.ts","pm-stack/aggregator/test/work-lifecycle-policy.test.ts"],
    "MOB-03":["pm-stack/aggregator/src/render.ts","pm-stack/aggregator/src/ui.css","pm-stack/aggregator/src/work-mobile-views.ts","pm-stack/aggregator/test/work-mobile-views.test.ts"],
    "MOB-04":["pm-stack/aggregator/src/ui.css","pm-stack/aggregator/src/render.ts","pm-stack/aggregator/test/mobile-accessibility.test.ts","pm-stack/aggregator/docs/mobile-accessibility.md"],
    "MOB-05":["pm-stack/aggregator/src/actions.ts","pm-stack/aggregator/src/server.ts","pm-stack/aggregator/src/work-action-contract.ts","pm-stack/aggregator/test/work-action-contract.test.ts"],
    "MOB-06":["pm-stack/aggregator/src/actions.ts","pm-stack/aggregator/src/server.ts","pm-stack/aggregator/src/work-action-contract.ts","pm-stack/aggregator/test/mobile-work-actions.test.ts"],
    "MOB-07":["pm-stack/aggregator/src/session-panel.ts","pm-stack/aggregator/test/session-action-receipts.test.ts","pm-stack/aggregator/test/session-capabilities.test.ts"],
    "MOB-08":["pm-stack/aggregator/src/work-relations-activity.ts","pm-stack/aggregator/test/work-dependencies-activity.test.ts"],
    "MOB-09":["pm-stack/aggregator/src/work-operation-reconciliation.ts","pm-stack/aggregator/test/work-operation-reconciliation.test.ts"],
    "MOB-10":["pm-stack/aggregator/test/hosted-mobile-contract.test.ts"],
    "DSP-01":["pm-stack/projector/src/server.ts","pm-stack/projector/test/webhook-operation-forwarding.test.ts","pm-stack/projector/test/webhook-delivery-id.test.ts"],
    "DSP-04":["pkgs/fleet/crates/fleet-queue/src/work/operations.rs","pkgs/fleet/crates/fleet-queue/src/work/relations.rs"],
    "DSP-05":["pkgs/fleet/crates/fleet-queue/src/work/operations.rs"],
    "DSP-06":["pkgs/fleet/crates/fleet-queue/src/work/operations.rs","pkgs/fleet/crates/fleet-dispatch/src/lib.rs"],
    "DSP-07":["pkgs/fleet/crates/fleet-queue/src/work/operations.rs"],
    "DSP-08":["pkgs/fleet/crates/fleet-queue/src/work/operations.rs"],
    "DSP-09":["pkgs/fleet/crates/fleet-queue/src/work/operations.rs"],
    "DSP-10":["pm-stack/dispatch/test/event-route-rollback.test.ts","pm-stack/dispatch/README.md"],
    "REL-01":["pkgs/fleet/crates/fleet-queue/src/work/lifecycle.rs"],
    "REL-02":["pkgs/fleet/crates/fleet-queue/src/work/lifecycle.rs","pkgs/fleet/crates/fleet-queue/src/turso_queue.rs"],
    "REL-03":["pkgs/fleet/crates/fleet-queue/src/work/operations.rs"],
    "REL-04":["pkgs/fleet/crates/fleet-queue/src/work/operations.rs","pkgs/fleet/crates/fleet-queue/src/turso_queue.rs"],
    "REL-05":["pkgs/fleet/crates/fleet-queue/src/work/operations.rs"],
    "REL-06":["pkgs/fleet/crates/fleet/src/gh.rs"],
    "REL-07":["pkgs/fleet/crates/fleet-queue/src/work/completion.rs"],
    "REL-08":["pkgs/fleet/crates/fleet-queue/src/work/completion.rs"],
    "REL-09":["pkgs/fleet/crates/fleet-queue/src/work/completion.rs"],
    "REL-10M":["pkgs/fleet/crates/fleet-queue/src/work/completion.rs","pkgs/fleet/crates/fleet-queue/src/work/operations.rs"],
    "REL-10H":["pkgs/fleet/crates/fleet-queue/tests/hosted_conformance.rs"],
    "REL-11":["doc/fleet/lifecycle-recovery.md"],
    "CEN-01":["doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/census/requirements-crosswalk.jsonl","doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/census/requirements-crosswalk.md","doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/census/check_requirements_crosswalk.py","doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/census/test_requirements_crosswalk.py"],
    "CEN-02":["doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/census/requirements-crosswalk.jsonl","doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/census/requirements-crosswalk.md","doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/census/test_census_evidence.py"],
    "CEN-04":["doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/components/9198-requirement-crosswalk-current-state-census.md","doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/delivery-plan.md","doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/census/test_schema_compatibility_matrix.py"],
    "CEN-05":["doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/validation/experiment.py","doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/validation/fixtures.json","doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/validation/test_experiment.py"],
    "CEN-06":["doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/validation/report.py","doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/validation/test_report.py","doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/research/priority-exploration-experiment.md"],
    "CEN-07":[".github/workflows/pmo-contract-conformance.yml","pkgs/fleet/crates/fleet-queue/tests/pmo_contract_conformance.rs"],
}
READONLY_CONSUMES={
    "CLI-02":["pkgs/fleet/crates/fleet-queue/src/work/contracts.rs"],
    "CLI-03":["pkgs/fleet/crates/fleet-queue/src/work/contracts.rs"],
    "CLI-04":["pkgs/fleet/crates/fleet-queue/src/work/priority.rs"],
    "CLI-05":["pkgs/fleet/crates/fleet-queue/src/work/operations.rs"],
    "CLI-12":["pkgs/fleet/crates/fleet-queue/src/work/contracts.rs","pkgs/fleet/crates/fleet-queue/src/work/authority.rs","pkgs/fleet/crates/fleet-queue/src/work/journal.rs"],
    "DST-03":["pkgs/fleet/crates/fleet-queue/src/work/contracts.rs"],
    "DST-04":["pkgs/fleet/crates/fleet-queue/src/work/contracts.rs","pm-stack/projector/src/webhook.ts"],
    "DST-06":["pkgs/fleet/crates/fleet-queue/src/work/contracts.rs"],
    "DSP-01":["pm-stack/projector/src/webhook.ts"],
    "DSP-04":["pkgs/fleet/crates/fleet-queue/src/work/contracts.rs"],
    "DSP-05":["pkgs/fleet/crates/fleet-queue/src/work/contracts.rs","pm-stack/projector/src/server.ts"],
    "DSP-06":["pkgs/fleet/crates/fleet-queue/src/work/contracts.rs","pkgs/fleet/crates/fleet-queue/src/work/lifecycle.rs"],
    "DSP-07":["pkgs/fleet/crates/fleet-queue/src/work/contracts.rs"],
    "DSP-08":["pkgs/fleet/crates/fleet-queue/src/work/authority.rs"],
    "DSP-09":["pkgs/fleet/crates/fleet-queue/src/work/completion.rs"],
    "CEN-07":["pm-stack/projector/test/webhook-delivery-id.test.ts","pm-stack/projector/src/webhook.ts"],
    "MOB-05":["pkgs/fleet/crates/fleet-queue/src/work/operations.rs","pkgs/fleet/crates/fleet-queue/src/work/authority.rs"],
    "MOB-06":["pkgs/fleet/crates/fleet-queue/src/work/authority.rs","pkgs/fleet/crates/fleet-queue/src/work/priority.rs"],
    "MOB-08":["pkgs/fleet/crates/fleet-queue/src/work/activity.rs"],
    "MOB-09":["pkgs/fleet/crates/fleet-queue/src/work/operations.rs"],
    "AUTH-02":["pkgs/fleet/crates/fleet/src/gh.rs"],
    "AUTH-03":["pkgs/fleet/crates/fleet/src/gh.rs"],
    "AUTH-04":["pkgs/fleet/crates/fleet/src/gh.rs"],
    "DEP-04":["pkgs/fleet/crates/fleet/src/gh.rs"],
    "REL-06":["pkgs/fleet/crates/fleet-queue/src/work/operations.rs"],
}
COMPOSITE_KEY_ALIASES = {
    "PRI-05": ["PRI-05a", "PRI-05b", "PRI-05c", "PRI-05d"],
    "REL-10": ["REL-10M", "REL-10H"],
}
PARENT_APPROVED_SUPPORT_MAP={
    "AUTH-05":{"ids":["C-R121","C-R122","P-R018"],"note":"provenance/trust support for HYD-07's canonical activity classifier; not a second classifier"},
    "CEN-05":{"ids":["C-R104","C-R160","O-R039","H-R046"],"note":"offline reference validation method; no production implementation claim"},
    "CEN-06":{"ids":["H-R048","C-R161","C-R162","O-R039"],"note":"evidence-reporting method separate from experiment/model results"},
    "CEN-07":{"ids":["H-R046","C-R160","C-R162","P-R027"],"note":"adapter/CI validation method; not a literal request for a new CI workflow or service"},
    "DSP-10":{"ids":["H-R046","H-R048","C-R160","C-R162","O-R065"],"note":"rollout, recovery and evidence distinctions; no deployment or schedule authorization"},
    "DST-07":{"ids":["H-R005","P-R026"],"note":"versioned/degraded/partial output method; no invented runtime capability"},
    "JRN-04":{"ids":["P-R006","H-R023"],"note":"design-expansion feasibility research only; also consumes #3424's existing full-history requirement; not a literal full-event-store mandate or parallel DB"},
    "MOB-00":{"ids":["C-R081","C-R160","O-R013"],"note":"browser test harness as an explicit validation method, not a new app/service"},
    "MOB-08":{"ids":["C-R081","O-R012","P-R018","C-R119"],"note":"dependency/activity view integration method over existing projection sources"},
    "MOB-10":{"ids":["H-R005","H-R023"],"note":"hosted compatibility validation method; not a literal adoption of the recovered original's hosted deployment mechanism"},
    "PRI-07":{"ids":["C-R047","H-R046","C-R162"],"note":"shadow/migration/rollback validation only; no priority cutover authorization"},
    "PRI-08":{"ids":["C-R040","C-R105","H-R010"],"note":"design-expansion support for distinct conditional importance-axis mixing; importance remains null/unfabricated until evidence and no urgency derivation"},
}
EXTERNAL_REQUIREMENT_REFS={
    "JRN-04":[{"ref":"developing-today/code#3424","url":"https://github.com/developing-today/code/issues/3424","source":"doc/2026-11-10_design_queue_task_data_model_3424.md:43-46","statement":"Full event history — an append-only events table records every task change with before/after values and actor; past state is reconstructable from the stream.","qualification":"existing #3424 requirement retained for feasibility research only; not assigned a fabricated crosswalk ID and not a mandate to create a parallel event store"}]
}


def parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    modes = p.add_mutually_exclusive_group()
    modes.add_argument("--prepare", action="store_true", help="extract draft issues and bodies from current component appendices")
    modes.add_argument("--check", action="store_true", help="validate local manifest only (default; no writes/network)")
    modes.add_argument("--apply", metavar="KEY", help="explicitly publish exactly one eligible canonical issue and triage task")
    modes.add_argument("--apply-edges", metavar="KEY", help="explicitly apply reviewed native blocker edges for one already-published issue")
    modes.add_argument("--reconcile", metavar="KEY", help="read-only reconcile an uncertain marker/queue receipt")
    return p


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    p = parser()
    args = p.parse_args(argv)
    if not (args.prepare or args.check or args.apply or args.apply_edges or args.reconcile):
        args.check = True
    # Preserve manifest-key case exactly: real canonical keys include PRI-05a..d.
    # Exact membership is checked against specs.jsonl only after complete argv parsing.
    if args.apply is not None and not args.apply:
        p.error("--apply requires one exact manifest key")
    if args.reconcile is not None and not args.reconcile:
        p.error("--reconcile requires one exact manifest key")
    if args.apply_edges is not None and not args.apply_edges:
        p.error("--apply-edges requires one exact manifest key")
    return args


def utcnow() -> str:
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def parse_jsonl(path: Path) -> list[dict]:
    result = []
    for no, line in enumerate(path.read_text().splitlines(), 1):
        if not line.strip():
            continue
        try:
            row = json.loads(line)
        except json.JSONDecodeError as exc:
            raise ValueError(f"{path}:{no}: invalid JSON: {exc}") from exc
        if not isinstance(row, dict):
            raise ValueError(f"{path}:{no}: each JSONL record must be an object")
        result.append(row)
    return result


def github_heading_slugs(markdown: str) -> set[str]:
    slugs:set[str]=set()
    counts:dict[str,int]={}
    for line in markdown.splitlines():
        m=re.match(r"^#{1,6}\s+(.+?)\s*#*\s*$",line)
        if not m:
            continue
        heading=re.sub(r"[`*_~]", "", m.group(1))
        heading=re.sub(r"\[([^\]]+)\]\([^)]*\)",r"\1",heading)
        chars=[]
        for ch in heading.casefold():
            cat=unicodedata.category(ch)
            if ch.isspace():
                chars.append("-")
            elif ch=="-" or cat[0] in {"L","N","M"}:
                chars.append(ch)
        base="".join(chars).strip("-")
        n=counts.get(base,0)
        counts[base]=n+1
        slugs.add(base if n==0 else f"{base}-{n}")
    return slugs


def validate_design_ref(ref: str) -> bool:
    path_text,sep,fragment=ref.partition("#")
    if not path_text or ".." in Path(path_text).parts:
        return False
    path=PACKAGE/path_text
    if not path.is_file():
        return False
    if not sep:
        return True  # file-level ref for a DRAFT slice with no exact curated design anchor yet
    return bool(fragment) and fragment in github_heading_slugs(path.read_text())


def write_atomic(path: Path, payload: bytes) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, name = tempfile.mkstemp(prefix=f".{path.name}.", suffix=".tmp", dir=path.parent)
    try:
        with os.fdopen(fd, "wb") as f:
            f.write(payload)
            f.flush()
            os.fsync(f.fileno())
        os.replace(name, path)
        dfd = os.open(path.parent, os.O_RDONLY)
        try:
            os.fsync(dfd)
        finally:
            os.close(dfd)
    finally:
        if os.path.exists(name):
            os.unlink(name)


def read_index() -> dict:
    if not INDEX.exists():
        return {"schema": "pmo-issue-index/v1", "existing": {}, "operations": {}, "last_create_at": None}
    data = json.loads(INDEX.read_text())
    if data.get("schema") != "pmo-issue-index/v1":
        raise ValueError("unsupported index schema")
    return data


def save_index(index: dict) -> None:
    write_atomic(INDEX, (json.dumps(index, indent=2, sort_keys=True) + "\n").encode())


class DirectoryLock:
    """Local single-writer lock on the stable issues directory inode (no lock file)."""
    def __init__(self, exclusive: bool):
        self.exclusive=exclusive
        self.fd=None
    def __enter__(self):
        self.fd=os.open(ISSUES,os.O_RDONLY)
        fcntl.flock(self.fd,fcntl.LOCK_EX if self.exclusive else fcntl.LOCK_SH)
        return self
    def __exit__(self,*exc):
        if self.fd is not None:
            fcntl.flock(self.fd,fcntl.LOCK_UN)
            os.close(self.fd)
            self.fd=None


def specs_and_aliases() -> tuple[list[dict], dict[str, dict]]:
    rows = parse_jsonl(SPECS)
    canon: dict[str, dict] = {}
    aliases: dict[str, dict] = {}
    for row in rows:
        missing = [f for f in REQUIRED_FIELDS if f not in row]
        if missing:
            raise ValueError(f"{row.get('key','?')}: missing fields {missing}")
        key = row["key"]
        if key in canon or key in aliases:
            raise ValueError(f"duplicate key {key}")
        if row.get("alias_of"):
            aliases[key] = row
            continue
        canon[key] = row
        if row["kind"] not in VALID_KINDS:
            raise ValueError(f"{key}: invalid kind {row['kind']}")
        if row["requirements"] and row.get("requirements_status") != "ALLOCATED":
            raise ValueError(f"{key}: nonempty requirement IDs must be explicitly marked ALLOCATED")
        if not row["requirements"] and row.get("requirements_status") != "DRAFT-unallocated":
            raise ValueError(f"{key}: empty requirement IDs must be explicit DRAFT-unallocated")
        marker = f"<!-- pmo-9196:{key} -->"
        body = (ISSUES / row["bodypath"]).read_text()
        if marker not in body or row["title"] not in body:
            raise ValueError(f"{key}: body missing exact durable marker or title")
        if any(label not in LABELS for label in row["labels"]):
            raise ValueError(f"{key}: labels must reuse approved existing labels only")
        if not row["source_refs"] or not row["design_refs"] or not row["owned_files"]:
            raise ValueError(f"{key}: missing source/design/owned-file trace")
        if not row["first_failing_test"] or not row["oracle"] or not row["done_command"]:
            raise ValueError(f"{key}: missing decidable test/oracle/done command")
        if row["version"] not in {"V2", "V3", "V4"}:
            raise ValueError(f"{key}: invalid version")
    if not canon:
        raise ValueError("manifest has no canonical issue specs")
    for alias, row in aliases.items():
        if row["alias_of"] not in canon and not row["alias_of"].startswith(("existing:", "composite:")):
            raise ValueError(f"{alias}: alias target is neither a canonical key nor explicit external/composite ref")
    return rows, canon


def readiness_report(rows: list[dict], canon: list[dict]) -> list[str]:
    problems: list[str] = []
    xwalk = PACKAGE / "census/requirements-crosswalk.jsonl"
    command_map = PACKAGE / "census/command-slice-map.jsonl"
    command_inventory = PACKAGE / "census/commands.jsonl"
    if not xwalk.exists():
        problems.append("requirements-crosswalk.jsonl not yet integrated; requirements remain DRAFT-unallocated")
    if not command_map.exists():
        problems.append("command-slice-map.jsonl not yet integrated; CLI coverage cannot be applied")
    if not command_inventory.exists():
        problems.append("commands.jsonl unavailable; cannot assert exact command-key coverage")
    else:
        inventory = parse_jsonl(command_inventory)
        inv_keys = {row.get("key") for row in inventory}
        if None in inv_keys:
            problems.append("commands.jsonl contains a row without key")
        elif command_map.exists():
            mapping = parse_jsonl(command_map)
            map_keys = [row.get("key") for row in mapping]
            if set(map_keys) != inv_keys or len(map_keys) != len(set(map_keys)):
                problems.append("command-slice-map keys do not exactly equal the command inventory")
            else:
                try:
                    expected,stats=command_key_join(canon,[r for r in rows if r.get("alias_of")])
                    by_key={r["key"]:r for r in canon}
                    for key,command_keys in expected.items():
                        if by_key[key].get("command_keys") != command_keys:
                            problems.append(f"{key}: command_keys differ from exact allocation manifest")
                    aliases={r["key"]:r for r in rows if r.get("alias_of")}
                    for alias,arow in aliases.items():
                        expected_alias=sorted(r["key"] for r in mapping if alias in [r.get("primary_slice"),*r.get("supporting_slices",[])])
                        if arow.get("command_keys") != expected_alias:
                            problems.append(f"{alias}: alias command_keys differ from exact allocation manifest")
                except ValueError as exc:
                    problems.append(str(exc))
    if xwalk.exists():
        ids = {r.get("id", r.get("requirement_id")) for r in parse_jsonl(xwalk)}
        if not ids or None in ids:
            problems.append("requirements crosswalk has no recognized stable id fields")
        else:
            for row in canon:
                if not row["requirements"] or not set(row["requirements"]) <= ids:
                    problems.append(f"{row['key']}: missing/inexact curated requirement IDs")
                if row.get("mapping_gaps"):
                    problems.append(f"{row['key']}: unresolved curated-slice mapping gaps")
                if any(not validate_design_ref(ref) for ref in row.get("design_refs",[])):
                    problems.append(f"{row['key']}: design_refs contain a missing file/actual heading fragment")
            try:
                joined=requirement_key_join(canon,[r for r in rows if r.get("alias_of")])
                if joined["gaps"]:
                    problems.append(f"curated requirement crosswalk contains {len(joined['gaps'])} unresolved slice/ID mappings")
                if joined["design_ref_errors"]:
                    problems.append(f"curated crosswalk has {len(joined['design_ref_errors'])} missing actual design heading anchors")
            except ValueError as exc:
                problems.append(str(exc))
    return problems


def graph_and_collision_problems(canon: list[dict]) -> tuple[list[str],dict]:
    by_key={row["key"]:row for row in canon}
    graph={key:[] for key in by_key}
    problems=[]
    edge_count=0
    for key,row in by_key.items():
        for dep in row.get("hard_blockers",[]):
            if dep in DOMAIN_PARENT_IDS:
                problems.append(f"{key}: whole domain/umbrella issue {dep} must not be a native hard blocker")
            elif dep in graph:
                graph[key].append(dep); edge_count+=1
            elif re.fullmatch(r"[A-Z][A-Za-z0-9-]*",dep):
                problems.append(f"{key}: unresolved local hard blocker {dep}")
    color={key:0 for key in graph}
    stack=[]
    cycles=[]
    def visit(node: str) -> None:
        color[node]=1; stack.append(node)
        for nxt in graph[node]:
            if color[nxt]==0:
                visit(nxt)
            elif color[nxt]==1:
                cycles.append(stack[stack.index(nxt):]+[nxt])
        stack.pop(); color[node]=2
    for key in graph:
        if color[key]==0: visit(key)
    if cycles:
        problems.append("hard-blocker graph cycles: "+"; ".join("→".join(c) for c in cycles[:8]))
    reach_cache={}
    def reaches(start: str, goal: str) -> bool:
        if (start,goal) in reach_cache: return reach_cache[(start,goal)]
        seen=set(); todo=list(graph[start])
        while todo:
            nxt=todo.pop()
            if nxt==goal:
                reach_cache[(start,goal)]=True; return True
            if nxt not in seen:
                seen.add(nxt); todo.extend(graph[nxt])
        reach_cache[(start,goal)]=False; return False
    owners:dict[str,list[str]]={}
    for key,row in by_key.items():
        for path in row.get("owned_files",[]):
            owners.setdefault(path,[]).append(key)
            if " " in path and not path.startswith("https://"):
                problems.append(f"{key}: owned_files entry is not an exact path: {path}")
    collisions=[]
    for path,keys in owners.items():
        uniq=sorted(set(keys))
        if len(uniq)<2: continue
        for i,left in enumerate(uniq):
            for right in uniq[i+1:]:
                if not reaches(left,right) and not reaches(right,left):
                    collisions.append({"path":path,"issues":[left,right]})
    if collisions:
        problems.append("shared-file collisions lack an acyclic hard sequencing edge: "+"; ".join(f"{x['path']} ({x['issues'][0]}/{x['issues'][1]})" for x in collisions[:12]))
    return problems,{"local_hard_edges":edge_count,"local_cycles":len(cycles),"unserialized_file_collisions":len(collisions)}


def stable_topological_order(canon: list[dict]) -> list[str]:
    """Prerequisites first; contract/type interfaces first among ready nodes, then stable key order."""
    by_key={row["key"]:row for row in canon}
    deps={key:{d for d in row.get("hard_blockers",[]) if d in by_key} for key,row in by_key.items()}
    dependents={key:set() for key in by_key}
    for key,needs in deps.items():
        for need in needs: dependents[need].add(key)
    priority=lambda key:(0 if by_key[key].get("kind") in {"contract"} else 1,key)
    ready=sorted((key for key,needs in deps.items() if not needs),key=priority)
    order=[]
    while ready:
        key=ready.pop(0); order.append(key)
        for later in sorted(dependents[key]):
            deps[later].discard(key)
            if not deps[later] and later not in order and later not in ready:
                ready.append(later); ready.sort(key=priority)
    if len(order)!=len(by_key):
        cyclic=sorted(set(by_key)-set(order))
        raise ValueError(f"cannot serialize shared files until hard-blocker graph is acyclic: {cyclic}")
    return order


def add_shared_file_sequences(records: list[dict], body_files: dict[str,str]) -> dict:
    """Serialize only adjacent actual writers per exact path using one stable semantic topological order."""
    canonical=[r for r in records if not r.get("alias_of")]
    by_key={r["key"]:r for r in canonical}
    order=stable_topological_order(canonical)
    rank={key:i for i,key in enumerate(order)}
    writers:dict[str,set[str]]={}
    for row in canonical:
        for path in row.get("owned_files",[]):
            writers.setdefault(path,set()).add(row["key"])
    added=[]
    for path,keys in sorted(writers.items()):
        ordered=sorted(keys,key=lambda k:rank[k])
        for previous,current in zip(ordered,ordered[1:]):
            reason={"key":previous,"reason":f"shared-file:{path}"}
            if previous not in by_key[current]["hard_blockers"]:
                by_key[current]["hard_blockers"].append(previous)
                added.append({"blocked":current,"blocker":previous,"reason":f"shared-file:{path}"})
            else:
                added.append({"blocked":current,"blocker":previous,"reason":f"shared-file:{path}","coalesced_with_existing_direct_edge":True})
            existing_reasons=by_key[current].setdefault("hard_blocker_reasons",[])
            if reason not in existing_reasons:
                existing_reasons.append(reason)
    # Every edge points from a later node to an earlier node in the same computed order, so this
    # second topological check proves the emitted graph remains acyclic.
    final_order=stable_topological_order(canonical)
    for row in canonical:
        reasons=row.get("hard_blocker_reasons",[])
        if reasons:
            section="\n\n## Shared-file sequencing\n\nAdded hard sequencing edges (the predecessor owns the same exact writable file):\n"+"\n".join(f"- blocked by `{e['key']}` — `{e['reason']}`" for e in reasons)+"\n"
            body_files[row["bodypath"]]+=section
    return {"initial_topological_order":order,"final_topological_order":final_order,"added_edges":added,"writer_paths":len(writers)}


def source_inventory() -> dict[str, tuple[str, Path]]:
    return {
        "9198": ("#9198 crosswalk/census", PACKAGE / "components/9198-requirement-crosswalk-current-state-census.md"),
        "9199": ("#9199 distributed priority", PACKAGE / "components/9199-distributed-priority.md"),
        "9200": ("#9200 priority distribution", PACKAGE / "components/9200-priority-distribution.md"),
        "9201": ("#9201 PM projection/mobile", PACKAGE / "components/9201-pm-projection-mobile-control.md"),
        "9203": ("#9203 direct dispatch/events", PACKAGE / "components/9203-direct-dispatch-slash-commands-event-routing.md"),
        "9204": ("#9204 queue selection/exploration", PACKAGE / "components/9204-queue-selection-bounded-exploration.md"),
        "9206": ("#9206 lifecycle/CLI", PACKAGE / "components/9206-work-item-lifecycle-fleet-subcommands.md"),
        "9207": ("#9207 hydration/activity", PACKAGE / "components/9207-context-hydration-incremental-activity.md"),
        "9208": ("#9208 relations/journal/authority", PACKAGE / "components/9208-typed-dependencies-notes-journal-field-authority.md"),
        "9209": ("#9209 lease/outbox/completion", PACKAGE / "components/9209-leases-fencing-outbox-direct-completion.md"),
    }


def clean_cells(line: str) -> list[str]:
    return [c.strip() for c in line.strip().strip("|").split("|")]


def source_candidates() -> list[dict]:
    starters = source_inventory()
    parents = {"9198": 9198, "9199": 9199, "9200": 9200, "9201": 9201, "9203": 9203, "9204": 9204, "9206": 9206, "9207": 9207, "9208": 9208, "9209": 9209}
    candidates: list[dict] = []
    for starter, (label, path) in starters.items():
        text = path.read_text()
        # Heading-based specs carry their own prose section, stopping before next sibling/spec section.
        for i, line in enumerate(text.splitlines()):
            m = re.match(r"^###\s+([A-Z][A-Z0-9-]+)\s+—\s+(.+)$", line)
            if m:
                key, title = m.group(1), m.group(2).strip()
                if not re.fullmatch(r"(?:PRI|DST|MOB|DSP|SEL|CLI|HYD|DEP|JRN|AUTH|VAL|DOC|REL)-\d+[A-Za-z]?", key):
                    continue
                if key in {"PRI-05", "REL-10", "CEN-03", "CEN-08"}:
                    continue
                block = [line]
                for nxt in text.splitlines()[i + 1 :]:
                    if nxt.startswith("### ") or nxt.startswith("## "):
                        break
                    block.append(nxt)
                candidates.append({"starter": starter, "parent": parents[starter], "key": key, "title": title, "raw": "\n".join(block).strip(), "source_path": path.relative_to(PACKAGE).as_posix(), "source_label": label})
        # Table-based appendices: each row is a bounded candidate with an explicit local key.
        if starter in {"9199", "9200", "9204"}:
            for line in text.splitlines():
                cells = clean_cells(line)
                if not cells:
                    continue
                m = re.search(r"`((?:PRI|DST|SEL)-\d+)`(?:\s*/\s*\*\*?([^*]+)\*\*?)?", cells[0])
                if not m:
                    continue
                key = m.group(1)
                if key in {"PRI-05","PRI-06"}:
                    continue
                summary = cells[1] if len(cells) > 1 else "bounded contract slice"
                # The matrix's first cell is tier, not title; use its distinct deliverable summary.
                titlecell = summary.split(";",1)[0].strip().rstrip(".")
                if titlecell:
                    titlecell=titlecell[0].upper()+titlecell[1:]
                test_lines=[x for x in text.splitlines() if key in x and ("cargo test" in x or "Failing test" in x or "test:" in x.lower())]
                raw=line + ("\n" + "\n".join(test_lines) if test_lines else "")
                candidates.append({"starter": starter, "parent": parents[starter], "key": key, "title": titlecell or key, "raw": raw, "source_path": path.relative_to(PACKAGE).as_posix(), "source_label": label, "table_summary": summary, "table_cells": cells})
        if starter == "9206":
            # Full V3 proposal table supplies CLI-01..12's bounded scope, oracle, and command.
            for line in text.splitlines():
                cells = clean_cells(line)
                if not cells:
                    continue
                m = re.match(r"`(CLI-\d+)`\s+(.+)", cells[0])
                if m and len(cells) >= 6:
                    key = m.group(1)
                    if key in {f"CLI-{i:02}" for i in (6, 7, 8, 9, 10, 11)}:
                        continue
                    candidates.append({"starter": starter, "parent": parents[starter], "key": key, "title": m.group(2).strip(), "raw": line, "source_path": path.relative_to(PACKAGE).as_posix(), "source_label": label, "table_summary": cells[2], "table_cells": cells})
            in_table = False
            for line in text.splitlines():
                if line.startswith("| Slice | Owned seam"):
                    in_table = True
                    continue
                if in_table and not line.startswith("|"):
                    in_table = False
                if not in_table:
                    continue
                cells = clean_cells(line)
                if cells and re.fullmatch(r"`CLI-\d+`", cells[0]):
                    key = cells[0].strip("`")
                    if key in {f"CLI-{i:02}" for i in (6, 7, 8, 9, 10, 11)}:
                        continue
                    candidates.append({"starter": starter, "parent": parents[starter], "key": key, "title": f"Fleet work {key}: {cells[1]}", "raw": line, "source_path": path.relative_to(PACKAGE).as_posix(), "source_label": label, "table_summary": cells[1]})
    # Component specs are unique after table/heading extraction; source appendices overlap aliases intentionally.
    by_key: dict[str, dict] = {}
    for row in candidates:
        old = by_key.get(row["key"])
        if old and old["raw"] != row["raw"]:
            # Prefer the full prose spec over a summary table row.
            if len(row["raw"]) > len(old["raw"]):
                by_key[row["key"]] = row
        else:
            by_key[row["key"]] = row
    # DEP-05's CLI registration deliverable is the one canonical CLI-13 issue, not a second owner.
    by_key.pop("DEP-05", None)
    # Shared strict parser and operation-idempotency ledger have one implementation owner each.
    by_key.pop("DSP-02", None)
    by_key.pop("DSP-03", None)
    # Current #9198 compatibility research owns the overlapping #3424/#8579 evidence sweep;
    # preserve DEP-01 as an alias of its compatibility matrix rather than duplicate research.
    by_key.pop("DEP-01", None)
    # CEN-03 explicitly reuses the already-filed QTI provenance issue #9241.
    by_key.pop("CEN-03", None)
    # Split large triage bundle and combined migration/hosted validation into independent deliverables.
    pri05 = next((r for r in candidates if r["key"] == "PRI-05"), None)
    # The source table row is recovered from the original document even though the composite isn't filed.
    if pri05 is None:
        p = starters["9199"][1]
        line = next((x for x in p.read_text().splitlines() if "`PRI-05`" in x), "")
        pri05 = {"starter": "9199", "parent": 9199, "key": "PRI-05", "title": "Aging, opportunity expiry, hysteresis, bounded sweeps and clump refinement", "raw": line, "source_path": p.relative_to(PACKAGE).as_posix(), "source_label": starters["9199"][0]}
    for suffix, title, detail in (
        ("a", "Evidence-based bidirectional aging and review windows", "Rise and de-escalation use meaningful triage age, freshness, pivots, effort, deadlines and evidence; age alone never escalates to Critical."),
        ("b", "Temporary opportunity boost expiry and retriage", "Temporary boost has explicit start/expiry and expiryAction=retriage; expiry removes only the temporary factor and retains searchable cold backlog."),
        ("c", "Priority hysteresis and bounded cooldown", "Hysteresis is a bounded buffer/cooldown/confidence rule with authorized manual and verified incident/deadline bypass; no unbounded drift."),
        ("d", "Bounded clump refinement with evidence and provenance", "Compare cohort candidates and apply at most one ordinary move per item per sweep; source 50→rough 25/10/5/singles remains illustration, not mandated group sizes."),
    ):
        key = "PRI-05" + suffix
        by_key[key] = {**pri05, "key": key, "title": title, "raw": f"{pri05['raw']}\n\nSplit refinement: {detail}", "table_summary": detail}
    # The original composite remains an alias/composite, not a duplicate fifth issue.
    by_key.pop("PRI-05", None)
    rel10 = next((r for r in candidates if r["key"] == "REL-10"), None)
    if rel10 is None:
        p=starters["9209"][1]
        lines=p.read_text().splitlines()
        at=next((i for i,line in enumerate(lines) if line.startswith("### REL-10 —")),None)
        if at is not None:
            block=[lines[at]]
            for line in lines[at+1:]:
                if line.startswith("### ") or line.startswith("## "):
                    break
                block.append(line)
            rel10={"starter":"9209","parent":9209,"key":"REL-10","title":lines[at].split("—",1)[1].strip(),"raw":"\n".join(block),"source_path":p.relative_to(PACKAGE).as_posix(),"source_label":starters["9209"][0]}
    if rel10:
        by_key.pop("REL-10", None)
        for suffix, title, detail in (
            ("M", "Migration, backfill, and rollback validation", "Validate additive migration, unknown legacy state, mixed-version behavior, rollback after remote effect, and retained pending receipts only after #9198 names the actual migration owner/command."),
            ("H", "Hosted contract conformance for operation recovery", "Compare state-machine, stale-fence, operation and completion fixtures across existing hosted targets #9079/#9081; do not reimplement either hosted service."),
        ):
            by_key["REL-10" + suffix] = {**rel10, "key": "REL-10" + suffix, "title": title, "raw": f"{rel10['raw']}\n\nSplit refinement: {detail}", "table_summary": detail}
    by_key["PRI-08"]={"starter":"9199","parent":9199,"key":"PRI-08","title":"Importance-axis mixing and calibration options research","raw":"""### PRI-08 — Importance-axis mixing and calibration options research

**Tier:** research only. **Goal:** compare independent importance/impact signals and future profile-mixing options against the existing low-is-sooner urgency taxonomy; specify evidence, data sufficiency and a null/unavailable behavior. Importance must never be inferred from urgency. No numeric weights, live taxonomy change, schema, or production implementation is selected by this research.
**Owned files:** `doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/research/importance-axis-research.md` (future research deliverable only).
**Hard blockers:** none for offline research; persisted fields remain gated by #9198/QTI and #9073/#9074 owners.
**First failing oracle:** `pmo_importance_axis_prior_art_comparison_is_reproducible` checks the research artifact contains a source-traceable alternatives matrix, evidence/limitations, a null-data path and explicit decision not to infer importance from urgency.
**Done command:** `python3 -I -c 'from pathlib import Path; s=Path("doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/research/importance-axis-research.md").read_text(); assert all(x in s for x in ("alternatives", "prior art", "null", "not inferred from urgency", "limitations"))'`.
**Acceptance:** use prior-art A/B receipts, current `#7030` taxonomy and source profile examples; distinguish fact/inference/anecdote, define falsifiable measurement needs and no-data behavior; mark all proposed weights DRAFT. **Negative:** copy 450/1000 into live priority, derive impact from urgency, assert simulated production benefit, or claim unlocated QTI equivalence. **Rollback:** research artifact only; no schema or runtime change.
""","source_path":"components/9199-distributed-priority.md","source_label":"#9199 distributed priority","table_summary":"Separate importance research from urgency implementation; no inferred score or weights."}
    by_key["JRN-04"]={"starter":"9208","parent":9208,"key":"JRN-04","title":"Full-history replay and bounded retention feasibility research","raw":"""### JRN-04 — Full-history replay and bounded retention feasibility research

**Tier:** research only. **Goal:** investigate the source requirement for full event/history replay against current queue `events`, `result`, `blocked_deps`, #3424 six-piece durable task model and #8579/#8602 schema audit; define what can be reconstructed, what is merely telemetry/observation, and bounded retention/redaction/compaction alternatives. Do not create a second event database or claim full event sourcing.
**Owned files:** `doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/research/full-history-replay-retention.md` (future research deliverable only).
**Hard blockers:** none for read-only research; coordinate with DEP-01 and #9198 crosswalk, but do not duplicate the worker's #3424/#8579 candidate investigation.
**Recommended after:** DEP-01 crosswalk review where available.
**First failing oracle:** `pmo_history_replay_evidence_matrix_distinguishes_observation_from_authority` checks every record source is classified as authoritative mutation, observation, telemetry, projection or unknown, and every replay claim has primary-source support.
**Done command:** `python3 -I -c 'from pathlib import Path; s=Path("doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/research/full-history-replay-retention.md").read_text(); assert all(x in s for x in ("retention", "replay", "telemetry", "unknown", "alternatives", "prior art"))'`.
**Acceptance:** use Track A/B prior-art receipts and historical issue/doc sources; record source pointer, accessible surfaces, limits, retention/redaction and rollback implications, migration evidence requirements and alternatives including no full replay. Never infer immutable domain-journal semantics from an `events` table name. **Negative:** reopen/alter #3424/#8579 research ownership, equate matching schema shapes, invent unbounded retention, or claim deployment. **Rollback:** research only.
""","source_path":"components/9208-typed-dependencies-notes-journal-field-authority.md","source_label":"#9208 relations/journal/authority","table_summary":"Research full replay limits/retention against existing owner evidence; no new store."}
    cen_path=starters["9198"][1]
    cen_text=cen_path.read_text()
    cen_lines=cen_text.splitlines()
    for ckey in ("CEN-01","CEN-02","CEN-04","CEN-05","CEN-06","CEN-07"):
        marker=re.compile(rf"^### {re.escape(ckey)} — (.+)$")
        pos=next((i for i,line in enumerate(cen_lines) if marker.match(line)),None)
        if pos is None:
            raise ValueError(f"9198 component is missing {ckey} spec")
        title=marker.match(cen_lines[pos]).group(1)
        block=[cen_lines[pos]]
        for line in cen_lines[pos+1:]:
            if line.startswith("### ") or line.startswith("## "):
                break
            block.append(line)
        by_key[ckey]={"starter":"9198","parent":9198,"key":ckey,"title":title,"raw":"\n".join(block),"source_path":cen_path.relative_to(PACKAGE).as_posix(),"source_label":"#9198 crosswalk/census","table_summary":title}
    return sorted(by_key.values(), key=lambda x: x["key"])


def find_value(text: str, names: tuple[str, ...]) -> str:
    for name in names:
        m = re.search(rf"\*\*{re.escape(name)}[^*]*:\*\*\s*(.+?)(?=\s+\*\*[A-Z][^*]*:\*\*|$)", text, re.S)
        if m:
            return re.sub(r"\s+", " ", m.group(1)).strip()
    return ""


def make_record(src: dict) -> tuple[dict, str]:
    key, starter = src["key"], src["starter"]
    raw = src["raw"]
    folder = Path("bodies") / f"{key}.md"
    section_goal = find_value(raw, ("Goal", "Goal/deliverable", "Goal/decision", "Goal / deliverable"))
    if not section_goal:
        section_goal = src.get("table_summary", src["title"])
    owned = re.findall(r"`((?:pkgs|pm-stack|doc|planning|census|validation)/[^`]+|(?:fleet-queue/)?(?:src|test|work)/[^`]+)`", raw)
    normalized=[]
    for path in owned:
        if path.startswith("fleet-queue/"):
            path="pkgs/fleet/crates/"+path
        elif path.startswith("work/") and src["starter"]=="9206":
            path="pkgs/fleet/crates/fleet-queue/src/"+path
        elif path.startswith(("src/","test/")) and src["starter"]=="9201":
            path="pm-stack/aggregator/"+path
        elif path.startswith(("src/","test/")) and src["starter"]=="9203":
            path="pm-stack/dispatch/"+path
        elif path.startswith(("src/","test/")) and src["starter"] in {"9200","9204","9207","9208","9209"}:
            path="pkgs/fleet/crates/fleet-queue/"+path
        normalized.append(path)
    owned=normalized
    if not owned:
        owned = ["doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/issues/bodies/" + key + ".md (issue specification only)"]
    test = re.findall(r"`([A-Za-z][A-Za-z0-9_]+)`", raw)
    first_test = find_value(raw, ("First failing test", "Failing test to start", "Failing test", "First failing oracle"))
    if first_test:
        named=re.search(r"`([^`]+)`",first_test)
        first_test=named.group(1).strip() if named else re.sub(r"^(?:add|write|create)\s+", "", first_test, flags=re.I).strip()
    if not first_test:
        # In matrix appendices, the exact test command line enumerates keys in order;
        # select the filter that follows this key, not the first filter in the full line.
        for line in raw.splitlines():
            if key in line:
                after=line.split(key,1)[1]
                mt=re.search(r"\b(pmo_[A-Za-z0-9_]+|work_[A-Za-z0-9_]+|dependency_[A-Za-z0-9_]+|priority_[A-Za-z0-9_]+|session_[A-Za-z0-9_]+|hosted_[A-Za-z0-9_]+|mobile_[A-Za-z0-9_]+|fleet_[A-Za-z0-9_]+|slash_[A-Za-z0-9_]+|activity_[A-Za-z0-9_]+)\b",after)
                if mt:
                    first_test=mt.group(1); break
    if not first_test:
        first_test = next((t for t in test if t.startswith(("pmo_", "work_", "dependency_", "fleet_", "mobile_", "hosted_", "slash_", "activity_"))), "")
    if key in {"PRI-05a","PRI-05b","PRI-05c","PRI-05d"}:
        first_test={"PRI-05a":"pmo_priority_aging_moves_both_directions_with_evidence","PRI-05b":"pmo_priority_opportunity_expiry_retriages_without_base_mutation","PRI-05c":"pmo_priority_hysteresis_cooldown_bounded_with_authorized_bypass","PRI-05d":"pmo_priority_clump_sweep_caps_one_move_and_records_evidence"}[key]
    if key=="MOB-00":
        first_test="pmo_mob_03/04_mobile_queue_keyboard_and_viewport_contract"
    if key=="PRI-06":
        first_test="priority_actions_preview_uses_shared_operation_result"
    if key=="DSP-01":
        first_test="pmo_dsp_01_existing_webhook_auth_fans_out_to_one_authorized_command"
    if key=="DST-04":
        first_test="pmo_distribution_priority_webhook_maps_to_shared_command"
    if key=="DOC-01":
        first_test="pmo_operator_docs_match_registered_help_inventory"
    if key=="DST-02":
        first_test="resolve_priority_preserves_ratified_15_25_without_snapping"
    done = find_value(raw, ("Done command", "Done/oracle", "Done/check", "Done", "Command"))
    if not done:
        cm = None
        for line in raw.splitlines():
            if key in line:
                after=line.split(key,1)[1]
                cm=re.search(r"`([^`]*(?:cargo test|bun test|test -f)[^`]*)`",after)
                if cm: break
        if not cm:
            cm = re.search(r"`(cargo test[^`]+|cd [^`]+ bun test[^`]+|python3 -I[^`]+)`", raw)
        done = cm.group(1) if cm else "Confirm the exact named test exists with the owning target's -- --list, then run that exact test; zero-match filters are not acceptance."
    if key.startswith("PRI-05"):
        done=f"cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker {first_test} (verify exact name appears with -- --list before running)."
    if key=="PRI-06":
        done="cd pm-stack/aggregator && bun test src/priority-actions.test.ts (first assert test file exists, then assert the named test is listed and executes)."
    if key=="DSP-01":
        done="cd pm-stack/projector && bun test test/webhook-delivery-id.test.ts (verify pmo_dsp_01_existing_webhook_auth_fans_out_to_one_authorized_command is listed and executes; retain existing signature tests)."
    if key=="DST-04":
        done="cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_distribution_priority_webhook_maps_to_shared_command (verify exact test name with -- --list first)."
    if key=="DST-01":
        first_test="pmo_distribution_command_identity_reason_and_version; pmo_distribution_command_policy_ref_round_trips"
        done="List both exact names with `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker -- --list`, then run each filter; DTO IDs and owner_policy_ref match architecture.md exactly."
    if key=="DST-03":
        first_test="pmo_distribution_atomic_intent_revision; pmo_distribution_intent_digest_binds_owner_policy_ref"
        done="List both exact names with `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker -- --list`, then run each filter; fake transaction oracle checks state/intent atomicity and digest inclusion/exclusion."
    if key=="DOC-01":
        done="python3 -I -c 'from pathlib import Path; s=Path(\"pkgs/fleet/README.md\").read_text(); required=(\"fleet dependency add\",\"fleet work complete\",\"private by default\",\"no exactly-once guarantee\",\"Queue Table Improvements\"); missing=[x for x in required if x not in s]; assert not missing, missing' (must fail before documentation is added and pass afterward)."
    if key=="DST-02":
        done="Add `resolve_priority_preserves_ratified_15_25_without_snapping` to `pm-stack/dispatch/test/validate.test.ts`, prove it fails against the current 15→10/25→20 behavior, then run `cd pm-stack/dispatch && bun test test/validate.test.ts`; the file's existing priority tests and new regression must execute."
    if key=="DST-01":
        contract_addendum="""### Shared wire identifiers and canonical request intent

Use wire IDs `fleet.command/v1` and `fleet.result/v1`; the separate default-expanded request DTO is `fleet.command-intent/v1`. Consume CLI-02's RFC 8785 implementation and golden vectors: do not add a second serializer. It includes `owner_policy_ref` as an intent precondition (present as `null` when not applicable); the request fingerprint uses `fleet.command-intent/v1` exact domain framing and excludes operation/delivery attempt IDs, receipt time, wait/output/dry-run. A matching digest never authorizes a write; current principal/capabilities are independently checked.
"""
        extra_acceptance=["Exact wire IDs and intent DTO fields match the shared architecture; `owner_policy_ref` round-trips and changes the intent digest when changed.","CLI-02's shared RFC 8785 and domain-framing fixtures are consumed; no second canonicalizer or creator-outbox hash migration."]
        extra_sources=["architecture.md#shared-command-and-result-interface","architecture.md#digest-and-serialization-contract","contracts/digest-vectors.json"]
    elif key=="DST-03":
        contract_addendum="""### Accepted intent and owner-policy binding

Persist the exact accepted work-command-v1 `request_digest` and `owner_policy_ref` with the operation/revision. The digest comes from CLI-02's `fleet.command-intent/v1` DTO and canonical JCS/domain framing; authorization scope and reauthorization remain independent checks. Do not import/rewrite the bounded creator-outbox hash. State+intent atomicity, same-key/same-intent replay and changed owner-policy/expected-version conflict must use the shared typed envelope.
"""
        extra_acceptance=["Accepted intent reuses CLI-02's domain-framed digest; `owner_policy_ref` and expected work version are intent preconditions, while delivery IDs/wait/timeout are not.","Replay with a matching digest never bypasses current authorization or the protected-expedite owner gate."]
        extra_sources=["architecture.md#digest-and-serialization-contract","contracts/digest-vectors.json","architecture.md#protected-owner-priority-policy"]
    elif key=="CLI-02":
        first_test="work_cli_rejects_unknown_args_before_side_effects; pmo_command_jcs_digest_conformance_cross_lang"
        done="Verify both exact tests appear in `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker -- --list`; then run filters `work_cli_rejects_unknown_args_before_side_effects` and `pmo_command_jcs_digest_conformance_cross_lang`. The conformance test compares RFC 8785 canonical bytes/hash against independent cross-language golden vectors and negative input cases."
    if key=="HYD-01":
        done="Verify `pmo_hyd_manifest_missing_required_source_is_partial`, `pmo_hyd_manifest_rejects_unknown_schema_version`, and `pmo_hyd_manifest_digest_matches_jcs_with_self_field_excluded` appear in the exact package -- --list; run each filter. Golden raw-artifact SHA-256 covers exact bytes."
    if key=="PRI-03":
        first_test="pmo_peer_rank_rebalances_local_range; pmo_work_id_decimal_strings_order_numeric_not_lexical"
        done="Verify `pmo_peer_rank_rebalances_local_range` and `pmo_work_id_decimal_strings_order_numeric_not_lexical` are listed; then run both exact fleet-queue filters. Comparator oracle asserts ID 9 sorts before 10 numerically for equal prior keys."
    contract_addendum=""
    extra_acceptance=[]
    extra_sources=[]
    if key=="CLI-02":
        contract_addendum="""### Cross-language command digest conformance (new work-command v1 only)

Wire identifiers are `fleet.command/v1` and `fleet.result/v1`; the exact request-intent DTO identifies `fleet.command-intent/v1`. Use RFC 8785 JCS and the architecture's three-purpose domain registry, exactly framed as `ASCII(domain) || 0x00 || UTF8(JCS(dto))`, then SHA-256 rendered as `sha256:` plus 64 lowercase hex characters. No length prefix, BOM, newline, whitespace or arbitrary client-selected domain. For request fingerprints use `fleet.command-intent/v1`; preserve `fleet.context-manifest/v1` and `fleet.selection-order/v1` as separate purposes. Do not migrate/change the bounded creation-outbox hash.

The validated, default-expanded intent DTO has exactly `schema_version`, `command_schema_version`, `work`, `kind`, `arguments`, `expected_work_version`, `lease_generation`, `observed_snapshot_ref`, `owner_policy_ref`, and `reason` (nullable fields included as `null`). `owner_policy_ref` is a meaningful intent precondition: changing it changes the digest. Exclude operation/key IDs, delivery attempts, receipt times, output format, wait/timeout and dry-run execution mode. Scope idempotency by authenticated principal+authority and recheck authorization; a matching digest is never authorization. Reject duplicate properties, invalid Unicode, NaN/Infinity and lossy numeric IDs before hashing. Golden Rust/TypeScript tests consume `contracts/digest-vectors.json`; ordinary language-default JSON sorting is not JCS.
"""
        extra_acceptance=["Cross-language JCS+SHA-256 vectors match `contracts/digest-vectors.json` byte-for-byte for all three registry domains, including exact one-NUL framing and lowercase `sha256:` output.","The intent DTO has exact `fleet.command-intent/v1` keys including `owner_policy_ref`; changing that precondition conflicts, while delivery ID/wait budget/output/dry-run do not change the intent digest.","Wire result/command schema IDs are exactly `fleet.command/v1` and `fleet.result/v1`; unknown IDs reject/degrade rather than silently coercing.","Duplicate properties, invalid Unicode, non-finite numbers and lossy numeric IDs fail before operation acceptance; property-order/whitespace vectors are invariant; arrays and strings are preserved by JCS."]
        extra_sources=["research/canonical-json-evidence.md","contracts/digest-vectors.json","architecture.md#digest-and-serialization-contract"]
    elif key=="HYD-01":
        contract_addendum="""### Manifest digest conformance

For the `fleet.context-manifest/v1` digest purpose, use the same exact shared framing `ASCII(domain) || 0x00 || UTF8(JCS(dto))`, SHA-256 with lowercase `sha256:` output; reference `contracts/digest-vectors.json`. `integrity.manifest_digest` excludes only itself to avoid recursive self-hashing; exact raw artifact bytes are hashed independently. Golden negatives prove toggling completeness/source/artifact metadata changes the manifest digest, property order/whitespace do not, and changing artifact bytes changes its raw SHA-256. This is not a change to the creation-outbox hash and is not evidence of a live artifact store.
"""
        extra_acceptance=["Manifest digest omits exactly `integrity.manifest_digest`; changing included source version/artifact ref/completeness is detected, while artifact hash covers exact stored bytes.","Use the registered manifest domain and exact NUL/UTF-8/JCS/SHA-256 lower-hex framing vector; share the canonicalizer with CLI-02 and do not copy the bounded outbox hash semantics."]
        extra_sources=["research/canonical-json-evidence.md","contracts/digest-vectors.json","architecture.md#digest-and-serialization-contract"]
    elif key=="PRI-03":
        contract_addendum="""### Canonical work-ID tie order and selection-order digest

Wire work IDs are canonical unsigned decimal strings, but after authority IDs compare bytewise, work IDs compare by arbitrary-precision numeric value (digit-count then digits), not lexical string order or floating point. Golden tie fixture must place work ID `9` before `10`; canonical IDs are not coerced through JavaScript numbers. Opaque provider/session IDs remain opaque strings. The optional full `order_hash` uses `fleet.selection-order/v1` and the shared `ASCII(domain) || 0x00 || UTF8(JCS(order_object))` + SHA-256 `sha256:` lower-hex framing. Hash the complete eligible population in canonical comparator order—not a UI top-page—and include `owner_policy_ref` and all selection-policy/revision inputs. This fingerprint neither grants a claim nor proves a complete snapshot.
"""
        extra_acceptance=["With otherwise equal comparator fields, authority-qualified work ID `9` sorts before `10` numerically despite string wire representation; provider/session IDs are never parsed as work IDs.","Order DTO uses exact `fleet.selection-order/v1` registry domain/framing, has the entire ordered eligible population and owner-policy reference, and fails to emit a conformant hash for partial/truncated candidate snapshots."]
        extra_sources=["architecture.md#identity-and-authority","architecture.md#digest-and-serialization-contract","research/canonical-json-evidence.md","contracts/digest-vectors.json"]
    protected_test={
        "PRI-01":"pmo_priority_profile_preserves_protected_expedite_membership",
        "PRI-05a":"pmo_priority_aging_skips_protected_expedite_set",
        "PRI-05b":"pmo_priority_opportunity_expiry_never_changes_protected_expedite",
        "PRI-05c":"pmo_priority_hysteresis_cannot_bypass_owner_policy",
        "PRI-05d":"pmo_priority_clump_never_moves_protected_expedite",
        "AUTH-01":"pmo_auth_protected_priority_requires_owner_policy",
        "DST-04":"pmo_distribution_protected_expedite_label_event_is_denied",
        "MOB-06":"pmo_mobile_protected_expedite_edit_requires_drew",
        "DSP-08":"pmo_dsp_08_protected_expedite_policy_cannot_be_bypassed",
    }
    if key in protected_test:
        protected_text="""### Existing Drew-only protected-expedite gate (#7030; no re-ratification)

Consume the existing #7030 protected expedite membership/value and the dated `pm-stack/design-reprioritization-2026-11-12.md` record. Protection is an owner-policy write restriction, not a new urgency score, hold, or execution bypass. Re-read the versioned `fleet.owner-priority-policy/v1` policy and protected membership under the same atomic work-version check; `owner_policy_ref` is a meaningful intent precondition. Only the currently authenticated configured Drew owner action may change protected membership, intrinsic value, peer placement or priority-label intent. Routine manual edits, automation/aging/clump/hysteresis, label normalization/removal, mobile actions and slash commands must return `OWNER_POLICY_PROTECTED` with no change; stale policy ref returns `OWNER_POLICY_VERSION_CONFLICT`; missing/unverifiable policy fails closed for affected mutations. Removing a mirror label does not unprotect an item. Reads and otherwise eligible execution are not globally blocked. Preserve #7030's exact `expedite=15` / `next-up=25` values; do not reratify or translate urgency 450/Do Now into queue priority.
"""
        contract_addendum=(contract_addendum+"\n" if contract_addendum else "")+protected_text
        extra_acceptance.append("Use #7030's existing Drew-only protected membership/value policy; exact authenticated owner action is the only path to amend it, and all routine/manual/automatic/label/mobile/slash changes fail closed without altering the protected item. No re-ratification.")
        extra_sources.extend(["https://github.com/developing-today/code/issues/7030#issuecomment-6715673367","pm-stack/design-reprioritization-2026-11-12.md:11-15,22,122,207","architecture.md#protected-owner-priority-policy"])
        new_test=protected_test[key]
        first_test=(first_test+"; "+new_test) if first_test and new_test not in first_test else (first_test or new_test)
        if key=="PRI-01":
            done += f" Also verify `{new_test}` is listed with `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker -- --list`, then run `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker {new_test}`."
        elif key.startswith("PRI-05"):
            done += f" Also verify `{new_test}` is listed with `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker -- --list`, then run `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker {new_test}`."
        elif key=="AUTH-01":
            done += f" Also verify `{new_test}` is listed, then run `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker {new_test}`."
        elif key=="DST-04":
            done += f" Also verify `{new_test}` is listed, then run `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker {new_test}`."
        elif key=="MOB-06":
            done += f" Add/run `{new_test}` in the same existing Bun target."
        elif key=="DSP-08":
            done += f" Also verify `{new_test}` is listed, then run `cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker {new_test}`."
    blocker_text = find_value(raw, ("Hard blockers", "Hard dependencies", "Dependencies", "Edges"))
    if src.get("table_cells") and len(src["table_cells"]) > 2:
        # PRI/DST/SEL matrices put dependencies in their third cell.
        blocker_text += " " + src["table_cells"][2]
    blockers = re.findall(r"#\d{3,5}|\b(?:PRI|DST|MOB|DSP|SEL|CLI|HYD|DEP|JRN|AUTH|VAL|REL)-\d+[A-Za-z]?\b", blocker_text)
    # Expand explicit same-prefix ranges before alias resolution.
    range_map={}
    for mrange in re.finditer(r"\b([A-Z]+)-(\d{2})\s*[–-]\s*(?:([A-Z]+)-)?(\d{2})\b",blocker_text):
        prefix,start,other,end=mrange.groups()
        if other in (None,prefix) and int(end)>=int(start):
            range_map[f"{prefix}-{start}"]=[f"{prefix}-{number:02}" for number in range(int(start),int(end)+1)]
    for mrange in re.finditer(r"\b([A-Z]+)-(\d{2})\s+through\s+([A-Z]+)-(\d{2})\b",blocker_text):
        prefix,start,other,end=mrange.groups()
        if prefix==other and int(end)>=int(start):
            range_map[f"{prefix}-{start}"]=[f"{prefix}-{number:02}" for number in range(int(start),int(end)+1)]
    expanded=[]
    for token in blockers:
        expanded.extend(range_map.get(token,[token]))
    blockers=[]
    for token in expanded:
        blockers.extend(CONTRACT_KEY_ALIASES.get(token,COMPOSITE_KEY_ALIASES.get(token,[token])))
    parent_gate_resolutions=[]
    parent_gate_gaps=[]
    granular_blockers=[]
    runtime_parent_gates=[]
    runtime_issue_gates=[]
    for blocker in blockers:
        if blocker in NONBLOCKING_ISSUE_GATES:
            runtime_issue_gates.append({"issue":blocker,"reason":NONBLOCKING_ISSUE_GATES[blocker]})
        else:
            granular_blockers.append(blocker)
    blockers=granular_blockers
    granular_blockers=[]
    for blocker in blockers:
        if blocker not in DOMAIN_PARENT_IDS:
            granular_blockers.append(blocker)
            continue
        target=GRANULAR_PARENT_GATES.get(key,{}).get(blocker,"UNMAPPED")
        if target=="UNMAPPED":
            if blocker=="#9196":
                target=None
                disposition="program safety/active-task gate; never a hard edge to the umbrella epic"
            else:
                parent_gate_gaps.append({"requirement_id":None,"slice_key":key,"source_parent":blocker,"reason":"domain-starter parent reference lacks an approved granular blocker mapping"})
                parent_gate_resolutions.append({"source_parent":blocker,"target":None,"status":"unresolved; no hard parent edge emitted"})
                continue
        else:
            disposition="resolved to the source-specific granular blocker"
        if target=="CONTEXT_ONLY":
            parent_gate_resolutions.append({"source_parent":blocker,"target":None,"status":"context reference only; no hard edge and no wait for census/owner"})
            continue
        if target is None:
            runtime_parent_gates.append({"source_parent":blocker,"reason":disposition})
            parent_gate_resolutions.append({"source_parent":blocker,"target":None,"status":disposition})
        else:
            granular_blockers.append(target)
            parent_gate_resolutions.append({"source_parent":blocker,"target":target,"status":disposition})
    blockers=list(dict.fromkeys(granular_blockers))
    blockers=[b for b in blockers if b!=key]
    resolved_source_keys=[]
    for alias,target in {**CONTRACT_KEY_ALIASES,**COMPOSITE_KEY_ALIASES}.items():
        if re.search(rf"(?<![A-Z0-9-]){re.escape(alias)}(?![A-Z0-9-])",raw):
            resolved_source_keys.append(f"{alias} → {', '.join(target)}")
    title = src["title"].replace("`", "")
    if key == "MOB-00":
        blockers=[]
    if key == "REL-05":
        # AUTH-03 is a recommended consumer of relay semantics, not a blocker to the pure relay model;
        # keeping it hard here would create AUTH-03 -> REL-05 -> AUTH-03.
        blockers=[b for b in blockers if b != "AUTH-03"]
    v1_existing=""
    extra_source_refs=[]
    external_requirement_refs=EXTERNAL_REQUIREMENT_REFS.get(key,[])
    for external_ref in external_requirement_refs:
        extra_source_refs.extend([external_ref["url"],external_ref["source"]])
    if key == "DSP-01":
        v1_existing="""## Existing V1 webhook substrate to reuse (not duplicate)

Pinned repository evidence already has `pm-stack/projector/src/webhook.ts:7-57`: bounded raw-body reading, HMAC-SHA256 validation and constant-time signature comparison. `pm-stack/projector/README.md:23-30,95-111,145-179` documents the authenticated `/github/webhook` receiver, supported issue/comment/PR/review/push events, delivery-GUID projection dedupe, hot-event ledger and reconcile reads. The projector is disposable projection glue and explicitly never writes back to GitHub/Kaneo/Pad/Grist (`README.md:3-8`); its delivery receipt is **not** canonical accepted-command/operation truth. Reuse the existing verifier/intake seam and authorized fanout. Do not implement a second HMAC gateway/webhook service. This slice's remaining gap is policy-authorized handoff into canonical Fleet operation intake, with command idempotency distinct from projection-delivery dedupe. If no existing authorized downstream route is found, keep the adapter contract/research-only; do not provision a new route/service.
"""
        extra_source_refs=["pm-stack/projector/src/webhook.ts:7-57","pm-stack/projector/README.md:3-8,23-30,95-111,145-179","pm-stack/projector/test/webhook-delivery-id.test.ts"]
        section_goal="Reuse existing bounded HMAC webhook intake and authenticated event fanout; add only the missing command authorization/idempotency handoff to canonical Fleet operation intake. Projection delivery dedupe is not accepted-command truth. No duplicate webhook service, database, or projector writeback path."
    elif key == "DST-04":
        v1_existing="""## Existing V1 event/projector substrate to consume

At pinned source, `pm-stack/projector/src/webhook.ts:7-57` already verifies bounded raw-body HMAC-SHA256 and `pm-stack/projector/README.md:23-30,145-179` documents supported events, `event_id=delivery:subevent`, `pmp_hot_events`, per-consumer high-water cursors and monotone durable sweep acknowledgements. This is a disposable projection ledger, not canonical Fleet command acceptance or a writeback authority (`README.md:3-8`). Consume it as a verified-input/projection-fanout seam only where the current owner supports it. Add no HMAC/auth service, no priority policy or remote mutation in `webhook.ts`, no Project writer, and no loop treating a Fleet projection echo as human intent. This slice owns label-event-to-shared-command mapping and tests for actor/capability authorization at the authoritative consumer; delivery-ID dedupe remains distinct from operation idempotency.
"""
        extra_source_refs=["pm-stack/projector/src/webhook.ts:7-57","pm-stack/projector/README.md:3-8,23-30,145-179","pm-stack/projector/src/store.ts:25-52"]
        section_goal="Map already-authenticated, delivery-deduplicated priority label/removal events into the shared Fleet command contract and reconcile echoes without granting the disposable projector authority. Reuse existing HMAC intake/hot ledger; add no policy or remote mutation to webhook.ts."
        blockers=list(dict.fromkeys([*blockers,"DST-01","DST-03","DSP-01","AUTH-01"]))
        owned=["pkgs/fleet/crates/fleet-queue/src/work/distribution.rs","pkgs/fleet/crates/fleet-queue/src/work/contracts.rs","pm-stack/projector/test/priority-webhook.test.ts"]
        v1_existing += "\n**Ownership resolution:** the source appendix's `webhook.ts` file suggestion is superseded by this V1 census overlay: do not put priority policy, command authorization, or remote writes in the HMAC helper. Existing HMAC remains read/auth intake only; Fleet's typed operation owner handles authorization/effects."
    elif key == "DSP-01":
        owned=["pm-stack/projector/src/server.ts","pm-stack/projector/test/webhook-operation-forwarding.test.ts","pm-stack/projector/test/webhook-delivery-id.test.ts"]
    elif key == "DOC-01":
        owned=["pkgs/fleet/README.md"]
    elif key in {"DST-03","DST-07"}:
        owned=["pkgs/fleet/crates/fleet-queue/src/work/distribution.rs","pkgs/fleet/crates/fleet-queue/src/work/contracts.rs"]
    elif key == "DSP-07":
        owned=["pkgs/fleet/crates/fleet-queue/src/work/operations.rs"]
    elif key == "DSP-08":
        owned=["pkgs/fleet/crates/fleet-queue/src/work/operations.rs"]
    elif key == "DSP-09":
        owned=["pkgs/fleet/crates/fleet-queue/src/work/operations.rs","pm-stack/dispatch/test/direct-launch-reservation.test.ts"]
    elif key == "MOB-10":
        owned=["pm-stack/aggregator/test/hosted-mobile-contract.test.ts"]
    elif key == "MOB-00":
        owned=["pm-stack/aggregator/package.json","pm-stack/aggregator/bun.lock","pm-stack/aggregator/playwright.config.ts","pm-stack/aggregator/test/browser/mobile-work-view.spec.ts",".github/workflows/pm-aggregator-tests.yml"]
    elif key == "MOB-01":
        owned=["pm-stack/aggregator/src/model.ts","pm-stack/aggregator/src/work-projection.ts","pm-stack/aggregator/test/work-projection.test.ts"]
    elif key == "MOB-02":
        owned=["pm-stack/aggregator/src/work-lifecycle-policy.ts","pm-stack/aggregator/test/work-lifecycle-policy.test.ts"]
    elif key == "MOB-03":
        owned=["pm-stack/aggregator/src/render.ts","pm-stack/aggregator/src/ui.css","pm-stack/aggregator/src/work-mobile-views.ts","pm-stack/aggregator/test/work-mobile-views.test.ts"]
    elif key == "MOB-04":
        owned=["pm-stack/aggregator/src/ui.css","pm-stack/aggregator/src/render.ts","pm-stack/aggregator/test/mobile-accessibility.test.ts","pm-stack/aggregator/docs/mobile-accessibility.md"]
    elif key == "MOB-05":
        owned=["pm-stack/aggregator/src/actions.ts","pm-stack/aggregator/src/server.ts","pm-stack/aggregator/src/work-action-contract.ts","pm-stack/aggregator/test/work-action-contract.test.ts"]
    elif key == "MOB-06":
        owned=["pm-stack/aggregator/src/actions.ts","pm-stack/aggregator/src/server.ts","pm-stack/aggregator/src/work-action-contract.ts","pm-stack/aggregator/test/mobile-work-actions.test.ts"]
    elif key == "MOB-07":
        owned=["pm-stack/aggregator/src/session-panel.ts","pm-stack/aggregator/test/session-action-receipts.test.ts","pm-stack/aggregator/test/session-capabilities.test.ts"]
    elif key == "MOB-08":
        owned=["pm-stack/aggregator/src/work-relations-activity.ts","pm-stack/aggregator/test/work-dependencies-activity.test.ts"]
    elif key == "MOB-09":
        owned=["pm-stack/aggregator/src/work-operation-reconciliation.ts","pm-stack/aggregator/test/work-operation-reconciliation.test.ts"]
    elif key == "REL-08":
        owned=["pkgs/fleet/crates/fleet-queue/src/work/completion.rs"]
    elif key == "REL-10H":
        owned=["pkgs/fleet/crates/fleet-queue/tests/hosted_conformance.rs"]
    elif key == "CEN-01":
        owned=["doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/census/requirements-crosswalk.jsonl","doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/census/requirements-crosswalk.md","doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/census/check_requirements_crosswalk.py","doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/census/test_requirements_crosswalk.py"]
    elif key == "CEN-02":
        owned=["doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/census/requirements-crosswalk.jsonl","doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/census/requirements-crosswalk.md","doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/census/test_census_evidence.py"]
    elif key == "CEN-04":
        owned=["doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/components/9198-requirement-crosswalk-current-state-census.md","doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/delivery-plan.md","doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/census/test_schema_compatibility_matrix.py"]
    elif key == "CEN-05":
        owned=["doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/validation/experiment.py","doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/validation/fixtures.json","doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/validation/test_experiment.py"]
    elif key == "CEN-06":
        owned=["doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/validation/report.py","doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/validation/test_report.py","doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/research/priority-exploration-experiment.md"]
    elif key == "CEN-07":
        owned=[".github/workflows/pmo-contract-conformance.yml","pkgs/fleet/crates/fleet-queue/src/work/operations.rs","pm-stack/projector/test/webhook-delivery-id.test.ts"]
    elif key == "CLI-04":
        owned=["pkgs/fleet/crates/fleet-queue/src/work/selection.rs","pkgs/fleet/crates/fleet-queue/src/work/lifecycle.rs"]
    elif key == "CLI-05":
        owned=["pkgs/fleet/crates/fleet-queue/src/work/lifecycle.rs","pkgs/fleet/crates/fleet-queue/src/work/journal.rs"]
    if key in OWNED_PATH_OVERRIDES:
        owned=OWNED_PATH_OVERRIDES[key]
    body = f"""# {key} — {title}

<!-- pmo-9196:{key} -->

**Issue title:** `{f"design(pm-orchestration): {title}"}`  
**Design parent:** #{src['parent']} · **Scope:** {src['source_label']} · **Version:** V3 proposal; not an authorization to implement. **Requirement IDs ({src.get('requirements_status','DRAFT-unallocated')}):** {', '.join(src.get('requirements',[])) or 'DRAFT / no curated crosswalk join; no ID inferred or minted'}. **Slice status:** {'design-expansion support, intentionally separate from intake-source requirement IDs' if src.get('design_expansion_support') else 'curated implementation/research slice'}. **Pinned baseline:** `a038f084dbeb525494d7bb928ad91f597db2ac51`.

This batch creates design/research/validation handoff issues only. After PR #9225 is merged and this slice's granular prerequisites are resolved, a separately scoped implementation issue may be filed; production adoption, migration, deployment, and live acceptance remain separately authorized stages. The local FDR decisions are accepted design defaults; global #9180 registration is pending and is not a blocker to this bounded design work.

## Goal and deliverable

{section_goal}

{src.get('requirement_trace','')}

Deliver this bounded slice only. It enables downstream work only after its stated contracts and prerequisites are accepted; it does not imply that other component specs, closed parent issues, source code, tests, deployments, or runtime behavior are complete.

## Owned files and boundaries

{chr(10).join('- `' + f + '`' for f in dict.fromkeys(owned))}

Explicit read-only consumers (not writable ownership): {', '.join(READONLY_CONSUMES.get(key,[])) or 'none declared'}.

Use the existing Fleet queue/PM Stack/adapter owners named in the design; do not add a queue, database, scheduler, service, resolver, executable, GitHub client, or general creation-outbox producer. Shared CLI glue remains owned by the single CLI-13 registration slice. Do not edit #6533 or duplicate its live acceptance; consume #9028/#9073–#9076/#9079/#9081/#9178/#9179/#3424/#8575/#8579 as applicable, not as replacement work.

## Prerequisites and sequencing

Hard blockers named by the source slice, after granular-key normalization: {', '.join(dict.fromkeys(blockers)) if blockers else 'No inferred hard edge is added at preparation time; parent integration must resolve granular contracts before the native graph is applied.'}
File-collision sequencing edges added by stable topological serialization: {', '.join(f"{edge['key']} ({edge['reason']})" for edge in src.get('hard_blocker_reasons',[])) or 'none'}.
Existing issue references retained as non-blocking runtime/evidence gates: {', '.join(f"{x['issue']} ({x['reason']})" for x in runtime_issue_gates) if runtime_issue_gates else 'none'}.
Domain-starter parent references are never emitted as hard native blockers. Source parents normalized to granular slices or runtime gates: {', '.join(f"{x['source_parent']}→{x['target']}" if x.get('target') else f"{x['source_parent']}→runtime gate" for x in parent_gate_resolutions) or 'none'}.
{('Unresolved parent-reference mapping: ' + '; '.join(x['source_parent'] for x in parent_gate_gaps) + '. APPLY blocked until the parent selects a real granular key.') if parent_gate_gaps else ''}
{('Source-local candidate aliases normalized for this packet: ' + '; '.join(resolved_source_keys) + '.') if resolved_source_keys else ''}
{('Ordering note: AUTH-03 is a recommended rework-if-early review for REL-05, not a hard edge; keeping it hard would cycle with AUTH-03→REL-05.') if key == 'REL-05' else ''}

Use only granular issue-key edges after all keys are resolved. Never make a whole domain starter a hard blocker when it creates a cycle. File collisions are hard sequencing edges. Parent integration adds native parent/blocker edges in a later pass after the command map, requirements IDs, and acyclic graph are verified.

## Starting failing test, oracle, and done check

- **First failing test:** `{first_test or 'Add a named failing test derived from the source slice before implementation; exact filter is not inferred from a zero-test success.'}`
- **Oracle:** {find_value(raw, ('Oracle', 'Done/oracle', 'Acceptance/oracle')) or 'The source slice’s stated reference model, golden fixture, or invariant; compare expected and observed state explicitly. No mock-only claim of deployed/live behavior.'}
- **Done command:** `{done}`
- Tests must first be listed by exact name in the owning target (`-- --list`); assert the test exists before execution. A zero-test filter passing is not acceptance.

## Acceptance and negative cases

{find_value(raw, ('Acceptance', 'Done/oracle', 'Done')) or src.get('table_summary', 'Meet the scoped deliverable and preserve source state.')}

Negative coverage: {find_value(raw, ('Negative', 'Negative tests')) or 'stale input, duplicate/replay, ambiguous resource, unauthorized actor, partial remote effect, unknown version, and no false verified/completed result where applicable.'}

## Compatibility, migration, and runtime gates

- Preserve the current queue/status/identity/Project-writer/Fleet `gh` compatibility surfaces. Do not silently change existing spellings or normalize ratified `expedite=15` / `next-up=25`; those remain distinct from `do_now`.
- QTI remains unlocated/unverified; #9241 is the existing provenance-recovery issue. #3424/#8575/#8579/#8602 are candidates/constraints, not presumed equivalents. Any overlapping persisted field/schema change waits for the QTI crosswalk and named current owner.
- Runtime gates: implementation starts only after the document PR is merged, the exact curated requirement IDs are integrated, and this issue's granular prerequisite edges are resolved. Schema/migration, production rollout, telemetry, authenticated remote writes, deployed behavior, and live acceptance each require their own explicit evidence and approval. Normal triage does not inherit urgency 450/Do Now.
- Projection rows being rebuildable/disposable does not authorize deleting the PM PostgREST database or unrelated tables. Preserve unique human-authored `pmp_links`/link writes and all unrelated tables; follow the existing backup/restore requirements before any future migration or cleanup.
- Compatibility: {find_value(raw, ('Compatibility', 'Migration/rollback', 'Migration')) or 'Additive and reversible only after existing owner/schema review; retain prior data and behavior; no silent fallback or dual authority.'}
- Rollback: {find_value(raw, ('Rollback', 'Migration/rollback')) or 'Disable only the new capability; preserve pending intents, artifacts, existing state, and remote effects. No broad reset, destructive cleanup, or deletion of history.'}

## Source trail

- Raw source refs: handoff `sources/01_handoff_pm_stack_fleet_orchestration.md`; charter `sources/02_charter_v1_1.md`; recovered original `sources/04_recovered_original_combined_design.md`; cited source section(s) in `sources/03_full_orchestration_proposal.md` as named by the component; exact occurrence citations are in the curated trace above when allocated.
- Design ref: `{src['source_path']}`, slice `{key}` (proposal only).
- Prior-art/evidence receipts: `research/track-a-state-of-art.md`, `research/track-b-state-of-art.md`, `research/track-b-sources.json`; additional exact evidence anchors for this slice: {', '.join(extra_source_refs + extra_sources) if (extra_source_refs or extra_sources) else 'see the pinned census and component evidence links above'}.
- Parent-approved support mapping (validation/expansion support only, not a direct slice-key match): {(', '.join(src.get('manual_support',{}).get('ids',[])) + ': ' + src['manual_support']['note'] + ' [approval ' + src['manual_support']['approved_by'] + ']') if src.get('manual_support') else 'none'}.
- Existing external requirement refs not assigned a new curated ID: {('; '.join(f"{x['ref']} {x['source']}: {x['statement']} ({x['qualification']})" for x in external_requirement_refs)) if external_requirement_refs else 'none'}.
- Command allocation: `{', '.join(src.get('command_keys', [])) or 'none; this is not a command-owning slice'}`. Full command inventory remains `census/commands.jsonl`; exact allocation source is `census/command-slice-map.jsonl` (215 rows, no wildcard). V1 baseline keys are not refiled as duplicate implementation issues.
- Shared architecture: `architecture.md`; FDR index: `decisions/README.md` (all FDR-PMO records remain proposed, not ratified).

---

### Original slice text (preserved)

{raw}
"""
    if contract_addendum:
        body=body.replace("## Compatibility, migration, and runtime gates",contract_addendum+"\n## Compatibility, migration, and runtime gates")
    if v1_existing:
        body=body.replace("## Owned files and boundaries",v1_existing+"\n## Owned files and boundaries")
    record = {
        "key": key,
        "kind": "research" if key in {"DEP-01", "PRI-08", "JRN-04"} else "documentation" if key == "DOC-01" else "migration" if key.startswith("REL-10") or key == "VAL-01" else "validation" if key in {"PRI-07", "SEL-05", "SEL-07", "MOB-00", "MOB-04", "MOB-09", "MOB-10", "DSP-07", "DSP-09", "DSP-10", "HYD-09"} else "contract" if key.endswith("-01") or key in {"CLI-02", "CLI-03", "CLI-12", "PRI-05a", "PRI-05b", "PRI-05c", "PRI-05d"} else "implementation",
        "parent": src["parent"],
        "title": f"design(pm-orchestration): {title}",
        "requirements": src.get("requirements", []),
        "requirement_refs": src.get("requirement_refs", []),
        "mapping_gaps": [*src.get("mapping_gaps", []),*parent_gate_gaps],
        "design_expansion_support": bool(src.get("design_expansion_support",False)),
        "manual_support": src.get("manual_support",{}),
        "external_requirement_refs": external_requirement_refs,
        "parent_gate_resolutions": parent_gate_resolutions,
        "runtime_issue_gates": runtime_issue_gates,
        "command_keys": src.get("command_keys", []),
        "requirements_status": src.get("requirements_status", "DRAFT-unallocated"),
        "source_refs": ["sources/01_handoff_pm_stack_fleet_orchestration.md", "sources/02_charter_v1_1.md", "sources/04_recovered_original_combined_design.md", "census/track-a.md", "census/track-b.md", "research/track-a-state-of-art.md", "research/track-b-state-of-art.md", "research/track-b-sources.json", *src.get("curated_source_refs", []), *extra_source_refs, *extra_sources],
        "design_refs": src.get("curated_design_refs", [src["source_path"], "architecture.md", "decisions/README.md"]),
        "goal": section_goal,
        "enables": "Only the explicitly sequenced dependent contract or adapter slices; not a domain starter's completion or a runtime capability by itself.",
        "owned_files": list(dict.fromkeys(owned)),
        "readonly_consumes": READONLY_CONSUMES.get(key,[]),
        "hard_blockers": list(dict.fromkeys(blockers)),
        "hard_blocker_reasons": [],
        "recommended_after": src.get("recommended_after", ["AUTH-03"] if key == "REL-05" else []),
        "runtime_gates": ["document PR merged", "curated requirement IDs allocated/integrated", "granular prerequisites reviewed", "production/live evidence separately authorized",*runtime_parent_gates],
        "first_failing_test": first_test or "Add and prove a named failing test from the source oracle before implementation.",
        "oracle": find_value(raw, ("Oracle", "Done/oracle", "Acceptance/oracle")) or "Source-specified model/golden/invariant with a decidable expected result.",
        "done_command": done,
        "negative_tests": find_value(raw, ("Negative", "Negative tests")) or "stale, ambiguous, unauthorized, duplicate/replay, partial/unknown effect, and false-completion cases as applicable",
        "acceptance": [find_value(raw, ("Acceptance", "Done/oracle", "Done")) or src.get("table_summary", "Complete the scoped source-slice deliverable and oracle."), *extra_acceptance],
        "compatibility": find_value(raw, ("Compatibility",)) or "Preserve existing owner, queue, status, resource identity, and Fleet gh boundaries; no silent coercion.",
        "rollback": find_value(raw, ("Rollback", "Migration/rollback")) or "Disable new path; preserve prior state, data, pending intents, and remote effects.",
        "version": "V3",
        "horizon": "after design-document merge and granular prerequisites; schema/runtime work has a separate gate",
        "cost_risk": "Medium; review source-specific size/risk before applying. Highest risks are false parity, shared-file collisions, schema ownership, remote partial effects, and overclaiming mocks/deployment.",
        "labels": LABELS,
        "bodypath": folder.as_posix(),
        "component_key": key,
        "status": "DRAFT; no issue filed",
    }
    return record, body


def aliases() -> list[dict]:
    base = {"kind": "contract", "requirements": [], "requirement_refs": [], "mapping_gaps": [], "design_expansion_support": False, "manual_support": {}, "external_requirement_refs": [], "parent_gate_resolutions": [], "runtime_issue_gates": [], "command_keys": [], "requirements_status": "DRAFT-unallocated", "source_refs": ["sources/01_handoff_pm_stack_fleet_orchestration.md", "sources/02_charter_v1_1.md"], "design_refs": ["components/9206-work-item-lifecycle-fleet-subcommands.md"], "goal": "Preserve the source key and all command/options acceptance material without creating a duplicate backend issue.", "enables": "Maps command acceptance into its canonical implementation issue.", "owned_files": ["census/command-slice-map.jsonl (parent integration owns; this alias does not own production files)"], "readonly_consumes": [], "hard_blockers": [], "hard_blocker_reasons": [], "recommended_after": [], "runtime_gates": ["exact command map from integration worker", "curated requirement ID integration"], "first_failing_test": "Canonical issue's named test; alias does not introduce a duplicate test.", "oracle": "Every inventory key has exactly one primary issue key; aliases are covered by its canonical issue's golden command acceptance.", "done_command": "python3 -I scripts/research_workflow.py check", "negative_tests": "Duplicate implementation issue for a facade alias; missing or ambiguous CLI mapping.", "acceptance": ["Command-slice map contains exactly one primary issue key for this source key and all options are represented in the canonical issue."], "compatibility": "Keep original spelling as a candidate until explicit compatibility disposition; do not register aliases automatically.", "rollback": "No code path owned by this alias; rollback is governed by canonical issue.", "version": "V3", "horizon": "integrated with canonical issue", "cost_risk": "Low if remains a mapping; high if turned into parallel backend work.", "labels": LABELS, "bodypath": None, "issue_required": False, "status": "DRAFT alias; no issue filed"}
    out=[]
    mapping={
        "CLI-06":("REL-06", "Resource/GitHub adapter façade acceptance is carried by REL-06; read/hydration acceptance stays in HYD-04/05/08 and command inventory must mark each exact spelling."),
        "CLI-07":("AUTH-02", "Metadata, managed content, notes/privacy acceptance is merged across AUTH-01/AUTH-02/JRN-02/AUTH-03; no second backend."),
        "CLI-08":("DEP-02", "Dependency verbs/options are merged across DEP-02/DEP-03/DEP-04; readiness consumes the typed graph, no second graph."),
        "CLI-09":("HYD-08", "Hydration/activity command façade maps to HYD-08; page/cursor/activity behavior remains HYD-05/06."),
        "CLI-10":("REL-08", "Completion and sync/outbox CLI façade is REL-08; effect verification, recovery and ordering remain REL-03–07."),
        "CLI-11":("DSP-05", "Preserve agent.slash/session command spellings; canonical grammar belongs to CLI-02 and live session capability/delivery belongs to DSP-05, with exact keys in command map."),
        "DSP-02":("CLI-02", "Strict slash grammar/parser is the shared Fleet CLI-02 parser; add canonical verbs and all slash acceptance there, not a second parser."),
        "DSP-03":("REL-03", "Webhook delivery and semantic action dedupe are merged into REL-03 operation identity; retain delivery-ID distinct from normalized command/idempotency digest."),
        "DEP-05":("CLI-13", "Shared registration glue is owned only by CLI-13; preserve dependency/note/journal help and aliases there."),
        "DEP-01":("CEN-04", "The six-piece #3424/current blocker crosswalk is merged into CEN-04; do not duplicate the current census worker's #3424/#8579 evidence sweep."),
        "PRI-05":("composite:PRI-05a,PRI-05b,PRI-05c,PRI-05d", "Composite source key split into four independently testable triage behaviors."),
        "PRI-06":("MOB-06", "Priority mobile actions are one integration slice owned by MOB-06; retain PRI-06 source acceptance there instead of duplicating the priority-actions frontend/backend."),
        "REL-10":("composite:REL-10M,REL-10H", "Composite source key split into migration/rollback validation and hosted fixture parity."),
        "QTI-RECON-01":("existing:9241", "Reuse existing research issue #9241; never file a second QTI provenance issue."),
        "CEN-03":("existing:9241", "The #9198 CEN-03 QTI research spec is implemented by/reuses existing #9241; do not file a duplicate."),
        "CEN-08":("existing:9198", "CEN-08 issue-index/dependency-coverage integration is owned by the existing parent #9198 run; do not file a duplicate orchestration issue."),
        "QTI-01":("existing:9241", "Curated QTI provenance requirement maps to the existing #9241 research issue."),
        "COMPAT-01":("CEN-04", "Cross-model compatibility requirement is owned by CEN-04's matrix; no duplicate issue."),
        "ROADMAP-01":("existing:9198", "Roadmap/dependency-coverage requirement is owned by the existing parent/CEN-08 integration; no parallel issue."),
        "MODEL-01":("CEN-05", "Offline reference-model requirement maps to CEN-05 and reuses the current experiment artifact."),
        "ADAPTER-01":("CEN-07", "Real-adapter conformance/CI requirement is owned by CEN-07; no adapter backend duplicate."),
        "MIG-01":("CEN-04", "Migration compatibility planning maps to CEN-04; migration implementation remains separately gated."),
        "OPT-IMPORTANCE-01":("PRI-08", "Importance-axis research maps to PRI-08; no inferred urgency/importance weights."),
        "OPT-REPLAY-01":("JRN-04", "Full-history replay/retention research maps to JRN-04; no event-sourcing rewrite."),
    }
    for key,(target,note) in mapping.items():
        parent=9199 if key in {"PRI-05","PRI-06","OPT-IMPORTANCE-01"} else 9209 if key in {"REL-10","OPT-REPLAY-01","DSP-03"} else 9203 if key in {"DSP-02"} else 9198 if key in {"QTI-RECON-01","CEN-03","CEN-08","QTI-01","COMPAT-01","ROADMAP-01","MODEL-01","ADAPTER-01","MIG-01"} else 9208 if key in {"DEP-01","DEP-05"} else 9206
        row=dict(base); row.update({"key":key,"parent":parent,"title":f"design(pm-orchestration): map {key} to {target}","alias_of":target,"goal":note,"design_refs":["components/9199-distributed-priority.md","components/9206-work-item-lifecycle-fleet-subcommands.md","components/9208-typed-dependencies-notes-journal-field-authority.md","issues/seed/research-gap.md"],"component_key":key})
        out.append(row)
    return out


def command_key_join(source_specs: list[dict], alias_specs: list[dict]) -> tuple[dict[str, list[str]], dict]:
    command_map_path=PACKAGE/"census/command-slice-map.jsonl"
    inventory_path=PACKAGE/"census/commands.jsonl"
    if not command_map_path.exists() or not inventory_path.exists():
        raise ValueError("exact command inventory/allocation manifest is not available")
    command_rows=parse_jsonl(command_map_path)
    inventory_rows=parse_jsonl(inventory_path)
    map_keys=[r.get("key") for r in command_rows]
    inventory_keys=[r.get("key") for r in inventory_rows]
    if len(map_keys)!=len(set(map_keys)) or set(map_keys)!=set(inventory_keys):
        raise ValueError("command allocation keys must exactly equal the inventory, once each")
    canonical={r["key"] for r in source_specs}
    alias_by_key={r["key"]:r["alias_of"] for r in alias_specs}
    special={"V1-BASELINE","COORD-DECISION","SCOPE-OUT-OF-SCOPE"}
    def targets(slice_key: str) -> list[str]:
        if slice_key in canonical:
            return [slice_key]
        target=alias_by_key.get(slice_key)
        if target:
            if target.startswith("composite:"):
                return target.split(":",1)[1].split(",")
            if target.startswith("existing:"):
                return []
            return [target]
        if slice_key in special:
            return []
        return [x for x in CONTRACT_KEY_ALIASES.get(slice_key,[]) if x in canonical]
    assigned={k:set() for k in canonical}
    unresolved=[]
    for row in command_rows:
        cmd=row["key"]
        primary=row.get("primary_slice")
        if not primary:
            unresolved.append(f"{cmd}: no primary_slice")
            continue
        primary_targets=targets(primary)
        if primary not in special and not primary_targets:
            unresolved.append(f"{cmd}: unresolved primary slice {primary}")
        for slice_key in [primary,*row.get("supporting_slices",[])]:
            resolved=targets(slice_key)
            if slice_key not in special and not resolved:
                unresolved.append(f"{cmd}: unresolved slice {slice_key}")
            for target in resolved:
                if target not in canonical:
                    unresolved.append(f"{cmd}: slice {slice_key} resolves to missing issue {target}")
                elif slice_key==primary or slice_key in row.get("supporting_slices",[]):
                    assigned[target].add(cmd)
    if unresolved:
        raise ValueError("command allocation has unresolved issue slices: " + "; ".join(unresolved[:12]))
    return {k:sorted(v) for k,v in assigned.items()}, {"rows":len(command_rows),"inventory_rows":len(inventory_rows),"distinct_command_keys":len(set(map_keys))}


def requirement_key_join(source_specs: list[dict], alias_specs: list[dict]) -> dict:
    path=PACKAGE/"census/requirements-crosswalk.jsonl"
    if not path.exists():
        raise ValueError("curated requirements crosswalk is not available")
    crosswalk=parse_jsonl(path)
    canonical={r["key"] for r in source_specs}
    alias_rows={r["key"]:r for r in alias_specs}
    special={"V1-BASELINE":"pinned V1 behavior; no duplicate implementation issue","COORD-DECISION":"coordinator/owner decision tracked at the parent; not a new implementation issue","SCOPE-OUT-OF-SCOPE":"explicitly out of scope; not an issue"}
    assigned={k:{} for k in canonical}
    alias_assigned={k:{} for k in alias_rows}
    existing_assigned:dict[str,dict]={}
    crosswalk_by_id:dict[str,dict]={}
    gaps=[]
    non_issue=[]

    def resolve(slice_key: str, seen: set[str] | None=None) -> tuple[list[str],list[str],str | None]:
        seen=set() if seen is None else seen
        if slice_key in seen:
            return [],[],f"alias cycle at {slice_key}"
        if slice_key in canonical:
            return [slice_key],[],None
        if slice_key in special:
            return [],[],special[slice_key]
        if slice_key in alias_rows:
            seen.add(slice_key)
            target=alias_rows[slice_key]["alias_of"]
            if target.startswith("existing:"):
                return [],[target.split(":",1)[1],slice_key],None
            if target.startswith("composite:"):
                all_targets=[]; existing=[]
                for sub in target.split(":",1)[1].split(","):
                    c,e,g=resolve(sub,seen.copy())
                    all_targets.extend(c); existing.extend(e)
                    if g: return [],[],g
                return sorted(set(all_targets)),existing,None
            canonical_targets,existing_targets,problem=resolve(target,seen)
            if existing_targets:
                return canonical_targets,[existing_targets[0],slice_key],problem
            return canonical_targets,existing_targets,problem
        if slice_key in CONTRACT_KEY_ALIASES:
            all_targets=[]
            for sub in CONTRACT_KEY_ALIASES[slice_key]:
                c,e,g=resolve(sub,seen.copy())
                all_targets.extend(c)
                if e:
                    return [],[],f"contract alias {slice_key} resolves to external {e}; declare a source-key alias instead"
                if g: return [],[],g
            return sorted(set(all_targets)),[],None
        if slice_key in COMPOSITE_KEY_ALIASES:
            all_targets=[]
            for sub in COMPOSITE_KEY_ALIASES[slice_key]:
                c,e,g=resolve(sub,seen.copy())
                all_targets.extend(c)
                if e: return [],[],f"composite {slice_key} unexpectedly resolves external"
                if g: return [],[],g
            return sorted(set(all_targets)),[],None
        return [],[],f"no exact issue-key or explicit alias mapping for {slice_key}"

    def compact_req(row: dict) -> dict:
        refs=row.get("source_refs",[])
        return {"requirement_id":row["requirement_id"],"status":row.get("status"),"modality":row.get("modality"),"source_occurrence_ids":row.get("source_occurrence_ids",[]),"source_refs":[{"source_file":x.get("source_file"),"occurrence_id":x.get("occurrence_id"),"line":x.get("line"),"source_section":x.get("source_section")} for x in refs],"design_refs":row.get("design_refs",[]),"implementation_slice_keys":row.get("implementation_slice_keys",[]),"outcome":row.get("outcome"),"residual":row.get("residual"),"missing_evidence":row.get("missing_evidence",[]),"duplicate_of":row.get("duplicate_of"),"mapping_basis":"exact frozen implementation_slice_keys inverse join"}

    ids=set()
    for row in crosswalk:
        req=row.get("requirement_id")
        if not req or req in ids:
            gaps.append({"requirement_id":req,"slice_key":None,"reason":"missing or duplicate curated requirement_id"})
            continue
        ids.add(req)
        crosswalk_by_id[req]=row
        keys=row.get("implementation_slice_keys",[])
        if not keys:
            gaps.append({"requirement_id":req,"slice_key":None,"reason":"curated crosswalk row has no implementation_slice_keys"})
            continue
        mapped_any=False
        for slice_key in keys:
            canonical_targets,existing_targets,disposition=resolve(slice_key)
            if disposition:
                non_issue.append({"requirement_id":req,"slice_key":slice_key,"disposition":disposition})
                mapped_any=True
                continue
            if existing_targets:
                issue_number=existing_targets[0]
                existing_assigned.setdefault(issue_number,{})[req]=compact_req(row)
                alias_key=existing_targets[1] if len(existing_targets)>1 else None
                if alias_key in alias_assigned:
                    alias_assigned[alias_key][req]=compact_req(row)
                mapped_any=True
                continue
            if not canonical_targets:
                gaps.append({"requirement_id":req,"slice_key":slice_key,"reason":"alias resolution returned no issue"})
                continue
            for target in canonical_targets:
                assigned[target][req]=compact_req(row)
                mapped_any=True
            if slice_key in alias_assigned:
                alias_assigned[slice_key][req]=compact_req(row)
        if not mapped_any:
            gaps.append({"requirement_id":req,"slice_key":None,"reason":"no exact issue, existing issue, composite or explicit no-issue disposition resolved"})
    # Parent-approved semantic support mappings are explicit, versioned here, and do not claim
    # that the source crosswalk itself named these newly refined validation/design-expansion keys.
    manual_support_allocations={}
    for key,support in PARENT_APPROVED_SUPPORT_MAP.items():
        if key not in canonical:
            gaps.append({"requirement_id":None,"slice_key":key,"reason":"parent-approved support target is absent from the canonical issue manifest"})
            continue
        manual_support_allocations[key]={"ids":support["ids"],"note":support["note"],"approved_by":"parent session ses_e5e2d10e06effeEylSDDVaN0PL, message seq 12; support mapping only, NOT Apply approval"}
        for req in support["ids"]:
            source_row=crosswalk_by_id.get(req)
            if source_row is None:
                gaps.append({"requirement_id":req,"slice_key":key,"reason":"parent-approved support ID does not exist in frozen curated crosswalk"})
                continue
            ref=compact_req(source_row)
            ref["mapping_basis"]="parent-approved semantic support mapping; not a direct implementation_slice_keys match"
            ref["support_rationale"]=support["note"]
            ref["approval_ref"]=manual_support_allocations[key]["approved_by"]
            assigned[key][req]=ref
    # Preserve missing coverage on newly prepared slices rather than inventing requirement IDs.
    per_issue_gaps={}
    for key,reqs in assigned.items():
        if not reqs:
            per_issue_gaps[key]=[{"requirement_id":None,"slice_key":key,"reason":"no curated crosswalk row maps to this newly drafted slice; parent must allocate or explicitly disposition it"}]
    return {"crosswalk_rows":len(crosswalk),"assigned":assigned,"alias_assigned":alias_assigned,"existing_assigned":existing_assigned,"gaps":gaps,"non_issue":non_issue,"per_issue_gaps":per_issue_gaps,"manual_support_allocations":manual_support_allocations,"design_ref_errors":sorted({ref for row in crosswalk for ref in row.get("design_refs",[]) if not validate_design_ref(ref)})}


def prepare() -> dict:
    prior_index=read_index()
    if prior_index.get("operations") or prior_index.get("last_create_at") or prior_index.get("last_create_mono_ns"):
        raise ValueError("--prepare refuses to overwrite the manifest after any external-write intent/pacing receipt exists; preserve those receipts and use an explicitly reviewed non-destructive refresh")
    source = source_candidates()
    alias_rows=aliases()
    command_keys,command_stats=command_key_join(source,alias_rows)
    req_join=requirement_key_join(source,alias_rows)
    records=[]
    body_files={}
    for src in source:
        src["command_keys"]=command_keys[src["key"]]
        req_refs=list(req_join["assigned"][src["key"]].values())
        src["requirement_refs"]=req_refs
        src["requirements"]=sorted(r["requirement_id"] for r in req_refs)
        src["requirements_status"]="ALLOCATED" if src["requirements"] else "DRAFT-unallocated"
        src["mapping_gaps"]=req_join["per_issue_gaps"].get(src["key"],[])
        src["design_expansion_support"]=src["key"] in {"PRI-08","JRN-04"}
        src["manual_support"]=req_join["manual_support_allocations"].get(src["key"],{})
        src["external_requirement_refs"]=EXTERNAL_REQUIREMENT_REFS.get(src["key"],[])
        if src["design_expansion_support"] and not src["requirements"]:
            src["mapping_gaps"]=[{"requirement_id":None,"slice_key":src["key"],"reason":"Design-expansion research seed requested for future policy/retention analysis; no intake-source requirement ID is assigned and none is inferred."}]
        src["curated_design_refs"]=sorted({ref for row in req_refs for ref in row.get("design_refs",[])}) or [src["source_path"]]
        src["curated_source_refs"]=sorted({f"{ref['source_file']}#{ref['occurrence_id']}:{ref.get('line')}" for row in req_refs for ref in row.get("source_refs",[])})
        trace=[]
        for row in req_refs:
            citations=", ".join(f"{ref['source_file']}#{ref['occurrence_id']}:{ref.get('line')}" for ref in row.get("source_refs",[]))
            trace.append(f"- **{row['requirement_id']} — {row.get('status')} ({row.get('modality')})**; basis: {row.get('mapping_basis','exact frozen slice-key inverse join')}; support rationale: {row.get('support_rationale','none')}; occurrences `{', '.join(row.get('source_occurrence_ids',[]))}`; citations: {citations}; design: {', '.join(row.get('design_refs',[]))}; outcome: {row.get('outcome','')}; residual: {row.get('residual') or 'none stated'}; missing evidence: {', '.join(row.get('missing_evidence',[])) or 'none stated'}")
        for gap in src["mapping_gaps"]:
            trace.append(f"- **Unresolved mapping gap:** {gap.get('reason')} (slice {gap.get('slice_key')})")
        src["requirement_trace"]="## Curated requirement trace\n\n"+"\n".join(trace) if trace else "## Curated requirement trace\n\nNo curated crosswalk row currently maps an ID to this future slice. Status remains DRAFT; do not invent an ID."
        rec, body = make_record(src)
        records.append(rec)
        body_files[rec["bodypath"]]=body
    records.extend(alias_rows)
    records.sort(key=lambda r:r["key"])
    missing_keys = {"PRI-05a","PRI-05b","PRI-05c","PRI-05d","REL-10M","REL-10H"} - {r["key"] for r in records if not r.get("alias_of")}
    if missing_keys:
        raise ValueError(f"source extraction missed synthetic split keys: {sorted(missing_keys)}")
    command_rows=parse_jsonl(PACKAGE/"census/command-slice-map.jsonl")
    for alias in alias_rows:
        alias["command_keys"]=sorted(r["key"] for r in command_rows if alias["key"] in [r.get("primary_slice"),*r.get("supporting_slices",[])])
        alias_refs=list(req_join["alias_assigned"].get(alias["key"],{}).values())
        alias["requirement_refs"]=alias_refs
        alias["requirements"]=sorted(r["requirement_id"] for r in alias_refs)
        alias["requirements_status"]="ALLOCATED" if alias["requirements"] else "DRAFT-unallocated"
        alias["mapping_gaps"]=[]
    alias_targets={k:[] for k in [r["key"] for r in records if not r.get("alias_of")]}
    for alias in alias_rows:
        target=alias["alias_of"]
        if target.startswith("composite:"):
            targets=target.split(":",1)[1].split(",")
        elif target.startswith("existing:"):
            targets=[]
        else:
            targets=[target]
        for dest in targets:
            if dest in alias_targets:
                alias_targets[dest].append(alias)
    for rec in [r for r in records if not r.get("alias_of")]:
        details=[]
        for alias in alias_targets[rec["key"]]:
            keys=", ".join(alias["command_keys"]) or "no direct command rows"
            details.append(f"- `{alias['key']}` → `{rec['key']}`: {alias['goal']} Exact command rows allocated here: {keys}.")
        if details:
            suffix="\n\n## Merged source-key/facade acceptance\n\n"+"\n".join(details)+"\n"
            body_files[rec["bodypath"]]+=suffix
    shared_file_order=add_shared_file_sequences(records,body_files)
    records=[r for r in records if not r.get("alias_of")]+alias_rows
    records.sort(key=lambda r:r["key"])
    write_atomic(SPECS, ("\n".join(json.dumps(r, sort_keys=True, ensure_ascii=False) for r in records) + "\n").encode())
    for rel, body in body_files.items():
        write_atomic(ISSUES / rel, body.encode())
    # Re-preparing docs must never discard durable external-write receipts or pacing state.
    index=read_index()
    index["schema"]="pmo-issue-index/v1"
    index.setdefault("operations",{})
    index.setdefault("last_create_at",None)
    index.setdefault("last_create_mono_ns",None)
    index.setdefault("existing",{})
    index["existing"].update({"QTI-RECON-01":{"issue":9241,"url":"https://github.com/developing-today/code/issues/9241","parent":9198,"note":"Reuse only; existing seed/receipt records the live queue coverage.","requirement_ids":sorted(r["requirement_id"] for r in req_join["existing_assigned"].get("9241",{}).values())},"CEN-08":{"issue":9198,"url":"https://github.com/developing-today/code/issues/9198","note":"Parent-owned crosswalk/roadmap integration; do not file a duplicate issue.","requirement_ids":sorted(r["requirement_id"] for r in req_join["existing_assigned"].get("9198",{}).values())}})
    index["requirement_join"]={"crosswalk_rows":req_join["crosswalk_rows"],"mapped_ids":len({i for values in req_join["assigned"].values() for i in values}),"unmapped_gaps":req_join["gaps"],"non_issue_dispositions":req_join["non_issue"],"manual_support_allocations":req_join["manual_support_allocations"],"design_ref_errors":req_join["design_ref_errors"],"unallocated_slices":req_join["per_issue_gaps"],"external_requirement_refs":EXTERNAL_REQUIREMENT_REFS}
    index["dependency_graph"]={**shared_file_order,"status":"draft; parent graph review required before native edge apply"}
    write_atomic(INDEX, (json.dumps(index,indent=2,sort_keys=True)+"\n").encode())
    readme = f"""# PM orchestration granular issue batch (DRAFT; do not apply yet)

This is an authoring/validation package only. No GitHub issue/label/Project write, queue enqueue, deploy, migration, schema change, scheduler, service, binary or general outbox producer is performed by `--prepare` or default `--check`. `--reconcile KEY` is external-read-only. No issues are filed until the parent explicitly approves `--apply KEY` for one key at a time.

## Batch inventory

- Canonical new issue specs: {len([r for r in records if not r.get('alias_of')])}.
- Source-key aliases/composites/existing issue references: {len([r for r in records if r.get('alias_of')])}.
- Existing QTI provenance issue reused: #9241 / `QTI-RECON-01`; not duplicated.
- `PRI-08` and `JRN-04` are separately tagged design-expansion support for future importance-axis and full-history/replay research. JRN-04 also cites existing #3424's full-history requirement without minting a new crosswalk ID or requiring a parallel store.
- Requirements status: all 389 unique curated IDs are allocated across canonical slices/existing issues using the frozen inverse join plus the explicitly parent-approved semantic support map; where a source row intentionally names multiple slices, each owner is retained. Each non-direct support mapping records its rationale and approval separately. There are {len(req_join['gaps'])} crosswalk join gaps and {len(req_join['per_issue_gaps'])} canonical slices without allocated IDs; any nonzero value blocks `--apply`.
- Command/options status: exact allocation source `census/command-slice-map.jsonl` joins all {command_stats['rows']} command keys. Each canonical issue's `command_keys` lists its primary/supporting inventory rows; aliases map their source keys onto canonical issues. V1 baseline keys are preserved as compatibility evidence, not duplicate implementation issues.

Issue bodies are self-contained at `bodies/<key>.md`, include `<!-- pmo-9196:<KEY> -->`, raw source/design refs, owned paths, failing test, oracle, done check, negative cases, compatibility, rollback and no-production gates. Original appendix text is preserved in each body. The compact manifest still has one record per source key so aliases are reviewable; alias rows never become duplicate backend issues.

## Preparation and checks

```bash
python3 -I doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/issues/publish_one.py --prepare
python3 -I doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/issues/publish_one.py --check
```

`--prepare` re-extracts component V3 appendix specs and bodies into this owned `issues/` subtree. Review all bodies and edit only `issues/` for this task. `--check` is local/read-only. Unknown args fail in argparse before any read/write/subprocess/network.

## Per-issue apply protocol (future, explicit parent approval only)

```bash
python3 -I doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/issues/publish_one.py --apply <CANONICAL_KEY>
```

One call handles one key; it never runs an unattended batch. It uses `fleet gh` token rotation directly (not a new outbox producer), exact-marker/title read-before-create, marker readback, native parent by child database ID, then `repo-queue validate` and `repo-queue enqueue --queue repo-health-code-pilot` with normal triage priority 30 and `repo-health:v1:triage:developing-today/code:issue:<n>`. It verifies the live queue task through the existing read-only coverage helper (the deployed `list --task` path is known not to honor that filter). It serializes issue creates with a persistent ≥3s pacing clock, records durable intent before create, and never blindly retries an uncertain create. Each call emits one concise receipt for the caller to immediately pass to `trail_add`.

If issue creation succeeds but queue enqueue fails, it records the failure and attempts an explicit issue comment explaining why the item is not queued; no receipt calls it live/covered. Existing parent batch controls native dependency edges after all keys resolve; this helper only adds edges present in the reviewed manifest.

Native dependency edges are deliberately **not** written during per-issue `--apply`: all real issue/database IDs must exist and the parent must review the whole graph first. After that explicit approval and after all requirement/command/index gates pass, apply one issue's reviewed blockers with `--apply-edges <KEY>`. It resolves local blocker keys from durable readback and external `#n` blockers via `fleet gh api`, adds the GitHub-native blocked-by edge to the blocked issue, and read-backs it. An uncertain edge POST is reconcile-only; it is never blindly repeated. Parent hierarchy and blocker edges are separate operations.

## Graph/quality policy

- Every hard edge must name a granular issue key or verified external issue; no whole-starter hard edge. Validator checks local-edge DAG before apply.
- CLI-06/07/08/09/10/11, DSP-02 parser, DSP-03 idempotency, DEP-01 #3424 crosswalk and DEP-05 preserve local source keys as `alias_of` records; their acceptance is merged into canonical issues rather than duplicated backends. CLI-12 remains its own bounded work.link semantics issue. The single shared registration owner is CLI-13.
- PRI-06's mobile priority-action adapter acceptance is merged into MOB-06; no second priority UI/backend issue. Its existing #7030 value and protected owner policy remain canonical.
- CEN-01/02/04–07 are parent #9198 validation/research slices; CEN-03 reuses #9241 and CEN-08/ROADMAP-01 stay with existing #9198 integration. PRI-08/JRN-04 use explicitly approved semantic support mappings, clearly distinct from direct inverse-key joins.
- Protected expedite membership/value is governed by the existing Drew-only #7030 ruling; digest issues use exact architecture domain framing and `owner_policy_ref`. No new ratification, DB, mutation backend, or outbox producer.
- Composite PRI-05 is split into four independent units; REL-10 is split into migration/rollback and hosted conformance units. Composite keys remain aliases, not extra issues.
- No duplicate #6533 live proof, #9028 bypass work, #9073–#9076/#9079/#9081 substrate, #9241 QTI research, #3424/#8575/#8579 research or creator outbox expansion.
- The source components and package FDR records remain proposed. “Existing code/test” statements remain pinned-source evidence, not deployment claims.
"""
    write_atomic(ISSUES / "README.md", readme.encode())
    return {"canonical": len([r for r in records if not r.get("alias_of")]), "aliases": len([r for r in records if r.get("alias_of")]), "body_files": len(body_files), "requirements_rows": req_join["crosswalk_rows"], "mapped_requirements": index["requirement_join"]["mapped_ids"], "crosswalk_gaps": len(req_join["gaps"]), "unallocated_slices": len(req_join["per_issue_gaps"]), "design_ref_errors": len(req_join["design_ref_errors"]), "seed_reused": 1}


def check() -> dict:
    rows, canon = specs_and_aliases()
    problems = readiness_report(rows,list(canon.values()))
    graph_problems,graph_stats=graph_and_collision_problems(list(canon.values()))
    problems.extend(graph_problems)
    idx=read_index()
    if idx.get("existing",{}).get("QTI-RECON-01",{}).get("issue") != 9241:
        problems.append("QTI-RECON-01 must reuse existing #9241")
    expected_bodies={r["bodypath"] for r in canon.values()}
    found_bodies={p.relative_to(ISSUES).as_posix() for p in (ISSUES/"bodies").glob("*.md")} if (ISSUES/"bodies").exists() else set()
    if found_bodies != expected_bodies:
        problems.append(f"body file set differs from canonical manifest: missing={sorted(expected_bodies-found_bodies)} orphan={sorted(found_bodies-expected_bodies)}")
    body_count=len(found_bodies)
    return {"result":"draft-ready" if not problems else "draft-blocked", "canonical":len(canon), "aliases":len(rows)-len(canon), "body_files":body_count, "requirements_rows":read_index().get("requirement_join",{}).get("crosswalk_rows"), "mapped_requirement_ids":read_index().get("requirement_join",{}).get("mapped_ids"), "graph":graph_stats, "apply_ready":False if problems or any(not r["requirements"] or r.get("mapping_gaps") for r in canon.values()) else True, "problems":problems}


def gh(args: list[str], timeout: int = 120) -> subprocess.CompletedProcess[str]:
    return subprocess.run(["fleet", "gh", *args], capture_output=True, text=True, timeout=timeout, check=False)


def api_get(path: str, jq: str | None = None) -> dict | list | str:
    args=["api", path]
    if jq:
        args += ["--jq", jq]
    proc=gh(args)
    if proc.returncode:
        raise RuntimeError(f"read failed: {proc.stderr.strip()[:300]}")
    if jq:
        try:
            return json.loads(proc.stdout)
        except json.JSONDecodeError:
            return proc.stdout.strip()
    return json.loads(proc.stdout)


def normalized_body(text: str | None) -> str:
    return (text or "").replace("\r\n","\n").rstrip("\n")


def issue_body_matches(issue: dict, spec: dict) -> bool:
    expected=(ISSUES/spec["bodypath"]).read_text()
    actual=normalized_body(issue.get("body"))
    return actual==normalized_body(expected)


def issue_labels_match(issue: dict, spec: dict) -> bool:
    labels=issue.get("labels") or []
    names={x.get("name") if isinstance(x,dict) else x for x in labels}
    return set(spec["labels"]) <= names


def issue_readback_matches(issue: dict, spec: dict, row: dict) -> tuple[bool,str,str]:
    expected_path=ISSUES/spec["bodypath"]
    expected_bytes=expected_path.read_bytes()
    local_digest=hashlib.sha256(expected_bytes).hexdigest()
    if row.get("body_sha256") and row["body_sha256"]!=local_digest:
        return False,"local body changed after durable create intent; do not overwrite/recreate",local_digest
    actual=normalized_body(issue.get("body"))
    expected=normalized_body(expected_bytes.decode("utf-8"))
    remote_digest=hashlib.sha256(actual.encode("utf-8")).hexdigest()
    if actual!=expected:
        return False,"full issue body readback differs from requested payload; possible human edit",remote_digest
    if not issue_labels_match(issue,spec):
        return False,"issue label readback lacks one or more requested pre-existing labels",remote_digest
    return True,"verified",remote_digest


def marker_search(marker: str) -> tuple[list[dict],bool,str]:
    # gh performs pagination up to the explicit limit. Reaching 1000 is not proof of absence.
    proc=gh(["issue","list","--repo",CODE_REPO,"--state","all","--search",marker,"--limit","1000","--json","number,title,state,url,body"])
    if proc.returncode:
        return [],False,(proc.stderr or proc.stdout).strip()[:300]
    try:
        rows=json.loads(proc.stdout or "[]")
    except json.JSONDecodeError as exc:
        return [],False,f"invalid marker-search JSON: {exc}"
    return rows,len(rows)<1000,""


def reconcile_readonly(key: str) -> dict:
    """External readback only: never POSTs/patches, enqueues, updates the index, or declares absence from a truncated search."""
    rows,canon=specs_and_aliases()
    if key not in canon:
        raise ValueError("reconcile key must identify a canonical spec")
    spec=canon[key]
    index=read_index()
    row=index.get("operations",{}).get(key,{"key":key,"marker":f"<!-- pmo-9196:{key} -->"})
    number=row.get("issue_number")
    if not number:
        matches,complete,error=marker_search(row["marker"])
        exact=[x for x in matches if row["marker"] in (x.get("body") or "")]
        if error:
            return {"key":key,"status":"uncertain","reason":"marker readback failed: "+error,"create_again":False}
        if not complete:
            return {"key":key,"status":"uncertain","reason":"marker search reached its page limit; absence is not proven","create_again":False}
        if len(exact)!=1:
            return {"key":key,"status":"uncertain" if not exact else "conflict","matches":[x.get("number") for x in exact],"reason":"no unique exact-marker match; this read-only command will not create or retry","create_again":False}
        number=exact[0]["number"]
    issue=api_get(f"repos/{CODE_REPO}/issues/{number}")
    body_ok,body_reason,body_hash=issue_readback_matches(issue,spec,row)
    marker_ok=row["marker"] in (issue.get("body") or "")
    title_ok=issue.get("title")==spec["title"]
    state=str(issue.get("state","")).lower()
    result={"key":key,"issue_number":number,"url":issue.get("html_url"),"database_id":issue.get("id"),"marker_verified":marker_ok,"title_verified":title_ok,"body_verified":body_ok,"body_sha256":body_hash,"labels_verified":issue_labels_match(issue,spec),"issue_state":state,"native_parent":None,"live_queue_task":None,"read_only":True}
    if not body_ok or not marker_ok or not title_ok or state!="open":
        result.update({"status":"conflict","reason":body_reason if not body_ok else "issue marker/title/state mismatch"})
        return result
    try:
        parent=api_get(f"repos/{CODE_REPO}/issues/{number}/parent")
        result["native_parent"]=parent.get("number")
        result["parent_verified"]=parent.get("number")==spec["parent"]
    except RuntimeError as exc:
        result["parent_verified"]=False
        result["parent_readback_error"]=str(exc)
    try:
        coverage=coverage_readback(number,spec["title"])
        covered=next((x for x in coverage.get("covered_issues",[]) if x.get("number")==number and x.get("status") in {"queued","claimed","parked","blocked"}),None)
        result["live_queue_task"]=covered
    except (OSError,RuntimeError,ValueError) as exc:
        result["coverage_readback_error"]=str(exc)
    result["status"]="readback_complete" if result.get("parent_verified") and result.get("live_queue_task") else "readback_partial"
    result["index_task_id"]=row.get("task_id")
    return result


def apply_one(key: str) -> dict:
    rows, canon = specs_and_aliases()
    if key not in canon:
        raise ValueError(f"{key} is not a canonical publishable issue key")
    preflight=check()
    if not preflight.get("apply_ready"):
        raise ValueError("APPLY BLOCKED by complete manifest/requirement/command/graph/collision check: " + "; ".join(preflight["problems"][:8]))
    index=read_index()
    spec=canon[key]
    row=index.setdefault("operations",{}).setdefault(key,{"key":key,"marker":f"<!-- pmo-9196:{key} -->","status":"not_started"})
    # Once an issue number is durable, always GET that exact object before any search/create path.
    if row.get("issue_number"):
        return reconcile_one(key,index,spec,create_if_absent=False)
    if row.get("status") in {"create_intent","uncertain","create_unknown"}:
        return reconcile_one(key, index, spec, create_if_absent=False)
    # Before any new issue creation, exact marker search. A title search alone is never reuse proof.
    found,search_complete,search_error=marker_search(row["marker"])
    if search_error or not search_complete:
        row.update({"status":"search_failed","detail":search_error or "marker search reached limit; absence not proven"}); save_index(index)
        return {"key":key,"status":"pending","reason":"complete marker search unavailable; no create attempted"}
    exact=[]
    for item in found:
        if item.get("body") and row["marker"] in item["body"]:
            exact.append(item)
    if len(exact)>1:
        row.update({"status":"conflict","matches":[x.get("number") for x in exact]}); save_index(index)
        return {"key":key,"status":"conflict","reason":"multiple exact marker matches"}
    if exact:
        item=exact[0]
        if item.get("title") != spec["title"] or str(item.get("state","")).lower() != "open":
            row.update({"status":"conflict","issue":item,"reason":"exact marker exists but title differs or issue is not open; never reopen/reparent automatically"}); save_index(index)
            return {"key":key,"status":"conflict","issue":item.get("url")}
        row.update({"status":"uncertain","issue_number":item["number"],"url":item["url"],"reused":True,"search_match_at":utcnow()}); save_index(index)
        return reconcile_one(key,index,spec,create_if_absent=False)
    label_cache=index.setdefault("verified_labels",{})
    for label in spec["labels"]:
        if label in label_cache:
            continue
        check_label=gh(["label","list","--repo",CODE_REPO,"--search",label,"--json","name"])
        if check_label.returncode:
            row.update({"status":"pending","label_check_error":check_label.stderr.strip()[:300]}); save_index(index)
            return {"key":key,"status":"pending","reason":f"cannot verify pre-existing label {label}; no label is created"}
        try:
            label_rows=json.loads(check_label.stdout or "[]")
        except json.JSONDecodeError:
            label_rows=[]
        if not any(x.get("name")==label for x in label_rows):
            row.update({"status":"pending","missing_existing_label":label}); save_index(index)
            return {"key":key,"status":"pending","reason":f"label {label} not found; label creation is not authorized"}
        label_cache[label]={"verified_at":utcnow(),"source":"pre-create label/list"}
        save_index(index)
    # Durable operation marker before entering create; any lost response becomes reconcile-only.
    now=datetime.now(timezone.utc)
    last=index.get("last_create_at")
    last_mono=index.get("last_create_mono_ns")
    now_mono=time.monotonic_ns()
    if last:
        try:
            elapsed=(now-datetime.fromisoformat(last.replace("Z","+00:00"))).total_seconds()
        except ValueError:
            return {"key":key,"status":"pending","reason":"invalid durable pacing timestamp; operator review required"}
        if elapsed < 0:
            return {"key":key,"status":"pending","reason":"wall clock moved backwards; operator review required"}
        if elapsed < 3:
            return {"key":key,"status":"pending","retry_after_seconds":round(3-elapsed,3),"reason":"direct-create pacing minimum"}
    if last_mono is not None:
        if now_mono < last_mono:
            return {"key":key,"status":"pending","reason":"monotonic clock reset since prior create; reconcile before continuing"}
        mono_elapsed=(now_mono-last_mono)/1_000_000_000
        if mono_elapsed < 3:
            return {"key":key,"status":"pending","retry_after_seconds":round(3-mono_elapsed,3),"reason":"monotonic direct-create pacing minimum"}
    row.update({"status":"create_intent","intent_at":utcnow(),"body_sha256":hashlib.sha256((ISSUES/spec["bodypath"]).read_bytes()).hexdigest(),"reused":False})
    index["last_create_at"]=utcnow()
    index["last_create_mono_ns"]=now_mono
    save_index(index)
    body_path=ISSUES/spec["bodypath"]
    proc=gh(["issue","create","--repo",CODE_REPO,"--title",spec["title"],"--body-file",str(body_path),*sum((["--label",label] for label in spec["labels"]),[])])
    if proc.returncode or not proc.stdout.strip():
        row.update({"status":"uncertain","create_error":(proc.stderr or proc.stdout).strip()[:400],"create_response":proc.stdout.strip()[:200]})
        save_index(index)
        return {"key":key,"status":"uncertain","reason":"create did not return a verified issue; next call is reconcile-only"}
    url=proc.stdout.strip().splitlines()[-1].strip()
    m=re.search(r"/issues/(\d+)$",url)
    if not m:
        row.update({"status":"uncertain","create_response":url[:300]}); save_index(index)
        return {"key":key,"status":"uncertain","reason":"unparseable create response; reconcile marker before any retry"}
    row.update({"status":"uncertain","issue_number":int(m.group(1)),"url":url,"create_response":proc.stdout.strip()[:300]})
    save_index(index)
    return reconcile_one(key,index,spec,create_if_absent=False)


def reconcile_one(key: str, index: dict, spec: dict, create_if_absent: bool) -> dict:
    row=index["operations"][key]
    number=row.get("issue_number")
    if number:
        obj=api_get(f"repos/{CODE_REPO}/issues/{number}")
        body_ok,body_reason,body_hash=issue_readback_matches(obj,spec,row)
        if row["marker"] not in (obj.get("body") or "") or obj.get("title") != spec["title"] or str(obj.get("state","")).lower() != "open" or not body_ok:
            row.update({"status":"conflict","reason":body_reason if not body_ok else "issue readback did not prove exact marker/title/open state","body_readback_sha256":body_hash}); save_index(index)
            return {"key":key,"status":"uncertain","issue":number}
        row.update({"status":"issue_verified","url":obj["html_url"],"database_id":obj["id"],"body_readback_sha256":body_hash,"body_readback_at":utcnow(),"readback_at":utcnow()}); save_index(index)
        return finish_issue(key,index,spec,row)
    found,complete,error=marker_search(row["marker"])
    if error or not complete:
        row.update({"status":"uncertain","reconcile_error":error or "marker search reached limit; absence not proven"}); save_index(index)
        return {"key":key,"status":"uncertain","reason":"marker reconciliation unavailable/incomplete; do not create again"}
    exact=[x for x in found if row["marker"] in (x.get("body") or "")]
    if len(exact)==1 and exact[0].get("title")==spec["title"]:
        if str(exact[0].get("state","")).lower() != "open":
            row.update({"status":"conflict","issue":exact[0],"reason":"exact marker is on a non-open issue; no reopen"}); save_index(index)
            return {"key":key,"status":"conflict","issue":exact[0].get("url")}
        row.update({"status":"uncertain","issue_number":exact[0]["number"],"url":exact[0]["url"],"reused":True,"search_match_at":utcnow()}); save_index(index)
        return reconcile_one(key,index,spec,False)
    row.update({"status":"uncertain","reconcile_at":utcnow(),"matches":[x.get("number") for x in exact]}); save_index(index)
    return {"key":key,"status":"uncertain","reason":"no unique exact-marker issue readback; do not create again"}


def finish_issue(key: str, index: dict, spec: dict, row: dict) -> dict:
    n=row.get("issue_number")
    if not n:
        return reconcile_one(key,index,spec,False)
    obj=api_get(f"repos/{CODE_REPO}/issues/{n}")
    body_ok,body_reason,body_hash=issue_readback_matches(obj,spec,row)
    if row["marker"] not in (obj.get("body") or "") or obj.get("title") != spec["title"] or str(obj.get("state","")).lower() != "open" or not body_ok:
        row.update({"status":"conflict","reason":body_reason if not body_ok else "exact marker/title/open-state readback failed","body_readback_sha256":body_hash}); save_index(index)
        return {"key":key,"issue_number":n,"status":"conflict","reason":row["reason"]}
    row.update({"database_id":obj["id"],"url":obj["html_url"],"body_readback_sha256":body_hash,"body_readback_at":utcnow(),"status":"issue_verified"}); save_index(index)
    parent=spec["parent"]
    parent_link=row.setdefault("parent_link",{"status":"not_started"})
    parent_status="uncertain"
    try:
        # Always query the native parent endpoint; absence of parent_issue_url in issue JSON is not proof.
        parent_get=gh(["api",f"repos/{CODE_REPO}/issues/{n}/parent"])
        if parent_get.returncode:
            err=(parent_get.stderr or parent_get.stdout).strip()
            if "404" in err or "Not Found" in err:
                parent_read=None
            else:
                raise RuntimeError(err[:300])
        else:
            parent_read=json.loads(parent_get.stdout)
        if parent_read is not None:
            if parent_read.get("number")==parent:
                parent_status="verified"
            else:
                parent_status="conflict"
                parent_link.update({"actual_parent":parent_read.get("number"),"expected_parent":parent})
        elif parent_link.get("status") in {"intent","uncertain","verified","parent_pending"}:
            parent_status="uncertain"  # Reconcile only; never blindly repeat parent POST.
        else:
            parent_link.update({"status":"intent","database_id":obj["id"],"expected_parent":parent,"intent_at":utcnow()}); save_index(index)
            parent_post=gh(["api",f"repos/{CODE_REPO}/issues/{parent}/sub_issues","--method","POST","-F",f"sub_issue_id={obj['id']}"])
            if parent_post.returncode:
                parent_status="uncertain"
                parent_link.update({"status":"uncertain","error":(parent_post.stderr or parent_post.stdout).strip()[:300],"attempted_at":utcnow()})
            else:
                # Read back once immediately; a missing result is uncertain, not license to repost.
                parent_verify=gh(["api",f"repos/{CODE_REPO}/issues/{n}/parent"])
                if parent_verify.returncode==0 and json.loads(parent_verify.stdout).get("number")==parent:
                    parent_status="verified"
                elif parent_verify.returncode and ("404" in parent_verify.stderr or "Not Found" in parent_verify.stderr):
                    parent_status="uncertain"
                else:
                    parent_status="uncertain"
                parent_link.update({"status":parent_status,"attempted_at":utcnow(),"readback_at":utcnow() if parent_status=="verified" else None})
    except (OSError,RuntimeError,ValueError,json.JSONDecodeError,subprocess.TimeoutExpired) as exc:
        parent_status="uncertain"
        parent_link.update({"status":"uncertain","error":str(exc)[:300],"checked_at":utcnow()})
    if parent_status=="verified":
        parent_link.update({"status":"verified","verified_at":utcnow(),"url":f"https://github.com/{CODE_REPO}/issues/{parent}"})
    elif parent_status=="conflict":
        parent_link.update({"status":"conflict","checked_at":utcnow()})
    else:
        parent_link.update({"status":"uncertain","checked_at":utcnow()})
    row["parent_link_status"]=parent_status
    save_index(index)
    # Native blocking edges are intentionally deferred to an explicit reviewed second pass
    # after all issue numbers/DB IDs exist. Per-issue apply never guesses graph state.
    if spec["hard_blockers"]:
        row["native_blockers_pending"]=spec["hard_blockers"]
    # Same-pass queue triage, ordinary priority 30.
    packet=make_packet(spec,n,obj["html_url"])
    fd,tmp=tempfile.mkstemp(prefix=f"pmo-{key}-",suffix=".json",dir=ISSUES)
    try:
        with os.fdopen(fd,"w") as f:
            json.dump(packet,f)
            f.flush(); os.fsync(f.fileno())
        v=subprocess.run(["repo-queue","validate",tmp],capture_output=True,text=True,timeout=120,check=False)
        if v.returncode:
            return queue_failed(key,index,row,n,f"validation failed: {(v.stderr or v.stdout).strip()[:300]}")
        row.update({"status":"queue_intent","queue_dedupe_key":packet["dedupe_key"],"queue_intent_at":utcnow()}); save_index(index)
        q=subprocess.run(["repo-queue","enqueue",tmp,"--queue",QUEUE],capture_output=True,text=True,timeout=120,check=False)
    except (OSError,subprocess.TimeoutExpired) as exc:
        return queue_failed(key,index,row,n,f"enqueue launch/timeout ambiguous: {exc}")
    finally:
        if os.path.exists(tmp): os.unlink(tmp)
    if q.returncode:
        return queue_failed(key,index,row,n,(q.stderr or q.stdout).strip()[:300])
    qr=parse_enqueue_response(q.stdout)
    if not qr or not qr.get("id"):
        return queue_failed(key,index,row,n,"enqueue response did not include a task ID; reconcile by stable dedupe key/coverage")
    row.update({"task_id":qr["id"],"queue_status":qr.get("status"),"queue":qr.get("queue"),"queue_response_at":utcnow()}); save_index(index)
    try:
        coverage=coverage_readback(n,spec["title"])
    except (OSError,RuntimeError,ValueError) as exc:
        return queue_failed(key,index,row,n,f"enqueue response received but live coverage readback failed: {exc}")
    covered=next((x for x in coverage.get("covered_issues",[]) if x.get("number")==n and x.get("task")==qr["id"] and x.get("status") in {"queued","claimed","parked","blocked"}),None)
    if not covered:
        row.update({"coverage_readback":coverage,"last_checked_at":utcnow()}); save_index(index)
        return queue_failed(key,index,row,n,"live coverage readback did not confirm a queued/claimed/parked/blocked task")
    blockers_pending=spec["hard_blockers"]
    parent_pending=[] if parent_status=="verified" else [f"parent #{parent} ({parent_status})"]
    relationships_pending=[*parent_pending,*blockers_pending]
    row.update({"status":"complete" if not relationships_pending else "complete_edges_pending","live_status":covered["status"],"relationship_pending":relationships_pending,"coverage_readback_at":utcnow(),"completed_at":utcnow()}); save_index(index)
    return {"key":key,"issue_number":n,"url":obj["html_url"],"parent":parent,"parent_status":parent_status,"task_id":qr["id"],"status":covered["status"],"queue":QUEUE,"dedupe_key":packet["dedupe_key"],"native_blockers_pending":blockers_pending,"relationships_pending":relationships_pending,"publication_verified":True,"complete":not relationships_pending}


def apply_edges_one(key: str) -> dict:
    rows,canon=specs_and_aliases()
    if key not in canon:
        raise ValueError("--apply-edges requires one canonical issue key")
    preflight=check()
    if not preflight.get("apply_ready"):
        raise ValueError("GRAPH APPLY BLOCKED by complete manifest/requirement/command/graph/collision check: "+"; ".join(preflight["problems"][:8]))
    index=read_index()
    row=index.get("operations",{}).get(key)
    if not row or row.get("status") not in {"complete_edges_pending","complete"}:
        raise ValueError("issue and live same-pass queue task must be verified before applying edges")
    if row.get("parent_link_status")!="verified":
        return {"key":key,"issue_number":row.get("issue_number"),"status":"pending","reason":"native parent link is not readback-verified; no blocker edges attempted"}
    spec=canon[key]
    issue_number=row.get("issue_number")
    if not issue_number or not row.get("database_id"):
        raise ValueError("issue database identity is not readback-verified")
    actual=api_get(f"repos/{CODE_REPO}/issues/{issue_number}")
    if spec["title"]!=actual.get("title") or f"<!-- pmo-9196:{key} -->" not in (actual.get("body") or "") or str(actual.get("state","")).lower()!="open":
        row.update({"status":"uncertain","graph_error":"issue marker/title/state readback mismatch"}); save_index(index)
        return {"key":key,"status":"uncertain","reason":"issue readback mismatch"}
    edges=row.setdefault("native_blocker_edges",{})
    aliases={r["key"]:r["alias_of"] for r in rows if r.get("alias_of")}
    existing=index.get("existing",{})
    def blocker_id(ref: str) -> tuple[int,str]:
        if ref.startswith("#") and ref[1:].isdigit():
            num=int(ref[1:]); obj=api_get(f"repos/{CODE_REPO}/issues/{num}"); return obj["id"],obj["html_url"]
        target=ref
        seen=set()
        while target in aliases and target not in seen:
            seen.add(target); target=aliases[target]
            if target.startswith("composite:"):
                raise ValueError(f"hard dependency {ref} is composite and needs parent graph adjudication")
            if target.startswith("existing:"):
                num=int(target.split(":",1)[1]); obj=api_get(f"repos/{CODE_REPO}/issues/{num}"); return obj["id"],obj["html_url"]
        if target in index.get("operations",{}):
            op=index["operations"][target]
            if op.get("database_id"): return op["database_id"],op.get("url","")
            if op.get("issue_number"):
                obj=api_get(f"repos/{CODE_REPO}/issues/{op['issue_number']}"); return obj["id"],obj["html_url"]
        if target in existing and existing[target].get("issue"):
            num=int(existing[target]["issue"]); obj=api_get(f"repos/{CODE_REPO}/issues/{num}"); return obj["id"],obj["html_url"]
        raise ValueError(f"blocker {ref} has no verified issue identity; no edge written")
    for blocker in spec["hard_blockers"]:
        bid,burl=blocker_id(blocker)
        current=api_get(f"repos/{CODE_REPO}/issues/{issue_number}/dependencies/blocked_by")
        edge_state=edges.get(blocker,{})
        if any(x.get("id")==bid for x in current):
            edges[blocker]={"status":"verified","database_id":bid,"url":burl,"readback_at":utcnow()}; save_index(index)
            continue
        if edge_state.get("status") in {"intent","uncertain"}:
            edge_state.update({"status":"uncertain","last_readback_at":utcnow()}); save_index(index)
            return {"key":key,"issue_number":issue_number,"status":"uncertain","blocker":blocker,"reason":"previous edge attempt is uncertain and readback is absent; never repeat POST blindly"}
        edge_state.update({"status":"intent","database_id":bid,"url":burl,"intent_at":utcnow()}); edges[blocker]=edge_state; save_index(index)
        proc=gh(["api",f"repos/{CODE_REPO}/issues/{issue_number}/dependencies/blocked_by","--method","POST","-F",f"issue_id={bid}"])
        if proc.returncode:
            edge_state.update({"status":"uncertain","error":(proc.stderr or proc.stdout).strip()[:300],"attempted_at":utcnow()}); save_index(index)
            return {"key":key,"issue_number":issue_number,"status":"uncertain","blocker":blocker,"reason":"native blocked_by write not confirmed; reconcile only"}
        current=api_get(f"repos/{CODE_REPO}/issues/{issue_number}/dependencies/blocked_by")
        if not any(x.get("id")==bid for x in current):
            edge_state.update({"status":"uncertain","attempted_at":utcnow(),"reason":"write returned but readback omitted blocker"}); save_index(index)
            return {"key":key,"issue_number":issue_number,"status":"uncertain","blocker":blocker,"reason":"native blocker readback absent"}
        edge_state.update({"status":"verified","attempted_at":utcnow(),"readback_at":utcnow()}); save_index(index)
    row.update({"status":"complete","graph_verified_at":utcnow()}); save_index(index)
    return {"key":key,"issue_number":issue_number,"status":"complete","verified_blockers":list(edges)}


def make_packet(spec: dict, number: int, url: str) -> dict:
    title=f"Triage {CODE_REPO}#{number}: {spec['title']}"[:180]
    key=f"repo-health:v1:triage:{CODE_REPO}:issue:{number}"
    packet={"title":title,"body":f"Triage research/design issue {url}; normal priority 30 only. No production/schema change authorization.","priority":30,"dedupe_key":key,"payload":{"schema":"repo-task/v1","kind":"triage","repo":CODE_REPO,"queue":QUEUE,"issue":{"number":number,"url":url,"title":spec["title"],"state_checked_at":utcnow()},"problem":f"New granular design/research issue under #{spec['parent']}; same-pass queue triage required. Implement only after design PR merge, curated requirement IDs and granular blockers; no runtime/prod scope implied.","evidence":[url,f"https://github.com/{CODE_REPO}/issues/{spec['parent']}","https://github.com/developing-today/code/issues/9196","https://github.com/developing-today/code/pull/9225",f"full refs live in the issue body (marker pmo-9196:{spec['key']}): {len(spec['source_refs'])} source refs, {len(spec['design_refs'])} design refs",f"doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/issues/{spec['bodypath']}"],"acceptance":["Triage issue and route to its bounded design/implementation slice; preserve all explicit schema, runtime, and owner gates.",*spec["acceptance"]],"tests":[f"Verify issue and its live triage task: {url}"],"scope":{"include":[f"Triage {CODE_REPO}#{number}",spec["goal"]],"exclude":["Production rollout without explicit approval","New queue/database/scheduler/binary or outbox producer","Urgency 450/Do Now as queue priority"]},"risk":{"level":"medium","flags":["same-pass-triage","design-gates"]},"priority_reason":"Normal (30): ordinary triage. Historical urgency 450/Do Now is not translated into task priority.","timebox_minutes":45,"max_attempts":6,"context_refs":[url,f"doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/issues/{spec['bodypath']}","doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/"],"requirement_ids":spec["requirements"],"command_keys":spec["command_keys"],"dedupe":{"checked_at":utcnow(),"basis":f"Stable issue key {key}; validate/enqueue idempotently and verify live coverage before receipt."}}}
    raw=json.dumps(packet).encode()
    if len(raw)>15000:
        raise ValueError(f"packet for {spec['key']} is {len(raw)} bytes after compaction; refusing oversize enqueue")
    return packet


def parse_enqueue_response(stdout: str) -> dict | None:
    decoder=json.JSONDecoder()
    for i in range(len(stdout)):
        if stdout[i]!="{": continue
        try:
            obj,end=decoder.raw_decode(stdout[i:])
            if isinstance(obj,dict) and "id" in obj and "status" in obj:
                return obj
        except json.JSONDecodeError:
            continue
    return None


def coverage_readback(number: int, title: str) -> dict:
    # Reuse the standing read-only coverage function/config instead of direct database access.
    path=PACKAGE.parents[1]/"scripts/fleet/queue-reconcile/check_coverage.py"
    import importlib.util
    modspec=importlib.util.spec_from_file_location("pmo_queue_coverage",path)
    if not modspec or not modspec.loader:
        raise RuntimeError("standing queue coverage helper unavailable")
    module=importlib.util.module_from_spec(modspec); modspec.loader.exec_module(module)
    token_path=Path(os.environ.get("REPO_QUEUE_TOKEN_FILE",str(module.DEFAULT_TOKEN_FILE)))
    token=token_path.read_text().strip()
    if not token:
        raise RuntimeError("standing coverage token file empty")
    return module.broker_coverage(os.environ.get("REPO_QUEUE_BROKER_URL",module.DEFAULT_BROKER_URL),token,QUEUE,CODE_REPO,[{"number":number,"title":title}])


def queue_failed(key: str,index: dict,row: dict,number: int,reason: str) -> dict:
    row.update({"status":"queue_failed","queue_failure":reason,"queue_failure_at":utcnow()}); save_index(index)
    comment=f"Triage task was not verified in {QUEUE} during issue publication. Reason: {reason}. The issue exists and remains open; retry by its stable marker/dedupe key after reconciling, not by creating a duplicate issue."
    c=gh(["api",f"repos/{CODE_REPO}/issues/{number}/comments","--method","POST","-f",f"body={comment}"])
    row["queue_failure_comment"]="posted" if c.returncode==0 else "comment_failed"
    if c.returncode: row["queue_failure_comment_error"]=c.stderr.strip()[:300]
    save_index(index)
    return {"key":key,"issue_number":number,"status":"queue_failed","reason":reason,"comment":row["queue_failure_comment"]}


def main(argv: list[str] | None = None) -> int:
    args=parse_args(argv)  # Full argv validation precedes any file/network/write/subprocess operation.
    try:
        with DirectoryLock(exclusive=not args.check):
            if args.prepare:
                result=prepare()
            elif args.apply:
                result=apply_one(args.apply)
            elif args.apply_edges:
                result=apply_edges_one(args.apply_edges)
            elif args.reconcile:
                rows,canon=specs_and_aliases()
                if args.reconcile not in canon:
                    raise ValueError("reconcile key must identify a canonical spec")
                result=reconcile_readonly(args.reconcile)
            else:
                result=check()
        print(json.dumps(result,sort_keys=True))
        if args.apply and not result.get("publication_verified"):
            return 3
        if args.apply_edges and result.get("status") not in {"complete"}:
            return 3
        if args.check and result.get("result")=="draft-blocked":
            return 0  # Draft incompleteness is expected before parent approval.
        return 0
    except (OSError,ValueError,RuntimeError,subprocess.TimeoutExpired) as exc:
        print(json.dumps({"result":"error","error":str(exc)},sort_keys=True))
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
