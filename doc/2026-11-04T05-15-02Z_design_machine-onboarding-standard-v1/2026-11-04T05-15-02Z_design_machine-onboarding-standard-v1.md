# Design: machine onboarding standard and registry shape v1 — fleet execution nodes

- **Issue:** #2179 (machine pool: onboard additional machines as fleet job
  nodes and redesign placement around a pool)
- **Queue task:** task 8927 (repo-health-code-pilot), lane code-d
- **Status:** v1 draft for review. This is the first reviewable,
  non-physical phase of #2179: it defines the contract a machine must
  satisfy to join the execution pool and the shape of the registry that
  describes pooled machines. It onboards nothing. #2179 stays open for
  the review round, the first-machine application, and the placement
  redesign that consumes this registry.
- **Inputs (verified fresh 2026-11-04 before writing):**
  - #2179 body and intake review (fleet monitor, 2026-10-29): draft the
    onboarding standard + registry shape first, then apply the standard
    to the first inventoried machine.
  - `doc/2026-11-03T15-10-56Z_design_task-placement-framework-v1/` —
    the placement framework deliberately defers the machine dimension
    to its v1.1 "once #2179's shape is known" (lines 241–243). This
    doc is that shape input; it does not change placement rules.
  - `doc/2026-10-27T03-14-08Z_status_basement-cluster/` — the basement
    machines have no OS installed; nothing is installed until the
    declarative bootstrap flow is proven. The standard therefore
    assumes a greenfield NixOS install from this flake, never a
    hand-built machine.
  - Current flake structure: hosts are declared in `./nixos/hosts`
    and turned into `nixosConfigurations` by
    `lib.make-nixos-configurations` (flake.nix). A pooled machine is
    a host entry in that existing mechanism, not a new one.
  - The laptop (amd-6) is the reference implementation of every
    requirement below: it is the one machine that runs the fleet
    today (OpenCode server, queue workers, OpenClaw gateway), so each
    requirement is phrased as "whatever amd-6 does, declared" rather
    than as a new invention.

## 0. Scope and the one boundary that matters

The fleet has two different machine programs, and this standard keeps
them separate:

| Program                            | What the machine is                                                                                     | Governed by                                                                   |
| ---------------------------------- | ------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------- |
| **Execution node** (this standard) | A fleet job node: it runs OpenCode sessions, queue workers, and scheduled jobs the way amd-6 does today | This doc; registry in §3                                                      |
| **Lab cluster node**               | A Talos/Cozystack (Kubernetes) node in the basement cluster program                                     | The lab-cluster goal and its TALM bootstrap flow; the cluster's own inventory |

- A machine is an execution node, a lab-cluster node, or — only by an
  explicit, recorded decision on #2179 — both. Nothing is both by
  default, and neither program's inventory silently doubles as the
  other's.
- **Identity is distinct:** an execution node's registry identity
  (§3) is its own record. If the same physical machine also appears
  in the Talos/Cozystack inventory, the registry record links to that
  inventory entry by reference; it never reuses the cluster node name
  as its identity, and a change in one program's inventory does not
  edit the other's.
- This standard covers execution nodes only. It says nothing about
  how Talos nodes are provisioned, and the lab-cluster flow says
  nothing about how execution nodes join the pool.

## 1. The onboarding contract — requirements

A machine joins the pool by satisfying every requirement in this
section, declared in the repo (NixOS configuration, registry record),
never by hand-crafting state on the machine. "Joins by applying the
standard" means: a reviewer can diff the machine's declared
configuration against this section and find no gaps.

### 1.1 NixOS flake configuration

- The machine is a host entry under `./nixos/hosts`, built by the
  flake's existing `lib.make-nixos-configurations` path — the same
  mechanism that builds amd-6.
- The host configuration composes the shared fleet modules (the same
  common configuration amd-6 uses, including the proxy port
  definitions in `home/common/default.nix`) plus a host-specific
  module limited to hardware facts (disks, network interfaces, GPU)
  and the machine's registry-declared capacity (§3).
- The machine rebuilds from the flake unattended: no interactive
  steps, no state that exists only on the machine. If it cannot be
  rebuilt from the repo, it is not onboarded.
- The host keeps the fleet's primary-clone rule: the repo clone used
  for deploys sits on `main`, clean; all work happens in worktrees.

