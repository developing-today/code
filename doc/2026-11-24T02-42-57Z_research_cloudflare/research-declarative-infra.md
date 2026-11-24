# Declarative ways to build Cloudflare infrastructure — and templates another person can run

Researched 2026-10-20. Every claim is tied to a source URL. Items I could not verify
from a primary source are flagged **UNVERIFIED**. "GA" vs "beta" is called out explicitly.

**Short version:** for a Worker-centric app, the repo itself is already the template:
`wrangler.jsonc` + Wrangler's automatic provisioning (or the Deploy to Cloudflare button
for zero-local-tooling). Terraform/Pulumi are the only routes that also manage zones,
DNS, WAF and account settings. Alchemy and SST sit in between: TypeScript-native,
Cloudflare-focused (Alchemy) or multi-cloud (SST), at the cost of an extra toolchain —
and Alchemy v2 is still in beta.

---

## 1. Wrangler config (`wrangler.jsonc`) as declarative infrastructure

Wrangler's docs explicitly position the config file as the source of truth for a Worker:
"It is best practice to treat Wrangler's configuration file as the source of truth for
configuring a Worker." One file declares the Worker name, entrypoint, compatibility
date, routes/custom domains, cron triggers, static assets, observability, and every
binding.
Source: https://developers.cloudflare.com/workers/wrangler/configuration/

### What the config can declare

Binding/resource sections documented in the configuration reference include:

- **Compute/app**: the Worker script itself, routes, cron `triggers`, static `assets`,
  `vars`, service bindings, `observability`, `placement`, `limits`
- **Storage**: `kv_namespaces`, `d1_databases`, `r2_buckets`, `analytics_engine_datasets`
  (Analytics Engine), `hyperdrive`, `vectorize`, AI Search namespaces/instances
- **State/messaging**: `durable_objects` (bindings + `migrations` or the newer `exports`),
  `queues` (producers and consumers), `workflows`
- **AI/media**: `ai` (Workers AI binding — no resource to create, it's just a binding),
  Browser Run, Images, mTLS certificates, Secrets Store / `secrets`
- **Containers**: a `containers` section declaring the image (Dockerfile path or a
  pre-built registry image), `instance_type`, `max_instances` and scheduling policy,
  attached to a Durable Object class
- **Platform**: dispatch namespaces (Workers for Platforms), email bindings/addresses,
  environments (`env.<name>` overrides for staging etc.)

Source: https://developers.cloudflare.com/workers/wrangler/configuration/
(binding list cross-checked against the reference's section list: D1, KV, R2, Queues,
Durable Objects, Hyperdrive, Vectorize, Workers AI, Workflows, Containers, Analytics
Engine, static assets, service bindings).

### Auto-created vs must pre-exist

**Automatic provisioning (the big change).** If a binding in the config has *no resource
ID*, Wrangler creates the resource for you:

- The configuration reference states: "Wrangler can automatically provision resources
  for you when you deploy your Worker… This currently works for the following resources:
  KV, R2, D1, Flagship, AI Search, Agent Memory, Dispatch Namespaces and Queues."
  Resources are created with the Worker name as prefix; on `wrangler deploy` the new IDs
  are written back into the config file; `wrangler dev` creates local equivalents that
  persist between runs.
  Source: https://developers.cloudflare.com/workers/wrangler/configuration/ (Automatic
  provisioning section)
- The feature launched 2025-10-24 as an **open beta** covering KV, R2 and D1, requiring
  wrangler ≥ 4.45.0, disable-able with `--no-x-provision`; the changelog also calls out
  the template angle: "resources will stay linked across future deploys even without
  adding the resource IDs to the config file. This is especially useful for shared
  templates, which now no longer need to include account-specific resource IDs."
  Source: https://developers.cloudflare.com/changelog/2025-10-24-automatic-resource-provisioning/
  The current configuration reference lists a wider resource set and presents it without
  a beta label; whether it has formally graduated from beta is **UNVERIFIED** beyond the
  docs presentation.

