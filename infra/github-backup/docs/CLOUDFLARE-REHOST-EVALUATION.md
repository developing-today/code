# Cloudflare rehost of the four fleet repos — evaluation (#8584)

Status: **decided and built.** Drew picked option 2 (Workers + R2
smart-HTTP) on 2026-11-24 — see the addendum at the end of this file.
The evaluation below is the record the decision was made from.

## The question

#8575 slice 6 asked to "evaluate Cloudflare repo hosting as the second
home" for the repos, and #8584 records Drew's ruling to do all three
repo backups: a Cloudflare rehost, a plain mirror elsewhere, and R2
bundles. The plain mirror (developing-today `<repo>-mirror` repos) and
the R2 bundles are landed. The rehost is the remaining leg.

The rehost's distinct value over the other two copies: a **cloneable**
copy that survives a GitHub-side failure. The plain mirror is cloneable
but lives on GitHub, the same provider as the source; the R2 bundles
survive GitHub but are storage, not a git endpoint (a restore downloads
a bundle and clones locally — drilled, but a manual path).

## The constraint

Cloudflare has no native git hosting product: no git server, no
repository primitive. Every "Cloudflare rehost" is something the fleet
builds or fronts with Cloudflare parts. The candidates:

## Options

### 1. Count the R2 bundles as the Cloudflare copy

Zero new work — the bundles already live in R2 with verified read-back.
Rejected: #8584 counts the rehost and the R2 bundles as separate
copies, and merging them leaves one failure domain where the design
wanted two. It also provides no clone endpoint.

### 2. Workers + R2 smart-HTTP git server (read-only)

Serve git smart-HTTP (`info/refs`, `upload-pack`) from a Worker backed
by pre-generated packfiles in R2. Feasible only with pre-generated
packs: on-the-fly pack generation for repos of this size (the
openchamber bundle alone is ~400 MB) exceeds Worker CPU/memory limits.
So the sync job would publish packs + a ref index to R2 on the bundle
cadence, and the Worker would be a thin static server over them.

- Pro: no fleet hardware in the serving path; storage and serving both
  on Cloudflare; a real `git clone https://…` endpoint.
- Con: effort/l of new protocol code to own forever (pack negotiation,
  ref advertisement, shallow-clone edge cases); read-only forever;
  the pack-publishing pipeline duplicates much of the bundle job.

### 3. Cloudflare Tunnel in front of a git server on the backup host

Run a read-only git HTTP server on the laptop (the backup host that
already holds the bare mirrors — `git http-backend` behind a small
local server, or Gitea), and expose it through a Cloudflare Tunnel
under a fleet hostname, gated by Cloudflare Access.

- Pro: smallest real rehost (effort/s–m, existing pieces); a genuine
  clone endpoint on a non-GitHub provider path; storage stays in the
  already-drilled mirror set, so the clone drill verifies against the
  same refs the mirror job verifies.
- Con: Cloudflare is the transport, not the storage — a laptop-down
  day is a rehost-down day (the copies still exist on disk and in R2;
  only the endpoint is down). Adds one running server to keep patched.

### 4. Cloudflare Pages serving the bundle files

Static HTTPS download of the same bundles as option 1. Same collapse
problem as option 1, plus Pages file-size limits below the larger
bundles. Rejected.

## Recommendation

**Option 3** if the goal is a cloneable second home soonest with the
least new machinery; **option 2** if Drew wants the serving path to
survive the laptop being down as well, accepting the larger build.
Either satisfies the acceptance clone drill (clone from the rehost,
verify HEAD against the source).

## The exact step that needs Drew

1. Drew picks option 3 or option 2 (or rules the rehost leg closed
   with option 1's reasoning, amending #8584).
2. Provisioning then touches production Cloudflare state, which no
   lane is authorized to change on its own:
   - Option 3: create a Tunnel + a DNS record for the chosen hostname
     in the fleet Cloudflare account, and an Access policy naming who
     may clone.
   - Option 2: create the Worker + its R2 bindings and route.
3. After that step, the remaining work is ordinary lane work: the
   server/pack-publisher code, the scheduled sync, the clone drill,
   and the freshness probe's rehost checker (today it reports the
   copy unverified by construction; it gains a HEAD comparison
   against the rehost endpoint, same shape as the mirror checker).

Recorded on #8584 with this evaluation; the freshness readout keeps
the leg visible as STALE until it lands.

## Decision and build (2026-11-24)

Drew picked **option 2** — "Go with Workers + R2 smart-HTTP instead"
(2026-11-24 04:24 CST, recorded on #8584) — over this evaluation's
option-3 recommendation, accepting the effort/l protocol build in
exchange for a serving path that survives the laptop being down.

Built in the same pass (PR pending Felix's review at the time of
writing): R2 bucket `fleet-git-rehost`; Worker `fleet-git-rehost`
serving protocol-v0 smart-HTTP at the custom domain
`git.security.cab` (workers.dev address also answers); the publisher
`bin/rehost_publish.py` + `bin/rehost_publish_all.py` on the daily
03:20 `github-backup-rehost.timer`; and the freshness probe's
`cloudflare_rehost` checker wired to a real HEAD comparison against
the live advertisement. Design: `infra/github-backup/rehost/README.md`.
