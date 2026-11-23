# atlas-github-comments

Source: [pkgs/atlas-search/crates/atlas-core/src/bin/atlas-github-comments.rs](https://github.com/developing-today/code/blob/main/pkgs/atlas-search/crates/atlas-core/src/bin/atlas-github-comments.rs) and [pkgs/atlas-search/crates/atlas-core/src/github_comments.rs](https://github.com/developing-today/code/blob/main/pkgs/atlas-search/crates/atlas-core/src/github_comments.rs) (extraction feed #6023; the writer is #10688).

`atlas-github-comments` is a one-verb writer: it extracts GitHub comments — issue comments, pull-request review comments, and pull-request reviews — from the repositories the user controls, and appends them as `atlas-github-comment/v1` records to a JSONL file, so the comments become a corpus source (source class `github comments`, source id `github-comments`). There are no subcommands and no other mode; the single invocation below is the whole tool.

The complete surface exists only in the source header. `--help` / `-h` prints the usage line to stdout, exit 0, and `--version` / `-V` prints `atlas-github-comments 0.1.0` — and, stated plainly, because it surprises everyone: **every failure in this binary exits 1, including pure argument errors**. There is no exit-2 path.

## Invocation

```
atlas-github-comments --state <watermarks.json> --out <records.jsonl> [--repo <owner/name>]... [--skip-reviews] [--observed-date <YYYY-MM-DD>]
```

| Flag | Argument | Default | Meaning |
|---|---|---|---|
| `--state` | path to a JSON watermark store | required | Per-repo progress: the stored `updated_at` watermark for each repository already extracted. Missing flag: `--state is required`, exit 1. |
| `--out` | path to a JSONL file | required | Append target for the emitted `atlas-github-comment/v1` records. Missing flag: `--out is required`, exit 1. |
| `--repo` | `&lt;owner&gt;/&lt;name&gt;` | — (whole estate) | Restrict the run to these repositories. Repeatable. With no `--repo`, the estate is enumerated from the GitHub API: the `developing-today` and `developing-today-forks` organisations plus the `dezren39` account — the #6023 our-repos estate. External repositories are never extracted here; that is gated on #6024. |
| `--skip-reviews` | (switch) | off | Skip the per-PR reviews walk (issue comments and review comments are still extracted). |
| `--observed-date` | `YYYY-MM-DD` | today, UTC (via `date -u +%F`) | The observation date stamped on the emitted documents. |

Any other argument — including the literal word `help` — is `atlas-github-comments: unknown argument: &lt;arg&gt;`, exit 1. A flag missing its value is named the same way, for example `--state needs a path`, exit 1.

## Behaviour

Extraction is done with the `gh` CLI only (`gh api`, with `--paginate`), and repositories are walked sequentially, as rate-limit discipline. For each repository the tool collects its issue comments, its review comments, and (unless `--skip-reviews`) its reviews, shapes them into comment documents, and appends one `atlas-github-comment/v1` record per document to `--out`. Each record carries its provenance — repository, thread number, comment id, and the observed date — so a record in the corpus names the exact comment it came from.

Progress is reported on stdout as it goes: an opening `estate: &lt;n&gt; repos` line, one line per repository — `&lt;repo&gt;: issue_comments=N review_comments=N reviews=N documents=N` — and a closing `totals: …` line.

### Idempotency and dedupe shape

Each repository starts from its last completed checkpoint. Issue comments and review comments both compare against the **same starting cutoff**. The per-PR submitted-review walk has its own cutoff and compares every PR to the start of that walk. A new issue comment therefore cannot hide an older eligible review. With `--skip-reviews`, submitted reviews retain their previous position.

The state file keeps the original `repos` map for comment streams and a `reviews` map for submitted reviews. On the first run with an older state file, the tool uses its saved cutoff for both. Upgrading preserves resume position. Historical omissions below a pre-upgrade cutoff require a separate reconciliation against source records, using a fresh state and separate output.

A completed repository's output is synced before its state is replaced. On a handled failure, the feed removes that repository's partial append and preserves its previous checkpoint. The process holds an advisory lock on the output and on a sibling `.lock` file next to the state. A hard crash between the append and checkpoint can leave replayed output; downstream promotion must deduplicate by source occurrence and revision. The feed's checkpoint only establishes extraction progress. Classification, promotion, publication, and retrieval need their own receipts.

### First-party source scope

An explicit `--repo` must name a single repository owned by `developing-today`, `developing-today-forks`, or `dezren39`. Comparison of owner names is case-insensitive. A malformed path or external owner fails argument parsing before output is opened. For both default discovery and explicit selection, the feed verifies the canonical API repository URL on returned issue, comment, and review rows. If GitHub follows a transfer or rename to a different repository, that pass fails closed; select the current eligible name explicitly after review. This checks exported provenance and cannot suppress redirects internal to the GitHub CLI.

### The HTTP 451 exception

One failure does not stop the run: a repository that is permanently access-blocked (HTTP 451, for example a DMCA block) is logged as `&lt;repo&gt;: skipped — repository access blocked (HTTP 451)`, counted in a final `skipped_repos: N (repository access blocked, HTTP 451)` line, and the run continues with the next repository. If nothing else failed, the run still exits 0.

See [Ingest today: staging, classification, promotion](../concepts/ingest.md) for how an extracted source like this one proceeds through staging and classification.

## Worked example

Extracting one toy repository into a fresh state and records file:

```
$ atlas-github-comments --state /tmp/toy-watermarks.json --out /tmp/toy-records.jsonl --repo developing-today/example-fixture
estate: 1 repos
developing-today/example-fixture: issue_comments=2 review_comments=1 reviews=1 documents=4
totals: …
```

Running the identical command again appends nothing for that repository unless new comments have appeared since the stored watermark — the second run is the resume/idempotency check, not a duplicate import.

## Failure shapes

- Missing `--state` or `--out`, an unknown argument, or a flag missing its value: `atlas-github-comments: &lt;message&gt;` on stderr, exit 1.
- A `gh` failure mid-run: the run stops, watermarks saved so far stay saved, the error notes `(state saved; rerun resumes)`, exit 1.
- File errors on the state or output paths: exit 1.
- The one non-failure: HTTP 451 repositories are skipped and counted, and do not by themselves change the exit code.

## Exit codes

| Code | Meaning |
|---|---|
| 0 | Success — including runs in which HTTP 451 repositories were skipped. |
| 1 | **Everything else**: argument errors, `gh` failures, and file errors alike. This binary has no exit-2 path. |
