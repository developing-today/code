# Decision record: host providers — terminal & website instantiation of the #5686 provider model

- **Issue:** #5902 (design split 7 of #5686)
- **Decided:** 2026-11-16, queue task 12879 (repo-worker-code, lane any-b)
- **Status:** decided for the host face (terminal, client and server modes;
  website). Model-level mechanisms stay with their owning splits: declaration
  #5898 (landed), grants #5900 (landed), security model #5905 (landed),
  offline/degraded #5904 (landed); discovery & negotiation #5899 is the one
  sibling still open — this record consumes it as an interface (assumption A1)
  and flags its one input there, exactly as #5901 and #5903 did. This record
  decides only what those splits do not own: how the one provider model is
  instantiated on terminal and website hosts — the sandbox and lifecycle
  rules #5902's charter names.
- **Inputs:** parent #5686 (the fixed model: signed, typed, versioned
  declarations offered into the ceiling model of #789; grants explicit,
  revocable, auditable; observation ≠ actuation), its 2026-11-14 triage
  (the parent is a tracking record, not a gate; #5899 and this issue are its
  live remainder), the field survey #5931 §1 (terminal), §3 (web/browser),
  §6 (cross-cutting layers), §7 (flags), the sibling instantiation records
  #5901 (constrained devices) and #5903 (embedded), and the client-provider
  design F13.7 (doc/2026-10-25T05-37-57Z_design_client_capability_providers),
  whose two-key rule and `side` field this record's terminal client mode and
  website mode both inherit.

## The decision in one paragraph