**Created by deploy itself:**

- Durable Object namespaces/classes are created by declaring the binding plus a
  migration (or the newer `exports` key); there is no separate "create DO" step.
  Source: https://developers.cloudflare.com/workers/wrangler/configuration/
- Container applications: declaring the `containers` block and running
  `wrangler deploy` builds the image (or uses the referenced registry image) and
  creates/updates the container application.
  Source: https://developers.cloudflare.com/workers/wrangler/configuration/
- Queues are now in the auto-provisioning list above. Workers AI needs nothing created.

**Must pre-exist (not auto-provisioned by `wrangler deploy`):**

- **Hyperdrive** — the config binding takes "the ID of the Hyperdrive configuration",
  i.e. a Hyperdrive config (which wraps an existing external Postgres/MySQL database
  and its credentials) must be created first (`wrangler hyperdrive create`, dashboard,
  or Terraform's `cloudflare_hyperdrive_config`).
  Source: https://developers.cloudflare.com/workers/wrangler/configuration/ (Hyperdrive
  binding section)
- **Vectorize** — not in the Wrangler auto-provisioning list; the binding references an
  existing index (`index_name`) created beforehand via `wrangler vectorize create` or
  the dashboard. (Note: the *Deploy button* flow below does list
...[truncated 14597 chars]  Vectorize among the resources it provisions — the two flows have different lists.)
- **The target zone** for any route or custom domain must already exist and be active
  in the account; Wrangler does not create zones or DNS.
- **Secrets** are never in the config: they are set out-of-band with
  `wrangler secret put` / `wrangler secret bulk` (locally, `.dev.vars`).
  Source: https://developers.cloudflare.com/workers/wrangler/configuration/

### Reproducibility for another person

Clone → `npm install` → `npx wrangler login` (OAuth) *or* set
`CLOUDFLARE_API_TOKEN` + `CLOUDFLARE_ACCOUNT_ID` → `npx wrangler deploy`.
With ID-less bindings, the deploy provisions KV/R2/D1/Queues/etc. into *their* account
and writes the fresh IDs back into their local config copy. `wrangler dev` runs the
same config locally (Miniflare/workerd emulation) with locally-provisioned resources.
This is the lowest-friction route that still leaves the user in control of a normal
git repo. The committed `wrangler.jsonc` should ship bindings *without* IDs (or with
clearly-placeholder IDs) so each deployer gets their own resources.

### What Wrangler does NOT manage

Zones, DNS records, zone settings, WAF/firewall rules, cache rules, Page Rules,
account membership/settings, Registrar domains, Tunnels/Access policy (beyond binding
a Worker), and billing plans. That gap is what Terraform/Pulumi fill.

---

## 2. Terraform — `cloudflare/cloudflare` provider

**Status: GA, currently v5.x** (Cloudflare docs examples pin `~> 5` and `~> 5.19`;
`cloudflare_workflow` was added in provider v5.11.0, 2025-10-09).
Sources: https://developers.cloudflare.com/pipelines/reference/terraform/ ·
https://developers.cloudflare.com/changelog/2025-10-09-workflows-terraform/index.md

### Breadth — the widest of any option here

The provider manages the layers Wrangler doesn't, plus most of the developer platform:

- **Zones & DNS**: `cloudflare_zone`, `cloudflare_dns_record`, zone settings
  (`cloudflare_zone_setting`), rulesets, Page Rules, cache/Tiered Cache, SSL/cert packs
- **Security/network**: WAF rulesets, firewall rules, rate limiting, Bot Management,
  API Shield, Load Balancers + pools/monitors, Spectrum, Argo/Tiered Cache,
  Magic Transit/WAN resources, Tunnels (`cloudflare_tunnel_config` in Cloudflare One docs)
- **Zero Trust / Access**: `cloudflare_zero_trust_access_application`, groups, policies,
  identity providers, Gateway
