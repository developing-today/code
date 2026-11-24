# Cloudflare ideas — parked for future consideration

Recorded 2026-10-26 at Drew's request. These came up during the
Cloudflare chat's Oct 2026 work (Lab Ops Hub, containers, Atlas
design). **None are approved or scheduled** — this file exists so the
ideas aren't lost. Each entry: what it is, why it surfaced, what it
builds on, and the smallest first step if it's ever picked up.

## 1. Personal link/file vault

A personal "save it for later" service on the Workers stack: send it a
URL or a file, it keeps both safe and findable. D1 as the catalog, R2
for the bytes, KV for the hot index, a Queue for async processing
(content extraction, hashing, later embeddings), a Durable Object for
live state.

- Why: Lab Ops Hub already implements most of the machinery
  (file upload → R2 → queue-driven indexing → D1; container crawl →
  stored extract). The vault is that demo generalized into a tool
  used daily instead of a surface tour.
- First step if picked up: decide what "save" captures for a URL —
  bookmark only, extracted text (the crawler already does this), or a
  full snapshot in R2 — then fork the Lab Ops Hub project as the base.

## 2. Email Workers → D1/R2 pipeline

Inbound mail to Drew's own domains, handled by Email Routing +
Email Workers: headers and metadata indexed in D1, bodies and
attachments stored in R2, a Queue feeding classification and
summarization. A searchable, owned mail archive plus alert
extraction, independent of Gmail.

- Why: builds directly on Drew's existing id-mail work, and
  complements the Daily Gmail summary goal with infrastructure he
  owns end-to-end for his own domains' mail.
- First step if picked up: choose one domain + one address rule and
  land the metadata-only version (no body storage) as the spike.

## 3. D1 as its own thread

Promote D1 from a sub-topic of the Cloudflare chat to a dedicated
line of work: scheduled export-to-R2 backups (the Cloudflare
Workflows pattern) as the standard exit ramp, read replicas (note:
billable from 2027-03-01), the Sessions API, and D1's settled Atlas
role — the Workers-native projection tier (catalog, hot metadata,
cached answers) beside Turso and Postgres.

- Why: D1 questions kept recurring (no raw .sqlite download, export
  semantics, where it fits vs Turso) and Atlas gives it a permanent
  job; it deserves its own home instead of scattered answers.
- First step if picked up: implement scheduled export-to-R2 for
  labops-db as the reference instance, then generalize.

## 4. Workflows for cluster-bootstrap pipelines

Use Cloudflare Workflows (durable steps, retries, waits, human
approval pauses) to orchestrate the lab-cluster standup pipeline —
the talm / talos / cozystack / postgres / rustfs goal — where
provisioning is long-running, multi-step, and failure-prone in the
middle.

- Why: the bootstrap is exactly the shape Workflows exists for, and
  it would give the cluster work a resumable, observable spine
  instead of shell sessions.
- First step if picked up: map the existing bootstrap runbook to
  workflow steps on paper and check the fit against the lab cluster
  goal's current state before writing code.

## 5. Sandbox SDK / Artifacts for agent code execution

Cloudflare's Sandbox SDK (with Artifacts as the git-backed versioned
storage layer) as an execution home for agent-generated code: run
untrusted or experimental code at the edge instead of on Drew's
machines, with versioned artifacts per run.

- Why: the agent fleet keeps growing, and Lab Ops Hub proved the
  account and patterns; a managed sandbox tier would complement the
  local/OpenCode execution paths and the containers work.
- First step if picked up: a minimal spike — one Worker driving one
  sandbox session — measuring cold start, limits, and cost against
  the $5 Workers Paid tier before committing to anything.
