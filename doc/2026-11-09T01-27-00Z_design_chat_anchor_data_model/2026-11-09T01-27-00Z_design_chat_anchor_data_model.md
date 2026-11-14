# Chat anchor data model: typed links and durable part selectors

**Status:** Decision record for [#5870](https://github.com/developing-today/code/issues/5870)

**Parent:** [#5682](https://github.com/developing-today/code/issues/5682)

**Decision date:** 2026-11-09 (UTC)

## Decision summary

Represent conversation-to-object relationships as first-class
`ConversationLink` projections, not duplicated mutable records. Ordinary
`about`, `anchored-to-part`, and `derived-from` links fold from #5872's
append-only `chat.anchor.v1` events. `resolved-into` links fold from
`chat.resolution.v1` edit results under #5874. A talk-page `talk-for` binding
is a separate projection over `chat.anchor.v1`, not an ordinary relation in
this link graph. Each part link carries one revision-bound, object-native
selector. Reverse navigation is an index over the owning event stream(s), not
a second mirrored link.

Keep the original target revision and selector immutable. Resolve an anchor on
later revisions using the object's stable part IDs and revision edit map first,
then a unique exact content fingerprint. Record accepted relocation as a new
`chat.anchor.v1` event; show a warning for content-based movement. Never
silently choose among approximate or concurrent matches. Distinguish
`current`, `relocated`, `drifted`, `disputed`, `unresolved`, `orphaned`,
`tombstoned`, and explicitly `unbound` links. Preserve event history and the
conversation through all of these states.

The link model is not a `chat.message.v1` body field. It consumes #5872's
additive `chat.anchor.v1` event contract; it does not introduce a second wire
format. All link, selector, fingerprint, status, and reverse-index data remains
subject to the target object's access checks: a caller who cannot read a
private target must not learn that a link or target exists.

## Context and scope

Issue #5682 extends ID chat v1 (#2051), which uses the messaging/relay protocol
in #2036, with conversation/object links, talk pages, and terminal
participation. Its first feature calls for links to addressable ID objects,
typed relationships, navigation in both directions, cross-object conversations,
and anchors that survive edits, versions, and deletion. It also requires that
private-object content and existence not leak to non-readers.

This record settles the link vocabulary, part-selector grammar, and anchor
survival states for the first feature only. It consumes [#5872's decision on
`chat.anchor.v1` and `talk-for`](https://github.com/developing-today/code/issues/5872#issuecomment-6658733227),
[#5873's fragment-safe selector requirement](https://github.com/developing-today/code/pull/7614),
and [#5874's `chat.resolution.v1` projection](../2026-11-08_decision_id5874_thread_resolution.md).
It leaves talk-page lifecycle and visibility policy, terminal interaction,
notifications, presence, search semantics, and discussion-to-edit mechanics to
the sibling designs #5871–#5875. It does not change `chat.message.v1`, add a
mandatory envelope field, require a protocol version bump, or choose who may
read a conversation. Any such policy must still satisfy #5682's private-object
non-disclosure constraint.

## Chosen model

### Link record

Fold the chained anchor events into one logical record with this projected
shape:

```text
ConversationLink {
  link_id: EventId, // owning event: initial bind or resolution event
  source: ConversationRef { conversation_id, thread_id? },
  target: ObjectRef { object_id, author_key },
  relation: "about" | "anchored-to-part" | "derived-from" | "resolved-into",
  part_anchor?: PartAnchor,
  target_revision_id?: RevisionId, // required for resolved-into edit results
  resolution?: AnchorResolution,
}

TalkPageBinding {
  binding_id: EventId,
  conversation_id: ConversationId,
  target: ObjectRef { object_id, author_key },
  state: canonical | disputed | unbound,
}
```

The source conversation is the group that contains the owning event;
`thread_id` is absent when the relation applies to the whole conversation.
`target` is an addressable ID object reference (object ID plus author key),
never a pasted URL or copied object title. A chat message/event is also an
addressable object and may be a target. `derived-from` targets the source chat
object (the source conversation or, for a message-level fork, its named
message/event). The link contains no target excerpt or preview. The four
relationship names in the table are the #5682/#5870 conversation-link
vocabulary:

| Relation           | Target                  | Part anchor | Meaning                                                                                                  |
| ------------------ | ----------------------- | ----------- | -------------------------------------------------------------------------------------------------------- |
| `about`            | ID object               | absent      | The conversation/thread discusses the object generally.                                                  |
| `anchored-to-part` | ID object               | required    | The conversation/thread refers to exactly one selected part of the object.                               |
| `derived-from`     | addressable chat object | absent      | The conversation/thread was forked or spun out of the source conversation/thread or named message/event. |
| `resolved-into`    | result object           | absent      | The discussion produced an addressable edit result; target revision identifies the exact edit/revision.  |

`about`, `anchored-to-part`, and `derived-from` edges are projections of
`chat.anchor.v1` bind/relocate events. A `resolved-into` edge is instead a
projection of a `chat.resolution.v1` event of kind `edit`, per #5874; its
`target_revision_id` is the named edit event/resulting revision. Do not also
emit a `chat.anchor.v1` edge for the same outcome. The `chat.resolution.v1`
fold owns re-open/re-resolve and concurrent-resolution behavior. A decision
summary or no-action resolution that has no addressable result object is not a
`resolved-into` edge; it remains a thread-resolution outcome, not a fabricated
object target. Proposal and approval mechanics remain with #5874.

Projecting a `resolved-into` edge requires the target `ObjectRef`, and the
mapping to it is guaranteed, not heuristic: the `chat.resolution.v1` edit
event names the target object and the resulting revision, and the
`author_key` is read from that object's own record. Where an object type
identifies edit results only by a stable logical ID, its adapter must
supply a deterministic mapping from that logical ID to the record
`(object_id, author_key)` pair as part of the identity contract below.
If no such mapping exists, the edge is not projected; the outcome stays
a thread-resolution outcome until the result has an addressable object
identity.

`talk-for` is a separate `chat.anchor.v1` binding projection, not a fifth
`ConversationLink` relation. #5872 already defines its single-canonical
binding and disputed-conflict fold; #5871 owns who may bind and the lifecycle
and visibility policy. This record neither adds a second talk-page relation
nor redefines that fold.

A conversation may have any number of links, including links to several
objects. Each part link names one selector; use another binding event for
another selected part. This keeps one selector's lifecycle independent of the
others. The initial `chat.anchor.v1` bind event ID is the stable `link_id`;
unbind and relocation events refer to it. For `resolved-into`, the
`chat.resolution.v1` event ID is the `link_id`. Object-to-conversation and
conversation-to-result navigation use reverse indexes over the owning folded
event set; do not write a duplicate reciprocal edge that could disagree.
Repeated delivery of the same event is idempotent, while distinct bind
events/selectors remain distinct links.

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

For references such as `id://<object-id>#<part-selector>`, serialize a
fragment-safe version-1 selector using these forms:

```text
pa1~r~<revision>~<part>~<start>~<end>
pa1~b~<revision>~<part>~<block>
pa1~g~<revision>~0~<selector-type>~<value>
pa1~g~<revision>~1~<part>~<selector-type>~<value>
```

`r`, `b`, and `g` mean range, block, and region. The revision, part, block,
and selector-type fields are UTF-8 percent-encoded, leaving only ASCII
alphanumerics, `.`, `_`, and `-` unescaped. Inside a field, a literal `~`
is written `~~` and a literal `%` is written `%25`; the field encoding
never uses `%7E`. Parsing scans a field left to right: `~~` decodes to
one literal `~`, a lone `~` is the separator, and percent-decoding runs
exactly once, only after the fields have been split. Because no field
can contain `%7E`, a URL normalizer that decodes percent-encoded
unreserved characters cannot manufacture a separator inside a field,
and a fragment whose field count does not match the form above is
malformed and must be rejected, never parsed leniently. In the region
form, `0` means there is no part ID and `1` means the next field is a part ID; no sentinel is
overloaded as a valid ID. Range offsets are decimal Unicode-scalar values. The
region `value` is the unpadded base64url encoding of the object type's
canonical UTF-8 selector value. This grammar is safe in a URL fragment and has
a stable parse; the target object type defines the canonical region-value
schema. The resolver fingerprint is deliberately not part of this public
selector.

The optional fingerprint is private resolver metadata, not a quote. It may be
computed from selected content and bounded context, but it must be opaque and
available only to the authorized resolver. Do not put raw selected text,
object titles, or previews in a link record or expose a fingerprint as a public
selector. The object type remains responsible for the canonical text and
revision mapping its selector uses.

#### Object identity and revision contract

`ObjectRef` identifies one addressable object record; `origin_revision_id`
identifies the exact immutable version selected. An object type that supports
anchors following edits MUST expose a stable logical object identity across
revisions, immutable revision IDs, and an ordered revision lineage. It must
also provide a deterministic selector mapping or enough authorized resolver
material to attempt exact relocation. Do not infer a logical identity by
matching content hashes or similar text.

Some ID object records use the signed event ID as their object ID, with each
save creating a new event ID (the document model described in #2259 is an
example). Such an ID is version-specific unless that object type separately
defines a stable logical ID. `about` may target any addressable ID object. A
part anchor can follow later edits only when the object's adapter supplies the
identity/revision contract above, including, for record-per-save models, a
deterministic mapping from each new record object ID to the same stable
logical ID; otherwise the anchor is pinned to that version and cannot
automatically follow a new object ID.

### Edit, version, and delete survival

The original `origin_revision_id` and selector are immutable. For a target at a
later revision, expose a separate resolution projection for that revision;
never rewrite the original selector to make a link appear current. Resolution
uses these states and rules:

1. If the object supplies a deterministic revision map and stable part/block
   identity maps the selection exactly, resolve it as **current** with no
   warning.
2. Otherwise, if the resolver finds exactly one match for the opaque content
   fingerprint in the target revision, resolve it as
   **relocated** and show a non-blocking “anchor moved” warning. Candidate
   counting is scoped to the target revision: matches in other revisions
   of the lineage neither add candidates nor make this one ambiguous.
   Preserve both original and resolved selectors. Confirmed relocation is
   recorded by a new `chat.anchor.v1` event; the initial binding event is
   never rewritten.
3. If only an approximate match exists, or more than one candidate exists,
   mark it **drifted**. Warn, retain the original anchor, and require a person
   or object-specific resolver to confirm a candidate before navigation
   retargets. Candidate details are visible only to readers of the object.
4. If the resolver lacks enough evidence to determine whether the selector
   moved or disappeared—for example, there is no revision map or fingerprint,
   or the origin revision is no longer retained—mark it **unresolved**. Warn
   that the anchor cannot be verified; do not navigate to a guessed current
   location or call it orphaned. If the origin revision remains available and
   authorized, it may still be opened directly. A person may explicitly
   re-anchor it with a later relocation event.
5. If the object's revision history proves the selected part or range was
   removed and no candidate remains, mark it **orphaned**. Keep the link and
   conversation; do not guess a new target.
6. If the whole object is deleted, mark its target **tombstoned**. Keep the
   link and conversation, but do not preserve a content preview or expose the
   tombstone to a caller who cannot read the target. A terminal client may
   render a deleted marker only after the authorized view includes the link;
   “always visible” never overrides the private-object non-disclosure rule.
7. If concurrent `chat.anchor.v1` mutations leave multiple maximal
   operations for one `link_id`, mark it **disputed**. Preserve the competing
   states, warn authorized readers, and do not choose a selector or navigate
   until a later mutation causally observes all competing heads and resolves
   them.
8. An explicit, causally unambiguous unbind makes the link **unbound**: exclude
   it from the active relation index but retain its event history. Rebinding
   creates a new `link_id`; a relocation cannot resurrect an unbound link.

An anchor follows revisions of the same stable logical object identity only. A
copied object, fork with a new object ID, or unrelated object that happens to
contain the same text does not inherit the link automatically. When the
original revision is retained and authorized, clients may offer it as an
explicit destination; otherwise navigation uses the latest confirmed
resolution, or no destination when the state is unresolved, orphaned,
tombstoned, disputed, or unbound. A change in anchor status never deletes the
conversation or changes its message history.

### Event, client, and privacy boundary

The durable source of `about`, `anchored-to-part`, `derived-from`, and
`talk-for` mutations is the additive `chat.anchor.v1` chained-permanent event
in the conversation's existing group, as decided by #5872. Ordinary link
events bind, unbind, or re-locate an edge and carry the object ID plus author
key, relation, and by-reference selector when the relation is
`anchored-to-part`; they never embed object content or a capability.
`resolved-into` is not an anchor event: it is projected only from
`chat.resolution.v1` edit events under #5874. The event log(s) are authoritative;
`ConversationLink`, `TalkPageBinding`, current resolution, and reverse indexes
are deterministic projections.

Link events replicate with the conversation group's existing event
stream, exactly as the other chained events in that group do; this
record defines no new replication transport and does not rely on a
#2036 application metadata/extension path, which #2036 does not define
for link records.

The existing `ChatLink` in `chat.message.v1` may also carry the `object` kind
selected by #5872 for an in-message object reference or a recognized pasted
link. That message link is not a substitute for a conversation/thread anchor
event. Ordinary `chat.message.v1` messages remain unchanged, and clients that
do not understand the anchor event can continue reading and sending ordinary
chat messages under #5872's unknown-event tolerance invariant.

For ordinary links, fold mutations by causal ancestry in the event's `after`
frontier, not by arrival time or a presentation sort. A causally later event
supersedes only the operations it observes. If two unbind/relocate operations
are concurrent maximal heads for one `link_id`, preserve both as `disputed`;
#5872's event-ID tie-break orders the fold deterministically but is never
read as choosing one mutation. A resolving mutation must causally observe all
current heads. An unbind that has been resolved cannot be undone by a
relocation; a fresh bind receives a new `link_id`. `talk-for`
conflicts use #5872's separate single-canonical/disputed fold, and
`resolved-into` concurrency uses #5874's resolution fold.

Apply authorization before returning a link or any projection of it. This
includes its ID, endpoint IDs, relationship, part selector, fingerprint,
resolution status, candidate count, reverse-index result, notification, and
search hit. If the caller cannot read the target object, return no indication
that the edge or target exists. Link metadata does not grant target access or
change conversation membership. The parent sets the non-disclosure invariant;
the mixed-access conversation/talk-page visibility policy belongs to #5871,
not this record. Implementations must ensure that transcript, title, preview,
and notification rendering do not disclose the private target. The “always
visible” tombstone rendering in the #5873 terminal design applies only after
the authorized view includes the link; it never means that non-readers may
learn a private target exists.

## Alternatives rejected

| Alternative                                                                            | Reason for rejection                                                                                                                                                                                                                  |
| -------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| A pasted URL in a message                                                              | Not a first-class relationship, cannot reliably support reverse navigation, and has no typed selector or lifecycle. URLs may be kept as user-authored content but are not the canonical link.                                         |
| Store anchor state only in `chat.message.v1` or require a new mandatory envelope field | Couples link lifecycle to message history, makes multi-object/thread links awkward, and risks exposing private target metadata through ordinary transcript delivery. Use the already-decided additive `chat.anchor.v1` event instead. |
| Store only offsets against the current object                                          | Edits and version changes silently retarget positions; the same offsets can point at unrelated content.                                                                                                                               |
| Store raw quote text in the link                                                       | Duplicates potentially private object content and can disclose it through sync, indexes, logs, or clients that lack object access.                                                                                                    |
| Always choose the “closest” or first relocation candidate                              | Approximate and ambiguous matches can point a discussion at the wrong content. Drift must be visible and confirmation required.                                                                                                       |
| Delete links when a part/object is deleted                                             | Breaks reverse history and disconnects a conversation that the parent design says must survive; use orphan and tombstone states instead.                                                                                              |
| Publish a tombstone or relation to every conversation reader                           | Reveals private-object existence; access checks must hide the edge itself, not merely its preview.                                                                                                                                    |
| Treat `talk-for` as an ordinary `about` link                                           | Loses #5872's one-canonical-talk-page binding and disputed-conflict semantics; keep a separate projection over its `chat.anchor.v1` binding.                                                                                          |
| Emit `resolved-into` as a `chat.anchor.v1` edge                                        | Conflicts with #5874, which derives result-edit edges from `chat.resolution.v1` and pins the exact edit/revision.                                                                                                                     |
| Resolve concurrent mutations with last-write-wins or event-ID order                    | Event arrival and presentation tie-breaks do not establish user intent; preserve concurrent maxima as disputed until a causally later event resolves them.                                                                            |
| One universal coordinate format for all object regions                                 | Text, media, tables, boards, and other object types have different semantics and units; typed object-native selectors are less ambiguous.                                                                                             |
| Duplicate a reverse link record                                                        | Creates two sources of truth that can disagree; derive reverse navigation from the canonical edge.                                                                                                                                    |
| Add another anchor wire format or change `chat.message.v1`                             | Duplicates the `chat.anchor.v1` event selected by #5872 or breaks v1 message compatibility; this record defines the selector carried by that event instead.                                                                           |

## Implementation invariants

Implementations of this decision must hold all of the following:

1. Every link has a typed source and object target and one of the four allowed
   relation/endpoint combinations above. Its `link_id` is the initial
   `chat.anchor.v1` bind event, or the owning `chat.resolution.v1` event for a
   `resolved-into` result. Bind, unbind, relocation, and resolution events are
   immutable and folded deterministically by their owning projection.
2. `anchored-to-part` has exactly one object target, one origin revision, and
   one valid selector. Ranges are non-empty half-open Unicode-scalar ranges;
   block and region selectors are valid for the target object type. Cross-edit
   following is available only when that type provides the stable
   identity/revision contract.
3. Re-delivery of an event is idempotent; separate bind events/selectors are
   not collapsed. Bidirectional navigation is derived from the same folded
   event history. `talk-for` is a separate binding projection, never a fifth
   ordinary conversation-link relation.
4. No link or anchor field, status, candidate, count, search result, or
   notification reveals a private target's content or existence to a
   non-reader. A link never grants access.
5. Original revision and selector are immutable. Cross-edit following is
   allowed only for object types with the stable identity/revision contract.
   Resolution is revision-bound derived state, and only an exact deterministic
   mapping or a unique exact fingerprint match within the target revision
   may retarget automatically.
6. Approximate or ambiguous candidate matches are `drifted`; insufficient
   evidence is `unresolved`; concurrent mutation heads are `disputed`. All
   three are visibly warned and never silently followed. Proven removal is
   `orphaned`; whole-object deletion is `tombstoned`; explicit unbind is
   `unbound` and removes the edge from active navigation without erasing its
   event history.
7. Orphaning or tombstoning a target does not delete the link, its conversation,
   or chat messages. No resolver follows content into a different logical
   object: following across changing record object IDs is permitted only
   where the identity contract maps them to the same stable logical
   identity, and content similarity alone never establishes that mapping.
8. Unknown or unsupported `chat.anchor.v1` events do not prevent a client from
   reading or sending ordinary `chat.message.v1` messages. `resolved-into`
   projection follows #5874's `chat.resolution.v1` fold; clients that render a
   link preserve its status and warning rather than presenting a stale
   selector as current. No `resolved-into` edge is duplicated in
   `chat.anchor.v1`.
9. Reverse lookup, search, and rendering apply object authorization before
   returning any existence-bearing result.
10. For one anchor `link_id`, a causally later mutation supersedes only the
    events it observes. Concurrent maximal mutations remain `disputed`;
    #5872's event-ID tie-break only makes the fold deterministic, never a
    semantic winner. An explicit resolving mutation must observe every
    competing head. A resolved unbind
    cannot be undone by relocation; rebinding creates a fresh `link_id`.
11. `talk-for` cardinality/conflict folding follows #5872, and target binding
    authority/visibility follows #5871; this link record does not duplicate or
    override either policy.

## Parent-decision check and implementation handoff

This record refines the first feature in #5682 and consumes #5872, #5873, and
#5874; it does not take on talk-page, thread-visibility, terminal, or edit-loop
policy. It applies the parent's private-object non-disclosure constraint to
link, tombstone, and resolver data; the terminal tombstone marker is never
visible to a non-reader.

One boundary is made explicit for the parent: #5682 describes `resolved-into`
as an edit/decision link, while #5874's selected event shape creates an
object-side `resolved-into` edge for an addressable edit result
(`chat.resolution.v1`, kind `edit`). A decision summary or no-action outcome
without an addressable result object remains a thread-resolution outcome, not
an object edge. This record does not alter #5874; if #5682 requires reverse
object navigation to non-object decision summaries as well, that requires a
parent decision and an addressable decision result.

No implementation code is included here. The existing implementation track
[#6222](https://github.com/developing-today/code/issues/6222) consumes the
#5870 link vocabulary and selector grammar along with the #5871 visibility
decision; its gates remain unchanged. This record does not create a duplicate
implementation issue or change those gates.
