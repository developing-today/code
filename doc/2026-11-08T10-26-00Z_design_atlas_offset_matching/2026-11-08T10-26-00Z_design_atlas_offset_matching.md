# Design: offset matching for delta-only ingestion (atlas)

- **Issue:** #6036 (parent #6028, delta-only ingestion + offset embeds)
- **Sibling:** #6035 (span-table schema) — the output contract in §4 is
  written against the span shape #6035 states; see the dependency note there.
- **Status:** design record. No implementation in this document's PR.
- **Prior art:** rsync's sliding-window matcher (Tridgell & Mackerras,
  TR-CS-96-05, 1996); Gear rolling hash as used by FastCDC for small-window
  content-defined chunking.

## 1. Problem

A file version in the offset-embed model (#6028) is an ordered list of span
references into previously embedded text. When a new version arrives, most
of it is usually byte-identical to the previous version — but not at the
same offsets, and not aligned to the old span boundaries. Chunk-then-compare
cannot see the reuse: a one-line insertion at the top shifts every later
chunk boundary and every chunk looks new.

The matcher must answer, for a new version `N` against the previous
version `O` of the same file: which maximal ranges of `N` are
byte-identical to ranges of `O`, and which ranges are genuinely new —
in the form Drew's example states it: "indices 0–16 matched old span A,
17–241 is new, 242+ matched old span B". That is exactly the rsync
problem, at embed scale instead of file-transfer scale.

## 2. Parameters (settled)

| Parameter                      | Value                                                                                                      | Why                                                                                                                                                                                                                                         |
| ------------------------------ | ---------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Window `W`                     | 64 bytes                                                                                                   | At 1–2 KB embed spans, rsync's ~700 B block is most of a span. A 64 B window finds matches well inside one span and resyncs within 64 B of any edit. Smaller windows raise the weak-hit rate (§6) faster than they find extra real matches. |
| Weak hash                      | Gear rolling hash, 64-bit, table-driven                                                                    | Designed for small rolling windows (FastCDC); O(1) roll per byte, good distribution at 64 B where Adler-style sums collide heavily.                                                                                                         |
| Strong verify                  | BLAKE3 over the candidate run, **plus byte comparison before emitting**                                    | The strong hash is a filter and an audit record, never the proof: a copy token is emitted only after the candidate bytes compare equal. A hash collision therefore costs time, never correctness.                                           |
| Minimum anchor run `MIN_MATCH` | 128 bytes (interior)                                                                                       | Below this, a copy token (span ref + two offsets + provenance, ~24–32 B on the wire/in the manifest) plus a span split costs more bookkeeping than re-embedding saves, and short runs are where reflow noise lives (§6).                    |
| Boundary rule                  | A byte-verified run touching offset 0, EOF, or the end of a previously emitted token is kept at any length | There is no cheaper representation for it: folding it into the neighbouring literal copies the same bytes anyway, and splitting it off loses the span reference it verifies against.                                                        |
| Index granularity              | Every byte offset of `O`                                                                                   | Offset embeds exist precisely to match at arbitrary offsets; indexing only span starts would re-impose aligned boundaries. Cost is accounted in §5.                                                                                         |

## 3. Algorithm

1. **Index the old version.** For every offset `i` in `O`, compute the
   Gear hash of `O[i .. i+W)` and insert `(hash -> i)` into a hash map.
   One pass, O(|O|) time and index space (§5).
2. **Slide over the new version.** Roll the Gear hash over `N` one byte
   at a time. On a weak hit at offset `j` (candidate old offset `i`):
   - verify: compare `N[j ..]` against `O[i ..]` byte by byte, extending
     the run as far as bytes agree, up to the end of either text;
   - if the verified run is `>= MIN_MATCH`, or qualifies under the
     boundary rule, record a BLAKE3 of the run, emit a copy token
     (§4), and resume sliding after the run;
   - if verification fails or the run is too short, advance one byte.
3. **Coalesce.** Adjacent copy tokens into the same old span with
   contiguous ranges merge; literals between copies shorter than a copy
   token's cost are folded into the neighbouring literal region (the
   bytes are identical either way — this is a manifest-size choice, not
   a correctness one).
4. **Emit** the token stream of §4 in new-version order. Literal regions
   are the only text the embedder sees; they are then cut into new spans
   of the usual 1–2 KB by the existing span-cutting step, and the new
   version's span list is assembled from copy-derived references plus
   the new spans.

