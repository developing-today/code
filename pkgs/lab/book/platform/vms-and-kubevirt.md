# Minimal VMs and KubeVirt

The estate has a disciplined design for Nix-built minimal VMs —
and exactly one defined guest, which is not enabled. KubeVirt,
the Kubernetes way of running those VMs, is researched-only, and
**the bridge between the two — a Nix-built guest image designed
for KubeVirt — is absent as a design**: the census searched for
it and found none. Status: **planned** (registry and one guest);
KubeVirt **researched-only**. Read by the platform census of
2026-11-23 (landed as
`doc/2026-11-24T01-20-46Z_research_code-book-census-platform/`,
PR [#13959](https://github.com/developing-today/code/pull/13959)).

## What exists today

- **The guest registry:** `nixos/vm-guests/default.nix` — in its
  own header, "pure data: the flake-level record every guest must
  appear in before it may be defined on a host"
  ([#5772](https://github.com/developing-today/code/issues/5772),
  open).
- **The renderer decision** (2026-11-12, recorded on #5772):
  the libvirt class renders through **nixvirt** (fallback:
  flake-generated libvirt XML checked into the guest generation,
  with the gap named per guest); the ephemeral/agent class
  renders through **microvm.nix**, already a flake input, with
  one guest under `nixos/microvm/`.
- **Generation discipline:** a guest generation is exactly the
  triple of domain definition, disk image, and NixOS source
  revision. Data volumes attach to a generation and never live
  inside it, so rollback swaps the triple and leaves data
  untouched. Rationale: `doc/2026-11-12_vm-guests-generation-discipline.md`.
- **Actual guests: exactly one** — `prometheus` (microvm class,
  host `amd`, config `nixos/microvm/prometheus`, qemu, 2 vCPU,
  1024 MiB), recorded in the registry itself as
  **defined-not-enabled**: the microvm profile is still commented
  out in the laptop's host definition (see
  [The laptop NixOS config](the-laptop-nixos-config.md)). The
  mutable carve-out list and the full-drift waiver list both
  start — and remain — empty.
- **The minimal-VM mechanism of record** is the microvm.nix
  model: store-shared, one image per guest. The estate's
  from-scratch container work is the neighbouring discipline —
  [Containers and Docker images](containers-and-images.md).

## KubeVirt — the research record

From the CNCF census (landed in-repo 2026-11-23): KubeVirt
graduated from the CNCF on 2026-11-04, and is the standard way
people make VMs in Kubernetes now. **Cozystack's VM offering is
KubeVirt underneath.** Cozystack is the cluster's chosen
platform — settled by Drew on 2026-11-23 — so Cozystack is the
given frame for this chapter: KubeVirt is treated as Cozystack's
VM layer, and the estate's KubeVirt path arrives through the
planned Cozystack cluster (part 1's
[Talos and Cozystack](../cluster/talos-and-cozystack.md)), not
through hand-written KubeVirt configuration. Chosen is not
built: the cluster itself is still planned-state. The census searched
`doc/`, `nixos/`, `pkgs/` and `infra/` for kubevirt: the hits are
only the CNCF records and fleet source documents. There is no
KubeVirt configuration anywhere in the estate.

## Gaps

- Enabling even the seed guest is unproven.
- No KubeVirt guest-image design exists — the Nix-VM-to-KubeVirt
  bridge is unbuilt and undesigned.
- #5772's lifecycle proof is still owed.

## Sources

Platform census §10 and supplements S5 and S12, 2026-11-23,
landed as PR #13959.
