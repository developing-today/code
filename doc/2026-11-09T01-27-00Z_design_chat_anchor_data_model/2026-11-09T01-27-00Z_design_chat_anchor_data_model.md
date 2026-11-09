# Chat anchor data model: typed links and durable part selectors

**Status:** Decision record for [#5870](https://github.com/developing-today/code/issues/5870)

**Parent:** [#5682](https://github.com/developing-today/code/issues/5682)

**Decision date:** 2026-11-09 (UTC)

## Decision summary

Represent conversation-to-object relationships as first-class, addressable
`ConversationLink` records, separate from chat messages. A record has a typed
relationship and typed endpoints; a part link carries one revision-bound,
object-native selector. Link records are the source of truth in both
directions: reverse navigation is an index, not a second mirrored link.

Keep the original target revision and selector immutable. Resolve an anchor on
later revisions using the object's stable part IDs and revision edit map first,
then a unique exact content fingerprint. Show a relocation warning when
content matching moves the anchor. Never silently choose among approximate or
multiple matches. Preserve an orphaned or tombstoned link and its conversation
rather than deleting or retargeting it.

The link model is application metadata, not message content or a new required
chat-protocol field. All link, selector, fingerprint, status, and reverse-index
data remains subject to the target object's access checks: a caller who cannot
read a private target must not learn that a link or target exists.

## Context and scope

Issue #5682 extends ID chat v1 (#2051), which uses the messaging/relay protocol
in #2036, with conversation/object links, talk pages, and terminal
participation. Its first feature calls for links to addressable ID objects,
typed relationships, navigation in both directions, cross-object conversations,
and anchors that survive edits, versions, and deletion. It also requires that
private-object content and existence not leak to non-readers.

This record settles the structural model for the first feature only. It does
not decide talk-page lifecycle or visibility policy, terminal interaction,
notifications, presence, search semantics, or the discussion-to-edit workflow
(the sibling designs #5871–#5875, including #5874). It does not add fields to
chat messages, require a new #2036 protocol version, or decide the policy for
who may read a conversation. Any such policy must still satisfy #5682's
private-object non-disclosure constraint.

## Chosen model

### Link record

Use one canonical record with this logical shape:

```text
ConversationLink {
  link_id: LinkId,
  source: ConversationRef { conversation_id, thread_id? },
  target: ObjectRef { object_id } | ConversationRef { conversation_id, thread_id? },
  relation: about | "anchored-to-part" | "derived-from" | "resolved-into",
  part_anchor?: PartAnchor,
  created_at: Timestamp,
  created_by: ActorId,
}
```

`thread_id` is absent when the relation applies to the whole conversation.
`target` is a tagged reference, never a pasted URL or copied object title.
The link contains no target excerpt or preview. The allowed combinations are:

| Relation | Target | Part anchor | Meaning |
| --- | --- | --- | --- |
| `about` | ID object | absent | The conversation/thread discusses the object generally. |
| `anchored-to-part` | ID object | required | The conversation/thread refers to exactly one selected part of the object. |
| `derived-from` | conversation/thread | absent | The conversation/thread was forked or spun out of that source conversation/thread. |
| `resolved-into` | ID object | absent | The discussion produced or points to an addressable edit, decision, or other result object. |

The `resolved_into` edge records a result reference only; it does not define
proposal, approval, or editing semantics. Those remain with the separate
discussion-to-edit design. A result that is not an addressable ID object is
not a `resolved_into` target until it has an object identity.

A conversation may have any number of links, including links to several
objects. Each part link names one selector; use another link record for another
selected part. This keeps one selector's lifecycle independent of the others.
`link_id` identifies the canonical edge. Object-to-conversation and
conversation-to-result navigation use reverse indexes over that same edge;
do not write a duplicate reciprocal edge that could disagree. Exact repeated
creation of the same edge is idempotent, while distinct part selectors remain
distinct links.

### Part-anchor representation

A `PartAnchor` is pinned to the immutable object revision in which it was
created:

```text
PartAnchor {
  origin_revision_id: RevisionId,
  selector: TextRange | Block | Region,
  resolver_fingerprint?: OpaqueTargetScopedFingerprint,
}
```

The selector variants are:

- **Text range:** `{ part_id, start, end, unit: "unicode-scalar" }`. `start`
  and `end` are zero-based, half-open offsets into the object's canonical text
  representation for that part; `end` must be greater than `start`. UI cursor
  and grapheme positions are converted at the client boundary. The wire/storage
  coordinates do not depend on UTF-8 byte offsets or a client's UTF-16 index.
- **Block:** `{ part_id, block_id }`, where `block_id` is stable within the
  object's revision lineage. Use this for a paragraph, board post/card, cell,
  or other addressable structured unit when a text range is not the right
  granularity.
- **Region:** `{ part_id?, selector_type, value }`, where `selector_type` is
  registered by the object type and `value` is its canonical typed selector.
  Units and coordinate spaces are part of that selector's definition (for
  example, a media time interval or a table cell address); untyped coordinates
  are not valid. Prefer stable logical identifiers over positions when the
  object type has them.

The optional fingerprint is private resolver metadata, not a quote. It may be
computed from selected content and bounded context, but it must be opaque and
available only to the authorized resolver. Do not put raw selected text,
object titles, or previews in a link record or expose a fingerprint as a public
selector. The object type remains responsible for the canonical text and
revision mapping its selector uses.

### Edit, version, and delete survival

The original `origin_revision_id` and selector are immutable. For a target at a
later revision, expose a separate resolution projection for that revision;
never rewrite the original selector to make a link appear current. Resolution
uses this order:

1. If the object supplies a deterministic revision map and stable part/block
   identity maps the selection exactly, resolve it as **current** with no
   warning.
2. Otherwise, if the resolver finds exactly one match for the opaque content
   fingerprint in the same object's revision lineage, resolve it as
   **relocated** and show a non-blocking “anchor moved” warning. Preserve both
   original and resolved selectors.
3. If only an approximate match exists, or more than one candidate exists,
   mark it **drifted**. Warn, retain the original anchor, and require a person
   or object-specific resolver to confirm a candidate before navigation
   retargets. Candidate details are visible only to readers of the object.
4. If the selected part or range no longer exists and there is no candidate,
   mark it **orphaned**. Keep the link and conversation; do not guess a new
   target.
5. If the whole object is deleted, mark its target **tombstoned**. Keep the
   link and conversation, but do not preserve a content preview or make the
   tombstone visible to a caller who could not read the object.

An anchor follows revisions of the same object identity only. A copied object,
fork with a new object ID, or unrelated object that happens to contain the
same text does not inherit the link automatically. When the original revision
is retained and authorized, clients may offer it as an explicit destination;
otherwise navigation uses the latest confirmed resolution. A change in anchor
status never deletes the conversation or changes its message history.

### Protocol and privacy boundary

The link is application-level metadata attached to a conversation/thread, not
a message body field. A client that understands links can resolve the record
and navigate; a client that does not understand links can still exchange the
underlying chat messages. If link records need replication, use the optional
application metadata/extension path already supported by #2036. Do not require
a new message kind, a new mandatory envelope field, or a protocol-version bump
for ordinary chat. The exact transport/storage endpoint remains an
implementation choice within that boundary.

Apply authorization before returning a link or any projection of it. This
includes its ID, endpoint IDs, relationship, part selector, fingerprint,
resolution status, candidate count, reverse-index result, notification, and
search hit. If the caller cannot read the target object, return no indication
that the edge or target exists. Link metadata does not grant target access or
change conversation membership. Conversation visibility itself remains a
#5682 policy decision; implementations must ensure that transcript, title,
preview, and notification rendering do not disclose the private target.

## Alternatives rejected

| Alternative | Reason for rejection |
| --- | --- |
| A pasted URL in a message | Not a first-class relationship, cannot reliably support reverse navigation, and has no typed selector or lifecycle. URLs may be kept as user-authored content but are not the canonical link. |
| Store the relationship only in the message body or envelope | Couples link lifecycle to message history, makes multi-object/thread links awkward, and risks exposing private target metadata through ordinary transcript delivery. |
| Store only offsets against the current object | Edits and version changes silently retarget positions; the same offsets can point at unrelated content. |
| Store raw quote text in the link | Duplicates potentially private object content and can disclose it through sync, indexes, logs, or clients that lack object access. |
| Always choose the “closest” or first relocation candidate | Approximate and ambiguous matches can point a discussion at the wrong content. Drift must be visible and confirmation required. |
| Delete links when a part/object is deleted | Breaks reverse history and disconnects a conversation that the parent design says must survive; use orphan and tombstone states instead. |
| Publish a tombstone or relation to every conversation reader | Reveals private-object existence; access checks must hide the edge itself, not merely its preview. |
| One universal coordinate format for all object regions | Text, media, tables, boards, and other object types have different semantics and units; typed object-native selectors are less ambiguous. |
| Duplicate a reverse link record | Creates two sources of truth that can disagree; derive reverse navigation from the canonical edge. |
| Require new core protocol fields or a protocol-version change | Expands the parent protocol/client scope unnecessarily; optional application metadata preserves compatibility with ordinary chat clients. |

## Implementation invariants

Implementations of this decision must hold all of the following:

1. Every link has a stable `link_id`, a typed source and target, and one of the
   four allowed relation/endpoint combinations above.
2. `anchored-to-part` has exactly one object target, one origin revision, and
   one valid selector. Ranges are non-empty half-open Unicode-scalar ranges;
   block and region selectors are valid for the target object type.
3. Link creation is idempotent for an identical edge, but separate selectors
   are not collapsed. Bidirectional navigation is derived from the same
   canonical record.
4. No link or anchor field, status, candidate, count, search result, or
   notification reveals a private target's content or existence to a
   non-reader. A link never grants access.
5. Original revision and selector are immutable. Resolution is revision-bound
   derived state, and only an exact deterministic mapping or unique exact
   fingerprint match may retarget automatically.
6. Approximate or ambiguous matches are `drifted`, visibly warned, and never
   silently followed. Missing selectors are `orphaned`; deletion of the whole
   object is `tombstoned`.
7. Orphaning or tombstoning a target does not delete the link, its conversation,
   or chat messages. No resolver follows content into a different object ID.
8. Unknown or unsupported link metadata does not prevent a client from reading
   or sending ordinary chat messages; clients that render a link preserve its
   status and warning rather than presenting a stale selector as current.
9. Reverse lookup, search, and rendering apply object authorization before
   returning any existence-bearing result.

## Parent-decision check and implementation handoff

This record refines the first feature in #5682; it does not reverse a parent
choice or take on talk-page, thread-visibility, terminal, or edit-loop policy.
It applies the parent's existing private-object non-disclosure constraint to
the link and resolver data. The optional-metadata boundary above is a
compatibility constraint for this model, not a replacement for #5682's
protocol design.

No implementation code is included here. The link shape and resolver form one
cohesive design unit; implementation sequencing and issue splitting remain
follow-up planning rather than additional decisions in this record.
