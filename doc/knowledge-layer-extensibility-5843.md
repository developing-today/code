# Knowledge layer — extensibility cluster decision record (#5843)

Status: decided (design only; implementation timing is a separate
decision, per #5843). Parent program: #5678. Programme epic: #2056.
Object model: #2048. Evidence base: the resolved feature matrix
from #6435, `doc/knowledge-layer-feature-matrix-6435.md` (PR #7887,
merged bfee2fe6b6b1a37b2c6f65871f5dd33465c2b328; #6435 closed
completed 2026-11-09), especially §3 and the #5843 findings in §5.
This record settles the extensibility cluster only: plugins/APIs,
embeds/transclusion, and scripting/automation for the knowledge
layer. It does not re-decide any parent or sibling decision (see
"Preserved parent decisions") and it designs no importer
(importers: #6436, #6438; the importer line consumes decision 2
below).

## Preserved parent decisions

- The ruled read-authority resolution order from #5678 applies
  unchanged and applies to every surface this record creates:
  **object → user default → server config → public**. An API read,
  an embed render, and an automation action each resolve the target
  object's own read decision; no surface inherits a wider decision
  from a caller, a page, or a token.
- The local-first baseline adopted by the sync cluster (#5838):
  core use never requires a vendor's server. Every surface below
  works on-device and offline.
- The access-scoped backlinks floor from the graph cluster
  (#5837): an edge — and by the same rule an embed — to an object
  the resolved policy denies must not leak that object's existence.
- #2048's object model is the substrate: extensions, their state,
  and automation runs are objects/records in that model, not side
  files with parallel bookkeeping — the same rule the attachments
  cluster (#5844) applied to files and derivatives.
- Program-level open questions recorded on #5678 / #6435 §6 stay
  open; nothing here answers them by assumption.

## Decisions

### 1. API first — ADOPT a local, token-authenticated API as the first extensibility surface

The first extensibility surface is a local, token-authenticated
API over the corpus (the Joplin Data API / Anytype local HTTP API
shape, matrix §3.6/§3.8 and §5 #5843): reads and writes of objects,
search, and export, served by the local application on-device. One
contract serves every consumer class — end-user tooling, the
importer line, and AI tools (including an MCP facade over the same
contract, the Anytype shape) — instead of one integration per
consumer.

- Tokens are scoped, revocable, expiring capabilities. A token
  names the identity it acts for and the corpus scope it may
  reach; per-object reach is still resolved per read through the
  read-authority order, so a token can never widen its holder past
  the policy the holder already has.
- The API is the *only* extension path to the store. No plugin,
  script, or importer reads or writes corpus storage directly
  (invariant 1).

Rejected alternatives:

- **Enterprise-only API tier (NotebookLM shape)** — the API exists
  but only behind an enterprise plan (matrix §5 #5843). Rejected
  in the matrix findings and rejected here: it inverts this
  fleet's operating model, where the user's own agents are
  first-class API consumers.
- **Hosted-only REST API as the sole surface (Notion shape,
  alone)** — a capable public API, connections gallery, and
  automations (matrix §3.3), but hosted-only: no on-device surface
  when the vendor service is unreachable. That contradicts the
  #5838 local-first baseline. (A hosted relay may be *added* later
  for remote access; it is a transport for this same contract,
  never a second contract.)

### 2. Plugins — ADAPT declarative, capability-scoped registration; no in-process code in v1

Plugins register **declaratively** against the API in decision 1:
a plugin declares an identity, the capabilities it requests
(object types it reads/writes, hooks it subscribes to, importer
modules it provides), and renders/integrates only through API
responses. Two plugin shapes are adopted on that basis:

- **Importer/exporter modules** (the Joplin shape, matrix §5
  #5843 adapt option): plugins may register import/export modules;
  the importer line (#6436, #6438) consumes this registration
  instead of growing importer code into the core.
- **Hook subscribers** (decision 3): plugins may subscribe to
  object events through the automation surface.

Plugin state is stored as corpus objects under the plugin's own
identity, so backup, export, and the read-authority order cover it
with no special cases.

Rejected alternatives:

- **In-process community plugins executing arbitrary code inside
  the core application (Obsidian shape) for v1** — thousands of
  community plugins prove the demand (matrix §3.1), but arbitrary
  in-process code inherits the whole application's authority: it
  bypasses per-object read resolution by construction, and its
  lifecycle (updates, signing, revocation) is an unsolved
  supply-chain problem at this program's scale. The declarative
  registration above keeps the door open: if a later phase adds an
  in-app host, plugins are already identified, capability-scoped,
  and state-in-corpus, so the host grants exactly the declared
  capabilities and nothing ambient.
- **Themes/visual plugins as an extensibility substitute** —
  presentation plugins (Obsidian/Logseq themes) do not settle any
  question this cluster was asked (data access, embeds,
  automation). Cosmetic theming is out of scope for this record,
  neither adopted nor rejected as a feature.

### 3. Embeds and transclusion — references resolved through read authority, never copies

An embed/transclusion is a **reference** to an object (or a block
of one), rendered by resolving the target through the embedding
reader's read-authority decision at render time:

- Granted: the target renders inline; updates to the target
  propagate, because nothing was copied.
- Denied: the embed renders an explicit tombstone ("an object is
  embedded here that your access does not cover") — visible as a
  gap in the page, but carrying no title, excerpt, or metadata of
  the target. This is the #5837 no-existence-leak floor applied to
  embeds, with the §5844 rule that an attachment referenced by a
  page keeps its own read decision.

Rejected alternatives:

- **Embed-by-copy / snapshot embeds (the OneNote attachment
  failure mode, rejected again from #5844)** — a copied embed
  forks identity and keeps rendering content after the target's
  read decision narrows or the target is deleted; the copy's
  authority is whatever the copying page had, forever.
- **Silent omission of denied embeds** — dropping the embed
  without a tombstone makes a page misrepresent its own content
  (the same silent-drop defect class the attachments record
  rejected for exports). A reader must be able to tell that
  something is withheld.

### 4. Scripting and automation — automations are API clients under a named identity

Scripting and automation run **as API clients** (decision 1)
under a named, auditable identity — the Notion automations /
webhooks shape (matrix §3.3) adapted to the local-first baseline:

- Triggers are object events (created / updated / deleted,
  property transitions) delivered to subscribers as webhooks or
  local callbacks; actions are ordinary API calls made with the
  automation's own token.
- An automation's token is scoped like any other (decision 1): it
  can do exactly what its identity's resolved policy allows, and
  every action it takes is an event in the log attributed to that
  identity.
- There is no separate scripting runtime with ambient authority
  inside the application in v1. User scripts (shell, Python, any
  MCP client) already reach the corpus through the API/MCP
  contract; embedding a second, privileged interpreter would
  create a second contract to secure and audit.

Rejected alternatives:

- **A built-in privileged scripting engine** (in-app code with
  application-level authority) — rejected for the same reason as
  in-process plugins (decision 2), with an additional audit cost:
  automation that cannot be attributed to a scoped identity
  cannot be reviewed after the fact.
- **Vendor-hosted automation only (cloud automations as the sole
  shape)** — breaks the local-first baseline (#5838): automations
  over a local corpus must fire locally, including offline.

## Implementation invariants

Any implementation of this cluster must hold all of the
following; a change that breaks one is a defect against this
record, not a design evolution of it.

1. **One contract.** Every extension surface — plugins,
   importers, scripts, AI tools — reads and writes through the
   local API of decision 1. No side channels to corpus storage.
2. **Read authority is resolved per object, per read**, in the
   ruled order (object → user default → server config → public),
   on every surface including API reads, embed renders, graph
   edges, and automation actions.
3. **Tokens are capabilities**: scoped, revocable, expiring, and
   never wider than the resolved policy of the identity they act
   for.
4. **Embeds are references, never copies**, and a denied embed
   leaks nothing about its target beyond the fact of a gap.
5. **Automation acts under a named identity** whose every action
   is an attributable event; no ambient-authority runtime exists.
6. **Extension state lives in the corpus** as objects under the
   extension's identity — exportable, backable-up, and
   policy-resolved like everything else.
7. **Local-first**: every surface works on-device and offline; no
   surface requires a vendor server (#5838 baseline).

## Follow-ons

- Implementation timing is a separate decision (per #5843's
  framing); this record is the input to that scheduling, not a
  schedule.
- Importer issues #6436 and #6438 consume decision 2's
  module-registration shape.
- If demand for an in-app plugin host is later proven, it is a new
  design ticket that starts from decision 2's registration and
  capability model — it does not reopen decisions 1, 3, or 4.