- **Developer platform**: `cloudflare_workers_script`, `cloudflare_workers_route`,
  `cloudflare_workers_kv_namespace`, `cloudflare_d1_database`, `cloudflare_r2_bucket`
  (+ R2 CORS/lifecycle/event notifications, R2 Data Catalog), `cloudflare_queue` +
  `cloudflare_queue_consumer`, `cloudflare_hyperdrive_config`,
  `cloudflare_pages_project`, `cloudflare_workflow`, Pipelines resources
  (`cloudflare_pipeline`, `_stream`, `_sink`)
- **Account/admin**: `cloudflare_account`, account tokens (`cloudflare_account_token`,
  `cloudflare_api_token`), account members, notification policies, Registrar domains,
  Stream, Images

Sources: https://developers.cloudflare.com/changelog/product/terraform/ (resource names
in release notes) · https://developers.cloudflare.com/tunnel/guides/terraform/ ·
https://developers.cloudflare.com/pipelines/reference/terraform/ ·
https://registry.terraform.io/providers/cloudflare/cloudflare/latest/docs/resources/workflow

Worker *code* in Terraform is clunky compared to Wrangler: `cloudflare_workers_script`
takes built file content + `content_sha256`, and the newer granular
`cloudflare_worker_version` / deployment resources were still labelled **beta** in the
2025-10-09 changelog.
Source: https://developers.cloudflare.com/changelog/2025-10-09-workflows-terraform/index.md

### State & maturity caveats

- Standard Terraform state rules apply: the state file is a local `terraform.tfstate`
  unless *you* configure a remote backend (for a team, you must). Nothing Cloudflare-
  specific hosts it for you. Auth is an API token passed to the provider
  (`api_token`, sourced from env in practice).
  Sources: https://developers.cloudflare.com/pipelines/reference/terraform/ (provider
  block + token variable) · https://developers.cloudflare.com/terraform/llms-full.txt
  (official tutorial series: init → plan/apply → track history/state guidance)
- **v5 had a publicly rocky launch.** Cloudflare's own changelog (2025-08-29, v5.9, and
  repeated in the 2025-11-20 v5.13 notes) acknowledges "the high number of issues
  reported by the Cloudflare community related to the v5 release" and commits to a
  2–3 week stabilization cadence with a resource-by-resource stabilization programme.
  Treat v5 as GA-but-still-hardening; pin versions and read the per-release notes.
  Source: https://developers.cloudflare.com/changelog/product/terraform/
- **cf-terraforming** is Cloudflare's official Go tool (built on cloudflare-go) that
  reads an existing account/zone and *generates* Terraform config + import statements,
  for adopting Terraform over hand-built infrastructure.
  Source: https://blog.cloudflare.com/introducing-cf-terraforming/

**Verdict:** the right tool for the zone/DNS/security/account layer and for
multi-cloud estates; overkill and worse DX if the whole estate is one Worker app.

---

## 3. Pulumi — `@pulumi/cloudflare`

**Status: GA; v6.22.1 (published 2026-10-13 per the Pulumi Registry).**
Source: https://www.pulumi.com/registry/packages/cloudflare/

- Official Pulumi package in TypeScript, Python, Go, .NET and Java. It is a bridge of
  the Terraform provider — the registry's package details state "This Pulumi package is
  based on the `cloudflare` Terraform Provider" — so resource breadth tracks
  Terraform's (zones, DNS, Workers, R2, D1, Zero Trust, Tunnels…), with the same
  coverage lag. Cloudflare's own Pulumi tutorial demonstrates a *dynamic provider*
  workaround for a resource the provider didn't support yet (Vectorize), and pairs
  Pulumi with Wrangler for Worker deployment.
  Sources: https://www.pulumi.com/registry/packages/cloudflare/api-docs/tunnel/
  (package details note) ·
  https://developers.cloudflare.com/pulumi/tutorial/dynamic-provider-and-wrangler/
- State: Pulumi's model — Pulumi Cloud, or a self-managed backend (file/S3/R2-style)
  that you configure; stacks give you dev/stage/prod separation.

