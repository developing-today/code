# DNS management

DNS is one subject with two live systems, and this is the book's
one DNS chapter: the registrar layer that owns the domains, and
the Cloudflare layer that serves the estate's names. Status:
**in use, split across two systems**, with classification and
backup work **planned**. Read by the platform census of
2026-11-23 (landed as
`doc/2026-11-24T01-20-46Z_research_code-book-census-platform/`,
PR [#13959](https://github.com/developing-today/code/pull/13959)).
This chapter is also a member of the between-lab-and-fleet trio;
its siblings are
[The pipes into Cloudflare and externally](pipes-to-cloudflare.md)
and [Automation and automated scaling](automation-and-scaling.md).

## The registrar layer — Porkbun, Terraform, Nix

- Domains are registered at **Porkbun** and their records are
  managed from `infra/dns/` — the estate's one Terraform corner,
  told in full in [The role of Terraform](terraform.md). The
  short form: `dns_config.yaml` is **generated from Nix**
  (`infra/dns/dns_config/default.nix`), and Terraform applies it
  as `porkbun_dns_record` resources.
- `infra/dns/domains.txt` lists **39 domains** — security.cab,
  developing.today, 01b.us, lair.cloud, the 1110x and 0x111
  families, and the rest of the register.
- **A discrepancy the census flagged and did not reconcile:** the
  standing domain register's first pass (completed 2026-10-25)
  is described as covering **40 domains**. The register
  description and `domains.txt` disagree by one; one of them is
  stale. This book carries the flag, not a guess.
- The recurring machinery — DNS backup job, renewal tracking and
  a dashboard view of the domain register — is
  [#902](https://github.com/developing-today/code/issues/902),
  open. (Its renewal element touches cost tracking; costs belong
  to the Project Management book and are cited here for existence
  only.)

## The Cloudflare layer — the service zone

- **`security.cab` is the working service zone.** Its names of
  record: `lab.security.cab` (a CNAME to the tunnel's
  `cfargotunnel.com` address, proxied — recorded in
  `nixos/services/lab-tunnel/default.nix`),
  `dispatch.security.cab`, `pm.security.cab`,
  `mail.security.cab` (plus the id-mail route names in
  `nixos/services/id-mail/worker/config.json`), and
  `sandbox-link.security.cab`. What those names front is
  [The pipes into Cloudflare and externally](pipes-to-cloudflare.md).
- Worker custom domains and routes are declared per project in
  the **28 wrangler configuration files** in the tree.

## Tooling direction

`pkgs/cf` packages Cloudflare's **`cf` CLI** — Wrangler's
successor, in open beta since 2026-09-28, packaged at
1.0.0-beta.20 with `buildNpmPackage` and a vendored lockfile
because cf is not in nixpkgs. The standing direction (Drew,
2026-10-20, recorded in the package) is to **prefer `cf` for
Cloudflare work**; `pkgs/wrangler` stays for existing
wrangler.jsonc projects until they migrate. Helper wrappers
(`pkgs/cf/wrangler-authed.py`, `wrangler-secret.py`) bridge
authentication in the meantime.

## A possible future shape

Drew's recorded thinking (2026-11-23, explicitly unformed) names
Porkbun as the example resource Crossplane could earn its place
managing — resources without operators or proper Kubernetes
projects, managed from the infra folder. See
[Crossplane and alternatives](crossplane-and-alternatives.md).
Nothing about today's two systems changes under that thinking.

## Rulings and gaps

- **Ruling of record** (`doc/living/turso-cloudflare-integration.md`,
  item C6): Porkbun stays the deliberate registrar layer;
  **per-domain classification is still owed**, and the
  classification checkbox in that document is still open.
- **Cloudflare Access is absent as a documented system.** The
  census searched for a dedicated Access policy or configuration
  record and found none — the record shows tunnels and
  application-level authentication instead (the dispatch
  gateway's expected 401, for one). The book flags this as a gap;
  it does not assert Access is unused.
- The `infra/dns` TODO list (alias-as-CNAME handling,
  multi-config refactor, per-section applies) is unowned.

## Cross-links

- The physical network this DNS names: part 1's
  [Switches](../network/switches.md) and
  [The cluster programme](../cluster/the-cluster-programme.md).
- A fleet-side cross-link to this chapter is owed from the
  Project Management programme's fleet pass
  ([#13900](https://github.com/developing-today/code/issues/13900));
  until that pass lands, this chapter is the DNS record both
  books point back to.

## Sources

Platform census §§11 and 13 and supplements S7, 2026-11-23,
landed as PR #13959.
