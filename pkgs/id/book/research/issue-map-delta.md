# ID issue census update, November 23, 2026

Use the [Current ID issue map](current-issue-map.md) for the latest published issue census and blocker changes.

The [The first ID apps and the #2056 family](research-apps-backlog.md) resolves the changed program denominator separately from the three-label census.

**218 distinct open issues** matched the three requested labels between **12:50:30 and 12:50:43 UTC**. The earlier 12:34–12:38 UTC snapshot held 219. The sole issue removed was [#12657](https://github.com/developing-today/code/issues/12657). No new issue entered the union during the interval.

| GitHub REST Search query | Earlier count | Updated count | Pages of up to 100 |
| --- | ---: | ---: | ---: |
| [Open app/id issues](https://api.github.com/search/issues?q=repo%3Adeveloping-today%2Fcode%20is%3Aissue%20is%3Aopen%20label%3A%22app%2Fid%22) | 204 | 203 | 3 |
| [Open epic/id-program issues](https://api.github.com/search/issues?q=repo%3Adeveloping-today%2Fcode%20is%3Aissue%20is%3Aopen%20label%3A%22epic%2Fid-program%22) | 188 | 187 | 2 |
| [Open epic/id-platform issues](https://api.github.com/search/issues?q=repo%3Adeveloping-today%2Fcode%20is%3Aissue%20is%3Aopen%20label%3A%22epic%2Fid-platform%22) | 35 | 35 | 1 |
| **Union by repository issue number** | **219** | **218** | **6 fetched** |

Each label query sorted by creation date ascending. Every page returned `incomplete_results: false` and agreed on that label’s total. The number of distinct issue numbers on the combined pages equalled that total for each label. The records all reported open status and carried the requested label. Search results represent a timed observation of GitHub’s index.

| Change | Verified consequence |
| --- | --- |
| [#12657](https://github.com/developing-today/code/issues/12657) closed at **12:41:25 UTC** | Present at rank 141 in the earlier union with both app/id and epic/id-program. Its scene, audio and widget steps are recorded by [#13214](https://github.com/developing-today/code/pull/13214), [#13244](https://github.com/developing-today/code/pull/13244) and [#13266](https://github.com/developing-today/code/pull/13266). The widget PR merged at 12:41:04 UTC. |
| [#3404](https://github.com/developing-today/code/issues/3404) gained `status:in-review` | This was the only full-label-set difference among retained issues. [#13167](https://github.com/developing-today/code/pull/13167) and [#13275](https://github.com/developing-today/code/pull/13275) were open when checked. #13275 specifically covers the durable source-freeze acknowledgement before export. |

## Program tracking at the earlier census

The [existing ID program tracker #13240](https://github.com/developing-today/code/issues/13240) was open and carried only `status:in-review`; this is why it needs a separate pointer in a book covering the exact three-label union. [#13243](https://github.com/developing-today/code/pull/13243) merged the dated program prompt at 12:08:49 UTC. The separate [Atlas book #13251](https://github.com/developing-today/code/issues/13251) belongs to Atlas.

A standalone issue-title search for “ID book” returned zero. The additional repository-wide search enumerated 15 issues created since 12:00 UTC, without truncation. It found no same-window separate issue filing for the newly identified chunk-order documentation ambiguity or fixture-reserved policy; [#5814](https://github.com/developing-today/code/issues/5814) and [#9082](https://github.com/developing-today/code/issues/9082) are the existing relevant work items. This describes the search scope and does not establish there is no older related filing. [Recent-issue Search REST query](https://api.github.com/search/issues?q=repo%3Adeveloping-today%2Fcode%20is%3Aissue%20created%3A%3E%3D2026-11-23T12%3A00%3A00Z) · [ID book title Search REST query](https://api.github.com/search/issues?q=repo%3Adeveloping-today%2Fcode%20is%3Aissue%20in%3Atitle%20%22ID%20book%22)

## November 23, 7:41 a.m. Central

The next three-label REST union contains **216 open issues**. App/id has 201, epic/id-program 185, and epic/id-platform 35. Two previously mapped items closed: [hosted export/import #3810](https://github.com/developing-today/code/issues/3810) and [FileOps audit #4767](https://github.com/developing-today/code/issues/4767). The import refusal and rebound-binding tests landed in [#13312](https://github.com/developing-today/code/pull/13312), superseding closed, unmerged [#13196](https://github.com/developing-today/code/pull/13196). The CSE-choice follow-up [#13313](https://github.com/developing-today/code/pull/13313) was open at the check. The FileOps audit landed in [#13308](https://github.com/developing-today/code/pull/13308). #3399 and #3400 gained the review label. #3818 lost status/blocked but retained queue/blocked.

[Book delivery #13332](https://github.com/developing-today/code/issues/13332) owned the publication requirements at that observation. The newly filed issue had one task-link comment when checked. [The program tracker](https://github.com/developing-today/code/issues/13240#issuecomment-7100577390) still had four comments, with Lane UI’s completed starter PR the latest. [Xaltha’s claim](https://github.com/developing-today/code/issues/1374#issuecomment-7100174570) covers the two-provider regression. The [Fleet retry](https://github.com/developing-today/fleet/actions/runs/45175679056) on the owner’s branch reports success. The test log was unavailable to this reviewer. The morning check preceded [Xaltha’s follow-up PR #13368](https://github.com/developing-today/code/pull/13368). Xaltha’s [full follow-up on the same commit](https://github.com/developing-today/code/actions/runs/45177254952/job/137701628677) failed in the Nix flake check. The [captured log](https://github.com/developing-today/code/actions/runs/45177254952/artifacts/14821898444) requires authenticated access. Inspect that log to identify the failed check before changing the test or dispatching again. The morning hold on #13084 was subsequently released and the issue closed with [join benchmark PR #13424](https://github.com/developing-today/code/pull/13424). See the [issue decision](https://github.com/developing-today/code/issues/13084#issuecomment-7105955143) for the authorized result.

## November 23, 8:15 a.m. Central

An independent three-label Search REST census matched all 216 saved issue identities. Zero were added and zero removed. Query pages, service dates and the exact ID comparison are preserved. [Membership comparison](evidence/membership-recheck.json)

## Reference publication

The reference book was delivered by [PR #13358](https://github.com/developing-today/code/pull/13358) for [#13332](https://github.com/developing-today/code/issues/13332). The [combined Pages run](https://github.com/developing-today/code/actions/runs/45191802593) publishes to the [ID section of the shared site](https://developing-today.github.io/code/id/).
