# Supplement 2 — Dictated-name research: agent sandbox / dev-environment space

_Research lane note, 2026-11-23, web only. Vendor claims labelled as
such; pricing from the vendors' own pages/docs as crawled that day._

## 1. "Freestyle.sh"

**Verdict: IDENTIFIED — real, and exactly in this space.**

- **What it is:** Freestyle (Freestyle Cloud Inc., San Francisco) sells
  full Linux VMs for AI agents — not containers or a narrow code runner.
  Each VM has root, SSH, systemd, multiple users/groups,
  configurable/default-deny networking, and nested virtualisation,
  running on Freestyle's own bare metal. Agents use it as a code
  interpreter, dev machine, app-builder runtime, or long-running
  production environment.
  - https://www.freestyle.sh/blog/product/top-5-most-powerful-sandboxes
  - https://agentsandboxes.org/sandboxes/freestyle/
- **Company / maturity / funding:** Founded 2024 by Benjamin Swerdlow
  (CEO) and Jacob Zwang (CTO), ex-Apple; Y Combinator Summer 2024. It
  pivoted: full-stack TypeScript cloud (2024) → deployment for AI site
  builders (2025; customers named include Onlook, Rork) → VM sandboxes
  for coding agents (Launch HN, April 2026, 322 points). Investors named:
  Y Combinator, Floodgate, Two Sigma Ventures. Still a very small team
  (~4–5 people in profiles) — early-stage, but positioning for
  production, with an Enterprise tier (SAML/SCIM, audit logs, stated
  99.99% SLA, GPU/dedicated hardware via sales). No large priced round
  found; aggregators show only ~$500K (July 2024, consistent with the YC
  deal).
  - https://www.ycombinator.com/launches/Lkp-freestyle-a-new-way-to-build-fullstack-typescript-apps
  - https://twosigmaventures.com/portfolio/company/freestyle/
- **Pricing shape:** Usage-metered per second, with monthly minimums that
  act as usage credit. Free $0 (200 vCPU-hours + 400 GiB-hours/month, 10
  concurrent VMs, no persistent VMs); Hobby $50/mo; Pro $500/mo;
  Enterprise custom. Unit rates: $0.04032/vCPU-hour, $0.0129/GiB-hour
  RAM, $0.000086/GiB-hour storage, $0.02/GB transfer.
  - https://www.freestyle.sh/pricing
  - https://www.freestyle.sh/docs/vms/pricing-and-limits
- **State / snapshot model — its differentiator:** Memory-preserving
  suspend/resume (vendor claim: resume <100 ms; paused VMs bill storage
  only), **live forking of a running VM including memory**, snapshots as
  immutable layers bootable into new VMs, and layered VM-spec caching.
  Paid VMs can run indefinitely with idle timeout disabled.
- **Self-hostable?** No evidence of it. Proprietary hosted service; not
  open source, self-host/BYOC "not documented" in independent
  comparisons. Only the npm SDK/CLI is MIT. (Its open-source _Adorable_
  app builder, MIT, runs on top of Freestyle VMs — that is not a
  self-hosted Freestyle.)

## 2. "boat.dev"

**Verdict: IDENTIFIED — boat.dev is literally a real agent-sandbox
product. This was not a misdictation.**

- **What boat.dev resolves to:** boat — cloud sandboxes for AI agents.
  Each sandbox is a full Ubuntu VM with its own kernel, root/sudo, SSH,
  Docker, /dev/kvm, a public URL per port, and a streamable 60 fps
  desktop with Chromium. Seven built-in agent harnesses (Claude Code,
  Codex, OpenCode, Pi, Kimi, Mistral, Prime) can be prompted, steered
  mid-task, and event-streamed via one API; TypeScript/Python SDKs, CLI,
  REST API (`https://boat.dev/api/v1`).
  - https://boat.dev/
  - https://docs.boat.dev/llms.txt
  - https://boat.dev/compare/boat-vs-exe-dev _(vendor's own comparison
    site — boat also built battleships.dev, so treat rankings as
    marketing)_
