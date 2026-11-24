# Project Management Space — OpenCode & OpenChamber: the forks programme

> Pack file 5 of 8 · Programme developing-today/code#13900 · Coordinator PM-BOOK
>
> ## ⚠ TODO STUB — survey input missing (checked 2026-11-23, end of SPACE-PACK run)
>
> The source survey for this file, **`~/workspace/pm-book/survey/oc-programme.md`,
> was not present** when this pack was assembled (checked at the start of the
> run and again at the end; the survey directory contained only `apps.md`,
> `disagreements.md`, `llm-evidence.md`, `models.md`, `push.md`,
> `roadmap.md`, `website-pages.md` plus fetch scripts). Per the lane brief,
> this file is therefore a stub: **nothing below the "What can be stated
> today" section is gathered content, and no forks-programme facts have been
> invented to fill the gap.** When the survey lands, this file is rewritten
> from it; the Space must not be built from this stub.

## Exactly what this file needs from `survey/oc-programme.md`

The survey must supply, with repo/issue citations the Space build can trust
without re-reading the forks' repos:

1. **The forks — identity and rationale.** For each fork —
   `developing-today-forks/opencode` and `developing-today-forks/openchamber`
   (the two fork repos named in the book skeleton and the Kaneo project
   registry): upstream project and upstream repo, what the fork is, why the
   estate carries it instead of using upstream, hosting/deploy shape, and
   how upstream changes are tracked or merged. (Book chapter fed:
   `opencode-openchamber/the-forks.md`.)
2. **Our changes, shipped and in flight.** The estate's own changes to each
   fork: merged/shipped changes (with PR or release references) separated
   from in-flight changes (with issue references and state). Any change the
   PM stack depends on must be named — e.g. the OpenCode server on the laptop
   (`127.0.0.1:13420`) that pm-dispatch and the claimant drive, the
   `experimental.model_routes` config surface the section-risk push class
   reads, OpenChamber scheduled tasks that scheduled-history reconciles —
   each marked as upstream behaviour or our change, per the survey's
   evidence. (Book chapter fed: `opencode-openchamber/our-changes.md`.)
3. **Goals and intentions.** The programme's stated goals for the forks:
   what they are for, what "done" or "upstreamed" would look like, and any
   standing decisions (e.g. merge authority, never-force-push, CI-on-main)
   as they apply to the fork repos specifically. (Book chapter fed:
   `opencode-openchamber/goals-and-intentions.md`.)
4. **OpenCode prioritised issue map.** The prioritised list of OpenCode
   (fork + upstream, distinguished) issues the programme is tracking: issue
   references with titles, priority/rank as recorded in the source system,
   and state — in an order the Space can render as a prioritised list
   without re-sorting on invented criteria. (Book chapter fed:
   `opencode-openchamber/opencode-issue-map.md`.)
5. **OpenChamber prioritised issue map.** The same, for OpenChamber.
   (Book chapter fed: `opencode-openchamber/openchamber-issue-map.md`.)
6. **Dependency map edges.** The forks programme's dependency edges as
   pairs (from → to, with the dependency kind), covering at minimum:
   fork → upstream, PM-stack components → fork behaviours they depend on,
   and issue → issue blocks/blocked-by edges the maps rely on. Edges must be
   sourced (issue relation, code reference, or config reference per edge) —
   the pack rule for `priorities-and-maps.md` is no invented edges, and it
   applies here too. (Book chapter fed:
   `opencode-openchamber/dependency-map.md`.)
7. **Dated state.** The survey's own census date and ground-truth commit(s)
   for each fork repo, in the same form as the other surveys (laptop clone +
   HEAD), so every figure in the rewritten file carries its date.

## What can be stated today (from sources already in this pack — not a substitute for the survey)

- The book gives the forks programme its own **liftable section** with its
  own SUMMARY part and framing (`skeleton.md`): `index.md` (OpenCode and
  OpenChamber: our forks programme), `the-forks.md`, `our-changes.md`,
  `goals-and-intentions.md`, `opencode-issue-map.md`,
  `openchamber-issue-map.md`, `dependency-map.md`. The Space section should
  mirror that shape.
- The two forks are two of the four core repos the PM stack tracks
  (`developing-today/code`, `developing-today-forks/opencode`,
  `developing-today-forks/openchamber`, `dezren39/darwin-nix` — `overview.md`),
  each with a Kaneo project and a ready-for-agent registration
  (`systems.md` A1/A4).
- PM-stack touchpoints with the forks, from the gathered surveys only:
  pm-dispatch creates OpenCode sessions on the laptop server
  `127.0.0.1:13420` (`systems.md` A8); the claimant launches/binds/prompts
  OpenCode sessions and supervises stuck ones (A19); scheduled-history
  reconciles OpenChamber scheduled-task runs (A24); the push section-risk
  class derives its sections from the deployed OpenCode config's
  `experimental.model_routes` (`push-flow.md` §4); the LLM route tiers
  (scout/workhorse/specialist/summit/speed/background) are OpenCode route
  heads in the deployed `opencode.jsonc` (`llm-plans.md` §2, read
  2026-11-23). **Which of these behaviours are upstream and which are our
  fork's changes is exactly what the missing survey must establish — it is
  not stated here.**

## Rewrite checklist (for whoever lands the survey)

- [ ] Replace this stub wholesale from `survey/oc-programme.md`; keep the
      pack header form (programme, coordinator, sources, census date).
- [ ] Forks summary first, then shipped/in-flight changes, then goals.
- [ ] Prioritised issue-map highlights for both forks, in the survey's
      recorded order — no re-ranking.
- [ ] Dependency edges as pairs, each with its source.
- [ ] Update `priorities-and-maps.md` Part 3 (forks dependency map row) and
      the `overview.md` reading order if the survey changes either.