**Verdict:** choose it if you are already a Pulumi shop or want infra in a general-
purpose language with components; for a Cloudflare-only solo project it adds state
machinery that Wrangler/Alchemy don't need.

---

## 4. SST (sst.dev)

**Status: GA framework (SST v3, built on Pulumi); Cloudflare is a supported but
second — to AWS — provider.**
Sources: https://sst.dev/docs/cloudflare/ ·
https://sst.dev/docs/component/cloudflare/worker/

- Setup: `sst add cloudflare` adds the Cloudflare provider to `sst.config.ts`. SST
  consumes the Cloudflare Pulumi/Terraform provider — the docs example pins
  `providers: { cloudflare: "5.37.1" }`.
  Source: https://sst.dev/docs/cloudflare/
- **Components** under `sst.cloudflare.*`: `Worker` (handler path, esbuild bundling,
  `url: true`, `link: [...]`, custom domains, assets) plus D1, KV, R2 and framework
  components (e.g. `sst.cloudflare.Remix`). "Linking" a resource to a Worker injects
  access via SST's `Resource` SDK instead of raw bindings.
  Sources: https://sst.dev/docs/component/cloudflare/worker/ ·
  https://sst.dev/docs/cloudflare/
- **State can live on Cloudflare itself**: set `home: "cloudflare"` in the app config
  and SST stores app state in an R2 bucket in your account.
  Source: https://sst.dev/docs/cloudflare/
- **Credentials**: an account API token; SST's Cloudflare guide says to create it from
  the **"Edit Cloudflare Workers"** token template and add permissions such as
  *Account – D1 – Edit*.
  Source: https://sst.dev/docs/cloudflare/

**Verdict:** nice if you want SST's component/linking model or one config spanning AWS
+ Cloudflare. For a pure-Cloudflare app it's a heavier stack (SST → Pulumi → TF
provider v5) than the problem needs.

---

## 5. Alchemy (alchemy.run)

**Status: BETA. Alchemy v2 is a ground-up, Effect-based TypeScript rewrite and is
shipping as `2.0.0-beta.x` (2.0.0-beta.75, Aug 2026, per its own release notes).
Not GA.**
Sources: https://alchemy.run/blog/2026-08-25-beta-75/ ·
https://alchemy.run/blog/2026-07-30-beta-66/

- Purpose-built for Cloudflare first (it now also targets other clouds). The whole
  stack — Worker, D1 (with migrations as part of deploy), KV, R2, Durable Objects,
  Containers, Workflows, Queues, Hyperdrive, Pipelines, Vite/static sites — is
  declared in one TypeScript program (`alchemy.run.ts`) with typed bindings:
  resources are passed directly as a Worker's `env`, and `Cloudflare.InferEnv<...>`
  derives the handler's env type so infra and code can't drift.
  Sources: https://alchemy.run/cloudflare/ ·
  https://alchemy.run/cloudflare/compute/workers/
- Deploy with `bun alchemy deploy`; `alchemy dev` emulates KV/R2/D1/Queues locally
  (beta.66+) with per-resource opt-in to real cloud resources.
  Source: https://alchemy.run/blog/2026-07-30-beta-66/
- **State**: stored per stack/stage. Default is local files under `.alchemy/`
  (gitignore it; each dev gets their own `dev_$USER` stage). For teams/CI there's a
  Cloudflare-backed remote store — `state: Cloudflare.state()` — which persists state
  in a Worker backed by a Durable Object with embedded SQLite, credentials kept in
  the account's Secrets Store.
  Source: https://alchemy.run/state-store/

**Verdict:** the most ergonomic "everything in TypeScript, one command" option for a
Cloudflare-native app, and the closest in spirit to "declarative wrangler, but typed
and composable" — weighed against beta status, an Effect-flavoured API learning
curve, and a much smaller community than Wrangler/Terraform. Fine for a solo dev who
accepts beta risk; hard to recommend as the default for other people to run.

---

## 6. Cloudflare's own template ecosystem

### 6a. Deploy to Cloudflare button — **GA** (launched 2025-04-08)

