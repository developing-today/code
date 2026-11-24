# basement cluster (skeleton)

Slice 1 of #14016: config home plus TALM bootstrap skeleton. Nothing here is applied to hardware. Every node file is an `EXAMPLE-` placeholder.

## Config home

Decision: cluster config lives at `infra/cluster/<cluster>/` in this repo, as recorded in `infra/cluster/README.md` (#1812). A separate repo is Drew's call only.

## Layout

- `Chart.yaml`, `charts/talm/`, `templates/`: the talm preset chart (cozystack preset).
- `values.yaml`: cluster-wide values. `endpoint` is empty on purpose; the chart refuses to render until it is set. `advertisedSubnets` is an RFC 5737 placeholder (`192.0.2.0/24`) until the #1810 shortlist lands.
- `nodes/<name>.yaml`: one file per node. Per drive class templates: `EXAMPLE-fast-990pro.yaml`, `EXAMPLE-mid-980.yaml`, `EXAMPLE-storage-hdd.yaml`. Real files come from `talm template -i` on a maintenance-mode node (#1810).
- `.talm-preset.lock`: pins `talosVersion: v1.14` and `kubernetesVersion: v1.36.5` from the cozystack preset. talm v0.36.0 (2026-10-20).
- `.gitignore`: excludes Tier A material (see below).

## Secrets

- Tier A (Talos PKI, `talosconfig`, `talm.key`, `secrets.yaml`) is never committed. It belongs in the #13948 broker or dual-store path, per PR #13992.
- Tier B config-shaped material may be SOPS-encrypted, using the recipients in `.sops.yaml`.
- No third secret path is introduced here.

## First-boot runbook (stub)

1. Pick the cluster endpoint and advertised subnet from the #1810 shortlist; set `endpoint` and `advertisedSubnets` in `values.yaml`.
2. Boot the node into maintenance mode. Hardware discovery and the storage layout are out of scope here: storage pool layout is #1813, and the Cozystack install design is #14017.
3. Generate the node file with `talm template -i`, then replace the matching `EXAMPLE-` file.
4. Render and review with `talm template -f nodes/<node>.yaml --offline`, then dry-run apply.

Hand-offs: real machines and BMCs go to #1086. The Cozystack install goes to #14017.

## Render proof

`talm template -f nodes/EXAMPLE-<class>.yaml --offline` exits 0 for each drive class when `endpoint` is set. With `endpoint` empty it fails loudly by design.
