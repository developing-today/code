---
created: 2026-11-23
last-modified: 2026-11-24
revision: 1
---

# Decentralized-systems review — quantitative stats

Gathered 2026-11-23 for the decentralized-systems review space. Every figure comes
from the project's primary repo host on that date; anything that could not be
verified is marked **not verified** rather than estimated.

## Methods

- **GitHub** (`gh api`): commits = commit count on the default branch (Link-header
  last-page count); contributors = `/contributors?anon=true` count (includes
  anonymous contributors); releases = non-draft GitHub releases with
  `published_at` ≥ 2025-11-23 (last 12 months), sampling at most 300;
  "last activity" = `pushed_at` (last push to any branch).
- **Codeberg** (Forgejo API): commits = `X-Total-Count` on the commits endpoint;
  last commit = newest commit on the default branch. Codeberg exposes **no
  contributors endpoint**, so contributor counts there are *not verified*.
  Worm Blossom and Spritely release through crates.io / tags, not Codeberg
  releases, so release figures for those come from crates.io and tags.
- **GitLab** (Veilid): contributors = `X-Total` on the contributors endpoint;
  commits = direct paged count (the commits endpoint returns no total header);
  releases from the releases endpoint (31 total).
- **Dependents**: taken from the investigation notes
  (`~/workspace/p2panda-investigation/research/`, `~/workspace/iroh-investigation/`)
  where they establish them (cited per row); otherwise from crates.io
  reverse-dependency data (labelled "crates.io reverse deps").

**Data-quality note:** one batch of GitHub reads taken during a service restart
returned a stale snapshot (wrong stars/commit counts for seven repos). Every
figure from that batch was re-read fresh and cross-checked (repo objects,
release lists, crates.io) before use; the numbers below are the verified set.

**Comparability caveats:** monorepos with per-package release trains (atproto,
Endo, Keyhive, Holochain) show inflated "releases in 12 months" and a "latest
release" that is just whichever package tagged most recently. Subduction's
release count is a nightly train. Stars measure attention, not adoption —
Delta Chat core is the clearest case (see surprises).

## Summary table

| Project | Repo | Latest release | Date | Commits | Contributors | Stars |
|---|---|---|---|---|---|---|
| p2panda | p2panda/p2panda | v0.8.1 | 2026-11-13 | 1,591 | 19 | 603 |
| Willow (willow25) | worm-blossom/willow_rs (Codeberg) | 0.7.11 stable; 0.7.12-alpha.1 newest (crates.io) | 2026-11-18 | 1,565 | not verified | 40 |
| Bab (bab_rs) | worm-blossom/bab_rs (Codeberg) | 0.8.2 (crates.io) | 2026-11-17 | 194 | not verified | 9 |
| Keyhive | inkandswitch/keyhive | keyhive-wasm/0.8.3 (per-crate tag); keyhive_core crate 0.12.3 | 2026-11-23 | 328 | 8 | 280 |
| Subduction | inkandswitch/subduction | v0.22.0-nightly.2026-11-20 | 2026-11-20 | 280 | 9 | 139 |
| iroh | n0-computer/iroh | v1.5.0 | 2026-11-19 | 2,688 | 77 | 13,087 |
| iroh-gossip | n0-computer/iroh-gossip | v0.103.0 | 2026-11-16 | 346 | 21 | 154 |
| iroh-blobs | n0-computer/iroh-blobs | v0.104.1 | 2026-11-04 | 760 | 31 | 183 |
| Veilid | veilid/veilid (GitLab) | v0.5.8 | 2026-11-02 | 2,300 | 69 | 454 |
| IPFS (Kubo) | ipfs/kubo | v0.43.1 | 2026-09-15 | 16,023 | 469 | 17,194 |
| rust-libp2p | libp2p/rust-libp2p | v0.57.1 | 2026-10-15 | 4,045 | 369 | 5,652 |
| UCAN (rs-ucan) | ucan-wg/rs-ucan | ucan-v0.4.0 | 2023-06-28 | 102 | 13 | 94 |
| Biscuit | biscuit-auth/biscuit-rust | biscuit-quote-0.3.0 (GH tag); biscuit-auth crate 7.0.0-alpha.4 (crates.io) | 2025-07-16 / 2026-11-17 | 868 | 33 | 254 |
| Delta Chat core | deltachat/deltachat-core-rust | v2.69.0 | 2026-11-09 | 10,851 | 68 | 952 |
| Pubky / pkarr | pubky/pkarr | v9.0.0 | 2026-11-09 | 888 | 26 | 473 |
| Holochain | holochain/holochain | holochain-0.8.0-dev.16 | 2026-11-23 | 13,602 | 71 | 1,403 |
| AT Protocol | bluesky-social/atproto | @atproto/ozone@0.16.3 (per-package tag) | 2026-11-21 | 3,628 | 143 | 9,732 |
| Cap'n Proto (Rust) | capnproto/capnproto-rust | no GH releases; capnp crate 0.28.2 (crates.io) | 2026-11-22 | 3,842 | 106 | 2,522 |
| Cap'n Web | cloudflare/capnweb | capnweb@0.13.1 | 2026-11-20 | 231 | 36 | 4,046 |
| Endo (Agoric) | endojs/endo | @endo/pass-style@2.0.1 (per-package tag) | 2026-08-17 | 7,386 | 62 | 1,076 |
| Spritely Goblins (Guile) | spritely/goblins (Codeberg) | v0.18.0 (tag) | Apr 2026 (per ocap-family.md) | 1,980 | not verified | 93 |

