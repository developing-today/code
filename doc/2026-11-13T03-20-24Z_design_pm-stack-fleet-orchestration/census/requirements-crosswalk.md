# Curated requirement crosswalk and current-state coverage

**Evidence pin:** `a038f084dbeb525494d7bb928ad91f597db2ac51` for source code; issue/ref evidence is separately dated. **Semantic requirements:** 389 records (349 non-duplicate outcome nodes plus 40 explicit duplicate-linked records; 9 group nodes). **Source dispositions:** 3,571 occurrence rows. This is design/census evidence, not implementation completion or deployment proof.

## Crosswalk schema and coverage rule

Machine ledger: [`requirements-crosswalk.jsonl`](requirements-crosswalk.jsonl), one row for every ID in [`../inventory/requirements.jsonl`](../inventory/requirements.jsonl). Each row records exact source occurrence refs, modality/candidate disposition, status, reason, actual design heading anchors, existing evidence fields, local component slice keys, residual, missing evidence, duplicate/conflict refs and exact command inventory keys where applicable. Every source occurrence remains in [`../inventory/dispositions.jsonl`](../inventory/dispositions.jsonl), linked to requirement IDs or explicit metadata/continuation disposition. Trace chain: source occurrence → curated requirement → design anchor → granular slice → evidence/status.

Allowed statuses are exactly `verified existing`, `partially implemented`, `open implementation`, `design-only`, `research`, `superseded with rationale`, `blocked`, and `unverified`. `verified existing` is bounded pinned code/test or dated authority, never deployment by inference. `partially implemented` names exact subset and residual; adjacent commands do not qualify. `design-only` means a proposed contract exists without implementation evidence. `research` names bounded evidence acquisition, not an assumed result. `superseded with rationale` preserves the outcome while identifying the replaced mechanism and reason. `blocked` is a named hard prerequisite for that scoped work, not every related activity. `unverified` names the evidence boundary. Candidate/example/history modality is retained and never promoted to approved runtime policy.

`existing_evidence` is explicit null-filled where no V1 claim is made. Verified/partial rows include pin, path, symbol, lines, test/evidence limitation, issue URL if relevant and timestamp. No test/deployment is inferred from component proposals or issue titles.

## Counts

- `verified existing`: 3
- `partially implemented`: 44
- `open implementation`: 12
- `design-only`: 305
- `research`: 5
- `superseded with rationale`: 2
- `blocked`: 0
- `unverified`: 18
- Modalities: `candidate` 142, `example` 13, `history` 11, `requirement` 223.

### Source coverage

| Source                                       | First…last ID     | Count | Status counts                                                                                                                                        |
| -------------------------------------------- | ----------------- | ----: | ---------------------------------------------------------------------------------------------------------------------------------------------------- |
| `01_handoff_pm_stack_fleet_orchestration.md` | `H-R001`…`H-R063` |    63 | design-only: 51, partially implemented: 6, research: 2, unverified: 4                                                                                |
| `02_charter_v1_1.md`                         | `C-R001`…`C-R167` |   167 | design-only: 99, partially implemented: 35, open implementation: 12, unverified: 14, research: 3, verified existing: 3, superseded with rationale: 1 |
| `03_full_orchestration_proposal.md`          | `P-R001`…`P-R070` |    70 | design-only: 68, superseded with rationale: 1, partially implemented: 1                                                                              |
| `04_recovered_original_combined_design.md`   | `O-R001`…`O-R077` |    77 | design-only: 76, partially implemented: 1                                                                                                            |
| `05_supplemental_raw_note.md`                | `S-R001`…`S-R012` |    12 | design-only: 11, partially implemented: 1                                                                                                            |

Group nodes and their child IDs: `C-R009` → `C-R005`, `C-R039`, `C-R040`, `C-R043`, `C-R096`, `C-R112`, `C-R118`, `C-R126`, `C-R129`, `C-R134`, `C-R145`; `H-R016` → `C-R150`, `C-R151`, `C-R152`, `C-R153`, `P-R022`, `C-R093`, `C-R135`, `P-R046`, `H-R021`; `H-R021` → `H-R038`, `H-R039`, `H-R040`, `H-R041`, `H-R042`, `H-R043`, `H-R044`; `H-R023` → `H-R045`, `H-R046`, `H-R047`, `H-R048`, `H-R049`, `H-R050`; `O-R005` → `O-R054`, `O-R055`, `O-R056`, `O-R057`; `O-R009` → `O-R058`, `O-R059`, `O-R060`, `O-R061`; `O-R014` → `O-R062`, `O-R063`, `O-R064`, `O-R065`; `P-R023` → `P-R041`, `P-R042`, `P-R043`, `P-R044`, `P-R045`, `P-R046`, `P-R047`, `P-R048`; `P-R027` → `P-R049`, `P-R050`, `P-R051`, `P-R052`, `P-R053`, `P-R054`, `P-R055`.

All requirement IDs are individually classified even where `duplicate_of` points to a canonical semantic row; H/P/O/S additions and source modality are retained. The machine ledger is the row-level oracle.

## Exact command and option coverage

The current census command inventory `census/commands.jsonl` contains **215 unique exact keys**. Status denotes exact spelling at pinned source, not complete parser/deployment coverage. `census/command-slice-map.jsonl` maps each exact key one-to-one to a primary slice plus supporting slices; this table consumes that owner map.

