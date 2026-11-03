# Initial review and research order

[Main reference](main.md) | [Transcript](transcript.txt)

Hard prerequisites use `requires`; other edges express sequencing cost, conflicts, shared files or optional enabling work.

<!-- workflow:view waves -->

Wave 1 can start now: everything it requires is done. Later waves assume the earlier ones finish. Concepts that share files never share a wave. Leases and open branches are live data, so run `ready` before dispatching.

- **Done:** none
- **Running:** none
- **Wave 1:** [C001](main.md#c001), [C002](main.md#c002), [C003](main.md#c003), [C005](main.md#c005), [C009](main.md#c009), [C011](main.md#c011), [C012](main.md#c012)
- **Wave 2:** [C004](main.md#c004), [C006](main.md#c006), [C007](main.md#c007), [C008](main.md#c008), [C010](main.md#c010)

<!-- /workflow:view waves -->

## Dependencies

- C004 requires C001: Roles v2 authorization design depends on the world manifest delta (roles/invites, apply rerun semantics).
- C006 requires C002: First providers depend on the provider registry seam; implementation waits for the registry design.
- C006 recommended-after C005: Durable audit should precede external-effect providers; research may start early, implementation must honor audit-first.
- C007 requires C003: SDKs and the WIT/component-model decision consume the versioned ABI and its conformance suite.
- C008 requires C003: Program upgrade/migration validation depends on the versioned ABI and conformance contract.
- C010 requires C002: Portals are new registry capabilities; the registry seam must exist first.
- C010 requires C005: Bilateral portal grants require the durable audit contract on both worlds.

## Review status

<!-- workflow:view run-status -->

S0 is `validated` when `render` runs, because render only runs on a package that passes `check`. Other stages show the `tasks` entries that record `concept`, `stage` and `status`. `pushed`, `pr-open` and `merged` come from git and GitHub through `status --live`, so they never appear here.

- **C001** [F12.1 declarative world manifest, apply, diff and dry-run delta — D1-019](main.md#c001): S0 `validated` · S1–S8 `not-started`
- **C002** [F12.4 capability provider registry with manifested components — D1-022](main.md#c002): S0 `validated` · S1–S8 `not-started`
- **C003** [F12.7 versioned guest ABI spec, plaza\_abi\_version export and conformance suite — D1-025](main.md#c003): S0 `validated` · S1–S8 `not-started`
- **C004** [F12.3 roles v2 builder and moderator scopes with journaled audit — D1-021](main.md#c004): S0 `validated` · S1 `blocked` (requires C001) · S2–S8 `not-started`
- **C005** [F12.6 durable structured audit log with OpenTelemetry export — D1-024](main.md#c005): S0 `validated` · S1–S8 `not-started`
- **C006** [F12.5 first new providers kv blob http.fetch notify — D1-023](main.md#c006): S0 `validated` · S1 `blocked` (requires C002) · S2–S8 `not-started`
- **C007** [F12.8 Rust and AssemblyScript-or-Zig guest SDKs with WIT/component-model decision — D1-026](main.md#c007): S0 `validated` · S1 `blocked` (requires C003) · S2–S8 `not-started`
- **C008** [F12.9 program upgrade with state migration and rollback — D1-027](main.md#c008): S0 `validated` · S1 `blocked` (requires C003) · S2–S8 `not-started`
- **C009** [F12.10 fair cross-world scheduling per-account quotas and slow-client back-pressure — D1-028](main.md#c009): S0 `validated` · S1–S8 `not-started`
- **C010** [F12.14 portals bilateral cross-world messaging by mutual grant — D1-032](main.md#c010): S0 `validated` · S1 `blocked` (requires C002, C005) · S2–S8 `not-started`
- **C011** [F12.17 module signature trust list and compiler-version provenance — D1-035](main.md#c011): S0 `validated` · S1–S8 `not-started`
- **C012** [Intake envelope phase-0 process directives ownership and tracker policy — G bucket](main.md#c012): S0 `validated` · S1–S8 `not-started`

<!-- /workflow:view run-status -->
