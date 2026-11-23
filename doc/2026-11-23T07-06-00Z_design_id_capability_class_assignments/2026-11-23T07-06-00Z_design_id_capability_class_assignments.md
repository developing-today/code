# Capability classes for the post-#5999 registrations — assignment design pass

- **Date:** 2026-11-23
- **Status:** PROPOSED design record, ready for ratification; no implementation is included here. Until this record is ratified and the implementation slice lands, the sixteen names below keep `CLASS_UNKNOWN` and always disclose — the fail-closed state #13004 records as correct behaviour, not a bug.
- **Issue:** [#13004](https://github.com/developing-today/code/issues/13004), extending the capability-class table of [#5999](https://github.com/developing-today/code/issues/5999) (§2.2–§2.4) as shipped by [#12982](https://github.com/developing-today/code/issues/12982); consumed by the comparison in [#12921](https://github.com/developing-today/code/issues/12921) and the notices of [#5959](https://github.com/developing-today/code/issues/5959); part of the worlds design program [#5955](https://github.com/developing-today/code/issues/5955).
- **Inputs:** the #5999 record (`doc/2026-11-08T10-12-00Z_design_id_capability_diff_semantics/`), §2.1's rule (a class is one sentence of consequence a person can act on), §2.4 (assignment is registry data; unknown fails closed; the table is versioned), and §3 (subsumption is declared, never inferred from a shared prefix, scope, or kind); the shipped table and registry in `pkgs/id/crates/id-caps/src/cap_class.rs` and `pkgs/id/crates/id-caps/src/world_caps.rs` at `origin/main` `016078b81a2`; the provider designs for `kv` (`doc/2026-11-22_design_id1062-kv-provider.md`), `blob` (`doc/2026-11-22_design_id1062-blob-provider.md`), and `notify` (`doc/2026-11-22_design_id1062-notify-provider.md`); the speech contract (`doc/2026-11-20T07-30-11Z_feature_speech-capabilities.md`); the shells design for `client.notify` (`doc/2026-11-10T20-40-00Z_design_more_shells_v1`); the `client.file.read` design (`doc/2026-11-01_design_id789.md`, named in its manifest).
- **Method:** every claim about what a provider does is tied to its manifest entry, its executor, or its provider design at this checkout. Where this record spends a concept another issue owns (final message wording — [#5960](https://github.com/developing-today/code/issues/5960); the comparison — #12921; the reserved filesystem vocabulary — [#5958](https://github.com/developing-today/code/issues/5958) S12), it names the seam and does not redefine that owner's part.

## Decision to make

#12982 shipped the class table exactly as #5999's record wrote it. Sixteen provider names registered after that record carry `CLASS_UNKNOWN`, so every world declaring one of them prompts with "Do something this version of the app cannot describe" — honest, but useless for a decision, and it trains users to wave through the one prompt that must stay meaningful. This pass assigns each of the sixteen a class: an existing class where the consequence genuinely matches, a new class where it does not, and the subsumption edges (if any) each new class carries into #5999 §3's lattice.

## Recommendation — the decision table

| Name              | Proposed class              | New? | Client-side sentence (semantics; final copy is #5960's)      |
| ----------------- | --------------------------- | ---- | ------------------------------------------------------------ |
| `kv.get`          | `world.records`             | no   | "Read the records this world stores." (unchanged)            |
| `blob.get`        | `world.records`             | no   | "Read the records this world stores." (unchanged)            |
| `kv.set`          | `world.storage`             | yes  | "Store data in this world so it persists between visits."    |
| `kv.delete`       | `world.storage`             | yes  | "Store data in this world so it persists between visits."    |
| `blob.put`        | `world.storage`             | yes  | "Store data in this world so it persists between visits."    |
| `blob.delete`     | `world.storage`             | yes  | "Store data in this world so it persists between visits."    |
| `notify`          | `world.notify`              | yes  | "Send notifications to this world's users."                  |
| `device.info`     | `cap.none`                  | no   | _(messageless)_ — under the boundary condition in §3         |
| `form.present`    | `device.form`               | yes  | "Show you forms and collect what you enter in them."         |
| `form.result`     | `device.form`               | yes  | "Show you forms and collect what you enter in them."         |
| `form.cancel`     | `device.form`               | yes  | "Show you forms and collect what you enter in them."         |
| `speech.transcribe` | `device.microphone`       | yes  | "Use this device's microphone to turn what you say into text for this world." |
| `speech.capture`  | `device.microphone.capture` | yes  | "Send this device's microphone audio to this world."         |
| `speech.synthesize` | `device.speaker`          | yes  | "Speak text aloud through this device."                      |
| `client.file.read` | `fs.read.selected`         | yes  | "Read files you choose on this device."                      |
| `client.notify`   | `device.notify`             | yes  | "Show notifications on this device."                         |

Eight new classes; two existing classes gain members (`world.records` takes the two storage reads, `cap.none` takes `device.info`). The only subsumption edge added is `device.microphone ≼ device.microphone.capture`. Adding classes bumps `CAP_CLASS_TABLE_VERSION` from 1 to 2 in the implementation change (§8); no existing class's message or meaning changes, and no reserved class gains a member.

## 1. World storage: `kv.*` and `blob.*`

The two stores are one shape: a bounded, durable, per-world store (kv: 1024 entries / 64 KiB per world, journaled; blob: content-addressed, 4 KiB per blob), inward-facing, `RequiredScope::Join`, included in default ceilings — registered, in their manifests' own words, "like `records.read` and `schedule.*`". #13004 asks two things of this family: whether `kv.get` is the same disclosure as `world.records`, and whether the read/write split matters. The answers are yes, and yes.

**Reads join `world.records`.** `kv.get` and `blob.get` read state the world stores; that state may include other participants' contributions (values and blobs are written by the world's program out of participant interaction), which is exactly §2.2's stated reason for the `world.records` disclosure. The sentence does not change, and under §3.2 rule 1 membership growth inside a class is invisible: a baseline that accepted `world.records` for `records.read` is not silently upgraded by `kv.get`, because no new kind of consequence appears — the user was already told the world can read what it stores.

**Writes are a new class, `world.storage`.** `kv.set`, `kv.delete`, `blob.put`, and `blob.delete` change what the world durably holds. The consequence a person acts on is persistence: what the world learns or derives while you are present is still there — for you and for other participants — when you come back. That is the same shape of consequence that earned `world.schedule` its class (an effect that outlives the moment), and it is not communicated by "Read the records this world stores."

Two alternatives were considered and rejected:

- **Fold the writes into `world.records` with a widened message** ("Read and change the records…"). Baselines in the field accepted that class as read-only. Widening its meaning under them makes write authority arrive silently for every world already holding the class — the silent upgrade #5999's trigger exists to prevent. If the ratifier prefers the fold anyway, §9 prices it: the widened class behaves as a split, and every existing baseline must re-prompt for worlds holding `world.records`.
- **Claim the reserved `fs.sandbox`** ("Read and write files in this world's own storage"). The words are close; the slot is not available. §2.3 holds `fs.sandbox` for the filesystem vocabulary #5958 S12 must express (sandboxed file read+write), and the shipped test `reserved_fs_classes_have_no_members_yet` pins it memberless until that design lands. A bounded key-value store is not a filesystem, and spending the reserved slot here would leave the future filesystem provider with no honest class of its own.

Subsumption: `world.storage` is incomparable with `world.records` — write authority does not cover the read disclosure (a write-only world still reads nothing), and read acceptance does not cover persistence. No other edges.

## 2. `notify` → new class `world.notify`

`notify` commits a `Notification` event carrying world-authored text to the world's event stream, broadcast to every joined participant. Its own design and manifest call it outward-facing — "its effect lands on users who asked it nothing" — and it is excluded from every default ceiling and rate-limited per world for that reason. The disclosure is its own: "Send notifications to this world's users."

It is not `chat.write`. A chat post enters chat history as conversation; a notification is a separate event kind that deliberately does not enter chat history, and it arrives as an interruption rather than as history a user chooses to scroll. A user who accepted a world posting chat has not accepted it sending notifications, and the reverse also fails — the two classes are incomparable, and §3.1 forbids collapsing them for sharing an "outward" flavour.

## 3. `device.info` → `cap.none`, with a boundary condition

`device.info` returns the hosting device's capability manifest (`DeviceManifest`): five affordance objects — `display`, `input`, `sensors`, `audio`, `network` — describing what the device can do (e.g. `{"kind":"pixel","width":128}`, `{"gnss":{"accuracy_m":5}}`). The executor serves a cached copy from the adapter boundary; in its own words it "never enumerates the device itself", and the wire shape has no identity fields: no name, no serial, no stable identifier, no readings. This is `world.info`'s reasoning one level down — §2.2 made the world's own metadata messageless because it is "not something a person decides about", and a static affordance description of the user's own device is the same kind of fact.

**Boundary condition (load-bearing).** The assignment holds only while the manifest carries affordances only. A future manifest version that adds identity-bearing fields voids this assignment and needs a disclosing class. The seam for that checkpoint already exists: `device.info` refuses a manifest version it does not speak (`requested_version`), so a version bump is a designed moment, and this record names the class question as part of that moment's work. This is the assignment the ratifier should contest hardest; §9 lists it first.

## 4. Forms: one mechanism, one class — `device.form`

`form.present` registers a validated form for the host's trusted presenter; the form renders in the user's client. `form.result` is the event that carries what the user entered back to the world. `form.cancel` cancels a pending presentation. The three names are one user-facing mechanism — present without result collects nothing, result without present cannot occur, cancel is lifecycle housekeeping — and the table's own precedents group such families (`schedule.at`/`schedule.cancel`, `players`/`players.list`). One class: "Show you forms and collect what you enter in them." The sentence names both halves because the data collection is the load-bearing consequence: a form's fields are world-supplied, and what a user types into them leaves the client.

`device.form` is incomparable with `device.screen`. `Screen` draws a full-screen document and returns nothing the user enters; accepting "draw a full-screen UI in your client" has not accepted "collect what you enter". §3.1: collapse is never inferred from a shared flavour — here, "both are UI".

## 5. Speech: the split is what leaves the device

The speech contract (`doc/2026-11-20T07-30-11Z_feature_speech-capabilities.md`) draws the family's real line, and the class split follows it:

- **`speech.transcribe` → new class `device.microphone`.** Transcription is on-device by default: "Audio stays on that device; the resulting text enters the existing `chat.say`-shaped or form-value path." What reaches the world is text the user spoke as input. The disclosure: "Use this device's microphone to turn what you say into text for this world."
- **`speech.capture` → new class `device.microphone.capture`.** Capture is the server-side path: microphone frames stream to the world under the `media.stream` consent rules, and are discarded after use unless an artifact is explicitly requested. What leaves the device is the audio itself. The disclosure: "Send this device's microphone audio to this world."
- **Edge: `device.microphone ≼ device.microphone.capture`.** Accepting raw-audio-out covers text-only transcription — a world holding capture can transcribe what it receives — and the reverse never holds. This is the `fs.sandbox ≼ fs.read.arbitrary` shape: the narrower class is silent under the broader, and adding the broader always triggers.

Both microphone caps are sensor caps (`is_sensor_cap`): excluded from default ceilings, explicit grant plus per-use consent, never reached by a wildcard — the same discipline §2.2 records for `device.location`, and the classes match that weight.

Two alternatives rejected:

- **One shared microphone class for both.** Adding `speech.capture` to a world whose baseline accepted transcription would then be silent — hiding the exact escalation (text → raw audio) the lattice exists to surface. Compare too coarsely and real escalations slip through; #5999's decision section names that as the forbidden failure.
- **`speech.transcribe` joining `device.keyboard`.** The `Key` class marks a platform module actively interpreting keystrokes; microphone input under per-use consent is a device-sensor consequence, and §2.2 keeps sensor classes per-sensor (`device.location` is its own). Different device, different data, different consent posture.

**`speech.synthesize` → new class `device.speaker`.** Synthesis is output, not capture: it is not a sensor cap, and it sits in default ceilings. The consequence is still one a person acts on — the world can make the user's device speak, at the world's initiative: "Speak text aloud through this device." Incomparable with `device.screen` and `device.notify`; sound is its own channel.

## 6. Client-side providers: `client.file.read` and `client.notify`

#5960's contract keeps client-side and server-side disclosures in two never-merged groups; these two names are the client table's questions, and neither answer is a server-side class.

**`client.file.read` → new class `fs.read.selected`: "Read files you choose on this device."** The provider is executed by the originating session's client, reads one `path` interpreted relative to a **user-selected root** (never as a host path), returns at most 32 KiB, and requires both the world ceiling/grant and the local client grant. #13004 names the trap directly: this is not `fs.read.arbitrary`, whose sentence is "Read any file this server can read" — different machine (the user's device, not the server), different data (the user's own files), different chooser (the user picks the root). No subsumption runs between `fs.read.selected` and `fs.read.arbitrary` in either direction, and none between `fs.read.selected` and `fs.sandbox` (the world's own storage is not the user's files). The shared `fs.` spelling is a family resemblance; §3.1 states collapse is never inferred from a shared prefix, and this record declares the non-edges explicitly so no future reader infers them.

**`client.notify` → new class `device.notify`: "Show notifications on this device."** The provider's schema carries no content at all — one optional journal-head integer; a notification under it is a re-sync hint, and the shell renders its own local wording. What the world gains is the power to make the user's device present a notification at the world's initiative. That is an attention consequence a person acts on (dismisses, disables, declines), which is why the shells design makes it a grant-gated power excluded from default ceilings with a local user grant required — the registry already treats it as declinable, and the class matches. It is not `world.notify`: no world-authored text, no broadcast, one device, shell wording. It is not `cap.none`: a clock read never interrupts anyone. The `cap.none` alternative is recorded in §9 as the ratifier's second contestable call.

## 7. The lattice after this pass

Declared facts added to #5999 §3.1, transitively, in full:

- `device.microphone ≼ device.microphone.capture`
- `cap.none ≼ c` for the new classes `c`, as for every class (§3.1 already states this for all classes).

Nothing else is ordered. In particular: `world.storage` ∥ `world.records`; `world.notify` ∥ `chat.write`; `device.form` ∥ `device.screen`; `device.speaker` ∥ `device.screen` ∥ `device.notify`; `device.notify` ∥ `world.notify`; `fs.read.selected` ∥ `fs.read.arbitrary` ∥ `fs.sandbox`. The reserved classes (`fs.sandbox`, `fs.read.arbitrary`, `net.out`) gain no members, so `reserved_fs_classes_have_no_members_yet` still passes after implementation.

## 8. The implementation slice (gated on ratification of this record)

One small change, in the shape #13004 specifies:

- `cap_class.rs`: add the eight class constants and table entries (all `reserved: false`, placed with the §2.2 live classes); bump `CAP_CLASS_TABLE_VERSION` from 1 to 2 (classes were added — §2.4's bump rule; the `world.records` member growth alone would not bump). Update `shipped_table_is_exactly_the_5999_table` to the extended table and the version assertion.
- `world_caps.rs`: set the `class:` field on the sixteen manifests per the decision table.
- Tests: `names_without_a_designed_class_carry_unknown_until_assigned` — the list empties; keep the test as the accounting point for future registrations (or convert it to assert no production manifest carries `CLASS_UNKNOWN`). Extend `section_22_assignments_hold_for_every_member` with the sixteen new name→class pairs, including the `device.microphone ≼ device.microphone.capture` pair's manifests.
- Client side: the eight new class ids need sentences in the client message tables #12921's comparison reads; the sentences in the decision table are the §2.1 semantics, and final copy remains #5960's wording review, exactly as for the #5999 classes.

Until that change lands, nothing about current behaviour changes: the sixteen names keep prompting as `unknown`.

## 9. Decisions the ratifier is asked to contest

1. **`device.info` → `cap.none`** (§3) — rests entirely on the affordances-only boundary condition. If the ratifier expects identity-bearing manifest fields, assign a disclosing class now instead.
2. **`client.notify` as a disclosing class** (§6) — the `cap.none` alternative is defensible (no content, no data, shell wording); this record recommends the class because the registry already gates the power behind a local user grant, and a class is what makes that gate explainable.
3. **`world.storage` split vs fold** (§1) — the fold into `world.records` is available only at the price of re-prompting every baseline that holds `world.records` for worlds declaring the write names; the split avoids that cost and keeps "read" meaning read.

## References

- [#13004](https://github.com/developing-today/code/issues/13004) — this design pass's issue; the gap, the sixteen names, the landing shape.
- [#5999](https://github.com/developing-today/code/issues/5999) — the class table, collapse rules, and subsumption lattice this record extends (`doc/2026-11-08T10-12-00Z_design_id_capability_diff_semantics/`).
- [#12982](https://github.com/developing-today/code/issues/12982) — shipped the table and the tripwire test naming these sixteen.
- [#12921](https://github.com/developing-today/code/issues/12921) — the comparison that consumes these classes.
- [#5959](https://github.com/developing-today/code/issues/5959) — the change notices; [#5955](https://github.com/developing-today/code/issues/5955) — the worlds design program; [#5958](https://github.com/developing-today/code/issues/5958) — the capability model, S12's reserved filesystem vocabulary; [#5960](https://github.com/developing-today/code/issues/5960) — permission UX and final wording.
- `pkgs/id/crates/id-caps/src/cap_class.rs` — the shipped table, `CAP_CLASS_TABLE_VERSION`, the tripwire test.
- `pkgs/id/crates/id-caps/src/world_caps.rs` — the sixteen manifests (`kv.*` :718–775, `notify` :799, `device.info` :835, `form.*` :844–870, `speech.*` :894–920, `CLIENT_FILE_READ_MANIFEST`, `CLIENT_NOTIFY_MANIFEST`), `is_sensor_cap`, `DeviceManifest`.
- `pkgs/id/crates/id-worlds/src/world/actor/execution.rs` — `execute_device_info` (the manifest is cached; the device is never enumerated), `execute_form_present`.
- Provider designs: `doc/2026-11-22_design_id1062-kv-provider.md`, `doc/2026-11-22_design_id1062-blob-provider.md`, `doc/2026-11-22_design_id1062-notify-provider.md`, `doc/2026-11-20T07-30-11Z_feature_speech-capabilities.md`, `doc/2026-11-10T20-40-00Z_design_more_shells_v1`, `doc/2026-11-01_design_id789.md`.
