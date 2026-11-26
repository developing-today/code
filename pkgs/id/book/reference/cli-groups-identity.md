# Groups, documents, claims and keys

Source: `pkgs/id/crates/id-cli/src/cli/mod.rs`, `pkgs/id/crates/id-cli/src/cli/groups.rs`, `pkgs/id/crates/id-cli/src/cli/doc.rs`, `pkgs/id/crates/id-cli/src/cli/claim.rs`, `pkgs/id/crates/id-cli/src/cli/talk.rs`, `pkgs/id/crates/id-cli/src/cli/writer_sets.rs` under https://github.com/developing-today/code/blob/main/pkgs/id/crates/id-cli/src/cli/mod.rs — provenance only; this chapter is complete without following it.

This chapter covers the event-backed identity surface of `id`: `group` with `delegate` and `revoke`; `doc`; `claim`; `talk`; `writer-sets`; and the identity commands `id`, `key`, `keyset` and `peers`. The concepts behind them are in [Identity, the directory and groups](../concepts/identity-directory.md) and [Events and the signed audit chain](../concepts/events.md). The event-backed claims here are a separate store from the file tags of [The tag family](cli-tags.md).

Two global flags place the stores (see [The id command line](cli.md) and [Configuration and environment](configuration.md)):

| Flag            | Argument       | Default                        | Meaning                                                                                                    |
| --------------- | -------------- | ------------------------------ | ---------------------------------------------------------------------------------------------------------- |
| `--groups-log`  | file path      | `groups.jsonl` in the data dir | The group log for `group`, `delegate` and `revoke`. A relative path is relative to the data dir.           |
| `--event-store` | directory path | `events` in the data dir       | The event store for `doc`, `claim`, `talk` and `writer-sets`. A relative path is relative to the data dir. |

`group`, `doc`, `claim`, `talk` and `writer-sets` each take a `--json` flag that is global to that parent command: it may appear before or after the subcommand. With `--json`, an action prints one JSON object; `group list`, `doc` lists, `claim list`/`search`, `talk list` and `writer-sets discover` print one JSON array. `delegate` and `revoke` are top-level commands with their own `--json` flag.

Exit behaviour, all commands in this chapter unless a command states otherwise: success exits `0`; a handler error prints `Error: <message>` to stderr and exits `1`; a clap parse error prints the error and usage to stderr and exits `2`. Accounts and groups are 64 hex characters throughout. The toy group used in the examples is the invented ID `0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef`; the toy accounts are `fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210` and `1111111111111111111111111111111111111111111111111111111111111111`.

## `group`

Each group action records one event in the group log and prints one line, or one JSON object with `--json`. A refused action exits non-zero, and its event stays in the log.

The role vocabulary used by `group add`, `group role` and `delegate` is one ordered enum, least to most: `pull` (replicates ciphertext without keys), `read` (reads content), `write` (edits content), `manage` (adds and removes members), `admin` (manages, and changes roles and the policy record).

### `group create`

Creates a group. A normal group makes its creator Admin. A friendship names its other side with `--with`, has no Admin unless `--admins-on-create`, and is active once the other side accepts.

```
id group create [--template <normal|friendship>] [--with <ACCOUNT>] [--max <N>] [--admins-on-create]
```

| Flag                 | Argument             | Default          | Meaning                                                                        |
| -------------------- | -------------------- | ---------------- | ------------------------------------------------------------------------------ |
| `--template`         | `normal\|friendship` | `normal`         | The kind of group to create.                                                   |
| `--with`             | account (64 hex)     | — (none)         | The other side of a friendship. Required for `--template friendship`.          |
| `--max`              | integer              | — (none; no cap) | The most members the group may have, counting the creator. Normal groups only. |
| `--admins-on-create` | switch               | off              | Make both friends Admin (friendships).                                         |
| `--json`             | switch               | off              | Parent flag: print one JSON object instead of a line.                          |

Example — a normal toy group capped at 8 members, and a toy friendship:

```
$ id group create --max 8
$ id group create --template friendship \
    --with fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210
```

Failures: a friendship without `--with` fails the command, exit `1`. A friendship with `--max` fails with `a friendship has two members, so it takes no --max`, exit `1`. An unknown `--template` value is a clap error, exit `2`.

### `group policy`

Shows a group's policy record.

```
id group policy <GROUP>
```

| Flag      | Argument          | Default             | Meaning                                               |
| --------- | ----------------- | ------------------- | ----------------------------------------------------- |
| `<GROUP>` | group ID (64 hex) | required positional | The group.                                            |
| `--json`  | switch            | off                 | Parent flag: print one JSON object instead of a line. |

This subcommand takes no flags of its own.

Example:

```
$ id group policy 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef
```

Failures: an unknown group fails the command, exit `1`.

### `group list`

Lists the groups in the log, one line each, or one JSON array with `--json`.

```
id group list
```

| Flag     | Argument | Default | Meaning                                                          |
| -------- | -------- | ------- | ---------------------------------------------------------------- |
| `--json` | switch   | off     | Parent flag: print one JSON array instead of one line per group. |

This subcommand takes no arguments and no flags of its own.

Example:

```
$ id group list --json
```

Failures: none specific to this subcommand beyond the common exit behaviour.

### `group add`

Adds a member with a role. The author must be a manager of the group.

```
id group add <GROUP> <MEMBER> [--role <ROLE>]
```

| Flag       | Argument                           | Default             | Meaning                                               |
| ---------- | ---------------------------------- | ------------------- | ----------------------------------------------------- |
| `<GROUP>`  | group ID (64 hex)                  | required positional | The group.                                            |
| `<MEMBER>` | account (64 hex)                   | required positional | The account to add.                                   |
| `--role`   | `pull\|read\|write\|manage\|admin` | `read`              | The member's role.                                    |
| `--json`   | switch                             | off                 | Parent flag: print one JSON object instead of a line. |

Example:

```
$ id group add 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef \
    fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210 --role write
```

Failures: an author who is not a manager is refused, exit `1`, and the refused event stays in the log. An unknown `--role` value is a clap error, exit `2`. Granting `admin` requires an Admin author, exit `1` when refused.

### `group remove`

Removes a member. Removing yourself is leaving. The group's default mode is used when `--mode` is omitted.

```
id group remove <GROUP> <MEMBER> [--mode <MODE>] [--successor-kind <KIND>] [--successor <ACCOUNT>]
```

