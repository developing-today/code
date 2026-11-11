# Commission and running questions — Atlas v2–v4

Date: 2026-11-11 UTC. Starter: [#6020](https://github.com/developing-today/code/issues/6020).
Program: [#6018](https://github.com/developing-today/code/issues/6018).
Queue: 14861; worker: `repo-worker-code`.
Baseline checkout: `56cf19199e7cb7ab37860ee1751506dd11c39042`.

## Deliverable and scope

One expansive, dated design document in this directory: statement inventory,
primary-source state-of-the-art expansion, the V1 baseline and dependency-tagged
V2/V3/V4 horizons with entry criteria, and granular, independently buildable
follow-ons. Survey all seven starting threads from #6020. Explicitly label dead
ends. Research breadth is required; implementation, provider migrations,
autonomous deletion, and premature implementation commitments are not authorized.

Supporting research files are evidence appendices, not separate competing designs.
The final design is the synthesis and navigation entry point.

## Running questions and chosen defaults

### Q01 — Does a merged v1 design mean its runtime is deployed?

**Default:** no. Distinguish the landed design contract from repository
implementation evidence and from deployment evidence. V2 entry requires the
runtime proof named by its gate; a closed issue or a merged document alone does
not satisfy that gate. Continue the design without waiting for deployment.

### Q02 — Do usefulness and plan pressure authorize deletion or lower classification?

**Default:** no. Usefulness can propose reversible placement changes; mandatory
retention, authorization, revocation, licensing and human-approved deletion stay
hard constraints. V1's cold-before-retirement lifecycle is not replaced. Required
privacy/legal erasure is a separate deletion-propagation workflow, not a
usefulness decision and not delayed by a cold-storage preference.

### Q03 — Are all seven threads required to ship together at each horizon?

**Default:** no. The horizons are capability gates, not calendar promises or a
big-bang release. Independently buildable evaluation and contract-verification
slices may proceed earlier; each names its actual hard prerequisites.

### Q04 — How should broad historical-completeness language handle forbidden or missing history?

**Default:** completeness is relative to the ingested, retained, authorized
snapshot. Never assert universal knowledge of every historical copy. Public
responses must not disclose the existence, count, path or graph degree of
forbidden occurrences; authorized audit surfaces can report permitted gaps.

## Working method

Five workhorse sessions own disjoint evidence files: baseline inventory,
personal-knowledge systems, provenance/lineage, feedback retrieval, and follow-on
deduplication. The summit owner retains architecture, stage boundaries, trade-off
judgment, synthesis and the final top-to-bottom pass. An independent reviewer
must agree before merge. All GitHub operations use `gh`.
