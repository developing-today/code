# First slice: versioned world role configuration

## Request and intent

Issue [#8299](https://github.com/developing-today/code/issues/8299) makes role definitions first-class, versioned world configuration. The accepted behavior includes a capability ceiling, quota class, delegation-lifetime bounds, review policy, owner-managed grants, editable starter definitions, audit events, and enforcement against the current role definition on each frame.

## Initial slice

Start at the existing declarative world boundary, `id-caps::world_manifest`. Add a versioned role-definition representation to `WorldManifest`, keeping the existing manifest parser strict about unknown fields. The first observable result is valid definitions round-tripping through TOML and malformed versions/definitions being rejected during parse/validation before an apply can partially mutate a live world.

This slice establishes the config contract only. It does not claim that manifest apply persists role state, that role grants are live/revocable, or that the policy point reads changed role ceilings. Those require subsequent slices connecting world state, audited grant/revoke events, and per-frame policy evaluation. The definition fields follow #6301 v2 §2.1: ceiling, quota class, delegation lifetime bounds, and review policy. This implementation must not widen authority beyond the existing policy point's ceiling checks.

## Test oracle

The existing typed TOML parser is the oracle: a valid versioned definition parses, serializes, and parses to the same typed value; unknown versions or invalid bounds fail explicitly rather than being silently ignored. The focused test command is `cargo test -p id-caps world_manifest` from `pkgs/id` (run through the approved Fleet build path when building on the laptop).

## Change log

- 2026-11-23: Initial slice design, before implementation.
