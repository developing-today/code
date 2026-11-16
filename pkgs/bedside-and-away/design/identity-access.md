# Identity and access component (DM-01)

## Intent and choices

Requirements BA-043–045, BA-078–089 require identities independent of names, multi-person stores, and per-item policies without information leaking through search, history or grouped items. This component extends the existing authenticated store role model rather than adding an alternative authentication provider. Tailscale remains the verified entry point. A UUID is continuity metadata, never login.

Alternatives considered: client-side visibility filtering is unsafe; generic snapshot merging of hidden rows is ambiguous under concurrency; server policy projection plus fail-closed writes is safe while typed commands are developed. Choose the third. No external dependencies or new production infrastructure are introduced.

## Portable contract v1

`state.access` (optional for backward compatibility) is `{schemaVersion:1, groups:{groupId:[accountId]}, policies:{entityKey:{default?:'allow'|'deny',inherit?:[entityKey],rules:[rule]}}}`. Entity keys are `item:id`, `tag:id`, `note:id`, `inventory:id` and `*`. IDs are opaque and remain independent of editable display names. Account IDs and store role originate at the trusted server/gateway. Group lists and policies are server-persisted data, never trusted request fields.

Rules have `{effect:'allow'|'deny',subject:{type:'everyone'|'user'|'group',id?},actions:['read'|'write'|'admin'],priority?:100,overrideDeny?:false}`. Only a store administrator may change any access policy or group membership. `everyone` means every authenticated principal already authorized for the store, never the public Internet. Roles impose an absolute capability ceiling: rules cannot turn a reader into an editor. Lower numeric priority is stronger (Nix-like); equal-priority conflict denies. Denies from any explicit inherited scope remain hard blocks unless an administrator-authored matching allow has `overrideDeny:true` and strictly smaller priority than every matching deny. An ordinary allow never defeats a deny. Cycles, absent group/parent references, excessive depth, malformed rules, and invalid versions fail closed.

The `*` policy is the store read gate. A deny there denies the entire view; use resource inheritance for narrower deny/priority-override behavior. A category's membership is not implicit policy inheritance: moving or tagging an item cannot publish it. Policy inheritance must be explicit and administrative.

## Read and write boundaries

- `evaluateAccess` returns a boolean decision plus a non-sensitive reason code.
- `projectSnapshot` filters items, categories, parent/member/related references, and known target-bound notes/inventory. With scoped access it emits only known root fields and resets global/computed aggregates; reports must be recomputed from that projection. Unknown target shapes and global notes are conservatively omitted. Open visitors additionally see neither notes nor inventory; concealed/archived objects are omitted. Policies/groups are not exposed in scoped views.
- `projectEvents` intentionally returns no snapshot history for scoped or visitor views: old snapshots and labels could reveal content now hidden. It also withholds old events that contained access policies from currently unscoped viewers.
- `assertSnapshotMutation` rejects any policy/group change by a nonadmin, and rejects whole-snapshot writes by scope-limited editors. Admin writes cannot alter denied records. This is an honest read-only limitation, not a finished per-entity editing solution. Admin policy recovery requires a dedicated endpoint that never overwrites unseen data.
- Apply the projection before search, summaries, exports, research context, and any callback response. API authorization must use stored state, not the client projection. Reject malformed policy before mutation persistence. Existing authenticated role and optimistic-version transaction checks remain mandatory.

## Guest/account proof design

`identity.mjs` supplies random UUID installation IDs and 256-bit guest-claim token primitives. Only the SHA-256 hash is persisted; token is returned once. Claims bind guest ID and store ID, expire in at most ten minutes, and reject consumed records, changed stores, invalid tokens and absent authenticated accounts. These primitives do not create sessions or authenticate accounts.

The future linking route must obtain guest proof through a secure HttpOnly/SameSite session, preview the exact store attachment, obtain authenticated account proof independently, then atomically consume the claim and transfer/link the store with an audit event. A unique claim ID and `consumed_at IS NULL` compare-and-set are required to prevent parallel replay. Collision handling preserves guest store IDs instead of overwriting an existing personal store. Account-to-account linking requires both sign-ins, reauthentication, revocation, and recovery design. Never infer account sameness from names, UUIDs or fingerprinting. Browser reset clears continuity; no respawning identity, permanent-storage promise, or covert fingerprint collector.

## Implementation and tests

1. New `worker/access.mjs` pure evaluator, validator and projections; `tests/access.mjs` role/deny/override/cycle/mutation/leak tests.
2. API integration: stored-state validation before writes; projections before all reads/search/history; preserve existing open-store restrictions; typed granular commands are follow-up work.
3. New `worker/identity.mjs`; `tests/identity.mjs` hash/expiry/binding/consumed-record tests. Do not wire it as login until atomic session/linking storage and callback tests exist.
4. Reviewer should test direct ID, cross-store role, old revision, concealed parent, mixed-target comment, malformed config, `__proto__` role, and equal-priority conflict cases. Type validators reject malformed policy rather than treating it as unrestricted.

## Acceptance and remaining scope

Implemented pure logic: BA-088/089 evaluation and projection foundation, role negative tests for BA-078, proof-token foundation for BA-083/084. Integration is not established merely by these tests. BA-043 browser persistence/reset UI remains partial; BA-044 fingerprinting deliberately absent pending explicit transparent consent and minimization design; BA-045 display-name changes remain existing prototype behavior; BA-079 invitation expiry and complete lifecycle integration remain follow-up; BA-080 anonymous store provision absent; BA-081 second-person ingress not granted; BA-082 Google/password login absent; BA-083/084 full guest/account linking not implemented; BA-085 open-store gateway audience remains existing approved scope; BA-086 filtered API integration required; BA-087 Atlas explicitly deferred. Scoped editor commands, granular history projections, policy administration UI, typed note-target adapters and ACL-safe derived reports are not claimed complete.

## Rollback and migration

New code is additive, config absent preserves existing role semantics. Do not remove policy-aware readers after a store contains policy: that would expose formerly concealed content. Rollback must keep policy enforcement or freeze affected stores. Export/backup must retain access schema and group data under admin authorization. No existing migrations, credentials, identity maps, or live records are changed by this component.
