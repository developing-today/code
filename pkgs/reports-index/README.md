# reports-index

Searchable reports index for the PM-stack `/reports` surface — C6 slice of
#9574, issue #15020.

The index is a **derived view** over the existing report records, not a
parallel store: the same entries the report pages render from are handed to
`ReportsIndex`, which adds:

- **Indexed fields:** report ID, title, kind, project/domain, period,
  create/update time, status, tags, freshness (live pages), and canonical
  links to source issues/incidents/pages/periods.
- **Search** over title, summary, tags, linked IDs and body text, with
  filters for kind, project, dates, status and tags, and deterministic
  sort (updated/created/title, report-ID tiebreak) + pagination.
- **Event refresh:** create/update events apply idempotently (event ID +
  record version) inside a 15-minute freshness budget. A failed refresh
  keeps the last-good index and surfaces `stale`/`unavailable`.
- **C7/C8 handoff:** every entry exposes stable metadata links —
  `canonical` (`/reports/<id>`), `timeline` (`/reports/<id>#timeline` for
  C7), and an `atlas_ingest` metadata block (for C8) — links from the one
  index, never a second index database, and no duplicated timeline
  composition or embedding logic.
- **Phone-first rendering** at 320px (`render_index_html`); empty/error
  states preserve the active search/filters and explain index freshness.

Stdlib-only Python. Tests:

```
python3 -m unittest discover -s pkgs/reports-index/tests -v
```
