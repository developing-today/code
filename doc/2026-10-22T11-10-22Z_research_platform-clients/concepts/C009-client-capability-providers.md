# C009: Client-side capability providers

[Main reference](../main.md#c009)

## Relevant transcript excerpts

<!-- workflow:excerpt C009 540 541 -->

[Transcript lines 540-541](../transcript.txt#L540-L541)

```text
Whole-phase exit: <5 min empty-dir→joinable world; a new capability = provider
+ manifest with **no `world.rs` edits**; ≥3 guest languages pass conformance;
```

<!-- workflow:excerpt C009 544 545 -->

[Transcript lines 544-545](../transcript.txt#L544-L545)

```text
redraw bandwidth <10% of a full frame; no new authority path outside named
ceilinged providers.
```

<!-- workflow:excerpt C009 548 549 -->

[Transcript lines 548-549](../transcript.txt#L548-L549)

```text
forking and templates; F12.3 roles v2 (builder, moderator); F12.4 provider
registry (test provider added without editing `world.rs`); F12.5 first
```

<!-- workflow:excerpt C009 754 756 -->

[Transcript lines 754-756](../transcript.txt#L754-L756)

```text
## 15. Beyond the terminal: native clients and client-side capabilities (long-term)

The direction set for the platform after the terminal story settles:
```

<!-- workflow:excerpt C009 763 767 -->

[Transcript lines 763-767](../transcript.txt#L763-L767)

```text
- **Client-side capabilities** — some apps need the client's file system,
  GPU, and network beyond the p2p link. This extends the capability model
  (scopes/roles/ceilings; F12.4 provider registry) to the client side: named,
  granted, ceilinged providers for *local* resources, keeping the Phase 12
  rule "no new authority path outside named ceilinged providers."
```

<!-- workflow:excerpt C009 774 777 -->

[Transcript lines 774-777](../transcript.txt#L774-L777)

```text
- **Placement:** this is post-Phase-12 material. F12.7 (ABI conformance),
  F12.14 (portals) and F12.12 (many worlds per node) are the stepping stones;
  a "client shells + client capability providers" sketch belongs in a future
  Phase 13 plan.
```

## Normalized purpose

Takes client-side capabilities: apps that need the client's file system, GPU and network get them through named, granted, ceilinged providers on the client, extending scopes, roles, ceilings and the F12.4 provider registry without a new authority path.

## Initial thoughts

- Some apps need the client's file system, GPU and network beyond the p2p link (L763-L764).
- Extend scopes, roles, ceilings and the F12.4 provider registry (L548-L549) to named, granted, ceilinged providers for local resources (L764-L767).
- Keep the Phase 12 rule: no new authority path outside named ceilinged providers (L544-L545, L766-L767). #538 tracks D1-055 after F12.4.

## Inferred suggestions

- Inference: Client-side providers invert the trust direction: the world asks the client for authority, so consent and ceilings live on the client.

## Stage results

Pending: research, synthesis, design, planning and implementation have not run.
