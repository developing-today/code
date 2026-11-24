# Foundations: the shared substrate

Under the named systems sits a shared substrate that no single system's book
owns outright. This chapter names its parts once, at project level; each
part's detail lives in the book named with it.

## The work queue

One fleet queue dispatches all agent work in the project: tasks are filed,
claimed by exactly one worker under a lease, and completed with evidence.
The full queue cutover completed 2026-10-26; the repo-queue to fleet queue
migration completed 2026-11-04. Queue durability hardened in W47: a durable
SQLite coordination replica (PR #12692) and a Turso-to-D1 backup with a
restore drill (PRs #12685 and #12686). The queue's correctness programme is
merged-list item 1, and its detail is the Fleet book's: [The queue: tasks,
lanes and leases](https://developing-today.github.io/code/fleet/concepts/queue.html).

## The secrets broker

Secrets are resolved by name from a resident broker — a cache of the vault
that answers in milliseconds and falls back to the Bitwarden command-line
tool only when it must — so callers never hold a vault credential of their
own. The broker stack self-heals on init: the cold test of 2026-11-22 killed
all five legs (Tailscale daemon, relay, session broker, secrets broker, and
the GitHub shim) and the init sequence rebuilt every leg, verified. The known
defect class is recorded as merged-list item 28's work: a partial refresh
must not silently evict entries (#5504).

## The fleet CA and the identity provider

Agent identity is certificate-based. On 2026-11-23 the IdP hierarchy built
that day became the interim primary CA for all other work, under the fleet
CA programme (#9155, in execution on Drew's order): the fleet CA,
credentials and secrets design, version 0.3, whose expert-ruling task is
parked as part of item 28. The ID book's identity and directory chapters are
the other half of this ground: accounts, groups and keysets are directory
functions there, and world access is a capability issued against that
identity.

## Transports, and the multi-transport direction

Today's reach between seats rests on the laptop-agent daemon, the Cloudflare
tunnel, the Tailscale network and SSH as fallback — one daemon, several
doors, with automatic fallback between them (#10796). The transport question
for the whole P2P, ID and fleet stack is merged-list item 3 (#13752, #13779).

Drew recorded the long-term direction on 2026-11-23: **multi-transport
reach**. The P2P/ID/fleet stack should be reachable over many alternate
transports and proxied systems, Veilid-style — relays, Tor, email, MQTT,
MCP, IRC, chat bots, Gopher and Gemini, physical media, and more. Each
adapter declares its level — read, bidirectional, write, or
sync-equivalent — and asymmetric send and receive paths are first-class: a
message sent over one transport and received over another is a design
requirement, not an edge case. The programme for this goal is being filed
against [#13752](https://github.com/developing-today/code/issues/13752); in
the merged list it rides item 3's line and adds no item of its own.

## The Turso and Cloudflare estate

The project's hosted data lives in a censused estate: 50 Turso databases and
22 Cloudflare Workers, live-verified 2026-11-23 in the PM census. Where each
store lives — Turso, Cloudflare D1, R2, or local — is the storage-placement
question of item 11, audited under #13844 (audit merged via PR #13845). The
estate's backup story today is one system's: the laptop's daily Atlas backup
into the shared R2 bucket. The coherent cross-system set is item 7.

## The docs and books system itself

The substrate includes its own record. The books are mdBook sites built by
Nix derivations from sources beside the code they document, staged into one
combined GitHub Pages deploy because a Pages deployment replaces the whole
site. The dated records under `doc/` — 619 entries at census, 452 of them
dated November 2026 — are the project's design memory, and the stewardship
cycle (hourly since 2026-10-24) is its oversight log. Keeping this book
current is [its own chapter](../development/keeping-this-book-current.md);
deploying it is [Deploying this book](../operations/deploying-this-book.md).
