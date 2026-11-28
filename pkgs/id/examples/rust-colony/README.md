# rust-colony

Pure Rust domain for the Colony hauling world (#13605), the Rust twin of the
Roc colony guest. A worker hauls ore from a 4x2 board to a stockpile.

The oracle is `colony/two-deliveries` in
`pkgs/id/book/research/evidence/app-vectors.json` at commit `b9d8177a6cf`.
Expected states in the tests are literal projections of that vector.

Not yet done: the Roc guest, wasm build, host wiring, book chapter, and a
Fleet remote run.

Run: `cargo test` in this directory.