A README badge linking to
`https://deploy.workers.cloudflare.com/?url=<git repo URL>`. On click, Cloudflare:

1. **Clones the source repo** (public GitHub/GitLab) into a new repo in the clicker's
   own Git account, after they connect GitHub/GitLab and Cloudflare accounts;
2. Lets them customize repository name, Worker name and resource names on one setup
   page (reflected in the cloned repo);
3. **Builds and deploys with Workers Builds** (Cloudflare's git-connected CI/CD) —
   every later push to the production branch redeploys, and PRs get preview URLs;
4. **Auto-provisions whatever the app's Wrangler config declares.** Per the docs,
   supported resources include: KV namespaces, D1 databases, R2 buckets, Hyperdrive,
   Vectorize databases, Secrets Store secrets (storage); Durable Objects, Workers AI,
   Queues (compute). "Cloudflare will read the Wrangler configuration file of your
   source repo to determine resource requirements" and updates the config with new
   IDs where applicable. The source repo should ship default values for resource
   names/IDs so provisioning succeeds.

Sources: https://developers.cloudflare.com/workers/platform/deploy-buttons/ ·
https://blog.cloudflare.com/deploy-workers-applications-in-seconds/

This is also the flow behind dashboard git deploys generally: connect a repo and
Workers Builds deploys on push; the Wrangler config page notes that dashboard/GitHub
deploys also trigger provisioning, except created IDs "will only be accessible via
the dashboard… will not be written back to your repository."
Source: https://developers.cloudflare.com/workers/wrangler/configuration/

### 6b. create-cloudflare (C3) — **GA**, official scaffolding

`npm create cloudflare@latest` — interactive scaffolder for new Workers and Pages
projects. Cloudflare's split of duties: "C3: To create new projects. Wrangler: To
build and deploy your projects." C3 generates `wrangler.jsonc`, `src/index.ts`,
`package.json`, `tsconfig.json`, initializes git, can wire framework starters
(Astro, Next.js, Nuxt, SvelteKit, Remix, Hono, React, Vue, etc.), and offers an
immediate first deploy after login.
Sources: https://developers.cloudflare.com/pages/get-started/c3/ ·
https://developers.cloudflare.com/learning-paths/workers/get-started/c3-and-wrangler/ ·
https://developers.cloudflare.com/workflows/get-started/guide/

The templates C3 offers live in the official `cloudflare/templates` GitHub
repository, which also hosts the create-cloudflare CLI itself.
Source: https://deepwiki.com/cloudflare/templates/3.1-create-cloudflare-cli
(secondary index of the repo; the repo itself is github.com/cloudflare/templates)

### 6c. Workers playground / dashboard "create from repo"

The dashboard's Workers & Pages → Create flow (import an existing Git repository)
is the same Workers Builds machinery as the deploy button, minus the badge; the
in-browser Playground is for throwaway experiments, not a distribution mechanism.
(The `workers.new` shortcut historically opened the playground — current behaviour
**UNVERIFIED**, not relied on here.)

---

## 7. Other declarative routes

- **cf-terraforming** (official, Go, built on cloudflare-go): generates Terraform
  config/imports from a live account — an adoption tool, not a runtime.
  Source: https://blog.cloudflare.com/introducing-cf-terraforming/
- **Cloudflare API + cloudflare-go / cloudflare-typescript SDKs**: everything above
  ultimately drives the REST API, so a bespoke script is always an option; not
  "declarative" in any managed-state sense. No separate source needed — this is the
  substrate the Terraform provider and cf-terraforming are built on.
- **Ansible / Crossplane**: no official Cloudflare collection/provider of
  significance was found in primary sources during this research — treat as
  **UNVERIFIED/marginal** and skip; community modules exist but are not a route to
  recommend for this use case.

---

## 8. Practical comparison — "someone clones a repo and runs 1–2 commands"

### What the other person needs, per route

| Route | They need | Commands | State to manage |
|---|---|---|---|
| Deploy button | CF account + GitHub/GitLab account | 0 local (clicks only) | None (CF hosts repo copy + Builds) |
| Wrangler config | CF account; Node; `wrangler login` OAuth **or** scoped API token + account ID env vars | `npm install`, `npx wrangler deploy` | None — config file is the state |
| Alchemy | CF account + API credentials; Bun | `bun install`, `bun alchemy deploy` | `.alchemy/` local dir, or CF-backed store |
| SST | CF account API token ("Edit Cloudflare Workers" template + perms) | `npx sst deploy` | SST state (can be an R2 bucket via `home: "cloudflare"`) |
| Terraform | CF API token; Terraform CLI; a state backend decision | `terraform init && terraform apply` | tfstate — **you** must host/share it |
| Pulumi | CF API token; Pulumi CLI + language runtime | `pulumi up` | Pulumi Cloud or self-managed backend |

### Ranking for a solo dev sharing a Cloudflare app

1. **Wrangler config + automatic provisioning** — the default. One committed file is
   the template; the runner needs only a Cloudflare account and login; no state
   files, no extra toolchain; covers the entire Worker-side stack (Workers, D1, R2,
   KV, Queues, DOs, Containers, Workflows, AI). Its blind spot is everything
   zone/account-level.
2. **Deploy to Cloudflare button** — the absolute lowest friction for *consumers*
   of your template (no local tooling at all, broadest auto-provisioning list in
   Cloudflare's own docs). Ranked second only because the deployer ends up with a
   forked repo + dashboard-managed IDs rather than a clean local-first workflow;
   ideal *in addition to* #1 — the same repo supports both.
3. **Alchemy** — best ergonomics for a TypeScript-native, multi-resource app
   (typed bindings, D1 migrations in the deploy, stages, local emulation), but
   **beta** and Effect-flavoured. Adopt knowingly.
4. **SST** — if you value its component/linking model or may span AWS + Cloudflare;
   heavier machinery (SST → Pulumi → TF provider v5) for a CF-only app.
5. **Terraform** — first choice only when the deliverable includes zones, DNS, WAF,
   Access/Zero Trust or account settings, or when it's joining an existing
   Terraform estate. Worst fit for "just run my Worker app": state backend to set
   up, clunky Worker-code story, v5 still stabilizing per Cloudflare's changelog.
   A common real-world split: **Terraform for the zone layer + Wrangler for the
   app layer.**
6. **Pulumi** — Terraform's breadth in a real programming language; sensible if
   you're already on Pulumi, otherwise the same state overhead with a smaller
   Cloudflare community than #1–#3.

### What CANNOT be fully automated (any route)

- **Having a Cloudflare account** — signup is a human, web flow. Every route assumes
  the runner already has one.
- **Paying plans** — e.g. upgrading an account to Workers Paid, and adding a payment
  method, is a dashboard/billing action; no IaC tool subscribes you.
- **R2 activation** — R2 requires an R2 subscription added through the dashboard
  checkout flow before buckets can be created ("You need a Cloudflare account with
  an R2 subscription… Complete the checkout flow to add an R2 subscription").
  Source: https://developers.cloudflare.com/r2/get-started/
- **The first API token** — token creation is a dashboard act (chicken-and-egg);
  only *after* a sufficiently-privileged token exists can Terraform manage further
  tokens (`cloudflare_account_token`). `wrangler login` OAuth avoids the token for
  interactive use.
- **Domains** — buying a domain (Registrar purchase is a checkout), transferring a
  domain in, and changing nameservers at an external registrar are outside IaC;
  a zone only becomes active in Cloudflare once nameservers point at it, so routes/
  custom domains in any tool wait on that human/DNS step.
- **Secrets' values** — secret *slots* can be declared, but the values must be
  supplied by the runner (`wrangler secret put`, SST/Pulumi/Terraform secret
  config); a template can never ship working third-party API keys.
- **Product ToS/first-run gates** — several products have dashboard first-run steps
  (R2 above is the documented example); expect an occasional one-time click-through
  that no tool can do for a fresh account.
