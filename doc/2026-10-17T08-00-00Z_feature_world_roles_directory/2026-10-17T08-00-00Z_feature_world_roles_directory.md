# Worlds as places, first slice: role-scoped invites and a machine-readable world list

See the [Phase 9 plan](../../.opencode/plans/phase-9-portable-platform.md) (Phase 10 sketch) and the [world CLI controls record](../2026-10-16T12-00-00Z_feature_world_cli_controls/2026-10-16T12-00-00Z_feature_world_cli_controls.md).

## Intent

Two concrete steps toward the Phase 10 direction:

1. A world host could only mint one kind of participant: a full guest. Give
   invites stable role presets so hosts can hand out read-only and play-only
   capabilities without a second secret or a manual scope conversation.
2. `id world list` printed tab-separated text, which no directory feed or web
   portal can consume. Add a JSON form.

## Decisions

- **Roles are presets over existing scopes.** `WorldScopes::from_role` maps:
  - `spectator` → `join` (read events, records, and screens; no chat or input)
  - `player` → `join` + `input`
  - `guest` → `join` + `chat` + `input` (the default, unchanged)
  - `delegator` → `guest` + `delegate` (may mint narrower capabilities)

  The names are stable; the mapping is protocol. No new scope bits, no new
  authority, and `id world caps` is unaffected. Roles that would need new
  authority (a builder who installs modules, a moderator who kicks) stay
  future work: those are admin-token powers today, and handing them to a
  capability needs its own design.

- **Additive wire change.** The `invite` frame gains an optional `role`
  string, defaulting to `guest` when absent, so older clients behave exactly
  as before. An unknown role is refused by name.
- **The service exposes `invite_scoped`.** `WorldService::invite` keeps its
  signature and delegates with `WorldScopes::GUEST`; the new method takes the
  scopes so the session layer maps roles without touching the admin check or
  the invite bounds.
- **`WorldClient::invite_with_role`.** `invite` delegates with `None`, so
  every existing caller and test is unchanged.
- **`id world list --json`.** Prints `{"default": <name>, "worlds": [{"name",
"open", "default"}]}`. The tab-separated form is unchanged when the flag is
  absent.

## Acceptance

- `role_presets_grant_and_withhold_the_expected_effects` checks each preset's
  scopes and effects, and that an unknown role is `None`.
- `invite_roles_grant_and_withhold_the_expected_scopes` drives the session
  protocol: an unknown role is refused by name; a spectator joins and reads
  but is refused for chat and input; a player is refused for chat and its
  input is accepted.
- `test_cli_parse_world_roles_and_list_json` covers `--role` (including a
  parser refusal for an unknown role) and `list --json`.
- Full library suite passes.

## Follow-ups

- Builder/moderator roles with their own authority model (module install,
  removal) and journaled grants.
- A world directory feed for other nodes: names, presence counts, and
  open/closed, sharing one serializer with `id world list --json`.
- Showing a participant's role in `players.list` or `id world stats` once a
  role is journaled rather than only carried by the capability.
