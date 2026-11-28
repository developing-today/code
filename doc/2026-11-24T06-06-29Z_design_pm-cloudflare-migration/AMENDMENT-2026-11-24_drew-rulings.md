# Amendment — Drew's rulings, 2026-11-24

This note amends the design record in this folder (`2026-11-24T06-06-29Z_design_pm-cloudflare-migration.md`). The dated record itself is not rewritten; this file rides beside it. The same rulings are recorded on the tracking issue: https://github.com/developing-today/code/issues/14107#issuecomment-7123885604

## D1 — settled: one front door

The canonical name is pm.security.cab, the single door for the PM surfaces. Drew raised no objection to the design's recommendation (§12, D1), so it stands as decided.

## D2 — decided: automatic fallback

When a hosted surface fails, reads fall automatically to the next tier, and the fall is always visible in route state. This adopts the design's recommendation (§12, D2). Writes still never auto-fall.

## Amendment to the fallback section — the client-fallback layer

Drew adds a mechanism the design did not have: client-side fallback. The hosted page's own JavaScript detects failure when it goes to fetch dynamic details and falls back to the next tier's endpoint itself, walking the chain from the user's side — including toward the tailnet address.

This qualifies the design's statement that the last tier "is a bookmark, not a failover": server-side failover to a different DNS name remains impossible, exactly as the design states, but the client walking the chain mechanises the fall, so the last tier is no longer only a bookmark a human picks. The layer applies to reads and dynamic details. Writes are unchanged: they never auto-fall — submissions ride the edge intake/held queue and are applied exactly once by the answer write-back applier (developing-today/code#14043).

## Generalisation

Automatic fallback is the estate pattern, not a PM-only behaviour: fleet commands adopt it later too, and the Android app does the same fallback natively and directly, rather than through a browser page.

## Frontend direction for these surfaces

The reactive/dynamic layer is Datastar with server-sent events (SSE). The UI stack is Tailwind with DaisyUI, with all default DaisyUI themes available.

## Status after this amendment

Slice M1 (front door + deployment pipeline) is packetised as queue task 21686 on `repo-health-code-pilot` at priority 40; the shell it serves takes the client-fallback layer above as a design input. Slices M2–M9 remain unqueued.