| Exact key                           | Spelling                                                            | V1 exact status | Primary slice        | Supporting slices                                | Starter |
| ----------------------------------- | ------------------------------------------------------------------- | --------------- | -------------------- | ------------------------------------------------ | ------- |
| `fleet.help`                        | `fleet help`                                                        | implemented     | `CLI-02`             | `CLI-13`                                         | #9206   |
| `fleet.capabilities`                | `fleet capabilities`                                                | absent          | `CLI-02`             | `CLI-13`                                         | #9206   |
| `agent.bootstrap`                   | `fleet agent bootstrap`                                             | absent          | `CLI-02`             | `DSP-02`, `CLI-13`                               | #9206   |
| `agent.dispatch`                    | `fleet agent dispatch`                                              | absent          | `DSP-06`             | `DSP-05`, `DSP-07`, `CLI-13`                     | #9203   |
| `agent.status`                      | `fleet agent status`                                                | absent          | `DSP-05`             | `CLI-13`                                         | #9203   |
| `agent.session.send`                | `fleet agent session send`                                          | absent          | `DSP-05`             | `DSP-09`, `CLI-13`                               | #9203   |
| `agent.session.resume`              | `fleet agent session resume`                                        | absent          | `DSP-05`             | `DSP-09`, `CLI-13`                               | #9203   |
| `agent.session.stop`                | `fleet agent session stop`                                          | absent          | `DSP-05`             | `DSP-09`, `CLI-13`                               | #9203   |
| `queue.validate`                    | `fleet queue validate; repo-queue validate`                         | implemented     | `V1-BASELINE`        | `CLI-13`                                         | #9206   |
| `queue.list`                        | `fleet queue list; repo-queue list`                                 | implemented     | `V1-BASELINE`        | `CLI-03`, `CLI-13`                               | #9206   |
| `queue.search`                      | `fleet queue search`                                                | absent          | `CLI-02`             | `SEL-01`, `CLI-13`                               | #9206   |
| `queue.get`                         | `fleet queue get`                                                   | absent          | `CLI-03`             | `CLI-13`                                         | #9206   |
| `queue.inspect`                     | `fleet queue inspect`                                               | absent          | `CLI-03`             | `CLI-13`                                         | #9206   |
| `queue.rank`                        | `fleet queue rank`                                                  | absent          | `SEL-01`             | `PRI-02`, `CLI-13`                               | #9204   |
| `queue.claim`                       | `fleet queue claim`                                                 | implemented     | `REL-01`             | `SEL-02`, `CLI-13`                               | #9209   |
| `queue.release`                     | `fleet queue release`                                               | implemented     | `REL-02`             | `CLI-13`                                         | #9209   |
| `queue.park`                        | `fleet queue park`                                                  | implemented     | `CLI-05`             | `REL-02`, `CLI-13`                               | #9206   |
| `queue.resume`                      | `fleet queue resume`                                                | absent          | `CLI-05`             | `REL-02`, `CLI-13`                               | #9206   |
| `queue.update`                      | `fleet queue update`                                                | absent          | `CLI-05`             | `AUTH-01`, `CLI-13`                              | #9208   |
| `queue.close`                       | `fleet queue close`                                                 | absent          | `REL-08`             | `CLI-13`                                         | #9209   |
| `queue.explore`                     | `fleet queue explore --count 3 --strategy weighted --explain`       | absent          | `SEL-03`             | `SEL-04`, `SEL-05`                               | #9204   |
| `priority.set`                      | `fleet priority set <work> high\|<number>`                          | absent          | `PRI-01`             | `PRI-02`, `DST-03`, `CLI-13`                     | #9199   |
| `priority.move`                     | `fleet priority move <work> --before\|--after <work>`               | absent          | `PRI-03`             | `DST-06`, `CLI-13`                               | #9199   |
| `priority.adjust`                   | `fleet priority adjust <work> --delta N`                            | absent          | `PRI-01`             | `PRI-02`, `CLI-13`                               | #9199   |
| `priority.explain`                  | `fleet priority explain <work>`                                     | absent          | `PRI-02`             | `SEL-01`, `CLI-13`                               | #9199   |
| `queue.fallback.scan`               | `repo-queue fallback scan`                                          | implemented     | `V1-BASELINE`        | `SEL-01`                                         | #9204   |
| `queue.fallback.status`             | `repo-queue fallback status`                                        | implemented     | `V1-BASELINE`        | `REL-01`                                         | #9209   |
| `queue.fallback.claim`              | `repo-queue fallback claim`                                         | implemented     | `V1-BASELINE`        | `REL-01`, `SEL-02`                               | #9209   |
| `queue.fallback.heartbeat`          | `repo-queue fallback heartbeat`                                     | implemented     | `V1-BASELINE`        | `REL-02`                                         | #9209   |
| `queue.fallback.release`            | `repo-queue fallback release`                                       | implemented     | `V1-BASELINE`        | `REL-02`                                         | #9209   |
| `queue.complete`                    | `fleet queue complete; repo-queue complete`                         | implemented     | `REL-08`             | `V1-BASELINE`, `CLI-13`                          | #9209   |
| `queue.wake`                        | `fleet queue wake; repo-queue wake`                                 | implemented     | `V1-BASELINE`        | `CLI-05`                                         | #9206   |
| `queue.block`                       | `fleet queue block; repo-queue block`                               | implemented     | `V1-BASELINE`        | `DEP-03`, `CLI-05`                               | #9208   |
| `queue.unblock`                     | `fleet queue unblock; repo-queue unblock`                           | implemented     | `V1-BASELINE`        | `DEP-03`, `CLI-05`                               | #9208   |
| `queue.checkpoint`                  | `fleet queue checkpoint; repo-queue checkpoint`                     | implemented     | `V1-BASELINE`        | `HYD-01`, `JRN-03`                               | #9207   |
| `queue.cancel`                      | `fleet queue cancel; repo-queue cancel`                             | implemented     | `V1-BASELINE`        | `CLI-01`                                         | #9206   |
| `queue.fail`                        | `fleet queue fail; repo-queue fail`                                 | implemented     | `V1-BASELINE`        | `CLI-01`                                         | #9206   |
| `queue.timeline`                    | `fleet queue timeline; repo-queue timeline`                         | implemented     | `V1-BASELINE`        | `HYD-07`                                         | #9207   |
| `queue.stack.record`                | `repo-queue stack record`                                           | implemented     | `V1-BASELINE`        | `REL-07`, `REL-08`                               | #9209   |
| `queue.stack.list`                  | `repo-queue stack list`                                             | implemented     | `V1-BASELINE`        | `REL-07`                                         | #9209   |
| `queue.stack.transition`            | `repo-queue stack transition`                                       | implemented     | `V1-BASELINE`        | `REL-07`                                         | #9209   |
| `queue.stack.restack`               | `repo-queue stack restack`                                          | implemented     | `V1-BASELINE`        | `REL-07`                                         | #9209   |
| `queue.stack.replay`                | `repo-queue stack replay`                                           | implemented     | `V1-BASELINE`        | `REL-06`, `REL-07`, `REL-08`                     | #9209   |
| `session.status`                    | `repo-queue session status`                                         | implemented     | `V1-BASELINE`        | `DSP-05`                                         | #9203   |
| `session.result`                    | `repo-queue session result`                                         | implemented     | `V1-BASELINE`        | `DSP-05`                                         | #9203   |
| `session.list`                      | `repo-queue session list`                                           | implemented     | `V1-BASELINE`        | `DSP-05`                                         | #9203   |
| `work.next`                         | `fleet work next`                                                   | absent          | `CLI-04`             | `SEL-01`, `SEL-02`, `HYD-04`, `CLI-13`           | #9206   |
| `work.open`                         | `fleet work open`                                                   | absent          | `CLI-04`             | `CLI-03`, `SEL-02`, `HYD-04`, `CLI-13`           | #9206   |
| `work.inspect`                      | `fleet work inspect`                                                | absent          | `CLI-03`             | `HYD-01`, `CLI-13`                               | #9206   |
| `work.hydrate`                      | `fleet work hydrate`                                                | absent          | `HYD-04`             | `HYD-01`, `CLI-13`                               | #9207   |
| `work.refresh`                      | `fleet work refresh`                                                | absent          | `HYD-08`             | `HYD-06`, `CLI-13`                               | #9207   |
| `work.changes`                      | `fleet work changes`                                                | absent          | `HYD-07`             | `HYD-05`, `HYD-06`, `CLI-13`                     | #9207   |
| `work.link`                         | `fleet work link`                                                   | absent          | `CLI-12`             | `AUTH-01`, `HYD-02`, `JRN-01`, `CLI-13`          | #9206   |
| `work.checkpoint`                   | `fleet work checkpoint`                                             | absent          | `JRN-03`             | `HYD-01`, `CLI-13`                               | #9207   |
| `work.note.add`                     | `fleet work note add`                                               | absent          | `JRN-02`             | `AUTH-02`, `CLI-13`                              | #9208   |
| `work.note.list`                    | `fleet work note list`                                              | absent          | `JRN-02`             | `AUTH-01`, `CLI-13`                              | #9208   |
| `work.note.edit`                    | `fleet work note edit`                                              | absent          | `JRN-02`             | `AUTH-02`, `CLI-13`                              | #9208   |
| `work.block`                        | `fleet work block`                                                  | absent          | `DEP-03`             | `CLI-05`, `CLI-13`                               | #9208   |
| `work.unblock`                      | `fleet work unblock`                                                | absent          | `DEP-03`             | `CLI-05`, `CLI-13`                               | #9208   |
| `work.park`                         | `fleet work park`                                                   | absent          | `CLI-05`             | `JRN-03`, `REL-02`, `CLI-13`                     | #9206   |
| `work.resume`                       | `fleet work resume`                                                 | absent          | `CLI-05`             | `SEL-02`, `HYD-08`, `REL-01`, `CLI-13`           | #9206   |
| `work.release`                      | `fleet work release`                                                | absent          | `REL-02`             | `CLI-05`, `CLI-13`                               | #9209   |
| `work.handoff`                      | `fleet work handoff`                                                | absent          | `DSP-06`             | `DSP-07`, `CLI-05`, `CLI-13`                     | #9203   |
| `work.complete`                     | `fleet work complete`                                               | absent          | `REL-08`             | `REL-06`, `REL-07`, `REL-09`, `CLI-13`           | #9209   |
| `work.status`                       | `fleet work status`                                                 | absent          | `CLI-03`             | `HYD-01`, `CLI-13`                               | #9206   |
| `github.issue.get`                  | `fleet github issue get`                                            | absent          | `HYD-04`             | `CLI-06`, `CLI-13`                               | #9207   |
| `github.issue.create`               | `fleet github issue create`                                         | absent          | `AUTH-01`            | `CLI-06`, `CLI-13`                               | #9208   |
| `github.issue.update`               | `fleet github issue update`                                         | absent          | `AUTH-04`            | `CLI-06`, `CLI-13`                               | #9208   |
| `github.issue.close`                | `fleet github issue close`                                          | absent          | `REL-06`             | `REL-08`, `CLI-06`, `CLI-13`                     | #9209   |
| `github.issue.comments`             | `fleet github issue comments`                                       | absent          | `HYD-05`             | `HYD-06`, `CLI-06`, `CLI-13`                     | #9207   |
| `github.issue.timeline`             | `fleet github issue timeline`                                       | absent          | `HYD-05`             | `HYD-06`, `CLI-06`, `CLI-13`                     | #9207   |
| `github.pr.get`                     | `fleet github pr get`                                               | absent          | `HYD-04`             | `CLI-06`, `CLI-13`                               | #9207   |
| `github.pr.create`                  | `fleet github pr create`                                            | absent          | `AUTH-01`            | `CLI-06`, `CLI-13`                               | #9208   |
| `github.pr.update`                  | `fleet github pr update`                                            | absent          | `AUTH-04`            | `CLI-06`, `CLI-13`                               | #9208   |
| `github.pr.diff`                    | `fleet github pr diff`                                              | absent          | `HYD-04`             | `HYD-05`, `CLI-06`, `CLI-13`                     | #9207   |
| `github.pr.commits`                 | `fleet github pr commits`                                           | absent          | `HYD-05`             | `HYD-04`, `CLI-06`, `CLI-13`                     | #9207   |
| `github.pr.reviews`                 | `fleet github pr reviews`                                           | absent          | `HYD-06`             | `HYD-07`, `CLI-06`, `CLI-13`                     | #9207   |
| `github.pr.checks`                  | `fleet github pr checks`                                            | absent          | `REL-06`             | `REL-07`, `CLI-06`, `CLI-13`                     | #9209   |
| `github.pr.merge`                   | `fleet github pr merge`                                             | absent          | `REL-06`             | `REL-07`, `REL-08`, `CLI-06`, `CLI-13`           | #9209   |
| `github.comment.add`                | `fleet github comment add`                                          | absent          | `AUTH-03`            | `REL-04`, `CLI-06`, `CLI-13`                     | #9208   |
| `github.comment.edit`               | `fleet github comment edit`                                         | absent          | `AUTH-03`            | `CLI-06`, `CLI-13`                               | #9208   |
| `github.comment.reply`              | `fleet github comment reply`                                        | absent          | `DSP-04`             | `DSP-02`, `CLI-06`, `CLI-13`                     | #9203   |
| `github.comment.list`               | `fleet github comment list`                                         | absent          | `HYD-06`             | `HYD-05`, `CLI-06`, `CLI-13`                     | #9207   |
| `github.events.since`               | `fleet github events since`                                         | absent          | `DSP-01`             | `DSP-03`, `DSP-04`, `CLI-13`                     | #9203   |
| `metadata.get`                      | `fleet metadata get`                                                | absent          | `AUTH-01`            | `CLI-07`, `CLI-13`                               | #9208   |
| `metadata.patch`                    | `fleet metadata patch`                                              | absent          | `AUTH-01`            | `AUTH-02`, `CLI-07`, `CLI-13`                    | #9208   |
| `metadata.title`                    | `fleet metadata title`                                              | absent          | `AUTH-01`            | `CLI-07`, `CLI-13`                               | #9208   |
| `metadata.summary`                  | `fleet metadata summary`                                            | absent          | `AUTH-01`            | `AUTH-04`, `CLI-07`, `CLI-13`                    | #9208   |
| `metadata.priority`                 | `fleet metadata priority`                                           | absent          | `PRI-01`             | `AUTH-02`, `CLI-07`, `CLI-13`                    | #9200   |
| `metadata.labels`                   | `fleet metadata labels`                                             | absent          | `AUTH-02`            | `DST-04`, `CLI-07`, `CLI-13`                     | #9208   |
| `metadata.fields`                   | `fleet metadata fields`                                             | absent          | `AUTH-01`            | `CLI-07`, `CLI-13`                               | #9208   |
| `dependency.list`                   | `fleet dependency list`                                             | absent          | `DEP-02`             | `CLI-08`, `CLI-13`                               | #9208   |
| `dependency.add`                    | `fleet dependency add`                                              | absent          | `DEP-03`             | `DEP-02`, `CLI-08`, `CLI-13`                     | #9208   |
| `dependency.remove`                 | `fleet dependency remove`                                           | absent          | `DEP-04`             | `DEP-02`, `CLI-08`, `CLI-13`                     | #9208   |
| `dependency.graph`                  | `fleet dependency graph`                                            | absent          | `DEP-03`             | `DEP-04`, `CLI-08`, `CLI-13`                     | #9208   |
| `dependency.blockers`               | `fleet dependency blockers`                                         | absent          | `DEP-03`             | `CLI-08`, `CLI-13`                               | #9208   |
| `dependency.ready`                  | `fleet dependency ready`                                            | absent          | `DEP-03`             | `SEL-01`, `CLI-08`, `CLI-13`                     | #9208   |
| `activity.list`                     | `fleet activity list`                                               | absent          | `HYD-06`             | `HYD-07`, `CLI-09`, `CLI-13`                     | #9207   |
| `activity.since`                    | `fleet activity since`                                              | absent          | `HYD-07`             | `HYD-06`, `CLI-09`, `CLI-13`                     | #9207   |
| `activity.reconcile`                | `fleet activity reconcile`                                          | absent          | `HYD-08`             | `HYD-05`, `HYD-06`, `CLI-09`, `CLI-13`           | #9207   |
| `activity.watch`                    | `fleet activity watch`                                              | absent          | `HYD-08`             | `HYD-06`, `CLI-09`, `CLI-13`                     | #9207   |
| `sync.status`                       | `fleet sync status`                                                 | absent          | `REL-04`             | `REL-05`, `CLI-10`, `CLI-13`                     | #9209   |
| `sync.run`                          | `fleet sync run`                                                    | absent          | `REL-05`             | `REL-04`, `CLI-10`, `CLI-13`                     | #9209   |
| `sync.verify`                       | `fleet sync verify`                                                 | absent          | `REL-06`             | `REL-07`, `CLI-10`, `CLI-13`                     | #9209   |
| `sync.outbox.list`                  | `fleet sync outbox list`                                            | absent          | `REL-04`             | `CLI-10`, `CLI-13`                               | #9209   |
| `sync.outbox.inspect`               | `fleet sync outbox inspect`                                         | absent          | `REL-05`             | `REL-04`, `CLI-10`, `CLI-13`                     | #9209   |
| `sync.outbox.retry`                 | `fleet sync outbox retry`                                           | absent          | `REL-05`             | `REL-06`, `CLI-10`, `CLI-13`                     | #9209   |
| `agent.slash.status`                | `/fleet status`                                                     | absent          | `DSP-02`             | `DSP-04`, `CLI-11`, `CLI-13`                     | #9203   |
| `agent.slash.triage`                | `/fleet triage`                                                     | absent          | `DSP-02`             | `DSP-04`, `CLI-11`, `CLI-13`                     | #9203   |
| `agent.slash.start`                 | `/fleet start`                                                      | absent          | `DSP-06`             | `DSP-05`, `DSP-07`, `CLI-11`, `CLI-13`           | #9203   |
| `agent.slash.resume`                | `/fleet resume`                                                     | absent          | `DSP-05`             | `DSP-04`, `CLI-11`, `CLI-13`                     | #9203   |
| `agent.slash.retry`                 | `/fleet retry`                                                      | absent          | `DSP-09`             | `DSP-03`, `CLI-11`, `CLI-13`                     | #9203   |
| `agent.slash.park`                  | `/fleet park`                                                       | absent          | `DSP-02`             | `CLI-05`, `CLI-11`, `CLI-13`                     | #9203   |
| `agent.slash.unblock`               | `/fleet unblock`                                                    | absent          | `DSP-02`             | `DEP-03`, `CLI-11`, `CLI-13`                     | #9203   |
| `agent.slash.priority.high`         | `/fleet priority high`                                              | absent          | `DSP-02`             | `PRI-01`, `CLI-11`, `CLI-13`                     | #9203   |
| `agent.slash.priority.numeric`      | `/fleet priority 575`                                               | absent          | `DSP-02`             | `PRI-01`, `CLI-11`, `CLI-13`                     | #9203   |
| `agent.slash.do-now`                | `/fleet do-now`                                                     | absent          | `DSP-08`             | `SEL-01`, `CLI-11`, `CLI-13`                     | #9203   |
| `agent.slash.merge`                 | `/fleet merge`                                                      | absent          | `DSP-08`             | `REL-06`, `REL-08`, `CLI-11`, `CLI-13`           | #9203   |
| `agent.slash.agent-status`          | `/fleet agent status`                                               | absent          | `DSP-05`             | `CLI-11`, `CLI-13`                               | #9203   |
| `github.plain-language-routing`     | `authorized actionable plain-language comment on live/parked issue` | absent          | `DSP-04`             | `DSP-01`, `DSP-02`, `DSP-08`, `CLI-11`, `CLI-13` | #9203   |
| `flag.scenario`                     | `--scenario queue-worker\|assigned-item\|resume\|triage\|review`    | absent          | `CLI-02`             | `DSP-02`, `CLI-13`                               | #9206   |
| `flag.format`                       | `--format compact`                                                  | absent          | `CLI-02`             | `CLI-13`                                         | #9206   |
| `flag.json`                         | `--json`                                                            | partial         | `DST-01`             | `CLI-02`, `CLI-13`                               | #9200   |
| `flag.status`                       | `--status`                                                          | absent          | `CLI-02`             | `CLI-13`                                         | #9206   |
| `flag.repo`                         | `--repo`                                                            | partial         | `CLI-02`             | `CLI-03`, `CLI-12`, `CLI-13`                     | #9206   |
| `flag.queue`                        | `--queue`                                                           | implemented     | `CLI-02`             | `CLI-13`                                         | #9206   |
| `flag.priority-min`                 | `--priority-min`                                                    | absent          | `SEL-01`             | `PRI-01`, `CLI-13`                               | #9204   |
| `flag.priority-max`                 | `--priority-max`                                                    | absent          | `SEL-01`             | `PRI-01`, `CLI-13`                               | #9204   |
| `flag.label`                        | `--label`                                                           | absent          | `CLI-02`             | `SEL-01`, `CLI-13`                               | #9206   |
| `flag.exclude-label`                | `--exclude-label`                                                   | absent          | `CLI-02`             | `SEL-01`, `CLI-13`                               | #9206   |
| `flag.assigned-to`                  | `--assigned-to`                                                     | absent          | `CLI-02`             | `SEL-01`, `CLI-13`                               | #9206   |
| `flag.unassigned`                   | `--unassigned`                                                      | absent          | `CLI-02`             | `SEL-01`, `CLI-13`                               | #9206   |
| `flag.queue-search.ready`           | `--ready`                                                           | absent          | `SEL-01`             | `DEP-03`, `CLI-02`, `CLI-13`                     | #9204   |
| `flag.queue-rank.ready`             | `fleet queue rank --ready`                                          | absent          | `SEL-01`             | `DEP-03`, `CLI-13`                               | #9204   |
| `flag.blocked`                      | `--blocked`                                                         | absent          | `DEP-03`             | `CLI-02`, `CLI-13`                               | #9208   |
| `flag.created-since`                | `--created-since`                                                   | absent          | `CLI-02`             | `CLI-13`                                         | #9206   |
| `flag.updated-since`                | `--updated-since`                                                   | absent          | `CLI-02`             | `CLI-13`                                         | #9206   |
| `flag.capability`                   | `--capability`                                                      | absent          | `SEL-01`             | `CLI-02`, `CLI-13`                               | #9204   |
| `flag.estimated-effort`             | `--estimated-effort`                                                | absent          | `SEL-01`             | `CLI-02`, `CLI-13`                               | #9204   |
| `flag.limit`                        | `--limit N`                                                         | absent          | `CLI-02`             | `CLI-13`                                         | #9206   |
| `flag.order`                        | `--order priority\|effective-urgency`                               | absent          | `PRI-03`             | `SEL-01`, `CLI-13`                               | #9199   |
| `flag.count`                        | `--count 3`                                                         | partial         | `SEL-03`             | `SEL-04`, `CLI-13`                               | #9204   |
| `flag.strategy`                     | `--strategy weighted`                                               | absent          | `SEL-03`             | `SEL-05`, `CLI-13`                               | #9204   |
| `flag.queue-rank.strategy`          | `fleet queue rank --strategy weighted`                              | absent          | `SEL-01`             | `SEL-03`, `CLI-13`                               | #9204   |
| `flag.queue-rank.explain`           | `fleet queue rank --explain`                                        | absent          | `SEL-01`             | `SEL-05`, `CLI-13`                               | #9204   |
| `flag.explain`                      | `--explain`                                                         | absent          | `SEL-05`             | `PRI-02`, `CLI-13`                               | #9204   |
| `flag.work`                         | `--work 842`                                                        | absent          | `CLI-03`             | `CLI-04`, `CLI-12`, `CLI-13`                     | #9206   |
| `flag.issue`                        | `--issue`                                                           | absent          | `CLI-03`             | `CLI-06`, `CLI-12`, `CLI-13`                     | #9206   |
| `flag.pr`                           | `--pr`                                                              | absent          | `CLI-03`             | `CLI-06`, `CLI-13`                               | #9206   |
| `flag.agent-id`                     | `--agent-id`                                                        | absent          | `DSP-04`             | `AUTH-01`, `CLI-11`, `CLI-13`                    | #9203   |
| `flag.session-id`                   | `--session-id`                                                      | implemented     | `DSP-05`             | `HYD-07`, `CLI-11`, `CLI-13`                     | #9203   |
| `flag.lease-token`                  | `--lease-token`                                                     | absent          | `REL-02`             | `CLI-10`, `CLI-13`                               | #9209   |
| `flag.lease`                        | `--lease 30m`                                                       | absent          | `REL-01`             | `REL-02`, `CLI-13`                               | #9209   |
| `flag.lane`                         | `--lane`                                                            | implemented     | `DSP-04`             | `CLI-11`, `CLI-13`                               | #9203   |
| `flag.mode`                         | `--mode immediate`                                                  | absent          | `DSP-08`             | `CLI-11`, `CLI-13`                               | #9203   |
| `flag.purpose`                      | `--purpose design`                                                  | absent          | `DSP-08`             | `CLI-11`, `CLI-13`                               | #9203   |
| `flag.since`                        | `--since ISO-8601`                                                  | absent          | `HYD-06`             | `HYD-08`, `CLI-09`, `CLI-13`                     | #9207   |
| `flag.start`                        | `--start`                                                           | absent          | `HYD-07`             | `CLI-09`, `CLI-13`                               | #9207   |
| `flag.since-start`                  | `--since-start`                                                     | absent          | `HYD-07`             | `CLI-09`, `CLI-13`                               | #9207   |
| `flag.since-checkpoint`             | `--since-checkpoint`                                                | absent          | `HYD-06`             | `JRN-03`, `CLI-09`, `CLI-13`                     | #9207   |
| `flag.external-only`                | `--external-only`                                                   | absent          | `HYD-07`             | `CLI-09`, `CLI-13`                               | #9207   |
| `flag.output`                       | `--output PATH`                                                     | absent          | `HYD-04`             | `HYD-01`, `CLI-09`, `CLI-13`                     | #9207   |
| `flag.dry-run`                      | `--dry-run`                                                         | absent          | `DST-01`             | `REL-03`, `CLI-10`, `CLI-13`                     | #9200   |
| `flag.expected-version`             | `--expected-version`                                                | absent          | `DST-01`             | `REL-02`, `CLI-10`, `CLI-13`                     | #9200   |
| `flag.idempotency-key`              | `--idempotency-key`                                                 | partial         | `REL-03`             | `DST-01`, `CLI-10`, `CLI-13`                     | #9209   |
| `flag.verify`                       | `--verify`                                                          | absent          | `REL-06`             | `REL-07`, `CLI-10`, `CLI-13`                     | #9209   |
| `flag.wait`                         | `--wait`                                                            | absent          | `REL-07`             | `CLI-10`, `CLI-13`                               | #9209   |
| `flag.timeout`                      | `--timeout`                                                         | absent          | `REL-07`             | `CLI-10`, `CLI-13`                               | #9209   |
| `flag.claim`                        | `--claim`                                                           | absent          | `CLI-04`             | `SEL-02`, `REL-01`, `CLI-13`                     | #9206   |
| `flag.hydrate`                      | `--hydrate [full\|incremental]`                                     | absent          | `CLI-04`             | `HYD-04`, `CLI-13`                               | #9206   |
| `flag.work-next.ready`              | `--ready`                                                           | absent          | `SEL-01`             | `DEP-03`, `CLI-04`, `CLI-13`                     | #9204   |
| `flag.reason`                       | `--reason TEXT`                                                     | partial         | `CLI-05`             | `JRN-01`, `CLI-13`                               | #9206   |
| `flag.checkpoint`                   | `--checkpoint`                                                      | absent          | `JRN-03`             | `HYD-06`, `CLI-05`, `CLI-13`                     | #9207   |
| `flag.release-lease`                | `--release-lease`                                                   | absent          | `REL-02`             | `CLI-05`, `CLI-13`                               | #9209   |
| `flag.merge-linked-prs`             | `--merge-linked-prs`                                                | absent          | `REL-07`             | `REL-06`, `CLI-10`, `CLI-13`                     | #9209   |
| `flag.close-linked-issues`          | `--close-linked-issues`                                             | absent          | `REL-07`             | `REL-06`, `CLI-10`, `CLI-13`                     | #9209   |
| `flag.close-queue`                  | `--close-queue`                                                     | absent          | `REL-08`             | `REL-07`, `CLI-10`, `CLI-13`                     | #9209   |
| `flag.merge-pr`                     | `--merge-pr REF`                                                    | absent          | `REL-07`             | `REL-06`, `CLI-10`, `CLI-13`                     | #9209   |
| `flag.close-issues`                 | `--close-issues REF-LIST`                                           | absent          | `REL-07`             | `REL-06`, `CLI-10`, `CLI-13`                     | #9209   |
| `flag.require-clean-reconciliation` | `--require-clean-reconciliation`                                    | absent          | `REL-09`             | `HYD-07`, `CLI-10`, `CLI-13`                     | #9209   |
| `flag.full`                         | `--full`                                                            | absent          | `HYD-04`             | `HYD-01`, `CLI-09`, `CLI-13`                     | #9207   |
| `flag.include`                      | `--include commits,reviews,checks,comments`                         | absent          | `HYD-08`             | `HYD-04`, `CLI-09`, `CLI-13`                     | #9207   |
| `flag.text`                         | `--text TEXT`                                                       | absent          | `JRN-02`             | `AUTH-02`, `CLI-07`, `CLI-13`                    | #9208   |
| `flag.mirror`                       | `--mirror github`                                                   | absent          | `AUTH-02`            | `REL-04`, `JRN-02`, `CLI-07`, `CLI-13`           | #9208   |
| `flag.add`                          | `--add VALUE`                                                       | absent          | `AUTH-01`            | `CLI-07`, `CLI-13`                               | #9208   |
| `flag.remove`                       | `--remove VALUE`                                                    | absent          | `AUTH-01`            | `DST-04`, `CLI-07`, `CLI-13`                     | #9208   |
| `flag.file`                         | `--file PATH`                                                       | absent          | `AUTH-04`            | `CLI-07`, `CLI-13`                               | #9208   |
| `flag.body-file`                    | `--body-file PATH`                                                  | absent          | `DSP-02`             | `DSP-04`, `CLI-11`, `CLI-13`                     | #9203   |
| `flag.comment-id`                   | `--comment-id ID`                                                   | absent          | `AUTH-03`            | `DSP-04`, `CLI-06`, `CLI-13`                     | #9208   |
| `flag.section`                      | `--section NAME`                                                    | absent          | `AUTH-04`            | `CLI-07`, `CLI-13`                               | #9208   |
| `flag.managed`                      | `--managed NAME`                                                    | absent          | `AUTH-03`            | `CLI-07`, `CLI-13`                               | #9208   |
| `flag.blocked-by`                   | `--blocked-by fleet:761`                                            | absent          | `DEP-02`             | `DEP-03`, `CLI-08`, `CLI-13`                     | #9208   |
| `flag.blocks`                       | `--blocks fleet:900`                                                | absent          | `DEP-02`             | `DEP-03`, `CLI-08`, `CLI-13`                     | #9208   |
| `flag.hard`                         | `--hard`                                                            | absent          | `DEP-03`             | `CLI-08`, `CLI-13`                               | #9208   |
| `flag.depth`                        | `--depth 3`                                                         | absent          | `DEP-04`             | `CLI-08`, `CLI-13`                               | #9208   |
| `flag.priority-before`              | `--before WORK`                                                     | absent          | `PRI-03`             | `DST-06`, `CLI-13`                               | #9199   |
| `flag.priority-after`               | `--after WORK`                                                      | absent          | `PRI-03`             | `DST-06`, `CLI-13`                               | #9199   |
| `flag.priority-delta`               | `--delta N`                                                         | absent          | `PRI-01`             | `PRI-02`, `CLI-13`                               | #9199   |
| `flag.note`                         | `--note TEXT`                                                       | absent          | `JRN-02`             | `JRN-03`, `CLI-05`, `CLI-07`, `CLI-13`           | #9208   |
| `flag.queue-update.status`          | `fleet queue update --status in_progress`                           | absent          | `CLI-05`             | `AUTH-01`, `CLI-13`                              | #9208   |
| `flag.work-next.strategy`           | `fleet work next --strategy weighted`                               | absent          | `SEL-03`             | `CLI-04`, `CLI-13`                               | #9204   |
| `flag.queue-search.type`            | `--type`                                                            | absent          | `CLI-02`             | `SEL-01`, `CLI-13`                               | #9206   |
| `current.claim.filter`              | `repo-queue claim --filter EXPR`                                    | implemented     | `V1-BASELINE`        | `SEL-01`, `SEL-02`, `REL-01`                     | #9209   |
| `current.claim.worker`              | `repo-queue claim --worker ID`                                      | implemented     | `V1-BASELINE`        | `REL-01`, `DSP-04`                               | #9209   |
| `current.claim.lease-seconds`       | `repo-queue claim --lease-seconds N`                                | implemented     | `V1-BASELINE`        | `REL-01`, `REL-02`                               | #9209   |
| `current.claim.bind`                | `repo-queue claim --bind`                                           | implemented     | `V1-BASELINE`        | `DSP-05`, `REL-01`                               | #9209   |
| `current.claim.from-session`        | `repo-queue claim --from-session ID`                                | implemented     | `V1-BASELINE`        | `DSP-05`, `REL-01`                               | #9209   |
| `current.claim.platform`            | `repo-queue claim --platform FAMILY`                                | implemented     | `V1-BASELINE`        | `REL-01`, `SEL-01`                               | #9209   |
| `flag.dispatch-idempotency-key`     | `--idempotency-key fleet-pm-priority-design-v1`                     | partial         | `DSP-03`             | `REL-03`, `CLI-11`, `CLI-13`                     | #9203   |
| `fleet-gh.create`                   | `fleet gh issue create / fleet gh pr create`                        | implemented     | `V1-BASELINE`        | `AUTH-01`, `CLI-06`, `CLI-13`                    | #9208   |
| `fleet-gh.passthrough`              | `fleet gh <gh-compatible command>`                                  | implemented     | `V1-BASELINE`        | `CLI-06`, `CLI-13`                               | #9208   |
| `fleet-gh.metadata-query`           | `fleet gh Metadata-aware issue/PR query by fields`                  | unverified      | `AUTH-01`            | `CLI-07`, `CLI-13`                               | #9208   |
| `outbox.generalization`             | `creation outbox producer new-issue-creator-code-a only`            | implemented     | `SCOPE-OUT-OF-SCOPE` | `REL-04`                                         | #9209   |
| `fleet.errors`                      | `structured domain error + exit code + receipt`                     | partial         | `DST-01`             | `CLI-02`, `CLI-13`                               | #9206   |
| `FDR-PMO-001`                       | `FDR-PMO-001 proposed namespace`                                    | unverified      | `COORD-DECISION`     | `DST-01`                                         | #9206   |

