# AGENTS.md

## Hardware research lives in a separate repository

Hardware device/component research is **not** in this repo. It lives in
**[`developing-today/hardware-doc`](https://github.com/developing-today/hardware-doc)**, checked
out beside this one and symlinked in at `doc/hardware` (gitignored).

```
<repo-parent>/
├── code/                      ← this repo
│   └── doc/hardware  ───────┐   symlink
├── hardware-doc/       ←────┘   the knowledge base
└── repo-archive/        bulk artifacts (separate repo, usually unpublished)
```

```bash
./scripts/hardware-doc-init.sh    # clone/update + symlink; runs automatically in the devshell
```

The script only ever **fast-forwards**, and refuses to touch the checkout if it is dirty,
detached, diverged or has unpushed commits — it warns instead. It is safe to re-run.

**Write and commit hardware research in `hardware-doc`, not here.** It is deliberately not a
submodule and not vendored: at ~440 MB it would make every clone of this repo roughly 6.5×
larger. Conventions live in `../hardware-doc/AGENTS.md`; the authoritative research method is
`../hardware-doc/.agents/skills/hardware-device-research/SKILL.md`.

Two sibling paths, both relative to the **real repository root** (worktree-safe):

```bash
ROOT="$(dirname "$(git rev-parse --path-format=absolute --git-common-dir)")"
PARENT="$(dirname "$ROOT")"     # $PARENT/hardware-doc  and  $PARENT/repo-archive
```

Use `--git-common-dir`, not `--show-toplevel`: inside a linked worktree the toplevel is the
worktree, whose parent is the wrong directory.

`doc/hardware` is a **tracked symlink**, committed relative as `../../hardware-doc`, so a
fresh clone gets a working link with nothing to run. Where that relative form cannot resolve
— typically a worktree outside the repo parent — the init script substitutes an absolute
path and marks the file `--skip-worktree`, because **`.gitignore` does not apply to tracked
paths** and cannot suppress the resulting diff.

While the flag is set git will not update that path, so if the committed target ever changes
upstream a flagged clone will not pick it up. Undo with:

```bash
git update-index --no-skip-worktree doc/hardware
```

The script prints that command whenever it sets the flag. The same pattern links
`hardware-doc/archive -> ../repo-archive` when the archive exists locally.

### Committing hardware research

`doc/hardware`, `archive/`, `scratch/` (and the `doc/` variants) are symlinks into **other
repositories**. Writing through them works; `git` will not cross them:

```
$ git add doc/hardware/devices/foo/README.md
fatal: pathspec '...' is beyond a symbolic link
```

So commit from inside the link, not from here:

```bash
git -C doc/hardware add -A && git -C doc/hardware commit -m "..."   # research records
git -C archive      add -A && git -C archive      commit -m "..."   # artifacts + scratch
```

`archive/` and `scratch/` land in the **same** repository, and `git add -A` from either stages
all of it, so one commit covers both. (`git add .` stages only the directory you are in.)

Commit freely in `archive/` and `scratch/` — including temporary or undecided material. An
uncommitted artifact is the one that gets lost.

### Do not disturb work you did not create

These checkouts are shared between sessions. **Never** `git reset`, `git stash`,
`git checkout -- <path>`, `git clean`, or delete files you did not create — in any of the
three repositories. Another session may be mid-task, and uncommitted work is unrecoverable.
If something is in the way, commit it or report it; do not clear it.

## Web retrieval fallback

When retrieving public web content with `curl`, `wget`, Python HTTP clients, or similar tools:

1. Use the normal/default request first.
2. If the request returns a 403, 429, obvious bot-block page, or otherwise fails because of automated-client filtering, retry with a normal modern browser User-Agent:
   `Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36`
3. If that still fails, try other well-known user agents — different sites block different agents, so it is worth rotating through several before giving up. Good next tries:
   - `WhatsApp/2.23.20.0` (link-preview fetcher; rarely blocked, often slips through news/paywall-adjacent CDNs)
   - `Mozilla/5.0 AppleWebKit/537.36 (KHTML, like Gecko); compatible; ChatGPT-User/1.0; +https://openai.com/bot`
   - `Mozilla/5.0 AppleWebKit/537.36 (KHTML, like Gecko); compatible; ClaudeBot/1.0; +claudebot@anthropic.com`
4. A full sorted list of known user-agent strings, which operators publish IP ranges for verification, and a site-by-site access matrix (which agents worked or failed on which domains) lives in [`ai-crawler-site-access-table.md`](./ai-crawler-site-access-table.md#user-agents) (the maintained copy is in the hardware-doc repo: `../hardware-doc/ai-crawler-site-access-table.md`). Check that section for details and prior findings — including notes about the specific domain you are hitting.
5. Do not use altered User-Agent headers when testing or debugging the actual HTTP behaviour of an application, API, website, authentication flow, or client, because doing so could hide genuine access-control or compatibility problems.
6. Do not treat a successful response obtained with an alternate User-Agent as evidence that the resource works normally for ordinary clients.
7. Do not use User-Agent substitution to bypass authentication, account permissions, or other explicit access controls.
8. **Document what works.** When plain requests fail but an alternate path succeeds, add your findings to `ai-crawler-site-access-table.md` (the agent's subsection under *User Agents*, the domain's subsection under *Sites*, or *General retrieval tips*) at any time, regardless of what the current session is about, and include the update in your next commit. Failures are worth recording too: e.g. "UA X has known failures on example.com, but UA Y worked."
9. **Preserve scarce sources.** If a URL was genuinely useful and hard to acquire — very few copies online, or hosted somewhere unlikely to persist — submit it to `https://web.archive.org/save/<url>` (works unauthenticated). This applies to the kind of thing that has no mirror: datasheets/whitepaper/manual PDFs, technical docs on fragile personal or CMS-hosted pages, demo/example projects that exist only to illustrate how some code works, hard-to-find header files or source snippets not in any repo or package manager, one-off benchmark posts. It does **not** apply to widely-mirrored content (Wikipedia, MDN, popular repos) or anything already covered elsewhere — you can't and shouldn't archive every page you visit. Anonymous saves are rate-limited to roughly a few per minute; don't hammer it.
   - **Rare case:** if you are confident the site is still up but you are blocked or rate-limited, and you already know from some other source that the URL is valuable (a citation, an API doc you need, source code relevant to the task), you may submit it unseen and record that it wasn't directly accessible but was submitted on `<date>` — e.g. in the relevant *Sites* subsection of `ai-crawler-site-access-table.md` — so a future agent can check `web.archive.org` for the capture. Only do this when there's a concrete reason the page matters, not speculatively.