| Flag               | Argument                                                          | Default                               | Meaning                                                                                                                                                                                                                                                                                                                                                                                                      |
| ------------------ | ----------------------------------------------------------------- | ------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `<GROUP>`          | group ID (64 hex)                                                 | required positional                   | The group.                                                                                                                                                                                                                                                                                                                                                                                                   |
| `<MEMBER>`         | account (64 hex)                                                  | required positional                   | The account to remove.                                                                                                                                                                                                                                                                                                                                                                                       |
| `--mode`           | `server-default\|revoke\|transfer\|keep-prior\|reassign-to-group` | the group's configured remove default | Grant-disposition mode. `server-default` is resolved to the group's configured default before the event is recorded. `revoke` ends the removed member's grants. `transfer` reissues the grants to a successor. `keep-prior` retains grants under their original grantor (a re-add starts clean: pre-removal grants do not revive and must be re-issued). `reassign-to-group` re-anchors grants at the group. |
| `--successor-kind` | `member\|admin\|designated\|actor`                                | `member`                              | The successor category for `--mode transfer`: a named member, a group administrator, a designated account, or the account performing the removal. The successor flags require `--mode transfer`.                                                                                                                                                                                                             |
| `--successor`      | account (64 hex)                                                  | — (none)                              | The account for a `member` or `designated` transfer successor. Requires `--mode transfer`.                                                                                                                                                                                                                                                                                                                   |
| `--json`           | switch                                                            | off                                   | Parent flag: print one JSON object instead of a line.                                                                                                                                                                                                                                                                                                                                                        |

Example — remove a toy member and transfer the member's grants to a named successor:

```
$ id group remove 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef \
    fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210 \
    --mode transfer --successor-kind member \
    --successor 1111111111111111111111111111111111111111111111111111111111111111
```

Failures: a mode or successor category the group's policy does not allow is refused, exit `1`, and the refused event stays in the log. Giving `--successor` or `--successor-kind` without `--mode transfer` fails the command, exit `1`. An unknown `--mode` value is a clap error, exit `2`.

### `group role`

Sets a member's role. The subcommand name is `role`. Only an Admin may use it.

```
id group role <GROUP> <MEMBER> <ROLE>
```

| Flag       | Argument                           | Default             | Meaning                                               |
| ---------- | ---------------------------------- | ------------------- | ----------------------------------------------------- |
| `<GROUP>`  | group ID (64 hex)                  | required positional | The group.                                            |
| `<MEMBER>` | account (64 hex)                   | required positional | The member whose role changes.                        |
| `<ROLE>`   | `pull\|read\|write\|manage\|admin` | required positional | Its new role.                                         |
| `--json`   | switch                             | off                 | Parent flag: print one JSON object instead of a line. |

Example:

```
$ id group role 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef \
    fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210 manage
```

Failures: a non-Admin author is refused, exit `1`, and the refused event stays in the log. An unknown role value is a clap error, exit `2`.

### `group join`

Parses an invitation ticket, then always fails. Group admission and network joining are not implemented: no group membership is ever recorded by this command. This is the source behaviour; the top-level help once listed `join` among the things `group` can do (disagreement CLI-9, resolved — it no longer does). The subcommand's own help states the truth in its first line: `Unavailable: group admission and network joining are not implemented yet`.

```
id group join <INVITE>
```

| Flag       | Argument              | Default             | Meaning                                                               |
| ---------- | --------------------- | ------------------- | --------------------------------------------------------------------- |
| `<INVITE>` | ticket or link string | required positional | The pasted ticket or link. It is parsed, and nothing further happens. |
| `--json`   | switch                | off                 | Parent flag. It does not change the outcome: the command still fails. |

Example — the outcome is the same for every well-formed ticket (`<ticket>` stands for one):

```
$ id group join <ticket>
Error: group join is unavailable: invitation parsing works, but admission and network acceptance are not implemented yet; no group membership was recorded
```

(exit `1`.)

Failures: this command has no success path. A malformed ticket fails earlier, with the invitation parse error, exit `1`. A well-formed ticket fails with the `group join is unavailable: ...` message above, exit `1`.

## `delegate`

Grants a delegate a scope in a group, with the group's delegation mode. It prints the grant's event ID; that ID is the `<GRANT>` argument of `revoke`.

```
id delegate <GROUP> <GRANTEE> [--scope <ROLE>] [--redelegate] [--mode <MODE>] [--json]
```

| Flag           | Argument                           | Default             | Meaning                                                                                                                                             |
| -------------- | ---------------------------------- | ------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------- |
| `<GROUP>`      | group ID (64 hex)                  | required positional | The group.                                                                                                                                          |
| `<GRANTEE>`    | account (64 hex)                   | required positional | The account that is granted the scope.                                                                                                              |
| `--scope`      | `pull\|read\|write\|manage\|admin` | `read`              | The scope to grant.                                                                                                                                 |
| `--redelegate` | switch                             | off                 | Let the grantee pass the scope on to others. Only a redelegatable grant extends a chain.                                                            |
| `--mode`       | `keep_prior\|cascade\|transfer`    | — (none)            | The group's delegation mode, set first if it differs. The spellings use underscores. `cascade` is the delegation default in the group policy layer. |
| `--json`       | switch                             | off                 | Print JSON instead of a line.                                                                                                                       |

Example:

```
$ id delegate 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef \
    fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210 \
    --scope write --redelegate
```

Failures: a grant the author is not authorised to make is refused, exit `1`. An unknown `--scope` or `--mode` value is a clap error, exit `2`. Note the mode spellings: `--mode keep-prior` (with a hyphen) is the spelling of `group remove --mode`; `delegate --mode` takes `keep_prior` (with an underscore). The hyphen form here is a clap error, exit `2`.

## `revoke`

Revokes a delegation by its grant's event ID, as `delegate` printed it.

```
id revoke <GROUP> <GRANT> [--json]
```

| Flag      | Argument          | Default             | Meaning                                            |
| --------- | ----------------- | ------------------- | -------------------------------------------------- |
| `<GROUP>` | group ID (64 hex) | required positional | The group.                                         |
| `<GRANT>` | event ID          | required positional | The grant's event ID, as `id delegate` printed it. |
| `--json`  | switch            | off                 | Print JSON instead of a line.                      |

Example:

```
$ id revoke 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef \
    toy-grant-event-id
```

(The grant value above is a toy placeholder; use the event ID a real `delegate` printed.)

Failures: an unknown grant ID fails the command, exit `1`.

