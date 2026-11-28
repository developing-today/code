# Goals and intentions

This chapter states what the forks programme is trying to make true. Every goal carries its issue link. Closed landmark items are marked (closed); open items appear again in the issue maps, where they are ordered as work. A goal in this chapter is an intention with a record, not a shipped change: shipped change is [Our changes, shipped and in flight](our-changes.md), and the two chapters are kept apart on purpose.

The programme's code-repo footprint is larger than any single list in this part. For context: the survey's broad search `repo:developing-today/code opencode` totals 4,147 items (all states, issues+PRs, body mentions included). The programme set this part uses for the code repo is the survey's defined set — open code issues whose **title** names opencode, openchamber or the (opencode) claimant: **91 open issues**. The narrower definition is the survey's, stated so the set is countable; the 480 open code issues that mention opencode anywhere are in the deposit, outside the defined set.

## Attach, don't SSH

Three closed items settle how OpenCode work reaches the laptop:

- [Attach, don't SSH: research which is more efficient for OpenCode work — SSH-on-laptop vs attaching to the laptop's running server](https://github.com/developing-today/code/issues/1344) (closed).
- [Attach-first OpenCode client helper — forward lifecycle, service discovery, stdin-safe run wrapper (the #1344 follow-through)](https://github.com/developing-today/code/issues/1357) (closed).
- [Evaluate a tailnet-bound `opencode serve` option on the laptop for attach clients (the #1344/#1357 family)](https://github.com/developing-today/code/issues/1363) (closed).

The family is a dependency chain as well as a goal; the edges are drawn in [The forks programme dependency map](dependency-map.md).

## A claimant owns dispatch

The OpenCode claimant/executor (code#3026, slice 1) is to discover unbound opencode-lane queue tasks, create/verify/bind sessions on the fork server, deliver prompts, and recover stuck sessions — under the session contract of code#4912 (phases #4757/#4870/#4758, closing audit #4916) and the seam contract of code#7770 (phases #7325/#7375/#7706, #7367/#7760, #7356/#7366/#7732; audit #7772). Those numbers are cited as they appear in the claimant README itself (the programme's own record); #1344/#1357/#1363 above were additionally verified individually against the API in this survey. The shipped claimant is recorded in [Our changes, shipped and in flight](our-changes.md); the PM-side account is [The claimant: keeping sessions honest](../concepts/claimant.md).

## OpenCode 2 migration of the whole fleet

- The migration epic: [opencode#374 "Migrate the fleet to OpenCode 2"](https://github.com/developing-today-forks/opencode/issues/374) (priority/high, open).
- Its core-migration tracking home: [opencode#2295](https://github.com/developing-today-forks/opencode/issues/2295).
- Plugin follow-through: [opencode#2290](https://github.com/developing-today-forks/opencode/issues/2290) — port the v1 plugins (mcpx bridge, mcpx-session, subtask2) and rebase the fork.
- The embedded-OpenCode upgrade on the OpenChamber side: [openchamber#384](https://github.com/developing-today-forks/openchamber/issues/384).
- The deployed-plugin gap list at `doc/2026-11-23T03-45-00Z_opencode-v2-deployed-plugin-gap-list/` in the code repo: three v1 constructs with no v2 equivalent, each with a chosen workaround.

The record disagrees with itself on this goal's state: opencode#2295 is titled "…core migration (**shipped** — tracking home)" while the parent migration opencode#374, the plugin follow-through opencode#2290 and the embedded upgrade openchamber#384 all remain open (D54). This book records the migration as open work with a shipped core, because the issues that carry the remaining work are open.

## Routes, pools and quota as fork-native machinery

- Model-route follow-ups: [opencode#2681](https://github.com/developing-today-forks/opencode/issues/2681).
- The config-projection gap: [opencode#2782](https://github.com/developing-today-forks/opencode/issues/2782) ("needed by code#13675 Part 2") — the fork fix blocks that code work; the edge is in [The forks programme dependency map](dependency-map.md).
- OpenChamber's pool-settings follow-ups: [openchamber#2498](https://github.com/developing-today-forks/openchamber/issues/2498).
- Retry-recovery controls: [openchamber#2126](https://github.com/developing-today-forks/openchamber/issues/2126) — titled "(shipped — tracking home)"; it sits in the open map as a tracking home, under the same record tension as D54.
- The provider doctrine: [opencode#1717](https://github.com/developing-today-forks/opencode/issues/1717) ("*-direct providers EMULATE the first-party wire").

The deployed pools and routes this machinery serves are the LLM plans part's subject: [Pools, routes and the switchboard](../llm-plans/pools-and-routes.md). This chapter records the fork-side intention only.

## Server façades and consolidation

- Generic OpenAI façades on the fork server: [opencode#1146](https://github.com/developing-today-forks/opencode/issues/1146) (priority/high; "proxy doors retire").
- OpenChamber server-control consolidation + quota credential: [opencode#2491](https://github.com/developing-today-forks/opencode/issues/2491) / [opencode#2490](https://github.com/developing-today-forks/opencode/issues/2490), both citing the code#5060 ruling recorded in [The forks: what they are and why we carry them](the-forks.md).

## Console work

The PM console (code repo `pm-stack/console`) consumes the forks' server/session surface. On the fork itself the console scope is one of the largest PR themes in the shipped record, and OpenChamber is the operator console for sessions: its epic is [openchamber#2153 "Epic: OpenCode platform"](https://github.com/developing-today-forks/openchamber/issues/2153), and the session dispatch + babysitting epic sits in the code title set (`code-title-openchamber.json` in the deposit; the code issue is code#1234 in the survey's §3.3 set).

## Observability without instrumentation

Usage export by pull over the fork's own DB (code#2072 slice 1) is shipped and recorded in [Our changes, shipped and in flight](our-changes.md). Slices 2–7 are stated follow-ups. They are intentions, not shipped work, and this chapter is where they belong until they merge.

Sources. Survey file `~/workspace/pm-book/survey/oc-programme.md` §2 and §3.3 (scope rule and the 91-issue set), §6 item 5 (cited in this part as D54); claimant README as deposited in `~/workspace/pm-book/sideline-material/oc-survey/pm-stack-opencode-claimant-README.md`; code title-search sets in the deposit (`code-title-opencode.json`, `code-title-openchamber.json`, `code-title-claimant.json`).
