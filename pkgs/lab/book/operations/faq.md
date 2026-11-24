# FAQ

## Is there a running Kubernetes cluster in the basement?

No. As of the latest status record (2026-10-27), nothing in the
current cluster programme has been built. The basement machines
have no OS installed, by design. See
[The cluster programme](../cluster/the-cluster-programme.md).

## Didn't a Talos cluster run there before?

Yes — a v1.9-era test cluster (VIP 10.10.0.42, controllers
a1/a2/b1, workers a3/a4/b2–b6). It was torn down, confirmed by
Drew on 2026-10-26, and is history only. See
[Talos and Cozystack](../cluster/talos-and-cozystack.md).

## What is actually proven, then?

Three things carry the proven marking in this book: the TALM
per-node disk flow (#1812 — design plus a local container flow,
not bare metal); the first lab Cloudflare Tunnel (#6455 — live
since 2026-11-06, running on the laptop); and the record-level
facts about the machines themselves (every CPU boots, all RAM
works, the firmware pass on the named board). Everything else is
in progress, planned, or researched-only, and the chapters say
which.

## Why do the machines have no OS? Isn't that backwards?

It is the programme's central rule: everything bootstrappable
from config, no hand-built state that just existed forever. The
machines get an OS when the TALM bootstrap flow installs it
declaratively — see [Boot and auto-join](../cluster/boot-and-auto-join.md).

## Which switches will the cluster use?

The only 100GbE fabric in the research inventory is the Celestica
DX010 (four recorded, one missing fans/PSU). The full inventory —
21 devices — and the conclusions the research supports are in
[Switches](../network/switches.md). No switch is recorded as
racked or configured for the cluster yet.

## Is `pkgs/lab-ops-hub` the lab's operations hub?

No. It is a Cloudflare full-stack demo that borrowed the lab's
vocabulary — its database was seeded with three fictional
machines matching the basement-cluster sketch. It is not the
physical lab, and this book gives it a footnote, not a chapter.
See [The cluster programme](../cluster/the-cluster-programme.md).

## Has any ESP32 board been bought or run?

No board is recorded as bought, flashed, or run. The ESP32-S3 is
the verified reference part and the ESP32-P4 the named future
tier; the platform design is decision-ready and the display-tier
research is landed — all of it research and design, with two
software proofs and zero physical verification. See
[ESP32](../microcontrollers/esp32.md).

## Where are the prices for all this hardware?

Not in this book. No per-item prices exist in any source the
census read; the priced inventory is the job of the AliExpress
aggregation (#13910), and cost material belongs to the Project
Management book in any case.

## When will the cluster be live?

No record carries a projected date for any lab milestone — the
census's gap 16. The [High-level roadmap](../roadmap/high-level-roadmap.md)
records every lab timing entry as "no timing in record" rather
than inventing one.
