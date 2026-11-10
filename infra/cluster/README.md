# Cluster configuration — TALM/Talos per-node disk config for non-identical drives

The worked answer and runbook for [issue #1812](https://github.com/developing-today/code/issues/1812)
(child of the TALM + Talos bootstrap epic [#1086](https://github.com/developing-today/code/issues/1086)):
**how do you declaratively configure, in Talos/TALM, a bunch of hard drives
that are all different per machine?**

Status: design answer + runbook, verified against current TALM/Talos docs and
the fleet triage record on issue #1812 (triage task 7746, 2026-11-03). The
local container flow below is the proven shape; the **bare-metal proof is
gated on the hardware inventory, issue #1810**, and is out of scope here.
Everything stays bootstrappable/declarative; no HashiCorp tooling anywhere
(hard constraint from the issue).

## The answer in one paragraph

Do not try to describe every node's disks in one shared template. Keep **one
TALM node file per node** in git (`infra/cluster/<cluster>/nodes/<hostname>.yaml`).
TALM harvests each node's real hardware into that file as comments, and
everything you write below the file's modeline is applied as a **strategic
merge patch** over the shared cluster template at `talm apply` time. The
cluster template is identical for every machine; only the node files differ,
and each node file pins itself to exactly one node. Inside the node file,
match disks by **identity, not description**: the boot disk with
`machine.install.diskSelector` on `serial`/`wwid`, data volumes with
`UserVolumeConfig` CEL selectors carrying the `!system_disk` guard.

## Mechanism: TALM node files

1. Boot the node in Talos maintenance mode and run `talm template -i`
   against it. TALM writes `nodes/<hostname>.yaml` containing:
   - a modeline — `# talm: nodes=[...], endpoints=[...]`;
   - the **discovered hardware as comments** — every disk's serial, wwid,
     model, size, and transport, exactly as the machine reported it;
   - a filled `machine.install.disk` from the discovery.
2. Anything written **below the modeline** in that node file is a strategic
   merge patch over the rendered cluster template on `talm apply`. Fields
   outside networking — `machine.install.disk`,
   `machine.install.diskSelector`, extra etcd args — merge as expected.
   This node-file body is the declarative per-node disk hook.
3. A non-empty body pins the file to exactly one node: `talm apply` refuses
   a multi-node modeline with a body. For per-node disk config that pinning
   is a safety property, not a limitation — a disk patch can never leak
   onto the wrong machine.

Worked example: [`examples/node-file.yaml`](examples/node-file.yaml).

## Disk matching rules for non-identical drives

- **System/boot disk** — `machine.install.diskSelector` with **`serial` or
  `wwid`**, never `size`/`model` alone. Serial/wwid are unique per physical
  disk; `size: '>= 1TB'` or `model: 'WDC*'` can silently match a _different_
  disk when a node is reinstalled after a drive swap, and the install then
  wipes the wrong device.
- **Data volumes (Talos ≥ v1.10)** — `UserVolumeConfig.spec.provisioning.diskSelector.match`
  is a CEL expression over `disk.{serial,wwid,model,size,transport,symlinks}`.
  Always include the **`!system_disk`** guard, so no data volume can ever
  land on the boot NVMe even if the rest of the selector is loose. Prefer an
  exact serial/wwid match; a transport/size match is acceptable only with
  `!system_disk` in place and a `--dry-run` check before applying.
  `machine.disks[]` is deprecated in v1.10 — do not build on it.
- **Selectors are evaluated at install/provisioning time only.** A volume
  config is applied while the volume is unprovisioned; changing a selector
  later does not move an existing volume. That is why disk replacement
  (below) is a reinstall drill, not an in-place edit.

Worked example: [`examples/user-volume.yaml`](examples/user-volume.yaml).

## Boot-disk layout (frozen host image)

Talos itself is a small, frozen, read-only host image installed on each
machine's NVMe boot drive. The rest of the boot NVMe is headroom: minor logs
and wear-leveling slack across the consumer SSDs (990 Pro class). Nothing
about the layout is per-node configuration — the _identity_ of the boot
drive is the only per-node fact, and it lives in the node file's
`diskSelector` (above), nowhere else.

## Where the harvested configs live

`infra/cluster/<cluster>/` in this repo — this directory. Node files change
with PR review, sit next to this runbook and the
[basement-cluster roadmap](../../planning/roadmaps/basement-cluster.md), and
need no second repo with its own access model. (`infra/talos/01b.us/` is the
torn-down v1.9-era test cluster — historical reference only.) If a separate
cluster-config repo is ever preferred, that is a Drew decision; nothing in
the mechanism depends on it.

## Runbook

### Adding a node

1. Rack the machine, boot it from the Talos image into maintenance mode.
2. From the cluster working directory: `talm template -i -n <node-ip>`
   (inquire) to harvest the node's hardware into
   `nodes/<hostname>.yaml`; check the harvested comments against the
   physical drives you expect (serials/wwids present, counts match).
3. Edit **only the disk fields** in the node file: set
   `machine.install.diskSelector` to the boot NVMe's serial, and add any
   `UserVolumeConfig` documents for the node's data drives (exact
   serial/wwid + `!system_disk`).
4. `talm apply -n <node-ip> --dry-run` and read the diff: confirm the
   install disk resolves to the intended device and nothing else changed.
5. `talm apply -n <node-ip>`, then commit the node file (PR, like any other
   config change). The node is reproducible from git from this moment.

### Replacing a disk

Boot disk, replaced with a **different model/size** (the design question's
hard case):

1. Harvest the new drive: boot the node to maintenance mode and run
   `talm template -I` to refresh the node file's harvested comments.
2. Edit `nodes/<node>.yaml`: put the **new drive's serial/wwid** in
   `machine.install.diskSelector`. (If the selector had been size-based,
   this is the moment it would have silently matched the wrong disk —
   which is why this runbook's configs are identity-based.)
3. `talm apply --dry-run`: confirm the selector resolves to exactly the new
   device before anything is written.
4. Reinstall + re-apply — this is the wipe/rebuild drill below; a boot-disk
   swap is a rebuild of that node from config, by design.

Data disk: edit the volume's CEL selector to the new drive's serial/wwid,
delete/unprovision the old volume, re-apply. The selector alone never moves
a provisioned volume (install-time evaluation, above).

### Wiping / rebuilding a node from config only

1. Boot the node from the Talos image (maintenance mode).
2. `talm template -I -n <node-ip>` to refresh harvested comments; diff them
   against the committed node file — hardware drift (a swapped drive) shows
   up here, before any destructive step.
3. Reconcile the node file if hardware changed (see replacing a disk).
4. `talm apply -n <node-ip> --dry-run`, verify the install disk and volume
   selectors resolve to the intended devices, then `talm apply -n <node-ip>`.
5. The node rejoins the cluster from git alone: node file + cluster
   template + this runbook are the entire input. This drill is also epic
   #1086's rebuild acceptance — run it on the first bare-metal nodes once
   issue #1810 unblocks them.

## Local container-flow proof

The flow is provable with no hardware, and that local proof is the one Drew
already has partially working. Recorded flow (commands as run against a
local Talos maintenance-mode container):

```sh
# 1. Start a Talos node in maintenance mode locally (container), then:
talm template -i -n <node-addr>        # harvest -> nodes/<hostname>.yaml
# 2. Hand-edit only the disk fields in the node file (diskSelector by serial)
talm apply -n <node-addr> --dry-run    # diff shows install disk + patch only
talm apply -n <node-addr>              # apply
```

Observed (fleet record: Drew's local container runs, and the design
verification in triage task 7746 on issue #1812, 2026-11-03):

- `talm template -i` produces the node file with the modeline, the
  discovered disks as comments (serial/wwid/model/size/transport), and a
  filled `machine.install.disk`.
- Edits below the modeline merge as a strategic patch: changing
  `machine.install.disk` / `diskSelector` in the node file changes only
  that node's rendered config; the shared template is untouched.
- A node file whose body is present refuses multi-node application —
  the one-node pinning holds.
- `--dry-run` before apply shows the resolved install disk, which is the
  check the replace-a-disk runbook depends on.

What this does **not** prove: real drive-swap behavior, installer behavior
on the actual heterogeneous bare-metal inventory, and the rebuild drill on
physical nodes. Those stay gated on issue #1810 (hardware inventory) under
epic #1086.

## Sources

- TALM — <https://github.com/cozystack/talm> (node files, `template -i`
  hardware inquire, strategic-merge node bodies, `apply --dry-run`).
- Talos configuration reference — <https://www.talos.dev/latest/reference/configuration/>
  (`machine.install.diskSelector` matchers; `UserVolumeConfig` block-volume
  documents and their CEL `diskSelector.match`, v1.10+).
- Design record: issue #1812 and its triage (task 7746, 2026-11-03), which
  verified the mechanism and the disk-matching rules against the TALM and
  Talos documentation above.
