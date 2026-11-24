# The laptop NixOS config

The laptop is this estate's daily driver and its build and deploy
host, and its NixOS configuration lives in the same flake as
everything else. Status: **proven and in use** — it is the
machine this book's work runs on. This chapter records the host
as read by the platform census of 2026-11-23 (landed as
`doc/2026-11-24T01-20-46Z_research_code-book-census-platform/`,
PR [#13959](https://github.com/developing-today/code/pull/13959)).
The physical machine is in part 1's
[Hardware register](../site/hardware-register.md).

## What exists today

- **Host `amd`** in `nixos/hosts/default.nix`: a Framework
  13-inch with 7040-series AMD hardware, disks `nvme0n1` (4 TB)
  plus a tmpfs root, and users user, backup, muse, dot-ssh and
  root. Six wireless profiles are defined.
- **Profiles.** Alongside the desktop profile, the host enables
  service profiles: flatpak, id-mail, pm-stack, **lab-tunnel**,
  **sandbox-link-tunnel**, pm-apps, power-guard,
  openai-cred-watchdog, opencode-go-seating-guard,
  opencode-watchdog, keyring-auto-unlock, signal-audit,
  archer-slack, networking/dhcp-nat, and an nvidia-egpu
  specialisation. The two tunnel profiles are the lab's pipes to
  the outside — see
  [The pipes into Cloudflare and externally](pipes-to-cloudflare.md).
- **Two profiles are commented out, deliberately:** `microvm` and
  `server`. The commented-out microvm profile is why the one
  defined VM guest sits unused — see
  [Minimal VMs and KubeVirt](vms-and-kubevirt.md). These are
  non-states the record keeps visible, not omissions.
- **Per-host fragments** sit beside the host definition:
  `amd-rustdesk.nix`, `amd-sunshine.nix`, `amd-krdp.nix`,
  `amd-ipv4-preference.nix` (the IPv6 workaround),
  `amd-openclaw-secrets.nix`, and `amd-agent-rebuild.nix`
  (agent-triggered rebuilds of the amd config from main).
- **The rest of the host tree:** a second host, `nixos`
  (Framework 13, 12th-generation Intel, 2 TB NVMe), and a
  PC Engines APU hardware configuration exist in the same tree.
- **Machine inventory of record:** `doc/hardware/machines/amd.md`,
  collected 2026-10-29 — Ryzen 7 7840U, roughly 128 GB RAM,
  Samsung 990 PRO 4 TB.
- **Standing mechanics:** deploys and rebuilds run from the
  primary clone on the laptop; the orchestration sandbox never
  builds. CI runs in GitHub Actions — see
  [CI/CD: Nix and GitHub Actions](ci-cd.md).

## What the record plans

No host redesign is recorded. The disabled microvm and server
profiles are the named future switches: enabling microvm is the
step that would activate the defined VM guest.

## Gaps

None material for a book chapter — the census found this one of
the best-documented areas of the platform. The chapter notes the
commented-out profiles as deliberate non-states, per the census.

## Sources

Platform census §4 and supplements S10, 2026-11-23, landed as
PR #13959.
