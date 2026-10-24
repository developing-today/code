# Research notes (2026-10-11)

Notes gathered while answering the design questions. Copied from the working folders `/tmp/research-cf`, `/tmp/research-relay` and `/tmp/research-disc`. Crate tarballs are not copied; fetch them from crates.io if needed.

| Folder | What it holds | Source |
| --- | --- | --- |
| `cf/` | Notes on iroh 1.3.0 crates (iroh, iroh-base, iroh-relay) and Cloudflare relay options | `notes.md` |
| `relay/` | Relay hosting research: REPORT, container tags and releases, and the iroh-tickets and hetzner-app records | `REPORT.md` and JSON records |
| `disc/` | Discovery research: DHT, pkarr and mDNS findings | `findings.md`, `report.md` |

Crate tarballs used for reading source (not copied here):

- `iroh-1.3.0`, `iroh-base-1.3.0`, `iroh-relay-1.3.0` from `static.crates.io`.
- `p2panda-core`, `p2panda-encryption`, `p2panda-spaces` 0.7.1 (sources extracted to `/tmp/research/q/`).
- `p2panda-store`, `p2panda-stream`, `p2panda-sync` 0.7.1 (sources extracted to `/tmp/research/p2p-*`).
- `beekem` 0.4.0, published 2026-09-25.

Each notes file records the commands and the URLs it used.
