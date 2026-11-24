## Notes and the public wake signal — the next bridge surface (in flight)

Everything above this section is the bridge as built and shipped: the
mailbox system the `fleet bridge` verbs drive today. A second, smaller
surface is being built on the same service — recorded as the design of
record in developing-today/code#13895 (filed 2026-11-23) and **in
flight, not landed**, as of 2026-11-23: no part of it is in
`pkgs/message-bridge` on `main` yet. This section documents the design
as recorded, so the book is ready for it; it will be revised against
the code when the build lands.

**Addressing: `class/name` identities.** A note's recipient is an
identity written `class/name` (for example `muse/archer`). An
authenticated sender can address one identity, or use a wildcard in
either segment — `muse/*` (every identity in a class), `*/archer`
(that name in every class), or `*/*` (every identity). Notes are
stored server-side in the bridge's Durable Object and delivered to
every registered identity matching the address. This is a different
grammar from the mailbox system's `agent:` / `group:` / `all` targets
above: classes are part of the identity itself, not a separate group
registry an agent joins by saying hello.

**The public signal.** `GET /<class>/<name>` is a public,
content-free wake signal. It returns the same bare UUID shape for
registered and unregistered paths. A registered path's signal changes
when a note is delivered to it — and every path, registered or not,
also rotates on a deterministic, per-path jittered schedule. The
privacy properties are the point of the design:

- the signal carries no note content;
- its shape is identical for registered and unregistered paths, so it
  is not an identity-existence proof;
- because every path rotates on its own jittered schedule, a change
  does not prove a delivery happened — decoy rotation is built in, and
  an unregistered path still rotates its synthetic UUID across
  windows.

**Inbox and acknowledgement.** The signal only says "look"; the notes
themselves sit behind the identity's own token. Authenticated inbox
and acknowledgement endpoints belong to that identity token: token
values are kept in the vault by name and are represented server-side
only by hashes — never committed, printed or logged. The recorded
flow is: poll the public signal, and when it changes, fetch the
inbox with the identity token, act on the notes, then acknowledge
them (the acceptance proof ends with the inbox empty after
acknowledgement).

**The client side, as designed.** A small standalone `bridge` helper
provides `send`, `inbox` and `ack` against these endpoints without
hand-written HTTP calls, and the first consumer is a Muse hook for
Archer: it polls only the public `muse/archer` signal, compares one
UUID to its last-seen value, and wakes only when it changes; on
waking, Archer fetches and acknowledges the inbox and surfaces or
acts on the notes in the main chat. The recorded end state for the
client surface is a `fleet` subcommand family — the follow-up issue
linked from #13895's record and from the fleet book programme
(#13900) — at which point this section's verbs move into the
reference chapter alongside the mailbox verbs above.
