# PM orchestration granular issue batch (PUBLISHED 2026-11-13 — see reviews/publication-audit-2026-11-13.md)

This is an authoring/validation package only. No GitHub issue/label/Project write, queue enqueue, deploy, migration, schema change, scheduler, service, binary or general outbox producer is performed by `--prepare` or default `--check`. `--reconcile KEY` is external-read-only. No issues are filed until the parent explicitly approves `--apply KEY` for one key at a time.

Publication record (2026-11-13, task 15381): all 91 canonical issues published as #9429-#9519 with native parents, 245 verified blocked-by edges, and a live triage task each in repo-health-code-pilot; live audit 91/91 with zero failures. Full record: reviews/publication-audit-2026-11-13.md.

## Batch inventory

- Canonical new issue specs: 91.
- Source-key aliases/composites/existing issue references: 24.
- Existing QTI provenance issue reused: #9241 / `QTI-RECON-01`; not duplicated.
- `PRI-08` and `JRN-04` are separately tagged design-expansion support for future importance-axis and full-history/replay research. JRN-04 also cites existing #3424's full-history requirement without minting a new crosswalk ID or requiring a parallel store.
- Requirements status: all 389 unique curated IDs are allocated across canonical slices/existing issues using the frozen inverse join plus the explicitly parent-approved semantic support map; where a source row intentionally names multiple slices, each owner is retained. Each non-direct support mapping records its rationale and approval separately. There are 0 crosswalk join gaps and 0 canonical slices without allocated IDs; any nonzero value blocks `--apply`.
- Command/options status: exact allocation source `census/command-slice-map.jsonl` joins all 215 command keys. Each canonical issue's `command_keys` lists its primary/supporting inventory rows; aliases map their source keys onto canonical issues. V1 baseline keys are preserved as compatibility evidence, not duplicate implementation issues.

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