- **Company / maturity:** Paris-based, 3-person team, Y Combinator Fall
  2026; founders Anicet Nougaret (CEO) and Kirill Makarov (CTO). **It was
  previously ASCII's "Box" (box.ascii.dev) — renamed Box → boat on 15
  September 2026**, same accounts/machines/data; legacy Box endpoints
  still work with deprecation headers. Self-reported (not independently
  verified) ARR: $1M on 2026-11-07, $1.43M in an investor memo dated
  2026-11-16.
  - https://docs.boat.dev/migrating-from-box
  - https://agentsandboxes.org/sandboxes/boat/
  - https://sandbox.watch/p/box-by-ascii
  - https://www.orangecollective.vc/memos/boat
- **Pricing shape:** Per-second billing while running; stopped sandboxes
  free. Plans are pre-bought VM time: $20 / $100 / $500 / $2,000 per
  month (100–2,000 concurrent sandboxes by plan), overage from a dollar
  wallet. Sizes: small 2 vCPU/4 GB $0.018/h; default 4 vCPU/8 GB
  $0.036/h (≈$26/mo full-time); large $0.072/h; xlarge $0.20/h. 7-day
  trial (card required, 25 h compute).
- **State / snapshot model:** Disk (not memory) snapshot automatically
  **every minute** while running and again at stop; a stopped sandbox's
  disk is kept at least 14 days, saved snapshots have no scheduled expiry
  and work as shareable templates; fork from the latest snapshot. Resume
  is effectively a reboot, except a resume within ~10 minutes of stop
  usually lands on the same live machine. EU regions only (Germany,
  Finland, France) on dedicated Ryzen hosts.
- **Self-hostable?** No — closed, hosted only; no self-hosting/BYOC
  documented.

## 3. "chai systems"

**Verdict: NOT PINNED — no product named "Chai Systems" (or "Chai")
exists in the agent-sandbox / dev-environment space that research could
find. Literal namesakes exist, but none match the context.**

Literal matches, all context mismatches:

- **CHAI Systems — https://chai-systems.dev/** — the closest literal hit.
  An AI-first one-founder company (founder: Himesh Ramjee) building a
  personal/founder "operating system": Business OS (in daily use by the
  founder), Memory OS, and an upcoming Family OS. Agent-adjacent in
  philosophy, but **not** developer infrastructure: no sandbox, no
  VM/code execution, no API/SDK or pricing found; early-access/waitlist
  stage.
- **ChAI Systems LLC — https://chai-systems.ai/** — Philadelphia-area AI
  _consulting_ firm: quantitative modelling, risk/credit evaluation,
  healthcare/lab analytics, AI training/governance. Not a product
  platform.
- **CHAI™ by CloudHedge — https://www.cloudhedge.io/events/aws-reinvent-2026/**
  — a patented agentic-AI platform for _enterprise application
  modernisation_. Agentic, but not sandboxes or dev environments.
- **Chai Discovery** — the largest "Chai" AI company, but it's AI drug
  discovery, not infrastructure.

Nearest real in-space sound-alikes:

- **shai (shai.run, by Colony-2)** — open-source CLI that sandboxes AI
  coding agents (Claude Code, Codex, Gemini CLI, Cline, OpenHands)
  inside Docker with restrictive-by-default "Resource Sets" in
  `.shai/config.yaml`; MIT, free/BYOK. Small and quiet (≈40 GitHub stars
  as of June 2026). Phonetically the closest in-space match to "chai."
  — https://rywalker.com/research/shai
- **chi (getchi.dev)** — open-source harness that runs a fleet of coding
  agents against an evaluator, with opt-in Docker sandbox tiers.
  — https://getchi.dev/blog/sandbox-ai-coding-agents-safely/
- **Cua (trycua/cua)** — open-source computer-use agent infrastructure
  with sandboxes — in-space, but phonetically further away.

Bottom line: if Drew saw the words "CHAI Systems" written down,
chai-systems.dev is almost certainly the entity — but it should not be
filed as a sandbox competitor. If he heard it spoken in a sandbox
discussion, **shai** is the most plausible intended product.

## 4. "Boxer.dev"

**Verdict: IDENTIFIED (literal domain) — but there are three different
"Boxer"s, and the one at boxer.dev is probably not the agent-sandbox one.
Do not merge them.**

### 4a. boxer.dev itself — Dan Phillips' Wasm compute project

