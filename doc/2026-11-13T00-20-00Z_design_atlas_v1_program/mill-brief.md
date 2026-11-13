# Atlas v1 program — mill brief for workstream sessions

**Status:** working spec written by the summit session that owns starter
issue #9091 (queue task 15341), 2026-11-13. Every workstream session of
this design mill reads this file first and follows it. It is kept in the
repo so the record shows what each session was asked to do.

## 1. What the program is

Drew's directive of 2026-11-12 asks for a full design for a strong v1 of
Atlas. The intake produced the proto design
[`../2026-11-12T23-59-00Z_design_atlas_v1_proto/note.md`](../2026-11-12T23-59-00Z_design_atlas_v1_proto/note.md)
(read it completely before anything else), the starter issue #9091 and
ten workstream issues (#9092–#9101) under epic #8983. This mill turns the
proto design into the full design set: one research document, one
versioned design and one list of granular implementation issues per
workstream.

The repository is `developing-today/code`. Atlas has no repository of its
own. The repository is **public**; see §7.

## 2. What you produce

You own exactly one directory, named in your launch prompt, of the form
`doc/2026-11-13T00-30-00Z_design_atlas_v1_<slug>/`. You write exactly
three files in it and no file anywhere else:

| File          | Content                                                                                 |
| ------------- | --------------------------------------------------------------------------------------- |
| `research.md` | State-of-the-art expansion: evidence, prior art, options with what makes each better or worse. |
| `note.md`     | The versioned design.                                                                   |
| `issues.md`   | Granular implementation issue specifications, in the exact format of §6.                |

Do not run `git add`, `git commit`, `git push`, `git stash` or any other
command that changes Git state. Do not create, edit, close or comment on
GitHub issues or pull requests. The summit session commits your files and
files the issues after its own review. Reading with `gh` and `git` is
expected.

## 3. Rules that bind every workstream

1. **Internal only.** The corpus is never exposed directly to a public
   surface. Public disclosure happens only through the classification
   workstream's disclosure contexts.
2. **The Metadata doctrine is decided.** Proto §C is shared with the
   issue/PR Metadata of #9060 §12, and amendments are already filed on
   #9073 and #9074. Elaborate inside it. Do not re-decide it.
3. **#9084 stays open.** The general reference-versus-copy question and
   the garbage-collection design belong to #9084. Atlas workstreams
   produce measurements and requirements as inputs to it and adopt its
   outcome. No Atlas design may settle it.
4. **Identity belongs to #9077.** Consume it as it is.
5. **Designs and issues only.** Nothing in this mill implements the
   build-out, deploys, changes a schedule or changes a running service.
6. **Coordinate, do not duplicate.** Epic #8983 already has about 68
   open children, and `doc/` has 17 earlier Atlas design records (list
   them with `ls -d doc/*atlas*`). Among them,
   `doc/2026-11-11T03-45-00Z_design_atlas_v2_v4/` (issue #6020) already
   stakes later horizons for several topics. Find what already exists for
   your topic, build on it, and cite it by issue number and path. Where an
   earlier merged design conflicts with the proto design, name the
   conflict and say which side your design takes and why.
7. **Detached operation.** Nobody is watching. Never call the `question`
   tool. When you have a question, write it in the "Open questions"
   section of `note.md` with the default you chose, and keep working.
8. **No waiting on an oracle.** Research, decide, write. Where a decision
   is Drew's alone (cost ceilings, irreversible deletions, his personal
   data), write the options and a recommendation and mark it
   **FOR DREW**; choose a working default so the design is complete
   either way.
9. **More valid options beat one converged answer.** Record the options
   you rejected and why. Capture ideas that go beyond the brief in a
   "Sparked ideas" section of `research.md`.

## 4. Evidence rules

Every factual claim in `research.md` and in the V1 section of `note.md`
carries one of these tags:

- `[code: path:line]` — you read it in this worktree.
- `[issue: #N]` — you read it with `gh issue view N --repo developing-today/code`.
- `[web: URL]` — you fetched that URL in this session and it says so.
- `[measured: command or method, date]` — you measured it, read-only.
- `[proto §X]` — taken from the proto design and not re-verified.
- `[assumption]` — not verified; say what would verify it.

Never cite a source you did not open in this session. Never write a
number, a benchmark result, a product limit or a price from memory
without the `[assumption]` tag. Proto §0 was verified on 2026-11-12, but
re-verify the parts your design depends on against the code and say where
the proto design is wrong or stale.

Measurements are allowed only when they are read-only, light and cannot
disturb a running service: file sizes, row counts, `PRAGMA` reads on a
read-only SQLite handle, read-only platform CLI listings. No writes, no
`VACUUM`, no deploys, no load tests against production. A measurement
that needs more than that becomes a `bench` or `research` issue in
`issues.md` with its protocol written out. Never print a credential
value.

## 5. Shape of `research.md` and `note.md`

`research.md` sections, in order:

1. **Question** — what this workstream must find out.
2. **What exists today** — code, data, schedules, earlier designs and
   issues for this topic, with evidence tags.
3. **State of the art** — how the best current systems and the published
   literature solve this, with sources. Cover prior and current examples
   of the same concept and what was learned from them.
4. **Options** — each option with architecture, scalability,
   performance, reliability, extensibility, cost and operational weight;
   what makes it better or worse here.
5. **Measurements** — what you measured, and the protocols for what
   still has to be measured (named oracle, method, pass condition).
6. **Sparked ideas** — anything worth an issue or a later design.
7. **Sources** — every source opened, with its tag.

`note.md` sections, in order:

1. **Status and scope** — one paragraph; the workstream issue number;
   the statement IDs covered (your launch prompt lists them; the full
   list is in [`j-rows.md`](j-rows.md)).
2. **V1 — today.** What exists, verified, with evidence tags. This is
   the baseline, not a plan.
3. **V2 — the program target.** The strong v1 that Drew directed.
   Everything in `issues.md` builds V2. Give data shapes, protocols,
   algorithms, failure behaviour, limits and budgets to the depth the
   evidence supports. State the invariants that must always hold; they
   become property tests.
4. **V3 and V4 — later directions.** Shorter. What V2 deliberately
   leaves out and where it goes next, short-term and long-term. Where
   #6020's design already stakes a horizon for the topic, cite its
   thread (for example "#6020 V2-S") instead of re-deciding it. Note
   that #6020's V2–V4 labels name Atlas horizons after the landed
   baseline, while V1–V4 in this design set are stages of one
   workstream; always write "#6020 V2-S" in full when you mean the
   former.
5. **Decisions** — a numbered list `D1…`: the decision, the options
   considered, the reason, and what evidence would reverse it.
6. **Interfaces** — two lists, **Provides** and **Consumes**, using the
   shared terms of §8. For each item: the exact name, the shape and the
   workstream on the other side. The summit reconciles these across the
   design set, so be exact about what you assume from others.
7. **Statement coverage** — a table: statement ID, the section of this
   note that satisfies it, and the issue IDs in `issues.md` that build
   it. Every statement ID in your launch prompt must appear. Add rows
   for any further statement of the proto design's section for your
   workstream that the J row does not capture.
8. **Risks and failure modes.**
9. **Open questions** — each with the default chosen. Mark **FOR
   DREW** where rule 8 of §3 applies, with options and a recommendation.
   Answer the proto §K questions that fall in your workstream.

Write plainly and exactly. Prefer a table to a paragraph where the
content is a comparison. Say what is decided, what is measured and what
is assumed, and never blur them. Aim for 15–35 KB for `note.md` and
10–30 KB for `research.md`; depth where the evidence is, not padding.

## 6. Format of `issues.md`

The summit files these with a script, so the format is exact.

The file starts with a short preamble, then a section
`## Mapped to existing issues` holding a table (existing issue number,
what it already covers, the amendment comment this program should leave
on it, if any). Search before you specify: for every issue you are about
to write, search the open and closed issues
(`gh issue list --repo developing-today/code --state all --search "<terms>" --limit 20`).
If an existing issue already covers the work, put it in the mapped table
instead of writing a duplicate.

Then one block per new issue, exactly:

````markdown
## ISSUE <ID>: <title>

```meta
parent: #<workstream issue number>
kind: impl | bench | research | design | task | decision
size: S | M | L
blocked-by: <comma-separated IDs from this file and/or #numbers, or none>
relates: <comma-separated #numbers, or none>
searched: <the search terms you used for the duplicate check>
```

### Problem

### Scope

### Acceptance criteria

- [ ] …

### Tests and oracle

### Out of scope
````

Rules for the blocks:

- `<ID>` is your workstream's prefix (given in your launch prompt) plus
  a two-digit number: `A1-01`, `H-07`.
- `<title>` follows the repository's style: `impl(atlas): …`,
  `bench(atlas): …`, `research(atlas): …`, `design(atlas): …`,
  `task(atlas): …`, `decision(atlas): …`.
- One issue is one pull request or one measurement run that one worker
  can finish: size S is under a day, M a few days, L about a week.
  Split anything larger.
- Each issue stands alone. A worker who has read only that issue and the
  files it names can do it. Name the design section and file it builds.
- Acceptance criteria are checkable by a command, a test or a read-back,
  not by opinion.
- "Tests and oracle" names the oracle, as the repository's testing
  doctrine requires: a reference implementation, a model, an invariant
  check or a golden corpus. Use property tests where the code has
  invariants (parsers, serializers, state machines, formats).
- Typical count is 6–14 issues per workstream. Cover all of V2. V3 and
  V4 get at most a few `design` or `research` issues.

## 7. The repository is public

Everything you write is public. Therefore:

- No corpus content, no chunk text, no real query text, no excerpts of
  conversations, in any file.
- Nothing about the content of Drew's personal data. Category names
  only ("personal-life content").
- No credential values, tokens, account identifiers beyond those already
  in the repository.
- Counts, sizes, schema names and table names are fine.

## 8. Shared terms across workstreams

Twelve sessions write in parallel. Use these terms for the things that
cross workstream boundaries, and state in your Interfaces section what
you need from each. The owning workstream fixes the exact field names;
the summit reconciles them afterwards.

| Term                   | Meaning                                                                                                                               | Owner         |
| ---------------------- | ------------------------------------------------------------------------------------------------------------------------------------- | ------------- |
| **Metadata object**    | The versioned metadata carried by every item, embed and query. It has a schema version and an `extras` dictionary.                   | C (#9095)     |
| **schema version**     | The version carried inside every Metadata object.                                                                                     | C (#9095)     |
| **sensitivity**        | The strict safety enum on every embed; today `shareable < personal < local-only < secret-bearing`.                                    | D (#9096)     |
| **disclosure context** | The surface a result is going to (for example a fleet task agent, or a public GitHub comment). Search filters by it; queries record it. | D (#9096)     |
| **corpus watermark**   | The value that changes whenever the corpus changes. Ingestion advances it; the cache keys on it; query records store it.              | E (#9097)     |
| **embedder identity**  | What names the model that produced a vector: model, revision, dimensions, prompt policy.                                              | A3 (#9092)    |
| **content address**    | The hash that names immutable bytes in the content-addressed store. The store itself is #9084's.                                      | #9084; F, H consume |
| **location record**    | One place where given bytes were seen: path, host, date, originating process. A reference in #9084's sense.                           | H (#9100)     |
| **query record**       | The stored record of one query: text, every flag and parameter, disclosure context, tool hashes and versions, corpus watermark.       | G (#9099)     |
| **shard manifest**     | The self-describing `atlas_manifest` of a shard, which exists today.                                                                  | A2 (#9092)    |
| **search surface**     | Any of: CLI, service protocol (`fleet serve search`), hosted Worker, MCP namespace.                                                    | A1 (#9092)    |

Workstream prefixes: A1 search contract, query understanding and cache;
A2 fan-out, Durable Objects and shard benchmarks; A3 second embedder and
mixed embedders; B backup mesh; BR OpenCode database backup research;
C Metadata schema; D classification; E ingestion; F embed-storage
economics; G query-as-data; H virtual filesystem; I worker instructions.

## 9. Own review before you return

When your three files are complete, launch one fresh reviewer
subsession (`subagent` tool, agent `general`, model
`opencode-route/workhorse`, foreground, in this same directory). Tell it
to read this brief, the proto design section for your workstream, your
workstream issue and your three files, and to report:

- claims whose evidence tag does not hold when it re-opens the source;
- statement IDs or proto-section statements not covered;
- acceptance criteria that are not checkable;
- contradictions inside your files or with the binding rules of §3;
- issues that duplicate an existing issue.

The reviewer writes nothing to disk. Fix what it finds, then have the
same reviewer (continue it by its session ID) confirm. Return only when
you and the reviewer agree the work is complete and correct. If the
reviewer cannot be launched, say so in your final message and do the
checks yourself.

## 10. Your final message

Return a short report: the three file paths with sizes; the decisions
made; the open questions with defaults, and which are FOR DREW; the
number of new issues and of mapped existing issues; what the reviewer
found and what you changed; anything you could not verify. Do not paste
the documents.
