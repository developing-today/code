# Research: beyond GitHub — forges, git over the ID graph, and cloud data planes (#10396)

- **Issue:** [#10396](https://github.com/developing-today/code/issues/10396), design mill pass 7 (exploratory), parent [#10389](https://github.com/developing-today/code/issues/10389).
- **Kind:** research. Options only. **No migration is authorized or proposed for action by this note.**
- **Related records:** Turso + Cloudflare integration design (`doc/2026-11-24T00-45-00Z_design_turso_cloudflare_integration.md`, #8870, still awaiting Drew's ratification); iroh investigation (`doc/2026-11-24T02-11-50Z_research_iroh-investigation/`); embedding research (`doc/2026-11-25T02-10-00Z_research_fleet-id-mcpx-embedding-10394/`).
- **Sources read:** 2026-11-26 (UTC). Prices move; every figure below carries its source and the day it was read.

## 1. Request

From the issue: the fleet relies on GitHub for repos, issues, forks, comments, PRs and free compute. Keep going all-in on GitHub, but scope alternatives toward a possible dual-home or many-home world (lab + Cloudflare + Turso). Three areas: (1) forge alternatives, including one self-hosted tool Drew remembers as "still uses git as its store" with a more sensible mental model, and brief notes on alternatives to git itself; (2) git over the ID graph and iroh; (3) cloud data planes (per-user, per-repo and per-fork databases, Turso tenancy, Durable Objects for merge queues and runners, D1, and Cloudflare Artifacts, which Drew suspects is expensive).

**Acceptance:** an options note per area with costs and honest unknowns, ending in a stay / dual-home / move recommendation per surface (repos, issues and PRs, CI).

**Verified when:** this note is merged under `doc/`, every price cites a primary page with its read date, and §6 gives one recommendation per surface. Nothing in the fleet changes as a result.

## 2. Short answer

| Surface             | Recommendation                         | Why, in one line                                                                                                                                                                                        |
| ------------------- | -------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Repos (git hosting) | **Stay on GitHub, add a cheap mirror** | Mirroring git is easy and reversible; a second home for the bytes costs almost nothing and removes the single point of loss.                                                                            |
| Issues and PRs      | **Stay**                               | This is where the fleet's tooling, queue, lanes and history live. Every alternative loses the API surface the fleet already uses, and none federates with GitHub yet. Export a periodic backup instead. |
| CI                  | **Stay, keep the lab as overflow**     | GitHub-hosted runners are free for this repo's use; the lab and Blacksmith are already the overflow paths. A Durable Object merge queue is buildable but replaces something that works.                 |

The one Drew could not remember is most likely **Radicle**: a peer-to-peer forge, written in Rust, built on git, that stores issues and patches inside the git repository itself and syncs them between peers with no central server. The second candidate is **Jujutsu (jj)**, which is not a forge but a version-control front end whose default backend is a plain git repository and whose mental model (the working copy is a commit, no staging area, undo for every operation) is the "more sensible" one people usually mean. §3.1 lays out both so Drew can say which.

The interesting long-term path is not a forge migration but **git objects in the ID graph** (§4): ID already moves BLAKE3-addressed blobs over iroh, and third-party git remote helpers over iroh already exist. That is a research track, not a move.

## 3. Forge alternatives

### 3.1 The tool that "still uses git as its store"

| Candidate        | What it is                                                                                                                                                                                                                                                                                                  | Why it fits the description                                                                                                                                         | Why it might not                                                                                                                                                                     |
| ---------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| **Radicle**      | P2P code collaboration stack in Rust, MIT/Apache, built on git. Each user runs a node that acts as a local git server; issues, patches and reviews are git objects in the repo and sync between peers. ([LWN, 2024](https://lwn.net/Articles/966869); [ArchWiki](https://wiki.archlinux.org/title/Radicle)) | A forge, self-hosted, git is the store for code _and_ collaboration data, no single source of truth. Matches §4's direction almost exactly.                         | Younger than mainstream forges. ArchWiki lists a read-only web frontend, no releases beyond tags, no milestones, due dates or dependencies (as of its last edit; may have improved). |
| **Jujutsu (jj)** | VCS whose UI and algorithms are separate from storage; the git backend is fully featured and produces ordinary git commits; can co-locate with a `.git` so `jj` and `git` work on the same repo. ([jj git-compatibility docs](https://docs.rs/crate/jj-cli/0.41.0/source/docs/git-compatibility.md))        | "Uses git as its store", "mental model more sensible": working copy is a commit, no staging area or stash, every operation undoable, conflicts don't stop a rebase. | Not a forge. No issues or PRs; it sits on top of any forge, including GitHub. No Git LFS.                                                                                            |
| git-bug          | Issue tracker that stores issues as git objects and refs.                                                                                                                                                                                                                                                   | Issues in git.                                                                                                                                                      | Issues only; not a full forge. Not verified this pass.                                                                                                                               |

**Open question Q1 (for Drew):** which one did you mean? If it is jj, the cheapest step is local: anyone can use `jj` in a co-located checkout today with no fleet change. If it is Radicle, §4 below is the closest fit.

### 3.2 Self-hosted forges with a conventional model

| Forge                           | Notes                                                                                                                                                                                                        | Federation with GitHub                                                                                                                                                                                                                         |
| ------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Forgejo** (Codeberg's engine) | Community fork of Gitea; v15.0 LTS released April 2026 (per [Codeberg overview](https://laoutaris.pages.dev/blog/codeberg/), secondary). Runs on one small box; has Actions-compatible CI (Forgejo Actions). | ForgeFed federation is partial: repository stars federate as of 2025 ([Wikipedia](https://en.wikipedia.org/wiki/Forgejo)); NLnet grant ran Oct 2022–Jun 2025 ([NLnet](https://nlnet.nl/project/Federated-Forgejo/)). **No GitHub federation.** |
| Gitea                           | Upstream of Forgejo; similar footprint.                                                                                                                                                                      | None.                                                                                                                                                                                                                                          |
| GitLab CE                       | Full-featured, heavy (multi-GB RAM). Has started ForgeFed work per Wikipedia.                                                                                                                                | None yet.                                                                                                                                                                                                                                      |
| ForgeFlux                       | Adapter project aiming to let Forgejo and GitHub federate ([NLnet](https://nlnet.nl/project/ForgeFlux/)). Status in 2026 unknown.                                                                            | The only bridge found; unproven.                                                                                                                                                                                                               |

**Honest unknown:** no forge today gives two-way issue and PR sync with GitHub. A dual-home for issues means building or adopting a bridge.

### 3.3 Alternatives to git itself (brief)

- **jj** (above): keeps git compatibility, so it is additive.
- **Pijul**: patch-based with a sound merge theory; does not use git storage, so it breaks every git tool and host. Not a fit for a git-centred fleet.
- **Sapling** (Meta): git-compatible client with a stack-oriented workflow; similar niche to jj.

None of these justify leaving git. jj is the only one worth trying, and it needs no fleet change.

## 4. Git over the ID graph and iroh

### 4.1 What exists

- ID already depends on iroh-blobs across its crates (`pkgs/id/Cargo.toml`, `pkgs/id/crates/id-store/Cargo.toml`, `id-sync`, `id-web` and others). iroh-blobs moves opaque bytes verified incrementally by their **BLAKE3** hash.
- Third-party git transports over iroh exist: [git-remote-iroh](https://codeberg.org/cmars/git-remote-iroh) (a git remote helper over iroh endpoints, access by peer key) and [wronex/iroh-git](https://github.com/wronex/iroh-git) (a daemon that publishes its address through n0's relay and DNS). Neither stores git objects _as_ iroh blobs; both carry git's own pack protocol over an iroh connection. n0's own experiments repo has no git project.
- ID's federation layer (B17 signed queries, B19 signed receipts, B32 ingest gate, B07 conflict reports) already gives signed provenance, scoped sync and preserved conflicts, which is what forge metadata (issues, comments, reviews) needs.

### 4.2 Three shapes, cheapest first

1. **Transport only.** A git remote helper over iroh, like git-remote-iroh, pointed at a bare repo on a lab host. Git stays the store; iroh replaces SSH/HTTPS. Cost: small. Gain: NAT-free peer pushes and pulls between fleet machines. Unknown: the helper's maturity; git's `stateless-connect` capability is still marked experimental in git's own docs.
2. **Packs as blobs.** Store each pushed packfile as an iroh blob, with refs as signed ID events naming the pack hash. Cost: medium. Gain: content-addressed replication and dedup across peers using machinery ID already has. Unknowns: git objects are SHA-1 (or SHA-256) addressed while iroh is BLAKE3, so every lookup needs a mapping; packs are already zlib- and delta-compressed, so the issue's compression question mostly answers itself: keep git's compression and do not re-compress.
3. **Objects as blobs, forge metadata as ID events.** Individual commits, trees and blobs as iroh blobs; issues, comments, reviews and merge requests as signed ID graph objects. This is Radicle's model rebuilt on ID. Cost: large. Gain: no single source of truth; writes over iroh can be cross-posted to GitHub. Unknowns: per-object overhead for large repos (this repo has a 15 GB ISO and VM images at the root, which would dominate any object store), and how a GitHub cross-post maps authorship and ids back.

**Sync model.** The issue suggests iroh as the de-facto source of truth with cross-posting outward. The honest constraint is that GitHub is not a peer: it accepts writes only through its API and assigns its own ids, so a cross-post is a one-way projection unless a bridge also reads GitHub back. Start with GitHub as the authority and iroh as a replica (shape 1 or 2), and only invert that once shape 3 exists and has been used.

**Recommendation for this area:** if Drew wants to explore, shape 1 is a weekend-sized experiment and fully reversible; open a separate issue for it. Shapes 2 and 3 belong in the ID roadmap after the federation slices B27–B30 land, because they reuse that evidence and claims machinery.

## 5. Cloud data planes

### 5.1 Prices (primary pages, read 2026-11-26)

| Service                                                                       | Billing                                                                             | Included on paid plan                                                            | Overage                                                                                                                              | Source                                                                                                                                                                          |
| ----------------------------------------------------------------------------- | ----------------------------------------------------------------------------------- | -------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Cloudflare Artifacts** (hosted git repos: create, push, pull, fork, delete) | Operations + storage; Workers Paid only; billing starts **2026-11-19**              | 10,000 operations/month; 1 GB-month storage                                      | $0.15 per 1,000 operations; $0.50 per GB-month. Repo count charge ($0.01 per repo over 100,000) announced, not yet billed            | [Artifacts pricing](https://developers.cloudflare.com/artifacts/platform/pricing/)                                                                                              |
| Cloudflare D1                                                                 | Rows read, rows written, storage; no compute or egress charge                       | 25 B rows read, 50 M rows written, 5 GB                                          | $0.001 per M reads, $1.00 per M writes, $0.75 per GB-month                                                                           | [D1 pricing](https://developers.cloudflare.com/d1/platform/pricing/)                                                                                                            |
| Durable Objects (SQLite-backed)                                               | Requests, duration, rows, storage                                                   | 1 M requests, 400,000 GB-s, 25 B rows read, 50 M rows written, 5 GB-month        | $0.15 per M requests, $12.50 per M GB-s, rows as D1, $0.20 per GB-month. Idle objects that can hibernate are not billed for duration | [DO pricing](https://developers.cloudflare.com/durable-objects/platform/pricing)                                                                                                |
| Turso                                                                         | Plans by database count, rows and storage; branching is metadata-only copy-on-write | Free: 100 databases; paid plans: unlimited databases; Developer from $4.99/month | Scaler overage $0.80 per B rows read, $0.80 per M rows written, $0.50 per GB (Turso blog)                                            | [turso.tech](https://turso.tech/what-is-turso); [overages post](https://blog.turso.tech/turso-introduces-additional-pricing-flexibility-with-support-for-overages-1fbe7e33b704) |

**Artifacts, honestly.** Drew's suspicion is half right. Storage is cheap ($0.50 per GB-month, replicas free), but **operations are the trap**: every clone, fetch and push is one operation, and content reads count at 100 per operation. Twelve lanes each fetching the monorepo every 5 minutes make about 3,500 operations a day and pass the 10,000 included in about three days; at, say, 50,000 fetches a month the bill is about $6 a month in operations, which is small, but a CI design that clones per job at scale grows linearly with no cap. Since GitHub does all of this free today, Artifacts only earns its place as a mirror target with few operations (push on merge, rare clones), or as a home for many small generated repos (per-agent scratch repos), where the 100,000 included repos matter.

**D1 vs Turso for per-repo or per-fork databases.** The issue's note holds up: D1 has no cheap fork (a copy is a new database with all rows rewritten, billed as writes), while Turso branches are copy-on-write metadata and count as ordinary databases. For "duplicate until changed" per-fork data, Turso is the better fit on price and mechanics. **New fact for this decision:** Supabase announced on 2026-10-02 that it is acquiring Turso ([Verdict](https://www.verdict.co.uk/supabase-150m-funding-turso-acquisition/); Turso's own site now carries the banner). The fleet is mid-migration from Supabase to Turso (#5690–#5693). No pricing or service change was announced in what I read, but the ownership change is a reason to keep the fleet's Turso usage behind the join contract the P6 design proposes, so a later move stays cheap.

**Durable Objects for merge queues and runner triggers.** A per-repo DO is a natural single-writer merge queue: one object serializes merges, holds the queue in its SQLite, and hibernates between events, so its idle cost is near zero. It cannot run CI itself; it would trigger GitHub Actions, the lab, or Blacksmith. Cost at fleet scale is in the free allowances. The reason not to build it now is that GitHub's merge path already works and the fleet merges by squash with no queue contention worth fixing.

### 5.2 Unknowns

- Whether Artifacts operations include a GitHub-style mirror sync as one operation or as many.
- Turso storage billing for shared branch segments (not stated in what I read).
- Any Turso plan change after the Supabase acquisition.
- Forgejo federation status in 2026 (primary sources not checked this pass).
- git-remote-iroh's maturity and maintenance.

## 6. Recommendation per surface

**Repos — stay, plus a mirror (dual-home for bytes only).** Keep GitHub as the authority. Add a read-only mirror that can be restored from: the lab (a bare repo, or Forgejo if a web view is wanted) or Cloudflare Artifacts with push-on-merge only. The repo-root ISO and VM images should be excluded from any mirror. Cost: near zero. This is the only step worth proposing as a follow-on issue, and even that needs Drew's yes because it touches where the fleet's code lives.

**Issues and PRs — stay.** No alternative syncs with GitHub, and the fleet's queue, lanes, agent notes and comment history are built on GitHub's API. Take a periodic export (issues, comments and PR metadata as JSON into the lab or R2) so the history is not single-homed. Revisit if Radicle or ForgeFlux reaches two-way GitHub sync, or once §4 shape 3 exists.

**CI — stay.** GitHub-hosted runners are the default by Drew's rule, with the lab and Blacksmith as explicit overflow. A Durable Object merge queue is cheap and buildable, but there is no queue pain to fix.

**Move — nowhere, for now.** Nothing found justifies moving any surface.

## 7. Open questions

| #   | Question                                                                         | Default until answered                                                                       | Decider                       |
| --- | -------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------- | ----------------------------- |
| Q1  | Which tool did Drew mean: Radicle, jj, or another?                               | Treat as Radicle for the forge track; jj is a personal-workflow choice with no fleet impact. | Drew                          |
| Q2  | Is a bytes-only mirror of the repo wanted, and where (lab, Artifacts, Codeberg)? | No mirror until asked.                                                                       | Drew                          |
| Q3  | Should a git-over-iroh transport experiment (§4.2 shape 1) get its own issue?    | Not filed; offered here.                                                                     | Drew                          |
| Q4  | Does the Supabase acquisition of Turso change the P6 stance?                     | No change; keep the join contract so a move stays cheap.                                     | Drew (P6 ratification, #8870) |
| Q5  | Is a periodic export of issues and PRs wanted?                                   | Not built until asked.                                                                       | Drew                          |

## 8. References

- Cloudflare Artifacts pricing: https://developers.cloudflare.com/artifacts/platform/pricing/
- Cloudflare D1 pricing: https://developers.cloudflare.com/d1/platform/pricing/
- Cloudflare Durable Objects pricing: https://developers.cloudflare.com/durable-objects/platform/pricing
- Turso: https://turso.tech/what-is-turso ; overages: https://blog.turso.tech/turso-introduces-additional-pricing-flexibility-with-support-for-overages-1fbe7e33b704
- Supabase acquiring Turso: https://www.verdict.co.uk/supabase-150m-funding-turso-acquisition/
- Radicle: https://lwn.net/Articles/966869 ; https://wiki.archlinux.org/title/Radicle
- Jujutsu git compatibility: https://docs.rs/crate/jj-cli/0.41.0/source/docs/git-compatibility.md
- Forgejo: https://en.wikipedia.org/wiki/Forgejo ; https://nlnet.nl/project/Federated-Forgejo/ ; ForgeFlux: https://nlnet.nl/project/ForgeFlux/
- git-remote-iroh: https://codeberg.org/cmars/git-remote-iroh ; iroh-git: https://github.com/wronex/iroh-git
