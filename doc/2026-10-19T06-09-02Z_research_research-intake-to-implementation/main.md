# Research and Delivery Workflow Framework

A portable, evidence-preserving framework transforming complex, rambling, or voice-transcribed requests into structured research, aligned designs, phased feature plans, and parallel PR deliveries.

<!-- workflow:state -->

{
"version": 1,
"id": "2026-10-19T06-09-02Z_research_research-intake-to-implementation",
"created_at": "2026-10-19T06:09:02Z",
"stage": "intake",
"status": "prepared",
"transcript": {
"path": "transcript.txt",
"sha256": "e250cfa228ae4e79e984ddac6a754da14da3429770d5ad4112d1fc579c21299f"
},
"concepts": [
{
"id": "C001",
"title": "Exact Transcript Storage and Integrity Sealing",
"path": "concepts/C001-transcript-preservation.md",
"status": "created",
"source_lines": [
[
1,
5
],
[
7,
7
]
],
"priority": "crucial",
"horizon": "now"
},
{
"id": "C002",
"title": "Main Reference and Review Order Meta Design",
"path": "concepts/C002-main-reference-design.md",
"status": "created",
"source_lines": [
[
8,
12
],
[
15,
15
],
[
17,
17
]
],
"priority": "crucial",
"horizon": "now"
},
{
"id": "C003",
"title": "Single-Writer Concept Files and Mutation Invariance",
"path": "concepts/C003-single-writer-concepts.md",
"status": "created",
"source_lines": [
[
19,
19
],
[
45,
45
]
],
"priority": "crucial",
"horizon": "now"
},
{
"id": "C004",
"title": "Preliminary Research and Trigger Preparation",
"path": "concepts/C004-research-prepare.md",
"status": "created",
"source_lines": [
[
21,
25
]
],
"priority": "recommended",
"horizon": "now"
},
{
"id": "C005",
"title": "Deep Factual Research and Cross-Agent Notes",
"path": "concepts/C005-deep-research-notes.md",
"status": "created",
"source_lines": [
[
27,
31
]
],
"priority": "crucial",
"horizon": "next"
},
{
"id": "C006",
"title": "Asynchronous Notes Reconciliation and Receipts",
"path": "concepts/C006-notes-reconciliation.md",
"status": "created",
"source_lines": [
[
33,
33
]
],
"priority": "crucial",
"horizon": "next"
},
{
"id": "C007",
"title": "Hierarchical Issue Handoff and Research Fanout",
"path": "concepts/C007-issue-handoff-fanout.md",
"status": "created",
"source_lines": [
[
35,
37
]
],
"priority": "recommended",
"horizon": "next"
},
{
"id": "C008",
"title": "Hierarchical Items, Status Trails, and Change History",
"path": "concepts/C008-status-trails-hierarchy.md",
"status": "created",
"source_lines": [
[
39,
41
]
],
"priority": "recommended",
"horizon": "now"
},
{
"id": "C009",
"title": "Worktree Boot Protocol and Merge-Commit Pull Requests",
"path": "concepts/C009-worktree-boot-protocol.md",
"status": "created",
"source_lines": [
[
49,
51
]
],
"priority": "crucial",
"horizon": "now"
},
{
"id": "C010",
"title": "Thin Orchestration, Merge Verification, and Worker Lifecycles",
"path": "concepts/C010-thin-orchestration.md",
"status": "created",
"source_lines": [
[
53,
55
]
],
"priority": "crucial",
"horizon": "now"
},
{
"id": "C011",
"title": "Disagreement Protocol, Reviewer Subagents, and Preliminary Decisions",
"path": "concepts/C011-disagreement-preliminary-decisions.md",
"status": "created",
"source_lines": [
[
58,
64
],
[
67,
67
]
],
"priority": "crucial",
"horizon": "next"
},
{
"id": "C012",
"title": "Phased Feature Implementation Planning and Parallel Frontier",
"path": "concepts/C012-feature-planning-frontier.md",
"status": "created",
"source_lines": [
[
47,
47
]
],
"priority": "recommended",
"horizon": "next"
},
{
"id": "C013",
"title": "Cairn Upstream Library, Generic Reusability, and Docs Architecture",
"path": "concepts/C013-cairn-library-docs-architecture.md",
"status": "created",
"source_lines": [
[
43,
43
],
[
69,
71
]
],
"priority": "crucial",
"horizon": "now"
},
{
"id": "C014",
"title": "Dependency Topology, Hard Blockers, and Priority Tiers",
"path": "concepts/C014-dependency-topology-priority.md",
"status": "created",
"source_lines": [
[
73,
73
]
],
"priority": "crucial",
"horizon": "now"
}
],
"dependencies": [
{
"from": "C002",
"to": "C001",
"kind": "requires",
"reason": "Main reference and design sequencing require a sealed transcript."
},
{
"from": "C003",
"to": "C001",
"kind": "requires",
"reason": "Single-writer concept stubs are scaffolded from the sealed transcript."
},
{
"from": "C004",
"to": "C003",
"kind": "requires",
"reason": "Preliminary research preparation operates on established concept stubs."
},
{
"from": "C005",
"to": "C004",
"kind": "requires",
"reason": "Deep research requires contextual research angles and trigger issues."
},
{
"from": "C006",
"to": "C005",
"kind": "requires",
"reason": "Notes reconciliation operates on directed notes generated during deep research."
},
{
"from": "C007",
"to": "C006",
"kind": "requires",
"reason": "Issue handoff requires that all cross-concept notes are fully reconciled."
},
{
"from": "C008",
"to": "C002",
"kind": "requires",
"reason": "Hierarchical item trails and status evolution belong inside main.md sections."
},
{
"from": "C009",
"to": "C003",
"kind": "requires",
"reason": "Worktree boot protocol enforces isolated execution for single-writer concepts."
},
{
"from": "C010",
"to": "C009",
"kind": "requires",
"reason": "Thin orchestrator monitors PRs and branches created by the worktree boot protocol."
},
{
"from": "C011",
"to": "C005",
"kind": "requires",
"reason": "Disagreement protocol arbitrates conflicting proposals arising from deep research."
},
{
"from": "C012",
"to": "C011",
"kind": "requires",
"reason": "Feature planning translates reconciled design records into granular steps."
},
{
"from": "C013",
"to": "C002",
"kind": "requires",
"reason": "Cairn library synchronization and docs architecture structure repository references."
},
{
"from": "C014",
"to": "C002",
"kind": "requires",
"reason": "Dependency topology governs scheduling across all concept documents."
}
],
"issues": [],
"prs": [],
"tasks": [],
"events": [
{
"at": "2026-10-19T06:09:02Z",
"stage": "intake",
"status": "prepared",
"reason": "Exact transcript captured and initial concept files prepared."
}
]
}

