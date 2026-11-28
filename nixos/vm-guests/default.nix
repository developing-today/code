# VM guest registry, generation shape, and mutable carve-out list.
# Issue #5772 (VM track of the deployment-architecture epic,
# doc/2026-11-04_deployment-architecture-services-layer.md SS6).
#
# This file is pure data: it is the flake-level record every guest
# must appear in before it may be defined on a host. It changes no
# host configuration by itself. Full rationale, the renderer
# decision record, and the deploy vm:<name> spec live in
# doc/2026-11-12_vm-guests-generation-discipline.md.
{
  # Renderer decision (recorded on issue #5772, 2026-11-12):
  # - libvirt class: nixvirt renders domains, networks, and pools
  #   from Nix data. Fallback, only where nixvirt cannot express a
  #   domain: flake-generated libvirt XML checked into the guest
  #   generation, with the gap named in the guest entry.
  # - ephemeral / agent class: microvm.nix (already a flake input,
  #   flake.nix microvm input; one guest defined under
  #   nixos/microvm/). Its store-shared, image-per-guest model fits
  #   short-lived guests better than libvirt pets.
  renderer = {
    libvirt = "nixvirt";
    libvirtFallback = "flake-generated-libvirt-xml";
    ephemeral = "microvm.nix";
  };

  # A guest generation is exactly this triple. Data volumes attach
  # to a generation; they never live inside it, so a rollback swaps
  # the triple and leaves data untouched.
  generationShape = [
    "domainDefinition" # nixvirt attrset, or generated libvirt XML
    "diskImage" # flake-built image (nixos.qcow2 / microvm runner)
    "nixosSourceRev" # flake rev the image was built from
  ];

  # Every guest known to the flake, by class. An entry here is a
  # definition record, not a running guest: "status" says how far
  # the guest has actually gotten, honestly.
  guests = {
    prometheus = {
      class = "microvm";
      host = "amd";
      config = "nixos/microvm/prometheus";
      hypervisor = "qemu";
      vcpu = 2;
      memMiB = 1024;
      # Defined in nixos/microvm/default.nix, but the microvm
      # profile is still commented out for amd in
      # nixos/hosts/default.nix, so the guest is not yet under
      # generation discipline on any host. Enabling the profile is
      # a host change and belongs to the lifecycle-proof visit
      # specced in the doc, not to this registry entry.
      status = "defined-not-enabled";
    };
  };

  # Mutable carve-out list. Starts EMPTY (issue #5772): no guest is
  # exempt from generation discipline today.
  #
  # A carve-out entry names a guest whose DATA may drift while its
  # OS definition still comes from the flake:
  #   { guest, owner, reason, dataVolumes = [ ... ]; }
  # Data-only drift is the whole exemption. A guest that must drift
  # in its OS definition as well needs a full-drift waiver instead:
  #   { guest, owner, reason, expires; }
  # recorded in "waivers" below, reviewed like a firewall rule, and
  # time-boxed by its expires date. Mutable is a waiver, never a
  # default (design doc SS6).
  carveOuts = [ ];

  # Full-drift waivers (OS definition may drift). Starts empty for
  # the same reason: nothing has been granted one.
  waivers = [ ];

  waiverRule = "full drift requires a named waiver with owner + reason + expiry, recorded in this file";
}