## Per-project detail

### p2panda — `p2panda/p2panda` (GitHub)
| Field | Value |
|---|---|
| Commits (main) | 1,591 |
| Contributors | 19 |
| Stars / forks | 603 / 54 |
| Primary language | Rust |
| Licence | MIT OR Apache-2.0 (workspace `Cargo.toml`) |
| Latest release | v0.8.1, 2026-11-13 |
| Releases, last 12 mo | 9 (25 total) |
| Last push | 2026-11-23 |

### Willow / willow25 — `worm-blossom/willow_rs` (Codeberg)
| Field | Value |
|---|---|
| Commits (main) | 1,565 |
| Contributors | not verified (no Codeberg endpoint) |
| Stars / forks | 40 / 10 |
| Primary language | Rust |
| Licence | Apache-2.0 (per willow-meadowcap.md repo listing) |
| Latest release | willow25 crate: 0.7.11 stable; newest 0.7.12-alpha.1, 2026-11-18 (crates.io) |
| Releases, last 12 mo | not verified (no Codeberg releases; ships via crates.io) |
| Last commit | 2026-11-17 |
| Note | Repo created on Codeberg 2025-10-23 (migration); commit history predates it |

### Bab — `worm-blossom/bab_rs` (Codeberg)
| Field | Value |
|---|---|
| Commits (main) | 194 |
| Contributors | not verified |
| Stars / forks | 9 / 5 |
| Primary language | Rust |
| Licence | not verified |
| Latest release | bab_rs crate 0.8.2, 2026-11-17 (crates.io) |
| Releases, last 12 mo | not verified |
| Last commit | 2026-11-17 |

### Keyhive — `inkandswitch/keyhive` (GitHub)
| Field | Value |
|---|---|
| Commits (main) | 328 |
| Contributors | 8 |
| Stars / forks | 280 / 22 |
| Primary language | Rust |
| Licence | Apache-2.0 |
| Latest release | keyhive-wasm/0.8.3, 2026-11-23 (per-crate tags); keyhive_core crate 0.12.3, 2026-11-23 |
| Releases, last 12 mo | 25 (32 total) |
| Last push | 2026-11-23 |
| Note | Beelay, the predecessor sync design, has no standalone repo (404); it lives in Keyhive's history/docs — Subduction is its successor (keyhive-subduction.md) |

