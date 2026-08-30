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

## Read the real skill

Once `doc/hardware` resolves:

| What | Where |
|---|---|
| **The method** (workflow, evidence rules, verification, required README content) | `doc/hardware/.agents/skills/hardware-device-research/SKILL.md` |
| Working conventions, symlink and archive rules | `doc/hardware/AGENTS.md` |
| Entry points, research passes, indexes | `doc/hardware/README.md` |
| Per-site retrieval findings and user-agent matrix | `doc/hardware/ai-crawler-site-access-table.md` |
| What is reproducible vs irreplaceable | `doc/hardware/SIZE-AUDIT.md` |
| Netlist parsers, ESP image parser, archiver | `doc/hardware/tools/` |

**Deliberately not duplicated here.** An earlier version of this file carried the full ~875-line
method, which is exactly the kind of copy that drifts from the original. One authoritative copy,
in the repo that uses it.

## The few things worth knowing before you get there

Enough to orient; not a substitute for the real skill.

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

Inside `hardware-doc` the archive is reachable as `archive/` and its working files as `scratch/` —
both tracked symlinks into `repo-archive`, which is namespaced per source repository.
