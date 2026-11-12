# Source-mark grammar — EBNF, formatter/parser contract, validation rules

**Date:** 2026-11-08 · **Issue:** [#6044](https://github.com/developing-today/code/issues/6044)
**Parent programme:** [#6018](https://github.com/developing-today/code/issues/6018) (Atlas ingestion)
**Registry shape:** [#3125](https://github.com/developing-today/code/issues/3125) · **Consumer un-parked by this spec:** [#6029](https://github.com/developing-today/code/issues/6029)
**Status:** v1 decision record. Implementation timing is a separate decision.

Verified at `origin/main` `79ee7f3615` (2026-11-08): no mark grammar, formatter,
or parser exists in the tree, and #6029 (existing-embed audit) is OPEN, parked
behind this standard.

---

## 1. Problem

A chunk of corpus content can come from a git repo at a commit, from a session
store, from a chat log, or from a loose file on a machine at a date. Today those
are recorded as prose ("OpenCode DB @ AMD laptop on this day", `repo@hash:path`)
and every consumer that wants to know *where something came from* writes its own
regex. #6029 is parked because there is no single form to parse and validate
against. This record settles that form: **one grammar, one formatter, one
parser, one mechanical validation rule list**.

## 2. Chosen shape

A **mark** is a single-line ASCII string naming one occurrence of content, of
the shape `system[@…]` with an optional content hash. Three *named shapes* cover
the common sources, and one *general form* covers everything else; all four are
productions of one grammar and one parser.

```
repo@<revision>:<path>@<captured-at>[#<hash>]        ; git / VCS source
system@<host>@<captured-at>[:<locator>][#<hash>]     ; session store, DB, device
chat@<session>@<captured-at>[:<locator>][#<hash>]    ; chat transcript
<system>@<host>@<captured-at>:<locator>#<hash>       ; general fallback (all parts)
```

`chat` and `system` are instances of the same production; `repo` is the one
reserved head. The **general fallback form is the second production with every
part present** — it is not a separate grammar.

```
Examples
repo@71dc2fdc01a1b2c3d4e5f60718293a4b5c6d7e89:pkgs/id/src/store.rs@2026-11-08T09:41:00Z#sha256:9f2c…ab
system@atlas-laptop@2026-11-04T12:00:00Z:~/atlas/canonical/atlas.db#sha256:1a2b…cd
chat@ses_2b2eab662ffeRZGv4MoA62kl6A@2026-04-02T18:31:00Z:line:1204#sha256:44ef…01
```

## 3. EBNF

```
mark            = repo-mark | host-mark ;

repo-mark       = "repo@" revision ":" path "@" captured-at hash? ;
host-mark       = system "@" host "@" captured-at ( ":" locator )? hash? ;

revision        = 40 hex | 64 hex ;           (* full git/object id; no abbreviations *)
path            = locator ;                    (* relative POSIX path, see V2 *)
system          = token - "repo" ;
host            = token ;                      (* machine / store / session id *)
captured-at     = rfc3339-utc | date ;
locator         = ( path | "line:" 1*DIGIT | "span:" 1*DIGIT "-" 1*DIGIT | opaque ) ;

hash            = "#" hash-alg ":" 64 lower-hex ;
hash-alg        = "sha256" ;                   (* v1 closed set; see V5 *)

token-char      = %x21-22 / %x24-39 / %x3B-3F / %x41-5A / %x5B-60 / %x61-7A / %x7B-7E ;
token           = 1*128 token-char ;           (* printable ASCII incl. letters %x41-5A / %x61-7A; delimiters @ %x40, : %x3A, # %x23 excluded; no space, Cc, or Cf *)
opaque          = 1*256 token-char ;
rfc3339-utc     = %x... "T" ... "Z" ;          (* RFC 3339, always UTC, always "Z" *)
hex             = "0".."9" / "a".."f" ;
```

The grammar is deliberately small: one head token, `@`-separated fields (the
convention git, npm, and OCI all use), a `:`-prefixed locator, and a `#`
fragment carrying the content hash.

## 4. Formatter / parser contract

Exactly one pair, in one module, with these properties:

```text
format : OccurrenceRef -> String        // canonical, total
parse  : &str -> Result<OccurrenceRef, MarkError>   // canonical only
```

1. **Canonical only.** `parse` accepts the canonical form and nothing else. It
   trims leading/trailing ASCII whitespace, and normalises Unicode to NFC; it
   does not repair, guess, or accept alternate spellings. Every accepted string
   round-trips: `parse(format(x)) == Ok(x)` and `format(x) == format(parse(format(x)).unwrap())`.
2. **Total and deterministic.** `format` emits byte-identical output for equal
   `OccurrenceRef` values: lowercase hex, UTC `Z` timestamps, `#`-fragment last.
3. **Enumerated errors.** `MarkError` has one variant per validation rule (§5),
   so an audit can report *why* a mark failed, not just that it did.
4. **No ambiguity.** The grammar admits exactly one parse per string (no
   optional-field ambiguity: in a host-mark a locator is present iff a `:`
   follows the third field; in a repo-mark the path contains no `@` (V2), so
   the `@` introducing `captured-at` terminates it unambiguously; the hash is
   present iff a `#` follows). A string that could parse two
   ways is a grammar defect, not an input to disambiguate.

`OccurrenceRef` is the registry-shaped record (§6), not a bespoke type: the
formatter is the inverse of the registry's text column, so a formatted mark is
directly storable.

## 5. Validation rules the audit (#6029) executes mechanically

| # | Rule | Failure |
| --- | --- | --- |
| V1 | The string parses as `repo-mark` or `host-mark` of §3, canonical. | `Unparseable` |
| V2 | `repo` only: `revision` is a full 40- or 64-hex object id; `path` is a relative POSIX path (no leading `/`, no `.`/`..` segment, no `#`, no `@`, NFC). | `BadRevision`, `BadPath` |
| V3 | `host-mark` only: `system`, `host`, and any `opaque` locator contain no `@`, `:`, `#`, whitespace, or `Cc`/`Cf` character (#3455); `system` is not the reserved head `repo`. | `BadSystem`, `BadHost`, `BadLocator` |
| V4 | `captured-at` (carried by both marks) is RFC 3339 UTC (`…Z`) or `YYYY-MM-DD` (input); the formatter always emits the UTC `T`…`Z` form (a date normalises to `T00:00:00Z`). The value is not in the future. | `BadTimestamp`, `FutureTimestamp` |
| V5 | `hash`, where present, is `sha256:` followed by 64 lowercase hex characters. `sha256` is the v1 closed set; adding an algorithm is a spec change, not a local choice. | `BadHash` |
| V6 | A `span:` locator has `start <= end`, both positive; a `line:` locator is positive. | `BadSpan` |
| V7 | The content hash **verifies against the stored content** the mark describes (registry digest comparison, constant-time). | `HashMismatch` |
| V8 | The mark maps to exactly one registry row tuple (§6): `(system, host, locator, captured_at, content_hash)`. A mark whose tuple is incomplete for its registry class is refused, not partly stored. | `IncompleteOccurrence` |
| V9 | Round-trip: `parse(format(x)) == Ok(x)` for every `OccurrenceRef` the registry holds (property test over generated refs). | `RoundTripFailed` |

The audit's result per string is therefore a decision from a closed set:
**valid** (all rules pass) or one named failure — never "looks wrong".

## 6. Registry mapping (#3125 shape)

| Mark part | Registry field (#3125) | Notes |
| --- | --- | --- |
| head `repo` / `system` / `chat` / other | `system` | `repo` folds to `system = "git"`, with `host = null` |
| `revision` (repo) | `locator` = `commit:path`, plus `commit` | keeps `repo@commit:path` resolvable if the repo is later lost |
| `captured-at` (repo) | `captured_at` | the `@` after the path carries it, exactly as in host marks; a repo occurrence captured twice at different times yields two distinct marks |
| `host`, `at`, `locator`, `hash` | `host`, `captured_at`, `locator`, `content_hash` | the general tuple, always |
| — | `observed_at` | registry write time, distinct from `captured-at` |

Rules that follow: **every occurrence carries its content hash** (the hash is
what makes the occurrence verifiable without the original source); `observed_at`
is never part of the mark (it is registry bookkeeping, not source provenance);
and each chunk's occurrence list is a list of marks, so `chunk_occurrences`
becomes directly derivable from parsed text.

## 7. Alternatives rejected, and why

| Alternative | Why rejected |
| --- | --- |
| **Regex-per-source, no grammar** | The status quo: variants accumulate, each consumer scrapes differently, and #6029 cannot be a parse-and-validate job. |
| **Free prose ("OpenCode DB @ AMD laptop on this day")** | Human-readable, not machine-parseable; the tuple is exactly what this grammar makes explicit. |
| **A URI scheme (`atlas://repo/…`)** | Needs percent-encoding for hosts, paths, and dates; the `@` form copies git/npm/OCI conventions that both authors and readers already know. |
| **JSON embedded in the mark** | Correct but illegible in prose, and it invites schema drift inside a single field. |
| **One mark per field** (a separate host mark, date mark, hash mark) | Multiplies forms and forces consumers to reassemble the tuple; the composite mark is the unit the registry stores. |
| **Abbreviated commit SHAs** | Ambiguity without repository context; an occurrence must resolve from the mark alone. The formatter accepts full ids only. |
| **Hash separate from the mark** | A mark without its hash cannot be validated where the repo/DB is gone, which is the case #6029 exists to audit. |

## 8. Invariants implementation must hold

1. One module owns the grammar; no other component re-derives the syntax.
2. `format`/`parse` round-trip (V9), byte-identical and total.
3. Marks are ASCII and free of `Cc`/`Cf` characters (#3455) — a mark is safe in
   terminal output and in a log line by construction.
4. Lowercase hex, UTC `Z`, and `#`-last are canonical and enforced on output;
   input is normalised to them or refused (never silently stored un-normalised).
5. A registry occurrence always carries a verifying content hash (V7, V8); an
   unhashed prose mention may be recorded as *text*, never as an occurrence.
6. `captured-at` and `observed_at` are distinct fields; the mark never carries
   registry bookkeeping.
7. Adding a hash algorithm or a new reserved head is a spec change recorded
   here, not a local extension.

## 9. Consumer un-parked

This spec is the standard #6029's existing-embed audit consumes: the audit
becomes parse → validate (V1–V9) → report, with `Valid` or a named failure per
embed. #6044's design is complete on landing this record; implementation of the
formatter/parser follows as its own task.

## 10. Open questions (with defaults)

| Question | Default |
| --- | --- |
| Does the registry store `sha256` or another digest (e.g. blake3) as CAS hash? | v1 fixes `sha256`; if the CAS uses another algorithm, the hash-alg token is the single extension point and this record is amended rather than forked. |
| Is `line:` 1-based? | Yes, 1-based, matching editor and git-blame convention. |
| Do spans allow a byte range as well as a line range? | `span:` is a line range in v1; byte ranges are a future token, not an `opaque` dodge. |
| How are multi-line occurrences recorded? | As one mark per chunk with a `span:` locator; a chunk with several disjoint ranges gets several occurrence rows for one mark tuple. |
| Where does the module live? | The implementation task decides; the grammar is host-neutral. |