On both host classes the authoritative declaration is `id`'s own signed,
versioned provider manifest (#5898) — never a host-platform registry. The
terminal provider is the **multiplexer session**, not any emulator surface
attached to it: its typed surface is OSC 133-structured **command blocks**,
with raw PTY bytes as a separate, higher-sensitivity capability. The website
provider is the **page in the user's tab**: WebMCP registration is a
generated projection of the manifest, never the declaration, and a website
provider must remain invocable through `id`'s own shell channel where WebMCP
is absent. On both hosts, grants bind to the session/tab **instance** and die
with it by default (the #5903 D4 rule, adopted unchanged), actuation is
never ambient, and every transport in this record (tmux control protocol,
WebMCP, native messaging) carries no authority — the grant check happens at
the writer-set layer on every invocation.

**Assumptions (explicit, in the #5901 pattern)**

- A1 — #5899 (OPEN) lands negotiation as one protocol shape that admits
  transport profiles, as #5901 and #5903 both assumed. If it settles on a
  single transport-less protocol instead, both faces here conform; no
  decision below depends on the profile claim except the D6 transport note.
- A2 — The F13.7 client-provider design's open dependencies (an F12.4-style
  registry with per-manifest `default_ceiling`, and the audit log) land in
  the shape that design names. D3's two-key rule is stated against that
  shape; if the registry shape changes, D3's mechanics reopen, its rule
  (both keys, either revokes) does not.

## D1 — One declaration, per-host projections (both hosts)

- The #5898 signed manifest is the sole declaration artefact for terminal
  and website providers, in the same shape as every other provider class.
  A host-specific declaration format is rejected for the reason #5903 D1
  gave: #5686's test is that the model needs no per-target special case.
- **Terminal projection:** multiplexer and emulator surfaces (tmux formats
  and pane state, kitty remote-control state, a Zellij layout) are
  projections of the session's manifest: generated from it, regenerable,
  never reverse-authoritative.
- **Website projection:** a page's WebMCP registrations
  (`document.modelContext.registerTool`, #5931 §3.1) are generated from the
  manifest — name, description, JSON Schema, and the cross-origin fields
  (`exposedTo` / `fromOrigins`) derived from the current grant set.
  Rejected as the authoritative declaration: WebMCP is a draft Community
  Group report (not a W3C standard; Chrome origin trial; WebKit's position
  is oppose), its hints (`readOnlyHint`, `consequentialHint`) are hints and
  not enforcement, and its authority model is the page's own — adopting it
  as the model would hand layer 4 to the platform, the standing warning of
  #5931 §6.7. Also rejected: ignoring WebMCP — it is the first
  platform-native page-as-provider surface, and the projection is cheap
  because it is generated. Drift is resolved by regenerating the
  projection, never by importing page state back into the manifest. (This
  answers survey §3's first decider question: yes, the writer-set object
  is authoritative and WebMCP is its in-tab projection.)

## D2 — Terminal provider shape: the session is the provider; command blocks are the typed surface

- **The provider is the multiplexer session layer, not the emulator.** A
  terminal emulator (kitty, WezTerm, Ghostty, iTerm2) is a surface: it
  attaches, renders, and detaches. The session — panes, processes,
  scrollback — outlives any surface, and it is the session that holds the
  capabilities (run a command, read a command's output and exit status).
  Declaring the emulator the provider would bind grants to the most
  transient object in the stack. (Answers survey §1's second decider
  question.)
- **Substrate is a protocol, not a product.** The reference substrate is
  the tmux control-mode class of interface (documented, line-based,
  scriptable: `capture-pane`, `send-keys`, `pipe-pane`, formats; #5931
  §1.1) because it is the deepest and most stable programmatic surface in
  the class. The design binds to the *shape* (a session substrate with
  pane/command introspection), not to tmux the binary: WezTerm domains
  and Zellij expose the same shape. Zellij's WASI-plugin panes (#5931
  §1.1) are noted as the field's proof that terminal capabilities can be
  sandboxed components — the provider-component convergence #5898/#1061
  own — but a single product's plugin API is not adopted as this class's
  substrate. Coder AgentAPI is rejected outright: archived 2026-09-13
  (#5931 §1.3, dead flag).
- **Two capability surfaces, two risk classes** (answers survey §1's first
  decider question — both, as different capabilities):
  1. **Command blocks (default, typed).** The provider offers shell
     sessions as structured command blocks: command text, cwd (OSC 7),
     start/finish marks and **exit status** (OSC 133), and bounded
     output. Shell integration (OSC 133 + OSC 7) is required for this
     surface; it is what makes a terminal a provider of typed,
     auditable events instead of scraped bytes (#5931 §1.3).
  2. **Raw PTY (separate, higher class).** A byte-stream capability
     (read/write the raw pane) exists for consumers that genuinely need
     it, declared as its own capability at a higher sensitivity class
     under #5900: actuation-side, per-use approval, never bundled with
     the block surface's grants.
  A shell without integration offers only the raw surface, and its
  manifest says so — absence of structure is declared, not discovered
  at call time.
- **Client mode and server mode are one provider shape with two
  identities** (D5): client mode is a terminal on the user's own machine,
  held by their shell (the F13.7 `side: client` case); server mode is a
  terminal hosted on a server/Node for authorised consumers. The
  capability set, manifest, and sandbox rules are identical; what
  differs is whose session it is and which identity signs (D5).

## D3 — Terminal sandbox rules

A terminal is the least sandboxed host in the family — a shell is
inherently powerful — so this record states the boundary honestly instead
of pretending a stronger one:

- **The sandbox is the declared envelope, enforced by the provider
  host:** declared working-directory roots (commands start and stay
  inside them; a `cd` out of the envelope is a provider-side refusal,
  not a consumer error), a declared environment (the session's env is
  the manifest's set, not the ambient login env — no inherited secrets
  beyond what the declaration names), and the manifest's bounds
  (output `max_bytes`, `per_minute`, `in_flight`, in the F13.7 manifest
  bounds shape) applied to every reply.
- **Observation is not free.** Scrollback and pane capture can contain
  anything the user ever saw, including secrets. Capture/read
  capabilities are observation-class under #5900 but never
  grant-on-use, and their grants name the session instance (D5).
- **Actuation is never ambient** (#5900 D2, #5905): running a command or
  sending input is actuation-class — per-use approval at the higher
  sensitivity classes, standing grants only where #5900's classes
  permit, and every actuation leaves the dual audit entries (host and
  client) the F13.7 design requires.
- **Two-key rule in client mode** (F13.7, adopted): a client-mode
  terminal provider requires both the world's grant (ceiling ∩ grant,
  with `default_ceiling: excluded` — it joins no ceiling until an admin
  names it) and the local user's per-world opt-in. Either key revokes.
  Grant-on-use never applies to a `side: client` provider. In server
  mode the second key is the hosting operator's admission of the
  session as a provider (#5905: admission is an explicit act; every
  class enters at `default_ceiling: excluded`).

## D4 — Website provider shape: the page is the provider; the tab is the sandbox

- **Direction is fixed.** "Website provider" in this record means the
  survey's §3.1 direction: a page offers capabilities to authorised
  consumers, executed in the page, in the user's session, visibly. The
  inverse direction (§3.4 — browser-automation servers exposing the
  browser itself to an external agent) is a provider *of the browser
  application*, a host-application class; it is not a website provider
  and is not decided here.
- **Execution context is the tab, and the grant check is `id`'s.** The
  page's tool callback does not execute on its own authority: calls
  route through the user's `id` shell (the client that hosts the tab
  session), which performs the writer-set grant check on every
  invocation before the page code runs — ingest and registration are a
  validity gate, never an authority gate (#5931 §0, #5905). WebMCP's
  Permissions-Policy gate (`tools`, default `'self'`) and its
  cross-origin grant fields are respected as platform constraints and
  generated from the `id` grant set (D1); where platform and `id`
  disagree, the intersection holds and the denial names its gate (the
  #5903 D3 composition rule, applied to platform-vs-`id` instead of
  OS-vs-`id`).
- **The WebMCP projection is optional; the provider is not.** Because
  WebMCP is a draft with a WebKit oppose on record, a website provider
  must also be invocable through `id`'s own channel — the browser
  extension / native-messaging seam of #5931 §3.2, which is also where
  a browser provider meets a local `id` node. Same manifest, same
  grants; WebMCP widens discovery, never authority. Consuming *other*
  sites' WebMCP tools makes them foreign providers under the same rule:
  ingested declarations are validated, and authority comes only from
  an `id` grant (answers survey §3's second decider question).
- **Browser device APIs are capabilities with availability as a
  lifecycle fact.** A page may declare Chromium-led device capabilities
  (WebSerial, WebUSB, Web Bluetooth, WebHID, File System Access; #5931
  §3.3) — this is where the website and constrained-device (#5901)
  classes touch, and the browser is then a bridge surface for the
  device, with the device class's rules applying behind it. Where the
  API is absent (non-Chromium engine, permission revoked, device
  unplugged) the capability is **suspended** — shown suspended in
  discovery, its grants inert — under the #5903 D3(3) suspension
  semantics, not merely failing at call time. ID's own browser fleet
  being Chromium-based makes these acceptable as a substrate
  (survey §3's third decider question: yes, with suspension declared).

## D5 — Lifecycles and identity (both hosts)

- **Terminal lifetime = session lifetime.** The provider exists while
  its multiplexer session exists. Session death tombstones its
  discovery entries (#5686 stale-discovery rule, #5903 D4): consumers
  see provider-gone, not a hang. Declared lifetime scope is
  `user-session` in client mode and `server-session` in server mode
  (server sessions may be long-lived; the scope is declared either
  way, and availability is never promised beyond it — #5904's
  lifecycle-honesty rule).
- **Website lifetime = tab lifetime** — the shortest scope in the
  family. Navigation, reload, or tab close is instance death:
  re-declaration on the new instance, discovery tombstoned for the
  old. A website provider never promises availability beyond the
  open tab.
- **Grants bind to the instance** (the #5903 D4 answer, adopted for
  this class unchanged): a grant names (provider identity,
  capability, session/tab instance). Instance death makes its grants
  dormant; they do not transfer to a later session or tab of the
  "same" provider. A standing grant may opt in to renewal against a
  new instance of the same provider identity; the default is no
  renewal. (This answers #5686's open question — do grants outlive a
  session-bound provider? — for the host classes: no, not by
  default, matching the embedded answer so the family has one rule.)
- **Identity.** Client-mode terminal: a delegated identity under the
  user sitting at the terminal; the user is ceiling owner, the
  delegation revocable by them. Server-mode terminal: the hosting
  node's operator identity, delegated per session. Website: an
  origin-bound provider identity — the site operator's key signs the
  declaration (verified at admission and on change, #5898 D3/#5905);
  the user's browser session is the execution context, not the
  provider identity, and the user's own key is their shell's
  consumer-side opt-in (D3's second key).
- **Transports carry no authority** (#5905, #5931 §6.7): tmux control
  protocol, WebMCP's in-page channel, extension native messaging,
  and any MCP surface a host exposes are layer-2 transports. A
  transport credential identifies a principal; it is never the
  grant. Bridges and relays are transport, never principals.

## Invariants implementation must hold

1. **Manifest sole authority.** Multiplexer/emulator state and WebMCP
   registrations are projections: generated from the signed manifest,
   regenerable, never a source of authority or declaration state.
2. **Ceiling composition.** No host mechanism raises a ceiling: a
   provider offers at most its delegated identity's ceiling, a
   projection exposes at most the manifest, the manifest at most the
   ceiling; both host classes enter at `default_ceiling: excluded`.
3. **Instance-bound grants.** Grants scope to (provider, capability,
   session/tab instance) on both hosts; instance death dormants
   them; renewal is opt-in per grant, never silent.
4. **Two-gate composition.** Client-mode terminal: world grant ∩
   local-user opt-in. Website: `id` grant ∩ platform gate
   (Permissions Policy, origin grants, device-API permission).
   Either denial is final for the invocation and names its gate.
5. **Actuation is never ambient.** Command execution, PTY writes,
   and page tool execution are actuation-class: no grant-on-use for
   `side: client` providers, per-use approval at the higher #5900
   classes, dual audit (host and client) for every use, allowed or
   refused.
6. **Lifecycle honesty.** Declared lifetime scope (user-session,
   server-session, tab) is part of the declaration; death tombstones
   discovery; suspended capabilities (missing shell integration,
   absent device API, revoked permission) show as suspended, not
   as silently failing.
7. **Data uniformity and provenance.** Output from a terminal or
   website provider lands in the same model as any other class,
   with provenance recording at minimum provider identity,
   capability, the grant it was obtained under, session/tab
   instance, and time (#5686 cross-cutting). Command-block output
   additionally carries its exit status; a non-zero exit is data,
   not a provider failure.

## Conflicts with the parent (#5686) — explicit check

- **No parent decision is changed.** #5686 fixes the model
  (declaration, discovery, negotiation, grants, ceiling composition)
  and leaves the host instantiation open as its decomposition
  item 7; this record is that item, decided inside the parent's
  constraints and its landed siblings (#5898, #5900, #5904, #5905,
  and the #5901/#5903 instantiation precedents).
- One parent open question is answered **for the host classes
  only** (grants outliving session-bound providers → D5: no, not
  by default) — deliberately the same answer #5903 gave for the
  embedded classes, so the family keeps one rule; the general
  statement remains #5900's to make.
- One position is flagged as **input to #5899, not a parent fork**:
  host negotiation takes transport profiles (tmux control mode,
  in-page WebMCP, native messaging). Per A1, if #5899 settles on a
  single transport-less protocol, these faces conform — the claim
  here is about message pattern, not a second negotiation model.
- The F13.7 client-provider design is consumed, not forked: its
  `side` field, two-key rule, `default_ceiling: excluded`, and dual
  audit are adopted as stated there for the client-mode cases it
  describes; this record extends the same rules to server-mode
  terminal and website hosts, which F13.7 did not cover.

## Splits and follow-ons

- **No further splits filed.** The design fits in this one record.
  Implementation is a separate decision, possibly months out (the
  issue's own framing): when it is filed, the terminal face is
  implemented against D2's substrate shape and the website face
  against D4's shell-channel-first rule, each consuming this
  record.
- With this record landed, #5686's decomposition is down to one
  live child (#5899, discovery & negotiation); the parent's own
  acceptance governs its closure from there.

## Alternatives rejected — summary

| Alternative | Why not |
| --- | --- |
| Emulator as the terminal provider | Binds grants to the most transient object in the stack; the session holds the capabilities and outlives every surface (D2) |
| Raw PTY as the only terminal surface | Bytes carry no structure to audit or bound; OSC 133 command blocks are the typed surface, raw PTY a separate higher-class capability (D2) |
| Binding the design to Zellij's plugin API | One product's internal API; its WASI shape is evidence for #5898/#1061's component question, not this class's substrate (D2) |
| Coder AgentAPI as substrate | Archived 2026-09-13 — dead (#5931 §1.3) |
| WebMCP registration as the authoritative declaration | Draft CG report, WebKit oppose, hints-not-enforcement, page-owned authority — layer 4 would belong to the platform (#5931 §3.1, §6.7) (D1) |
| Ignoring WebMCP entirely | Loses the first platform-native page-as-provider discovery surface, which the projection gets almost free (D1) |
| WebMCP-only website provider (no `id` channel) | A draft API with an oppose on record cannot be the only path; the extension/native-messaging channel works in every browser today (D4) |
| Browser-automation servers as "website providers" | The inverse direction — a provider of the browser application, not of the page; a different class (D4) |
| Standing grants surviving tab/session death | Breaks instance binding; replays authority into a context nobody granted (D5) |
| Grant-on-use for client-mode terminal providers | A shell is actuation-capable by nature; first use is exactly the use that must be opted into (D3, F13.7) |
