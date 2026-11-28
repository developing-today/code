# Concept file after S1

Read this in steps 2, 3 and 6. The state block and excerpt formats are owned by [records.md](../../../../planning/workflow/records.md). This file gives the section order and examples.

## Section order

1. Title, then the ` ```workflow-state ` block (single writer: you).
2. Links: main reference section, transcript, Research issue (once it exists).
3. `## Relevant transcript excerpts`: every exact excerpt, most relevant first.
4. `## Normalized purpose`: intake's summary. Keep it as written. Add refinements as items, never as rewrites.
5. `## Initial thoughts` and `## Inferred suggestions`: from intake, kept.
6. `## Repo map`: `path:line`, what it is, what it tells the research.
7. `## Leads for research`: sources with URL, date and one line each.
8. `## Research questions`
9. `## Facets`
10. `## Notes sent`: one line per note file.
11. `## Stage results`: one line for S1 (date, PR, issue). Later stages add theirs.

## Excerpt snippet (never type an excerpt)

This prints one excerpt block for a segment range. The block is a link line, a blank line, then a fence longer than any backtick run inside.

```sh
python3 -I - "$PKG" T007 T009 <<'PY'
import json, re, sys, pathlib
pkg, first, last = pathlib.Path(sys.argv[1]), sys.argv[2], sys.argv[3]
meta = json.loads((pkg / "package.json").read_text())
raw = (pkg / meta["transcript"]["path"]).read_bytes()
text = raw.decode(meta["transcript"].get("encoding", "utf-8"))
seg = {s["id"]: s for s in meta["segments"]}
body = text[seg[first]["start"]:seg[last]["end"]]
fence = "`" * max(3, 1 + max((len(r) for r in re.findall(r"`+", body)), default=0))
label = first if first == last else f"{first}-{last}"
print(f"[Transcript {label}](../transcript.txt)\n\n{fence}workflow-excerpt {label}\n{body}\n{fence}")
PY
```

Paste its output into the file. `rw check` then proves the excerpt is exact. If the tool has a command that writes excerpts, use that instead.

## Good research questions

Each question can be answered, names what its answer decides, and says where to look first.

- Weak: "How should notes work?"
- Good: "Can two workers create receipts for the same note at once without a lock, and does `O_EXCL` on the target filesystem guarantee it? Decides whether receipts need a lease. First source: the `open(2)` man page, then `scripts/research_workflow.py` `resolve-note`."

Across the set, cover:

- feasibility;
- prior art and similar projects;
- alternatives (at least two per real choice);
- use cases and user anecdotes;
- failure modes and their mitigations;
- integration seams with the repository map;
- cost and effort;
- what the concept enables for others (`enables`).

## Facets

A facet is a part of the concept that could be researched in its own session and handed off as its own issue at S5. Write each one like this:

```
- **<facet name>**: <scope in one line>. Questions: Q2, Q5. Why separate: <different sources | different files | independent decision>.
```

A concept usually has 2 to 6 facets. More than about 8 means the concept should be split. Send that proposal as a note to the router, and don't split it yourself.

## Hint path (`type: hint`)

The user only gestured at this idea. Prepare what is needed to resolve it:

- the hinting excerpts;
- what the hint could mean (inference-labelled readings);
- what in the repository or earlier packages might hold the missing context (`git log --grep`, older `doc/*_research_*`, issues);
- the exact questions for the user, each with a default reading.

The Research issue says "blocked on: user context for hint" and lists those questions. The concept stays `type: hint`, and S2 doesn't start until a note or a human answer changes its type.
