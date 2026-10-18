# World Operator CLI Controls for Capability Ceilings and Resource Profiles

See the [Phase 9 plan](../../.opencode/plans/phase-9-portable-platform.md), the [resource profile record](../2026-10-16T10-30-00Z_feature_durable_world_resource_profiles/2026-10-16T10-30-00Z_feature_durable_world_resource_profiles.md), and the [capability ceilings record](../2026-10-16T11-00-00Z_feature_world_capability_ceilings/2026-10-16T11-00-00Z_feature_world_capability_ceilings.md).

## Intent

Expose client-side operator CLI commands under `id world` for managing world capability ceilings and execution profiles over Iroh:

1. `id world caps <NODE> [--grant <G>] [--revoke <R>] [--ceiling <C1,C2,...>]`: configure capability ceilings alongside grants.
2. `id world profile <NODE> [--admin-token <T>] [--wasm-fuel <N>] [--wasm-memory <BYTES>] [--wasm-message <BYTES>] [--native-cpu <SECS>] [--native-memory <BYTES>] [--native-deadline <MS>]`: inspect or update the persisted resource execution profile.

## Decisions

- **Caps ceiling flag:** Added `--ceiling` to `id world caps`. Setting a ceiling requires `--admin-token`. When combined with `--grant` or `--revoke`, the ceiling is applied and then grants/revocations are updated, returning the authoritative `CapsReport`.
- **Profile command:** Added `id world profile` subcommand. When any resource bound is passed, `--admin-token` is required to persist the new profile, returning an `ExecutionProfileUpdate` notice with `applies_on_next_program_install_or_process_restart: true`. When no limit flag is passed, queries and prints the current persisted profile (authorized by `--admin-token` or guest `--capability`).
- **Wire protocol:** Added `ClientFrame::ExecutionProfileInfo` to query execution profile state without mutating it. Handled in both pre-join (authorized by admin token or guest capability) and post-join session phases.
- **CLI memory efficiency:** Boxed `Command::World(Box<WorldCommand>)` in `Cli` to reduce stack allocation pressure in clap parsers.

## Acceptance

- `id world caps NODE --ceiling time.now,chat.say --admin-token ADMIN` sets the ceiling over Iroh and prints the updated report.
- `id world profile NODE --admin-token ADMIN --wasm-fuel 500000` updates the profile and reports deferred activation.
- `id world profile NODE --capability CAP` reads the current profile.
- Unauthenticated queries are rejected with access denied.
- 106 CLI parser tests and 245 world tests pass.