## `doc`

Each document action signs one event into the event store (the global `--event-store`) and prints one line, or one JSON object with `--json`. The store stands in for sync: nodes that share it see each other's events. A document is a `document.put` event over the content's hash. Lists print one line per row, or one JSON array with `--json`.

### `doc publish`

Publishes a file as a document, and sets a tag for each `--tag`.

```
id doc publish <FILE> --media-type <TYPE> [--tag <NAME[=VALUE]>]...
```

| Flag           | Argument                                | Default             | Meaning                                                             |
| -------------- | --------------------------------------- | ------------------- | ------------------------------------------------------------------- |
| `<FILE>`       | file path                               | required positional | The file to publish.                                                |
| `--media-type` | media type string, e.g. `text/markdown` | required            | The media type of the file. Omitting it is a clap error, exit `2`.  |
| `--tag`        | `NAME` or `NAME=VALUE`                  | — (none)            | A tag to set on the document; repeatable. The flag name is `--tag`. |
| `--json`       | switch                                  | off                 | Parent flag: print one JSON object instead of a line.               |

Behaviour: prints `publish document=<ID>`, the document's event ID. The file's bytes are hashed, and the event names their hash and size. The bytes are not stored in the event: publish stores them in the blob store under their hash, so `doc content` can fetch them back.

Example:

```
$ id doc publish ./toy-notes.md --media-type text/markdown --tag topic=toy-plans
publish document=toy-document-event-id
```

(The document ID above is a toy placeholder for the event ID a real publish prints.)

Failures: a missing file fails the command, exit `1`. Omitting `--media-type` is a clap error, exit `2`.

### `doc events`

Lists the tags and removal on a document, in causal order.

```
id doc events <DOCUMENT>
```

| Flag         | Argument | Default             | Meaning                                              |
| ------------ | -------- | ------------------- | ---------------------------------------------------- |
| `<DOCUMENT>` | event ID | required positional | The document's event ID, as `publish` printed it.    |
| `--json`     | switch   | off                 | Parent flag: print one JSON object instead of lines. |

Example:

```
$ id doc events toy-document-event-id
```

Failures: an unknown document ID fails the command, exit `1`.

### `doc frontier`

Lists the head of each author's chain: the latest event each author has recorded. It takes no arguments.

```
id doc frontier
```

| Flag     | Argument | Default | Meaning                                              |
| -------- | -------- | ------- | ---------------------------------------------------- |
| `--json` | switch   | off     | Parent flag: print one JSON object instead of lines. |

Example:

```
$ id doc frontier --json
```

Failures: none specific to this subcommand beyond the common exit behaviour.

### `doc history`

Lists a document's events in causal order, its `document.put` first.

```
id doc history <DOCUMENT>
```

| Flag         | Argument | Default             | Meaning                                              |
| ------------ | -------- | ------------------- | ---------------------------------------------------- |
| `<DOCUMENT>` | event ID | required positional | The document's event ID, as `publish` printed it.    |
| `--json`     | switch   | off                 | Parent flag: print one JSON object instead of lines. |

Example:

```
$ id doc history toy-document-event-id
```

Failures: an unknown document ID fails the command, exit `1`.

### `doc content`

Prints a document's blob bytes, by document event ID. It resolves the `document.put` event to the content hash it names and exports the bytes the publish step stored.

```
id doc content <DOCUMENT> [OUTPUT]
```

