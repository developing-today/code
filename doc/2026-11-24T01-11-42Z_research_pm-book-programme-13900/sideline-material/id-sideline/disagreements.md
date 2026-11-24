# Numbered disagreements — ID-SIDELINE survey (docs vs code)

Numbering follows the id book's survey convention (cf. CLI-*, WEB-9 in the
existing chapters). MW = mail worker surface, POC = proof-of-concept surface.
Each item names its evidence and where the reconciled behaviour is carried.
These are reported in the PR body and on #13900; per the lane brief, the
coordinator (PM-BOOK) owns any follow-up filing.

## MW-1 — Mail route count: two named, four declared

- Claim side: the programme brief and the coverage census shorthand name the
  mail worker's routes as `mail.security.cab` / `id-mail.security.cab`
  (the census Table A row reads "routes mail.security.cab,
  id-mail.security.cab, …" — with an ellipsis, so the census itself is not
  wrong, only abbreviated).
- Source side: `nixos/services/id-mail/worker/config.json` declares FOUR
  routes: `mail.security.cab`, `mail.id.security.cab`,
  `id-mail.security.cab`, `mail-id.security.cab`. `deploy.mjs` maps each to
  `{pattern, custom_domain: true}` in the generated Wrangler config.
- Carried in: reference/mail-worker.md, "Routes" table (all four, with the
  host's chosen route identified).

## MW-2 — The retire list: "not acted on" is workflow-scoped only

- Claim side: `nixos/services/id-mail/worker/DEPLOYMENT.md` — "The existing
  retire list in config.json is not acted on." and, in its scope notes, "No
  caller, route, binding, secret, or Worker retirement changes are included
  in this cutover."
- Source side: `deploy.mjs` (the operator `id-mail-deploy` path that
  DEPLOYMENT.md's first paragraph says is "unchanged") acts on the list: its
  header says it "retires old Workers", and the deploy sequence retires each
  `config.retire` entry (`id-mail`) via the Cloudflare API + `wrangler
delete --force` BEFORE `wrangler deploy`, because a hostname can belong to
  only one Worker.
- Reconciliation: both statements are true of different paths — the GitHub
  publish/promote workflows never retire; the operator command does.
  DEPLOYMENT.md's sentences are scoped by its own context (the workflows /
  "this cutover") but read unscoped in isolation.
- Carried in: reference/mail-worker.md, "Operations" (the two paths are
  described separately, with the distinction stated in exactly those terms).

## MW-3 — No wrangler.toml for the mail worker

- Expectation side: the census method keys workers by their wrangler
  configs, and a reader of the package reasonably expects one; the package
  even pins `wrangler` 4.166.0 as a devDependency.
- Source side: there is no `wrangler.toml` (or `.jsonc`) in
  `nixos/services/id-mail/worker/`. Configuration of record is
  `config.json`, consumed by `cloudflare.config.ts` (pinned `cf` CLI
  1.0.0-rc.2 `defineConfig`) and by `deploy.mjs`, which GENERATES
  `.generated/wrangler.json` at deploy time (`.generated/` is a build
  artifact; the dir also has a `.gitignore`).
- Carried in: reference/mail-worker.md, intro (stated explicitly so readers
  do not hunt for a file that does not exist).

## MW-4 — "Works for 15 minutes": the Worker enforces none of it

- Claim side: the configured template body — "Your code is {code}. It works
  for 15 minutes." — reads as a worker property.
- Source side: the Worker never generates a code, never stores one, and has
  no expiry logic; template matching (`src/transports/email.js`) only
  requires the `{code}` slot to be exactly six digits (`\d{6}`). Code
  generation, the 15-minute lifetime and verification live in the native
  directory code: `code_mail` in
  `pkgs/id/crates/id-directory/src/directory_view.rs` builds exactly the two
  templated messages, and `directory_auth.rs` holds the code state.
- Carried in: reference/mail-worker.md, "What it does, and for whom" (the
  flow assigns each step to its owner) and "Relationship to the hosted
  Worker and the IdP".

## POC-1 — worker-poc README omits GET /state from its route list

- Claim side: `pkgs/id/worker-poc/README.md`, layout bullet for
  `src/worker.ts`, names `POST /step` and `POST /cold-start` only.
- Source side: `src/worker.ts` implements three routes — its own header
  comment lists `POST /step`, `GET /state`, `POST /cold-start`, and the
  handler answers GET /state with `{step, view, records, snapshotHash}`.
- Carried in: reference/worker-poc.md (route table follows the source; the
  final paragraph notes the README gap). The pre-existing book chapters
  (concepts/hosted.md, reference/hosted-worker.md) already list all three
  routes correctly — the README is the only stale side.

## Checked and found NOT to be disagreements

- Census Table A calls the mail worker's status "retires id-mail" — matches
  config.json's retire list (subject to MW-2's path scoping).
- Book's existing d1-journal-poc route list (hosted-worker.md) vs
  worker.mjs: identical (GET /health, POST /append, GET /snapshot,
  GET /tail).
- Book's existing worker-poc route/binding claims (hosted.md,
  hosted-worker.md) vs wrangler.toml + worker.ts: identical (WORLD binding,
  v1-sqlite migration, CompiledWasm rule, journal as source of truth, 64 MB
  loader ceiling, portability=native refusal).
- config.json templates vs native code_mail output: subject strings and
  body template match exactly, including the trailing newline.
- cloudflare.config.ts bindings vs deploy.mjs generated wrangler config:
  same four bindings (MAIL_CONFIG with the same four keys, EMAIL with the
  same sender list, LIMITER → Limiter, CALLERS secret), same migration
  (v1, new_sqlite_classes ["Limiter"]).
- DEPLOYMENT.md's probe expectation (unauthenticated POST → 401 after
  CALLERS/MAIL_CONFIG validation, before DO/email access) vs src/index.js
  ordering: method check (405) precedes config validation, but a POST probe
  follows exactly the documented path — config validation (500 if broken),
  then auth (401) — and rate-limit/DO access happens only after auth.
  Consistent.
