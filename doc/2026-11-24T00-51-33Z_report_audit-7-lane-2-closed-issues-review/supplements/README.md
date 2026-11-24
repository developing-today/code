# Supplements — AUDIT-7 Lane 2 raw material

All raw data gathered and files produced by Lane 2, committed per Drew's steer (2026-11-23 18:46 CST): nothing lives only in a summary.

- list-*.json (4 files) — full census payloads from fleet gh api search windows (body, labels, state_reason, comment count for all 8,161 closed issues).
- ledger.csv — per-issue ledger with verdicts (8,161 rows). ledger-flags.json — triage flag table. flagged-full.json — 493-issue high-risk union, full bodies.
- dump2.log — census run log (per-window counts). comments.log — partial detached comment-dump log.
- dump.py (first attempt, superseded), dump2.py, analyze.py, extract.py, ledger.py, comments.py — gathering/analysis scripts exactly as run.
- packets/ — the four pilot-queue feature_slice packets as enqueued (queue tasks 21595, 21596, 21598, 21600).
- tracking-body.md — tracking issue #13878 original body.
