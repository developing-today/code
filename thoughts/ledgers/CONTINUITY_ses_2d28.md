---
session: ses_2d28
updated: 2026-03-27T14:13:24.194Z
---



## Summary of Migration Project: HTMX → Datastar + Custom CSS → DaisyUI/Tailwind

### Goal
Migrate `pkgs/id` web UI from HTMX → Datastar and custom CSS → DaisyUI/Tailwind. Keep WebSockets as-is. Phased approach with plan document approved before implementation.

### Completed Work

**Phase 1: Build Pipeline + Theme Setup** ✅
- TailwindCSS v4 + DaisyUI v5 build pipeline with Bun
- Custom DaisyUI themes: sneak(blue), arch(green), mech(orange)
- `input.css` with `@import "tailwindcss"` + `@plugin "daisyui"`

**Phase 2: CSS/Component Migration** ✅ (commit `92a33407`)
- Migrated `terminal.css`/`themes.css` → DaisyUI utility classes

**Phase 3: HTMX → Datastar Conversion** ✅ (commit `eeec370c`)
- **routes.rs**: `is_htmx_request()` → `is_partial_request()` using `X-Partial-Request` header; added `/api/peers` endpoint
- **templates.rs**: All 23 `hx-*` attrs → `data-nav`/`data-page-nav`/`data-auto-refresh`
- **main.ts**: Removed htmx.org; custom SPA nav layer (`navigateTo`, `fetchPartial`, `onMainSwapped`, `onPartialSwapped`, `initSearchDebounce`, `initPeersAutoRefresh`); click delegation + popstate handler
- **mod.rs**: Doc comments updated
- All 117 web tests pass

**Phase 4: Cleanup & Polish** 🔄 IN PROGRESS
- ✅ JSDoc comments: 6 htmx references updated
- ✅ Debug console.logs: ~15 verbose logs removed, kept error/warn/WS logs
- ✅ Build passes (`bun run build` succeeded)
- 🔄 `web/README.md`: Needs re-read then edit to update HTMX→Datastar text
- 🔄 Need to run `cargo test` final verification
- 🔄 Need to commit Phase 4

### Key Architecture Decisions
- Datastar imported (`@starfederation/datastar`) for future reactivity but SSE directives NOT used — custom SPA nav instead (avoids server-side SSE format changes)
- Server still returns HTML fragments, no SSE conversion
- `X-Partial-Request` header replaces `HX-Request`
- `data-auto-refresh="10"` attribute for peers polling

### Files Modified
- `src/web/routes.rs`, `src/web/templates.rs`, `src/web/mod.rs`
- `web/src/main.ts`, `web/package.json`, `web/bun.lock`
- `web/README.md` (pending)

### Git State
- Branch `main`, 2 commits ahead of origin
- Unstaged: `web/src/main.ts`, `web/README.md`
- New JS hash: `main.v4n6c16g.js`

### Next Steps
1. Re-read `web/README.md` and apply HTMX→Datastar text updates
2. Run `cargo test --features web -- web::` to verify all 117 tests pass
3. Commit Phase 4 cleanup
4. Migration complete

### User Preferences (persistent)
- Verify builds with `just` and `nix flake`
- Use Chrome MCP screenshots for visual verification
- Keep generated artifacts synced/up-to-date
- Use `alias` in justfiles per docs
- Don't add `id` as a nix path input
