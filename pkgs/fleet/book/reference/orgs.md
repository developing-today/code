# `fleet orgs`

`fleet orgs` maintains and reads the Turso-backed registry of GitHub Apps,
organizations and installations. The registry is populated using each
configured fleet App's own GitHub App JWT; it does not use OAuth-token
organization reads as a substitute for installation coverage.

```text
fleet orgs list [--app SLUG] [--org LOGIN] [--permission NAME[=LEVEL]] [--json]
fleet orgs sync
```

## `list`

Reads the stored registry and prints one row per installation: organization,
App, installation ID, repository selection and permissions. Output includes
the last successful sync time. Options may be combined:

| Option                    | Meaning                                                    |
| ------------------------- | ---------------------------------------------------------- |
| `--app SLUG`              | Match the GitHub App slug.                                 |
| `--org LOGIN`             | Match the account login.                                   |
| `--permission NAME`       | Match installations granting that permission at any level. |
| `--permission NAME=LEVEL` | Match a permission at the specified level.                 |
| `--json`                  | Emit the same filtered rows as JSON.                       |

## `sync`

Records a sync run, then for each fleet App (`app`, `devtoday`, `backup`) calls
`GET /app` and paginated `GET /app/installations` with that App's JWT. It
upserts the Apps, organizations and installations; installations no longer
returned by GitHub are removed. A successful command prints the sync-run ID
and per-App counts. Missing configuration, unresolved secrets, or GitHub/
database failures are operational errors (exit 69); unknown verbs and invalid
options are usage errors (exit 2).

## Configuration

| Name                     | Purpose                                                            |
| ------------------------ | ------------------------------------------------------------------ |
| `FLEET_ORGS_TURSO_URL`   | HTTP URL of the registry database.                                 |
| `turso.fleet-orgs-token` | Secret name resolved for the Turso auth token.                     |
| App-specific secrets     | Private keys (and IDs where not public) used to mint each App JWT. |

The command is implemented by `fleet-orgs`; see [The crates](../library/crates.md#fleet-orgs)
for the storage model and sync details. The binary must include the `orgs`
capability for these verbs to be available.
