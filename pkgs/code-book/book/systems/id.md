# ID

ID is the project's identity and graph platform. It started as a peer-to-peer
file sharing tool and is now a local-first, federated, capability-based
application platform built on the Iroh networking stack. Its parts:

- **Content-addressed objects.** A file is stored under the hash of its
  bytes; a human-readable name points at the hash, so the same content is the
  same object on every node that holds it.
- **Worlds with Roc guests.** A world is a multiplayer program written in
  Roc, compiled to an import-free Wasm module and run by a host under the
  Plaza ABI. Guests have no direct access to the filesystem, the network, the
  clock or randomness; every effect goes through the capability protocol.
- **Capabilities.** World access is granted as capabilities: an invite minted
  by a host's admin, a per-world ceiling, and provider grants enforced on the
  host side. Reaching a node grants no authority on that node.
- **The directory.** A home-server directory holds accounts, groups, invites
  and keysets, recorded in an append-only journal.
- **The hosted edition.** A per-user edition of the same system as a
  Cloudflare Worker with one Durable Object per user. In the source tree at
  census it is a local-only scaffold, not a deployed service, and the ID book
  says so wherever it describes it.

The code is a Rust workspace of 17 packages under `pkgs/id/`, surveyed for
the ID book at commit `1ccb4a19457` on 2026-11-23.

## Current state (2026-11-23)

- The platform core is built and in daily use locally; the build-out is the
  merged list's Tier 4 (items 16–23). The store abstraction's conformance
  fixtures landed (#10191, PR #13194); the remaining foundational step at
  census was #9082's adopt/defer/reject decision on the IndexedDB
  profile. Correction (2026-11-23, re-verified against live state):
  #9082 has since closed as completed on 2026-11-23, adopting the
  IndexedDB request-batch profile.
- Hosted worlds are in flight with the chain blocked at its head: #10693 to
  #10697 were all verified open with a blocked status at census, with the
  queue tasks recorded as blocked until 2026-11-26. The export/import halves
  of #3810 landed 2026-11-23 (PRs #13201 and #13212).
- Scale measurement is under way with real numbers: the stress-world
  generator produced 10× scale (11,001 worlds, 200,000 events in 35.7
  seconds, #13083, closed) and frame cost measured flat to 1,000,000 events
  (#13085, closed).
- The ID graph programme was unified under a new epic during the census
  window: [#13880](https://github.com/developing-today/code/issues/13880),
  ID apps on one graph (item 15).
- The Bevy incorporation design was adopted 2026-11-23 (#5781), making Bevy
  the native reference shell speaking the same shell contract as the Android
  and web shells (item 22).

## Where its detail lives

- Book: the [ID book](https://developing-today.github.io/code/id/) — 67
  chapters, sources at `pkgs/id/book/`.
- This programme's slice: the ID book's [high-level
  roadmap](https://developing-today.github.io/code/id/design/high-level-roadmap.html)
  carries ID's share of the merged list (items 15–23 in full).
- Programme tracker: [#13240](https://github.com/developing-today/code/issues/13240);
  the ID-100 list of 2026-11-23 enqueued the programme's next hundred items.
- Merged-list items: 15–23 are ID-primary; ID also carries cross-links to
  items 3, 9, 11, 12, 13, 24, 25, 29 and 30.
