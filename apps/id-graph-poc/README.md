# id-graph-poc: PoC 1 (Rummager-like organising)

Slices 1–2 of #14037. Slice 1 is built on the D3 record
(`doc/2026-11-24_design_id-graph-layer-typed-links-verbs/D3-graph-layer.md`).
Slice 2 is a fixture-backed live-transclusion host renderer using the D2
record (`doc/2026-11-25T01-40-00Z_design_id-apps_d2_transclusion/`).

Run the done checks from the repo root:

    python3 -I apps/id-graph-poc/check.py
    python3 -I apps/id-graph-poc/check_transclusion.py

## What it shows

- A curator (`ed25519:dddd4444curator`) who authored none of the D3 fixture
  objects collects them into `coll-meetup-2026-11-22` with `collect` links.
- Discovery over verb links: backlinks (`what links here`), neighbourhood
  (`what is this near`), and recent changes.

## Model gap: `collect` is provisional

D3 v1 has no collection kind and no `collect` verb. This PoC admits `collect`
provisionally so that organising has a link to carry it. It is not an
admission under D3 §5: it has no create/traverse rule, rendering hint, or
second implementation yet. Per the build-on-system principle the gap is
recorded for D3 (#13885) to decide, not patched app-locally. PoC 2 and the
live app are out of scope here.