Determinism: same `(O, N)` always produces the same token stream —
offsets are processed in order, ties between candidate old offsets break
to the lowest old offset. The matcher is a pure function of the two
texts; it holds no state between files or versions.

## 4. Output contract — and the #6035 dependency

The matcher emits an ordered token stream over the new version:

```
token  = copy    { new_start, new_end, old_start, old_end, strong: blake3 }
       | literal { new_start, new_end }
```

Invariants of the stream:

- Tokens are contiguous and non-overlapping and cover `[0, |N|)`.
- Every `copy` range is byte-verified: `N[new_start..new_end)` equals
  `O[old_start..old_end)`.
- `literal` ranges are maximal (never adjacent to another literal).

**Resolution into spans** is where this design meets #6035. #6035 states
a version is an ordered list of span references `(embed_id, start, end)`
with the span's embedded-text snapshot stored once, reused only when
byte-identical. Resolution rules:

- A copy token that exactly covers one or more whole old spans becomes
  those span references unchanged — the common case for an unedited
  region.
- A copy token covering _part_ of an old span (an edit landed mid-span)
  becomes a reference to a **sub-span**: the old span's snapshot
  restricted to `[old_start, old_end)`. The sub-span inherits the parent
  span's vector **only if** #6035's reuse rule is read at the text
  level — the embedded text of the sub-span is _not_ the text that was
  embedded for the parent span, so under #6035's own invariant
  (embeddings are not hashes; context changes the vector) a sub-span
  **must be re-embedded as a new span** whose snapshot is the restricted
  text, unless its range happens to be a whole previously embedded span.

