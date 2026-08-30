---
name: hardware-device-research
description: Build or audit a source-traceable hardware knowledge base. Use when asked to research or identify a board, module, appliance, component, teardown, firmware platform, datasheet, pinout, internal hardware, compatibility, source code, firmware, or existing device documentation.
---

# Hardware Device Research

> **This is a pointer, not the method.** Hardware research does not live in this repository.
> The authoritative skill, and every record it produces, live in a separate repo.

## Where the work happens

**[`developing-today/hardware-doc`](https://github.com/developing-today/hardware-doc)** — checked
out beside this repo and symlinked in at `doc/hardware`:

```
<repo-parent>/
├── code/                       ← this repo
│   └── doc/hardware  ────────┐   symlink (tracked)
├── hardware-doc/        ←────┘   the knowledge base — WRITE HERE
└── repo-archive/                 bulk artifacts, namespaced per source repo
```

Set it up — clones if missing, fast-forwards if clean, never clobbers local work:

```bash
./scripts/hardware-doc-init.sh      # also runs automatically in the devshell
```

## You can work the structure from here

The sibling checkout and its archive are reachable from **this** repo's root — the shortcuts
resolve to exactly the same bytes the knowledge-base repo sees:

| From here | Reaches | Called this in `hardware-doc` |
|---|---|---|
| `doc/hardware/…` | the knowledge base | `./…` |
| `archive/…` *or* `doc/archive/…` | archived artifacts | `archive/…` |
| `scratch/…` *or* `doc/scratch/…` | working files, raw fetches | `scratch/…` |

So you can read, download into, and archive from these paths without resolving anything by
hand. `archive/devices/foo/bar.step` is the same file from either side.

Two things still hold:

- **Write records into `doc/hardware/` and commit them there.** Git will not cross a symlink
  (`fatal: pathspec ... is beyond a symbolic link`), so commit from inside it:
  `git -C doc/hardware add -A && git -C doc/hardware commit -m "..."`, and
  `git -C archive add -A && git -C archive commit -m "..."` for artifacts. `archive/` and
  `scratch/` are the same repository — `git add -A` from either stages both tiers.
- **Inside a record, use the paths that repo sees**: `archive/…`, `scratch/…`, and
  repo-relative links. `doc/hardware/devices/…` is correct from here and broken from there,
  and that repo is also read standalone.

Commit freely in `archive/` and `scratch/`, temporary material included. But **never** reset,
stash, `checkout --`, clean or delete work you did not create — these checkouts are shared, and
another session's uncommitted work is unrecoverable.

## Read the real skill — in full, once

**Before starting any research pass, read the whole method file end to end.**

```
doc/hardware/.agents/skills/hardware-device-research/SKILL.md      (~900 lines)
```

It opens with a table of contents marking the **core** sections. Read all of it the first
time anyway: the parts interlock — the workflow assumes the evidence rules, the verification
checklist assumes the manifest schema, and the failure-modes section explains why several
otherwise-odd steps exist. Jumping straight to the step you think you need is how passes end
up with unsourced claims and stranded artifacts.

If context is tight, read the **core** sections in this order and do not skip any of them:

1. *What makes this different from summarising a datasheet* — the five habits
2. *Where research is written* — layout, sibling directories, path resolution
3. *Workflow* — all 18 steps, in order
4. *Handle licensing and storage honestly* + *Analyze vendored dependencies…* — what may be kept, moved, omitted
5. *Artifact and Acquisition Manifests* — provenance schema
6. *Conflict Protocol* and *Evidence discipline and known failure modes*
7. *Verification* — the checklist a pass must survive

Then, on subsequent invocations in the same session, use the table of contents to jump back
rather than re-reading.

## Supporting documents in that repo

| What | Where |
|---|---|
| Working conventions, symlink and archive rules | `doc/hardware/AGENTS.md` |
| Entry points, research passes, indexes | `doc/hardware/README.md` |
| Per-site retrieval findings and user-agent matrix | `doc/hardware/ai-crawler-site-access-table.md` |
| What is reproducible vs irreplaceable | `doc/hardware/SIZE-AUDIT.md` |
| Netlist parsers, ESP image parser, archiver | `doc/hardware/tools/` |

**Deliberately not duplicated here.** An earlier version of this file carried the full ~875-line
method — a copy that drifts, and that reads as though research may be done in *this* repo. One
authoritative copy, in the repo that uses it.

## The few things worth knowing before you get there

Orientation only — **not** a summary you may work from. Read the full method first.

- **Write records in `hardware-doc` and commit them there**, not in this repo. It is not a
  submodule and not vendored: at ~440 MB it would make every clone of this repo ~6.5× larger.
- **Prefer primary machine-readable evidence.** A parsed `.kicad_pcb` netlist beats a vendor wiki
  table. Several findings in that repo exist only because the PCB was parsed rather than the docs
  read.
- **Label every consequential claim** — `executed-success` · `reported-working` · `inferred` ·
  `not-tested`. Never present an untested command as verified.
- **Record conflicts rather than resolving them by preference.** Vendors contradict themselves
  often, and the contradiction is usually the finding.
- **Never delete an acquired artifact.** Move it to `repo-archive` with a `*.ARCHIVED.md`
  placeholder carrying size, SHA-256, upstream commit/author/licence and recovery URLs.
  `tools/archive_artifact.py` does this and verifies the move by content fingerprint.

## Resolving the sibling paths

Both siblings are relative to the **real repository root**, which is not the working directory and
not `~`. Resolve worktree-safely with the git *common* directory:

```bash
ROOT="$(dirname "$(git rev-parse --path-format=absolute --git-common-dir)")"
PARENT="$(dirname "$ROOT")"     # $PARENT/hardware-doc  and  $PARENT/repo-archive
```

`--git-common-dir`, not `--show-toplevel`: inside a linked worktree the toplevel is the worktree,
whose parent is the wrong directory.

You rarely need this — the shortcuts above already reach everything. Resolve explicitly only in
a script that must work from an unknown directory, or when the sibling checkout is missing and
you are about to run the init script.

`repo-archive` is namespaced per source repository (`hardware-doc/` for artifacts,
`scratch/hardware-doc/` for working files), so material archived out of another repo would sit
beside it rather than collide.
