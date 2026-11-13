# Source section and disposition map

This register supplies stable structural anchors for the later requirement crosswalk. It separates source metadata/history/examples from normative and candidate material. The `statements.jsonl` IDs are preserved occurrence IDs, not atomic requirement IDs. The curated register uses a separate `*-R###` namespace.

## Source-to-design domain map

| Source | Section/anchor | Primary starter destination(s) | Content class |
|---|---|---|---|
| 01 handoff | Preservation and intended result (lines 9–29) | #9198, #9206 | governing requirement / workflow direction |
| 01 handoff | Reconcile current work (30–43) | #9198, #9200, #9206 | census, precedence, integration constraint |
| 01 handoff | Preparation and verified launch (45–49) | #9198, #9203 | historical coordination/process requirement; current live changes prohibited by this design-only pass |
| 01 handoff | Requirements survive refinement (51–59) | #9199–#9209 | normative cross-track checklist; preserve individual outcomes |
| 01 handoff | Supervision and completion (61–65) | #9198, #9203, #9209 | execution evidence and design deliverables |
| 01 handoff | Preservation/distributed priority (67–73) | #9198–#9209, especially #9200 | preservation, command inventory, distribution contract |
| 01 handoff | Historical prompt (121–129) | #9198, #9203 | historical example, superseded dispatch details; preserve Queue Table Improvements anchor |
| 02 charter | §§1–2 | #9198, #9206 | mandate and coordinator requirements |
| 02 charter | §§3–5 | #9199, #9200, #9204 | axes, score bands, dynamic triage, aging, hysteresis, boosts |
| 02 charter | §§6–8 | #9200, #9201, #9203 | labels, projection, mobile, authenticated commands/events |
| 02 charter | §9 | #9204 | selection and exploration requirements + candidate defaults |
| 02 charter | §§10–11 | #9206, #9207 | CLI/workflows, hydration, activity and cursors |
| 02 charter | §§12–14 | #9208, #9209 | authority, dependency types, leases, outbox, direct completion |
| 02 charter | §§15–18 | #9198, #9206–#9209 | scenarios, acceptance, evidence, source references |
| 03 proposal | §§1–2 | #9198, #9206–#9209 | workflow outcomes and principles; examples are not normative by themselves |
| 03 proposal | §§3–5 | #9204, #9206–#9209 | logical component responsibilities and composed commands |
| 03 proposal | §§6–7 | #9200, #9204, #9206 | illustrative data model, ranking criteria |
| 03 proposal | §§8–9 | #9207 | hydration sources, fields, files, completeness and summary |
| 03 proposal | §§10–12 | #9200, #9207, #9208 | synchronization authority, activity attribution, typed dependencies |
| 03 proposal | §§13–15 | #9206, #9209 | lifecycle/lease/checkpoint/time-budget/outbox requirements |
| 03 proposal | §§16–18 | #9206–#9209 | worked examples; scenario examples retain examples modality |
| 03 proposal | §§19–21 | #9209, #9206 | direct completion, failures, machine-readable output and errors |
| 03 proposal | §§22–26 | #9198, #9206–#9209 | implementation discovery, auth, observability, phases, acceptance, direction |
| 04 recovered original | flattened transcript | #9199–#9209 | historic requirements, alternatives and examples; use exact byte spans, not inferred line sections |
| 05 supplemental | raw note (line 9) | #9199–#9204, #9201, #9203 | user-authored candidate requirements; typo/garbling remains source data |
| 05 supplemental | weighted selection (lines 12–21) | #9204 | parameterized candidate formula and rationale |

## Occurrence disposition rules

The complete file/range disposition is generated in `occurrence-disposition.jsonl` by the validator's build step. A source span is designated as one of: `requirement` (mapped to one or more curated R IDs), `candidate` (mapped to R IDs but explicitly non-governing), `history` (preserved historical process/prompt, mapped or superseded with rationale), or `non-requirement` (e.g. source metadata, heading, separator, syntax-only continuation, example output). No unmatched semantic prose may be silently put in the last category: it must be added to the curated requirements or reported as a blocking unmapped span.

Target references such as #9207 are **planned design destinations**, not evidence of issue implementation coverage. An audit must report requirements with no design decision separately from implementation/test/issue status.
