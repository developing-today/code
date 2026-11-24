# The forks programme dependency map

This map draws only the edges the survey states. Each edge names its source: an issue title, an issue body, or a README the survey read. An edge not in this chapter is not an edge this book asserts, however plausible it reads. The survey's own scope line for its dependency notes stands: only edges seen in titles/bodies/READMEs actually read.

## Edges within the code repo

1. **code#1357 follows code#1344; code#1363 follows code#1344.** The attach-first family: the research issue [code#1344](https://github.com/developing-today/code/issues/1344) (closed) is the parent; the client helper [code#1357](https://github.com/developing-today/code/issues/1357) (closed) is titled "the #1344 follow-through", and the tailnet `serve` evaluation [code#1363](https://github.com/developing-today/code/issues/1363) (closed) belongs to the #1344/#1357 family. Source: titles/bodies of the three issues. The goal is stated in [Goals and intentions](goals-and-intentions.md).

## Edges from the code repo into the forks

2. **code#13675 Part 2 is blocked by opencode#2782.** [opencode#2782](https://github.com/developing-today-forks/opencode/issues/2782) (config projection strips `model_routes` "purpose" and top-level `model_designations`) states in its title that it is "needed by code#13675 Part 2" — the fork fix blocks that code work. Source: the issue title.
3. **code#5060 is the ruling behind opencode#2490 and opencode#2491.** Both fork issues cite the code#5060 ruling. #2491 (consolidation: merge OpenChamber's server-side control into OpenCode) is the parent intention; #2490 (quota credential: provider-only session-cookie credential in the server credential store) is named "(code#5060 fix)". Source: the two issues as recorded in the survey §5. The ruling itself is stated in [The forks: what they are and why we carry them](the-forks.md).

## Edges within the opencode fork

4. **opencode#2681 follows opencode#2674, #2676 and #2679.** [opencode#2681](https://github.com/developing-today-forks/opencode/issues/2681) is explicitly "follow-ups after #2674, #2676 and #2679" (same-repo opencode numbers). Source: the issue title.
5. **opencode#2505 is about opencode PR #2499; the TursoDB evaluation is opencode#2492.** [opencode#2505](https://github.com/developing-today-forks/opencode/issues/2505) records that PR #2499 adds zero tests and `OPENCODE_DB_ENGINE=turso` is never exercised before merge. The evaluation itself is the open spike [opencode#2492](https://github.com/developing-today-forks/opencode/issues/2492), which also owns the standing branch `spike/2492-tursodb-backend`. Source: the survey §5, from the issue and branch records.

## Edges between the forks

6. **openchamber#384 depends on the opencode fork's v2 line.** [openchamber#384](https://github.com/developing-today-forks/openchamber/issues/384) (update the embedded OpenCode to a v2-based build) is the OpenChamber side of the same migration as opencode#374/#2295/#2290. The openchamber README likewise states its messaging surface "needs an OpenCode build with messaging (developing-today-forks/opencode)" — the fork's claimed divergence (7) in [The forks: what they are and why we carry them](the-forks.md) rests on the other fork's build. Source: the issue record and the fork README, as read in the survey.

## Edges inside the PM-stack integration surface

7. **The claimant chain.** Babysitter rounds (code#1338) produce the confirmed stalls that the nudges slice (code#1187's next slice) consumes. The claimant's session contract is code#4912 phase 4 (audit code#4916) over phases code#4757/#4870/#4758, and its seam contract is code#7770 over code#7325/#7375/#7706, #7367/#7760, #7356/#7366/#7732. Source: the claimant README, as deposited. The claimant itself is recorded in [Our changes, shipped and in flight](our-changes.md).
8. **Usage export slices 2–7 depend on slice 1.** `pm-stack/opencode-usage-export` is code#2072 slice 1; its README states slices 2–7 (webhooks, views, rollups, projections) are separate follow-ups — they depend on this exporter existing, not vice versa. Source: the exporter README.
9. **Scheduled jobs depend on the claimant babysitter and on the aggregator's quota snapshot.** The `opencode-scheduled-jobs` README states stall supervision "remains the babysitter's responsibility", and the registry's one job takes the aggregator's quota snapshot file as its only input. Source: the scheduled-jobs README and `jobs.json`.

## Edges within the openchamber fork

10. **openchamber#1573 and openchamber#937 share parent PR #216.** [openchamber#1573](https://github.com/developing-today-forks/openchamber/issues/1573) is a split of openchamber PR #216 ("Split 216b of PR #216"); [openchamber#937](https://github.com/developing-today-forks/openchamber/issues/937) is the stale-PR resolution for the same PR #216. Source: the two issue titles.

## Non-edges, recorded as non-edges

11. **No other blocking edges are asserted.** No `blocked-by` label or explicit dependency field exists on the fork repos beyond the `status/blocked`/`queue/blocked` labels shown in the issue maps. Those labels mark an issue as blocked; they name no blocker, and this map draws no edge from them. A reader who needs a blocker for a `status/blocked` issue — opencode#2502, opencode#1708, openchamber#1642, and the blocked rows of the code set among them — finds none stated in the records the survey read. That absence is the record, and this chapter leaves it as an absence rather than filling it with inferred edges.

For the wider estate, the fleet book's agent chapter records how the `opencode` and `openchamber` kinds enrol and take work: [Agents: enrolment, doctor and the intake ladder](https://developing-today.github.io/code/fleet/concepts/agents.html). That is the fleet programme's record of the same two names in a different role — kinds of agent, not forks — and no dependency edge between the two records is asserted here.

Sources. Survey file `~/workspace/pm-book/survey/oc-programme.md` §5 (all ten edges and the non-edge record), §1.3 (the openchamber README messaging dependency), §4.1–§4.3 (the integration-surface edges); deposit `~/workspace/pm-book/sideline-material/oc-survey/` (claimant, scheduled-jobs and usage-export READMEs as deposited).
