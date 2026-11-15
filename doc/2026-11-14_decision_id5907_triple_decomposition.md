# Decision: triple decomposition rules for KDL statements

- **Date:** 2026-11-14
- **Status:** decision record — no implementation is included in this change.
- **Issue:** [#5907](https://github.com/developing-today/code/issues/5907),
  "id: design: triple decomposition rules for KDL statements (from #5688)".
- **Parent design:** [#5688](https://github.com/developing-today/code/issues/5688)
  — its §2.2 pipeline stages, §2.3 "namespaces and qualifiers: more
  triples, never special cases", §2.5 tagging, and §3.2 triple-table
  model are the frame this record fills in. The parent's decisions
  (KDL as the human presentation format, atomic triples as the
  canonical data model, ANSI SQL/PGQ as the query model, postcard as
  the wire format) are **not re-decided here**; §8 lists the two
  places this record refines a question the parent and the sibling
  records explicitly left open, and both refinements are additive.
- **Inputs cited:**
  - [#5906](https://github.com/developing-today/code/issues/5906) /
    `doc/2026-11-08_decision_id5906_kdl_json_contract.md` — the
    KDL ⇄ JSON canonical conversion contract. It fixes the canonical
    JSON of one authored compound statement as the provenance anchor
    (§2.2 stage 2) and explicitly leaves to this record: statement-node
    identity (content hash vs minted vs derived), whether each
    triple's provenance linkage is a column or triples, and the
    schema-side interpretation of annotation payloads.
  - [#5910](https://github.com/developing-today/code/issues/5910) /
    `doc/2026-11-08_decision_id5910_kdl_presentation_surfaces.md` —
    presentation surfaces. It promises nothing about store → KDL
    reconstruction, reimport idempotency, or statement identity, and
    assigns all three to this record (§3.3–§3.4 there).
  - [#5912](https://github.com/developing-today/code/issues/5912) /
    `doc/2026-11-06_decision_id_tagging_triples.md` — tag claims as
    atomic-triple projections. It fixes the claim-node shape
    (`subject` / `predicate` / `object` / `asserted-by` triples plus a
    live-claim projection) and explicitly leaves to this record: the
    exact names of the assertion predicates, tag-term ID creation,
    signer-key-to-ID mapping, and the encoding of typed values.
  - [#5908](https://github.com/developing-today/code/issues/5908)
    (payload boundary) — a sibling successor. The decision procedure
    for what stays attached as payload is **#5908's**, not this
    record's; §7 states the boundary this record assumes.
- **Evidence baseline:** `origin/main` at
  `0204dde6465eaa0559b7cdfebeab4b2762b612aa`, worktree
  `~/git/fleet/queue-16813-5907-triple-decomposition`, 2026-11-14.
  Verified at that baseline: `doc/` holds the landed sibling records
  (#5906, #5910, the #5912 tagging decision) and no decomposition
  record; #5688 and #5907 are open; no PR is linked from #5907. This
  record is the first decomposition design.

## Decision summary

| Question                             | Decision                                                                                                                                                                                                                                                                                                           |
| ------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Statement node identity              | A **minted ID** at ingest, one per authored compound statement per ingest event. The canonical-JSON content hash is recorded **as a triple on the node** (`anchor-hash`), never as the ID itself (§2)                                                                                                              |
| Reimport / update                    | Reimport of the same canonical JSON from the same source is **idempotent** — it resolves to the existing statement node via `(anchor-hash, asserted-by)` and adds nothing. An edited statement is a **new** statement node linked to its predecessor by a `supersedes` triple; statements are never mutated (§2.3) |
| Triple provenance linkage            | **Triples, not a column.** Every atomic fact is an assertion node with `subject` / `predicate` / `object` / `stated-in` triples — the #5912 claim shape, generalised. The live `(subject, predicate, object)` edge is a projection over assertion nodes (§3)                                                       |
| Assertion predicates                 | A small **core vocabulary**, settled in §3.2: `subject`, `predicate`, `object`, `stated-in`, `asserted-by`, `asserted-at`, `index`, `supersedes`, `is-a`, `in-namespace`, `anchor-hash`, `label`, `has-key`, `dtype`, `literal` — predicate IDs in the core namespace, themselves ordinary IDs                     |
| N-ary facts                          | A statement node (ID'd) typed by an `is-a` triple, plus one binary triple per participant hanging off it, with role predicates taken from vocabulary IDs. The headline `(subject, action, target)` fact is the projection, not a privileged storage form (§4)                                                      |
| Namespaces                           | A term's namespace is a triple: `(term, in-namespace, namespace)`. Qualified names resolve to term IDs at ingest; the string form is presentation (#5910), never identity. Same string, two namespaces → two term IDs (§5)                                                                                         |
| Qualifiers                           | Triples on the **assertion node** (time, place, certainty, source, role), with vocabulary predicate IDs — never columns, tables, or parser branches (§6)                                                                                                                                                           |
| Typed values (non-ID objects)        | A literal is a **value node** whose ID is **derived** from `(dtype, canonical scalar form)` — equal literals are the same node, so joins over values work. A value node carries `dtype` and `literal` triples. Statement IDs are minted because claims differ; value IDs are derived because values don't (§7)     |
| Tag terms / signer keys (from #5912) | A tag term's ID is minted once per `(namespace, canonical string)` and recorded with `label` + `in-namespace` triples; vocabularies are data. A signer is an identity ID; the key mapping is a `(identity, has-key, key-value-node)` triple asserted at registration — decomposition never invents it (§7.3)       |
| Re-composition (for #5910)           | The statement node is the grouping boundary, and `index` triples give the order. Export groups by statement node; any grouping beyond that is presentation heuristics, not identity (§9)                                                                                                                           |

## 1. What the parent already fixed (not re-decided here)

#5688 fixes, and this record takes as given:

1. **The pipeline is parse → canonical JSON → atomic triples →
   resolve through IDs** (§2.2). Stage 2's canonical JSON is the
   audit / provenance anchor; stage 4's triples reference IDs, not
   embedded copies.
2. **Decomposition is into `(subject, action, target)` triples** —
   one fact each — and namespaces and qualifiers are **more triples,
   never special cases** (§2.3), including its rule for implementers:
   a proposed special column, table, or parser branch for a
   qualifier or namespace is a smell; an exception needs explicit
   justification in its follow-on ticket.
3. **An n-ary fact decomposes into a statement node (itself ID'd)
   plus binary triples hanging off it** (§2.3). This record's job is
   to give that sentence operational content: identity, naming,
   ordering, and invariants.
4. **Tags are ordinary triples over IDs and vocabularies are data**
   (§2.5), with the #5912 claim/projection shape as the compatibility
   seam.
5. **The triple table is the storage fact** (§3.2): subject ID,
   predicate ID, object ID, "plus the statement/provenance linkage
   from §2.2" — the _form_ of that linkage is one of the questions
   #5906 left to this record, settled in §3.1.

## 2. Statement nodes and their identity

### 2.1 One node per authored compound statement

Decomposition operates on one stage-2 canonical JSON document at a
time. Each such document produces exactly **one statement node**:
the ID'd anchor that every assertion decomposed from that document
points back to with `stated-in` (§3). The statement node is the unit
of grouping, provenance, retraction, and re-composition. It is not
the document: the document is data (kept as the anchor payload,
under #5908's boundary); the node is the claim _that this statement
was asserted_, by someone, at some time.

### 2.2 Identity: minted, with the content hash as a triple

A statement node's ID is **minted at ingest** — a fresh ID from the
same ID space as every other entity — and the node carries, at
minimum:

```text
stmt --anchor-hash--> hash-value-node   (the #5906 canonical-JSON content hash)
stmt --asserted-by--> signer-ID         (when the ingest is signed)
stmt --asserted-at--> time-value-node
stmt --is-a--> statement-type-ID
```

**Why not the content hash as the ID** (the candidate #5906 §5
raised): identity-by-content collapses claims that must stay
distinct. The same compound statement asserted by two people, or by
one person at two times, is two claims with different provenance,
different retraction behaviour, and potentially different qualifier
sets — the tagging decision (#5912) already depends on exactly this
distinction for tag claims, and §2.3 of the parent makes the
statement node the thing qualifiers hang off. A hash also cannot
represent "the same statement, corrected": an edit would either
keep the ID and mutate history, or change the ID and lose the link.
Minted identity plus a recorded hash gives both: the hash is data
you can join and dedupe on; the ID is identity you can retract and
supersede. (Literals are the opposite case and do use derived IDs —
§7.1 states the principle that separates them.)

### 2.3 Reimport is idempotent; updates are new statements

- **Reimport.** Ingesting canonical JSON whose `anchor-hash` and
  `asserted-by` match an existing statement node is a no-op: the
  ingest resolves to that node and asserts nothing new. This is the
  reimport-idempotency answer #5910 §3.3 asked for, and it is keyed
  on `(anchor-hash, asserted-by)` — the same bytes from a _different_
  signer are a different claim (§2.2) and mint a new node.
- **Update.** An edited statement mints a new statement node and a
  `supersedes` triple `(new-stmt, supersedes, old-stmt)`. The old
  node's assertions are never mutated or deleted; projections (§3.3)
  follow the supersedes chain. Retraction (e.g. `tag.clear`, #5912)
  is likewise a new statement naming the assertion nodes it
  retracts, never a delete.

## 3. Assertion nodes: how a triple carries its provenance

### 3.1 The shape

Every atomic fact decomposed from a statement is stored as an
**assertion node** `a` with exactly these structural triples:

```text
a --subject-->   subject-ID
a --predicate--> predicate-ID
a --object-->    object-ID        (an entity ID, or a value node — §7)
a --stated-in--> statement-node-ID
```

plus, where they exist, `a --index--> n` (§4.3) and qualifier
triples on `a` (§6). This is the #5912 claim shape with
`stated-in` added and `asserted-by` / `asserted-at` carried on the
statement node (all assertions in one statement share a signer and
a time; repeating them per assertion would be denormalisation, and
#5912's per-claim `asserted-by` is preserved by resolution through
`stated-in`).

The linkage is **triples, not a `statement_id` column** — the
alternative #5906 left open. Three reasons. First, the parent's
§2.3 rule: a provenance column on the triple table is precisely a
special case that only one kind of metadata gets, and the next
kind (qualifiers) would need another. Second, the query model:
SQL/PGQ views (§3.2 of the parent) are defined over the triple
table; provenance that lives in triples is visible to the same
queries that traverse facts, while a column needs a second,
non-graph access path. Third, the #5912 projection already works
this way for the one claim family that exists today; a column
would fork tag claims from every other fact.

### 3.2 The core predicate vocabulary

The structural predicates are a fixed, small vocabulary. Their
names — the question #5912 deferred here — are:

| Predicate      | Meaning                                                          |
| -------------- | ---------------------------------------------------------------- |
| `subject`      | assertion node → its subject ID                                  |
| `predicate`    | assertion node → its predicate ID                                |
| `object`       | assertion node → its object ID or value node                     |
| `stated-in`    | assertion node → its statement node                              |
| `asserted-by`  | statement node → signer identity ID                              |
| `asserted-at`  | statement node → time value node                                 |
| `index`        | assertion node → ordinal value node (order within the statement) |
| `supersedes`   | statement node → the statement node it replaces                  |
| `is-a`         | any node → its type / activity / class ID                        |
| `in-namespace` | term ID → its namespace ID                                       |
| `anchor-hash`  | statement node → canonical-JSON hash value node                  |
| `label`        | term ID → its canonical display string value node                |
| `has-key`      | identity ID → its signing-key value node                         |
| `dtype`        | value node → its scalar type ID                                  |
| `literal`      | value node → its canonical scalar string value node              |

These names are the spellings in this record and in #5912; as data
they are predicate **IDs in the core namespace**, minted once, and
the strings are their `label`s. Domain predicates (the `predicate`
values of ordinary facts — `did`, `with`, `tagged-with`, …) are
**not** core vocabulary: they are vocabulary data (§5), created and
namespaced like any other term. The core list changes only by a new
decision record; a decomposition that needs a sixteenth structural
predicate has found a design question, not an implementation
detail.

### 3.3 The projection

The ordinary edge `(subject-ID, predicate-ID, object-ID)` is a
**projection**: it is visible when at least one assertion node with
those `subject` / `predicate` / `object` triples is _live_ — its
statement is not superseded on that assertion's chain and it has
not been retracted — exactly the live-claim rule #5912 defined for
tags, generalised to all facts. The projection retains its
supporting assertion IDs (and through them the statement, signer,
and qualifiers) as provenance. Storage never writes projection
edges directly; they are derived, and any cache of them is
rebuildable from assertion nodes alone.

## 4. N-ary decomposition

### 4.1 The rule

A compound statement whose fact has more than two participants, or
whose participants play distinct roles, decomposes as:

```text
stmt-fact --is-a--> activity-type-ID
stmt-fact --<role-1>--> participant-ID
stmt-fact --<role-2>--> participant-ID
...
```

where `stmt-fact` is a fact node minted for this occurrence (it is
the statement node's principal assertion subject; for a statement
carrying several n-ary facts, one fact node each, distinguished by
`index`). Role names are predicate IDs from vocabulary data — the
authored KDL's node/property names, resolved per §5 — never core
vocabulary and never columns. The binary case is not special: a
two-participant fact is an assertion whose `subject` and `object`
are the participants directly (§3.1), and an n-ary fact's headline
edge `(primary-participant, activity, …)` appears only in the
projection if a corresponding binary assertion exists. What the
graph query sees for an n-ary fact is the fact node and its role
edges; that _is_ the decomposition the parent §2.3 specified, with
the names filled in.

### 4.2 Worked example: Drew's person/activity statement

The parent's canonical shape (§2.1 of #5688):

> this person did activity with other person; other activity with
> other person; third activity with three people

One authored statement, three activities, the third with three
participants besides the actor. Decomposition (IDs abbreviated;
`p1` is "this person"; every assertion also carries `stated-in -->
stmt-1`, elided in the fact listing and shown once below):

```text
# the statement node
stmt-1 --anchor-hash--> hash("…canonical JSON…")
stmt-1 --asserted-by--> p1
stmt-1 --asserted-at--> 2026-… (value node)

# fact 1: p1 did activity A with p2 — binary, one assertion node
a1 --subject--> p1        a1 --predicate--> did
a1 --object--> act-A      a1 --stated-in--> stmt-1   a1 --index--> 1
a2 --subject--> act-A     a2 --predicate--> with
a2 --object--> p2         a2 --stated-in--> stmt-1   a2 --index--> 1

# fact 2: p1 did activity B with p3 — same shape, index 2

# fact 3: p1 did activity C with p2, p3, p4 — n-ary, a fact node
f3 --is-a--> act-C
a3 --subject--> f3        a3 --predicate--> by
a3 --object--> p1         a3 --stated-in--> stmt-1   a3 --index--> 3
a4 --subject--> f3        a4 --predicate--> with
a4 --object--> p2         … index 3
a5 --subject--> f3        a5 --predicate--> with
a5 --object--> p3         … index 3
a6 --subject--> f3        a6 --predicate--> with
a6 --object--> p4         … index 3
```

The grouping the naive decomposition would lose (§2.1 of the
parent) survives twice over: `stated-in` ties every assertion to
the one authored statement, and fact 3's participants share the
fact node `f3` rather than fanning out into independent
person–activity edges that could be read as three separate
activities. A qualifier on fact 3 alone — say it happened at a
place — is a triple on `f3`'s assertions or on `f3` itself (§6),
not on the whole statement and not on any participant.

### 4.3 Ordering

Authored order is load-bearing where the author expressed a
sequence (the example's three activities are an ordered telling).
Each assertion carries an `index` triple whose object is the
ordinal value node for its position in the statement's
decomposition. Re-composition (§9) and any "what was said, in
order" view sort by it. Indexes are per-statement, start at 1, and
are assigned in decomposition order; two assertions of one n-ary
fact share its index.

## 5. Namespaces

- A **namespace** is an ID'd entity like any other. A term's
  membership is one triple: `(term, in-namespace, namespace)`.
- A qualified name in authored KDL (`ns:term`, or the fork's
  annotation forms — conversion is #5906's contract, opaque at this
  layer) resolves at ingest to the term ID recorded for that
  `(namespace, canonical string)` pair, minting the term — and its
  `label` and `in-namespace` triples — on first use. This is the
  tag-term creation rule #5912 asked for, stated for all terms:
  **identity is the pair; the string alone is never an ID**, so
  equal strings in different vocabularies are different terms, and
  renaming a display string touches one `label` triple, not any
  identity.
- Unqualified names resolve against the statement's default
  namespace, which is itself recorded as a triple on the statement
  node rather than living in parser state — a decomposition must be
  reproducible from the store plus the canonical JSON alone.
- Predicates are terms. `did`, `with`, `by`, `tagged-with` in the
  examples are vocabulary IDs resolved exactly as above; only the
  §3.2 core predicates are fixed, and they live in the core
  namespace by the same mechanism.

## 6. Qualifiers

A qualifier — time, place, certainty, source, role-in-activity,
or any future kind — is **triples on the assertion node** (or, for
a qualifier of a whole n-ary fact, on its fact node):

```text
a4 --at-place--> place-ID
a4 --certainty--> 0.8 (value node)
```

The qualifier predicates are vocabulary IDs (§5). The rules:

1. **No qualifier gets a column, a table, or a parser branch** —
   the parent's §2.3 smell rule, made a conformance test for
   implementations: the triple table's schema does not change when
   a new qualifier kind appears.
2. A qualifier qualifies **one assertion** by default. Qualifying a
   whole statement is a triple on the statement node; qualifying a
   participant's role is a triple on the role assertion. The three
   scopes are distinct and never inferred from one another.
3. A qualifier that itself needs qualifying (a certainty about a
   time, a source for a place) is an assertion about the assertion
   node — assertion nodes are IDs, so this is ordinary triples, not
   a meta-level. Depth is data, not schema.

## 7. Typed values, payloads, and the #5912 leftovers

### 7.1 Value nodes: derived identity for literals

Stage 4 says triples reference IDs — but a time, a number, a short
string is not an entity anyone minted. A literal object is
therefore a **value node** whose ID is **derived deterministically
from `(dtype, canonical scalar form)`**, where the canonical form
is #5906's typed-scalar representation. The node carries:

```text
v --dtype--> integer-type-ID
v --literal--> "4549" (a further value node only if needed; in
             practice literal carries the canonical string as the
             node's payload — see #5908 boundary note below)
```

The asymmetry with §2.2 is deliberate and is the principle:
**claims are minted because two assertions of the same content are
different facts about the world; values are derived because two
occurrences of the same literal are the same value.** Derived
value IDs make equality joins over times, amounts, and ordinals
(the `index` objects of §4.3) work without a value registry, and
they make decomposition deterministic: the same canonical JSON
always yields the same value nodes.

### 7.2 The payload boundary (assumed, not decided)

Long text bodies, images, geometries, documents stay **attached
payloads** per the parent §2.4 — referenced by triples, never
flattened. Where exactly the line falls, and the decision procedure
for grey cases, is **#5908's** record. This record assumes only:
a payload hangs off an ID'd node (statement, fact, or value node),
and decomposition never reaches _inside_ a payload. The `literal`
carrying of §7.1 is for short canonical scalars in #5906's typed
forms, not a payload channel.

### 7.3 Signer-key mapping (from #5912)

A signer is an **identity ID**. The binding of a signing key to
that identity is a triple `(identity, has-key, key-value-node)`
asserted by the registration act that created the identity — it is
part of the graph, queryable and supersede-able like anything
else. Decomposition **never** derives an identity from a key, and
never mints one: an ingest whose signer has no identity record
fails validation at stage 1 of the pipeline, with an error the
author can act on. Key rotation is a `supersedes` chain on the
`has-key` assertions, not a change to the identity.

### 7.4 Annotation payloads (schema side, from #5906)

The conversion layer carries structured annotation payloads
opaquely (#5906 §3.4.2). At this layer: an annotation on an
authored node is interpreted by the statement-shape validator as
either a qualifier (§6 — the common case: time, certainty, source
annotations become qualifier triples on the resulting assertions)
or a namespace signal (§5). An annotation the validator does not
recognise is a stage-1 validation error, never silently dropped
and never stored as a pseudo-triple.

## 8. Parent-decision impact

**No parent decision changes.** Two questions the parent and
siblings left open are settled, and both settlements are the
additive kind #5906's §9 pattern allows:

1. **Statement/provenance linkage form** — the parent §3.2 wrote
   "(plus the statement/provenance linkage from §2.2)" without
   fixing its form, and #5906 named "column or anchor's hash" as
   this record's call. §3.1 chooses triples. Flagged explicitly
   because a reader of §3.2 could have assumed a column: none is
   created, and an implementation that adds one for provenance is
   non-conformant with this record (a _cache_ rebuilt from triples
   is not a column in this sense).
2. **Statement identity candidate** — #5906 fixed the anchor and
   called its hash "the statement node's identity candidate,
   decided in #5907". §2.2 declines the candidacy: the hash is a
   triple, the ID is minted. The anchor's role in #5906 is
   unchanged.

Everything else here — assertion shape, core predicate names,
value-node derivation, namespace and qualifier rules — fills space
the parent deliberately left blank.

## 9. Re-composition and export (for #5910)

- The **statement node is the grouping boundary** for any
  store → KDL reconstruction: one statement node re-composes to
  one authored statement, its assertions ordered by `index`, its
  n-ary facts re-grouped around their fact nodes.
- A KDL document containing several statements is a presentation
  concatenation of statement reconstructions; document boundaries
  are not identity (#5910 §3.4's grouping question, answered at the
  level this record owns: grouping _within_ a statement is
  decomposition's; grouping _across_ statements is presentation).
- Re-composition reads the **live projection plus provenance**:
  superseded or retracted assertions do not re-export as current
  facts. Exporting history (the superseded chain itself) is a
  distinct, explicitly requested operation, not a flag on ordinary
  export.

## 10. Invariants

An implementation of decomposition conforms only if all of these
hold:

1. **No unprovenanced facts.** Every assertion node has `subject`,
   `predicate`, `object`, and `stated-in` triples; every statement
   node has an `anchor-hash` triple. There is no write path that
   creates a bare stored triple.
2. **No special cases.** Namespaces and qualifiers are triples.
   The triple table's schema, the parser, and the store gain no
   column, table, or branch when a new qualifier kind, namespace,
   or domain predicate appears.
3. **Determinism modulo minted IDs.** Decomposing the same
   canonical JSON against the same vocabulary state yields the
   same assertion structure, the same value nodes, and the same
   term IDs; only statement, assertion, and fact node IDs differ.
4. **Immutability.** Statements and assertions are never mutated
   or deleted. Correction is `supersedes`; retraction is a new
   statement; the projection is a pure function of the assertion
   set.
5. **Order survives.** `index` triples reproduce the authored
   order of facts within a statement after any store round trip.
6. **Identity is not a string.** No entity, term, predicate, or
   namespace is identified by its display string anywhere in the
   store; strings appear only as value nodes or payloads.
7. **Payloads are opaque.** Decomposition never reads inside an
   attached payload (#5908 boundary), and no payload content
   becomes a triple by a side path.
8. **Core vocabulary is closed.** The §3.2 predicates are the only
   structural predicates; extending the list requires a decision
   record, not an implementation choice.

## Alternatives considered

1. **Content-hash statement identity.** Rejected (§2.2): collapses
   distinct claims by different signers or at different times,
   breaks the #5912 retraction model, and cannot represent a
   correction without either mutating history or losing the link.
2. **Bare triples plus a `statement_id` column.** Rejected (§3.1):
   a provenance special case the parent's §2.3 rule names as a
   smell; invisible to graph queries; forks tag claims (already
   node-shaped) from all other facts.
3. **Quads / a named-graph store (subject, predicate, object,
   graph).** Rejected: the parent fixed a _triple_ table with
   SQL/PGQ views over it (§3.2); a quad store is a different
   storage commitment, and the assertion-node shape expresses the
   same provenance inside the model the parent chose.
4. **Blank nodes for n-ary facts and qualifiers** (RDF-style
   reification without identity). Rejected: what has no ID cannot
   be qualified, retracted, superseded, or pointed at by another
   statement — the worked example's fact 3 needs all four.
5. **Per-qualifier columns on a statement table** (a `time` column,
   a `place` column, …). Rejected: the canonical §2.3 smell; every
   new qualifier kind becomes a migration, and qualifier-of-
   qualifier (§6 rule 3) is inexpressible.
6. **Namespace as an ID prefix** (`ns:term` string IDs). Rejected:
   identity-by-string (§10.6), unrenamable, and it smuggles a
   second naming system into the ID space; §5 gets the same
   disambiguation with ordinary triples.
7. **Minted IDs for literal values** (a value registry). Rejected
   (§7.1): equality of literals would depend on registry state,
   decomposition would stop being deterministic, and identical
   times/amounts in different statements would fail to join.
8. **One triple per authored sentence** (no sub-statement
   decomposition). Rejected by the parent itself (§2.2 stage 3:
   "one triple per person–activity–person fact, not one triple per
   sentence"); listed here because it is the alternative every
   implementer reaches for first.

## Scope and follow-ons

Design only. No code, schema, or migration is included in this
change, and implementation timing remains the separate decision
#5907's filing note reserves. Open items this record deliberately
does **not** settle: the payload-boundary decision procedure
(#5908), the SQL/PGQ engine capability matrix (the parent's §3.3
follow-on), and presentation grouping across statements (#5910).
On merge, #5907 closes with this record as the decision, per the
#5906 / PR #7593 convention.
