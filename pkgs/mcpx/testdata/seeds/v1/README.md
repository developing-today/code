# mcpx run seeds — store v1

Where the seeds of record for mcpx CI test runs are kept (#1953), so a
seed that produced something interesting — above all a failure — can be
found and replayed long after the run's logs and artifacts expire
(Actions keeps those for 90 days; this store is in git, forever).

Seed *minting* (how a run gets its UUIDv7 seed and uploads it as the
`test-seed` artifact) is #1949. This directory is only the store and
its schema; `.github/workflows/mcpx-seed-collector.yml` is the only
writer.

## Layout

```
pkgs/mcpx/testdata/seeds/v1/<yyyy>/<mm>/<seed-id>.json
```

One file per seed, named by the seed id, in the month directory of the
record's `date`. The date is derivable from the id itself — a UUIDv7's
first 48 bits are the minting time in Unix milliseconds — so the date
tree is both the browsable view and the id lookup
(`ls v1/*/*/<seed-id>.json`), and no second index or symlink tree is
kept (symlinks also break Windows checkouts; there are no symlinks
anywhere in this store).

`v1` is the layout/format version. Generator versions live inside the
record, not in the path.

## Record schema

Each file is one line of compact JSON:

```json
{"seed":"<uuidv7>","commit":"<full sha the run tested>","ref":"refs/heads/main","event":"push","run":"<actions run url>","attempt":1,"result":"success","derivation":"v1","generators":"sha256:<manifest hash>","date":"2026-10-30"}
```

| field | meaning |
| --- | --- |
| `seed` | the run's UUIDv7 seed; equals the filename without `.json` |
| `commit` | full commit SHA the run tested |
| `ref` | git ref the run ran for (`refs/heads/main`, a PR head branch, …) |
| `event` | the Actions event that started the run (`push`, `pull_request`, …) |
| `run` | URL of the Actions run |
| `attempt` | run attempt number (a re-run mints a new seed; #1949) |
| `result` | the run's conclusion (`success`, `failure`, …) |
| `derivation` | seed derivation scheme version (#1949) |
| `generators` | `sha256:` hash of the generator `name@version` manifest at that commit (#1958); empty until generator manifests land |
| `date` | UTC date (`yyyy-mm-dd`) of the timestamp embedded in the seed id; equals the `<yyyy>/<mm>` directories the file lives in |

## Which seeds are kept

Per #1953, the collector keeps:

- main-branch push runs,
- the run that validated a PR's merge (a completed run on the head of
  a since-merged PR),
- **every failing seed**, from any branch — a failing seed is a
  regression candidate even if its PR never merged.

Passing runs of unmerged PR branches and local runs are not kept.

## Guard

`internal/seedstore`'s test validates every record in this store on
every `go test ./...`: the file parses, the seed is a well-formed
UUIDv7, the filename and the month directories match the record, and
the record's `date` equals the UTC date of the timestamp embedded in
its id. A corrupt record fails CI, not a future replay.

The store was bootstrapped with one format-sample record (run
41008926724, a main push) so the guard and the layout are exercised
before #1949's minting starts producing real ones; the collector
treats it like any other record — present, therefore skipped.