<!-- /workflow:state -->

## Reading of Garbled Input

The source transcript was captured from spoken/voice transcription. The following interpretation register records phonetic substitutions and inferred terminology:

| Raw Token                       | Normalized Reading              | Category                    | Evidence / Rationale                                                             |
| ------------------------------- | ------------------------------- | --------------------------- | -------------------------------------------------------------------------------- |
| `dezren39 cairn` / `Karen repo` | `dezren39/cairn` repository     | Homophone / Transliteration | Personal agent prompt/skill repository referenced in task instructions.          |
| `Matthew Picock`                | Matt Pocock                     | Name phonetic error         | Author of AI engineering agent skills and `aihero.dev` workflows.                |
| `**§**`                         | "features" / "spec sections"    | Symbol substitution         | Spoken word "sections" or "features" transcribed as section symbol `§`.          |
| `zeta files`                    | "extra files" / "scratch notes" | Phonetic error              | Greek letter zeta substituted for "extra files" or "beta files" (to/from notes). |
| `ale skills`                    | "make skills"                   | Transcription typo          | Spoken "make skills" misheard as "ale skills".                                   |

## Files

- [main.md](main.md)
- [transcript.txt](transcript.txt)
- [design.md](design.md)
- [changes.md](changes.md)
- [concepts/C001-transcript-preservation.md](concepts/C001-transcript-preservation.md)
- [concepts/C002-main-reference-design.md](concepts/C002-main-reference-design.md)
- [concepts/C003-single-writer-concepts.md](concepts/C003-single-writer-concepts.md)
- [concepts/C004-research-prepare.md](concepts/C004-research-prepare.md)
- [concepts/C005-deep-research-notes.md](concepts/C005-deep-research-notes.md)
- [concepts/C006-notes-reconciliation.md](concepts/C006-notes-reconciliation.md)
- [concepts/C007-issue-handoff-fanout.md](concepts/C007-issue-handoff-fanout.md)
- [concepts/C008-status-trails-hierarchy.md](concepts/C008-status-trails-hierarchy.md)
- [concepts/C009-worktree-boot-protocol.md](concepts/C009-worktree-boot-protocol.md)
- [concepts/C010-thin-orchestration.md](concepts/C010-thin-orchestration.md)
- [concepts/C011-disagreement-preliminary-decisions.md](concepts/C011-disagreement-preliminary-decisions.md)
- [concepts/C012-feature-planning-frontier.md](concepts/C012-feature-planning-frontier.md)
- [concepts/C013-cairn-library-docs-architecture.md](concepts/C013-cairn-library-docs-architecture.md)
- [concepts/C014-dependency-topology-priority.md](concepts/C014-dependency-topology-priority.md)

Transcript SHA-256: `e250cfa228ae4e79e984ddac6a754da14da3429770d5ad4112d1fc579c21299f`. Source bytes are immutable.

<a id="c001"></a>

## C001: Exact Transcript Storage and Integrity Sealing

Document: [concepts/C001-transcript-preservation.md](concepts/C001-transcript-preservation.md). Current status: created.

Preserve the raw user prompt byte-for-byte in transcript.txt without normalization or whitespace changes, verifying its integrity via SHA-256.

