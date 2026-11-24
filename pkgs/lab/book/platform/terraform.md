# The role of Terraform

Terraform's role in this estate is small, exact, and easy to get
wrong: it is **in use in exactly one corner — DNS — and absent
everywhere else**. The natural assumption that it is simply
absent is wrong, and the correction is this chapter. Status:
**in use** (`infra/dns/` only). Read by the platform census of
2026-11-23 (landed as
`doc/2026-11-24T01-20-46Z_research_code-book-census-platform/`,
PR [#13959](https://github.com/developing-today/code/pull/13959)).
The DNS system it serves is [DNS management](dns-management.md).

## What exists today

- **Where it lives:** `infra/dns/` only. Its `main.tf` uses a
  local backend and two providers — `cullenmcdermott/porkbun`
  0.3.0 and `carlpett/sops` 1.4.1, with credentials read from a
  sops file. It declares one resource type,
  `porkbun_dns_record`, driven by `for_each` over
  `dns_config.yaml`.
- **That YAML is generated from Nix**
  (`infra/dns/dns_config/default.nix`, rendered by
  `generate_dns_config.sh`) — Terraform here is the apply tool
  for a Nix-declared configuration, not the source of truth.
- **State is manual and known-awkward:** the encrypted state is
  committed (`terraform.tfstate.enc`), and shell wrappers provide
  plan, apply, apply-skip-plan, init, load, save — plus cleanup
  scripts (`cleanup_terraform.sh`,
  `cleanup_terraform_tfstate.sh`) whose existence says the state
  juggling is handled by hand. `infra/dns/TODO` records open
  wishes: alias-as-CNAME handling, a multi-config refactor,
  per-section applies.
- **The search evidence for "everywhere else":** a repo-wide
  search for terraform returns 44 files; after `infra/dns/`, they
  are documentation mentions, vendored agent skills, the CNCF
  records, and `.gitignore`/dependabot noise. No other `.tf`
  configuration exists, and no Terraform appears in `pkgs/`,
  `lib/`, `nixos/`, or any workflow.

## Context the record gives

The cluster programme's hard rule is "no HashiCorp anything"
(part 1's cluster chapters). Terraform's survival is as a
registrar-DNS tool that predates and coexists with that rule —
not as platform infrastructure-as-code. The CNCF record covers
the successors as research: OpenTofu (Linux Foundation; CNCF
Sandbox, April 2025) and Atlantis (CNCF Sandbox) — and
[Crossplane and alternatives](crossplane-and-alternatives.md)
notes Crossplane also ships a Terraform provider.

## A sketched future role

Drew's recorded thinking (2026-11-23, unformed — see
[Crossplane and alternatives](crossplane-and-alternatives.md))
gives this Terraform corner a possible second job: in the chain
Kubernetes → Crossplane → Terraform, Terraform applies the
versioned declaration, and rollback is pushing the old version
number back. That is a sketch, not a plan; the corner's actual
role today is the DNS one above.

## Gaps

- No record proposes migrating `infra/dns` to OpenTofu. That is a
  decision gap, currently unowned.

## Sources

Platform census §11 and supplements S7, 2026-11-23, landed as
PR #13959.
