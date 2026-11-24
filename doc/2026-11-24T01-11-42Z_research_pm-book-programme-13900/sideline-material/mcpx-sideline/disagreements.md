# MCPX-SIDELINE — disagreements found between the Worker's docs/design and its code

Numbered for grouped filing by the coordinator (also carried in PR #13906's body).

1. **`worker/README.md` had no compendium manifest entry.** The mcpx book's
   `TestEveryDocumentHasAChapter` (internal/settings/compendium_test.go) requires
   every markdown file under pkgs/mcpx to be placed in
   docs/compendium.manifest.json; the Worker README was absent (156 entries,
   none for it), so the gate failed on the tree as found.
   **Fixed in PR #13906**: registered as `not-compendium` (book chapter =
   docs/worker.md), mirroring daemon/README.md's entry. Gate passes after the fix.

2. **The laptop half of the door chain is asserted, not configured.**
   worker/README.md and src/index.ts describe a bridge proxy on 127.0.0.1:8932
   and the mcpx-link.security.cab tunnel ingress, but grep of home/,
   pkgs/mcpx/modules/ and pkgs/mcpx/daemon/ finds no unit, service or ingress
   defining either. The only bridge in home/common/default.nix is the Desktop
   Commander bridge on 127.0.0.1:8931 (commander-link.security.cab, #13404).
   The chapter states this openly in its door-chain section.

3. **"Transparent proxy" is only true for `?ns=*`.** #13657's architecture line
   calls the Worker an "OAuth/bearer gate + transparent proxy", and src/index.ts
   calls the /mcp proxy "a transparent streamable-HTTP proxy". In the
   conservative and selected profiles the Worker in fact gates request bodies
   and buffers, transforms and re-emits responses (src/filter.ts); only the
   full profile passes the response through untouched. The chapter reconciles
   this by describing transparency as a property of the full profile.

4. **The #8299 citation for CapabilityCeiling/PolicySession is indirect.**
   #13659's scope and the Worker README both cite #8299 as the
   CapabilityCeiling/PolicySession work the daemon auth should align with;
   #8299's own title is "id(phase-14) v2 step 1: role configuration — roles as
   first-class world config (#6301)", an id-programme item. The alignment may
   be real through the id policy work, but the citation's target does not say
   so on its face. The chapter follows the source's citation and this list
   flags it.

5. **Minor: deployment config is split across two files with no cross-reference
   in either.** wrangler.toml defines the Worker (name, vars, KV) but has no
   route; the route `mcpx.security.cab` lives only in config.json (the cf
   tool's config). A reader of wrangler.toml alone cannot tell the Worker is
   bound to the custom domain (only the header comment says so). Noted in the
   chapter's source table; not filed as a defect beyond this list.

6. **Minor nuance, consistent but surprising: `mcpx_types` is listed yet
   uncallable under the default profile.** tools/list under `conservative`
   exposes mcpx_types (it is in DISCOVERY_TOOLS), but gatewayCallAllowed
   refuses every mcpx_types call in that mode (it requires the selected mode
   and an in-set namespace list). The Worker README does state the selected-set
   condition; the chapter spells out the consequence explicitly. Recorded here
   so the grouped filing can decide whether listing-without-calling is intended.
