# Crossplane and alternatives

Crossplane — Kubernetes objects for external infrastructure —
has been fully censused for this estate, given a verdict, and not
adopted. Status: **researched, with a verdict recorded; none
adopted**. This chapter is short because one strong census file
is the whole record. Read by the platform census of 2026-11-23
(landed as
`doc/2026-11-24T01-20-46Z_research_code-book-census-platform/`,
PR [#13959](https://github.com/developing-today/code/pull/13959)).

## What the record says

- **Primary source:** the CNCF investigation's
  `platform-and-gitops.md` §3, landed in-repo at
  `doc/2026-11-23T22-33-41Z_research_cncf-landscape/`.
- **Crossplane itself:** CNCF **Graduated on 2025-11-06** — the
  census corrects the older "incubating" assumption. It manages
  external infrastructure through Providers and
  Compositions/XRDs; version 2.0 (August 2025) made resources
  namespaced and added Operations.
- **The verdict for this estate: mostly no — not now.** The
  reasons of record: this estate's infrastructure is NixOS and
  Talos declared in Nix and Git, not cloud APIs, so providers
  have almost no surface to act on; Cozystack already *is* a
  Kubernetes-native provisioning layer for databases, buckets,
  clusters and VMs; and fleet already owns dispatch and
  placement.
- **The named revisit trigger:** the day the estate needs to
  provision and continuously reconcile *external* resources —
  cloud buckets, **DNS**, GitHub organisation objects, SaaS
  tenants — through the same GitOps flow as cluster state. That
  trigger points straight at [DNS management](dns-management.md).

## Drew's thinking — recorded 2026-11-23, explicitly unformed

The census verdict above is the researched record. Separately,
and **not as a decision**, Drew recorded his current thinking on
where Crossplane could still earn its place. This section is
that thinking, with its open edges kept open:

- **The niche: resources that don't have operators or proper
  Kubernetes projects.** Where no operator exists, a Crossplane
  provider may be the only Kubernetes-native way to manage the
  resource. His example: **managing Porkbun — the DNS registrar —
  from the infra folder**, the same `infra/dns/` corner that
  [The role of Terraform](terraform.md) and
  [DNS management](dns-management.md) describe.
- **Alternative shape (a): Nix handling APIs directly.** Nix
  packages and built-in Nix apps that call the APIs themselves,
  as an alternative to Crossplane providers — the estate already
  generates its app surface from the flake (see
  [NixOS flake apps](flake-apps.md)), so an API-managing app is a
  shape the estate knows how to build.
- **Alternative shape (b): the Cloudflare provider under a
  versions/uploads rule.** Crossplane's Cloudflare provider used
  under a rule that all objects are Cloudflare versions/uploads,
  with Cloudflare's own native tooling doing the management —
  Crossplane as the versioned declaration layer, not the
  operator.
- **The versioned-infra chain he sketched:** Kubernetes →
  Crossplane → Terraform applies the version; **rollback is
  pushing the old version number back**. Versioned declarations
  in the cluster, applied through the Terraform corner that
  already exists, reversed by re-pushing a number rather than by
  hand-editing state.

None of this is decided, scheduled, or ticketed in the record
this book was written from. It is recorded here because the
programme's thinking belongs in the book — labelled as thinking.

## The neighbouring field, as censused

The same census file covers the GitOps engines (ArgoCD and Flux —
FluxCD is already the planned cluster GitOps, per part 1), the
Terraform successors OpenTofu and Atlantis (see
[The role of Terraform](terraform.md)), and the platform
frameworks (Cozystack first among them). Its estate lens is
explicit that several famous projects compete with things this
estate already owns — fleet above all.

## Gaps

- The revisit trigger — external-resource reconciliation, with
  DNS named — has no owner and no tracking issue that the census
  found. It connects directly to the open DNS classification
  work (#902) in [DNS management](dns-management.md).

## Sources

Platform census §12 and supplements S7 and S12, 2026-11-23,
landed as PR #13959.
