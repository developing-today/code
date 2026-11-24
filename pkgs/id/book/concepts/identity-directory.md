# Identity, the directory and groups

## Two identities per node

Every node holds two identities: a server identity and a client identity. `id id` prints the server identity; `id id --client` prints the client identity. Each identity is an Ed25519 keypair in its own key file. The file names are constants from `id-core`: `KEY_FILE = ".iroh-key"` for the server key and `CLIENT_KEY_FILE = ".iroh-key-client"` for the client key, both relative to the data directory. The node ID that other commands take as a remote target is the 64-hex form of the identity key; parsers reject a value that is not 64 hex characters with `invalid node ID: must be 64 hex characters`.

Almost every identity command takes `--client` to act on the client identity instead of the server identity. The one recorded exception is `id keyset recovery status`, which takes no `--client` flag (CLI survey §1.8).

## Keys: export, import, seed phrases

`id key` manages the raw key files. Passphrases are always hidden prompts, never flags. The commands are:

- `id key export <OUTPUT>` — write the key to a file; `--client` selects the client key.
- `id key import <INPUT>` — read a key from a file; `--client` selects the client key, `--replace` permits overwriting an existing key.
- `id key export-seed` — print a 24-word seed phrase once; it writes no file.
- `id key import-seed` — restore a key from a seed phrase; `--client` and `--replace` apply.

The failure shapes are exact: a short passphrase fails with `passphrase must be at least {MIN_PASSPHRASE_BYTES} bytes` (the placeholder is shown here in backticks because the value is substituted at run time), mismatched passphrases fail with `passphrases do not match`, and a file that is not a 32-byte key fails with the message that the path `is not a 32-byte key file`. The seed phrase mechanism is the `seed_phrase` module in `id-core`. The local key envelope — the identity key wrapped under a passphrase — is `key_envelope` in `id-core`, with a design record dated 2026-11-04.

## Keysets and guardians

A keyset turns one identity into a set of device keys under an authorization log. Keyset records live in JSONL files: `.id-keyset.jsonl` for the server identity and `.id-keyset-client.jsonl` for the client identity. Every `id keyset` subcommand takes `--client` except `recovery status`. The commands are `show`, `enrol <DEVICE_KEY_FILE> --label <NAME>` (the label is required), `revoke <DEVICE_KEY>` with `--reason <TEXT>` defaulting to `revoked`, `rotate <NEW_ADMIN_KEY_FILE>`, `guardians <GUARDIANS>... --threshold <N>` (the threshold is required), and `checkpoint`.

First keyset use migrates a single-key account through a genesis record. The identity ID does not change in the migration.

Guardians are the recovery path. A guardian set with a threshold is recorded by `id keyset guardians`. Recovery then runs as a six-step exchange over a relay file; `--relay <FILE>` is required on all six subcommands:

- `id keyset recovery publish --relay <FILE>` — publish the recovery material.
- `id keyset recovery request <IDENTITY> --relay <FILE>` — open a request; `--reason <TEXT>` defaults to `lost every key`.
- `id keyset recovery approve <REQUEST> --relay <FILE>` — a guardian approves.
- `id keyset recovery cancel <REQUEST> --relay <FILE>` — cancel the request.
- `id keyset recovery status <REQUEST> --relay <FILE>` — read the request state.
- `id keyset recovery complete <REQUEST> --relay <FILE>` — finish the recovery.

