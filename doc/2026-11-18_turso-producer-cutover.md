# Turso producer follow-up cutover, 2026-11-18

Supplement to [the recovery receipt](2026-11-18_supabase-turso-recovery.md).
Scope: laptop user-systemd producers and PM package consumers. This is a bounded
operational audit, not a claim that every remote Muse process was inspected.

## Quota gatherer nested alerts dependency

Although quota collection and its ledger already used Turso, the installed
`quota-gatherer.service` referenced the older `4jiky...-pm-aggregator` package.
Its `gather-quota.ts` imported `getAlertsStore`; that package's implementation
still resolved Supabase for provider-auth notification subscriptions.

Built only the generated quota unit through `~/tools/fleet-build-lock` from
integration commit `0809001c3b1681f0b6fff2dfa907588cfa286d51`:

- Unit: `/nix/store/wf904821s831zkph4fmrmnqm6n00idn7-quota-gatherer.service/quota-gatherer.service`
- Package: `/nix/store/qw96c6yk7yr7nc6r1dwkm4i7lmq9s0hp-pm-aggregator-0.1.0`
- Backup/receipt: `~/.local/state/pm-aggregator/quota-turso-cutover-20261118T220525Z/`

Paused only its timer, verified the oneshot was inactive, atomically replaced
its unit link, reloaded systemd, and restored the active timer. Existing drop-ins
were preserved. No running job was interrupted and no push was sent. Effective
ExecStart points to the new package. The first natural post-cutover quota tick
was not yet observed at this receipt's initial verification; old `Result=success`
was not treated as evidence that the new code had run.

The stale `default.target.wants/pm-aggregator.service` link was also preserved
in that backup directory and changed to `../pm-aggregator.service`. A daemon
reload, without restart, confirmed the effective running unit still uses the
new package. All PM-aggregator package references in the bounded, symlink-following
user-service scan now resolve to the new package (aggregator, quota, both digests,
and dashboard backup).

## AA event gatherer

The active daily `ainventory-gatherer.timer` used an untracked task scratch
script still reading the deleted Supabase project. PR [#11938](https://github.com/developing-today/code/pull/11938),
merge `e6326026443f647759e21397cb0033fbf0532385`, canonicalized it as
`scripts/ainventory_gather.py` and `scripts/ainventory-gatherer.service`.
The original scratch source was left untouched.

The migration preserves AA/models.dev parsing and comparison logic, translates
the current-event views with the original partition/order semantics, uses the
strict shared Hrana client, and inserts the parent and events atomically.
Arguments are validated before I/O. Private UUID intent files support read-back
after ambiguous transport outcomes; the writer does not blindly retry. Parent
`ran_at` and `created_at` are explicit UTC millisecond timestamps because the
live legacy parent table has no SQL defaults.

Verification:

- Nine focused tests independently passed after review, including empty-run
  zero writes, strict arguments, SQL literal safety, ordering, rollback, and
  explicit parent timestamps.
- Production read-only baseline: 107 models, 1,444 measurement events,
  566 current measurements, 110 current price rows.
- Actual dedicated Turso staging: atomic parent/event inserts, defaults,
  append-only guarded IDs, failure rollback, and explicit timestamps against
  a legacy-shaped no-default parent all passed.
- Full external-source dry run succeeded before activation.
- Narrow activation preserved the old service in
  `~/.local/state/ainventory-gatherer/cutover-20261118T221031Z/`, retained existing
  timer cadence, and did not interrupt any active oneshot.
- First live run exited 0 at 22:10:41 UTC. Batch
  `0324cd06-d677-4531-aae4-3fcb9085e842` contains exactly one parent, zero
  measurement events, and one genuine new price event. Direct read-back verified
  the recorded counts and both timestamps `2026-11-18T22:10:40.797Z`.
- Immediate second full **read-only** run found zero pending measurements or
  prices, demonstrating no repeated insertion of that observation. Timer active.

Local cutover directories preserve prior unit/link artifacts and machine-readable
receipts. These are recovery evidence, not an instruction to roll back to the
unavailable Supabase backend; repair forward.

## Remaining boundaries

The retired Supabase mirror/backup timers are inactive; historical failed service
status was not erased. Cold-store's Supabase-named step copies existing R2 archive
objects rather than querying the deleted database. No verified remote Muse
shell/API access path was available, so remote consumer coverage remains unproven.

Phone registration remains user-operated at `/alerts/preferences` on the existing
Tailscale dashboard HTTPS origin. **Enable push on this browser** re-posts an
existing local subscription into Turso. The page's local subscription status can
still say subscribed when its server record is missing; this is not delivery
verification. No test notification was sent during these changes.
