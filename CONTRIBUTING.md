# Contributing

This repository is worked on by people and by a fleet of agents, and both
follow the same flow. This page states that flow once and links to the
documents that own each rule; it does not restate them.

## How a change lands

1. **Work in a worktree, on a branch.** Every change starts from a fresh
   worktree on its own branch cut from `origin/main`. Changes land through
   a pull request, never by pushing to `main`. Rules and reasons:
   [`doc/agents/parallel-writers-and-subsessions.md`](doc/agents/parallel-writers-and-subsessions.md)
   and [`planning/workflow/collaboration.md`](planning/workflow/collaboration.md).
2. **Leave the primary clone alone.** Nobody works in the primary
   checkout: it stays on `main` and clean, and every session works in its
   own worktree ([the worktrees rule](doc/agents/parallel-writers-and-subsessions.md)).
3. **Verify CI-first.** Builds and tests run in CI before heavy local
   runs; the policy is [`.github/ci-policy.md`](.github/ci-policy.md)
   (issue #1413).
4. **Put documentation where it belongs.** Maintained control documents
   live in [`planning/`](planning/README.md) and change in place; dated,
   immutable records live in [`doc/`](doc/README.md). The placement rules
   are [`planning/DOCUMENTATION_ARCHITECTURE.md`](planning/DOCUMENTATION_ARCHITECTURE.md),
   and record naming is [`doc/DOCUMENTATION_PROTOCOL.md`](doc/DOCUMENTATION_PROTOCOL.md).
5. **Index new architecture and design records.** A PR that adds an
   architecture or design record also adds it to the
   [architecture index](planning/architecture/README.md) in the same PR.
   Architecture decision records go in
   [`planning/architecture/adr/`](planning/architecture/adr/README.md),
   started from [its template](planning/architecture/adr/TEMPLATE.md).

Agents also follow the always-loaded rulebook in [`AGENTS.md`](AGENTS.md).

## Licensing and sign-off

Contributions are dual-licensed `MIT OR Apache-2.0` and signed off under
the [Developer Certificate of Origin](DCO-1.1); the terms are in
[`README.md`](README.md#contributing-information) and
[`COPYRIGHT.md`](COPYRIGHT.md).