### 1.2 SSH access pattern

- SSH is the only operator/agent access path, and it follows the
  amd-6 pattern exactly: key-only authentication, the fleet's
  existing authorized keys, reached over the tailnet/LAN address
  recorded in the registry record (§3) — no per-machine passwords, no
  one-off tunnels.
- The sandbox and the other pooled machines reach the new node the
  same way they reach amd-6 today (the multiplexed SSH helper pattern
  used for the laptop), so existing tooling works against the new
  node unchanged once its registry record exists.
- Root login is disabled; the fleet operator account is the single
  login identity, with sudo, matching amd-6.

### 1.3 OpenCode server (attach pattern)

- The machine runs the fleet's OpenCode server (the 2.x fork build
  the fleet standardised on — see `home/common/default.nix`), as a
  managed service under its NixOS configuration, listening on the
  port its configuration declares from the shared proxy/port
  definitions — never a hand-picked port.
- Consumers **attach** to the machine's server; they do not spawn
  ad-hoc servers. OpenChamber and the dispatcher attach using the
  registry record's endpoint (§3), exactly as they attach to amd-6
  today.
- The server is expected to hold multiple concurrent sessions up to
  the machine's declared capacity (§3); the current single-machine
  session cap is a property of amd-6's record, not a fleet constant.

### 1.4 Queue-worker capability

- The machine can run queue workers for the repo-health fleet: it
  holds a clean clone of each repo it serves, creates per-task
  worktrees under its fleet worktree directory, and runs worker
  shifts under the same runbooks as amd-6 (lane lock, atomic claim,
  checkpoint/park discipline unchanged — those are properties of the
  queue, not of the machine).
- Builds, tests, and linters for fleet work run on the pooled
  machine or in CI — the CI-first rule is unchanged; the pool exists
  precisely so this load no longer concentrates on amd-6.
- The machine's worker capability is declared in its registry record
  as capability flags (§3), so a scheduler never has to probe a
  machine to learn whether it may claim.

### 1.5 Health reporting

