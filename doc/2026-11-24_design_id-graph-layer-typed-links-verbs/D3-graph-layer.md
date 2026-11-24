# D3 — The ID graph layer: typed links and verbs as first-class objects

Status: design record for issue #13885, written 2026-11-24 by lane oc-code-02.
Parent epic: #13880 (ID apps on one graph). Programme order: 2 — HEAD OF THE PROGRAMME.
Substrate: composition A of #13779 (p2panda data plane + Meadowcap-as-spec + broker renewal).
Inputs: R2 (#13883) and R1 (#13884) were still OPEN with no landed record at claim time
(verified 2026-11-24); the on-disk workspace sources they name
(`~/workspace/p2panda-investigation/…`, `~/workspace/worm-blossom-mirror/`) are absent
from this environment. This design therefore proceeds from the issue text, the
comparison record (`doc/2026-11-23T21-39-17Z_design_p2p-auth-transport-comparison/`),
the family notes (`doc/2026-11-23T22-02-29Z_research_p2p-auth-transport/`, esp.
`willow-meadowcap.md`), the social reference model
(`doc/2026-11-23_design_id9128_social_app_reference_model.md`), and the merged
#6062 publication-format design. The R1/R2 gap is recorded on #13885; nothing
here contradicts their stated questions, and §10 lists what they could still change.

## 1. What this decides, and what it does not

Decides: the object model (content objects + link objects), ID assignment and
authorship/signing on composition A, the verb registry v1 with operational semantics,
freeform nesting expressed in the model, the cross-app interop contract for
chat/forum/media apps, per-verb capability rules (create/traverse) plus the Roc
declaration mapping, reconciliation with #6062 and the note-edge case, the SSB
tangle evaluation and decline-lessons annex, and dependency-ordered build slices.

Does not decide: the URI scheme (D1 — consumes object/link IDs defined here), live-vs-snapshot
transclusion mechanics (D2 — consumes the `nest`/`quote` verbs defined here), the display
view-model and renderers (D5 — consumes the per-verb rendering hints in §5), the bundle
format (D4 — carries the objects defined here), multi-device identity (owned by the
IdP/linking story; consumed here, §9), or any #13779 composition-A decision point.

## 2. Model core: content objects and link objects

There are exactly two kinds of graph object. Both are immutable, signed, content-addressed
records carried as p2panda operations on the composition-A data plane — explicitly NOT a
new store on raw Willow entries, and NOT fields inside other objects.

### 2.1 ContentObject

A content object is authored bytes with an identity:

```
ContentObject = {
  object_id,      // content hash of the canonical encoding (§2.3)
  author,         // signer public key (identity per the IdP/linking story, §9)
  created_claim,  // signer-claimed time; untrusted, display only (cf. #9128 §2)
  kind,           // chat-message | forum-post | image | ... (open registry, §8)
  body,           // kind-specific payload, or a content-plane reference for large bytes
  schema_version, // kind schema version (uint)
}
```

`body` for text kinds is inline (bounded size; the bound is an implementation
parameter, recommended ≤ 1 MiB). For media kinds, `body` is `{ digest, size, mime }`
referencing bytes fetched out of band — the graph never embeds blobs.

### 2.2 LinkObject

A link between ID objects is itself a first-class object carrying a verb:

```
LinkObject = {
  object_id,      // content hash of the canonical encoding (§2.3)
  author,         // signer public key — the link's author, independent of
                  // the authors of source and target
  created_claim,  // signer-claimed time; untrusted, display only
  verb,           // reply | mention | quote | attach | supersede | nest (§5)
  source,         // object_id of the subject: the object that asserts the link.
                  // For reply/quote/attach: the new object doing the referring.
  target,         // object_id of the object referred to
  context?,       // verb-specific structure; for reply: { root, previous[] } (§7)
  snapshot?,      // for quote: { digest } pinning the target bytes quoted (§6)
  schema_version, // link schema version (uint, currently 1)
}
```

Direction convention, fixed: the **source** is the object that speaks — the reply, the
quoting post, the attaching message — and the **target** is what it speaks about.
A reader answering "what replies to X?" queries links with `target = X, verb = reply`;
a reader answering "what does Y reply to?" follows Y's outbound `reply` link.
Backlinks are always a computed projection (an index over links), never stored edges —
storing a backlink inside the target would be a cross-author write into someone else's
object, which §3 rejects.

Because a link is an object, it can be signed, capability-gated, replied to (a link's
`object_id` is a valid link target), counted, revoked (superseded or expired, §9),
rendered (D5), named (D1), embedded (D2), and carried in a bundle (D4) — uniformly, by
every app, with no app-pair agreements.

### 2.3 ID assignment, authorship, signing

- `object_id` is the hash of the canonical (field-ordered, length-prefixed) encoding of
  the object excluding `object_id` itself. The hash function MUST match the estate's
  content-hash choice at implementation time (BLAKE3 vs WILLIAM3 is still open
  estate-wide; this design is agnostic and records the dependency rather than picking).
- `author` is an Ed25519 public key today (composition-A Meadowcap validity core is
  signature-scheme-generic per the family notes, so P-384 later is spec-legal).
  Multi-device authorship — one identity, several keys — is owned by the IdP/linking
  story; this layer treats `author` as an identity-stable key and names the dependency
  in §9 instead of re-deciding it.
- Every object travels as (a small number of) p2panda operations, signed by `author`.
  The Meadowcap-as-spec capability token (area = topic × path-prefix × time window,
  bound as a header extension) travels with the operation; short-window grants are
  renewed at broker check-in, and revocation is non-renewal plus supersession hygiene —
  all per composition A. Link objects introduce no new crypto and no new store.

### 2.4 JSON Schemas (normative for the fixtures)

Schemas live alongside this record in `fixtures/` (`content-object.schema.json`,
`link-object.schema.json`). Every fixture in `fixtures/` MUST validate; the checker
`fixtures/check.py` (stdlib only) enforces: required fields, `verb`/`kind` membership
in the v1 registries, `source`/`target` referential integrity across the fixture set,
reply-`context` shape (`root` + `previous[]`), and quote-`snapshot.digest` presence.

## 3. Alternatives weighed (with tradeoffs)

| Alternative                                                                                  | Verdict                 | Why                                                                                                                                                                                                                                                                                                           |
| -------------------------------------------------------------------------------------------- | ----------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Links as fields on the source/target object (reply-to fields, backlink counters in payloads) | **Rejected**            | Links inherit the object's authorship and lifecycle: a reply from another app becomes a write into someone else's object, and deleting the target silently deletes the reply evidence. Backlink counters additionally fork on concurrent writes.                                                              |
| Links as substrate-native relations (extend the p2panda op layer)                            | **Rejected**            | Best performance story, worst portability story; composition A was chosen partly to avoid forking the substrate. A link object on the existing data plane gets sync, partial replication, and the renewal service for free. Revisit only if link-query load proves the index insufficient (D5 measures this). |
| Freeform nesting only (the Rummager model: nesting as the single primitive)                  | **Rejected**            | Maximum app freedom, no interop guarantee. Nesting says "inside", which cannot express mention-without-containment, supersede-across-apps, or quote-with-pinning. Nesting survives as the `nest` verb (§5), not as the whole model.                                                                           |
| Tangle-only model (SSB-style: replies are the only structured link)                          | **Adopted as a subset** | Proven socially for conversation, too narrow for attach/supersede/quote. The reply verb carries the tangle structure (`context.root` + `context.previous[]`, §7); the registry generalises the pattern to non-conversational relations.                                                                       |

## 4. Freeform nesting expressed in the model

Rummager's insight — chat, forums, and image sharing as emergent shapes of nesting —
is expressed as the structural verb `nest`: `source` = child, `target` = parent
(container, thread, board, album). Consequences, all deliberate:

- Nesting is a signed assertion by the child's author, not a container write: putting
  your message "inside" a forum thread does not modify the thread object.
- A child may nest under several parents (cross-posting is multi-parent nesting, and
  transclusion in D2 is `nest` with a live/read rule — D2 decides the mechanics).
- Depth caps, fold points, and page sizes are view policy (cf. #9128 §2, §7), never
  admission rules: the model caps nothing.
- A container's visible children are a projection (links with `target = container,
verb = nest`, ordered by an append-list or ranking the app names), parallel to #9128's
  append-list admission. The social record's `parent_ref` is exactly a `nest` link
  factored out of the envelope — D3 keeps it factored out so nesting shares links'
  capability and revocation story.

## 5. Verb registry v1

Each verb has operational semantics (not just social meaning), a create-rule, a
traverse-rule, and a rendering hint for D5. `source`/`target` orientation is per §2.2.

### reply

Operational semantics: `source` is offered as a conversational successor of `target`.
`context` REQUIRED: `{ root: <object_id of thread root>, previous: [<object_id>, …] }`
— the tangle structure (§7). `root = target` for a direct reply to the root.
Readers thread by `root` and order within it; `previous` repairs ordering under loss.
Create-rule: author holds write capability for the conversation area containing `target`
(§8: chat-room area, forum-thread area, or the target author's public area for
cross-app replies). Traverse-rule: read capability for the conversation area.
Rendering hint: nested/indented under `target` in conversation order; cross-app
replies carry the source app's badge, never silently restyled as native.

### mention

Operational semantics: `source` names `target` (a person-root, an object, a thread)
without containing or succeeding it. No `context`. Notify-side effects are app policy,
not model semantics: the link only asserts "source refers to target".
Create-rule: write capability for `source`'s own area (mentioning never needs rights
on the target — this is what makes cross-app mention safe). Traverse-rule: read
capability for `source`'s area. Rendering hint: inline chip/pill resolving to the
target's D5 summary; unresolvable targets render as plain text, never as errors.

### quote

Operational semantics: `source` embeds the bytes of `target` (or a slice) frozen at
quote time. `snapshot` REQUIRED: `{ digest }` of the exact bytes quoted. A reader
that can fetch `target` compares digests: match → "quoted text matches current";
mismatch → "target has changed; showing quoted snapshot" (the snapshot itself is
carried in `source`'s body so it always renders). Deletion-wins-over-links, adopted from the Willow URI lesson: a quote of pruned data
fails closed with the snapshot shown, never silently re-pointed.
Create-rule: write capability for `source`'s area (quoting needs no rights on the
target). Traverse-rule: read capability for `source`'s area; resolving `target` for
the freshness check needs read on the target's area, and failure there degrades to
snapshot-only display, never to an error. Rendering hint: blockquote with attribution
and a freshness badge (match / changed / unavailable); changed quotes offer "view current".

### attach

Operational semantics: `source` carries `target` as a payload attachment (image in a
post, file in a message). The attachment bytes are referenced by digest (media) or
embedded (small files); the link asserts "source presents target as part of itself".
`source` SHOULD also `nest` under the same container where the attachment must appear
in listings, or the app defines attach ⇒ listed (per-app rule, declared in its manifest).
Create-rule: write capability for `source`'s area PLUS read capability for `target`
(an app cannot attach what it cannot read — the anti-exfiltration rule). Traverse-rule:
read capability for `source`'s area (attachment visibility follows the message, not the file).
Rendering hint: thumbnail/card inline at the end of `source`; large media behind a
fetch-or-consent gate per app policy.

### supersede

Operational semantics: `source` replaces `target` as the current version for the
relation the app names (edit, retraction, moderation tombstone — distinguished by
`source`'s kind, not by sub-verbs). Readers resolving "current X" follow the
supersede chain to its head; history readers can walk it backwards. Tombstones are
ordinary content objects of a `tombstone` kind carrying reason + scope, linked by
`supersede` — removal is a link, never a rewrite (cf. #9128 §8).
Create-rule: write capability for the area containing `target`, AND (`author(source)`
= `author(target)` OR the author holds a moderation grant for that area). Nobody
supersedes a stranger's object with a bare write grant. Traverse-rule: read capability
for the area. Rendering hint: only the chain head renders in normal views; "edited /
retracted / removed" badge links to history where policy admits.

### nest

Operational semantics: `source` is placed inside `target` (container, thread, board,
album). The container's visible children are the projection of inbound `nest` links
(§4). No `context`. Create-rule: write capability for the area containing the
`target` container (posting into a room needs room write). Traverse-rule: read
capability for the container's area. Rendering hint: containment in thread/board/album
views; multi-parent `nest` renders in each parent with an "also in …" marker rather
than duplicating identity.

### Admission rule for new verbs

A new verb is admitted by a design PR that states: (1) operational semantics in one
paragraph, (2) a create-rule, (3) a traverse-rule, (4) a rendering hint, (5) two
independent app implementations OR one implementation plus Drew's sign-off, and
(6) the degradation story for old readers (what they show for links they cannot
interpret — the default is "skip the link, keep the endpoints"). Verbs are never
removed; a verb superseded in practice is marked historical in the registry with its
replacement named.

## 6. Cross-app interop contract

The contract every app (chat, forum, media, and later arrivals) signs by implementing
this layer. It is stated as reader rules and writer rules so conformance is testable.

### Reader rules

1. **Shared IDs, no translation.** Object IDs are global. A reader MUST NOT remap,
   prefix, or namespace another app's IDs.
2. **Unknown verbs and kinds are preserved and skipped, never fatal.** A reader that
   meets a verb outside its registry or a kind outside its schemas MUST keep the
   object for re-sync, MUST still render the endpoints it understands, and MUST NOT
   fail sync, fail the thread, or drop the conversation. Unknown-verb links are
   invisible in that app's views but present in its store.
3. **Degradation is specified, not silent.** Quote without target access → snapshot
   display (§5). Reply to an unreadable target → "message in a thread you can't read"
   placeholder that preserves position. Supersede chain crossing an unreadable object
   → resolve to the newest readable head.
4. **Backlinks are derived.** "Replies to X", "quoted by", "attached in" are index
   queries over links, computed locally. A reader MUST NOT require the target to carry
   inbound references.

### Writer rules

1. **Base content renders without the new verb.** A writer introducing a new verb or
   kind MUST ensure its objects still render usefully in an app that skips the novelty
   (the degradation story in the admission rule is normative for this).
2. **No cross-author writes.** A writer MUST NOT modify, annotate, or countersign
   another author's object. Everything cross-author is a new link object by the writer.
3. **Quote pins; reply points.** Writers MUST attach `snapshot.digest` when quoting
   (§5) and MUST fill `context.root`/`previous` when replying (§7).

### Three-app walkthrough (acceptance)

The fixtures in `fixtures/` enact it end to end: Ana's chat message `msg-ask-photo`
asks for the group photo; Bo's forum thread `post-meetup-thread` collects retrospective
material; Cyp's media object `img-group-photo` is the photo. Cyp answers from the chat
app (`msg-photo-answer`) with `lnk-reply-chat-to-forum` — a chat-authored reply whose
target is a forum object (cross-app reply, no forum write involved) — plus
`lnk-nest-answer-in-thread` so it lists in the thread; publishes from the forum app
(`post-photo-published`) with `lnk-attach-photo` (payload) and `lnk-quote-chat-ask`
(snapshot-pinned, so Ana's later edit of her question cannot rewrite the thread's
context); `lnk-mention-photo` shows mention-without-containment. Run
`fixtures/check.py` to verify.

## 7. SSB tangle evaluation

### What the tangle model is (for this record)

In SSB, a reply references its thread root and its immediate predecessors, forming a
DAG ("tangle") per thread: ordering and completeness are checkable from the messages,
not from any server. This design's `reply` verb carries exactly that structure in
`context: { root, previous[] }`.

### Adopt / adapt / reject

- **Root + previous references on replies — ADOPT.** `context.root` scopes the thread;
  `context.previous[]` names the messages the author had seen. Loss-tolerant ordering
  and "missing messages" detection fall out of the DAG, same as SSB.
- **Thread DAG as the ordering primitive — ADAPT.** SSB tangles are per-feed orderings
  merged across replicating feeds. Here the DAG is scoped to the conversation area,
  ordered by (root, causal depth, created_claim, object_id tiebreak) — deterministic
  without global clocks, and partial replication just yields a sparser DAG, never a
  corrupt one.
- **Feed-as-identity with per-feed sigchains — REJECT.** Link authorship is keyed to
  the IdP-linked identity (§9), not to a single device feed. Concurrent same-author
  links from two devices are DAG siblings that merge, never a "forked feed" corruption.
- **Tangle-only relations — REJECT as the whole model** (see §3): the tangle pattern
  covers conversation, not attach/supersede/quote. The registry generalises it.

### Decline-lessons annex (each lesson gets a named answer or open question)

1. **Single-device identity; multi-device use corrupts forked feeds.** ANSWER: identity
   is the IdP/linking story's stable identity, not a device key; the data plane has no
   per-feed chain to fork. Concurrent writes are DAG branches with deterministic merge
   (§2.3, §7). Residual dependency, not a new decision: the IdP story must actually
   deliver stable multi-key identity — tracked as open question 3 (§9).
2. **Append-only forever; no deletion or pruning.** ANSWER: composition-A non-renewal
   (grants expire; the dead stay readable only where already replicated, and renewal
   stops extending them) plus `supersede` tombstones (§5) give removal-with-evidence.
   Pruning is legal and expected; digest-pinned references (quote snapshots, D1 URI
   digest expectations) fail closed on pruning instead of dangling. Deletion-wins-over-links.
3. **Replicate-everything within two hops; unbounded growth.** ANSWER: scoped sync —
   Meadowcap areas are simultaneously the unit of authorisation and of interest, so a
   seat subscribes to (area × verbs), not to the world. The analogue of SSB's hops is
   an explicit subscription, budgeted (AreaOfInterest-style count/size/time bounds).
   What an app pre-fetches for a thread view is app policy over this mechanism.
4. **No capability scoping: a feed is all-or-nothing public.** ANSWER: this record —
   per-verb create/traverse rules (§5) under composition-A areas (§8), so a grant can
   mean "reply in this room but not attach", "read but never supersede". The SSB
   all-or-nothing feed has no analogue here by construction.
5. **Brutal first sync / onboarding.** ANSWER: ticket-based bootstrap over #13752's
   carriers (broker, pkarr/DHT, email-carried tickets, QR, sneakernet bundle) — one
   signed ticket format, many carriers. A new participant joins a conversation with the
   ticket + the thread DAG slice, not with the world's history. Bundle-carried onboarding
   is D4's slice; the ticket format is #13752's, consumed here.

## 8. Capability mapping: per-verb rules on composition A + Roc declarations

### How a link authorises on composition A

Creating a link object is writing a p2panda object: it requires a Meadowcap write
capability (as-spec token in the op header extension) covering the link's area, within
a live window renewed at broker check-in. Traversing (reading) a link requires read
capability for the area it is stored under. Per-verb tightenings:

| Verb      | Create (write grant for…)     | Traverse (read grant for…)                               | Extra condition                                               |
| --------- | ----------------------------- | -------------------------------------------------------- | ------------------------------------------------------------- |
| reply     | conversation area of `target` | conversation area                                        | —                                                             |
| mention   | `source` author's own area    | `source`'s area                                          | never needs target rights                                     |
| quote     | `source` author's own area    | `source`'s area (target access only for freshness check) | `snapshot.digest` mandatory                                   |
| attach    | `source`'s area               | `source`'s area                                          | author must already hold read on `target` (anti-exfiltration) |
| supersede | area of `target`              | area of `target`                                         | author(`source`) = author(`target`) OR moderation grant       |
| nest      | area of `target` container    | container's area                                         | —                                                             |

Storage-area convention (recommended, D5/D1 may refine): a link is stored under the
area of its `source` for mention/quote/attach (it extends the author's object), and
under the conversation/container area for reply/nest/supersede (it extends the shared
surface). Readers subscribe to areas, so placement decides who pays sync cost — the
convention keeps shared-surface links where subscribers of that surface already sync.

### Roc declaration/grant/omission mapping

Each app ships a manifest declaring the verbs it uses, split into creates and
traverses. The grant authority (broker at check-in, per composition A) issues
capabilities per declared verb: a chat app that declares `reply{create,traverse},
mention{traverse}, quote{create,traverse}, attach{create,traverse}, nest{traverse}`
receives no `supersede` capability at all, so `supersede` is unnameable inside it —
submission attempts are rejected at the edge, and sync filters drop inbound
`supersede` links before they reach the app store. Ungranted verbs are not errors the
user sees; they are absences the app never renders (reader rule 2, §6).

### Worked examples per app type

- **Chat app** declares: create {reply, quote, attach, mention}, traverse {reply,
  quote, attach, mention, nest, supersede(heads only)}. Omitted: supersede/create
  (chat cannot retract the room's history — moderation happens in the forum app),
  nest/create (chat messages land in rooms by room membership, not by user nesting).
  A chat user cross-app-replies to a forum thread (§6 walkthrough) with the thread
  area's reply grant, obtained because the forum's area policy admits chat-app
  identities — the pairwise agreement this design eliminates lives exactly here, once,
  as area policy, not per app pair.
- **Forum app** declares: create {reply, quote, attach, mention, nest, supersede},
  traverse all six. Omitted: nothing in v1 — the forum is the moderation surface, so
  it alone holds supersede/create via moderation grants. Its thread view is the
  projection: root post + inbound reply DAG + inbound nest children + head-only
  supersede resolution.
- **Media app** declares: create {attach, mention, reply}, traverse {reply, mention,
  quote, attach, nest, supersede(heads only)}. Omitted: supersede/create, nest/create,
  quote/create (the media app presents objects; curation — nesting into albums,
  quoting into essays — happens in apps that declare those verbs). Its album view is a
  nest-projection over its own area; chat replies to a photo arrive as traverse-only
  reply links the media app renders as comments it could never have been forced to store.

## 9. Open questions (deciders named)

1. **Verb-set v1 membership — designer proposes, Drew decides any verb with social
   semantics.** Proposal: v1 = the six verbs in §5; `react` is NOT a verb (it is an
   app-level ephemeral signal with no interop or capability story — a reaction that
   another app must interpret is a `reply` with a reaction kind, or it is private app
   state). If Drew wants social reactions as first-class links, the design admits
   `react` under the §5 admission rule with create = conversation area, traverse =
   conversation area, rendering = count badge — but recommends against, because
   countable cross-app reaction totals recreate the backlink-counter fork problem (§3).
2. **Link deletability — Drew decides if the recommendation differs from composition
   A's non-renewal default.** Recommendation: link objects are deletable ONLY by their
   own author and ONLY by supersede-tombstone (never by rewrite); a target's author
   cannot delete inbound links but CAN hide them in their app's projections and CAN
   petition moderation where that grant exists. Expiry follows the grant window
   (non-renewal). This differs from "anyone deletes anything" and from "nothing is
   ever deletable"; it matches the SSB-lesson-2 answer (§7).
3. **Multi-device identity for link authorship — owned by the IdP/linking story.**
   This design consumes its answer (identity-stable author keys, §2.3) and names the
   dependency rather than re-deciding it. If the IdP story lands device-key-only
   authorship, this design's SSB-lesson-1 answer weakens to "same as SSB" and must be
   re-opened on this issue before any build slice starts.

## 10. Dependency-ordered build slices

Each slice is a future queue item; none starts until Drew-visible surfacing of §9
(the open questions above, posted to #13885 with this record).

1. **Object + link model core.** Schemas as IDL + canonical encoding + hash-function
   binding (§2.3 dependency); p2panda operation mapping for both object kinds.
   Acceptance: the §6 fixtures validate byte-for-byte against the implemented encoding.
2. **Verb registry v1.** Registry data + admission-rule tooling + per-verb create/
   traverse enforcement at the submission edge. Acceptance: every v1 verb demonstrates
   its create-rule (granted), its traverse-rule, its rendering hint, and one rejection
   (ungranted verb refused).
3. **Capability mapping.** Area-placement convention (§8) + Roc manifest schema +
   broker grant-per-verb issuance + sync-filter for omitted verbs. Acceptance: the
   three §8 worked examples run as integration scenarios (chat/grant, forum/grant,
   media/grant matrices green).
4. **Interop contract conformance.** Reader/writer rule test-suite: unknown-verb
   preservation, degradation badges, backlink-index queries, the three-app walkthrough
   as an executable scenario. Acceptance: a verb the suite does not know is introduced
   and all four reader rules still pass.
5. **Tangle ordering + SSB lesson regression checks.** Reply-DAG ordering under loss,
   missing-previous detection, tombstone/supersede resolution, expiry non-renewal.
   Acceptance: each §7 lesson has a failing-without-the-fix regression test.

## 11. Reconciliation record (what this design covers, not forks)

- **#6062 publication format** (merged design): Atlas versions → content objects;
  logical-file `note.create`/`note.revise` chains → content objects + `supersede`
  links; `atlas.*` tag-claim occurrences → `attach`/`mention` links; ingest-batch
  `attestation.claim` → content object authored by the ingest identity. PROV-lite:
  `wasRevisionOf` → `supersede`, `specializationOf` → `nest`, `wasDerivedFrom` →
  `quote` (with snapshot) or `attach`, `wasGeneratedBy` → `attach`,
  `wasAttributedTo` → authorship (`author` field, never a link). Canonical authority
  stays where #6062 put it (Atlas tables; ID graph as re-projectable view).
- **Note reference edges + backlink query** (task packet's "#13233" case): the issue
  number does not resolve in developing-today/code (verified 2026-11-24 — recorded
  gap, no silent substitution). The case itself — note reference edges with a backlink
  query — is covered regardless: outbound references are `mention`/`quote`/`attach`
  links at the author's discretion, and the backlink query is the derived index of §6
  reader rule 4. When the correct issue number is known, link it here.
- **#9128 social reference model**: its envelope factors into this design's content
  object + `nest` link (`parent_ref` ≅ `nest`, append-list surfaces ≅ nest-projections,
  freshness token unchanged). D5 consumes both records; where they disagree on nesting
  representation, this record governs for link-shaped nesting and #9128 governs views.
