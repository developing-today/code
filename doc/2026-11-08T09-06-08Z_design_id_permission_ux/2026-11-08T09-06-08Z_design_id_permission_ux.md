# Design: Permission UX for worlds — pre-connect disclosure, grant modes, and memory

- Date: 2026-11-08
- Status: **design decision record**, ready for issue-owner review; no implementation is included.
- Issue: [#5960](https://github.com/developing-today/code/issues/5960), the permission-UX child of the worlds design programme [#5955](https://github.com/developing-today/code/issues/5955) (statements S19–S23 of the 2026-11-04 brief).
- Sibling records: capability model [#5958](https://github.com/developing-today/code/issues/5958), capability-change notices [#5959](https://github.com/developing-today/code/issues/5959), precedent research [#5961](https://github.com/developing-today/code/issues/5961) (coordination and dependencies in [Sibling coordination](#sibling-coordination)); knowledge-layer sharing & permissions [#5840](https://github.com/developing-today/code/issues/5840) (different layer, shared vocabulary); attenuated capabilities for world access (`doc/2026-10-09T09-59-07Z_design_attenuated_capabilities/`).
- Baseline: current `main` at `62e722d5a1`.

## Decision in brief

Before a client connects to a world, it shows a **pre-connect disclosure** in
the shape of the Play Store permissions screen: a short, plain-language list
of what the world will be able to do, split into two visibly separate groups —
capabilities exercised **using your client** and capabilities exercised
**through the server side**. The two groups are never merged into one list,
because they implicate different parties: client-side capabilities spend the
user's own machine, network, and identity; server-side capabilities spend the
server operator's.

Granting is asymmetric, mirroring who is implicated. **Client-side**
capabilities are granted (or denied) by the connecting user, per capability.
**Server-side** capabilities are a **server admin decision**: the world's
operator configures them on the server, and the connecting user only sees
them disclosed — the prompt informs, it does not ask the user to approve
someone else's machine.

Both sides remember. Admin grants follow the grant-mode matrix in §3
(approve generally; ignore for all time for specific websites; ignore for
specific permissions; YOLO). The client-side per-user memory mirrors the same
shape: ask once, then remember per world, per permission, or globally, with
YOLO as an explicit user choice that is never a default and is always loud
in the UI (§5).

A denial never refuses the connection outright unless the capability is
**load-bearing** (§4): the world loads with the denied capability inert, and
the UI shows which features are degraded.

## Terms

- **Capability:** one named, user-comprehensible power a world may exercise
  (e.g. "connect to the internet", "read your files", "store data on the
  server"). Capabilities are the units of disclosure, granting, and memory —
  never raw API surfaces.
- **Client-side capability:** exercised using the connecting user's client:
  their device, their network egress, their local storage, their identity.
- **Server-side capability:** exercised through the serving server: the
  operator's network egress, storage, and compute.
- **Requested set:** the capabilities the world declares it wants, from its
  manifest.
- **Used set:** the capabilities the world has actually been observed to
  exercise (from the capability-model instrumentation, a sibling child of
  #5955).
- **Grant:** a recorded decision — by the user (client-side) or the server
  admin (server-side) — that a capability may be exercised, at a stated
  scope.
- **Load-bearing capability:** one without which the world cannot function
  at all (§4). Everything else is **optional** and degrades gracefully.

## 1. The pre-connect disclosure

The disclosure is shown **before the first connection completes**, and it is
a reading surface first and a decision surface second: server-side lines are
informational, client-side lines carry the grant controls.

### 1.1 Content

One line per capability, in plain language, grouped and headed:

```
This world can:

Using your client
  • Connect to the internet using your client
  • Read files you choose on this device
  • Keep data on this device between visits

Through the server side
  • Read things from the internet through the server side
  • Store world state on the server
  • Send messages to other worlds from the server
```

Rules:

- The actor phrase is part of the sentence, never a footnote or an icon:
  "using your client" vs "through the server side" is the distinction the
  whole design exists to surface (S19).
- No capability appears in both groups. If a world can do the same verb
  both ways, it is two lines, one per group, because the grant and the
  implicated party differ.
- The list is the **requested set**. When a **used set** is known (the
  world has been observed, by this client or by published instrumentation)
  and it differs from the requested set, the disclosure shows both, in
  the world's favour-neutral wording: a line that was requested but never
  used is annotated "asks for this, has not used it"; a capability used
  but never requested is a defect — it is shown highlighted, treated as a
  new request, and reported through the capability-change channel (§2).
  Over-requesting is visible, not punished silently; under-declaring is
  never hidden.
- The disclosure names the world and the server it will connect to, so a
  remembered grant is never re-presented as if it were a fresh decision
  about a different counterparty.

### 1.2 When prompts fire

- **First connect** to a world (no memory): full disclosure, decision
  required for client-side lines.
- **Capability change**: the requested set grows or a line moves between
  groups → prompt again, showing only the delta, in the change-notice
  shape owned by the capability-change child of #5955. A shrinking
  requested set never prompts; memory narrows silently.
- **Unchanged reconnect** with a remembered grant covering every requested
  client-side capability: no prompt. The connection proceeds; the grant
  state remains inspectable from the world's page (§5).
- Server-side lines never block a user decision; if the server-side set
  changed since the user's last visit, the reconnect shows a one-line
  notice ("the server side can now also …") with no decision attached.

## 2. Denial semantics

- Denial is **per capability**. There is no single "reject the world"
  control on the disclosure; there is a per-line deny, and leaving is
  always possible but never forced.
- A denied **optional** capability is **inert**: the world loads, the
  capability's calls fail closed at the client (or are refused by the
  server, for server-side lines an admin has not granted), and the UI
  marks the affected features as unavailable with the reason ("file
  access is off for this world"). The world is told only that the
  capability is unavailable, in the same shape as a platform that lacks
  it — denial is not fingerprinted beyond necessity.
- A denied **load-bearing** capability refuses the connection, and the
  refusal says so plainly: "This world cannot run without X." A
  capability may be declared load-bearing only in the world's manifest,
  only for capabilities the world genuinely cannot degrade around, and
  the disclosure marks such lines before the decision ("required").
  Declaring an optional capability load-bearing to coerce grants is a
  manifest-review defect, not a UX choice.
- Denials are remembered with the same scoping as grants (§3, §5): a
  user who denied a capability for a world is not re-asked on every
  reconnect. Changing a remembered denial happens from the world's
  grant page, or when the capability's requested shape changes.

## 3. Server-admin grant modes

Server-side capabilities are granted by the **server admin**, from the
server's admin surface — never by the connecting user, and never as a
side effect of a user grant. The admin decides at one of four modes,
settable per capability and per world (site):

| Mode                                        | Meaning                                                                                                  | Prompt behaviour                                                                                                          |
| ------------------------------------------- | -------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------- |
| **Approve generally**                       | This capability is granted, for all worlds/sites the server serves.                                      | No per-world admin prompt. Disclosed to connecting users as a server-side line.                                           |
| **Ignore for all time — specific websites** | For the named websites (world origins), this capability is never granted and never prompted about again. | Admin is never re-asked for those sites; the capability is inert there. Connecting users see the line marked unavailable. |
| **Ignore — specific permissions**           | The named permissions are never granted on this server, for any world.                                   | Admin is never re-asked for those permissions; they are inert everywhere on the server.                                   |
| **YOLO mode**                               | Everything is granted; the admin has decided not to decide.                                              | No admin prompts at all. See §5 for scoping and loudness.                                                                 |

Rules:

- "Ignore" is a decision, not an absence of one: it is recorded, listed
  on the admin grant page, and revocable from there — never by being
  re-prompted. Re-prompting after an ignore is a bug.
- Modes compose by specificity: a per-website ignore beats a general
  approve for that website; a per-permission ignore beats a general
  approve for that permission. YOLO beats nothing — any recorded ignore
  survives YOLO (§5).
- Every admin grant decision is recorded with who, when, mode, and scope,
  in the server's grant log; the log is the audit surface for "why can
  this world do that through my server".

## 4. Load-bearing declarations

A world's manifest may mark a capability **required**. The set of
required capabilities is part of the requested set shown pre-connect.
A world whose required client-side capability is denied does not load
(§2). Servers apply the same rule to server-side lines: a world whose
required server-side capability is ignored or ungranted is not served.
Required markings are expected to be rare — a well-built world degrades
— and the manifest review (human or automated) treats a long required
list as a smell to justify, per capability.

## 5. Memory, scoping, and YOLO

### 5.1 Client-side memory (per user)

A user's client-side decisions are remembered at the scope the user
chose on the prompt, with the narrowest scope the default:

- **This world only** (default pre-selection): grant/deny applies to
  this world, this permission.
- **This permission, all worlds**: applies across worlds for that one
  capability.
- **All permissions, this world**: full trust in one world.
- **Globally**: all permissions, all worlds — offered, never
  pre-selected.

The grant page per world lists every remembered decision with its
scope, and any decision can be revoked there; revocation takes effect
on the next capability call, not the next reconnect. Memory is stored
per user identity, never per device fingerprint, and follows the user
across their devices with their other ID state.

### 5.2 YOLO mode

YOLO exists on both sides (admin §3, user §5.1 global scope is its
client-side cousin but is _not_ YOLO — it is a remembered grant set;
YOLO proper is "stop asking, grant what arrives, including future
capabilities not yet requested").

- YOLO is **explicit opt-in only**, behind its own confirmation that
  names the consequence ("new capabilities will be granted without
  asking"). It is never a default, never a first-run suggestion, and
  never enabled by importing settings.
- YOLO is scoped **per server** (admin) and **per user** (client), and
  the admin form additionally offers a **time box** (YOLO until a date,
  then fall back to the pre-YOLO modes). Client-side YOLO is per user
  across servers, or per server when the user narrows it.
- YOLO is **loud**: while it is on, the surfaces it governs carry a
  persistent, non-dismissible indicator (admin console banner; client
  connection badge). A YOLO state that can be forgotten is a defect.
- Recorded **ignores survive YOLO** on both sides: YOLO waives future
  decisions, it does not rescind past ones.

## 6. Worked example

A user first-connects to a world hosted on someone else's server. The
world requests, client-side: internet via the client, local file read;
server-side: internet via the server, persistent storage. Storage is
marked required (server-side).

1. The disclosure shows four lines in two groups; the storage line is
   marked "required".
2. The server admin has storage on _approve generally_ and server-side
   internet on _ignore — specific permissions_: the world is served,
   its server-side internet line is disclosed as unavailable, and the
   world runs without it (it was optional).
3. The user grants client internet "this world only", denies file read
   globally ("this permission, all worlds" scope on a deny). The world
   loads; its file features show as unavailable.
4. On reconnect, nothing prompts: memory covers every requested line.
5. The world ships an update requesting client-side notifications.
   The reconnect prompts on the delta only — one new line, decision
   required, everything else untouched.

## Sibling coordination

Recorded 2026-11-08 against the live state of the design children of
[#5955](https://github.com/developing-today/code/issues/5955). The sibling
outputs are not finished, so every dependency below is named rather than
silently assumed; nothing here claims a sibling decision it does not yet own.

| Sibling                                                                                                            | State at authoring                                                                                       | What this record takes from it                                                            | Dependency                                                                                                                                                                                                                                                                                                   |
| ------------------------------------------------------------------------------------------------------------------ | -------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| [#5958](https://github.com/developing-today/code/issues/5958) capability model — requested vs discovered-used sets | open; design PR [#7599](https://github.com/developing-today/code/pull/7599) in review, not yet on `main` | the requested/discovered-used vocabulary and the registry listing that names capabilities | **dependency.** The requested-vs-used disclosure in §1.1 presumes #5958's used-set measurement. Until it lands, the used set is shown only when a world publishes instrumentation, and the disclosure degrades to the requested set alone.                                                                   |
| [#5959](https://github.com/developing-today/code/issues/5959) capability-change notices                            | open, triaged, waiting on dependency                                                                     | nothing yet                                                                               | **dependency.** The "capability change" firing rule in §1.2 consumes #5959's classification of expansion vs sideways change. This record fixes only the prompt behaviour for a change, not the notice taxonomy or transport.                                                                                 |
| [#5961](https://github.com/developing-today/code/issues/5961) permission-prompt precedents                         | open, in progress; research not landed                                                                   | nothing yet                                                                               | **assumption.** The prompt shape follows the Play Store precedent fixed by the 2026-11-04 brief (S19–S23). If #5961's findings contradict a specific content rule here (group labelling, denial copy, memory defaults), the issue owner revises that rule; it is not treated as frozen against new evidence. |
| [#5840](https://github.com/developing-today/code/issues/5840) knowledge-layer sharing & permissions                | separate layer                                                                                           | shared vocabulary (principal, grant, scope)                                               | none — the surfaces are deliberately different.                                                                                                                                                                                                                                                              |

## Out of scope

- The capability model itself (what capabilities exist, how the used
  set is measured): [#5958](https://github.com/developing-today/code/issues/5958).
- Capability-change notices beyond the prompt behaviour named here:
  [#5959](https://github.com/developing-today/code/issues/5959).
- Knowledge-layer sharing (notes, groups, read/edit grants): #5840.
  This record governs _world connection_ permissions; the vocabulary
  (principal, grant, scope) is shared deliberately, the surfaces are
  not.
- Implementation: this record is the UX contract an implementation
  issue can be written against; it contains no code.

## Open questions for the issue owner

- Should the client-side used-set annotation ("asks for this, has not
  used it") also feed a public, per-world reputation surface, or stay
  a private per-user disclosure aid in v1? This record assumes private
  for v1.
- Does the admin grant log need export (e.g. for hosted multi-admin
  servers), or is the on-server page sufficient for v1? Assumed
  sufficient here.
