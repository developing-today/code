# Lab Book

The lab is the physical side of the project: the machines in the
basement of a house in Osseo, Wisconsin, the switches that will join
them, the Kubernetes cluster they are meant to become, and the
ESP32-class microcontrollers at the small end of the same programme.
This book is the record of that lab — what is physically there, what
is planned for it, what has actually been proven, and what is still
only research.

The book's primary source is the lab census gathered on
2026-11-23/24 under the programme tracker
[#13905](https://github.com/developing-today/code/issues/13905) and
landed as a dated research record in the code repository
(`doc/2026-11-24T00-58-48Z_research_code-book-census-lab/`, PR
[#13937](https://github.com/developing-today/code/pull/13937)).

## The honesty note — read this first

This book marks every claim with the status the record supports:

- **Proven** — a record shows it working or verified.
- **In progress** — work or a live partial state exists.
- **Planned** — ticketed or designed, not built.
- **Researched-only** — documents or research exist; there has been
  no build and no physical verification.

Those markings are load-bearing, and this book keeps them exactly as
honest as the census kept them. The headline, stated plainly: as of
the latest status record (2026-10-27), nothing in the current
cluster programme has been built yet. The basement machines have no
operating system installed, by design — no hand-built state that
just exists forever. An earlier Talos test cluster from the v1.9 era
was torn down, confirmed by Drew on 2026-10-26, and is history in
this book, never present tense. The one proven cluster piece is the
TALM per-node disk flow
([#1812](https://github.com/developing-today/code/issues/1812)). The
one live network artefact of the lab programme is a Cloudflare
Tunnel running on the laptop, not on any cluster machine. A book
that is mostly to-do is the correct book for this lab today; the
gap list in [Gaps and open questions](plans/gaps-and-open-questions.md)
is a deliverable, not an embarrassment.

## How to read this book

- **The site** — [The Osseo site](site/the-osseo-site.md) describes
  the physical location, power, racking state and home network as
  recorded; [Hardware register](site/hardware-register.md) records
  the hardware itself and the per-machine inventory that does not
  exist yet.
- **The cluster** — [The cluster programme](cluster/the-cluster-programme.md)
  gives the programme structure and current state;
  [Talos and Cozystack](cluster/talos-and-cozystack.md),
  [Storage: LINSTOR and DRBD](cluster/storage-linstor-and-drbd.md)
  and [Boot and auto-join](cluster/boot-and-auto-join.md) cover the
  three design areas in turn.
- **The network** — [Switches](network/switches.md) folds in the
  dedicated switching research: 21 devices, enriched in April 2026.
- **Platform and toolchain** — the part added from the platform
  census of 2026-11-23 (landed as
  `doc/2026-11-24T01-20-46Z_research_code-book-census-platform/`,
  PR [#13959](https://github.com/developing-today/code/pull/13959)):
  [GitHub Actions](platform/github-actions.md),
  [Nix and the NixOS flake](platform/nix-and-the-flake.md),
  [NixOS flake apps](platform/flake-apps.md),
  [The laptop NixOS config](platform/the-laptop-nixos-config.md),
  [CI/CD: Nix and GitHub Actions](platform/ci-cd.md),
  [Making Nix packages and apps](platform/making-nix-packages-and-apps.md),
  [Containers and Docker images](platform/containers-and-images.md),
  [DNS management](platform/dns-management.md),
  [The Nix cache worker and its alternatives](platform/nix-caches.md)
  and
  [The pipes into Cloudflare and externally](platform/pipes-to-cloudflare.md),
  with shorter chapters on dev shells, clan, VMs and KubeVirt,
  Terraform, Crossplane, next-gen Nix tooling, build servers,
  artifact repositories, automation and scaling, and the
  RustFS-versus-SeaweedFS object-storage question.
- **Microcontrollers** — [ESP32](microcontrollers/esp32.md) covers
  the ESP32-S3 reference part, the ESP32-P4 tier, and the Tanmatsu.
- **Roadmap and plans** — the
  [High-level roadmap](roadmap/high-level-roadmap.md) carries this
  book's share of the project's merged priority list, and
  [Gaps and open questions](plans/gaps-and-open-questions.md) is the
  census's honest to-do record.
- **Keeping the book honest** —
  [Keeping this book current](development/keeping-this-book-current.md)
  states the sync rule, and
  [Deploying this book](operations/deploying-this-book.md) describes
  how the book is built and published.

The platform and toolchain part above is that second part: it
landed under
[#13905](https://github.com/developing-today/code/issues/13905)
from the platform census of 2026-11-23, with the same honesty
markings as the rest of the book — including where the record is
thin, planned, or absent, which its short chapters say plainly.

## Sibling books and records

- The Code Book — the whole-project book:
  [https://developing-today.github.io/code/code/](https://developing-today.github.io/code/code/)
- The fleet book:
  [https://developing-today.github.io/code/fleet/](https://developing-today.github.io/code/fleet/)
- Programme tracker:
  [#13905 — Code Book + Lab Book programme](https://github.com/developing-today/code/issues/13905)
- The priced hardware inventory this book lacks is the job of
  [#13910 — aggregate AliExpress purchase history into a hardware inventory](https://github.com/developing-today/code/issues/13910).

One boundary, stated once: this book contains no cost material.
Costs belong to the Project Management book.
