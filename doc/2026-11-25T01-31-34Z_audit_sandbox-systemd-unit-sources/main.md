# Audit: committed sources of the sandbox systemd units — live-verified mapping

- Record created: 2026-11-25T01:31:34Z
- Issue: developing-today/code#3235 (Drew's 2026-10-28 directive: all system
  services + tools are committed to one repo or the other)
- Auditor: Felix (Muse lane, worker `felix`), running **on the sandbox seat
  itself** (`muse`, 100.107.76.35) — the live-sandbox access every prior pass
  on this issue lacked (tasks 8228, 12103, 13491, 15251, 16812, 18253, 19800).
- Method: `systemctl cat` + `sha256sum` against the live units on this seat;
  `git grep` over current `origin/main` of developing-today/code; working-tree
  checks of dezren39/hatch (this home directory) and dezren39/hatch-personal.
  Raw capture: `supplements/live-capture.txt`.

## Verdict per unit

| Unit | Live on this seat | Committed source | Verdict |
| --- | --- | --- | --- |
| `resource-sampler.service` | **Not installed** on the current seat incarnation (no unit file in any systemd path, not in `systemctl list-units`) | dezren39/hatch-personal `scripts/resource-sampler/` — unit, payload `resource-sampler.sh`, and `install-sampler-units.sh` | **PINNED** (repo side). Deployment gap only: see below. |
| `hatch-execd.service` (+ `hatch-execd.socket`) | Present in `/etc/systemd/system/`, loaded, inactive; file timestamps at epoch (baked into the platform runtime-cell image, not hand-installed) | **None** in code, hatch, or hatch-personal | **UNPINNED** — remainder lives in dezren39/hatch#14 |
| `hatch-ca-trust.service` (+ `hatch-ca-trust.path`) | Present in `/etc/systemd/system/`, enabled; `.path` active/waiting, oneshot service inactive/dead (fires on anchor change) | **None** in code, hatch, or hatch-personal | **UNPINNED** (unit files only; payload is stock binaries) — remainder lives in dezren39/hatch#14 |

## Detail

### resource-sampler — PINNED; not currently deployed on this seat

- Committed in dezren39/hatch-personal, `scripts/resource-sampler/`:
  - `resource-sampler.service` — sha256 `23116a2b679601e93764af5067b6160736039384c37636541a05ecc51f80ea2a`
  - `resource-sampler.sh` (payload; the unit's `ExecStart` points at it in the
    hatch-personal checkout) — sha256 `adcadcc741e840d78a5cd09c959cfcc6be1a13473692251788e8ebcfc8328b4f`
  - `install-sampler-units.sh` (idempotent installer) — sha256 `f5d5b0692b8f4835167807827afde7debcb88433cf7fd7c3443ed72b79940389`
- The 2026-11-14 pass (task 15251) verified the then-live unit byte-identical
  to this committed copy.
- New finding this pass: the current seat incarnation (rebuilt 2026-11-21)
  has **no** `resource-sampler` unit installed at all, even though the
  installer header says `~/tools/init.sh` calls it on every boot where the
  sampler is not active. Either the init hook no longer calls it or the call
  is failing silently on this incarnation. That is a deployment/restore gap,
  not a source-pinning gap; it is recorded here and on #3235 for the hatch
  side to pick up (the fix, if any, belongs to the hatch repos' init path).

### hatch-execd — UNPINNED

- Live unit: `/etc/systemd/system/hatch-execd.service`
  sha256 `41ab8434f10374674c754be02e7fe95591880c3c7ecae42f99463212fa0f93ac`;
  companion socket `/etc/systemd/system/hatch-execd.socket`
  sha256 `c0a39e26e6f9d0dc82c39bdfb82de9286f4fc133609e2acb329387a228cc3c33`.
  Verbatim copies: `supplements/hatch-execd.service`,
  `supplements/hatch-execd.socket`.
- `ExecStart=/opt/hatch/runtime-cell/run-execd.sh --socket /run/hatch/execd/execd.sock`,
  gated by `ExecStartPre` on `/opt/hatch/runtime-cell/runtime-cell.ready`.
  The payload script `run-execd.sh` is **not present** on this seat (the
  `/opt/hatch/runtime-cell/` directory holds the cell tooling but no
  `run-execd.sh`), and the unit is inactive — consistent with an image-baked
  unit whose cell payload is only staged when the runtime cell is provisioned.
- No committed copy of the unit, the socket, or the payload exists in
  developing-today/code, dezren39/hatch, or dezren39/hatch-personal
  (`git grep` finds the name only in audit/issue-map documents).

### hatch-ca-trust — UNPINNED (unit files only)

- Live units: `/etc/systemd/system/hatch-ca-trust.service`
  sha256 `3666a09a9146d03b71705b778242d930d80c8db881aca575f6593d2b9fd0032c`,
  and `/etc/systemd/system/hatch-ca-trust.path`
  sha256 `71c91eae35a2518a52cb24ed0bbd553e261ce3327e2c9442a930876bfd8234e1`.
  Verbatim copies: `supplements/hatch-ca-trust.service`,
  `supplements/hatch-ca-trust.path`. (The `.path` unit is new evidence — prior
  passes recorded only the `.service`.)
- Payload is stock system binaries only (`mkdir`, `ln`, `update-ca-certificates`)
  over the host-published anchor `/run/hatch/cell-anchors/hatch-egress-ca.pem`
  (present on this seat). Nothing custom to pin except the two unit files.
- No committed copy exists in code, hatch, or hatch-personal.

## Where the remainder lives

The pinning commits for `hatch-execd` and `hatch-ca-trust` belong to the hatch
repos (public `hatch` vs private `hatch-personal`, by content) and are already
filed, with this audit's predecessor evidence, as **dezren39/hatch#14**
("Pin committed sources for sandbox systemd units: hatch-execd +
hatch-ca-trust (resource-sampler verified pinned) — from code#3235"), still
open at this pass. The verbatim live unit files in `supplements/` are the
exact bytes that issue needs to commit. #3235 stays open as the 2026-10-28
directive's tracking record until hatch#14 lands the two pins; this record is
the live-verified mapping its acceptance asks for.

<!-- workflow:state -->

{
 "version": 1,
 "id": "2026-11-25T01-31-34Z_audit_sandbox-systemd-unit-sources",
 "created_at": "2026-11-25T01:31:34Z",
 "stage": "delivery",
 "status": "complete",
 "transcript": {
  "path": "supplements/live-capture.txt",
  "sha256": "75b45308a862306b0fe6c3b98c6caa36c068577d8328ef2fcb9fd9a47dcd5218"
 },
 "concepts": [
  {
   "id": "C001",
   "title": "Live-verified committed-source mapping for resource-sampler, hatch-execd, hatch-ca-trust (code#3235)",
   "path": "main.md",
   "status": "complete",
   "source_lines": [
    [
     1,
     60
    ]
   ]
  }
 ],
 "dependencies": [],
 "issues": [
  "https://github.com/developing-today/code/issues/3235"
 ],
 "prs": [],
 "tasks": [],
 "events": [
  {
   "at": "2026-11-25T01:31:34Z",
   "stage": "delivery",
   "status": "complete",
   "reason": "Audit run from the sandbox seat itself; live systemctl/sha256 capture recorded as the transcript and the per-unit mapping written into main.md."
  }
 ]
}

<!-- /workflow:state -->
