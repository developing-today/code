# VM guests: renderer decision, generation discipline, carve-outs, and the deploy vm:<name> spec

Issue: #5772 (VM track of the deployment-architecture epic;
design: doc/2026-11-04_deployment-architecture-services-layer.md SS6).
Machine-readable companion: nixos/vm-guests/default.nix (guest
registry, carve-out list, waiver list), exposed as the flake
output vmGuests.

## 1. Renderer decision (step 1 of the #5772 plan)

**libvirt class: nixvirt is the renderer.** Domains, networks, and
pools are declared as Nix data and rendered by nixvirt, for these
reasons:

- One language end to end: the guest NixOS config, its disk image,
  and its domain definition all evaluate in the same flake, so a
  guest generation is one derivation graph, not Nix plus a second
  XML authoring workflow.
- Diff and review happen on Nix attrsets, not on generated XML
  blobs; a renderer change shows up as a data change in review.
- The design (SS6) already leans this way; the cost is one new
  flake input plus the module learning curve, paid once.

Fallback, deliberately narrow: where nixvirt cannot express a
domain feature, that one guest falls back to flake-generated
libvirt XML checked into its generation, and the guest entry in
nixos/vm-guests/default.nix names the gap. Hand-written virsh XML
outside the flake is never a renderer.

**Ephemeral / agent class: microvm.nix is confirmed.** It is
already a flake input, one guest (prometheus, qemu, 2 vCPU /
1 GiB, virtiofs /nix/store share) is already defined under
nixos/microvm/, and its store-shared, image-per-guest model fits
short-lived sandbox guests better than libvirt pets. The step-2
lifecycle proof below is the remaining chance to refute this; if
prometheus cannot be built, started, rolled back, and destroyed
from flake state, this decision reopens with that evidence.

## 2. Guest generation shape

A guest generation is exactly the triple:

    { domain definition, disk image (flake-built), NixOS source rev }

Data volumes attach to a generation; they never live inside one.
Rollback is redefining the guest from the previous triple; the
data volumes are untouched by construction. This is the shape
deploy vm:<name> (SS5) operates on.

## 3. Mutable carve-out list and the waiver rule (step 4)

The carve-out list lives in nixos/vm-guests/default.nix and
**starts empty**: no guest is exempt from generation discipline
today. A carve-out entry names a guest whose data may drift while
its OS definition still comes from the flake, with owner, reason,
and the data volumes the exemption covers (database hosts and the
lab clusters stateful nodes are the expected first candidates —
they are not pre-listed; an entry is added when the guest exists).

Anything that must drift in OS definition as well requires a
full-drift waiver: a named entry in the waivers list with owner,
reason, and an expiry, reviewed like a firewall rule. Mutable is
a waiver, never a default.

## 4. deploy vm:<name> integration spec (step 5)

Implementation rides the deploy tools later target classes (per
the issue); this is the contract it must implement:

- deploy vm:<name> — materialize the guests current generation
  from flake state alone (render the domain via the class
  renderer, build or fetch the disk image at the recorded rev),
  define it, and start it. Idempotent: re-running on an unchanged
  generation is a no-op that reports the running generation.
- deploy vm:<name> status — report current vs previous
  generation triples and guest state (running / shutoff /
  undefined).
- deploy vm:<name> rollback — redefine the guest from the
  previous generation triple; data volumes are not touched.
- deploy vm:<name> destroy — shut down and undefine the guest.
  Data volumes survive unless an explicit --purge-data flag is
  given; the flag is never implied.
- Generations are recorded per host under the guests state
  directory as {current, previous} triples in the shape of SS2,
  so status and rollback never guess.

## 5. What remains: the two lifecycle proofs (steps 2 and 3)

These need the amd host itself (a nixos-rebuild switch and real
guest starts on the laptop), so they are a dedicated host visit,
not part of the change that lands this doc and registry:

- Step 2 (microVM class): enable the microvm profile for amd
  deliberately (nixos/hosts/default.nix), switch, then build
  prometheus from a clean checkout, start it, roll it back,
  destroy it; record commands and outputs on #5772.
- Step 3 (libvirt class): add the nixvirt flake input, define one
  throwaway test guest end to end (define, start, rollback,
  destroy) from flake state alone, and record the proof on #5772.
  A real libvirt guest is defined only if a real need has
  appeared by then.

When both proofs are recorded, the acceptance in #5772 (one guest
of each class through its lifecycle from a clean checkout, the
carve-out list in the flake, deploy vm:<name> specced) is met and
the issue closes.
