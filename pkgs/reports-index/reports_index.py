#!/usr/bin/env python3
"""Searchable reports index for the PM-stack /reports surface (#15020, C6 of #9574).

The index is a *derived view* over the existing report records — it does not
create a parallel report store. Callers hand it the current report entries
(the same records the /reports pages render from); the index adds search,
filters, deterministic pagination/sort, freshness tracking, and the stable
canonical metadata links that C7 (timeline) and C8 (Atlas ingestion) consume.

Scope (issue #15020):
- Index report ID, title, kind, project/domain, period or affected time,
  create/update time, status, tags, freshness for live pages, and canonical
  links to source issues/incidents/pages/periods.
- Search title, summary, tags, linked IDs and indexed report text.
  Deterministic pagination/sort; filters for kind, project, dates, status
  and tags.
- Refresh from report create/update events, idempotently, inside a
  15-minute freshness budget; on refresh failure retain the last-good index
  and surface stale/unavailable.
- Stable metadata links / canonical URLs for C7 timeline and C8 Atlas
  ingestion; no duplicated composition or embedding logic.
- Phone-first rendering at 320px; empty/error states preserve the active
  filters and explain index freshness.

Stdlib only. Run tests:
    python3 -m unittest discover -s pkgs/reports-index/tests -v
"""

from __future__ import annotations

import html
import json
import re
from dataclasses import dataclass, field, asdict
from datetime import datetime, timezone

FRESHNESS_BUDGET_SECONDS = 15 * 60  # 15-minute freshness budget (#15020)

REPORT_KINDS = ("incident", "quota", "period", "rollup", "design", "status")
REPORT_STATUSES = ("draft", "live", "final", "archived")


def _parse_ts(value):
    """Parse an ISO-8601 timestamp to an aware datetime (UTC assumed)."""
    if isinstance(value, datetime):
        dt = value
    else:
        text = str(value).strip()
        if text.endswith("Z"):
            text = text[:-1] + "+00:00"
        dt = datetime.fromisoformat(text)
    if dt.tzinfo is None:
        dt = dt.replace(tzinfo=timezone.utc)
    return dt


