# s11-relay spike certificates

This directory holds the TLS material for the s11-relay spike (see
`../RESULTS.md` for the runs it fed).

**Private keys are never committed.** The tracked files are the public
certificates (`gen/*.pem`, `gen/*.der`), the leaf extension template
(`gen/leaf.ext`), and `generate-certs.sh`. The `*.key`, `*.csr`, and
`*.srl` files are produced locally and are covered by this directory's
`.gitignore`.

To (re)generate everything, with the topology the spike expects
(CA 1 signs leafA/leafB, CA 2 signs leafC, leafB installed as the live
leaf, all leaves `CN=localhost` with SAN `DNS:localhost, IP:127.0.0.1`,
EC P-256 keys):

```sh
./generate-certs.sh
```

The configs under `../config/` (`tls-manual.toml`, `tls-reloading.toml`)
read `live/leaf.pem` / `live/leaf.key`, which the script installs.

History note: the original spike run committed its throwaway keys
(removed under developing-today/code#12150). They only ever signed this
spike's test chains — no environment trusts CA 1 or CA 2 — and the
certificates in git history are expired. Rewriting history to purge
them was left as a separate human decision.
