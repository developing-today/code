# Automation and automated scaling

When does this estate grow and shrink its own capacity without a
human asking? The answer differs by layer: builds already scale
by dispatch, edge workers scale on their platform, servers do
not scale automatically at all, and cluster autoscaling is
research waiting for a cluster. Status: **in use** for builds
and edge workers; **researched/planned** for servers and the
cluster. This chapter is a member of the between-lab-and-fleet
trio, with [DNS management](dns-management.md) and
[The pipes into Cloudflare and externally](pipes-to-cloudflare.md).
Read by the platform census of 2026-11-23 (landed as
`doc/2026-11-24T01-20-46Z_research_code-book-census-platform/`,
PR [#13959](https://github.com/developing-today/code/pull/13959)).

## Builds — in use

Fleet placement plus the queue broker scale build capacity by
dispatch: jobs spill across GitHub-hosted runners, Blacksmith,
Codespaces, and Actions-exec ephemeral sessions that are
created, handed off, renewed and closed programmatically (see
[GitHub Actions](github-actions.md) and
[Build-server alternatives](build-servers.md)). That *is*
automated scale-up and scale-down of build capacity, with the
queue as the scaler. The laptop joins as a placement surface,
and a lab-integration draft (P9) would add cluster nodes as
overflow — **planned and unratified** (lab census §2.4).

The fleet side of this story — placement and the compile
coordinator — is carried by the fleet book's placement and
compile-coordinator material
([the fleet book](https://developing-today.github.io/code/fleet/));
the Project Management programme owns those fleet sections, and
this chapter is the lab-side summary that points at them.

## Containers and edge — in use, by the platform

The Cloudflare Workers estate (28 wrangler projects) scales on
Cloudflare's platform. The estate's own automation there is the
deployment and promotion workflows of
[CI/CD: Nix and GitHub Actions](ci-cd.md) — not scaling logic.

## Servers — planned

NixOS hosts are rebuilt and replaced from configuration; clan's
`machines install|update` model is the researched shape for
doing that across a fleet (see
[clan.lol and machine fleets](clan.md)), and the TALM flow gives
the cluster the same shape (part 1's
[Talos and Cozystack](../cluster/talos-and-cozystack.md)). No
automated provisioning of new physical or VM servers runs today;
the one defined VM guest is not even enabled (see
[Minimal VMs and KubeVirt](vms-and-kubevirt.md)).

## Cluster autoscaling — researched

The CNCF runtimes census rates **KEDA** (CNCF Graduated;
event-driven autoscaling including scale-to-zero) "the highest
value-per-effort project in this file after KubeVirt" — a
candidate for when a cluster exists. No HPA, KEDA or Karpenter
configuration exists anywhere in the estate, because no cluster
exists yet. One neighbouring design must not be conflated with
this: the ID currency-autoscaling design (#5825) is
*application-level* scaling design — a stateless front end and
its scaling triggers — not infrastructure autoscaling.

## Gaps

- No written automation and scaling strategy ties these layers
  together; this trio chapter is that missing synthesis, written
  from the records that do exist.
- Cluster scaling is entirely downstream of the cluster existing.

## Sources

Platform census §18 and supplements S12, 2026-11-23, landed as
PR #13959.