### Subduction — `inkandswitch/subduction` (GitHub)
| Field | Value |
|---|---|
| Commits (main) | 280 |
| Contributors | 9 |
| Stars / forks | 139 / 22 |
| Primary language | Rust |
| Licence | Apache-2.0 |
| Latest release | v0.22.0-nightly.2026-11-20 |
| Releases, last 12 mo | 110 (110 total — nightly train; repo created 2025-07-18) |
| Last push | 2026-11-23 |

### iroh — `n0-computer/iroh` (GitHub)
| Field | Value |
|---|---|
| Commits (main) | 2,688 |
| Contributors | 77 |
| Stars / forks | 13,087 / 768 |
| Primary language | Rust |
| Licence | Apache-2.0 (GitHub API detection) |
| Latest release | v1.5.0, 2026-11-19 |
| Releases, last 12 mo | 19 (76 total) |
| Last push | 2026-11-23 |

### iroh-gossip — `n0-computer/iroh-gossip` (GitHub)
| Field | Value |
|---|---|
| Commits (main) | 346 |
| Contributors | 21 |
| Stars / forks | 154 / 51 |
| Primary language | Rust |
| Licence | Apache-2.0 |
| Latest release | v0.103.0, 2026-11-16 |
| Releases, last 12 mo | 8 (16 total) |
| Last push | 2026-11-23 |

### iroh-blobs — `n0-computer/iroh-blobs` (GitHub)
| Field | Value |
|---|---|
| Commits (main) | 760 |
| Contributors | 31 |
| Stars / forks | 183 / 59 |
| Primary language | Rust |
| Licence | Apache-2.0 |
| Latest release | v0.104.1, 2026-11-04 |
| Releases, last 12 mo | 8 (16 total) |
| Last push | 2026-11-23 |

### Veilid — `veilid/veilid` (GitLab)
| Field | Value |
|---|---|
| Commits (main) | 2,300 (direct paged count; API returns no total) |
| Contributors | 69 |
| Stars / forks | 454 / 292 |
| Primary language | Rust (84% per GitLab language stats) |
| Licence | MPL-2.0 |
| Latest release | v0.5.8, 2026-11-02 |
| Releases, last 12 mo | 8 (31 total) |
| Last commit (main) | 2026-11-10 (project activity through 2026-11-23) |

### IPFS — `ipfs/kubo` (GitHub)
| Field | Value |
|---|---|
| Commits (master) | 16,023 |
| Contributors | 469 |
| Stars / forks | 17,194 / 3,176 |
| Primary language | Go |
| Licence | NOASSERTION (GitHub API; project is dual MIT/Apache) |
| Latest release | v0.43.1, 2026-09-15 |
| Releases, last 12 mo | 14 (100+ total) |
| Last push | 2026-11-23 |
| Note | v0.43 line is the last Interplanetary Shipyard release (veilid-ipfs-libp2p.md) |

### rust-libp2p — `libp2p/rust-libp2p` (GitHub)
| Field | Value |
|---|---|
| Commits (master) | 4,045 |
| Contributors | 369 |
| Stars / forks | 5,652 / 1,282 |
| Primary language | Rust |
| Licence | MIT |
| Latest release | libp2p crate v0.57.1, 2026-10-15 |
| Releases, last 12 mo | 2 GitHub releases (39 total; most releases are per-crate on crates.io) |
| Last push | 2026-11-23 |

### UCAN — `ucan-wg/rs-ucan` + spec repos (GitHub)
| Field | Value |
|---|---|
| Commits (main) | 102 |
| Contributors | 13 |
| Stars / forks | 94 / 26 |
| Primary language | Rust |
| Licence | Apache-2.0 |
| Latest release | ucan-v0.4.0, **2023-06-28** |
| Releases, last 12 mo | 0 |
| Last push | 2026-06-22 |
| Spec repos | `ucan-wg/spec` ★300, 97 commits, 17 contributors, tags incl. `v1.0-rc.1`, last push 2026-10-22; `ucan-wg/delegation` ★2, 7 commits, 2 contributors; `ucan-wg/invocation` ★20, 17 commits, 5 contributors; `ucan-wg/revocation` ★3, 3 commits, 1 contributor. None of the spec repos cut GitHub releases |

