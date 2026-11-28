# 0007 — Elicitation state binds no user identity (refuse-identity)

```
number:   0007
status:   accepted
date:     2026-11-17
issues:   #7843 · touches #1957 #257 #255
code:     pkgs/mcpx at the merge commit of this record's PR
```

> mcpx has no authenticated user, and elicitation state today is
> bound to the request while "the session id is whatever the caller
> sends" (conformance gaps #257/#255). #7843 — split from #1957 by
> task 13636, which required the decision before any implementation
> — asked how mcpx establishes user identity for elicitation and
> sampling flows, or refuses to. This record refuses: there is no
> user identity in mcpx, elicitation state binds to the request and
> to the daemon's own principal (the OS owner of the daemon), and
> no client-supplied value is ever an identity input.

---

## Context

Verified at origin/main on 2026-11-17, before this record's PR:

- `docs/spec/conformance-matrix.md` marks four rows **gap** on this
  one missing decision: `elicitation-server-state-bound-to-user`
  (2025-11-25, MUST) and `elicitation-state-not-session-id-alone`
  (2025-11-25, MUST NOT) and
  `elicitation-server-no-trust-client-user-id` (2026-07-28,
  MUST NOT) as #257 — "mcpx has no authenticated user: state is
  bound to the request, the session id is whatever the caller
  sends" — and `elicitation-server-follow-best-practices`
  (2026-07-28, SHOULD) as #255, noting what is already sound:
  requestState signing, and owner-only question storage
  (0700/0600, #1676; row `elicitation-state-storage-protected` is
  tested and shipped).
- `internal/elicit/elicit.go` gives every elicitation a `Session`
  string, and `internal/elicit/store.go` stores it
  (`session TEXT`, indexed) exactly as the caller supplied it.
  There is no user field anywhere in the schema, and no
  user-id-shaped identifier is read in `internal/elicit` or
  `internal/daemon` (grep-verified for this record).
- The shipped design is broker-only on purpose
  (`docs/elicitation.md` §5.2; decided 2026-11-03, #1935/ELI-21):
  the daemon brokers questions between the calling client and
  whoever answers through the daemon's own surfaces. The daemon is
  a local, single-principal program: the only principal that
  exists is the OS user who owns the daemon, its socket, and its
  store directory.

## Options

**A. Authenticated identity.** Grow an identity layer — accounts,
tokens, or an IdP binding — so state can bind to a user.
Rejected: it invents a principal the rest of mcpx does not have.
Every other subsystem (tasks, artifacts, exec) authorizes by
daemon access, not by user; an elicitation-only identity would be
a second, weaker authorization model carried for one feature, and
the spec rows' threat (a server trusting a client-asserted user
id) is not answered by bolting on assertions — it is answered by
having nothing to assert to.

**B. Refuse identity (taken).** Keep exactly the principals that
exist — the request and the daemon owner — and say so as a
decision, so the absence of a user is a designed property with
rules, not an unexamined gap.

## Decision

1. **There is no user identity in mcpx.** Elicitation and sampling
   state binds to the elicitation record itself (its id, trace,
   and parent task) and to the daemon principal: the OS owner of
   the daemon process, its socket, and its owner-only store. That
   binding is mcpx's conformance position for
   `elicitation-server-state-bound-to-user`: state is bound, to
   the only principal the program has.
2. **Client-supplied identifiers are correlation labels, never
   identity.** The `Session` string and any user-id-shaped value a
   client or an MCP server supplies may be stored, logged, and
   searched, but no code may branch authorization, visibility, or
   answer-acceptance on them. This is the conformance position for
   `elicitation-server-no-trust-client-user-id` (MUST NOT — mcpx
   does not trust them, by rule, not by accident) and for
   `elicitation-state-not-session-id-alone` (state is keyed by
   elicitation id over signed requestState in an owner-only store;
   the session label alone keys nothing).
3. **Answering authority is daemon access.** Whoever can drive
   the daemon as its OS owner may see and answer its questions —
   the shipped broker model. Nothing about a question's content or
   routing grants or denies an answer.
4. **Future identity enters at the transport, or not at all.** If
   mcpx ever runs multi-user or remote, identity must be
   established by the transport — peer credentials on the daemon
   socket, or an authenticated bearer on the daemon's HTTP surface
   (the surface the merged bearer work already gates) — never from
   MCP message fields, and only under a new record superseding
   this one.

For `elicitation-server-follow-best-practices` (#255, SHOULD):
the practices that apply to a no-identity design are adopted —
signed requestState and owner-only storage (both shipped) and no
trust in client identity values (this record). The remaining
practice, binding state to an authenticated user, is declined for
the reason in option A.

## Consequences

- The four matrix rows above are re-dispositioned from **gap** to
  this decision in the same PR that lands this record; #257 and
  #255 close as gaps (the decision is the disposition).
- One implementation issue follows (filed from #7843): pin the
  rules in code and tests — document `Session` as correlation-only
  at its definition and schema sites, audit the elicitation and
  daemon answer paths for any authorization use of client-supplied
  identifiers, and add conformance tests for the two MUST NOT rows
  (a client-supplied user/session value changes nothing about who
  may answer or what state binds where).
- `docs/elicitation.md` gains a pointer to this record where it
  describes the broker model, in the same implementation issue.
- No implementation code changes under #7843 itself: the decision
  describes what the code already does, plus the guard work above.
