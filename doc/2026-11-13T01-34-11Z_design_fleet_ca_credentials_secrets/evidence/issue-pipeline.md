# Issue publishing pipeline — verified evidence and operator contract

Status: source/live inspection only; this work has **not** created or enqueued any issues. The publisher accepts the shared proposals JSON array and defaults to plan-only. The final combined manifest still needs coordinator review and an explicit `--publish` invocation.

## Authoritative design constraints

- Coordinator handoff §4.2 requires each issue to name one deliverable, kind, requirement IDs and source ranges, measurable acceptance, repo evidence, technical boundaries, native dependencies, safety/test/rollback, numeric priority plus label, lifecycle, estimate, and open decisions. It also says hierarchy and blocking are distinct native links. Source: [`sources/COORDINATOR_summit_handoff.md`](../sources/COORDINATOR_summit_handoff.md#L102-L122).
- The same handoff §5 asks for inspectable stable proposal IDs, duplicate search before creation, bounded writes under GitHub limits, issue number/URL receipts, native child/blocker links, and read-back. It requires Project writes through the approved single writer, not direct client edits. Source: [§5](../sources/COORDINATOR_summit_handoff.md#L124-L139).
- Only the existing v0.3 hierarchy is in scope: program epic #9155 → existing domain starters #9156–#9167 (the current direct-child readback includes #9156–#9160 and #9162–#9167; **#9161 is absent**). New proposals are children of their existing domain starter, not replacement epics or a new hierarchy. Source: [`note.md`](../note.md#L3-L4), [`note.md`](../note.md#L52-L54), and live read-back `fleet gh api repos/developing-today/code/issues/9155/sub_issues?per_page=100` on 2026-11-12.
- #9155 safety gates remain controlling: design/synthetic proof only; no production CA generation/signing, root escrow/export, live vault writes/reconciliation, credential rotation, DNS/trust changes, or live deployment. Source: [`note.md`](../note.md#L26-L30).

## Verified issue write path

1. `fleet gh` is the installed and source-backed GitHub entry point (`fleet 0.1.0-dev.11851+gbe29b46.feat-9046-transfer-window` was present). `pkgs/fleet/crates/fleet/src/gh.rs:1-12,14-34` documents personal token then three GitHub App installation-token legs; every `gh` argument is passed through. `:46-53` permits write failover only when GitHub proves pre-execution refusal (rate-limit or permission/scope); any other write failure is returned without replay because it may have landed. Credentials are not logged (`:55-63`). Do not call `/usr/bin/gh`, interpolate tokens, or retry ambiguous write errors in this publisher.
2. Live `fleet gh issue create --help` advertises `--parent`, `--blocked-by`, repeated `--label`, `--body`, and `--title`; `fleet gh issue edit --help` advertises native `--parent`, `--add-sub-issue`, and `--add-blocked-by`. The publisher creates with the parent and label flags; it separately reads native relationships back. The marker `<!-- fleet-ca-v03-proposal:<stable-id> -->` is searched first in issue body, including closed issues. One match is resumed, multiple matches stop for human reconciliation, and an ambiguous create is not replayed.
3. The legacy fleet issue-tracker skill gives the native sub-issue API shape (`POST repos/<owner>/<repo>/issues/<parent>/sub_issues -F sub_issue_id=<child-db-id>`) and states `sub_issue_id` is the child **database ID** (not issue number): [issue-tracker-github.md:10](../../../.opencode/skills/setup-matt-pocock-skills/issue-tracker-github.md#L7-L14). It documents the native dependency API as `POST repos/<owner>/<repo>/issues/<child>/dependencies/blocked_by -F issue_id=<blocker-db-id>` and explicitly distinguishes issue number from database ID: [line 43](../../../.opencode/skills/setup-matt-pocock-skills/issue-tracker-github.md#L37-L44). `fleet gh api` GETs verify parent `/sub_issues?per_page=100` and child `/dependencies/blocked_by` after linking. Live checks on 2026-11-12: #9155 returned existing domain children; #9156's blocked-by endpoint returned `[]`.
4. Native hierarchy and dependencies are different edges; descriptions alone do not count. Source: [`pm-stack/README.md:94-112`](../../../pm-stack/README.md#L94-L112). The single Project writer is the orchestrator; this publisher never edits Projects fields/membership or uses `--project`. The lineage record says Project board writes were scope-blocked in its own pass, so a missing Project field cannot be inferred from a GitHub issue read: [`lineage-manifest-2026-11-12.md:31-48`](../../../pm-stack/lineage-manifest-2026-11-12.md#L31-L48).

## Labels, urgency, lifecycle, and queue contract

- Live `fleet gh label list --repo developing-today/code --limit 500 --json name,description` confirmed `priority/critical`, `priority/high`, `priority/normal`, `priority/low`; `kind/design`, `kind/feature`, `kind/test`, `kind/security`, `kind/docs`, `kind/chore`, `app/fleet`, `status/ready`, `status/blocked`, `status/parked`, and `status/needs-triage` also exist. #9156 live labels were `kind/design`, `app/fleet`, `epic/fleet-ca-secrets`. The script does **not create labels**: publishing performs a read-only label preflight and refuses before its first write if a mapped label is not already present. `kind/research` is absent, so research proposals do not get a fabricated kind label; their exact kind remains in the body. `status/ready` is described as “Triaged and ready for queued implementation work”; `status/blocked` and `/parked` have matching descriptions. There is no semantically equivalent generic Backlog label; Backlog is recorded in the body/receipt only.
- Numeric urgency is not guessed: `pm-stack/dispatch/README.md:41` says `critical|high|normal|low → 10/20/40/70`; queue validator `pkgs/fleet/crates/fleet-queue/src/validate.rs:20-34,52-53` allows only `10,20,30,40,50,70,90`. The publisher maps the four shared urgency values to those established priority values/labels (10/20/40/70; critical/high/normal/low respectively). It refuses other urgency terms.
- The script preserves the exact lifecycle in the issue body and dry-run plan; it adds existing `status/ready`, `status/blocked`, or `status/parked` only where the label description matches. Backlog has no equivalent label and is body/receipt only. No Project fields are written.
- Queue validator currently accepts only `repo-task/v1` kinds `triage`, `pr_shepherd`, `small_fix`, and `ci_investigation`, not a generic design/implementation task kind (`validate.rs:52-53`). Therefore each newly published issue gets a deterministic **initial triage** packet, keyed by the established family `repo-health:v1:triage:developing-today/code:issue:<number>` (see `validate.rs:73-78`), not a fictitious implementation task. Priority scale, required packet fields, timebox and retry defaults are validated in `validate.rs:26-34,80-210,811-829`.
- Enqueue uses the installed `repo-queue` alias and the sanctioned `repo-queue validate <packet>` then `repo-queue enqueue <packet> --queue <named-pilot>` path. `scripts/fleet/queue-reconcile/README.md:23-35` says this path is validating and dedupe-keyed and requires every issue to be filed and enqueued in the same pass. `check_coverage.py:219-265` demonstrates unique packet files, explicit queue selection and failure behavior. An actual read-only `repo-queue list --queue repo-health-code-pilot --limit 1` returned a task row with ID, priority, status, and dedupe key; `repo-queue list --queue <queue> --task <id>` is the implemented read-back route. Do not use a `*-shadow` queue.
- The creation outbox exists but is explicitly a bounded pilot: only `new-issue-creator-code-a` is wired and wide rollout is not authorized; its README says no direct-create fallback for that producer. It is not the general authoring path for this run. Source: [`doc/fleet/creation-outbox.md:1-74`](../../../doc/fleet/creation-outbox.md#L1-L74).

## Publisher behavior and safe invocation

`tools/publish_issues.py` accepts the common JSON array objects exactly as handed between domain workers:

```json
[{"id":"stable-id","title":"…","parent":9158,"kind":"design",
  "source_ranges":["spec §…"],"requirement_ids":["REQ-…"],"stage":"design",
  "lifecycle":"Ready","urgency":"normal","depends_on":[],"body":"…"}]
```

It validates all proposal objects, IDs, domain parents, references, and dependency DAG **before** any command. Dependencies are topologically ordered and linked with native API database IDs. It is serial by design: do not parallelize the 60–100 writes. Every completed issue has a durable JSON receipt entry; after issue + link verification the receipt is atomically written in `queue-pending` state, then updated with the returned queue row and a `repo-queue list` read-back. A crash or ambiguous write can be resumed by re-searching the marker and deterministic queue dedupe key. On any failed command it stops; `fleet gh` owns token failover, and the publisher will not repeat an uncertain write.

```bash
# Plan only; no network calls or writes (default, --dry-run is explicit alias)
python3 -I doc/2026-11-13T01-34-11Z_design_fleet_ca_credentials_secrets/tools/publish_issues.py \
  --manifest proposals.json --queue <repo-health-code-pilot>

# After coordinator review of the final combined manifest:
python3 -I doc/2026-11-13T01-34-11Z_design_fleet_ca_credentials_secrets/tools/publish_issues.py \
  --manifest proposals.json --queue <repo-health-code-pilot> --publish --receipt publish-receipt.json
```

`<repo-health-code-pilot>` above is a placeholder, not an assertion that the correct queue has been selected; choose the actual code pilot queue from the task's declared context before publishing. `--publish` requires `--receipt`; unknown arguments and abbreviations fail. No production secrets are part of the manifest or receipts.

Test command: `python3 -I doc/2026-11-13T01-34-11Z_design_fleet_ca_credentials_secrets/tools/test_publish_issues.py`.