### Biscuit — `biscuit-auth/biscuit-rust` (GitHub)
| Field | Value |
|---|---|
| Commits (main) | 868 |
| Contributors | 33 |
| Stars / forks | 254 / 57 |
| Primary language | Rust |
| Licence | Apache-2.0 (crate `Cargo.toml`; GitHub licence API 404s on the workspace) |
| Latest release | GitHub: biscuit-quote-0.3.0, 2025-07-16 (per-crate tag); crates.io biscuit-auth newest 7.0.0-alpha.4, 2026-11-17 |
| Releases, last 12 mo | 0 GitHub releases (ships via crates.io) |
| Last push | 2026-11-21 |
| Downloads | biscuit-auth crate: 12,979,475 all-time (crates.io) |

### Delta Chat core — `deltachat/deltachat-core-rust` (GitHub)
| Field | Value |
|---|---|
| Commits (main) | 10,851 |
| Contributors | 68 |
| Stars / forks | 952 / 153 |
| Primary language | Rust |
| Licence | NOASSERTION (GitHub API; multi-licence repo) |
| Latest release | v2.69.0, 2026-11-09 |
| Releases, last 12 mo | 42 |
| Last push | 2026-11-23 |

### Pubky / pkarr — `pubky/pkarr` (GitHub)
| Field | Value |
|---|---|
| Commits (main) | 888 |
| Contributors | 26 |
| Stars / forks | 473 / 46 |
| Primary language | Rust |
| Licence | MIT |
| Latest release | v9.0.0, 2026-11-09 (crates.io confirms pkarr 9.0.0, updated 2026-11-12) |
| Releases, last 12 mo | 14 (24 total) |
| Last push | 2026-11-23 |

### Holochain — `holochain/holochain` (GitHub)
| Field | Value |
|---|---|
| Commits (develop) | 13,602 |
| Contributors | 71 |
| Stars / forks | 1,403 / 199 |
| Primary language | Rust |
| Licence | NOASSERTION (GitHub API) |
| Latest release | holochain-0.8.0-dev.16, 2026-11-23 (dev train; no stable 0.8 yet) |
| Releases, last 12 mo | 74 (100+ sampled) |
| Last push | 2026-11-23 |

### AT Protocol — `bluesky-social/atproto` (GitHub)
| Field | Value |
|---|---|
| Commits (main) | 3,628 |
| Contributors | 143 |
| Stars / forks | 9,732 / 944 |
| Primary language | TypeScript |
| Licence | NOASSERTION (GitHub API; project is dual MIT/Apache) |
| Latest release | @atproto/ozone@0.16.3, 2026-11-21 (per-package tags) |
| Releases, last 12 mo | 300+ (count capped at 300 sampled; all sampled were within 12 months) |
| Last push | 2026-11-23 |

### Cap'n Proto, Rust — `capnproto/capnproto-rust` (GitHub)
| Field | Value |
|---|---|
| Commits (master) | 3,842 |
| Contributors | 106 |
| Stars / forks | 2,522 / 269 |
| Primary language | Rust |
| Licence | MIT |
| Latest release | No GitHub releases; capnp crate 0.28.2, 2026-11-22 (crates.io) |
| Releases, last 12 mo | 0 GitHub releases (ships via crates.io) |
| Last push | 2026-11-22 |
| Downloads | capnp crate: 16,597,972 all-time (crates.io) |

### Cap'n Web — `cloudflare/capnweb` (GitHub)
| Field | Value |
|---|---|
| Commits (main) | 231 |
| Contributors | 36 |
| Stars / forks | 4,046 / 155 |
| Primary language | TypeScript |
| Licence | MIT |
| Latest release | capnweb@0.13.1, 2026-11-20 |
| Releases, last 12 mo | 23 (24 total) |
| Last push | 2026-11-20 |

