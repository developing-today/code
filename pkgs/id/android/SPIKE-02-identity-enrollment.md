# Slice 2 spike — identity and enrollment (`#7938` child `#15842`)

Status: **spike recorded, doc-derived; on-device confirmation still owed.**
No emulator or physical device was available to this lane, so every
API-level claim below is labelled honestly as **doc-derived** and the
matrix names the exact runs that would promote it. No production key
handling and no real user keys are involved anywhere in this slice.

## 1. Per-API-level Ed25519 Keystore/StrongBox verdict

Sources: AOSP Android Keystore documentation and contemporaneous
reports (notably the Android 13 Keystore-Ed25519 report placing
`AndroidKeyStoreEdEC*` support on the Android 13 release branch, i.e.
API 33). Executable form: `direct_keystore_ed25519_supported` in
`pkgs/id/crates/id-client/src/enrollment.rs`.

| API level          | Direct Keystore Ed25519             | StrongBox Ed25519                         | Spike decision                                                                                                      | Provenance  |
| ------------------ | ----------------------------------- | ----------------------------------------- | ------------------------------------------------------------------------------------------------------------------- | ----------- |
| 21–32              | No (no doc source reports it)       | No                                        | **Envelope fallback mandatory**                                                                                     | Doc-derived |
| 33+ (TEE Keystore) | Reported yes                        | —                                         | **Envelope fallback default**; direct only after a real-device run proves algorithm + hardware level on that device | Doc-derived |
| 33+ (StrongBox)    | Device-dependent; frequently absent | **Assume absent until proven per-device** | Envelope with StrongBox-wrapped AES key where present, TEE wrap otherwise; report the actual level                  | Doc-derived |

Fallback decision: **the encrypted-key-envelope path is the default
on every API level.** Direct hardware Ed25519 (`Ed25519` via
`KeyGenParameterSpec`) is attempted only where a real-device spike
has verified it, per design §2.1 rule 1. The `recommended_posture`
function encodes exactly this: it never returns
`DirectHardwareEd25519` on doc evidence alone.

Owed runs (promote doc-derived → emulator/device-proven): on
representative API levels (21-class, 29-class, 33, 34/35) verify
algorithm support, hardware level (TEE vs StrongBox vs software),
lock-screen-reset behavior, process-death behavior, and backup
behavior — the §2.1 confirmation list.

## 2. Encrypted-key-envelope fallback (prototype)

- ID Ed25519 key lives in an encrypted envelope in app-private storage.
- Envelope wrap key: AES, generated in Keystore; request StrongBox
  when present, otherwise platform Keystore (TEE), otherwise
  software-backed with explicit user-visible disclosure.
- Plaintext exists only inside `SecretBuffer` (zeroized on drop, short
  lifetime, one signing operation); the Kotlin↔Rust boundary passes
  sign operations or a short-lived handle, never a long-lived seed.
- `WrappingKeyLevel` is reported honestly end-to-end: software backing
  is never labelled StrongBox, and permitting it is an explicit setup
  choice, not a silent downgrade.

## 3. Backup exclusions

Key envelopes, recovery secrets, and delegation signing material are
excluded from Android cloud backup, device-to-device transfer, and
auto-backup (`dataExtractionRules` / manifest exclusion; Keystore keys
are non-exportable and never backed up regardless). Pinned as code in
`BACKUP_EXCLUDED_SUFFIXES` (`.keyenvelope`, `.idkey`,
`.recoverycodes`, `.delegation`). Local-journal backup/export is a
separate user choice and never implies key recovery.

## 4. Biometric invalidation

`BiometricPrompt` gates use of the local wrapping key (unlock
convenience only — never identity, recovery, or grant proof). Key
built with user-authentication required and biometric-enrollment
invalidation on: a changed biometric set, lost device, cleared app
data, or invalidated Keystore key surfaces as
**key-unavailable / re-enrollment state**, resolved through the #5684
recovery path — never by claiming biometrics or Google sign-in can
recover the ID.

## 5. Key zeroization

Rust side: `SecretBuffer` zeroizes on drop and on explicit `zeroize`;
plaintext lifetime is one signing operation. Kotlin side (shell slice):
zero-fill key byte arrays after use; no seed in logs, bundles, or
saved instance state.

## 6. `#5684` recovery handoff (owned by `#5684`, adopted here)

`#5684` is CLOSED with the parent synthesis record
(`doc/2026-11-23T20-55-00Z_design_id5684_identity_login_synthesis/`,
PR `#13715`, Felix reviews) over the seven closed child records. The
mobile boundary adopted for this slice:

- Enrollment creates/imports the ID per the selected #5684 custody
  variant, then creates the phone's **scoped, device-bound delegated
  identity** under the user's ceiling (`#5903` D5); each device is
  independently revocable and never raises the ceiling.
- The phone is never silently made the user's only recovery copy.
- **Loss of every authorized recovery path can make the identity
  unrecoverable** — onboarding states this plainly (variant-(c)
  security-code wording from the synthesis record).
- No competing Android-side recovery protocol is invented here.

## 7. Design-invariant check (`#5780` §10)

No invariant is contradicted: no root-key upload (envelope stays
app-private, backup-excluded); biometrics are a second gate at most,
never the ID grant; journal truth untouched; client-only boundary held
(`android_dependency_boundary` still green — this slice adds no
dependency); protocol reuse and lifecycle honesty unaffected.
