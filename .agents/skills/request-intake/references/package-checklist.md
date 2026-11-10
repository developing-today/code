# Package checklist, zero-context reader, PR body

Read this in steps 7 to 9. Section order and generated-view markers are in [records.md](../../../../planning/workflow/records.md). This file lists what must be true after intake.

## `main.md`

- [ ] The title and an introduction (2 to 5 sentences): what was asked, how much input there was (segments, characters), and where intake stopped.
- [ ] The transcript link with its sha256, and the one-line source.
- [ ] A file list with every concept stub and every target, each marked `new` or `edit`.
- [ ] A `G` section listing the global directives with their segment ids.
- [ ] "Reading of garbled input": one entry per token, or "none found".
- [ ] One section per concept, containing:
  - the generated view;
  - a summary;
  - preliminary thoughts as hierarchical items (`C001.1` ...);
  - inference-labelled suggestions;
  - an empty "Issues and PRs" placeholder.
- [ ] An "Issues and PRs" section at the bottom, empty but present. Later stages list every issue and PR there with the concept or item it refers to.

## `design.md`

- [ ] A first-pass review order. Each concept gets a one-line reason (foundations first, then what most others `require`).
- [ ] Waves from `rw ready --package "$PKG"`. Where nothing blocks, say so plainly: "run one agent per concept; all can start now".
- [ ] The generated run table (`rw render`). At intake, every row reads `not-started` for S1.
- [ ] An empty "Initial research stages" section, which S5 fills.

## `changes.md`, stubs, manifest

- [ ] `changes.md` has the intake line.
- [ ] Each stub in `concepts/` has the state block, links to its `main.md` section and to the transcript, and its exact excerpts at the top.
- [ ] `package.json` has `transcript.sha256` equal to `$SHA`, the source line, the visibility and the segment list.
- [ ] The package contains no other files: no notes, receipts or decisions yet; they are created lazily.

## Zero-context reader prompt

Send this to a fresh subagent that is given no other context:

```
You know nothing except the file <PKG>/main.md and the files it links to. Do not use other context.
Return: 1) the goal, your role, and the first three actions you would take if handed this package;
2) every question you would need answered before starting, most blocking first;
3) claims you could not find support for in the package; 4) links that do not resolve;
5) places where two readings lead to different work. Be short and concrete.
```

## PR body (`$WORK/pr.md`)

```
## What
Research package <PKG>: <n> segments, <n> concepts, G with <n> directives. Docs only.
## Why
Captures <one line: the request> so research can start per concept.
## Evidence
- rw verify-transcript --package <PKG> --against <SRC> -> <output>
- rw check --package <PKG> -> <output>
- prettier --check ... -> <output>
## Not verified
<e.g. readings in the garbled register; bytes retyped from chat>
## Next
S1: one research-prepare agent per concept; ready now: <ids>.
```