- https://boxer.dev/ is a sparse teaser: _"Boxer: A new compute
  primitive for the next era — Much more info coming Fall 2026,"_ linking
  to **Weave** (github.com/dphilla/weave — universal live-migration and
  snapshot/restore for WebAssembly across runtimes/hosts).
- Its documented 2025 incarnation (FOSDEM 2025, "Moving Beyond Containers
  — Introducing Boxer," by Dan Phillips / dphilla): open-source Rust
  tooling that converts existing containerised workloads/Dockerfiles
  into near-universally deployable WebAssembly distributions called
  "Boxes," with the Marcotte layer-virtualisation tool underneath.
  - https://archive.fosdem.org/2025/schedule/event/fosdem-2025-6115-moving-beyond-containers-introducing-boxer/
  - https://deepwiki.com/dphilla/boxer
- Maturity: solo/open-source research project, pre-release relaunch; no
  company, pricing, or funding found. Adjacent to sandboxing (Wasm
  isolation, snapshot/restore), but **not** a hosted agent dev-environment.

### 4b. The in-space namesake: BarakChamo/boxer (not boxer.dev)

- **"Parallel coding agents, each in its own sandbox."** One sandbox per
  git worktree (smolvm microVM with its own kernel + network allowlist
  by default, or Docker/Podman/Apple Container), with per-worktree
  dev-server ports/URLs, installed via hooks/MCP for Claude Code, Codex,
  OpenCode, Gemini CLI, Copilot CLI, Pi, and orchestrators like
  Conductor. Go, Apache-2.0, self-hosted/local, v1.x (docs reference
  v1.4.2). Docs live on GitHub Pages, **not** boxer.dev — no evidence the
  two Boxers are related. If Drew's mention was about parallel local
  coding agents, this is the best fit.
  - https://pkg.go.dev/github.com/BarakChamo/boxer@v1.3.0
  - https://barakchamo.github.io/boxer/docs/quickstart

### 4c. Boxlite — the requested check: real, and distinct from both Boxers

- **BoxLite (boxlite-ai) — IDENTIFIED:** an embeddable micro-VM runtime
  for AI agents. A "Box" is a hardware-isolated micro-VM (KVM /
  Hypervisor.framework) that runs any OCI image and persists.
  Daemonless: embed it as a library (Python, Node, Go, Rust, C SDKs),
  run the binary, run it as a REST server, or deploy your own agentic
  cloud into your AWS/GCP account. QCOW2 copy-on-write disks (clone,
  export/import `.boxlite` archives), egress allowlists, host-side
  secret injection (real secrets never enter the VM). Apache-2.0,
  **self-hostable by design** — its main pitch vs. hosted sandboxes.
  - https://github.com/boxlite-ai/boxlite (created 2025-12-07; 2,684
    stars, 54 releases at fetch time)
  - https://docs.boxlite.ai/manage-sandbox/sandbox-types

### 4d. Other near-names that could be the dictation source

- **Boxes.dev** (YC Fall 2026; Nick Bushak & Drew Regitsky, ex-Gem):
  every Claude Code/Codex agent gets its own cloud Linux computer cloned
  from a snapshot of your dev setup (Firecracker VMs, on E2B
  underneath); sleeping boxes free, plans $19/$99/$249 per user/mo.
  — https://boxes.dev/
- **DevBoxer** (devboxer.com): delegate tasks to background coding
  agents in hosted cloud sandboxes, built on the open-sourced Terragon
  codebase as its replacement; $30/$60 per seat/mo. Phonetically the
  closest single word to "Boxer.dev." — https://www.devboxer.com/

---

**Summary:** Freestyle.sh ✅ real (hosted full-VM agent sandboxes, YC
S24, usage pricing, memory-preserving suspend + live fork, not
self-hostable). boat.dev ✅ real and literal (formerly ASCII Box, YC F26,
cheap EU Ubuntu VMs, per-minute disk snapshots). "Chai Systems" ❌ not
pinned in this space — literal chai-systems.dev is a founder/personal OS,
not infra; best spoken-word candidate is **shai**. "Boxer.dev" ⚠️ literal
domain is Dan Phillips' Wasm/Boxer+Weave project; the agent-sandbox
"boxer" is BarakChamo/boxer (per-worktree local sandboxes), and
**Boxlite** is a separate, real, self-hostable embedded micro-VM runtime.