**Absent exact spellings remain implementation work** under the mapped granular slices; they are not dropped because an adjacent command exists. `list`≠`search`, `wake`≠`resume`, queue `complete`≠composed `work complete`, and `fleet gh`≠a new `fleet github` registration. Local slice keys are not GitHub issue IDs; actual issue URLs are pending batch filing and `issues/index.json`.

## V1 census facts and dated issue boundaries

**Queue / claim.** `turso_queue.rs:3110–3129` documents task requirements AND explicit caller filter, both empty-by-default true. `:4210–4272` selects and conditionally updates the same row atomically. Claim order lacks final ID tie while list adds `id ASC` at `:6454–6457`. Pinned source/test evidence only.

**Priority.** `pm-stack/dispatch/src/validate.ts:11–27,50–60` has live 10–90 values and snaps 15/25 to 10/20; #7030 already ratified exact `expedite=15` and `next-up=25`. Preserve those values; do not ask again. Dated readback does not prove continuing staffing/coverage.

**Project / PM.** `pm-stack/README.md:94–112` documents Project #4 and orchestrator single writer. Aggregator queue view is board/Project projection, not live broker claim order. Live mutation/deployment unverified.

**Projector intake substrate.** `pm-stack/projector/src/webhook.ts:7–57` implements bounded raw-body HMAC-SHA256 verification; `src/server.ts:147–201` exposes webhook route; `test/webhook-delivery-id.test.ts:90–154` covers signature failure/no effect, missing/empty GUID and same-GUID duplicate. README lines 23–34 describe supported event projections/cursors; lines 145–179 describe disposable hot ledger/high-water ACK. This is reusable ingress/projection only—not canonical Fleet command acceptance, action authorization, operation receipt, lifecycle outbox, or launch. Projection ACK is not accepted command. The projector does not write back upstream (README lines 3–8).

