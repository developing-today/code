# Decision record — KDL fork, #5753 §5 design pass (payload grammar, extension marking, serde shape)

- Issue: developing-today/code#5753 §5 (follow-on questions for the design pass)
- Parent design: developing-today/code#5688 (KDL → JSON → atomic triples → SQL/PGQ; conversion contract §2)
- Fork base: `kdl-org/kdl-rs`, the `kdl` crate v6.7.1 (selected in #5753 §3)
- Evidence base: `pkgs/mappy/migrations/limit.kdl` (Drew, 2023-12-17; quoted in #5753 §1.1),
  upstream Discussions kdl-org/kdl#344 and #362, #5753 §4 commitments (Drew, 2026-11-04)
- Queue task: 16766 (design-record follow-on from triage task 15229)
- Date: 2026-11-16

This record DECIDES §5 questions 1–3 and records recommendations on 4–5.
Each decision is stated once, as a position, with the rationale tied to
the evidence above. The §4 commitments bind throughout: (a) compatibility
is absolute — extensions are additive and never change the meaning of a
spec-valid document; (b) annotations are universal, multiple, and
structured; (c) performance is first-class; (d) the long-term artifact is
our document language with KDL as its default text form.

---

## Q1 — Annotation payload grammar: DECIDED

**Decision: two layers. The payload's surface syntax stays exactly the
spec's annotation slot — one identifier in parentheses. Structure is a
conversion-layer convention, never parser grammar.**

### 1.1 Surface layer (parser)

The fork generalizes annotation _placement and multiplicity_ only:
annotations may appear on every part (node name, each argument, property
keys, property values, the children block — the #344/#362 asks) and a
part may carry several. What sits inside the parentheses remains a
single KDL identifier, character-for-character the spec's type slot.

Rationale: any internal sub-grammar (treating `#` or `:` as syntax
inside the parens) would change the meaning of documents that are
spec-valid today, where those characters are ordinary identifier
characters — a direct §4a violation. It would also make every payload
unreadable to strict tooling even when the document uses no other
extension. `limit.kdl`'s packing (`(object#book#author:BrianGriffin)`,
`(yes:yes:no:::bool)`) is _authoring practice inside an opaque string_,
and the spec-shaped surface keeps that practice fully expressible.

### 1.2 Structure layer (converter): the parameter convention

The #5688 converter interprets a payload identifier by convention:

```
payload  := path ( "#" segment )*
path     := name ( ":" name )*          ; namespace path, e.g. document:object
segment  := [ key ":" ] value           ; keyed or positional parameter
```

- `(object#book#author:BrianGriffin)` → path `object`; parameters:
  positional `book`, keyed `author = BrianGriffin`.
- `(document:object)rdf` → path `document:object`, no parameters.
- `(yes:yes:no:::bool)` → path segments `yes`, `yes`, `no`, `, `,
  `bool` (the string before the first `#`, split on `:`). Empty path
  segments are legal and preserved; the example is a torture case from
  `limit.kdl`, not a shape to optimize for — and the raw-payload rule
  in §1.3 makes the decomposition lossless regardless.

A payload with no `#` and no `:` is a bare name — the common case, and
identical to a spec type hint.

### 1.3 Triple decomposition (under #5688 §2)

Per #5688 §2.3 — namespaces and qualifiers are _more triples, never
special cases_ — an annotation occurrence decomposes as:

- The annotated part is (or hangs off) a statement node `S` in the
  triple stream. For each annotation occurrence `A` on that part, in
  source order:
  - `(S, annotation, A)` and `(A, position, <i>)` (order is data, not
    encoding);
  - `(A, name, <last path segment>)`; one `(A, namespace, <segment>)`
    triple per preceding path segment, in order — a namespace is a
    chain of triples, not a string with a separator;
  - per parameter: `(A, param, <value>)` for positional parameters
    (with `(A, param-position, <i>)` on a per-parameter node when more
    than one positional parameter exists), `(A, param:<key>, <value>)`
    for keyed parameters — the predicate is data-derived exactly as
    #5688 §2.3 prescribes for qualifiers;
  - `(A, payload, "<raw identifier>")` — **always**. Per #5688 §2.4's
    payload boundary, the raw payload is carried verbatim: decomposition
    is additive, re-rendering is exact, and a convention revision never
    loses information that was in the document.

Worked example — `(subject)character "Brian"` with
`(predicate#NotVeryWell)"writesBooks"` from `limit.kdl`: the argument
`"writesBooks"` carries occurrence `A` with `(A, name, predicate)`,
`(A, param, NotVeryWell)`, `(A, payload, "predicate#NotVeryWell")`.

## Q2 — Extension marking: DECIDED

**Decision: yes — an extended document declares itself with a
declaration node as its first node. The declaration is for consumers,
not for our parser.**

```kdl
kdlx version=1 extensions="multi-annotation prop-key-annotation children-annotation"
```

- The node name `kdlx` and the `version` / `extensions` properties are
  the whole mechanism. The extension tokens are the closed list this
  record defines (`multi-annotation`, `prop-key-annotation`,
  `children-annotation`); a new extension mints a token in its own
  decision record.
- A strict-KDL consumer detects extension use by reading the first node
  only — O(first node), no full parse — and can warn precisely
  ("this document declares prop-key-annotation; strict KDL will reject
  or drop it") _before_ choking, which is what §5 Q2 asks for.
- Spec-valid documents carry no declaration and need none (§4a).
- Our parser does not require the declaration: extension syntax parses
  with or without it. A document that _uses_ extension syntax without
  declaring is a lint warning in our tooling, not an error — the
  declaration protects other consumers, and making it load-bearing for
  our own parse would punish hand-authored files for a bookkeeping
  omission.
- In the triple stream the declaration is document metadata, not
  content: `(doc, kdlx-version, 1)` and one `(doc, extension, "<token>")`
  per token; the node itself emits no statement triples. In the §4d
  JSON/YAML renderings it surfaces as a top-level `"$kdlx"` object, so
  the declaration survives conversion out of KDL text form.

Rejected alternative: a document-level annotation or header comment.
KDL has no document-annotation slot in either spec version (inventing
one is a bigger grammar change than any extension it would mark), and
comments are non-semantic — tooling that warns based on comments is
tooling that lies when the comment is edited.

## Q3 — Serde shape: DECIDED

AST change first (the contained change #5753 §3 identified): in the
fork, `KdlNode.ty: Option<KdlIdentifier>` and
`KdlEntry.ty: Option<KdlIdentifier>` become
`annotations: Vec<KdlIdentifier>`; property keys gain the slot the spec
AST lacks — an entry's key is a `KdlIdentifier` and gains its own
`key_annotations: Vec<KdlIdentifier>`; the children block gains
`children_annotations: Vec<KdlIdentifier>` beside
`KdlNode.children`. Empty vec = the spec case, so existing documents'
ASTs are unchanged in shape and meaning.

Serde decisions, on top of that AST:

1. **Annotation lists serialize as arrays of strings.** Where the
   upstream serde layer surfaces a node's or value's type as a single
   string, the fork surfaces the annotation list as a JSON array of the
   raw payload identifiers (`["object", "predicate#NotVeryWell"]`).
   No collapsing of the single-annotation case to a bare string:
   two shapes for one concept is a permanent tax on every consumer,
   and the fork is pre-1.0 in its own versioning — it takes the clean
   break here, once, and documents it.
2. **Derive consumers read annotations via a reserved pseudo-field.**
   A struct deserialized from a node may declare
   `#[serde(rename = "@annotations")] annotations: Vec<String>` (and
   `"@children-annotations"` likewise); the fork's deserializer serves
   those pseudo-fields from the AST before touching entries. This is
   the attribute-style exposure §5 Q3 asks about, implemented inside
   serde's existing data model — no new derive macro.
3. **Prop-key annotations are NOT exposed through serde map keys.**
   Serde's data model has no metadata slot on keys; keys stay plain
   strings, full stop. Key annotations are available (a) in the AST,
   (b) in the document/JSON rendering as a sidecar —
   `"@key-annotations": { "<prop-name>": ["<payload>", ...] }` sibling
   to the node's props — and (c) in the triple stream per Q1. A
   consumer who needs key annotations uses the document API, not
   derive. Pretending otherwise would require forking serde's data
   model, which this fork does not do.

## Q4 — Upstreaming: RECOMMENDATION

Recommend proposing, in this order, without blocking the fork on any
of it (§5 Q4's own caveat):

1. **Use-case follow-ups on Discussions #344 and #362** — the
   maintainers' objection was use-case-shaped ("can just be expressed
   with a property"; "asked for a use case"). We now have the use
   cases: the #5688 triples pipeline, where the annotation _is_ the
   statement structure and a property cannot carry it. Post the
   `limit.kdl`-derived examples (anonymized shapes are already public
   in #5753) and this record's Q1 decomposition as the worked answer.
2. **Test cases to the official KDL test suite** — children-block and
   prop-key annotation cases, marked as extension cases. Cheap,
   permanent, and the likeliest first artifact to land.
3. **Multiplicity (`Vec` annotations) as a kdl-rs conversation, not a
   spec proposal** — spec 2.0 is freshly frozen; a spec change
   proposal this soon reads as relitigating it. The fork keeps its
   extension code isolated in the annotation module so a future
   merge-back — in either direction — stays a small diff.

## Q5 — Benchmarks and the §4c acceptance bar: RECOMMENDATION

Corpus: `limit.kdl` and the production identity configs
(`pkgs/identity/config.kdl`, `pkgs/identity/configuration/embed/config.kdl`)
as the real-world set; a generated ingest corpus of #5688 compound
statements at 1k / 10k / 100k nodes, in plain and annotation-heavy
variants; the official KDL test-suite documents as the correctness
floor (all must pass, both spec versions, before any number counts).

Method: criterion benches in the fork mirroring upstream's bench
layout; upstream `kdl` 6.7.1 vs the fork, same machine, two modes —
format-preserving parse and hot-path parse (preservation off, per §4c).

**Acceptance bar for §4c (the number a fork PR must meet):**

- Hot-path throughput on the ingest corpus: **≥ 95% of upstream**
  (≤ 5% regression), per size class.
- Annotation-heavy variant vs its annotation-free twin, fork only:
  **≤ 10% throughput cost** — annotations are the point of the fork;
  they may not be a cliff.
- Hot path makes **no format-preservation allocations**: preservation
  is opt-in per parse (§4c), verified by allocation counts in the
  bench harness, not by inspection.
- CI gate: a bench regression beyond the bar fails the PR check
  (relative comparison, same runner — never absolute numbers).

## Implementation split (what gets filed next)

The fork implementation is NOT one packet. Recommended split, in
dependency order:

1. **Packet A — parser/AST** (blocks everything): fork scaffolding
   tracking upstream; `annotations: Vec` generalization + prop-key and
   children slots; `kdlx` declaration parse + lint; official test
   suite green on both spec versions.
2. **Packet B — serde + conversion** (after A): Q3's serde shape;
   Q1's payload decomposition into the #5688 JSON intermediate and
   triple stream; `"$kdlx"` in the JSON/YAML renderings.
3. **Packet C — benchmarks + upstream engagement** (parallel with B,
   lands last): Q5's corpus, benches, and CI gate; Q4's Discussion
   follow-ups and test-suite cases.

The §4d renderers beyond JSON/YAML (the LLM sentence rendering) stay
with #5688's follow-ons, not the fork packets — the fork's job is the
document model they render from.

— Design pass executed 2026-11-16 under queue task 16766. Parent
#5688's conversion contract (§2) is the authority this record
implements against; where this record and #5753 §4 could be read to
differ, §4 (Drew's commitments) wins and this record should be amended.
