# `acceptance` family

Contract owner: DSP-01 (#9445). Fleet side:
`fleet-queue/src/work/operations.rs` (`AcceptanceLedger`). PM Stack
side: `pm-stack/projector/src/operation-forwarder.ts`, read only here.

Each delivery is a signed `issue_comment` payload, whether the
projector applied it, and the envelope the projector forwards (or
`null` when the comment carries no command). The Rust side maps the
forwarded envelope to `operations::Command` and presents it to the
ledger in order. Acceptance is keyed by the semantic key, never by
the delivery id; a projection ACK alone is never acceptance.

`key_template` and `forwarded_envelope_fields` must match the
projector source exactly. If the projector changes either, this
family fails until the fixture and the Rust mapping change together.

`known_mismatches` records C1 and C2 from the design record. They are
not fixed here; REL-03 and DSP-02 own them.