**Dispatch.** PM dispatch retains a Supabase/PostgREST task path at `pm-stack/dispatch/src/server.ts:50–53,79–83,214–250`; #9028 owns reconciliation before canonical reservation cutover. #8187 is handoff baseline; #6533 live launch/bind proof remains open.

**Lifecycle / outbox.** Queue `complete`, stack merge replay and one-producer creation outbox are bounded substrates only. No generic mutation outbox or direct verified work completion is evidenced. Exact command/test coverage is itemized above in `commands.jsonl` and Track B census.

**QTI / schema.** Queue Table Improvements remains unlocated/unverified within enumerated accessible sources. #9241/task 15480 is the real research issue; #3424/#8575/#8579/#8580–#8585 are compatibility candidates, never assumed equivalent. Schema-coupled work remains gated; pure models/read-only contracts proceed.

**Related issue evidence.** #9060/#9078 parent epics are closed while component children remain open; #9178/#9179/#9180/#9016/#8887/#9028/#7030/#8187/#6533 are distinguished in Track A. Parent closure does not prove child acceptance.

## Unmapped and missing-evidence report

- Semantic IDs and source counts are checked against the live `inventory/requirements.jsonl`; IDs are not renumbered. Every source occurrence ID referenced by the crosswalk exists in dispositions; no occurrence is silently omitted.
- Structural join result at this snapshot: 389/389 requirement IDs have one crosswalk row and a real design anchor; all 3,571 occurrence rows are dispositioned, and every substantive `requirement_link`/`candidate_link`/`history`/`example` occurrence has a semantic reference. There are zero structurally unmapped semantic IDs at this snapshot. This does **not** mean zero implementation gaps.
- Command coverage: all 215 inventory keys match 215 one-to-one slice-map entries; each exact key has a primary/supporting local slice. Those local keys still need real issue URLs after batch filing.
- Actual issue-URL join: **pending** batch filing and `issues/index.json`; no new local slice is reported as an existing GitHub issue. This is an explicit unresolved coverage join, not a fabricated zero.
- Design refs resolve to actual file headings; verified/partial rows have concrete source fields. Projector partial evidence is constrained to intake/projection, not Fleet operation acceptance.
- QTI source identity, live/deployed producer routing, Project mutation, full hydration/cursors, typed durable graph/journal, general mutation outbox and direct completion remain gaps at the stated evidence boundary.
- Source 04 remains flattened in export; all curated requirement rows/occurrences remain mapped, but missing formatting hierarchy is not treated as missing requirement.

## Validation oracle

Re-run exact set equality and uniqueness of crosswalk IDs vs current requirements JSONL; occurrence-ref existence against dispositions; command-key equality vs command-slice-map; actual-heading resolution for every design ref; enum check for status; concrete evidence fields for verified/partial rows; and one primary slice for every exact command row. A zero unmapped count is valid only after those checks; issue URL joins remain pending until real filing.