- The machine reports health into the same places amd-6 reports
  today: the fleet watchdog's coverage and the per-machine
  attribution that joins per-job/per-model attribution (#2072).
- Minimum health record, refreshed on the watchdog's cadence:
  reachable (SSH + OpenCode server responding), load headroom against
  declared capacity, disk headroom on the worktree volume, server
  session count, and last successful queue job.
- A machine that stops reporting is treated as **left the pool**
  (§4), not as healthy-but-quiet: silence is a state change, never a
  pass.

### 1.6 Secrets — names only

- A pooled machine may hold only the fleet secrets its role requires,
  provisioned through the fleet's existing secret managers
  (Bitwarden Secrets Manager / 1Password service accounts), never
  copied by hand and never recorded by value anywhere — including
  this standard and the registry.
- The classes a standard execution node may need, **by name only**:
  the Bitwarden Secrets Manager access token for its worker
  environment; the 1Password service-account token where the worker
  runbooks require it; the GitHub authentication its `gh` CLI uses;
  provider route credentials the OpenCode server is configured with.
  A machine that needs a name not on this list gets the list amended
  by review before it gets the secret.
- The registry record (§3) names which secret _classes_ a machine
  holds. It never holds values, fingerprints, or expiry data.

## 2. Join procedure

1. **Inventory input exists.** The machine has an entry in the
   on-site evidence cluster (#1095/#1097/#1099/#1101) — hardware,
   firmware, power. Software steps below do not substitute for this.
2. **Host entry lands.** A PR adds the machine under `./nixos/hosts`
   per §1.1 and its registry record per §3, in the same PR.
3. **Install from the flake.** NixOS is installed from the repo's
   unattended-installer configuration; no manual post-install steps.
4. **Attach check.** From the sandbox: SSH per §1.2, OpenCode server
   attach per §1.3, both verified and recorded on #2179.
5. **First real work.** The machine runs one real queue job to a
   terminal state and one OpenCode session, reviewed like laptop
   work. This is #2179's acceptance test for the first machine; a
   machine that has not done this is `declared`, not `active` (§3).
6. **Watchdog coverage confirmed.** The machine appears in health
   reporting per §1.5; only then does its record become `active`.

## 3. Registry shape

One record per execution node. The registry is a reviewed repo
artifact (a single data file consumed by schedulers and the
watchdog); this section defines its shape, not its storage
implementation — the placement redesign (#2179 step 3) chooses where
it lives, and must not change the shape to fit the store.

```yaml
# one record per execution node
id:
  amd-6 # registry identity; distinct from any
  # Talos/Cozystack node name (§0)
hostname:
  amd # NixOS host key under ./nixos/hosts — the
  # flake target (nixosConfigurations.amd in
  # nixos/hosts/default.nix); distinct from the
  # registry id above, which is fleet identity
state: active # declared | joining | active | draining | retired
roles:
  [execution] # execution | lab-cluster | both (both only
  # by a recorded #2179 decision, §0)
links:
  talos_inventory:
    null # reference into the lab-cluster inventory
    # when the same hardware appears there
  evidence_issues: [] # #1095/#1097/#1099/#1101 entries for this box
access:
  ssh: user@host-or-tailnet-name # pattern of §1.2; no credentials
  opencode_server: http://host:port # attach endpoint of §1.3
capabilities:
  queue_worker: true # may claim repo-health queue work (§1.4)
  opencode_sessions: true
  builds: true # may run laptop-class builds/tests (§1.4)
  gpu: false
capacity:
  max_sessions: 12 # amd-6's current value; per-machine, §1.3
  cpu_cores: 0
  ram_gb: 0
  worktree_disk_gb: 0
secrets: [] # secret CLASS names only (§1.6), never values
health:
  source: watchdog # who reports (§1.5)
  last_seen: null # ISO-8601; stale => treated as left (§1.5)
  status: unknown # healthy | degraded | unreachable | unknown
```

Rules for the shape:

- **Identity is the registry's own.** `id` is assigned when the
  record is created and never reused after `retired`. It is not the
  Talos node name, not an IP address, and not a session or host that
  happens to be current.
- **Capabilities and capacity are declarations, not probes.**
  Schedulers place work from these fields; changing what a machine
  may take is a reviewed edit to this record plus the host
  configuration that backs it.
- **Health is a cache, not a source of truth.** The watchdog writes
  `health`; everyone else reads it. A stale `last_seen` demotes the
  machine for placement purposes without editing its declared state.
- amd-6 is the first record: writing it (with its real capacity
  values) is part of the first application of this standard, so the
  pool of one is described by the same shape as the pool of many.

## 4. Leaving the pool — drain shape

From #2179 step 4, stated against this standard's terms:

- A machine leaves by `state: draining`: schedulers stop placing new
  work from its registry record; in-flight sessions/jobs run to a
  terminal state or are visibly orphaned.
- Orphaned work appears in **one place** (the queue's claimed/parked
  state plus the watchdog's per-machine view) — never as silent loss,
  and never as corruption of another machine's state: worktrees,
  clones, and server state are per-machine by construction (§1.1,
  §1.4).
- Unplanned departure (off, overloaded, network) is the same shape
  with no notice: health goes stale (§1.5), placement stops, and the
  orphaned-work view is how the fleet notices what it held.

## 5. What can proceed now vs what is blocked

**Software work that can proceed now (no hardware dependency):**

- This standard and registry shape (this doc) through review.
- The registry artifact itself, with amd-6 as its first record (§3).
- The placement framework v1.1 machine dimension, which was deferred
  pending this shape.
- Watchdog per-machine reporting for amd-6 against §1.5's minimum
  record.

**Blocked on the physical world (Drew, on-site):**

- Hardware inventory, firmware/BIOS audit, power and rack work for
  the basement machines — the evidence cluster #1095/#1097/#1099/
  #1101. Until a machine has its inventory entry, §2 step 1 fails
  and no install is attempted.
- The first-machine application of §2 (install, attach check, first
  real work): blocked on the first inventoried machine existing.
- Any claim that a second node is onboarded: nothing in this doc
  onboards a machine, and no registry record may say `active` for a
  machine that has not passed §2 step 5.

## 6. Follow-ups this design creates

- Registry artifact + amd-6's record (software, unblocked) — to be
  tracked on #2179.
- Placement framework v1.1 machine dimension — input to the
  task-placement framework doc, tracked there and on #2179.
- First-machine application — blocked on #1095/#1097/#1099/#1101;
  tracked on #2179, not filed separately (the set is one issue).