The library code is in `id-directory`: `keyset`, `keyset_file`, `recovery`, `recovery_flow`, `recovery_relay`, `recovery_view` and `restored_key`. The fuller ceremony designs — guardian social recovery record formats and successor-key authority (#2382), the keyset authorization log ADR (#2380), and the delegation and cross-signing ceremony — are design records carried in [Design: identity, directory, groups and delegation](../design/identity-directory-groups.md).

## The directory

The directory is the home-server account layer, in the crate `id-directory` (split step 6b, #4409). Its core type is `Directory`, with methods `new`, `open`, `replay`, `replay_read_only`, `append`, `sign_up`, `sign_up_keyed`, `add_key`, `remove_key`, `confirm_email`, `account_for` and `verify_account`, among others named in the library survey.

The directory persists as a journal: `directory.jsonl`, one JSON `DirectoryEntry` per line, in the server-config directory. Replay runs entries through the same `apply` path as live changes. The journal has 17 entry kinds: `account_signed_up`, `account_keyed`, `account_verified`, `account_key_added`, `account_key_removed`, `account_email_confirmed`, `account_email_removed`, `group_created`, `group_updated`, `group_visibility`, `group_deleted`, `member_set`, `tier_set`, `friend_requested`, `friend_accepted`, `friend_removed` and `friend_removal_noted`.

Supporting types are fixed. A directory `Member` is one of two kinds, tagged `account` or `group`. Access levels are `Level`: `access`, `read`, `write`, `manage`, `moderator`, `admin`. Account tiers are `Tier`: `anonymous`, `registered`, `verified`. Account secrets enter the directory only as SHA-256 digests, and the account key domain is `id-account-v1`. Directory groups are the directory's own group records (`group_created` and its sibling entries); they are distinct from the event-log groups in the next section.

Admission to a directory runs through seitan, in `id-directory` (modules `seitan` with accept, admit, derive, freshness and token parts). The seitan V2 admission protocol derives invites with HMAC-SHA512 and a scrypt stretch. Invite records carry the magic `IDINV` (V2). The web surface exposes the same flow at `POST /api/seitan/invite` and `POST /api/seitan/admit`. World invites are a separate minting path with their own shape: `id world invite <NODE> --name <NAME>` requires the participant display name, takes `--role` from `spectator`, `observer`, `player`, `guest`, `delegator`, `builder`, `moderator` (documented default `guest`), `--expires-in` (documented default 7 days, maximum 30 days) and `--uses <N>` (at least 1; omitted means unlimited).

Operator provisioning writes directory state at serve startup. The `--server-config` TOML names `[[users]]` entries (a key name, a 64-hex Ed25519 public key as `id`, an optional alias) and `[[groups]]` entries (key, name, a creator user key who becomes Admin, optional alias, and `[[groups.members]]` rows with roles from `pull`, `read`, `write`, `manage`, `admin`). Provisioning is idempotent. The full key list is in [Configuration and environment](../reference/configuration.md).

## Event-log groups

The second group system lives in the signed event log. The model — group record, deterministic fold projection, friendship, policy, wire forms — is owned by `id-events` (`groups` with delegation, event, fold, friendship, policy and wire parts). `id-groups` is the enforcement remainder over that model: group actions, the group log, local warnings, the blob gate and group encryption. Group encryption runs on p2panda-encryption's `DataScheme`, with causal ordering, history modes and welcome chunking bounded by `id_core::frames::MAX_FRAME_BYTES`.

The CLI writes group actions to a log file: the global flag `--groups-log <FILE>`, with runtime default `groups.jsonl` in the data directory. `id group --json` prints one JSON object per action, and an array for `list`. A refused action exits non-zero, and its event stays in the log.

Group creation takes a template. `id group create --template <normal|friendship>` defaults to `normal`. A friendship group requires `--with <ACCOUNT>`, has exactly two members, and rejects `--max`: the error is `a friendship has two members, so it takes no --max`. A normal group takes `--max <N>` and `--admins-on-create`.

Membership commands are `id group add <GROUP> <MEMBER>` with `--role` defaulting to `read`, `id group role <GROUP> <MEMBER> <ROLE>`, `id group remove <GROUP> <MEMBER>`, `id group policy <GROUP>` and `id group list`. Removal takes `--mode` from `server-default`, `revoke`, `transfer`, `keep-prior`, `reassign-to-group`; the successor flags `--successor-kind` and `--successor <ACCOUNT>` require `--mode transfer`.

One command is a stub, and the book states it plainly. `id group join <INVITE>` parses the invitation ticket, then always fails: `group join is unavailable: invitation parsing works, but admission and network acceptance are not implemented yet; no group membership was recorded`. No group membership is recorded by that command. Disagreement CLI-9: the top-level help text lists join among the group verbs, while the verb's own help and handler record it as unavailable. The handler is the correct side.

## Delegation and revocation

Delegation is two top-level commands over the group log. `id delegate <GROUP> <GRANTEE>` grants a role: `--scope <ROLE>` defaults to `read`, `--redelegate` permits the grantee to delegate further, and `--mode` takes `keep_prior`, `cascade` or `transfer`. The command prints the delegation's event ID. `id revoke <GROUP> <GRANT>` revokes, where the grant argument is the event ID that `delegate` printed. Both commands accept `--json`.

The same delegation vocabulary is present in the event store's group projection (`id_events::groups::delegation`) and in the world layer's agent delegation (`id_caps::agent_delegation`, with `Delegation`, `JoinGrant` and `verify_join`). The generic invite-delegation design — encoding, depth, leakage and group live-set semantics — is a design record, carried in [Design: identity, directory, groups and delegation](../design/identity-directory-groups.md).

## Writer sets

A writer set names the set of authors whose events count for an object. The types are in `id-events`: `idset::{WriterSet, Entry, Excludes}`, with the view projection `idset::view::expand_set`. Writer sets are also a sync gate: `id-events` owns writer sets and sync gating together, so the set decides whose writes a node accepts during ingest.

The CLI surface is `id writer-sets`, with `--event-store` as its store (runtime default `events` in the data directory) and three subcommands: `show <EVENT>`, `expand <EVENT>` (with `--resolution <HEX>` and `--retro`), and `discover` (with `--scope <SCOPE>` and `--resolution <HEX>`; no scope means all sets). The writer-set v2 design — view-as-object, set algebra, federation and discovery v2 — is a design record carried in [Design: sync, replication and FileOps](../design/sync-fileops.md). The shipped surface is the v1 projection and discovery above.
