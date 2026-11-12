# F12.4 Provider Registry — Slice 1

## Request and intent

Issue [#1061](https://github.com/developing-today/code/issues/1061) asks for capability providers to become manifest-carrying components, with the ledger, compile gate, and CLI reading one registry instead of a hand-maintained capability catalog. This slice introduces the registry for the existing 17 capabilities only. It must preserve current behavior and does not add providers or implement dynamic loading.

The slice was gated on issue [#838](https://github.com/developing-today/code/issues/838), which prevents a newly registered provider from widening existing world ceilings. The gate cleared when PR [#4656](https://github.com/developing-today/code/pull/4656) merged on 2026-11-03 (merge commit `f28aa79e3f7b70102ef112b1788ec3e68c7241aa`). Registry defaults must preserve that per-provider ceiling behavior.

## Initial design

- Represent provider metadata with the fields required by F12.4: name, version, argument schema, bounds, required scope, and default ceiling.
- Register the current 17 capabilities statically and derive the capability catalog from those entries, retaining the pre-change names and order.
- Route the existing catalog, ledger, compile-gate, and CLI reads through the registry without changing grants, ceilings, refusal codes, or other runtime behavior.
- Keep this slice limited to the registry and existing consumers. A provider added without editing `world.rs` and dynamic/plugin loading remain outside scope; the former belongs to the later F12.4 slice 2.
- Add equivalence coverage for the exact existing 17-name catalog and run the relevant `pkgs/id` tests.

---

## 2026-11-03T10-08-00Z Discovery: build on the #838 catalog entries

Merged PR #4656 already added `DefaultCeiling`, `CatalogEntry`, `CATALOG_ENTRIES`, and `default_ceiling_names()` to the capability catalog. The current 17 entries explicitly opt into the existing default ceiling. Slice 1 should extend this existing entry/registry seam with the remaining provider manifest metadata and derive the legacy catalog/readers from it, rather than adding a second registry type. The task packet's pre-#838 evidence that no entry/manifest type exists is consequently stale; its requested registry behavior and scope remain intact.

---

## 2026-11-03T10-09-46Z Decision: retain the default-ceiling distinction

PR #4656 changed `CapLedger`, world restoration/checkpoint handling, and the server/world CLI defaults to use `default_ceiling_names()`. Keep this narrower default-ceiling API distinct from the full catalog used for explicit ceilings. Provider-manifest defaults must continue to control whether each entry joins a default ceiling; registry derivation must not silently substitute the full catalog in any default path.

---

## 2026-11-03T10-21-18Z Decision: preserve the current provider boundaries

The registry distinguishes host requests, subscriptions, and Roc platform modules. Request manifests record recognized argument fields; subscription manifests record their spec; platform-module manifests have no host-request schema. The registry also records existing provider-specific bounds and effect scopes. These are descriptive metadata in slice 1: request validation and dispatch stay at their existing boundaries so this refactor does not change acceptance, refusal codes, or execution behavior. The parser's general `wants` limits remain global.

## Change log

- 2026-11-03: Initial implementation intent recorded before code changes; based on queue task 4880 and the F12.4 plan.