Related issue/PR results: pending; none created.

<a id="c002"></a>

## C002: Main Reference and Review Order Meta Design

Document: [concepts/C002-main-reference-design.md](concepts/C002-main-reference-design.md). Current status: created.

Generate main.md as the primary collection anchor with per-concept sections, preliminary thoughts, and design.md specifying initial review order and waves.

Related issue/PR results: pending; none created.

<a id="c003"></a>

## C003: Single-Writer Concept Files and Mutation Invariance

Document: [concepts/C003-single-writer-concepts.md](concepts/C003-single-writer-concepts.md). Current status: created.

Enforce strict single-writer file ownership where concepts own dedicated files and shared wide documents are modified only via pull requests.

Related issue/PR results: pending; none created.

<a id="c004"></a>

## C004: Preliminary Research and Trigger Preparation

Document: [concepts/C004-research-prepare.md](concepts/C004-research-prepare.md). Current status: created.

Map repository context, codebase architecture, and prior art to draft research angles and self-contained trigger issues before deep investigation.

Related issue/PR results: pending; none created.

<a id="c005"></a>

## C005: Deep Factual Research and Cross-Agent Notes

Document: [concepts/C005-deep-research-notes.md](concepts/C005-deep-research-notes.md). Current status: created.

Conduct deep factual research per concept, steelmanning alternatives and exchanging cross-cutting findings via immutable directed notes.

Related issue/PR results: pending; none created.

<a id="c006"></a>

## C006: Asynchronous Notes Reconciliation and Receipts

Document: [concepts/C006-notes-reconciliation.md](concepts/C006-notes-reconciliation.md). Current status: created.

Process all directed notes sent to a concept, recording atomic resolution receipts and incorporating findings before handoff.

Related issue/PR results: pending; none created.

<a id="c007"></a>

## C007: Hierarchical Issue Handoff and Research Fanout

Document: [concepts/C007-issue-handoff-fanout.md](concepts/C007-issue-handoff-fanout.md). Current status: created.

Generate self-contained parent and child GitHub issues for concepts and facets, establishing the initial research stages in design.md.

Related issue/PR results: pending; none created.

<a id="c008"></a>

## C008: Hierarchical Items, Status Trails, and Change History

Document: [concepts/C008-status-trails-hierarchy.md](concepts/C008-status-trails-hierarchy.md). Current status: created.

Maintain granular hierarchical items (C001.1, C001.2.1) and prepend status trails (created -> clarified -> confirmed) to track evolution over time.

Related issue/PR results: pending; none created.

<a id="c009"></a>

## C009: Worktree Boot Protocol and Merge-Commit Pull Requests

Document: [concepts/C009-worktree-boot-protocol.md](concepts/C009-worktree-boot-protocol.md). Current status: created.

Execute parallel workers in isolated git worktrees, immediately pushing empty branches, creating draft PRs, and landing via merge commits.

Related issue/PR results: pending; none created.

<a id="c010"></a>

## C010: Thin Orchestration, Merge Verification, and Worker Lifecycles

Document: [concepts/C010-thin-orchestration.md](concepts/C010-thin-orchestration.md). Current status: created.

Enforce thin orchestrators that manage ready frontiers, verify worker commit SHAs before merging, and restart stalled workers without implementing.

Related issue/PR results: pending; none created.

<a id="c011"></a>

## C011: Disagreement Protocol, Reviewer Subagents, and Preliminary Decisions

Document: [concepts/C011-disagreement-preliminary-decisions.md](concepts/C011-disagreement-preliminary-decisions.md). Current status: created.

Arbitrate conflicting worker recommendations by dispatching reviewer and critic subagents to make reasoned preliminary decisions without stalling.

Related issue/PR results: pending; none created.

<a id="c012"></a>

## C012: Phased Feature Implementation Planning and Parallel Frontier

Document: [concepts/C012-feature-planning-frontier.md](concepts/C012-feature-planning-frontier.md). Current status: created.

Translate consolidated designs into granular end-to-end implementation plans with numbered steps, automated test seams, and [P] parallel markers.

Related issue/PR results: pending; none created.

<a id="c013"></a>

## C013: Cairn Upstream Library, Generic Reusability, and Docs Architecture

Document: [concepts/C013-cairn-library-docs-architecture.md](concepts/C013-cairn-library-docs-architecture.md). Current status: created.

Maintain Cairn as the generic prompt and workflow library, sync portable components via sync script, and organize repo docs into a clean hierarchy.

Related issue/PR results: pending; none created.

<a id="c014"></a>

## C014: Dependency Topology, Hard Blockers, and Priority Tiers

Document: [concepts/C014-dependency-topology-priority.md](concepts/C014-dependency-topology-priority.md). Current status: created.

Distinguish hard blockers (requires) from soft sequencing (recommended-after), concurrency fences, and priority tiers (crucial, recommended, contingent).

Related issue/PR results: pending; none created.

## Issues and PRs

None created. Each later entry must identify its concept, stage, status and disposition.
