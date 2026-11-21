# Independent review: generator and operations proposals

## Final review and agreement

Final recheck of GEN-11 in `designs/generators.md:160–166` and JSON GEN-11 at `proposals/generators-ops.json:254` confirms the previous API mismatch is **fixed**: both use `--profile <approved-versioned-profile>` for `fleet key derive`; JSON no longer advertises arbitrary `--params`.

GEN-11's profile interface agrees with its fail-closed gate: KDF capability remains unsupported until a reviewed versioned profile records numeric min/max memory, time/passes, parallelism, salt length, output length, and target resource budget; parameters are validated before allocation. The per-purpose digest allowlists, SHA-256 fingerprint/DER-integrity default with explicit protocol exceptions, weak-digest refusal, Argon2id candidate, official vectors/independent oracle, protected stdin/secret sink, no secret argv/log/output, and root/equality-index exclusions are consistent across design and proposal.

The prior OPS-10 legacy BWS finding remains fixed. Design and JSON require trusted project/native-ID authority, exclusion/proof against old broker/runtime list/get/cache paths before first copy, no caller `DataClass`/sidecar authority, an original broker-compatible synthetic negative test, and no project/grant action. The cited current code confirms the legacy risk: `secrets_broker.rs:190–260` refreshes every visible item into cache; `fleet-secrets/src/lib.rs:517–604` resolves accessible BWS names/IDs; `fleet/src/secrets.rs:110–127` prints the resolved value. No such actions were performed.

The other previously verified owned-file checks remain resolved: 22 unique JSON objects with the exact common key set, publisher-allowed kinds, valid lifecycle/urgency, per-body priority/queue projection plus distinct importance rationale, and an acyclic owned DAG. OPS-07 depends on OPS-09; OPS-09 consumes OPS-08/CERT-13 and does not depend on OPS-07. ROOT-02/CERT-01 hard manifest prerequisites and OPS-08 ROOT-03/ROOT-04/CERT-06 producer dependencies are explicit. OPS-10 uses ROOT-05/09/10; OPS-03/04 is acyclic with REC-03/04 soft dependencies. The combined-manifest projection and strict-kind rules remain documented in `operations-evidence.md:65`.

**Author-reviewer agreement: the three owned design/proposal files are closed.** This final recheck found no remaining issue within the owned files.

## Separate package/global blockers

`operations-evidence.md:65` records external blockers: malformed `proposals/idea-seeds.json`, publisher syntax error at line 913, and projection/normalization required for other domain arrays. These are outside the owned subgraph; no package/global publisher invocation was made and no global readiness is claimed.

No Git, queue, GitHub, production, or cryptographic command was executed.
