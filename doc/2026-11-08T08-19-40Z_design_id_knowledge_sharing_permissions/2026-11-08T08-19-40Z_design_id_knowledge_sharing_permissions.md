# Design: Knowledge-layer sharing and permissions

- Date: 2026-11-08
- Status: **design decision record**, ready for issue-owner review; no implementation is included.
- Issue: [#5840](https://github.com/developing-today/code/issues/5840), a sharing-and-permissions cluster under [#5678](https://github.com/developing-today/code/issues/5678).
- Parent object model: [#2048](https://github.com/developing-today/code/issues/2048) and [`doc/2026-11-01_design_id2048.md`](../2026-11-01_design_id2048.md).
- Baseline: current `main` at `e81491d4cf8025dfcba172184c46594581d8e8f5`.

## Decision in brief

Use ID accounts and existing groups as the only knowledge-sharing principals. Resolve **read** policy in the already-ruled order: note → owner's account default → server default → public. At the selected scope, a public rule admits everyone; a restricted rule admits its explicitly named users and groups. Multiple grants within that rule are additive. The note owner always retains control.

Keep **editing** separate from reading: the owner and members of explicitly selected groups with `Write` (or a stronger group role) may append revisions; `Write` includes read. Direct per-user read grants are allowed by the parent ruling, but direct per-user editor grants are not part of v1. Group administrators manage group membership, not a note's ACL.

Do not add a workspace ACL layer, anonymous private guests, or bearer-link access. A URL is a locator, never proof of access. Read policy may inherit defaults; edit authority does not inherit through the note graph or a workspace. Before restricted content is enabled, event sync, blob fetch, metadata/search and collaboration writes must all enforce the same policy. A permission change cannot erase content or keys already copied by an authorized reader.

## Scope and constraints

This record settles the choices named by #5840: write/edit sharing, link sharing, workspace/team principals, guest access, and per-object versus inherited permissions. It does not redesign note identity, revisions, friendly-name resolution, or the world permission UX. Those remain with #2048, #2047, and the separate world-permission work (including #5960).

The parent #5678 has already ruled the read default and precedence: public read; owner grants may name users and groups; resolution is object → user default → server configuration → public. This record makes that order operational without changing it. ID groups already distinguish `Pull`, `Read`, `Write`, `Manage`, and `Admin` (`pkgs/id/src/groups/mod.rs:48-65`). The #2048 baseline also records that the current event-sync pull gate is open and the blob-reference gate is not wired into `id serve` (`doc/2026-11-01_design_id2048.md:21-23`).

## Terms

- **Principal:** a stable ID account key or stable group ID. Display names and friendly references are labels, never authorization identities.
- **Read policy:** the rule selecting which principals may read a note's plaintext and its discoverable metadata.
- **Edit grant:** authority to append a signed note revision. It is distinct from permission to manage access.
- **Inherited policy:** a read policy obtained from the owner or server because the note has no more-specific rule. Links, backlinks and collections are not policy parents.
- **Guest:** an authenticated ID account that is not the note owner. Anonymous visitors are not guests for private-content purposes.

## Decisions

### 1. Read policy and inheritance

Each note may carry an explicit `public` or `restricted` read rule. A restricted rule contains zero or more stable account IDs and group IDs. If the note has no rule, resolve the first configured rule in this order:

1. the note's own rule;
2. the owner's account-level default;
3. the serving server's default;
4. `public`.

An unset value is different from explicit `public`. The first rule present wins; rules at less-specific scopes are not silently unioned into it. Within the selected restricted rule, a direct user grant or membership in any named group at `Read` or higher is sufficient. The owner always reads their own note. A more-specific explicit `public` rule can therefore widen an inherited restricted default, as required by the parent's precedence; the UI must make that effect visible. A server default is a default, not a mandatory maximum. Deployment sensitivity limits remain a separate, stricter boundary and cannot be relaxed by this policy.

Changing an account or server default changes every note that still inherits it. Before applying such a change, the UI should show the affected-note count and resulting visibility. The owner can pin a note to an explicit rule to stop it inheriting later default changes.

### 2. Editing and access administration

The owner may read, edit, and change the note's policy. A note may name one or more group IDs as editors. A current member holding `Write` or a stronger role in any named group may append a new signed revision; that authority includes read. A `Read` member cannot edit. Direct per-user grants remain read-only in v1; an owner who wants an individual editor adds that account to an editor group. There is no inherited write default and no write inheritance through links, backlinks, tags, or collections.

Only the note owner may change the note's sharing policy. A group's `Manage`/`Admin` authority remains authority over that group's roster and policy; it does not silently become note-ACL administration. Group removal or role changes alter the effective group grant according to the group's current signed state.

### 3. Teams, workspaces, and guests

Existing ID groups are the team principals. A note can grant read to several groups and can name editor groups independently. Do not introduce a second workspace/team membership system for this cluster. A workspace or collection may organize notes in the UI, but it does not grant access and does not propagate permissions to linked notes.

Public notes may be read anonymously. A non-public reader must have an ID account named directly or be a current member of a granted group. A guest account can receive `Read` (or, when deliberately added to an editor group, `Write`) like any other account. There is no anonymous private guest role, temporary guest ACL, or access-by-email in v1.

### 4. Links are locators, not capabilities

A friendly name, URL, QR code, or copied note reference identifies a note; it does not grant access. Public notes are readable through their normal link. A restricted note link requires the same authenticated read grant as any other lookup and must not reveal its title, existence, backlinks, or blob hashes to an unauthorized viewer. Reject bearer URLs and "unlisted but anyone with the link" as v1 access modes: the parent already defaults to public read, while secret-link access would create a second, hard-to-revoke authorization path.

### 5. Enforcement and confidentiality boundary

The policy decision must be made before returning note content **or sensitive metadata**. It must apply consistently to event-range sync, blob fetch/observe, search, friendly-name resolution, backlinks, history, web save/checkpoint, and live collaboration. No path may infer read authority from a writer-set view projection (#2238), a note link, or possession of a blob hash. A denial for an inaccessible note must not disclose whether its ID or blob exists.

The group-role contract distinguishes ciphertext replication (`Pull`) from reading content (`Read`) and editing (`Write`). Preserve that distinction: `Pull` alone never authorizes plaintext or metadata disclosure; private content keys must be available only to current read principals. Use the existing group crypto/key-envelope machinery rather than inventing a second permission meaning. The exact storage/wire integration belongs in the implementation slices, but restricted sharing is **not enabled** until both event and blob serving enforce it and the encrypted-content path has tests. Current `event_sync` serves the whole store under `PullPolicy::Open` (`pkgs/id/src/event_sync.rs:1-31`); `blob_gate` has no reference-index producer and is not wired into `id serve` (`pkgs/id/src/blob_gate.rs:1-25`). These are explicit prerequisites, not presumed capabilities.

Membership revocation blocks future reads and future key delivery after the new group state is observed. It cannot recall plaintext, keys, or ciphertext already copied by a formerly authorized member. V1 promises no retroactive erasure. New revisions after a removal must use key material unavailable to the removed member; historical material remains readable to anyone who already obtained its key. UI language must say "remove access for future sync" rather than promise remote deletion.

## Alternatives rejected

| Alternative | Why it is not v1 |
|---|---|
| New workspace/team ACL system | Duplicates ID groups, creates two membership authorities, and makes revocation/role precedence ambiguous. Use groups as principals; workspaces are presentation only. |
| Per-note arbitrary role matrix for every user | Duplicates the existing group role/delegation model and makes editor administration scale with each note. Direct user grants remain available for read per the parent; editing is group-based. |
| Permission inheritance through folders, links, or backlinks | The knowledge graph is not a tree. A backlink or reference must not silently widen an audience; use only the parent's explicit owner/server defaults. |
| Bearer link / unlisted access | A leaked URL becomes an untracked grant and revocation is unclear. Links remain locators; public read already has a clear meaning. |
| Anonymous private guests | There is no stable principal for auditing, role changes, or revocation. Use an ID account and an explicit group grant. |
| Rely on client filtering or #2238 view membership | It does not protect sync/blob serving or metadata. The actual event and blob boundaries must enforce the policy. |

## Invariants for implementation

1. Policy resolution is deterministic and follows note → owner default → server default → public; explicit `public` and unset are distinct.
2. Within a selected restricted read rule, direct user grants and group `Read`-or-higher grants are additive; `Write` implies `Read`.
3. Only the owner changes a note's access policy. Group admins manage group membership, not note policy.
4. A note reference, friendly name, backlink, collection, URL, or blob hash never grants access or leaks restricted existence/metadata.
5. No restricted plaintext is served until event sync, blob paths, metadata/search and live collaboration enforce the same current policy. `Pull` alone is ciphertext-only.
6. Removing a grant prevents future authorized fetch/key delivery after state convergence; it does not promise erasure of previously copied data.
7. A note's stable identity and revision history remain those of #2048; permission changes do not rename the note or rewrite its historical revisions.

## Implementation order and acceptance

1. Add a policy resolver with property/table tests for unset versus explicit public, precedence, multiple groups, direct user grants, owner access, role thresholds, and changing inherited defaults.
2. Define signed policy updates and connect them to the stable note identity/revision model. Test that non-owners and group admins cannot change a note ACL.
3. Integrate current group membership/key delivery with event sync and blob serving; test outsider, `Pull`-only, `Read`, `Write`, removed member, missing reference, and unknown hash cases. Denials must be indistinguishable where existence would leak.
4. Gate collaboration checkpoints and revision ingestion on `Write`; ensure all read surfaces share the same decision function.
5. Add UI for effective policy, explicit override, inheritance changes, guest/account requirements, and link-not-capability wording.

The feature is not ready to enable restricted multiplayer content until steps 2–4 have landed and pass boundary tests. This is a design record only; implementation issues should be filed from the reviewed design rather than bundled into this PR.

## Change log

- 2026-11-08: Initial decision record for #5840, based on the #5678 read-authority ruling and #2048 object model.
