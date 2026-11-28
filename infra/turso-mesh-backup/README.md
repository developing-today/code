# turso-mesh-backup — mesh class-A restic leg (#13455 slice 2, #14247)

Repo-shipped, not yet installed. The runner is
`pkgs/cold-store-reaper/scripts/turso-mesh-restic-runner.py`: per
class-A database (`fleet-queue`, `fleet-platform`, `atlas`) it runs
the export seam (the `export-turso.sh` proven path with its repair
filter), builds the slice-1 export manifest, streams the dump through
zstd, and takes one restic snapshot — compressed dump + manifest
together — tagged `class-a`, `mesh=turso-mesh`, `db=<name>` against
the dedicated repository `restic-turso-mesh` (live: the R2
`supabase-backups` bucket). Retention after each run: keep-daily 7 /
keep-weekly 8 / keep-monthly 12 with `--prune` (design §5).

`systemd/` holds the templated service `turso-mesh-restic@.service`
and one timer per database implementing the slice-1 stagger plan
(`turso-mesh-class-a.py plan`): atlas Saturday, fleet-platform
Sunday, fleet-queue Monday, all 05:10 + `RandomizedDelaySec=30m`.

Secrets are by name only: the repository URL and the NAME of the
restic password variable arrive through the unit's EnvironmentFile,
materialized from Bitwarden at install time. No live export, R2
write, or credential access happens in this slice — live first light
is the follow-on slice under #13455.
