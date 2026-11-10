# Note: parts of this design superseded

This design is superseded in part by [events, groups, delegation, removal, and history](../2026-10-11T03-44-33Z_design_groups_events_delegation/2026-10-11T03-44-33Z_design_groups_events_delegation.md). The replacement is not one-to-one. It changes direction in several places.

Changed:

- The author-tag rule ("a tag's author must match its subject's author") is withdrawn. Anyone may tag anything, and claims stay attributed to their signing key.
- The signing basis (serde_json re-serialization, SHA-256 `hash`) is replaced by deterministic CBOR and BLAKE3 event IDs.
- `after` is a required frontier, not an optional list of applied items. It is used for strong removal.
- Strong removal, previously unimplemented here, is specified in the new document (§6).
- Pull pages are sized by bytes as well as by count (§11.4).

Still valid as written: per-author chains with `prev` and `seq`, refusal of forks, the move from HTTP to iroh ALPNs, and the `friend_remove` and `home_declared` kinds. Those two kinds are to be ported to events in the new model.
