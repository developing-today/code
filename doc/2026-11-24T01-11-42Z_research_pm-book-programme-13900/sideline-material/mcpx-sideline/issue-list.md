# MCPX-SIDELINE — issue list for the mcpx door Worker

All in developing-today/code. States/titles read via `fleet gh api` on 2026-11-23.
Bodies of #13657 and #13659 were read in full; the rest are title/state level
plus the references inside those bodies, the Worker README, and search results.

## The door's own items (the chapter's prioritised order)

| Order | Issue | State | Title / role |
|---|---|---|---|
| 1 | #13770 | open | "main CI: mcpx worker publish fails — mcpx Worker does not exist on the Cloudflare account (versions upload before first deploy)" — blocks the publish workflow; a first full deployment must precede the versions flow |
| 2 | #13754 | open | "main CI failure: developing-today/code — mcpx worker publish workflow (run 45264370211)" — the recorded main failure of the publish step |
| 3 | #13657 | open | "mcpx: public endpoint at mcpx.security.cab with OAuth/Google-SSO front door and machine bearer" — the door's spec (Drew, 2026-11-23) and acceptance list: discovery documents; RFC 7591 registration; /authorize behind Access, owner-only, Google identity; /token issue/rotate + bad-grant rejection; /mcp with machine bearer returns the real tool list from outside; / ≡ /mcp; unauthenticated /mcp = 401 with resource_metadata pointer; bridge door service-token-only; publish/promote proven end to end. Drew's one step at the end: add https://mcpx.security.cab/mcp as a connector in Claude/ChatGPT and approve on the Access page. Architecture in the spec: client → Worker (OAuth/bearer + transparent proxy) → bridge door hostname (Access, service-token-only) → tunnel → laptop mcpx surface. Cloudflare: Developing.today account, zone security.cab; scoped deploy token named cloudflare.api-token-mcpx-worker; GitHub environment mcpx-production; cf versions publish/promote via GitHub Actions |
| 4 | #13659 | open | "mcpx: per-user/per-key authentication in the daemon, aligned with CapabilityCeiling/PolicySession" — follow-on, does NOT block #13657. Daemon currently has no auth at all (verified first-hand 2026-11-23 per the issue: systemd daemon on 127.0.0.1 only; health/status/MCP initialize answer with no credentials). Scope: per-user/per-key auth on /mcp and /v1/*; identity resolves to PolicySession/CapabilityCeiling (cites #8299 and neighbours); loopback migration path for existing unauthenticated local callers; unix-socket permission model stays or is folded in explicitly. Acceptance includes documenting the #13657 door's selector model as mapping onto these ceilings |
| — | #13697 | closed | "mcpx: public front-door Worker with OAuth and selector enforcement" — the implementation of #13657 (found via GitHub search: repo:developing-today/code mcpx worker in:title) |
| — | #13404 | open | "Self-hosted Desktop Commander MCP endpoint: Worker front door, Access gate, laptop bridge" — the proven pattern the mcpx door mirrors (its Worker build is PR #13488 per #13657); its bridge is the one actually defined in home config (127.0.0.1:8931, commander-link.security.cab) |

## Cited / adjacent

| Issue | State | Why it appears |
|---|---|---|
| #8299 | open | Cited by #13659 and the Worker README for the CapabilityCeiling/PolicySession alignment — but its actual title is "id(phase-14) v2 step 1: role configuration — roles as first-class world config (#6301)". See disagreement 4 |
| #12417 | (not opened by this lane) | Referenced inside #13657's spec: the Google OAuth client for Cloudflare Access exists per this issue |
| #13900 | open | The programme issue: "Project Management book — fifth house book, coverage audit, sidelines, Space". Its Sidelines section assigns this lane: "mcpx book: add coverage of the mcpx door Worker (mcpx.security.cab — gate/proxy, ?ns= enforcement) with maps/lists for its related items" |
| #13906 | merged PR | This lane's PR: "docs(mcpx): book chapter for the door Worker at mcpx.security.cab". Commit a0c78e1259a; merge commit c4e2e7294b3e30cfc3d0f8af1139c748d02e42ba |
| #13709 | closed PR | "build(deps): bump undici, cf and wrangler in /pkgs/mcpx/worker" (from the same title search) |
| #13264 | open | The earlier mcpx full-investigation programme; its docs/program/issue-map.md is the book's existing prioritised-list style the chapter mirrors, and its row for #13404 marks the Commander door "adjacent" |
| #1888 | open | mcpx book tracker (the book this chapter joins) |

## Search note

GitHub search used: `search/issues?q=repo:developing-today/code+mcpx+worker+in:title`
returned #13770, #13754, #13697, #13709. No other Worker-specific issues found
by that query; the door family is otherwise exactly the brief's list
(#13657, #13659, #13404 family).
