# Verification log — ID-SIDELINE (id book worker coverage, #13900)

All commands run 2026-11-23 (CST) unless noted. Repo work on the laptop via
`~/tools/laptop-ssh.sh`, worktree `~/wt-id-workers-book`, branch
`book/id-workers`. No platform execution-risk flag tripped at any point; the
primary checkout `~/git/code` was never modified (all git mutations ran in
the worktree; one combined command was refused by the laptop-agent
primary-checkout guard, code#10095, and was re-run split, worktree-only).

## Landing identifiers

| Item | Value |
|---|---|
| Worktree base (origin/main at creation) | `0ecb31eef33031853569ee9548c12a211563ee39` |
| Commit | `bb0b439c8ad3bdb9f7a5d757147345d04d378be7` — "docs(id book): cover the mail Worker and the two Worker proofs of concept (#13900)" (message via file, surgical staging: exactly the 8 files listed in book-survey-notes.md) |
| PR | https://github.com/developing-today/code/pull/13914 |
| Merge | MERGED 2026-11-24T00:51:24Z, merge commit `68480311fc1db8b1f71553185f4af394ff73cf37` (merge commit, via laptop `gh pr merge 13914 --merge`) |
| Pre-merge freshness check | At merge time origin/main was `c4e2e7294b3e30cfc3d0f8af1139c748d02e42ba` (intervening merges #13906 mcpx book, #13904 audit report). `git diff HEAD origin/main -- pkgs/id/book nixos/services/id-mail pkgs/id/worker-poc` showed exactly this PR's own changes and nothing else — no surveyed source or book file had moved underneath the chapters. `gh pr view` reported MERGEABLE. |

## Transfer integrity

- Drafts tar (mail-worker.md, worker-poc.md, d1-journal-poc.md) gzip'd in
  the sandbox, base64'd (19,064 chars), sent as 4 argument chunks through
  the laptop channel, reassembled on the laptop.
- sha256 both sides: `b7b46952c5e81de9912c75827b6cc64a0c2853b95ee300d0d44629f04d0623dc` — match.
- Commit message and PR body likewise travelled as files (base64), never as
  heredocs through the channel.

## Mechanical audit (script run in the worktree over pkgs/id/book)

Script checks: every SUMMARY.md entry's file exists; every book .md file is
listed in SUMMARY; every chapter has exactly one H1 outside code fences and
it equals its SUMMARY text; every internal .md link resolves and its text
equals the target chapter's title; worker-name coverage in SUMMARY.

Result: `entries=106 problems=0`
- coverage mail: ['The mail Worker API']
- coverage id-worker-poc: ['id-worker-poc: the World Durable Object proof of concept']
- coverage id-d1-journal-poc: ['id-d1-journal-poc: the D1 journal proof of concept']

## Builds

1. Plain mdBook (laptop's mdbook v0.5.4), in `pkgs/id/`:
   `mdbook build` → exit 0; log 3 lines, all INFO ("Book building has
   started", "Running the html backend", "HTML book written to …/rendered")
   — zero warnings. Rendered dir contained d1-journal-poc.html,
   mail-worker.html, worker-poc.html. (Rendered dir removed afterwards;
   it is untracked build output.)
2. Nix gate build (the CI derivation), in the worktree, files staged first
   (Nix flakes only see tracked files):
   `nix build .#id-docs --print-build-logs` → built
   `/nix/store/af85rvq91073im1a2f3mslcygy8yls5h-id-docs.drv`; mdBook inside
   emitted only the same 3 INFO lines — zero warnings. Output contained
   `index.html` (INDEX-OK) and the three new pages. The only other line was
   nix's standard "Git tree is dirty" notice, present because the changes
   were staged-but-uncommitted at build time; it is not a book warning.
   (`result` symlink removed afterwards.)
3. Rendered-HTML spot check (grep over the nix output):
   - mail-worker.html: `mail.security.cab` ×6
   - worker-poc.html: `id-worker-poc` ×6, `id-d1-journal-poc` ×2
   - d1-journal-poc.html: `id-d1-journal-poc` ×8, `id-worker-poc` ×1

Zero NEW warnings claim: both builds' mdBook logs contain no warning lines
at all, so there is nothing to diff against a baseline.

## Hedge and placeholder passes

- `grep "shaped|reportedly|appears to"` over the drafts: three "shaped"
  phrasings found and reworded before transfer; final drafts contain none
  of the hedge forms as unverified claims.
- Angle-bracket scan: remaining `<…>` occurrences are inside code fences or
  backtick code spans only (verified by grep with line contexts).

## Survey reads (ground truth, all on the laptop clone)

Read in full: mail worker `config.json`, `cloudflare.config.ts`,
`package.json`, `src/index.js`, `src/transports/email.js`, `src/limiter.js`,
`deploy.mjs`, `deploy.sh`, `deploy.nix`, `DEPLOYMENT.md`;
`nixos/services/id-mail/default.nix`, `cloudflare-mail.sh`;
`nixos/hosts/default.nix` (id-mail host block); the three
`id-mail-worker-*.yml` workflows (headers/env greps);
`runtime.test.mjs` test titles; worker-poc `wrangler.toml`, `README.md`,
`package.json`, `src/worker.ts`, `src/manifest.ts`, `src/guest.ts` (header +
interface); d1-journal-poc `wrangler.toml`, `README.md`, `worker.mjs`;
native `directory_mail.rs` (head + sink env section) and the `code_mail`
function in `directory_view.rs`; the id book's `book.toml`, `docs.nix`,
`id-docs.yml`, SUMMARY.md, hosted/hosted-worker/services/
keeping-this-book-current/deploying-this-book/roadmap chapters (heads +
targeted sections + greps as recorded in the survey files).

NOT run (stated limits): the mail worker's Node test suites and the PoCs'
smoke/compare harnesses were not executed — their results are reported from
the packages' own records (README acceptance list, workflow definitions),
and the chapters cite them as such. No live Cloudflare state was checked;
the mail Worker's production status is stated from repo config + host
wiring, and DEPLOYMENT.md's "environment does not yet exist" note is
repeated as the package's own record, not as a live check. The combined
Pages deploy was not re-run by this lane (it runs on main via
mcpx-docs.yml; sibling-lane merges #13906/#13914 both feed it).
