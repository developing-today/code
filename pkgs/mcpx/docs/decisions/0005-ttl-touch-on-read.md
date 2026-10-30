# 0005 — TTLs renew by touch-on-read, bounded by a maxAge ceiling

```
number:   0005
status:   accepted
date:     2026-10-29
issues:   #1884 · touches #309 #1887 #1893
code:     pkgs/mcpx at the merge commit of this record's PR
```

> mcpx has at least seven TTLs across five subsystems and, until now,
> none of the absolute family (elicit, tasks, artifacts, ask, proto
> state) could be renewed: a question expired from its creation no
> matter how actively it was being used, while the idle family
> (daemon leases, MCP sessions, daemon idle-exit) already renewed by
> use — with no rule for which family a new resource joins. #1884
> asked for one convention. This record is that convention.

---

## Context

Verified at origin/main 7d1b081c, before this record's PR:

- `internal/elicit/elicit.go` — an elicitation's `ExpiresAt` was set
  once at creation (`Created.Add(defaults.ElicitTTL)`, 120s) and never
  moved. 120s was therefore the ceiling on how long a human could
  take to answer a question, however close they were to finishing.
- `internal/defaults/defaults.go` — `elicit.ttl` existed only as a
  compiled-in default read from defaults.json; it was not a declared
  registry setting, so no config key, `MCPX_*` variable, flag, or
  `/v1/settings` could change it.
- `internal/daemon/server.go` — the idle family already renews by
  use: `trackActivity`/`shouldIdleExit` keep a daemon alive while it
  is being used and let it die when it is not.
- `internal/artifacts/artifacts.go` — artifacts expire absolutely at
  put (`Expires: now.Add(ttl)`).

## Options

### A — A `/renew` endpoint or per-resource renew operation

Every resource grows a renewal verb. Rejected in #1884: it puts the
burden on every caller forever, and a caller that forgets is exactly
the case renewal exists for.

### B — Touch-on-read, unbounded

Any read renews forever. Rejected: a resource somebody keeps
glancing at becomes immortal, and expiry stops meaning anything.

### C — Touch-on-read with an absolute ceiling (chosen)

Reading a resource renews its idle deadline; no renewal can push
the deadline past creation + maxAge. An explicit lifetime given at
creation is the one exception: creation is where a lifetime is
chosen, reads only renew within the convention.

## Decision

1. **Touch-on-read is the default.** An *item* read of a pending
   resource sets `lastTouched = now` and the effective deadline to
   `min(created + maxAge, lastTouched + ttl)`. *Collection* reads do
   not touch: listing what exists must not keep it alive.
2. **Touch never shortens and never revives.** A deadline set
   explicitly at creation stands; a resource whose deadline has
   passed is expired, not renewed by a late read.
3. **One vocabulary.** Every expiring representation carries
   `expiresAt` (RFC 3339), `lastTouched`, and `ttlRemainingMs`
   (milliseconds until `expiresAt` when the representation was
   produced, never negative).
4. **No `/renew` endpoint** or per-resource renew operation is added,
   anywhere, for any family.
5. **The knobs are declared settings.** `elicit.ttl` and
   `elicit.maxAge` are registry settings (config key, `MCPX_ELICIT_TTL`
   / `MCPX_ELICIT_MAX_AGE`, flags, `/v1/settings`), defaulting to
   120s and 30m.
6. **Internal reads do not touch.** Machinery that polls a resource
   to enforce its deadline (the broker's `Await`, `Respond`'s
   validation) reads without touching; a read that renewed the
   deadline it enforces would never let it fall.

First family implemented: **elicit** (this record's PR). The
remaining absolute families adopt the same convention in follow-ups
named on #1884: tasks, ask, artifacts (absolute by default, opt-in
`artifacts.idleTtl`). **proto state is the documented exception:**
its expiry is signed into the token, so it cannot be touched; it is
renewed by re-minting.

## Consequences

- #1884: implemented for the elicit family by this record's PR;
  remaining families split into linked follow-ups, named on the
  issue, none silently dropped.
- Forbidden: any `/renew` verb; any new expiring resource that does
  not state which side of this convention it is on.
- Deferred: tasks/ask/artifacts adoption (follow-ups on #1884);
  security/auth questions (the issue's tracker, #1887/#1893).

## Mapping

| term              | from                    | becomes                                        |
| ----------------- | ----------------------- | ---------------------------------------------- |
| renew endpoint    | #1884 option A          | forbidden — touch-on-read instead              |
| absolute expiry   | elicit/tasks/artifacts  | idle deadline + maxAge ceiling (per family PR) |
| `elicit.ttl`      | compiled-in default     | declared registry setting                      |
| stateTTL renewal  | proto state             | re-minting the token (documented exception)    |
