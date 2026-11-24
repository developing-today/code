# fleet bridge: a subcommand family for the class/name notes surface

Follow-up to #13895 (Bridge: class/name notes, public wake signal, and Archer hook), filed from the fleet book programme #13900. Drew's direction, recorded on #13895 (2026-11-23 ~18:29 CST): the bridge client eventually becomes a `fleet` subcommand family (get / send / receive / watch) that replaces or reimplements the standalone `bridge` helper built under #13895.

## What already exists (not the gap)

`fleet bridge` already ships as the mailbox client for the existing bridge service: `send`, `agents`, `log` (owner token) and `hello`, `inbox`, `post`, `ack`, `health` (fleet token), documented in the fleet book (`pkgs/fleet/book/reference/bridge.md`). Those verbs drive the mailbox system (`agent:` / `group:` / `all` targets). Do not rebuild them.

## The gap

The #13895 notes surface — `class/name` identities with wildcards (`muse/*`, `*/archer`, `*/*`), the public per-path wake signal, and the per-identity authenticated inbox — has no `fleet` client. Its recorded client is a small standalone `bridge` helper (`send` / `inbox` / `ack`). This issue is the fleet-side family that replaces that helper and ends ad-hoc HTTP access to the notes endpoints.

## Scope: four verbs over the notes endpoints

- `fleet bridge send` (notes form) — send a note to a `class/name` identity or a wildcard address, as an authenticated sender.
- `fleet bridge get` — read the public signal for a `class/name` path (the content-free UUID), for scripts and hooks that poll it.
- `fleet bridge inbox` (notes form; the receive verb in Drew's wording) — fetch the calling identity's notes inbox over its authenticated endpoint.
- `fleet bridge watch` — poll the public signal for a path and surface or fetch on change: the Archer hook's loop (`muse/archer`) as a supported verb instead of a hand-rolled script.

Exact verb spelling is this issue's to settle against the existing mailbox verbs (the names `send` and `inbox` already exist with mailbox semantics — a notes mode, a sub-family, or distinct verbs are all acceptable if the help text keeps the two surfaces unambiguous).

## Rules

- The family wraps the bridge service's authenticated endpoints; it does not add a second store or bypass the service.
- Tokens resolve **by vault name only** through fleet-secrets, exactly as the existing bridge verbs do: values never in argv, files, output or logs. Per-identity tokens follow the #13895 model (server-side represented only by hashes).
- The public signal stays public and content-free: `get` / `watch` need no token, and nothing in the family may weaken the signal's privacy properties (same shape for registered and unregistered paths; jittered decoy rotation means a change never proves delivery).
- Exit codes and JSON passthrough match the existing bridge domain's conventions (0 ok, 2 usage, 3 transport or credentials, 4 the bridge refused).

## Links

- Design and build: #13895
- Programme: #13900 (the fleet book's bridge audit records this surface as in flight in `pkgs/fleet/book/concepts/bridge.md`)
