# Fleet Turso databases — creation rule, inventory, and conversion debt

Canonical page for how the fleet creates and converts [Turso](https://turso.tech)
databases. Issue [#6827](https://github.com/developing-today/code/issues/6827) is
the tracking record for the remaining conversion debt and stays open until Turso
ships a rename or mode-convert verb.

Companions: [queue-turso-cutover.md](queue-turso-cutover.md) (the queue core
cutover, #6741) and [agent-events-turso.md](agent-events-turso.md) (the
`agent.events` stream, #5690). Those pages describe _specific_ migrations; this
one states the rule every DB creator needs and the debt that rule leaves behind.

## The rule: every new fleet Turso DB is born `--tursodb`

```bash
turso db create <name> --group <group> --tursodb
```

A database created without `--tursodb` is **libsql mode**, and there is no
supported way to change its mode afterwards. Getting it wrong costs a full data
migration later — an export/import of the whole database, a token re-mint, and a
producer flip for what should have been a metadata operation. Create it right the
first time.

This is not a queue-specific convention. It applies to every fleet Turso
database: the queue core, `agent.events`, and anything created from now on. The
rule was first written down in the queue cutover context
([queue-turso-cutover.md](queue-turso-cutover.md), "What exists"), where the
first `muse-queue` instance had to be destroyed and recreated as `fleet-queue`
because it was created without the flag.

### How to tell which mode a database is in

`turso db list` is the only mode evidence:

| `TYPE` column | Mode           | Verdict                                         |
| ------------- | -------------- | ----------------------------------------------- |
| `Turso`       | TursoDB (MVCC) | Correct — this is what `--tursodb` produces     |
| `SQLite`      | libsql         | Wrong for a new DB — conversion debt, see below |

### Things that do not work (verified on CLI v1.0.33)

- **There is no `turso db rename`.** A database's name is fixed at creation.
- **A `--from-db` fork inherits the source database's type.** `turso db create
--from-db X --tursodb` fails with _"tursodb flag contradicts the database
  derived type"_. A fork can never change mode, so forking a libsql database
  produces another libsql database.
- **There is no `turso db dump`.** The subcommand does not exist in this CLI
  version; it prints help. Use `turso db export`.

## Conversion debt: libsql holdouts

These databases predate the rule and are deliberately left in libsql mode. They
are known debt, not oversights.

| Database                      | Notes                                                                                                                                  | Status                    |
| ----------------------------- | -------------------------------------------------------------------------------------------------------------------------------------- | ------------------------- |
| **atlas monolith**            | Deliberately untouched during the 2026-11-07 audit + migration program ([#6619](https://github.com/developing-today/code/issues/6619)) | Open debt — not scheduled |
| **great-star-lord group DBs** | Same program and same deliberate hold ([#6619](https://github.com/developing-today/code/issues/6619))                                  | Open debt — not scheduled |

The 2026-11-07 program converted the fleet's eight libsql databases by full data
move (~7.7 GB of exports total). These holdouts were excluded from it.

**When to convert:** when Turso ships a rename or mode-convert verb, convert the
holdouts with it rather than by export/import, and close
[#6827](https://github.com/developing-today/code/issues/6827) citing that
conversion. Until then, this debt is tracked, not worked.

## Conversion procedure (export → import), if you must convert before then

This is the only working conversion in CLI v1.0.33. It is a full data move; plan
for downtime on the producer side.

1. **Freeze the source writers.** Stop and drain every producer that
   writes to the source database, and verify no writes are in flight.
   Keep the writers stopped until step 6 passes: a write accepted
   between export and repointing is lost at cutover, and the step 6
   row-count checks need not detect it (in-place updates, e.g. queue
   heartbeat progress writes, change no row counts). Resume writers
   only against the imported destination, never the source.

2. **Export the source database.**

   ```bash
   turso db export <source-db>
   ```

   This produces dump files in the current directory.

3. **Copy the export files to the target basename.** The new database is named
   after the _file_ basename, so rename the exported `.db` file to the name you
   want the new database to have (`<target>.db`).

   The export is a pair, not a single file: CLI v1.0.33 also writes the
   WAL sidecar `<source>.db-wal`, which can hold committed transactions
   the `.db` file alone does not. Never rename the `.db` away from its
   `-wal` sidecar. Before renaming, checkpoint the exported pair into a
   standalone database (open the exported `.db` with its `-wal` in
   place and run `PRAGMA wal_checkpoint(TRUNCATE);`), verify the `-wal`
   is folded in (empty or gone), and only then rename the single
   standalone `.db` file.

4. **Import with `--tursodb`.**

   ```bash
   turso db import <target>.db --tursodb
   ```

   The resulting database is named `<target>` and is in TursoDB mode. Confirm
   with `turso db list` — `TYPE` must read `Turso`.

5. **Repoint every producer.** Update each consumer's connection URL and token
   (the new database has its own token; mint it and vault it in BWS). Producers
   for the queue are listed in
   [queue-turso-cutover.md](queue-turso-cutover.md); for `agent.events`, in
   [agent-events-turso.md](agent-events-turso.md).

6. **Confirm.** Run a live read against the new database from each producer
   (`SELECT 1` through the real client path, not just the CLI) and check the row
   counts against the source.

7. **Delete the old database.**

   ```bash
   turso db destroy <source-db>
   ```

   Only after step 5 passes. Keep the export files until the old database is
   gone and the new one has served real traffic.

## Related

- [#6827](https://github.com/developing-today/code/issues/6827) — the tracking
  issue for this rule and the conversion debt.
- [#6619](https://github.com/developing-today/code/issues/6619) — the 2026-11-07
  fleet-wide DB audit and migration program.
- [queue-turso-cutover.md](queue-turso-cutover.md) — queue core on Turso (#6741).
- [agent-events-turso.md](agent-events-turso.md) — `agent.events` Turso sink
  (#5690).
