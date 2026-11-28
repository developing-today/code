---
session: ses_2d1d
updated: 2026-03-27T07:25:14.287Z
---

## Summary

### Task
Quick exploration of the Datastar JavaScript client API (`@starfederation/datastar` v1.0.0-beta.11) to understand if/how it can be used programmatically from JS (not just via HTML `data-*` attributes).

### What Was Done
Thoroughly examined the Datastar package installed at `/home/user/code/pkgs/id/web/node_modules/@starfederation/datastar/` by reading:
- `package.json` (entry points/exports)
- All engine files (`engine.js`, `engine.d.ts`, `types.d.ts`, `signals.d.ts`, `index.d.ts`)
- All backend action plugins (`get.js`, `delete.js`, `patch.js`, `sse.js`)
- Backend shared module (`shared.js`, `shared.d.ts`)
- Plugin index (`plugins/index.d.ts`)
- Bundle file (`bundles/datastar.js`)

### Key Finding: NO Programmatic JS API Exists

**Datastar does NOT provide a `Datastar.get(url)` or similar programmatic API.** The entire public API is only 3 functions:
- `load(...plugins)` — register plugins
- `apply()` — walk DOM + start MutationObserver
- `setAlias(prefix)` — set data attribute prefix

The action plugins (GET, POST, PUT, PATCH, DELETE) all require a `RuntimeContext` as first arg, which is an **internal-only** object constructed by the engine when processing `data-*` attributes. The `signals` singleton and `actions` registry are **module-private** variables in `engine.js` — never exported.

### Workarounds Identified
1. **Inject `data-*` attributes on DOM elements** — Datastar's MutationObserver will detect and process them
2. **Use plain `fetch`/`EventSource` directly** — Datastar's SSE protocol is standard HTTP SSE
3. **Listen to SSE lifecycle events** via `document.addEventListener('datastar-sse', handler)` — custom events with `{type, elId, argsRaw}` detail

### No Files Were Modified
This was a read-only exploration — no changes to any project files.

### Remaining Work
None for this specific exploration task. The user now has the complete API surface understanding to decide how to integrate Datastar (or not) into the `id` web frontend. The project currently uses HTMX + ProseMirror + WebSockets, not Datastar.
