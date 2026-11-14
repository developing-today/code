# mcpx dedup — one canonical mcpx source in developing-today/code

Living document · Status: shipped (2026-10-19) · Register entry F2.2 · Tracking epic: developing-today/code#8805

## Purpose

- Remove the duplicate mcpx copy from `dezren39/nix`.
- Make `developing-today/code` the one canonical mcpx source.
- Do not lose anything the nix copy implemented that code lacked.

Origin (2026-10-19):

- Drew asked whether there were "two full source apps in 2 places?" —
  confirmed yes.
- Recorded directive: "remove the duplicate mcpx copy from dezren39/nix;
  first determine everything the nix copy implements that
  developing-today/code lacks and migrate each thing; file a GitHub issue
  BEFORE each thing; merge PRs whenever ready."

## What landed (features, and how each works)

- Audit / migration run:
  - A coordinator ran the audit and migration the same day (2026-10-19).
  - Per-item tracking was by design: a GitHub issue was filed BEFORE each
    migrated item, rather than one tracking issue for the whole dedup.
- Vendored copy removed:
  - Same-day follow-up record: "mcpx dedup — vendored copy in dezren39/nix
    removed (Drew's directive, ~11:51 CDT)".
- Nix packaging rewired:
  - Nix now consumes mcpx via a pinned flake input on the code repo,
    instead of the vendored copy in nix.

## Rough architecture

- Canonical source: `developing-today/code` (mcpx lives there).
- Consumer: `dezren39/nix` packaging, via a pinned flake input pointing
  at the code repo.
- No second full mcpx source tree in nix.

Related consolidation (register F2.4, 2026-11-03, separate entry):

- CheckPhase adaptations were later moved into code's `pkgs/mcpx/package.nix`.
- Daemon modules originate with mcpx in code, are re-exported by opencode,
  passed through by openchamber, and consumed by nix via the single
  openchamber input. That layering is F2.4, not part of this entry's scope.

## Status

- Design status in register: built-into-running-system.
- Implementation status in register: shipped (2026-10-19).
- Tracking: NEVER-ISSUED as one tracking issue — searched "mcpx dedup
  single source". By the directive's own design, per-item issues were the
  tracking mechanism.
- Triage: (b) DONE-WITHOUT-ISSUE.

## Next steps — immediate / near-term / long-term

- Immediate: none recorded.
- Near-term: none recorded beyond the open follow-up below.
- Long-term: not recorded.

## Open follow-ups

- Drew had not yet said whether he also wants a submodule at the old mcpx
  path for local editing, on top of the flake-input rewiring.
  - That question is recorded as open in the register entry.
  - No decision is recorded.

## Future ideas not yet specced

- Not recorded in the register entry, apart from the submodule question
  above, which is an open follow-up rather than a specced idea.

## Links

- Register entry: F2.2 in `~/workspace/design-audit-60d/REGISTER.md`
- Related register entries: F2 (mcpx platform program, parent),
  F2.1 (mcpx + opencode platform program), F2.4 (mcpx consolidation —
  modules originate with mcpx in code)
- Tracking epic: developing-today/code#8805
- Design artifact: none named in the entry; tracking was per-item issues
