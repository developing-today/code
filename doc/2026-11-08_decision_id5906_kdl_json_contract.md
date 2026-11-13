# Decision: KDL ⇄ JSON canonical conversion and round-trip contract

- **Date:** 2026-11-08
- **Status:** decision record — no implementation is included in this change.
- **Issue:** [#5906](https://github.com/developing-today/code/issues/5906),
  "id: design: KDL ⇄ JSON canonical conversion + round-trip contract (from #5688)".
- **Parent design:** [#5688](https://github.com/developing-today/code/issues/5688)
  ("KDL as human presentation format + triples + ANSI graph SQL") — its
  §1.2 conversion contract, §2.2 stage 2 canonical JSON, and §4
  "the KDL layer is ours to build" are the frame this record fills in.
  The parent's decisions (KDL as the human presentation format, atomic
  triples as the canonical data model, ANSI SQL/PGQ as the query model,
  postcard as the wire format) are **not re-decided here**; §9 states
  the only place this record proposes a refinement, and it is
  additive.
- **Inputs cited:**
  - [#5753](https://github.com/developing-today/code/issues/5753)
    (fork `kdl-org/kdl-rs`; universal/multiple/structured annotations;
    the annotation-payload open question) — its findings are the
    parser substrate this contract sits on. Read in full at drafting.
  - [#5911](https://github.com/developing-today/code/issues/5911)
    (source hunt for the late-2023 KDL/graph brainstorming) — **still
    open and blocked** (status `blocked`, `user/escalated`; the ChatGPT
    export is not available). §8 records why this record proceeds
    without it and what would change if the export lands.
  - [#5907](https://github.com/developing-today/code/issues/5907)
    (triple decomposition rules) and
    [#5908](https://github.com/developing-today/code/issues/5908)
    (payload boundary) — successors; §7 states which questions this
    record deliberately leaves to them.
  - [#5912](https://github.com/developing-today/code/issues/5912) /
    `doc/2026-11-06_decision_id_tagging_triples.md` — the sibling
    decision on tag claims as triple projections; the "claim node +
    provenance triples" shape there is the same shape §5 uses for the
    compound-statement anchor.
  - The KDL 2.0 specification (IETF draft `draft-marchan-kdl2`, KDL
    2.0.0 released 2024-12-21), read at `https://kdl.dev/spec/` while
    drafting: §3.2 (node/entries/children), §3.4–3.6 (property,
    argument, children block), §3.7–3.8 (values and type annotations,
    including the reserved annotation list), §3.9–3.13 (string
    forms), §3.14 (numbers), §3.17 (whitespace/comments).
  - In-tree evidence: `pkgs/mappy/migrations/limit.kdl` (Drew,
    2023-12-17 — the ancestor scratchpad recovered by #5753 §1.1),
    `pkgs/id/README.original.md` line ~38, `pkgs/roc/out-in.kdl`.
- **Evidence baseline:** `origin/main` at `5ab0c06928`, worktree
  `~/git/fleet/12851`, 2026-11-08. Verified at that baseline:
  `git grep -il "lossless-enough" -- doc pkgs/id` returns nothing;
  `git grep -il kdl -- doc` returns only the essay answer, a PDF, and
  a fleet YAML — no conversion contract exists on main. No PR is
  linked from #5906. This record is the first design for the
  conversion layer.

## Decision summary

| Question                     | Decision                                                                                                                                                                                                                                                                  |
| ---------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| What "lossless-enough" means | A **two-tier** promise: **Tier A (required)** — the document tree and every typed scalar survive KDL→JSON→KDL and JSON→KDL→JSON with semantic equality; **Tier B (not promised)** — comments, whitespace, quoting style, escape spelling, and raw-string hash counts (§2) |
| Canonical JSON               | A **plain-data node tree**: `{"kdl":2,"nodes":[…]}`, each node `{name, type?, args:[], props:{}, children:[]}`, each value a JSON scalar plus an optional `type` wrapper only where JSON cannot carry it (§3)                                                             |
| Where annotations live       | A single annotation is a sibling `"type"` key; the fork's **multiple** annotations become `"types":[…]` in the same slot, never a second structural form (§3.4)                                                                                                           |
| Ordering                     | Argument order and child order are **load-bearing and preserved**; property order is **not** load-bearing and is canonicalised to sorted-by-key (§2.3)                                                                                                                    |
| Numbers                      | Integers are emitted as JSON integers when they round-trip through IEEE-754 exactly, otherwise as `{"$int":"…"}`; floats as JSON numbers when finite and exact, otherwise `{"$float":"…"}`; `inf`/`nan` always the wrapper (§3.5)                                         |
| XML                          | **One-way export only** (KDL→XML), with an explicit lossy-edge list; XML→KDL is not a supported path (§4)                                                                                                                                                                 |
| YAML                         | **Both ways**, but YAML is a _rendering_, not a canonical form: YAML→KDL must first go through JSON (§4.2)                                                                                                                                                                |
| Canonical form               | **JSON is the canonical form.** KDL is the human presentation; YAML and XML are renderings. The fixed point is `JSON → KDL → JSON` (§3.1)                                                                                                                                 |
| Round-trip corpus            | A checked-in corpus with three classes — **must-round-trip**, **must-normalise**, **must-reject** — plus property tests over generated documents (§6)                                                                                                                     |
| Statement anchor             | The canonical JSON document for one authored compound statement **is** the provenance anchor §2.2 stage 2 names; its content hash is the statement node's identity candidate, decided in #5907 (§5)                                                                       |

## 1. What the parent already fixed (not re-decided here)

#5688 fixes, and this record takes as given:

1. **KDL is the human presentation format** over ID data — not the
   wire format (#2036's postcard decision stands) and not the query
   substrate. This record is about the _conversion_, not about
   promotion or demotion of KDL's role.
2. **The pipeline is parse → canonical JSON → atomic triples →
   resolve through IDs** (§2.2). The canonical JSON is stage 2, "the
   most conventional JSON form preserving the compound shape", and is
   "the audit / provenance anchor for the decomposition that
   follows".
3. **Decomposition is into `(subject, action, target)` triples**,
   with namespaces and qualifiers expressed as _more triples_, never
   special cases (§2.3), and a payload boundary for things that stay
   attached rather than decomposed (§2.4, owned by #5908).
4. **"Lossless-enough" is to be defined precisely by this ticket**
   (§1.2), covering node names, ordered children, properties /
   arguments, typed scalars, ordering guarantees, comments and
   formatting, type fidelity, and an explicit list of known-lossy
   edges each with a test.
5. **The conversion layer is ours to build** (§4) — no off-the-shelf
   KDL-in-database adapter exists; the layer is "a standalone,
   separately testable component with the round-trip test corpus".
6. **The parser stack is `kdl-org/kdl-rs`** per #5753, extended with
   universal, multiple, structured annotations. This record does not
   pick a parser; it constrains what any parser must be able to
   produce for the contract to hold (§3.4, §7.1).

## 2. "Lossless-enough", defined

The parent asks for a definition "precise enough to test". The
definition below is stated as a **promise with a tier**, because the
parent already excludes comments and formatting from the promise
(§1.2) while requiring typed scalars and order to survive — those two
requirements cannot be met by a single uniform "byte-identical"
promise, and pretending otherwise is what makes "lossless-enough"
a slogan.

### 2.1 Tier A — required, tested, non-negotiable

For every document `d` that parses under KDL 1.0 or KDL 2.0 (or the
fork's superset dialect):

1. **Tree shape.** The ordered list of top-level nodes, and for each
   node its ordered children, are identical after any round trip.
2. **Names.** Each node name's _string value_ is identical. (Its
   lexical form — bare identifier vs quoted vs raw — is Tier B.)
3. **Entries.** Arguments and properties are preserved, with
   argument order intact and each property's key and value intact.
   The _interleaving_ of properties among arguments is Tier B
   (the spec says property order is not semantically meaningful,
   §3.17 and §3.2 of the spec).
4. **Typed scalars.** Every value's _type_ (string / integer / float
   / boolean / null) and its _value_ are identical, including:
   - integer magnitude, up to the full `i128` range the spec allows;
   - float bit pattern for every finite IEEE-754 double, including
     `-0.0` and subnormals;
   - `inf`, `-inf`, `nan` (and their sign / payload where the parser
     exposes it);
   - string contents after escape processing, including embedded
     newlines, tabs, NUL-adjacent code points written as `\u{…}`,
     and astral-plane characters;
   - `#null` distinct from the string `"null"`, `#true` distinct
     from `"true"`, and `#true` distinct from `1`.
5. **Annotations.** Every type annotation — on a node name, an
   argument, a property key, a property value, or (fork extension) a
   children block — is preserved in order, as a list, with its
   payload string byte-identical.
6. **Document version.** A `/- kdl-version 1|2` marker, if present,
   is preserved (it is data-bearing: it tells a fallback parser
   which grammar to try first).

If any of these fails, the round trip is **broken**, not
"lossy-but-fine".

### 2.2 Tier B — explicitly outside the promise

These are _presentation_, not data, and the parent already says so
(§1.2 "comments and formatting: explicitly out of the losslessness
promise"):

- comments of every kind (`//`, `/* */`, `/-`);
- whitespace, indentation, line breaks, line continuations;
- semicolon vs newline node separators;
- whether a string was written as a bare identifier, a quoted string,
  a multi-line string, or a raw string, and how many `#`s a raw
  string used;
- which of the several equivalent spellings of a number was used
  (`1_000`, `0x3e8`, `1000`, `1e3` for the float form of `1000`);
- escape spelling (`\u{41}` vs `A`, `\ ` escaped whitespace);
- the order of properties relative to arguments and to each other.

A round trip **may** change any of these and remain conformant. The
fork's parser preserves them for _editing_ (kdl-rs is
format-preserving, #5753 §3); the conversion layer is allowed to drop
them, and dropping them is what makes the JSON canonical.

**Why this is the right cut.** Tier B is exactly the set of things
that cannot survive a trip through a format with no comment or
whitespace concept (JSON), so promising them would make the contract
impossible to satisfy; Tier A is exactly the set that carries
meaning, so failing it would make the conversion useless. The cut is
also testable: Tier A has an oracle (semantic equality of the parsed
tree), Tier B has none and needs none.

### 2.3 Ordering guarantees, stated precisely

| Element                          | Load-bearing? | Guarantee                                                                                                                                                                                                                          |
| -------------------------------- | ------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Top-level nodes                  | **Yes**       | Order preserved exactly; two documents differing only in top-level order are _different_ documents                                                                                                                                 |
| Children within a node           | **Yes**       | Order preserved exactly                                                                                                                                                                                                            |
| Arguments within a node          | **Yes**       | Order preserved exactly (spec §3.2: "Arguments are ordered relative to each other and that order must be preserved")                                                                                                               |
| Properties (key/value)           | **No**        | Order is canonicalised — sorted by key in canonical JSON. A repeated key is a **must-reject** case (spec: a node may not carry two properties with the same name; a parser that accepts it must report it, not silently last-wins) |
| Interleaving of props among args | **No**        | Not representable in the canonical JSON; a document that relies on it has no semantic meaning to preserve                                                                                                                          |
| Annotations on one part          | **Yes**       | List order preserved (the fork's extension; order is the only way to distinguish `(a)(b)` from `(b)(a)` and a consumer may read positionally)                                                                                      |

The canonical JSON therefore has **three ordered lists** (nodes,
args, children) and **one sorted map** (props). That asymmetry is
deliberate and is the whole of the ordering decision.

### 2.4 Comments: the one place the parent left an option

The parent says comments are out "unless a follow-on ticket decides
otherwise". **This ticket decides: they stay out.** Reasons:

- JSON has no comment concept, so including them would require an
  out-of-band side channel in every canonical document — a second
  representation of the same thing, which is exactly the "special
  case" the parent's §2.3 rule forbids in spirit.
- The fork's parser already preserves comments for the editing use
  case (#5753 §3), so nothing is lost: a person editing KDL keeps
  their comments; a person round-tripping through JSON does not
  expect to.
- If a comment is _data_ (an annotation the author wants to keep),
  the answer is to write it as an annotation, not a comment — which
  is what `limit.kdl` already does with `/*(result)*/`-style
  experiments (and which the fork's dialect makes legal as a real
  annotation).

## 3. The canonical JSON shape

### 3.1 What "canonical" means here

**JSON is the canonical form**; KDL is the human presentation of it.
This is not a reversal of the parent — the parent's pipeline already
makes JSON the intermediate that "is the canonical record of what was
authored" (§2.2 stage 2), and §4 says the conversion layer is
"KDL AST ⇄ canonical JSON". This record states the direction
explicitly because it determines which round trip is the _fixed
point_:

- **`JSON → KDL → JSON` is byte-identical** under the canonical
  serialisation rules in §3.6. This is the invariant the
  implementation is tested against.
- **`KDL → JSON → KDL` is semantically identical** (Tier A), not
  byte-identical (Tier B).

A document that has already been through the conversion once is a
fixed point from then on. That is what makes the canonical JSON safe
to hash, diff, store as a provenance anchor, and check into a repo.

### 3.2 The envelope

```json
{
  "kdl": 2,
  "nodes": [ … ]
}
```

- `"kdl"` is the **major** KDL version the document was parsed as
  (`1` or `2`), or the fork's dialect major when the document uses
  extensions. It is a hint for re-serialisation, not a claim that
  the document is strictly spec-valid — a document using extensions
  is a superset document (#5753 §4a) and says so by carrying an
  extension annotation (§3.4.3).
- No other top-level keys. Unknown keys are a **must-reject** on
  ingest, so that a future addition is a version bump and not a
  silent divergence.

### 3.3 A node

```json
{
  "name": "character",
  "type": "subject",
  "args": [ … ],
  "props": { "author": … },
  "children": [ … ]
}
```

- `"name"` — the node name's string value. Always present.
- `"type"` — the single type annotation's string value, omitted when
  there is none. Present as `"types": ["a","b"]` when the fork's
  multiple-annotation extension is used (§3.4).
- `"args"` — ordered array of values (§3.5). Omitted when empty.
- `"props"` — object of key → value. Omitted when empty. Keys are
  the property key's string value; the object is **sorted by key**
  in the canonical serialisation (§3.6).
- `"children"` — ordered array of nodes. Omitted when empty.

Omission of empty arrays/objects is part of the canonical form: an
empty `args` and an absent `args` mean the same thing and must not
produce two different canonical bytes.

### 3.4 Annotations, including the fork's extension

#### 3.4.1 The single-annotation case (spec KDL 1.0 and 2.0)

A spec-valid document has at most one annotation per part, so the
canonical JSON uses the obvious sibling key:

```json
{ "name": "date", "type": "published", "args": ["1970-01-01"] }
```

```json
{ "name": "node", "args": [{ "type": "u8", "value": 123 }] }
```

Property keys carry theirs as a parallel map, because a JSON object
key cannot itself be an object:

```json
{ "name": "person", "props": { "name": "Foo McBar" }, "propTypes": { "name": "contributor" } }
```

#### 3.4.2 The multiple-annotation case (fork extension)

#5753 §4b generalises the slot to a **list** on every part. The
canonical JSON expresses that as the same slot, with a plural key,
**only when the list has more than one element**:

```json
{ "name": "writesBooks", "types": ["predicate", "NotVeryWell"], "args": [{ "types": ["object", "book", "author:BrianGriffin"], "value": "Wish It, Want It, Do It" }] }
```

Two rules keep this from becoming a second structural form:

1. **One slot, two spellings, one meaning.** `"type": "x"` and
   `"types": ["x"]` are the same value; the canonical serialisation
   always emits the singular form for a one-element list and the
   plural form otherwise. A reader accepts both; a writer emits the
   canonical one.
2. **The payload stays an opaque string.** #5753 §4b leaves open
   whether `(object#book#author:BrianGriffin)` decomposes into a
   structured sub-grammar or stays opaque. **This record decides:
   opaque.** The conversion layer never interprets `#` or `:` inside
   an annotation; it carries the string. Interpretation is a
   _schema_ concern (the statement-shape validator in #5907), not a
   _conversion_ concern. This keeps the conversion layer's contract
   small enough to test exhaustively, and it means a future decision
   to give the payload structure is a change to the validator, not
   to the format.

#### 3.4.3 The children-block annotation (fork extension)

The fork allows annotating a children block (#5753 §4b, from
upstream discussion #344). Canonical JSON:

```json
{ "name": "adventures", "children": [ … ], "childrenType": "predicate" }
```

The children block is not a node and has no name, so it gets its own
sibling key rather than being wrapped in a pseudo-node. A
`childrenType` with no `children` is a **must-reject** (an annotation
on an absent block has nothing to annotate).

#### 3.4.4 Extension marking

#5753 §5.2 asks whether a document declares itself extended. **This
record decides: yes, and it is the `"kdl"` envelope field plus an
optional document-level annotation.** A document that uses any fork
extension serialises with `"kdl": 2` and carries the extension
marker in the canonical JSON; a strict-KDL consumer can detect it
before choking. The exact marker spelling is left to the parser fork
(it is a parser-dialect concern), but the canonical JSON must be
able to carry it, which is why the envelope is versioned.

### 3.5 Values, and the JSON type-system gap

KDL has five value types: **string, integer (`i128`), float
(`f64`), boolean, null**. JSON has four: **string, number, boolean,
null** — with no integer/float distinction and no exact `i128`.

The decision is **plain JSON wherever plain JSON is exact, and a
one-key wrapper only where it is not**:

| KDL value                                | Canonical JSON                                                            | Why                                                                                                                 |
| ---------------------------------------- | ------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------- |
| string                                   | JSON string                                                               | exact                                                                                                               |
| boolean                                  | JSON boolean                                                              | exact                                                                                                               |
| null                                     | JSON null                                                                 | exact                                                                                                               |
| integer, `                               | n                                                                         | ≤ 2^53−1`                                                                                                           | JSON integer | exact in every JSON consumer, including JS |
| integer, larger                          | `{"$int":"123456789012345678901234567890"}`                               | JSON numbers are IEEE-754 doubles in the dominant consumers; a bare 30-digit integer silently loses precision in JS |
| float, finite                            | JSON number, shortest round-tripping decimal (Rust `f64` `Display` / Ryu) | exact by construction                                                                                               |
| float, `-0.0`                            | `{"$float":"-0"}`                                                         | JSON `-0` and `0` are indistinguishable after a JS parse                                                            |
| float, `inf` / `-inf` / `nan`            | `{"$float":"inf"}` / `"-inf"` / `"nan"`                                   | JSON has no non-finite numbers                                                                                      |
| string that would collide with a wrapper | never wrapped — wrappers are objects, strings are strings                 | no collision possible                                                                                               |

The `$` prefix marks the wrapper as reserved: **a KDL node or
property named `$int` is unaffected** (node names are strings in a
different position), and the wrapper only ever appears in the
_value_ position. An object in value position is otherwise
impossible in KDL, so there is no ambiguity.

**Rejected alternative:** always wrapping numbers as
`{"$int":"…"}` / `{"$float":"…"}`. It is simpler to specify and
trivially lossless, but it violates the parent's "most boring,
conventional JSON representation" (§2.2 stage 2): every consumer of
the canonical JSON — a spreadsheet, a `jq` query, a JS tool — would
have to unwrap numbers before using them. The gap rule above keeps
99% of documents plain and pays the wrapper cost only where JSON
would actually lie.

### 3.6 Canonical serialisation rules

The byte-level form of the canonical JSON is fixed so that hashes
and diffs are stable:

1. **UTF-8**, no BOM.
2. **No insignificant whitespace** — `,` and `:` only, no spaces
   (RFC 8785 JCS-style compact form).
3. **Object keys sorted** by Unicode code point (which for the
   ASCII keys in this schema is plain lexicographic).
4. **Strings** escaped minimally: `"` `\` and control characters
   below U+0020 as `\u00XX`; everything else literal UTF-8
   (including non-ASCII and astral characters). No `\uXXXX` for
   printable ASCII.
5. **Numbers**: integers as decimal with no leading `+`, no leading
   zeros, `-0` never (that case is wrapped); floats as the
   shortest decimal that round-trips to the same `f64`, with a
   trailing `.0` when the value is integral so a reader can tell
   `1.0` (float) from `1` (integer).
6. **Trailing newline** at end of file — one, always.

Rule 5 is the one that needs care: `1` and `1.0` are different KDL
values (integer vs float) and must serialise differently. Emitting
`1.0` for the float keeps the distinction visible without a
wrapper, and `serde_json`'s default does not do this — the
implementation must use a custom float formatter or the `$float`
wrapper for integral floats. **This record chooses the `.0`
suffix** and records the wrapper as the fallback if the chosen
serde path cannot guarantee it.

### 3.7 A worked example

`limit.kdl`'s headline fragment (#5753 §1.1), converted:

```kdl
(subject)character "Brian" {
    (predicate)"writesBooks" (object)"Wish It, Want It, Do It" (object)"Faster Than the Speed of Love"
}
```

becomes

```json
{ "kdl": 2, "nodes": [{ "name": "character", "type": "subject", "args": ["Brian"], "children": [{ "name": "writesBooks", "type": "predicate", "args": ["Wish It, Want It, Do It", "Faster Than the Speed of Love"] }] }] }
```

Note what survives: the `(subject)` / `(predicate)` annotations (the
triples shape), the argument order (two books, in order), the node
nesting (the compound statement). Note what does not: the quotes
around `writesBooks` (Tier B), the indentation, the line breaks.

## 4. XML and YAML edges

### 4.1 XML: export only, and explicitly lossy

**Decision: KDL → XML is a one-way export. XML → KDL is not
supported by this contract.**

Reasons:

- XML's element model has no place for an _argument list_ distinct
  from child elements, so a KDL node with three arguments has three
  plausible XML encodings and no canonical one. A round trip would
  have to pick one, and every pick loses something (argument order
  vs. node-name vs. attribute-vs-element).
- XML has no integer/float distinction and no `null`, so the same
  type-fidelity problem as JSON appears, with no plain-data escape
  because XML has no scalar type system at all.
- The parent lists KDL ⇄ XML as a conversion (§1.2) but does not
  make it a _round trip_; §1.2's "lossless-enough" bullet list is
  about the conversion, and the known-lossy-edge requirement exists
  precisely to name cases like this.

The export mapping (normative for the exporter):

| KDL             | XML                                                                                        |
| --------------- | ------------------------------------------------------------------------------------------ |
| node name       | element name (invalid XML names are escaped per a documented scheme, e.g. `_xHHHH_`)       |
| type annotation | `kdl:type` attribute                                                                       |
| argument        | `<kdl:arg>` child element, in order, with `kdl:type` if annotated                          |
| property        | attribute `kdl:prop-<key>` (or `<kdl:prop name="…">` when the key is not a valid XML name) |
| children        | child elements, in order                                                                   |
| value type      | `kdl:value-type` attribute (`string`/`int`/`float`/`bool`/`null`)                          |

**Known-lossy edges for XML (each needs a test, §6.4):**

1. Property vs argument distinction survives only via the
   `kdl:arg` / `kdl:prop-*` convention — a consumer that ignores
   the namespace sees a flat element list.
2. Integer magnitude beyond `i64` and float bit patterns are
   carried as text and are exact only if the consumer does not
   re-parse them as doubles.
3. `-0.0`, `inf`, `nan` are text; a consumer that parses them as
   numbers loses them.
4. Node names that are not valid XML names are escaped and only
   reversible by this exporter's documented scheme.
5. XML has no annotation list, so multiple annotations (fork
   extension) are joined into one `kdl:type` value with a
   documented separator, or the export **refuses** if the
   separator appears in an annotation — refusal is preferred over
   a silently ambiguous export.

Because of (5) and (1), the exporter is **not** a canonical form and
its output is never hashed or used as a provenance anchor.

### 4.2 YAML: a rendering, both ways, through JSON

**Decision: YAML is a rendering of the canonical JSON, not an
independent canonical form.** KDL → YAML is `KDL → canonical JSON →
YAML`; YAML → KDL is `YAML → JSON → canonical JSON → KDL`.

Reasons:

- YAML's data model is a superset of JSON's (it has the same
  scalars plus tags, anchors/aliases, and merge keys), so a YAML
  document that is not already a JSON document has no KDL
  equivalent without a decision about which YAML features matter.
- YAML has the same integer/float collapse as JSON (and worse:
  YAML 1.1's `yes`/`no`/`on`/`off` booleans, and the Norway problem),
  so the same `$int`/`$float` wrappers apply.
- Routing through the canonical JSON means there is exactly **one**
  conversion implementation to test, and the YAML layer is a
  serialiser swap.

The YAML emitter must use **YAML 1.2 core schema**, must quote any
string that a YAML 1.1 parser would read as a boolean or number
(the Norway problem), and must not emit anchors, aliases, or merge
keys (the canonical JSON has no shared structure to anchor).

**Known-lossy edges for YAML (each needs a test, §6.4):**

1. A YAML document using anchors/aliases/merge keys is **refused**
   on ingest, not silently expanded (expansion changes the
   document's identity).
2. A YAML document using a non-core tag (`!!binary`, `!!timestamp`,
   custom `!foo`) is **refused** unless the tag maps to a KDL value
   type; `!!binary` maps to a `(base64)`-annotated string, which is
   the spec's own reserved annotation for binary.
3. YAML 1.1 boolean spellings (`yes`, `no`, `on`, `off`) are
   **strings** in 1.2 core; the emitter quotes them so they stay
   strings.
4. Multi-document YAML (`---` separated) is **refused** — a KDL
   document is one document; the caller splits and converts each.

## 5. The canonical JSON as the statement anchor

The parent (§2.2 stage 2) makes the canonical JSON "the audit /
provenance anchor for the decomposition that follows". This record
makes that concrete:

- **One authored compound statement = one canonical JSON document =
  one anchor.** The anchor's content hash (SHA-256 over the §3.6
  bytes) is a stable identity for "what was authored", independent
  of how it was later decomposed.
- The decomposition (#5907) produces triples that each carry a
  reference to the anchor. Whether that reference is a triple, a
  column, or the anchor's hash is #5907's call; this record only
  fixes that **the anchor exists, is canonical, and is hashable**.
- Because `JSON → KDL → JSON` is byte-identical (§3.1), the anchor
  is reproducible: re-serialising the KDL and re-hashing gives the
  same bytes. That is the property that makes the anchor usable as
  provenance rather than as a snapshot.

This is the same shape as the sibling decision for tags
(`doc/2026-11-06_decision_id_tagging_triples.md`): a claim node
with provenance triples, not a bare row. Here the claim node is the
statement anchor.

## 6. The round-trip test corpus

The corpus is a checked-in directory of fixtures with a manifest.
Three classes, each with a decidable oracle:

### 6.1 Must-round-trip (Tier A oracle)

Fixtures where `KDL → JSON → KDL → JSON` must produce **identical
canonical JSON bytes**, and where the second KDL must parse to a
tree semantically equal to the first.

Coverage (each is at least one fixture):

- **Node basics**: empty document; one bare node; node with only
  children; deeply nested children (≥ 10 levels).
- **Names**: bare identifier, quoted, raw, multi-line-adjacent,
  names that look like numbers (`"1.0v2"` quoted), names with
  spaces and punctuation, astral-plane names.
- **Arguments**: zero, one, many; mixed types in one node;
  argument order that would be lost by a set (`"b" "a" "c"`).
- **Properties**: one, many; a key that is also an argument name;
  a property whose value is a nested-looking string.
- **Scalars**: every reserved annotation type in
  `limit.kdl`-style use; `i128::MAX`, `i128::MIN`, `2^53`,
  `2^53+1`; `-0.0`, `0.0`, `1.0`, `1e-320` (subnormal), `1.7976931348623157e308`
  (max double); `inf`, `-inf`, `nan`; `#true`, `#false`, `#null`;
  the strings `"true"`, `"null"`, `"1"` (must stay strings).
- **Strings**: empty string; string with every escape
  (`\n \r \t \\ \" \b \f \s \u{…}`); string with embedded
  newlines via `\n`; multi-line string with dedent; raw string
  with `#`s; string containing `"""`; string containing a literal
  NUL written as `\u{0}`; astral characters; emoji ZWJ sequences
  (the `limit.kdl` tail is exactly this torture test — it is a
  fixture).
- **Annotations**: on node name, on each argument, on a property
  value, on a property key, on a children block; multiple
  annotations on each of those (fork extension); annotation
  payloads containing `#`, `:`, spaces, parentheses (quoted
  annotation strings).
- **Ordering**: two documents differing only in top-level order
  must produce _different_ canonical JSON (the anti-oracle: a
  converter that sorts nodes fails this).
- **The real corpus**: `pkgs/mappy/migrations/limit.kdl`,
  `pkgs/roc/out-in.kdl`, `pkgs/roc/platform.kdl`,
  `pkgs/identity/config.kdl` — the in-tree KDL files, each
  round-tripped.

### 6.2 Must-normalise (Tier B oracle)

Fixtures where the round trip **must change** the document in
exactly the ways §2.2 allows, and must change nothing else. The
oracle is: canonical-JSON-after == canonical-JSON-of-the-expected-
normalised-form.

- Comments stripped (`//`, `/* */`, `/-`).
- Whitespace and indentation collapsed.
- `1_000` → `1000`; `0x10` → `16`; `1e3` → `1000.0`.
- Quoted-vs-bare name spelling (`"foo"` → `foo` where `foo` is a
  valid identifier).
- Property order sorted.
- Raw string `#"... "#` re-emitted in the canonical string form.

### 6.3 Must-reject (error oracle)

Fixtures that must fail loudly, with a named error, never silently
succeed:

- Duplicate property key on one node.
- `childrenType` with no children (§3.4.3).
- Unknown top-level envelope key (§3.2).
- An annotation payload that the exporter cannot represent in the
  target format (XML separator collision, §4.1).
- YAML anchors/aliases/merge keys (§4.2).
- YAML non-core tags not mapped to a KDL type (§4.2).
- Multi-document YAML (§4.2).
- A KDL document that fails to parse at all (the parser's own
  errors, surfaced with spans — the parent's "errors a
  non-developer author can act on", §2.2 stage 1).

### 6.4 Property-based tests (the general oracle)

Beyond the fixtures, the implementation carries property tests
(the repo's testing doctrine: PBT where invariants exist, and this
layer has strong ones):

1. **Idempotence**: `canon(canon(d)) == canon(d)` for every
   generated document.
2. **Fixed point**: `canon(parse(toKdl(canon(d)))) == canon(d)`.
3. **Tree equality**: `parse(toKdl(canon(d)))` is structurally equal
   to `d` modulo Tier B.
4. **Type preservation**: the multiset of `(type, value)` pairs in
   the tree is unchanged by any round trip.
5. **Order preservation**: the sequence of node names at every
   depth is unchanged.
6. **Hash stability**: `sha256(canon(d))` is unchanged by any
   Tier-B-only edit of `d`.

The generator produces KDL documents from the grammar (nodes with
random names from all three string forms, random entries, random
nesting, random annotations), which is exactly the model-based
testing the repo doctrine asks for: the grammar _is_ the model.

### 6.5 Where the corpus lives

`pkgs/id/tests/kdl-conversion/` (or the crate the conversion layer
lands in), with `fixtures/` and a `manifest.toml` naming each
fixture's class and expected outcome. The corpus is a **golden
corpus** oracle for the fixtures and a **model** oracle for the
property tests — both are decidable, per the repo's testing
doctrine.

## 7. Invariants implementation must hold

These are the contract; each maps to a test class in §6.

1. **I1 — Round-trip fixed point.** `canon(parse(toKdl(canon(d)))) == canon(d)`.
2. **I2 — Semantic round trip.** `parse(toKdl(canon(d)))` ≡ `d`
   under Tier A equality.
3. **I3 — No silent loss.** Any input the converter cannot
   represent in the target format is **refused with a named error**;
   it is never dropped, truncated, or coerced.
4. **I4 — No silent gain.** The converter never invents structure:
   it does not add a node, an argument, a property, or an
   annotation that was not in the input.
5. **I5 — Order is data.** Node order, argument order, child order,
   and annotation order are preserved exactly; property order is
   sorted, and only property order.
6. **I6 — Types are data.** Integer vs float vs string vs boolean
   vs null is preserved exactly, including `i128` magnitude,
   `-0.0`, subnormals, and non-finite floats.
7. **I7 — Annotations are data.** Every annotation string is
   preserved byte-identically, in order, on every part it can
   attach to (including the fork's prop-key and children-block
   slots).
8. **I8 — Canonical bytes are stable.** The §3.6 serialisation is
   deterministic: same tree ⇒ same bytes, across runs, platforms,
   and serde versions.
9. **I9 — The anchor is reproducible.** `sha256(canon(d))` is
   unchanged by Tier-B edits and by re-serialisation.
10. **I10 — No interpretation in the conversion layer.** The layer
    never parses annotation payloads (`#`, `:`), never resolves IDs,
    and never decomposes into triples. Those are #5907/#5908's jobs;
    this layer carries structure, it does not give it meaning.
11. **I11 — Version honesty.** A document using fork extensions is
    marked as extended (§3.4.4); the converter never emits a
    document that claims to be strict-KDL while using extensions.
12. **I12 — Refusal is total.** A refused conversion produces no
    partial output — no half-written file, no partially-populated
    anchor.

## 8. Ordering: why this record proceeded, and what changes if #5911 lands

The packet (and the triage notes on #5906) order this design pass
**after** #5911 (source hunt) and #5753 (parser stack). Status at
drafting (2026-11-08, verified fresh):

- **#5753 — landed enough.** The issue is OPEN but its research is
  complete and its decisions are recorded in the issue body: the
  fork base (`kdl-rs`), the annotation model (universal, multiple,
  structured), the compatibility rule, and the `limit.kdl` recovery.
  This record cites those decisions and depends on none of the
  still-open follow-on questions (§7.1 below). **No blocker.**
- **#5911 — not landed, and blocked on an external artefact.**
  The issue is OPEN and labelled `status/blocked` /
  `user/escalated`: the late-2023 ChatGPT export does not exist on
  the laptop, and the ticket owns the export-side recovery
  "whenever the export becomes available". That is not a condition
  this ticket can wait for without blocking indefinitely.

**Decision: proceed, and record the dependency explicitly.** The
reasons, in order of weight:

1. **The deliverable does not depend on the missing source.** The
   contract above is derived from the KDL 2.0 spec, the in-tree
   `limit.kdl`, and #5753's recovered material — all present and
   read. The late-2023 brainstorming could change _emphasis_ (which
   annotation payloads matter, whether triples were already
   spelled a particular way) but not the _shape_ of the contract:
   a round-trip contract is constrained by the formats, not by the
   author's intent.
2. **The ordering was a sequencing preference, not a hard
   blocker.** The triage note says "ordered after those inputs
   land"; #5911 is blocked on an artefact that may never arrive,
   and a design that waits forever is a design that does not
   exist. The parent's own open-questions list expects this ticket
   to answer its questions, not to wait.
3. **The cost of being wrong is small and bounded.** If #5911
   recovers material that changes a decision here, the change is
   an **amendment to this record** (a new dated section), not a
   rewrite: the Tier A/B cut, the plain-data JSON, and the
   invariants are format-driven and would survive.

**What would change if the export lands**: if the recovered
brainstorming shows that Drew's late-2023 design _already_ fixed a
canonical JSON shape, or that the annotation payload was intended
to be structured (not opaque), this record's §3.4.2 opaque-payload
decision and §3.3 shape are the two places to revisit. Both are
recorded as **amendable** and both are isolated: the opaque-payload
decision is one rule in one section, and the node shape is one
section. Neither is load-bearing for the Tier A/B definition.

## 9. Where this record touches a parent decision

The parent's decisions are not re-decided. One place this record
**refines** a parent statement, recorded explicitly per the issue's
instruction:

- **Parent §1.2 lists "KDL ⇄ XML" and "KDL ⇄ YAML" as conversions
  alongside "KDL ⇄ JSON".** This record narrows that: JSON is the
  canonical form and the only bidirectional one; XML is export-only
  (§4.1) and YAML is a rendering routed through JSON (§4.2). This
  is a **refinement, not a reversal** — the parent asks for the
  conversions, and this record says which of them can be a round
  trip and which cannot, with the reasons. If the parent's intent
  was that XML must be bidirectional, that is a parent decision to
  revisit, and it is flagged here rather than silently forked.
- **Parent §2.2 stage 2 says the intermediate JSON is "the most
  conventional JSON form".** This record's §3.5 wrapper rule is the
  precise reading of "conventional": plain JSON wherever plain JSON
  is exact, a wrapper only where it would lie. No parent decision
  changes.

Nothing else in the parent is touched: KDL stays the presentation
format, triples stay the canonical data model, SQL/PGQ stays the
query model, postcard stays the wire format.

## 10. Alternatives rejected, and why

| Alternative                                                                                                 | Why rejected                                                                                                                                                                                                                                                                                                                                          |
| ----------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Byte-identical round trip for everything, comments included**                                             | Impossible through JSON (no comment or whitespace concept). Promising it would make the contract unsatisfiable, and a promise that cannot be kept is worse than a precise smaller one. The fork's parser still preserves formatting for _editing_ (#5753 §3), so nothing is lost in the use case that needs it.                                       |
| **KDL as the canonical form, JSON as a rendering**                                                          | Reverses the parent's pipeline (§2.2 stage 2 makes JSON the intermediate and the provenance anchor) and makes the fixed point a format with comments and formatting, which cannot be hashed stably. JSON is the canonical form precisely because it has no presentation layer.                                                                        |
| **A bespoke JSON dialect with typed wrappers everywhere** (`{"$type":"int","$value":"…"}` for every scalar) | Trivially lossless, but violates "most boring, conventional JSON" (§2.2 stage 2): every consumer would have to unwrap before use. The §3.5 gap rule pays the wrapper cost only where JSON would actually lose data.                                                                                                                                   |
| **JSON5 / JSONC (comments allowed)**                                                                        | Would let comments survive, but is not JSON: no standard consumer reads it, no canonical serialiser exists, and it re-opens the Tier B boundary for whitespace anyway.                                                                                                                                                                                |
| **XML as a bidirectional format**                                                                           | XML has no argument/child distinction, no scalar type system, and no `null`; three plausible encodings and no canonical one. Export-only with a named lossy-edge list is honest; a "round trip" would be a lie with a test that passes by accident.                                                                                                   |
| **YAML as an independent canonical form**                                                                   | YAML's data model is a superset of JSON's (anchors, aliases, tags), so "canonical YAML" would have to either refuse most YAML or silently expand it. Routing through JSON gives one implementation and one oracle.                                                                                                                                    |
| **Structured annotation payloads** (`(object#book#author:BrianGriffin)` parsed into a sub-structure)        | #5753 §4b leaves this open. This record chooses **opaque** because the conversion layer's job is to carry structure, not interpret it (§I10); interpretation belongs to the statement-shape validator (#5907). A future structured decision changes the validator, not the format — which is the property that makes the small contract worth having. |
| **Sorting nodes/arguments for canonical output**                                                            | Would make `canon` idempotent but would destroy order, which the spec says is load-bearing (§3.2). The anti-oracle fixture in §6.1 exists to catch exactly this.                                                                                                                                                                                      |
| **Dropping the `kdl` version field**                                                                        | The version marker is data-bearing (it tells a fallback parser which grammar to try), and the canonical JSON must be able to say whether it is a strict or extended document (§3.4.4). Without it, a strict-KDL consumer cannot detect an extended document before choking.                                                                           |
| **Waiting for #5911 before drafting**                                                                       | #5911 is blocked on a ChatGPT export that does not exist on the laptop and may never arrive. The contract is format-driven and does not depend on the missing source; §8 records the dependency and the amendment path. Waiting would mean the design does not exist.                                                                                 |

## 11. What this record leaves open (and to whom)

| Question                                                    | Owner                         | Why not here                                                                                                     |
| ----------------------------------------------------------- | ----------------------------- | ---------------------------------------------------------------------------------------------------------------- |
| Statement-node identity (content hash vs minted vs derived) | #5907                         | The anchor's hash is fixed here (§5); whether that hash _is_ the statement node's ID is a decomposition decision |
| Annotation payload grammar (opaque vs structured)           | #5907 (schema side)           | This record fixes the _conversion_ as opaque (§3.4.2); the validator may interpret                               |
| Payload boundary (decompose vs attach)                      | #5908                         | The parent §2.4 owns it; this record only guarantees the anchor carries what was authored                        |
| Parser fork's exact extension-marker spelling               | #5753 follow-on               | A parser-dialect concern; the canonical JSON only needs to carry it (§3.4.4)                                     |
| KDL spec version policy (pin v1, adopt v2, accept both)     | #5753 / parser fork           | This record's envelope carries the version and both are supported (§3.2)                                         |
| Whether XML export is ever made bidirectional               | parent #5688, if ever         | Flagged as a refinement in §9; not decided here                                                                  |
| SQL/PGQ query shapes over the triples                       | parent §3 / capability ticket | Untouched by the conversion contract                                                                             |

## 12. Change log

- 2026-11-08 — first recorded decision (task 12851, lane
  `262eb403-3347-4a7c-ba6b-e7bc263ca853`, queue
  `repo-health-code-pilot`). Drafted from the KDL 2.0 spec read at
  `https://kdl.dev/spec/`, #5753's findings, the in-tree
  `limit.kdl`, and the sibling tag-triples decision. #5911 recorded
  as blocked-and-proceeded (§8).