| Flag         | Argument          | Default             | Meaning                                                            |
| ------------ | ----------------- | ------------------- | ------------------------------------------------------------------ |
| `<DOCUMENT>` | event ID          | required positional | The document's event ID, as `publish` printed it.                  |
| `[OUTPUT]`   | file path, or `-` | `-` (stdout)        | Output path, or `-` for stdout.                                    |
| `--json`     | switch            | off                 | Parent flag (effect on this subcommand's byte output: unverified). |

Example — write a toy document back out to a file, and to stdout:

```
$ id doc content toy-document-event-id ./toy-notes-copy.md
$ id doc content toy-document-event-id
```

Failures: an unknown document ID fails the command, exit `1`. An unwritable output path fails the command, exit `1`.

## `claim`

A claim is a signed `tag.set` event that sets a tag on a subject, where a subject is any event ID, such as a document's. `clear` removes the live claims it names. Claims live in the event store (the global `--event-store`).

Scope: a claim is `shared` (published to peers, the default) or `local` (kept on this node only, never published or synced).

### `claim set`

Sets a tag on a subject.

```
id claim set <SUBJECT> <NAME> [VALUE] [--scope <shared|local>]
```

| Flag        | Argument        | Default             | Meaning                                                                                                                                                                                                                 |
| ----------- | --------------- | ------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `<SUBJECT>` | event ID        | required positional | The subject's event ID.                                                                                                                                                                                                 |
| `<NAME>`    | tag name        | required positional | The tag name. A `local:` prefix is shorthand for `--scope local` when `--scope` is absent.                                                                                                                              |
| `[VALUE]`   | string          | — (none)            | The tag value. Without one, the tag is bare.                                                                                                                                                                            |
| `--scope`   | `shared\|local` | `shared`            | Whether the claim is shared with peers or stays on this node. An explicit `--scope` wins over a `local:` name prefix: with `--scope shared`, a name written `local:NAME` is a shared claim whose name keeps the prefix. |
| `--json`    | switch          | off                 | Parent flag: print one JSON object instead of a line.                                                                                                                                                                   |

Example — a shared toy claim, and the same claim kept local by the prefix shorthand:

```
$ id claim set toy-document-event-id topic toy-plans
$ id claim set toy-document-event-id local:topic toy-plans
```

Failures: none specific to this subcommand beyond the common exit behaviour.

### `claim clear`

Clears the live claims with this name on a subject, whoever set them.

```
id claim clear <SUBJECT> <NAME>
```

| Flag        | Argument | Default             | Meaning                                               |
| ----------- | -------- | ------------------- | ----------------------------------------------------- |
| `<SUBJECT>` | event ID | required positional | The subject's event ID.                               |
| `<NAME>`    | tag name | required positional | The tag name.                                         |
| `--json`    | switch   | off                 | Parent flag: print one JSON object instead of a line. |

Behaviour: clearing stops future replication. It cannot unsend copies peers already hold.

Example:

```
$ id claim clear toy-document-event-id topic
```

Failures: none specific to this subcommand beyond the common exit behaviour.

### `claim search`

Lists the live claims with this name, and value if given, on any subject.

```
id claim search <NAME> [VALUE]
```

| Flag      | Argument | Default             | Meaning                                                                         |
| --------- | -------- | ------------------- | ------------------------------------------------------------------------------- |
| `<NAME>`  | tag name | required positional | The tag name.                                                                   |
| `[VALUE]` | string   | — (none)            | The tag value to match as a whole. Without one, any value and a bare tag match. |
| `--json`  | switch   | off                 | Parent flag: print one JSON array instead of lines.                             |

Example:

```
$ id claim search topic toy-plans --json
```

Failures: none specific to this subcommand beyond the common exit behaviour.

### `claim scope`

Changes the scope of this node's claims with this name on a subject.

```
id claim scope <SUBJECT> <NAME> <shared|local> [--yes]
```

| Flag        | Argument        | Default             | Meaning                                                                                                                     |
| ----------- | --------------- | ------------------- | --------------------------------------------------------------------------------------------------------------------------- |
| `<SUBJECT>` | event ID        | required positional | The subject's event ID.                                                                                                     |
| `<NAME>`    | tag name        | required positional | The tag name.                                                                                                               |
| `<SCOPE>`   | `shared\|local` | required positional | The target scope. It is explicit, so it wins over any `local:` prefix shorthand used when a claim was set.                  |
| `--yes`     | switch          | off                 | Confirm publishing local claims as shared. Required when the move would publish a local claim: publishing cannot be undone. |
| `--json`    | switch          | off                 | Parent flag: print one JSON object instead of a line.                                                                       |

Behaviour: moving to `local` stops future replication but cannot unsend copies peers already hold. Moving to `shared` publishes this node's local claims and cannot be undone.

Example:

```
$ id claim scope toy-document-event-id topic shared --yes
```

Failures: moving to `shared` when a local claim would be published, without `--yes`, is refused, exit `1`. An unknown scope value is a clap error, exit `2`.

### `claim review`

Reviews every live claim visible to this node, with its scope. It takes no arguments.

```
id claim review
```

| Flag     | Argument | Default | Meaning                                              |
| -------- | -------- | ------- | ---------------------------------------------------- |
| `--json` | switch   | off     | Parent flag: print one JSON object instead of lines. |

Example:

```
$ id claim review
```

Failures: none specific to this subcommand beyond the common exit behaviour.

### `claim list`

Lists the live claims on a subject.

```
id claim list <SUBJECT>
```

| Flag        | Argument | Default             | Meaning                                             |
| ----------- | -------- | ------------------- | --------------------------------------------------- |
| `<SUBJECT>` | event ID | required positional | The subject's event ID.                             |
| `--json`    | switch   | off                 | Parent flag: print one JSON array instead of lines. |

Example:

```
$ id claim list toy-document-event-id
```

Failures: none specific to this subcommand beyond the common exit behaviour.

## `talk`

A talk page is the signed discussion attached to one object, where an object is any event ID this node holds, such as a document's or a note's. Each post is a signed `talk.post.v1` event in the event store (the global `--event-store`), attributed to this node's key. The record is the #5834 design's: a post is never edited, and a correction is a new post that supersedes the old one.

Visibility is the only gate. This node may post on, and list, the talk page of an object it can see: an event its store has applied, and, when that event belongs to a group, one whose group this node's key is a member of. An object it cannot see and an object it does not hold are refused with the same message, `the object is not visible to this node`, so the refusal never confirms that an object exists.

Post IDs are content IDs in their text form, `content/raw-blake3-256.<HEX>`. `--reply-to` and `--supersedes` take them as `talk list` prints them.

### `talk post`

Posts on an object's talk page, signed by this node.

```
id talk post <OBJECT> <BODY> [--reply-to <POST>] [--supersedes <POST>]
```

| Flag           | Argument | Default             | Meaning                                                           |
| -------------- | -------- | ------------------- | ----------------------------------------------------------------- |
| `<OBJECT>`     | event ID | required positional | The object's event ID.                                            |
| `<BODY>`       | text     | required positional | The post text, at most 64 KiB.                                    |
| `--reply-to`   | post ID  | — (none)            | The post this one replies to. It must be on the same talk page.   |
| `--supersedes` | post ID  | — (none)            | This node's earlier post on the same page that this one corrects. |
| `--json`       | switch   | off                 | Parent flag: print one JSON object instead of a line.             |

Behaviour: prints `post object= post= author= authored_at= [reply_to=] [supersedes=] event= body=`. A superseded post is not changed: `talk list` keeps listing it and marks it `superseded_by=` the correction.

Example — a post, then a correction of it:

```
$ id talk post toy-document-event-id "teh wording"
$ id talk post toy-document-event-id "the wording" --supersedes toy-post-id
```

Failures: an object this node cannot see, a `--reply-to` or `--supersedes` post that is not on the page, a `--supersedes` post by another author, and an empty or oversized body are refused, exit `1`.

### `talk list`

Lists every post on an object's talk page, oldest first by the author's clock.

```
id talk list <OBJECT>
```

| Flag       | Argument | Default             | Meaning                                             |
| ---------- | -------- | ------------------- | --------------------------------------------------- |
| `<OBJECT>` | event ID | required positional | The object's event ID.                              |
| `--json`   | switch   | off                 | Parent flag: print one JSON array instead of lines. |

Behaviour: one line per post, with the fields `talk post` prints and no command word. A post that a later post by the same author supersedes carries `superseded_by=<POST>`; a display may collapse it, and it stays listed. A post synced from a peer is listed only if its signature verifies, its author is the event's signer and, for a group's object, its author is a member of that group.

Example:

```
$ id talk list toy-document-event-id --json
```

Failures: an object this node cannot see is refused, exit `1`.

### Thread steps

A thread in a conversation group can end in a resolution, and a change to an object can be proposed out of it (#7849). Each step is a signed event in the group, written only by a member of it: `propose` (`chat.proposal.v1`), `resolve` (`chat.resolution.v1`), `reject` (`chat.proposal-rejection.v1`) and `withdraw` (`chat.proposal-withdrawal.v1`). `accept` is not an event of its own: it is the note revision that cites the proposal. Whether a step counts is decided on read: a member may resolve a thread, an object's writer may reject a proposal on it, and only the proposer may withdraw one. A key writes a note when it created it or this store holds a revision of it by that key.

Each step is one terminal line, and the command writes its arguments as that line, checks it with the same field rules the fold applies, and prints it back as `line=`. An object is `<object-id>/<author-key>`, a blob is `<hash>/<author-key>`, optionally followed by `@<hint>`, and every other ID is 64 hex characters.

#### `talk propose`

```
id talk propose <GROUP> <OBJECT> --base <REVISION> --content <BLOB> --thread <THREAD> [--supersedes <PROPOSAL>]
```

| Flag           | Argument | Default             | Meaning                                                        |
| -------------- | -------- | ------------------- | -------------------------------------------------------------- |
| `<GROUP>`      | event ID | required positional | The conversation group's ID.                                   |
| `<OBJECT>`     | object   | required positional | The object to change.                                          |
| `--base`       | event ID | required            | The exact revision the proposal is against.                    |
| `--content`    | blob     | required            | The full proposed content.                                     |
| `--thread`     | event ID | required            | The root of the thread the proposal came out of.               |
| `--supersedes` | event ID | — (none)            | This node's earlier proposal against the same object and base. |
| `--json`       | switch   | off                 | Parent flag: print one JSON object instead of a line.          |

Behaviour: prints `propose group= kind= event= line=`. The proposal's ID is `event=`.

#### `talk resolve`

```
id talk resolve <GROUP> <THREAD> (--edit <EDIT> --object <OBJECT> | --decision <BLOB> | --no-action)
```

| Flag          | Argument | Default             | Meaning                                               |
| ------------- | -------- | ------------------- | ----------------------------------------------------- |
| `<GROUP>`     | event ID | required positional | The conversation group's ID.                          |
| `<THREAD>`    | event ID | required positional | The thread's root event ID.                           |
| `--edit`      | event ID | — (none)            | The revision the thread produced. Needs `--object`.   |
| `--object`    | object   | — (none)            | The object the edit is on.                            |
| `--decision`  | blob     | — (none)            | The decision text.                                    |
| `--no-action` | switch   | off                 | The thread was considered and nothing changes.        |
| `--json`      | switch   | off                 | Parent flag: print one JSON object instead of a line. |

Behaviour: prints `resolve group= kind= event= line=`. Concurrent resolutions of one thread are all kept; `talk threads` shows them side by side and chooses none.

#### `talk reject` and `talk withdraw`

```
id talk reject <GROUP> <PROPOSAL> --because <BLOB>
id talk withdraw <GROUP> <PROPOSAL>
```

Behaviour: each prints `<step> group= kind= event= line=`. A rejection counts when its signer writes the proposal's object, and a withdrawal when its signer made the proposal.

#### `talk accept`

```
id talk accept <GROUP> <PROPOSAL> <FILE> --media-type <TYPE> [--parent <REVISION>]...
```

| Flag           | Argument   | Default             | Meaning                                               |
| -------------- | ---------- | ------------------- | ----------------------------------------------------- |
| `<GROUP>`      | event ID   | required positional | The conversation group's ID.                          |
| `<PROPOSAL>`   | event ID   | required positional | The proposal to accept, on a note.                    |
| `<FILE>`       | path       | required positional | The revision's content, normally the proposal's.      |
| `--media-type` | media type | required            | The media type of the file.                           |
| `--parent`     | event ID   | the proposal's base | A revision the edit builds on. Repeat for a merge.    |
| `--json`       | switch     | off                 | Parent flag: print one JSON object instead of a line. |

Behaviour: adds a revision to the proposal's note that cites the proposal, and prints `accept group= proposal= note= revision= matches_proposal= line=`, where `matches_proposal` says whether the file's bytes are the proposal's content. The proposal is accepted when this node writes the note.

#### `talk threads`

```
id talk threads <GROUP>
```

Behaviour: prints the group's threads as this node may see them: one line per resolved thread, or a heading and one indented line per head for concurrent resolutions, then one line per proposal with its state (`open`, `accepted by edit`, `rejected by`, `withdrawn by`, `superseded by`). An open proposal on a note whose base is no longer one of the note's heads ends `(stale: its base is no longer a head of the object)`; it can still be accepted. With `--json` it prints one JSON array of those lines.

Failures: a node that is not a member of the group is refused by every thread step, exit `1`.

## `writer-sets`

Writer sets stored in the event store (the global `--event-store`). A writer set is a signed `idset.writer` event whose body the store retains at ingest. `show` prints the stored object, `expand` runs the view projection over the store's resolved group state at the set's own publishing frontier, and `discover` lists every stored set for a scope with the writers each expands to. `--resolution` names the resolution snapshot the projection is pinned under; without one, the zero hash names the store's current resolved state.

### `writer-sets show`

Shows one stored writer set: its ID, version, owner, scope, and the counts of its admission entries and exclusions.

```
id writer-sets show <EVENT>
```

| Flag      | Argument | Default             | Meaning                                              |
| --------- | -------- | ------------------- | ---------------------------------------------------- |
| `<EVENT>` | event ID | required positional | The publishing event's ID.                           |
| `--json`  | switch   | off                 | Parent flag: print one JSON object instead of lines. |

Example:

```
$ id writer-sets show toy-writer-set-event-id
```

Failures: an unknown event ID fails the command, exit `1`.

### `writer-sets expand`

Expands one stored writer set at its publishing frontier: whether the expansion is public, and the explicit authors it admits after exclusions.

```
id writer-sets expand <EVENT> [--resolution <HEX>] [--retro]
```

| Flag           | Argument   | Default                 | Meaning                                                                                                                           |
| -------------- | ---------- | ----------------------- | --------------------------------------------------------------------------------------------------------------------------------- |
| `<EVENT>`      | event ID   | required positional     | The publishing event's ID.                                                                                                        |
| `--resolution` | hex string | — (none; the zero hash) | The resolution snapshot to pin the projection under, as hex. Without one, the zero hash names the store's current resolved state. |
| `--retro`      | switch     | off                     | Expand retroactively: only entries marked include-history contribute. Exclusions always apply.                                    |
| `--json`       | switch     | off                     | Parent flag: print one JSON object instead of lines.                                                                              |

Example:

```
$ id writer-sets expand toy-writer-set-event-id --retro
```

Failures: an unknown event ID fails the command, exit `1`.

### `writer-sets discover`

Discovers stored writer sets by scope: one row per set, with the writers it expands to.

```
id writer-sets discover [--scope <SCOPE>] [--resolution <HEX>]
```

| Flag           | Argument   | Default                 | Meaning                                                                                                                           |
| -------------- | ---------- | ----------------------- | --------------------------------------------------------------------------------------------------------------------------------- |
| `--scope`      | string     | — (none; all sets)      | The scope (subject) to match, exactly. Without `--scope`, every stored set matches.                                               |
| `--resolution` | hex string | — (none; the zero hash) | The resolution snapshot to pin each expansion under, as hex. Without one, the zero hash names the store's current resolved state. |
| `--json`       | switch     | off                     | Parent flag: print one JSON array instead of lines.                                                                               |

Example:

```
$ id writer-sets discover --scope toy-metrics
```

Failures: none specific to this subcommand beyond the common exit behaviour.

## `id`

Prints the local node's public ID. The command name collides with the binary name by design: the invocation is `id id`. The node ID is derived from the keypair and is what remote nodes connect to.

```
id id [--client]
```

| Flag       | Argument | Default | Meaning                                                                                                                                                                                                                                       |
| ---------- | -------- | ------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `--client` | switch   | off     | Print the client identity instead of the server identity. The client identity is the one this machine uses when connecting to other nodes; a server owner adds it with `id serve --allow-node <ID>` so this machine may write to that server. |

Example:

```
$ id id
$ id id --client
```

Each prints one node ID (64 hex characters) on stdout.

Failures: none specific to this command beyond the common exit behaviour.

## `key`

Exports or imports a local identity key. Passphrases are always hidden prompts, never flags: no key subcommand accepts a passphrase on the command line. A passphrase must be at least `8` bytes; the exact refusal is `passphrase must be at least 8 bytes`. A mismatched confirmation is refused with `passphrases do not match`. A key file that is not 32 bytes is refused with `<path> is not a 32-byte key file`.

### `key export`

Exports the server identity (or, with `--client`, the client identity) to an encrypted file. The passphrase is entered at a hidden prompt.

```
id key export <OUTPUT> [--client]
```

| Flag       | Argument  | Default             | Meaning                                                    |
| ---------- | --------- | ------------------- | ---------------------------------------------------------- |
| `<OUTPUT>` | file path | required positional | Destination for the encrypted key envelope.                |
| `--client` | switch    | off                 | Export the client identity instead of the server identity. |

Example:

```
$ id key export ./toy-server-key-envelope
Passphrase: (hidden prompt)
```

Failures: a passphrase under 8 bytes, or a confirmation that does not match, fails the command, exit `1`, and no envelope is written by that attempt.

### `key import`

Imports an encrypted key envelope as the server or client identity. The passphrase is entered at a hidden prompt.

```
id key import <INPUT> [--client] [--replace]
```

| Flag        | Argument  | Default             | Meaning                                                       |
| ----------- | --------- | ------------------- | ------------------------------------------------------------- |
| `<INPUT>`   | file path | required positional | The encrypted key envelope to import.                         |
| `--client`  | switch    | off                 | Import as the client identity instead of the server identity. |
| `--replace` | switch    | off                 | Replace an existing identity key file atomically.             |

Example:

```
$ id key import ./toy-server-key-envelope --replace
Passphrase: (hidden prompt)
```

Failures: a wrong passphrase fails the command, exit `1`. An existing identity key file without `--replace` fails the command, exit `1`.

### `key export-seed`

Prints the server identity (or, with `--client`, the client identity) as a 24-word seed phrase. The phrase is the key: anyone holding it holds the identity. It is printed once, never written to a file by this command, and never taken on the command line.

```
id key export-seed [--client]
```

| Flag       | Argument | Default | Meaning                                                    |
| ---------- | -------- | ------- | ---------------------------------------------------------- |
| `--client` | switch   | off     | Export the client identity instead of the server identity. |

Example:

```
$ id key export-seed --client
```

The output is the 24-word phrase on stdout, once. This chapter does not print an example phrase: a seed phrase is a secret value, and only its shape (24 words, printed once, no file written) is documented here.

Failures: none specific to this subcommand beyond the common exit behaviour.

### `key import-seed`

Restores the server or client identity from a 24-word seed phrase entered at a hidden prompt, never on the command line.

```
id key import-seed [--client] [--replace]
```

| Flag        | Argument | Default | Meaning                                                       |
| ----------- | -------- | ------- | ------------------------------------------------------------- |
| `--client`  | switch   | off     | Import as the client identity instead of the server identity. |
| `--replace` | switch   | off     | Replace an existing identity key file atomically.             |

Example:

```
$ id key import-seed --replace
Seed phrase: (hidden prompt)
```

Failures: a phrase that does not decode to a key fails the command, exit `1`. An existing identity key file without `--replace` fails the command, exit `1`.

## `keyset`

Shows or changes this identity's keyset: its admin key and the device keys allowed to act for it. Each subcommand acts on the server identity, or the client identity with `--client`, and signs with that identity's key, which must be an active admin key of the keyset to change it.

First use: the first keyset use migrates a single-key account by writing a genesis record signed by the existing key. The identity ID does not change.

Every keyset subcommand takes `--client` except `keyset recovery status`, which has no `--client` flag at all.

### `keyset show`

Prints the identity ID, admin keys and device keys.

```
id keyset show [--client]
```

| Flag       | Argument | Default | Meaning                                                    |
| ---------- | -------- | ------- | ---------------------------------------------------------- |
| `--client` | switch   | off     | Show the client identity's keyset instead of the server's. |

Example:

```
$ id keyset show
```

Failures: none specific to this subcommand beyond the common exit behaviour, apart from the first-use migration described above, which is not a failure.

### `keyset enrol`

Enrols another device key. The device's 32-byte key file is read to countersign the enrolment, proving the device holds the key.

```
id keyset enrol <DEVICE_KEY_FILE> --label <NAME> [--client]
```

| Flag                | Argument  | Default             | Meaning                                                                                  |
| ------------------- | --------- | ------------------- | ---------------------------------------------------------------------------------------- |
| `<DEVICE_KEY_FILE>` | file path | required positional | The new device's key file (the `.iroh-key` it was created with).                         |
| `--label`           | string    | required            | A name for the device, shown by `id keyset show`. Omitting it is a clap error, exit `2`. |
| `--client`          | switch    | off                 | Change the client identity's keyset instead of the server's.                             |

Example:

```
$ id keyset enrol ./toy-device.iroh-key --label toy-laptop
```

Failures: a device key file that is not 32 bytes is refused with `<path> is not a 32-byte key file`, exit `1`. A signing key that is not an active admin key of the keyset is refused, exit `1`.

### `keyset revoke`

Revokes a device key, given as its hex public key. Records it signed before the revocation stay valid history.

```
id keyset revoke <DEVICE_KEY> [--reason <TEXT>] [--client]
```

| Flag           | Argument       | Default             | Meaning                                                      |
| -------------- | -------------- | ------------------- | ------------------------------------------------------------ |
| `<DEVICE_KEY>` | hex public key | required positional | The device's hex public key, as `id keyset show` prints it.  |
| `--reason`     | text           | `revoked`           | Why it was revoked, kept in the record.                      |
| `--client`     | switch         | off                 | Change the client identity's keyset instead of the server's. |

Example:

```
$ id keyset revoke fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210 \
    --reason "toy device retired"
```

Failures: an unknown device key fails the command, exit `1`. A signing key that is not an active admin key of the keyset is refused, exit `1`.

### `keyset rotate`

Hands admin power to another key. The new key's 32-byte key file is read to countersign the rotation; afterwards this key can no longer change the keyset.

```
id keyset rotate <NEW_ADMIN_KEY_FILE> [--client]
```

| Flag                   | Argument  | Default             | Meaning                                                      |
| ---------------------- | --------- | ------------------- | ------------------------------------------------------------ |
| `<NEW_ADMIN_KEY_FILE>` | file path | required positional | The new admin key's key file.                                |
| `--client`             | switch    | off                 | Change the client identity's keyset instead of the server's. |

Example:

```
$ id keyset rotate ./toy-new-admin.iroh-key
```

Failures: a new admin key file that is not 32 bytes is refused with `<path> is not a 32-byte key file`, exit `1`. A signing key that is not an active admin key of the keyset is refused, exit `1`.

### `keyset guardians`

Names the guardians who can approve a recovery of this identity, and how many must approve.

```
id keyset guardians <GUARDIANS>... --threshold <N> [--client]
```

| Flag             | Argument                       | Default                           | Meaning                                                                                   |
| ---------------- | ------------------------------ | --------------------------------- | ----------------------------------------------------------------------------------------- |
| `<GUARDIANS>...` | identity IDs (hex public keys) | required positional, at least one | Each guardian's identity ID.                                                              |
| `--threshold`    | integer                        | required                          | How many of the guardians must approve a recovery. Omitting it is a clap error, exit `2`. |
| `--client`       | switch                         | off                               | Change the client identity's keyset instead of the server's.                              |

Behaviour: adding guardians or raising the threshold takes effect at once. Removing a guardian or lowering the threshold waits out the recovery delay, so a stolen admin key cannot swap in guardians of its own.

Example:

```
$ id keyset guardians \
    fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210 \
    1111111111111111111111111111111111111111111111111111111111111111 \
    --threshold 2
```

Failures: a signing key that is not an active admin key of the keyset is refused, exit `1`.

### `keyset checkpoint`

Checkpoints the current keyset head with this identity's key, which must be an active device key. The head becomes final here: a fork that later arrives can no longer displace it.

```
id keyset checkpoint [--client]
```

| Flag       | Argument | Default | Meaning                                                          |
| ---------- | -------- | ------- | ---------------------------------------------------------------- |
| `--client` | switch   | off     | Checkpoint the client identity's keyset instead of the server's. |

Example:

```
$ id keyset checkpoint
```

Failures: a signing key that is not an active device key is refused, exit `1`.

### `keyset recovery`

Guardian recovery of an identity whose keys are all lost. Records go through a recovery relay file, which attests when it first saw each one; share that file among the devices and guardians taking part. `--relay` is required on all six recovery subcommands; omitting it is a clap error, exit `2`. Each recovery subcommand signs with this device's identity key, or the client key with `--client` — except `status`, which takes no `--client`.

#### `keyset recovery publish`

Copies this identity's keyset records into the relay, so a successor can pin its head and its approvals can be checked. Owners and guardians both publish.

```
id keyset recovery publish --relay <FILE> [--client]
```

| Flag       | Argument  | Default  | Meaning                                                       |
| ---------- | --------- | -------- | ------------------------------------------------------------- |
| `--relay`  | file path | required | The recovery relay file.                                      |
| `--client` | switch    | off      | Publish the client identity's keyset instead of the server's. |

Example:

```
$ id keyset recovery publish --relay ./toy-relay.json
```

Failures: an unwritable relay file fails the command, exit `1`.

#### `keyset recovery request`

From a new device, asks the guardians of an identity to recover it. This device's key becomes the identity's admin and device key once enough guardians approve and the recovery delay has passed.

```
id keyset recovery request <IDENTITY> --relay <FILE> [--reason <TEXT>] [--client]
```

| Flag         | Argument        | Default             | Meaning                                                      |
| ------------ | --------------- | ------------------- | ------------------------------------------------------------ |
| `<IDENTITY>` | hex identity ID | required positional | The identity to recover (its hex identity ID).               |
| `--relay`    | file path       | required            | The recovery relay file.                                     |
| `--reason`   | text            | `lost every key`    | Why, shown to every device and guardian told of the request. |
| `--client`   | switch          | off                 | Sign with the client key instead of the server key.          |

The command prints the request ID; that ID is the `<REQUEST>` argument of `approve`, `cancel`, `status` and `complete`.

Example:

```
$ id keyset recovery request \
    fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210 \
    --relay ./toy-relay.json
```

Failures: an identity with no keyset records in the relay fails the command, exit `1`.

#### `keyset recovery approve`

As a guardian, approves a recovery request. Also publishes this guardian identity's keyset, so the approval can be checked.

```
id keyset recovery approve <REQUEST> --relay <FILE> [--client]
```

| Flag        | Argument   | Default             | Meaning                                                 |
| ----------- | ---------- | ------------------- | ------------------------------------------------------- |
| `<REQUEST>` | request ID | required positional | The request ID, as `request` printed it.                |
| `--relay`   | file path  | required            | The recovery relay file.                                |
| `--client`  | switch     | off                 | Approve as the client identity instead of the server's. |

Example:

```
$ id keyset recovery approve toy-request-id --relay ./toy-relay.json
```

Failures: a signer who is not a named guardian of the identity is refused, exit `1`. An unknown request ID fails the command, exit `1`.

#### `keyset recovery cancel`

Cancels a recovery request with any surviving admin or device key of the identity being recovered.

```
id keyset recovery cancel <REQUEST> --relay <FILE> [--client]
```

| Flag        | Argument   | Default             | Meaning                                             |
| ----------- | ---------- | ------------------- | --------------------------------------------------- |
| `<REQUEST>` | request ID | required positional | The request ID.                                     |
| `--relay`   | file path  | required            | The recovery relay file.                            |
| `--client`  | switch     | off                 | Sign with the client key instead of the server key. |

Example:

```
$ id keyset recovery cancel toy-request-id --relay ./toy-relay.json
```

Failures: a signer holding no surviving admin or device key of the identity is refused, exit `1`. An unknown request ID fails the command, exit `1`.

#### `keyset recovery status`

Shows where a recovery request stands. This is the one keyset subcommand with no `--client` flag; passing `--client` is a clap error, exit `2`.

```
id keyset recovery status <REQUEST> --relay <FILE>
```

| Flag        | Argument   | Default             | Meaning                  |
| ----------- | ---------- | ------------------- | ------------------------ |
| `<REQUEST>` | request ID | required positional | The request ID.          |
| `--relay`   | file path  | required            | The recovery relay file. |

Example:

```
$ id keyset recovery status toy-request-id --relay ./toy-relay.json
```

Failures: an unknown request ID fails the command, exit `1`.

#### `keyset recovery complete`

On the requesting device, completes a recovery that is ready by relaying the rotation to the successor keys.

```
id keyset recovery complete <REQUEST> --relay <FILE> [--client]
```

| Flag        | Argument   | Default             | Meaning                                             |
| ----------- | ---------- | ------------------- | --------------------------------------------------- |
| `<REQUEST>` | request ID | required positional | The request ID.                                     |
| `--relay`   | file path  | required            | The recovery relay file.                            |
| `--client`  | switch     | off                 | Sign with the client key instead of the server key. |

Example:

```
$ id keyset recovery complete toy-request-id --relay ./toy-relay.json
```

Failures: a request that is not ready — too few guardian approvals, or the recovery delay has not passed — fails the command, exit `1`. An unknown request ID fails the command, exit `1`.

## `peers`

Discovers and lists known peers. It discovers other `id` servers via gossip-based networking, RPC queries to a running serve instance, or both, and supports depth-based recursive crawling to find peers-of-peers.

```
id peers [NODE] [OPTIONS]
```

| Flag                     | Argument                 | Default                                                          | Meaning                                                                                                                                                                                                                                                                                                                                                                                                      |
| ------------------------ | ------------------------ | ---------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `[NODE]`                 | 64 hex characters        | — (none; the local serve)                                        | Remote node ID to query. When set, queries this specific node instead of the local serve instance.                                                                                                                                                                                                                                                                                                           |
| `--gossip`               | switch                   | off                                                              | Gossip-only discovery: join the topic and collect announcements, bypassing RPC entirely. Useful when no local serve is running.                                                                                                                                                                                                                                                                              |
| `--rpc`                  | switch                   | off                                                              | RPC-only discovery: query the server's `ListPeers` RPC. Fails if no local serve is running, unless a node ID is given.                                                                                                                                                                                                                                                                                       |
| `--depth`                | integer, `0..`           | `1`                                                              | Recursive depth for peer-of-peer crawling. `0` = no crawl, `1` = query direct peers, `N` = recursively query up to `N` levels deep. A negative value fails: clap's value parser rejects it (the handler also refuses a negative depth).                                                                                                                                                                      |
| `--max-peers`            | integer                  | `1000`                                                           | Maximum total peers to discover. A hard cap to prevent unbounded crawling.                                                                                                                                                                                                                                                                                                                                   |
| `--timeout`              | integer (seconds)        | `30`                                                             | Per-crawl timeout in seconds.                                                                                                                                                                                                                                                                                                                                                                                |
| `--bootstrap`            | comma-separated node IDs | — (none)                                                         | Node IDs for gossip bootstrapping, supplementary to DHT auto-discovery. These nodes are contacted first when joining the gossip topic.                                                                                                                                                                                                                                                                       |
| `--topic`                | string                   | from `defaults.json`, else the constant `id-peer-discovery-v1`   | Gossip topic name for peer discovery. Use a custom topic for private networks.                                                                                                                                                                                                                                                                                                                               |
| `--topic-secret`         | string                   | from `defaults.json`, else the constant `id-public-discovery-v1` | Shared secret for topic access control, used to encrypt DHT bootstrap records. The value is never echoed.                                                                                                                                                                                                                                                                                                    |
| `--no-default-bootstrap` | switch                   | off                                                              | Skip default bootstrap node IDs from `defaults.json`; only CLI-provided `--bootstrap` nodes are used.                                                                                                                                                                                                                                                                                                        |
| `--no-default-topic`     | switch                   | off                                                              | Skip the topic and secret from `defaults.json`; fall back to the hardcoded constant values.                                                                                                                                                                                                                                                                                                                  |
| `--no-relay`             | switch                   | off                                                              | Disable relay servers and use direct connections only.                                                                                                                                                                                                                                                                                                                                                       |
| `--no-mdns`              | switch                   | off                                                              | Disable mDNS local peer discovery, which is on by default.                                                                                                                                                                                                                                                                                                                                                   |
| `--abuse`                | switch                   | off                                                              | Show the local serve instance's per-peer abuse counters instead of listing peers: frames and bytes admitted by the abuse evaluator, split by direction, with the policy rates shown alongside. Read-only. Local serve only; cannot be combined with `--gossip` or a remote `[NODE]`. The counters cover world-protocol traffic only: a serve running without `--world` records none and the report is empty. |

Behaviour: the default mode is RPC-then-gossip: try RPC (query the local serve), then gossip as fallback. `--rpc` and `--gossip` each pin one mode. The topic and topic-secret defaults follow the same resolution as `serve`: file defaults from `defaults.json` are consulted first and the named constants are the fallback, which is why `--no-default-topic` exists (the same pattern as disagreement CLI-6 for `serve`).

Examples — list peers from the local serve, crawl three levels deep, and discover directly by gossip on a toy private topic:

```
$ id peers
$ id peers --depth 3
$ id peers --gossip --topic toy-net
$ id peers --abuse
```

Failures: an invalid `[NODE]` fails with `invalid node ID: must be 64 hex characters`, exit `1`. `--rpc` with no local serve running and no node ID fails the command, exit `1`. `--abuse` combined with `--gossip` or a remote `[NODE]` fails the command, exit `1`. A negative `--depth` is a clap error, exit `2`.
