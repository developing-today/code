# Cloudflare rehost — Workers + R2 smart-HTTP git server (#8584)

The fifth backup copy: a **cloneable** git endpoint for the four
fleet repos that survives both a GitHub-side failure and the backup
laptop being down. Surface chosen by Drew 2026-11-24 ("Go with
Workers + R2 smart-HTTP instead", recorded on #8584) over the
evaluation's Tunnel recommendation — see
`../docs/CLOUDFLARE-REHOST-EVALUATION.md`.

## Shape

```
backup mirror (laptop)                    Cloudflare
─────────────────────                     ─────────────────────────────
bin/rehost_publish.py   ── rclone ──▶     R2 bucket fleet-git-rehost
  pack-objects --all                        <owner>/<repo>/packs/<hash>.pack
  refs.json                                 <owner>/<repo>/refs.json
                                                  │ reads
git clone https://git.security.cab/…  ──▶ Worker fleet-git-rehost
```

- **Publisher** (`../bin/rehost_publish.py`, all-repos driver
  `../bin/rehost_publish_all.py`): per repo, builds ONE
  self-contained pack holding every object in the mirror
  (`git pack-objects --all --delta-base-offset`) plus a `refs.json`
  index (HEAD symref + oid, refs/heads + refs/tags with peeled tag
  targets, pack key). Upload order is the consistency rule: pack
  first (read back, SHA-256 compared), refs.json last (read back,
  byte compared) — refs.json is the commit point. Packs from older
  generations are pruned except the current and previous.
- **Worker** (`worker/`): a thin protocol-v0 shell over those two
  objects. `GET info/refs?service=git-upload-pack` renders the
  advertisement from refs.json; `POST git-upload-pack` validates the
  wants against the published oids, answers `NAK`, and streams the
  pack from R2. Read-only by construction: there is no receive-pack.
- **Schedule**: `github-backup-rehost.timer`, daily 03:20 + 30m
  randomized delay on the laptop (after the 00:00 fetch, 01:30 R2
  bundles and 02:15 mirror sync; before the 04:10 freshness probe).
  Registry row in `doc/fleet-schedule-registry.md`.
- **Verification**: the freshness probe's `cloudflare_rehost`
  checker compares the HEAD oid in this Worker's live advertisement
  against the local mirror HEAD for each fleet repo — the copy is
  stamped only on equality.

## Protocol notes (what v0-with-pregenerated-packs means)

- No `multi_ack` / side-band is advertised, so negotiation is the
  simple form: the client lists wants and haves, the server answers
  NAK and sends the pack. The pack is the whole repo, so a client
  with haves gets objects it already has again — correct, just not
  incremental. For a backup endpoint that is the right trade.
- `deepen` (shallow) requests are not honored with a shallow pack;
  the client receives full history. Clones are always complete.
- Wants must name published objects (ref tips, peeled tag targets,
  HEAD). Anything else is refused in-band with an `ERR` pkt-line —
  this is a backup of exactly four repos, not a general git host.
- The pack is streamed from R2 through the Worker; nothing is
  buffered in Worker memory, so repo size is an R2/bandwidth
  question, not a Worker-limits one.

## Operations

- Deploy the Worker: `worker/deploy.sh` (token from the vault by
  name, via `pkgs/cf/wrangler-authed.py`). `--dry-run` validates.
- Publish once by hand (laptop, credentials via the wrapper):
  `systemd/run-rehost.sh`, or one repo:
  `python3 ../bin/rehost_publish.py --repo developing-today/fleet`.
- Clone drill (the #8584 acceptance shape):
  `git clone https://git.security.cab/developing-today/fleet` and
  compare `git rev-parse HEAD` against the source repo's head.
- Rollback: redeploy the previous Worker version
  (`wrangler rollback` / versions flow) or delete the
  `git.security.cab` custom domain; the bucket contents are plain
  objects and the publisher regenerates them on the next run.