### Endo — `endojs/endo` (GitHub)
| Field | Value |
|---|---|
| Commits (master) | 7,386 |
| Contributors | 62 |
| Stars / forks | 1,076 / 90 |
| Primary language | JavaScript |
| Licence | Apache-2.0 |
| Latest release | @endo/pass-style@2.0.1, 2026-08-17 (per-package tags; repo pushes daily) |
| Releases, last 12 mo | 98 (100+ sampled) |
| Last push | 2026-11-23 |

### Spritely Goblins (Guile) — `spritely/goblins` (Codeberg)
| Field | Value |
|---|---|
| Commits (main) | 1,980 |
| Contributors | not verified (no Codeberg endpoint) |
| Stars / forks | 93 / 8 |
| Primary language | Scheme |
| Licence | not verified via API |
| Latest release | v0.18.0 (tag; Apr 2026 per ocap-family.md); 12 tags on Codeberg |
| Releases, last 12 mo | not verified |
| Last commit | 2026-11-20 |
| Note | Moved from GitLab to Codeberg (Codeberg repo created 2025-03-11); the GitLab project page now says "MOVED TO codeberg.org/spritely/goblins" |

## Biggest dependents / projects built on each

- **p2panda** — Pillow, the p2panda × Worm Blossom collaboration (worm-blossom-catalogue.md);
  the ASIMOV project crates (asimov-id, asimov-kb) (crates.io reverse deps);
  otherwise mostly its own workspace crates — 23 reverse deps on p2panda-core (crates.io reverse deps).
- **Willow / willow25** — Earthstar 11 (Earthstar rebuilt on Willow) and Sneakerweb
  (willow-meadowcap.md); Ourcana, the only production deployment (~9–10k users,
  independent Dart implementation) (worm-blossom-questions.md); also Leaf/Weird and Riot (same notes).
