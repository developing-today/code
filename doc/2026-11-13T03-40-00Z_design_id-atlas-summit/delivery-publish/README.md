# ID/Atlas delivery publisher (preparation only)

This is a standard-library Python publisher skeleton for the issue/queue
delivery in #9106. It was not executed against GitHub or the fleet queue. Its
default mode is offline dry-run; external writes require both `--execute` and
`--go 9106`. Those flags are a deliberate interlock, not a substitute for the
parent's approval. Do not use them until the parent explicitly authorizes the
exact manifest digest and merged design commit.

## Inputs and approval binding

Run from this checkout's repository root. The ordered input manifests are the
seven package-relative paths in `publish.py`'s `DEFAULT_MANIFESTS` (or pass all
seven `--manifest` arguments in that exact order). For the digest, each is
prefixed with the package's repo-relative path. The script recomputes the
preparation helper's SHA-256 over `[[repo-relative path, parsed JSON], ...]` with sorted keys,
compact separators and UTF-8, checks it against `--approved-digest`, then
requires the helper's `--report` JSON to contain `valid: true`, no errors, the
same `manifest_sha256`, normalized `entries` in identical ID/order, and a
normalized `orchestration` record. Draft files are read from `--draft-dir`;
each must exactly match the standard-library rendering of that normalized row
and digest (including stable marker and settled `main` URL). The verified draft
body is re-used for publication. `--approved-report-digest` binds the exact
report bytes by SHA-256; execution requires it, and dry-run checks it when
provided. The report digest is part of journal identity, so normalized-field
changes cannot reuse the prior approval or journal even when manifest IDs are
unchanged.

`--design-sha` must be a full 40-hex SHA. In execute mode, read-only GitHub API
calls verify the exact commit SHA and that `SHA...main` is `ahead` or
`identical` (the design commit is an ancestor of remote `main`). The journal is external to the package and bound to repository,
manifest digest, report digest and SHA. A single non-blocking `flock` excludes simultaneous publishers;
each state transition is atomically replaced and fsynced.

Before any GitHub call in execute mode, the publisher compares all seven
manifest JSON values, the preparation helper/schema and validation inputs, and
every referenced design-source file against `git show <design-sha>:<path>`.
JSON inputs use canonical parsed-value comparison; helper, schema and design
sources compare byte-for-byte. A dirty or otherwise different local input
refuses before issue creation, even if the supplied SHA is an ancestor of
`main`.

Example (dry run only; paths are illustrative):

```sh
python3 -I doc/2026-11-13T03-40-00Z_design_id-atlas-summit/delivery-publish/publish.py \
  --approved-digest <approved-sha256> --design-sha <full-merged-sha> \
  --approved-report-digest <approved-report-sha256> \
  --report doc/2026-11-13T03-40-00Z_design_id-atlas-summit/delivery/report.json \
  --draft-dir doc/2026-11-13T03-40-00Z_design_id-atlas-summit/delivery/drafts \
  --journal "$HOME/.local/state/id-atlas-9106/publisher.json"
```

## Safety boundary / unresolved blockers

- The script deliberately targets only `developing-today/code` and
  `repo-health-code-pilot`; it must refuse any other checkout.
- Slice issues use their canonical draft body, native component parent, and
  native blockers for hard edges only. Soft sequencing is never converted to
  a native blocker. The orchestration/meta issue is separate under #9112.
- Each issue gets a create-intent journal record before the create call. On
  uncertain outcomes the script checks paginated newest-issue GraphQL
  results for the exact stable marker and exact body before it can resume. It
  will not blindly replay an ambiguous create or delete/recreate to repair a
  mismatch. GitHub search indexing is not used as the sole dedupe source.
- The approved queue packet uses the supported `repo-task/v1` fields and a
  `triage` dedupe family tied to the created issue number. Its mandatory
  preflight says inspect native GitHub blockers before implementation; if any
  remain open, do no implementation and self-block the claimed task using the
  worker's own identity and typed issue dependency. The publisher does not
  claim tasks or call `repo-queue block`, and does not describe queue rows as
  broker-blocked. It explicitly says not to start contingent research before
  its trigger and grants no production-rollout authorization.
- Queue readback uses authenticated `POST /list` with the verified non-rich
  `{queue, task, limit:1, offset:0}` filter, then independently checks exact
  task ID, queue, dedupe key and live status (`queued`, `claimed`, `parked`, or
  `blocked`). Do not switch this to rich mode: the rich route's task filter was
  observed returning a different task. The base URL comes from
  `--broker-url`, `REPO_QUEUE_BROKER_URL`, or
  `~/.config/repo-queue/broker-url`; token path comes from `--token-file`,
  `REPO_QUEUE_TOKEN_FILE`, or `~/.config/queue-broker/token`, sent as
  `X-Agent-Token`. Credentials are checked before any issue write. The enqueue
  command's row JSON supplies its task ID. Resume never intentionally
  re-enqueues an already journaled task.
- Do not change the journal key or delete/recreate issues to repair errors. An
  issue whose queue enqueue/readback fails stays filed and is retained in the
  journal; a marker-deduped comment says coverage was not verified.

No remote writes, queue calls, commits, or external artifact creation were
performed while preparing this tool.