That last point is the dependency, stated plainly: **the matcher
settles where the identical bytes are; #6035 must settle whether
sub-span ranges get their own span rows (split-on-reference) or the
matcher rounds copy ranges outward to whole old spans.** This design
recommends split-on-reference (it is the only reading under which a
mid-span edit doesn't force re-embedding the whole 1–2 KB span), and
the token contract above is final either way — only the resolution
step changes. The two designs should land together; if #6035 lands
whole-span-only, §4 resolution rounds outward and §5's savings drop by
roughly one span per edit point.

Context metadata (heading path, section) versions with the _reference_,
per #6035 — the matcher contributes the ranges; it attaches no context.

## 5. Cost model

Per new version, with `n = |N|`, `o = |O|`, `m` = matched bytes:

- **Matcher CPU:** one Gear roll + map probe per byte: O(n), a small
  constant (a table lookup, a shift, an add per byte — the FastCDC
  operation). Verification adds O(m) byte compares, paid once per
  matched byte. No other pass over the text.
- **Reads:** `O` is read once to build the index (it is already read to
  be diffed against; the span snapshots in #6035 are the same bytes).
  Index space is the honest cost: ~16–24 B per old offset in a hash map,
  so ~20 MB per MB of old text, held only for the duration of one
  file's match and then discarded. Files above a size threshold should
  match span-block by span-block (index a window of old spans around
  the expected position) — a memory bound, not an algorithmic change.
- **Embeddings saved:** `m` bytes never reach the embedder. Embedding
  is the expensive, metered operation in this pipeline (and the one
  that produces a stored vector per span); the matcher is local CPU.
  The trade is worth it whenever `m/n` exceeds a few percent.

Per source type:

| Source                                 | Expected behaviour                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                             |
| -------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Prose / docs (this repo's `doc/`)      | Edits are local: a paragraph changes, the rest matches at shifted offsets. `m/n` typically >90%. The phantom-diff risk is reflow (§6), not the matcher.                                                                                                                                                                                                                                                                                                                                                                        |
| Code                                   | Same shape as prose; long repeated lines (braces, imports) create weak-hit noise that verification discards at O(run) cost.                                                                                                                                                                                                                                                                                                                                                                                                    |
| Append-mostly logs                     | Near-total match; the new tail is one literal. Cheapest case.                                                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| Prepend-heavy logs (newest-first)      | Every offset shifts by the prepend size. A chunk-aligned differ fails completely here; the sliding window resyncs within `W` bytes of the start and matches the rest, so `m/n` stays high. The real costs are (a) the index rebuild per version — O(o) CPU paid even though "only the top changed" — and (b) per-line timestamps/ids inside the prepended block defeat matching _within that block only_. This is the case where the span-block windowing note above matters most: index the old head first, extend on demand. |
| Generated / minified single-line files | One giant line means one mid-line edit still matches before and after it; but a regenerated file with new hashes/ids throughout matches almost nothing and pays full index + slide cost for no saving. The matcher should report match ratio per file so ingestion can spot sources where matching is a wasted pass.                                                                                                                                                                                                           |

## 6. Failure cases

- **Reflow / re-wrap phantom diffs.** Re-wrapping prose to a new line
  width inserts/removes a newline every ~line-width bytes. Each run of
  unchanged bytes between newlines is then shorter than `MIN_MATCH`,
  so the matcher emits nearly the whole file as literal: a formatting
  change reads as a total rewrite and everything is re-embedded. The
  matcher is byte-honest — this is not a bug in it — but the design
  records it as the known worst case. Mitigations belong to a
  normalisation layer _before_ matching (a separate decision under
  #6028), never inside the matcher: a matcher that ignores bytes
  cannot keep §4's byte-verified invariant.
- **Edits spaced closer than `MIN_MATCH`.** Dense small edits (a
  find-replace touching every paragraph) fragment matches below the
  anchor threshold; savings collapse even though the text is "mostly
  the same". The boundary rule rescues runs at token edges only.
  Lowering `MIN_MATCH` trades manifest size for these files; 128 B is
  the settled balance, and the match-ratio report (§5) is how a source
  that wants a different value gets noticed.
- **Weak-hash collision storms.** Highly repetitive text (log lines
  differing only in a trailing id) produces many weak hits per offset.
  Verification keeps this correct; the cost is bounded by capping
  candidate old offsets tried per position (lowest offsets first) and
  counting discards in the match report.
- **Adversarial collisions.** Gear and BLAKE3 are not keyed; a crafted
  file can force collisions. Byte comparison before emitting makes
  this a CPU denial-of-service at worst, never wrong output. Ingestion
  of untrusted files inherits the candidate cap above.
- **Empty / tiny files.** Below `W` there is nothing to roll: the whole
  new version is one literal (or byte-equal to the old, caught by a
  whole-file compare first). Handled as a degenerate case, not an
  error.
- **Encoding changes.** UTF-8 → UTF-16 (or a BOM appearing) changes
  every byte: no matches, full literal, correctly. Recorded so nobody
  files it as a matcher bug later.

## 7. Worked run — the 17–241 example

The example (from #6028 via #6036): the new version's indices 0–16
match old span A, 17–241 is new, 242 onward matches old span B. (The
indices are in the example's own units; read them as byte offsets for
the walk.)

1. Whole-file compare fails; the index of the old version is built —
   every 64-byte window of spans A and B.
2. Sliding from offset 0: the window at 0 hits in span A and verifies,
   but the run stops at 17 bytes — the text diverges. 17 < `MIN_MATCH`,
   **but** the run touches offset 0, so the boundary rule keeps it:
   emit `copy { new 0–17, old A.start–A.start+17 }`.
3. Offsets 17–241: windows either miss or fail verification (this is
   the inserted/rewritten region). No copy qualifies; the region
   accumulates as literal.
4. At offset 242 the window hits in span B and verifies to the end of
   the example: a run well over `MIN_MATCH`. Emit
   `copy { new 242–end, old B.start–B.start+(end-242) }`, absorbing
   the literal boundary at 241/242.
5. Token stream:

   ```
   copy    0–17     -> old span A[0..17)
   literal 17–242
   copy    242–end  -> old span B[0..end-242)
   ```

6. Resolution (§4): the first copy is a sub-span of A (re-embedded as
   a new span under the recommended split-on-reference reading, or
   rounded outward under whole-span-only); the literal is cut into
   new 1–2 KB spans and embedded; the tail copy resolves to span B
   references. Net embedding work for the version: the 225-unit
   literal plus possibly the 17-unit head — instead of the whole file.

## 8. Non-goals

- Implementing the matcher (a separate implementation issue, filed
  when #6028's turn comes).
- The span-table schema itself (#6035), the normalisation question
  (§6), and chunk-store migration (#6035).
- Matching across files or across versions other than the immediate
  predecessor (cross-file dedupe is the span store's content
  addressing, not the matcher's).
