# Maintaining and publishing the ID book

The reference book was delivered by [PR #13358](https://github.com/developing-today/code/pull/13358) for [#13332](https://github.com/developing-today/code/issues/13332). The [combined Pages run](https://github.com/developing-today/code/actions/runs/45191802593) publishes to the [ID section of the shared site](https://developing-today.github.io/code/id/).

## Keep reference and research evidence in sync

Changes to a public crate, capability, host profile or protocol should update the affected reference chapter in the same change. The research section records its source revision and the date of its issue census. Preserve the historical observation when reporting later changes and link the successor issue, PR or run. An architectural decision and a passing runtime check answer different questions.

Use the [Integrated implementation plan](implementation-plan.md) for the order of work, the [Route each open issue to its next proof](roadmap-routing.md) for individual acceptance criteria and the [coverage evidence](evidence/coverage-receipt.json) for the November 23 research inventory. The archive and the book retain separate roles. The [Three design records for repository review](research-contracts.md) give the decision boundary and link to the complete repository records.

## Publish through the combined site

1. Read the documentation contract in the [ID README](https://github.com/developing-today/code/blob/main/pkgs/id/README.md) and the [book configuration](https://github.com/developing-today/code/blob/main/pkgs/id/book.toml). Keep the existing navigation and add research chapters to the same contents file.
2. Verify a targeted docs build with Fleet against the proposed ref. Check chapter titles against the contents, local links, diagrams, downloadable evidence and warnings from the builder. Record the commit, run and artifact.
3. Merge after reviewing the latest issue and ownership comments. The main-only shared Pages workflow publishes the repository site. Confirm the ID section, a deep research link and an evidence asset from the deployed artifact. Reference the run and the revision it actually deployed.

## Evidence for readers

The 7:41 a.m. Central issue census and its 8:15 a.m. membership recheck on November 23, 2026 establish exactly which issues were present in that edition. A later code or issue change needs a separate timestamp and source. A code-level invariant can have a worked acceptance fixture before a production host, physical device or performance run has been verified. [ID issue map](issue-map.md) · [What the scale measurements establish and which experiment comes next](research-measurement.md) · [Program tracker](https://github.com/developing-today/code/issues/13240)