- **Bab** — willow25 itself (Bab/WILLIAM3 is Willow's hash family); independent Zig
  and Dart implementations exist (worm-blossom-catalogue.md). No other dependents verified.
- **Keyhive** — Automerge Repo: Subduction is being integrated as its sync provider
  and Keyhive is dogfooded in the Automerge Repo + GAIOS stack (keyhive-subduction.md).
  No public crates.io reverse dependents of keyhive_core verified (0 listed).
- **Subduction** — Automerge Repo (same integration, keyhive-subduction.md);
  its `subduction_iroh` transport is the Keyhive stack's iroh composition (same note).
- **iroh** — Delta Chat (webxdc realtime via iroh-gossip; backup transfer over iroh)
  (iroh-investigation/real-projects-and-community.md); Prime Intellect, Psyche and
  Tandemn (decentralised AI training/inference — largest-scale non-chat users) (same note);
  OpenArchive Save (iroh blobs/replication) (veilid-ipfs-libp2p.md).
- **iroh-gossip** — Delta Chat webxdc realtime channels, shipped since 1.48 (Nov 2024)
  (iroh-investigation notes).
- **iroh-blobs** — Sendme (n0's file-transfer app, widely cloned) and OpenArchive Save
  (iroh-investigation notes; veilid-ipfs-libp2p.md).
- **Veilid** — VeilidChat (flagship app; beta, 1-to-1 text) and OpenArchive Save
  (Veilid for anonymous routing/group DHT) (veilid-ipfs-libp2p.md).
- **IPFS / Kubo** — IPFS Desktop, IPFS Companion, Rainbow gateway (all Interplanetary
  Shipyard, maintenance ended 2026-09-30) (veilid-ipfs-libp2p.md).
- **rust-libp2p** — Lighthouse (Ethereum consensus) and Forest (Filecoin in Rust)
  (veilid-ipfs-libp2p.md); Substrate — with the caveat that Polkadot moved to its
  own litep2p rather than deepen the commitment (same note).
- **UCAN** — Storacha (ucanto / ucantone; the largest production deployment, on
  0.10-era semantics) and Fission (originator; production use in Webnative/WNFS,
  company since shut down) (ucan-deep.md). No major dependents of rs-ucan itself verified.
- **Biscuit** — Hessra (hessra-token auth service) and Hedle (capability verifier)
  (crates.io reverse deps, 54 total); 12.98M crate downloads against only 254 GitHub stars.
- **Delta Chat core** — the Delta Chat Android, iOS and Desktop apps, all built on
  this shared Rust core (iroh-investigation/real-projects-and-community.md).
- **Pubky / pkarr** — Pubky Core / Pubky App / Pubky Ring (flagship consumer);
  iroh's address lookup (n0 maintains a pkarr fork) (iroh-investigation/protocol-census.md).
- **Holochain** — AD4M (ad4m-executor) (crates.io reverse deps, 11 total);
  Holochain's own launcher/client tooling and tauri-plugin-holochain (same source).
- **AT Protocol** — Bluesky itself (PDS/AppView/labeler run this codebase's protocol);
  Blacksky (independent Rust PDS + AppView) and ~3,600 independent PDS hosts
  (p2panda-investigation/research/bluesky-atproto.md).
- **Cap'n Proto (Rust)** — Sequoia PGP (sequoia-ipc, sequoia-keystore), Envio's
  HyperSync, Quantinuum's hugr (crates.io reverse deps, 135 total).
- **Cap'n Web** — Cloudflare Workers RPC / workerd: workerd is Cap'n Proto under the
  hood and Workers RPC is its production surface (capnp-and-survey.md).
- **Endo** — Agoric (agoric-sdk chain platform) and MetaMask (SES confinement —
  "Agoric and MetaMask rely on it", ocap-family.md).
- **Spritely Goblins** — few public dependents verified; primarily the Spritely
  Institute's own platform work, and the OCapN interop effort with Endo
  (ocap-family.md).

## Surprises

1. **pkarr is at v9.0.0, not 5.0.0.** The same-day iroh census recorded the pkarr
   crate at 5.0.0; both the GitHub release list and crates.io now say 9.0.0
   (2026-11-09/12), with v8.0.0 → v9.0.0 inside four months. Either the census
   read n0's fork version or upstream is moving much faster than recorded.
2. **Cap'n Web out-stars its parent.** 4,046 stars on 231 commits and 36
   contributors, vs capnproto-rust's 2,522 stars on 3,842 commits — a Cloudflare
   launch effect: maximum attention per unit of code in the whole table.
3. **Stars ≠ adoption, twice.** Delta Chat core — the flagship production iroh
   deployment — has only 952 stars; biscuit-auth has 12.98M crates.io downloads
   against 254 stars.
4. **rs-ucan is frozen while its spec moved.** No GitHub release since 2023-06-28
   and zero releases in 12 months, while `ucan-wg/spec` carries a `v1.0-rc.1`
   tag — the spec/implementation skew from ucan-deep.md, quantified.
5. **Subduction's release train.** 110 releases in its first ~16 months (repo
   created 2025-07-18), nearly all nightlies — release *count* there means
   automation, not milestones. Same caution for Holochain (74 dev releases,
   still no stable 0.8) and atproto (300+ per-package tags).
6. **Kubo is still the biggest repo here** (16,023 commits, 17,194 stars,
   469 contributors) two months after its stewardship ended — and it is still
   being pushed to daily. Momentum and maintenance have visibly decoupled.
7. **Veilid is quietly substantial:** 2,300 commits and 69 contributors on
   GitLab — more commits than iroh itself — with only 454 stars to show for it.
8. **Willow's footprint is tiny next to its design weight:** willow25 has 2,217
   all-time crates.io downloads and the repo 40 stars, against 1,565 commits.


## Change log

- r1 (2026-11-24): Migrated from hatch-personal workspace/decentralized-systems/stats.md under #13877; content unchanged (dataset compiled 2026-11-23).