def _iso(dt):
    return dt.astimezone(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


@dataclass
class ReportEntry:
    """One report record, as held by the existing /reports surface."""

    id: str
    title: str
    kind: str
    project: str
    created_at: str
    updated_at: str
    status: str = "live"
    period_start: str = ""
    period_end: str = ""
    tags: list = field(default_factory=list)
    summary: str = ""
    body_text: str = ""
    # Canonical links to source records: {"issues": [...], "incidents": [...],
    # "pages": [...], "periods": [...]} — values are IDs or URLs as recorded
    # by the source surface; the index never rewrites them.
    links: dict = field(default_factory=dict)
    live: bool = False  # live pages carry a freshness expectation
    version: int = 1

    def __post_init__(self):
        if self.kind not in REPORT_KINDS:
            raise ValueError(f"unknown report kind: {self.kind!r}")
        if self.status not in REPORT_STATUSES:
            raise ValueError(f"unknown report status: {self.status!r}")
        _parse_ts(self.created_at)
        _parse_ts(self.updated_at)
        if self.period_start:
            _parse_ts(self.period_start)
        if self.period_end:
            _parse_ts(self.period_end)

    @property
    def canonical_url(self):
        return f"/reports/{self.id}"

    def linked_ids(self):
        ids = []
        for values in (self.links or {}).values():
            ids.extend(str(v) for v in values)
        return ids

    def metadata_links(self):
        """Stable metadata links for C7 timeline / C8 Atlas ingestion.

        These are links *from* the one index — C7 composes its timeline and
        C8 embeds from the canonical report URL; neither needs (or gets) a
        second index database.
        """
        return {
            "canonical": self.canonical_url,
            "timeline": f"{self.canonical_url}#timeline",
            "atlas_ingest": {
                "id": self.id,
                "url": self.canonical_url,
                "kind": self.kind,
                "project": self.project,
                "updated_at": self.updated_at,
            },
        }

    def freshness(self, now, last_refresh_at):
        """Freshness label for live pages; static pages are always 'static'."""
        if not self.live:
            return "static"
        if last_refresh_at is None:
            return "unavailable"
        age = (_parse_ts(now) - _parse_ts(last_refresh_at)).total_seconds()
        return "fresh" if age <= FRESHNESS_BUDGET_SECONDS else "stale"

    def searchable_text(self):
        parts = [
            self.id,
            self.title,
            self.summary,
            self.body_text,
            self.project,
            self.kind,
            self.status,
        ]
        parts.extend(self.tags)
        parts.extend(self.linked_ids())
        return "\n".join(str(p) for p in parts if p)


@dataclass
class SearchResult:
    items: list
    total: int
    page: int
    page_size: int
    pages: int
    state: str  # "fresh" | "stale" | "unavailable"
    filters: dict


class ReportsIndex:
    """Derived, searchable view over report entries (no parallel store)."""

    def __init__(self, entries=()):
        self._entries = {}
        self._seen_event_ids = set()
        self.last_refresh_at = None
        self.state = "unavailable"  # until the first successful refresh
        self.last_error = None
        for entry in entries:
            self._entries[entry.id] = entry
        if entries:
            self.state = "fresh"

    # -- refresh ---------------------------------------------------------

    def refresh(self, source, now):
        """Rebuild from `source()` (a callable returning ReportEntry list).

        On failure the last-good index is retained and the state surfaces as
        stale (had data) or unavailable (never had data).
        """
        try:
            entries = source()
        except Exception as exc:  # keep last-good; surface, don't hide
            self.last_error = str(exc)
            self.state = "stale" if self._entries else "unavailable"
            return False
        self._entries = {e.id: e for e in entries}
        self.last_refresh_at = _iso(_parse_ts(now))
        self.last_error = None
        self.state = "fresh"
        return True

    def apply_event(self, event, now):
        """Apply one create/update event idempotently.

        Events: {"event_id", "type": "created"|"updated", "entry": {...},
        "version": int}. Replays of an event_id, and events whose version is
        not newer than the indexed record, are no-ops.
        """
        event_id = event.get("event_id")
        if not event_id or event_id in self._seen_event_ids:
            return False
        entry = ReportEntry(**event["entry"])
        existing = self._entries.get(entry.id)
        if existing is not None and entry.version <= existing.version:
            self._seen_event_ids.add(event_id)
            return False
        self._entries[entry.id] = entry
        self._seen_event_ids.add(event_id)
        self.last_refresh_at = _iso(_parse_ts(now))
        self.last_error = None
        self.state = "fresh"
        return True

    def freshness_state(self, now):
        if self.state == "unavailable" or self.last_refresh_at is None:
            return self.state
        age = (_parse_ts(now) - _parse_ts(self.last_refresh_at)).total_seconds()
        if age > FRESHNESS_BUDGET_SECONDS:
            return "stale"
        return self.state

    # -- search ----------------------------------------------------------

    def search(
        self,
        query="",
        kind=None,
        project=None,
        status=None,
        tag=None,
        date_from=None,
        date_to=None,
        page=1,
        page_size=20,
        sort="updated",
        now=None,
    ):
        if page < 1:
            raise ValueError("page must be >= 1")
        if page_size < 1:
            raise ValueError("page_size must be >= 1")
        tokens = [t.lower() for t in re.split(r"\s+", query or "") if t]
        matched = []
        for entry in self._entries.values():
            if kind and entry.kind != kind:
                continue
            if project and entry.project != project:
                continue
            if status and entry.status != status:
                continue
            if tag and tag not in entry.tags:
                continue
            if date_from or date_to:
                when = _parse_ts(entry.period_start or entry.created_at)
                if date_from and when < _parse_ts(date_from):
                    continue
                if date_to and when > _parse_ts(date_to):
                    continue
            if tokens:
                haystack = entry.searchable_text().lower()
                if not all(t in haystack for t in tokens):
                    continue
            matched.append(entry)
        # Deterministic sort: primary key, then report ID ascending.
        if sort == "updated":
            matched.sort(key=lambda e: (-_parse_ts(e.updated_at).timestamp(), e.id))
        elif sort == "created":
            matched.sort(key=lambda e: (-_parse_ts(e.created_at).timestamp(), e.id))
        elif sort == "title":
            matched.sort(key=lambda e: (e.title.lower(), e.id))
        else:
            raise ValueError(f"unknown sort: {sort!r}")
        total = len(matched)
        pages = (total + page_size - 1) // page_size if total else 0
        start = (page - 1) * page_size
        items = matched[start : start + page_size]
        state = self.freshness_state(now) if now is not None else self.state
        return SearchResult(
            items=items,
            total=total,
            page=page,
            page_size=page_size,
            pages=pages,
            state=state,
            filters={
                "query": query or "",
                "kind": kind,
                "project": project,
                "status": status,
                "tag": tag,
                "date_from": date_from,
                "date_to": date_to,
                "sort": sort,
            },
        )

    def get(self, report_id):
        return self._entries.get(report_id)

    def __len__(self):
        return len(self._entries)


# -- rendering -------------------------------------------------------------

_PAGE_CSS = """
body{font-family:system-ui,sans-serif;font-size:18px;line-height:1.45;
margin:0 auto;max-width:60rem;padding:1rem}
main{max-width:100%}
input,select,button{font-size:1rem;max-width:100%}
form{display:flex;flex-wrap:wrap;gap:.5rem}
.card{border:1px solid #8884;border-radius:.5rem;padding:.75rem;margin:.5rem 0}
.meta{font-size:.85em;opacity:.8}
.stale{border-left:.35rem solid #d97706;padding-left:.5rem}
.unavailable{border-left:.35rem solid #dc2626;padding-left:.5rem}
a{overflow-wrap:anywhere}
"""


def render_index_html(result, now, last_refresh_at=None):
    """Phone-first (320px) index page; empty/error states keep filters."""
    f = result.filters
    esc = html.escape
    state_note = {
        "fresh": "Index is fresh (refreshed within the 15-minute budget).",
        "stale": "Index may be stale: the last refresh is outside the "
        "15-minute budget or failed; showing last-good results.",
        "unavailable": "Index unavailable: no successful refresh yet; "
        "showing last-good results if any.",
    }[result.state]
    rows = []
    for e in result.items:
        links = e.metadata_links()
        freshness = e.freshness(now, last_refresh_at)
        src_links = " · ".join(
            f'<a href="{esc(str(v))}">{esc(str(v))}</a>'
            for values in (e.links or {}).values()
            for v in values
        )
        rows.append(
            f'<article class="card"><h2><a href="{esc(links["canonical"])}">'
            f"{esc(e.title)}</a></h2><p>{esc(e.summary)}</p>"
            f'<p class="meta">{esc(e.kind)} · {esc(e.project)} · '
            f"{esc(e.status)} · updated {esc(e.updated_at)} · "
            f"freshness: {esc(freshness)}</p>"
            f'<p class="meta">tags: {esc(", ".join(e.tags))}</p>'
            f'<p class="meta">source: {src_links}</p>'
            f'<p class="meta"><a href="{esc(links["timeline"])}">timeline</a>'
            f"</p></article>"
        )
    if not rows:
        rows.append(
            '<p class="card">No reports match. Your search and '
            "filters are kept above — clear or widen them to see "
            "more.</p>"
        )
    filter_inputs = "".join(
        f'<label>{esc(k)} <input name="{esc(k)}" value="{esc(str(v or ""))}"></label>'
        for k, v in (
            ("q", f["query"]),
            ("kind", f["kind"]),
            ("project", f["project"]),
            ("status", f["status"]),
            ("tag", f["tag"]),
            ("date_from", f["date_from"]),
            ("date_to", f["date_to"]),
        )
    )
    return (
        '<!DOCTYPE html><html lang="en"><head><meta charset="utf-8">'
        '<meta name="viewport" content="width=device-width, initial-scale=1">'
        f"<title>Reports index</title><style>{_PAGE_CSS}</style></head>"
        f"<body><main><h1>Reports</h1>"
        f'<p class="{esc(result.state)}">{esc(state_note)}</p>'
        f'<form method="get" action="/reports">{filter_inputs}'
        f'<button type="submit">Search</button></form>'
        f'<p class="meta">{result.total} report(s) · page {result.page} '
        f"of {result.pages}</p>" + "".join(rows) + "</main></body></html>"
    )


def entry_to_dict(entry):
    return asdict(entry)


def index_snapshot_json(index, now):
    """Machine-readable snapshot C8 can link to (metadata only, no embeds)."""
    payload = {
        "state": index.freshness_state(now),
        "last_refresh_at": index.last_refresh_at,
        "reports": [
            {
                "id": e.id,
                "title": e.title,
                **{"links": e.metadata_links()},
                "tags": e.tags,
                "status": e.status,
                "kind": e.kind,
                "project": e.project,
                "updated_at": e.updated_at,
            }
            for e in sorted(index._entries.values(), key=lambda x: x.id)
        ],
    }
    return json.dumps(payload, indent=2, sort_keys=True)
