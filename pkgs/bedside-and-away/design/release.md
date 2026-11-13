# Release and recovery design

Parent: [#9359](https://github.com/developing-today/code/issues/9359). Covers BA-076, 077, 090, 091, 093–096.

Use one authoritative Turso placement behind the existing private gateway. Retain D1 as a local-compatible persistence surface, not as a second production writer. Adding an optional D1 placement later requires independent per-store backup, migration and access checks. Keep Atlas, Snowflake, public access and ChatGPT cookie forwarding out of this release.

## Options considered

- **Rewrite and cut over:** easier to normalize but high risk to IDs, snapshot history and memberships. Rejected.
- **Add compatible API and immutable release to the current stack:** preserves the existing data and origin, allows staged tests, and can roll back API/UI in order. Selected.
- **Expose public Worker frontend with new login:** different security boundary, provider and session design, not selected.

## Release checklist

1. Record branch and source commit, currently deployed Worker version, gateway executable target, health and migration checksum list. Never output secret values, real data, or private topology into Git. Do not overwrite the prior release.
2. Run local suites plus production-shape Worker dry run with native `cf`; compare allowlist and route before/after. Review new ESM imports and the exact packaged manifest. No Wrangler fallback.
3. Rehearse access and migrations with synthetic isolated data. Verify backup restore separately, including snapshot revisions, event counts and membership boundaries. Do not claim a live backup rehearsal from a fixture.
4. Run a private staging smoke across HTML, loaded scripts, signed gateway, database read/write and permission denials. A staging Worker must never receive production DB credentials or write production data.
5. Confirm this change needs no new network listener, no expanded access, and no destructive DB change. Release API before UI only if old clients remain compatible. Otherwise stop and use a coordinated maintenance window.
6. Verify deployed SHA/version, unsigned rejection, authenticated navigation, writes, version conflict handling and variants/steering on disposable data. Do not imply external research or Archer delivery from a saved request.
7. If a change fails, restore the prior gateway and Worker versions only if they can read the stored schema and honor all policy changes. If new ACLs were saved and old code cannot enforce them, freeze writes and keep the secured API in place pending repair. Never silently overwrite data written after the backup.

The prior Sites D1 copy is a distinct installation and cannot be linked by nickname or browser ID. Two signed identities and an audited source/target mapping are prerequisites before moving any existing user content. Rotation and backups need configured operators and verified evidence; no automatic rotations or new background jobs are enabled here.
