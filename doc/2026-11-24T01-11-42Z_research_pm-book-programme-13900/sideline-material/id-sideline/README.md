# ID-SIDELINE raw material — PM-BOOK programme #13900

Lane: ID-SIDELINE (coordinator: PM-BOOK). Date: 2026-11-23 (America/Chicago).
Outcome: PR https://github.com/developing-today/code/pull/13914 MERGED,
merge commit `68480311fc1db8b1f71553185f4af394ff73cf37`; closing record on
#13900 at https://github.com/developing-today/code/issues/13900#issuecomment-7118517853.
Task: give three uncovered ID Workers proper coverage in the id book
(`pkgs/id/book` in developing-today/code): the `mail` worker (id-mail),
`id-worker-poc`, and `id-d1-journal-poc` — the three id-book GAP rows in
`~/workspace/pm-book/coverage-ledger.md` Table A.

Deposited per Drew's standing rule (2026-11-23 ~18:46 CST): a lane's gathered
work is never thrown away into a summary; raw material is deposited as files.
This folder is the deposit for the programme's dated record
(`doc/<UTC>_research_pm-book-programme-13900/` in the code repo).

## Files

| File                       | What it is                                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| -------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `survey-mail-worker.md`    | Full surface/config dump of the mail worker from source: `config.json` verbatim, `cloudflare.config.ts` bindings, handler behaviour from `src/index.js`, email transport constants from `src/transports/email.js`, the `Limiter` DO, deploy paths (`deploy.mjs`/`deploy.sh`/`deploy.nix`, `DEPLOYMENT.md`, the three GitHub workflows), host wiring (`default.nix`, `cloudflare-mail.sh`, `nixos/hosts/default.nix`), and the native side (`code_mail`, mail sinks). |
| `survey-worker-poc.md`     | Full dump of `pkgs/id/worker-poc`: `wrangler.toml` and `package.json` verbatim, README acceptance record, `src/worker.ts` routes/journal/concurrency, `src/manifest.ts` loader refusals, `src/guest.ts` ABI notes, harness commands and results.                                                                                                                                                                                                                     |
| `survey-d1-journal-poc.md` | Full dump of `pkgs/id/worker-poc/d1-journal-poc`: `wrangler.toml` verbatim, `worker.mjs` routes/entry validation/schema/ordinal allocation/failure shapes, README scope-and-limits list, smoke harness behaviour.                                                                                                                                                                                                                                                    |
| `book-survey-notes.md`     | Survey of the id book itself: layout, SUMMARY registration point, the second-navigation-gate check (none exists — evidence), the sync contract, the issue-map/prioritised-list check (none exists for workers — evidence), existing PoC mentions found by grep, chapter placement and cross-link decisions.                                                                                                                                                          |
| `disagreements.md`         | The numbered disagreement list (MW-1…MW-4, POC-1) with evidence for each, exactly as reported in the PR and on #13900.                                                                                                                                                                                                                                                                                                                                               |
| `verification-log.md`      | Every verification actually run: mechanical audit script + result, mdbook build, `nix build .#id-docs`, rendered-HTML spot checks, transfer sha256, commit/push/PR identifiers.                                                                                                                                                                                                                                                                                      |
| `chapter-drafts/`          | The three chapter drafts as transferred to the laptop worktree (base64 tar, sha256-verified both sides). The merged book text may differ by the small in-worktree cross-link edits recorded in `book-survey-notes.md`; the chapters themselves were not edited after transfer.                                                                                                                                                                                       |

## Survey basis

- Repo: existing laptop clone `~/git/code` (never fresh-cloned). Survey reads
  began at HEAD `f86b52989c31b0697428eeb09e9af5eeb4f10ece`; the worktree
  `~/wt-id-workers-book` (branch `book/id-workers`) was created off
  `origin/main` at `0ecb31eef33031853569ee9548c12a211563ee39` (main moved
  during the run: PR #13901 merged; a diff of the moved range over
  `pkgs/id/book`, `nixos/services/id-mail`, `pkgs/id/worker-poc` was checked
  before landing — see verification log).
- All repo work happened in that laptop worktree via `~/tools/laptop-ssh.sh`.
  The primary checkout was never touched; no platform execution-risk flag
  tripped at any point.
- Method (bookwright skill): survey from source, mirror the book's existing
  layout/voice, verify every load-bearing name (routes, env, config) by
  grepping the repo, mechanical coverage audit, zero-warning build gate.
- Secrets: none appear in this deposit. Caller tokens are referenced only by
  their Bitwarden secret names (`tokenKey` values in `config.json`), exactly
  as they appear in the repo. The `CALLERS` binding holds SHA-256 digests,
  never token values; no digest is reproduced here either.
