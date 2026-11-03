# C004: F12.3 roles v2 builder and moderator scopes with journaled audit — D1-021

[Main reference](../main.md#c004)

## Relevant transcript excerpts

<!-- workflow:excerpt C004 29 33 -->

[Transcript lines 29-33](../transcript.txt#L29-L33)

```text
- [ ] **D1-021 — F12.3 Roles v2: journaled builder and moderator scopes with audit** (M, design)
  Journaled grants for builder (install modules) and moderator (revoke participants) as scope bits, with an audit record for each use; builder never gets network/file/process authority.
  - Status: not started. WorldScopes checks only CHAT/HISTORY/JOIN-style bits (pkgs/id/src/world.rs:201-204); `grep -rn 'BUILDER\|MODERATOR' pkgs/id/src` finds nothing.
  - Source: .opencode/plans/phase-12-platform-flexibility.md:33; doc/2026-10-17T08-00-00Z_feature_world_roles_directory/
  - Prerequisites: D1-019 F12.1 Declarative world manifest (world.toml) with apply / --dry-run / diff (in this list)
```

<!-- workflow:excerpt C004 149 156 -->

[Transcript lines 149-156](../transcript.txt#L149-L156)

```text
[Comment by @dezren39 at 2026-10-27T07:45:46Z; https://github.com/developing-today/code/issues/534#issuecomment-6436491422]
🤖 **Agent note** · cross-reference from Phase 10 validation (#1077 slice 5, queue task 1258)

Mapping the validation's Phase 12 remainders onto this umbrella so nothing is double-tracked:

- **F12.3 Roles v2** = this issue's **D1-021** (design, prereq F12.1) — tracked here; no separate issue filed.
- **F12.5 First new providers** (incl. `notify`, the 'outbound notifications' remainder) = **D1-023** (design, prereqs F12.4 + F12.6) — tracked here; no separate issue filed.
- **F12.2 Forking and templates** was **missing from the D1 checklist** and tracked nowhere — now filed as dedicated issue #1354 (spec + exit criteria from the plan, dependency F12.1 noted). Add it to the checklist or treat #1354 as its home.
```

## Normalized purpose

Intake for F12.3 journaled builder/moderator scopes and per-use audit. Reconciles the merged design finding (role bits and journal records exist; live install still requires the admin token) and open remainder #835 with D1-019 ordering.

## Initial thoughts

- Frame the remaining design as the live wire authorization path, not role bits: scope bits and journals already exist.
- Carry the #835 link and the F12.1 prerequisite so S2 does not re-file or re-order them.

## Inferred suggestions

- Inference: confirm at S1 whether any install/revoke path other than the documented session frame still bypasses builder identity.

## Stage results

Pending: research, synthesis, design, planning and implementation have not run.
