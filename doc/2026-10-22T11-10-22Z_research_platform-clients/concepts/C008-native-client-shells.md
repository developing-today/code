# C008: Native client shells

[Main reference](../main.md#c008)

## Relevant transcript excerpts

<!-- workflow:excerpt C008 101 102 -->

[Transcript lines 101-102](../transcript.txt#L101-L102)

```text
- **Screen contract:** one structured grid; web, SSH and captured buffers
  present the same structure. No guest HTML/CSS/escape sequences.
```

<!-- workflow:excerpt C008 541 541 -->

[Transcript lines 541-541](../transcript.txt#L541-L541)

```text
+ manifest with **no `world.rs` edits**; ≥3 guest languages pass conformance;
```

<!-- workflow:excerpt C008 552 553 -->

[Transcript lines 552-553](../transcript.txt#L552-L553)

```text
→ audit lines, replayable); F12.7 ABI spec + conformance suite
(`plaza_abi_version`); F12.8 second/third guest SDKs (Rust + TS/Zig); F12.9
```

<!-- workflow:excerpt C008 754 762 -->

[Transcript lines 754-762](../transcript.txt#L754-L762)

```text
## 15. Beyond the terminal: native clients and client-side capabilities (long-term)

The direction set for the platform after the terminal story settles:

- **Native client shells** — game engines (Bevy), Qt, C#/.NET, Unity,
  Electron, Windows desktop apps — implementing the same screen/input
  contract for things a terminal cannot do even with kitty protocol and high
  fps: 3D, audio, native widgets. The portable surface contract and the ABI
  conformance suite (F12.7) are what make a shell swappable.
```

<!-- workflow:excerpt C008 774 777 -->

[Transcript lines 774-777](../transcript.txt#L774-L777)

```text
- **Placement:** this is post-Phase-12 material. F12.7 (ABI conformance),
  F12.14 (portals) and F12.12 (many worlds per node) are the stepping stones;
  a "client shells + client capability providers" sketch belongs in a future
  Phase 13 plan.
```

## Normalized purpose

Takes native client shells (Bevy, Qt, .NET, Unity, Electron, Windows desktop) implementing the same screen/input contract for what a terminal cannot do, made swappable by the portable surface contract and the F12.7 ABI conformance suite.

## Initial thoughts

- Native shells (Bevy, Qt, .NET, Unity, Electron, Windows) implement the same screen/input contract for 3D, audio and native widgets (L758-L762).
- The portable surface contract (L101-L102) and the F12.7 conformance suite (L552-L553) make a shell swappable; no plaza_abi_version exists in pkgs/id yet.
- This is post-Phase-12 material: F12.7, F12.14 and F12.12 are stepping stones, and it belongs in a Phase 13 plan (L774-L777; #564). #538 tracks D1-054.

## Inferred suggestions

- Inference: A native shell needs the input contract too; kitty-style key events (the presentation tiers concept) are a natural baseline for native input.

## Stage results

Pending: research, synthesis, design, planning and implementation have not run.
