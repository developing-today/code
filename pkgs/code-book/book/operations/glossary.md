# Glossary

Terms this book uses in their project sense. Each system's book carries its
own fuller glossary; these are the words a reader of the whole project
needs first.

- **Broker** — a resident fleet service that answers for a shared resource:
  the queue broker holds the work queue; the secrets broker holds the vault
  cache. Callers talk to the broker, not to the store behind it.
- **Capability** — in ID, a grant that bounds what a program or participant
  may do, issued against an identity and enforced on the host side. In the
  merged list, a *capability set* is one of the four item types.
- **Census** — a dated, sourced count or survey of a system's state. This
  book rests on three: Census A (retrospective), Census B (priorities) and
  Census C (the lab), all taken 2026-11-23/24.
- **Content-addressed store (CAS)** — a store that names objects by the hash
  of their bytes, so identical content is stored once and the same content
  is the same object everywhere. Atlas's provenance rebuild and ID's object
  store are both content-addressed.
- **Dated record** — one of the project's design and research documents
  under `doc/`, filed in a folder named with its UTC datetime, kind and
  name, and indexed from `doc/README.md`. 619 existed at census.
- **Flag hold** — a platform execution-risk gate on a specific item of work.
  In this book, flag holds are recorded per item as gates with reasons;
  they pace several merged-list items.
- **Golden set** — Atlas's fixed evaluation query set (311 queries, v3,
  plus 54 held out). A ranking change that hurts on the golden set does not
  ship.
- **High-level roadmap** — the chapter each per-system book carries for this
  programme: that system's share of the merged list, in the merged list's
  numbering.
- **Intake ladder** — the fleet rule that decides what an agent works on
  next, in order, so no agent stalls asking what to do.
- **Lane** — one running agent session working a queue item. The agent work
  loop's floor stood at a minimum of 16 running lanes on 2026-11-23.
- **Lease** — a time-limited claim on a queue task; it expires if the
  holder stops reporting in, so a dead worker's task returns to the queue.
- **Merged list** — the 30-item, 7-tier ranking in [The merged priority
  list](../priorities/the-merged-list.md). Item numbers are stable across
  all books.
- **Packet** — a unit of queue work: a self-contained task description an
  agent can claim and complete with evidence.
- **Provenance** — where a stored unit came from: its session, client,
  source reference, and commit or observation date. In Atlas, a result that
  cannot state its provenance is not served.
- **Seat** — one running agent instance in one place: a Muse session in a
  sandbox, an OpenCode session on the laptop, a chat assistant over HTTPS.
- **Seed item** — a merged-list item that entered from Drew's seed list on
  tracker #13905, as opposed to one derived from a programme census.
- **Stewardship cycle** — the hourly oversight cycle, running since
  2026-10-24, that reads the project's queues, blockers and stalls and
  records them in its cycle log.
- **Tier** — one of the merged list's seven dependency bands, from Tier 0
  (execution substrate) to Tier 6 (the record itself). See [How priorities
  work](../priorities/how-priorities-work.md).
