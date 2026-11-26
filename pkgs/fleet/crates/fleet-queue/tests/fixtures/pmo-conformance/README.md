# PM orchestration contract conformance fixtures (CEN-07)

Golden fixtures for [#9437](https://github.com/developing-today/code/issues/9437),
per the design record
`doc/2026-11-25T20-54-29Z_design_cen07_adapter_conformance_ci/`. The
test target `tests/pmo_contract_conformance.rs` feeds them to the
code that serves each contract on main. The venue is
`.github/workflows/pmo-contract-conformance.yml` (nightly plus manual).

`manifest.json` lists every family. A `landed` family names the exact
tests that cover it; a `gated` family names its gating issue and has
no tests, so it is reported as gated and never counted as passing.
Moving a family to `landed` is a fixture-and-manifest change in the
same PR that lands its implementation.

| Family | Directory | Contract owner |
| --- | --- | --- |
| `command` | `command/` | DST-01 / CLI-01 (#9451) |
| `acceptance` | `acceptance/` | DSP-01 (#9445) |
| `selector` | `selector/` | PRI-01 (#9478) |
| `authority.managed_section` | `authority/managed_section/` | AUTH-04 (#9431) |
| `receipt` | gated on REL-03 (#9513) | |
| `cursor` | gated on HYD-06 (#9508) | |
| `authority.field_ownership` | gated on AUTH-01 (#9429) | |

The fixtures are synthetic: no credentials, tokens, real comment
bodies or production identifiers. Expected digests were computed
outside the Rust code, from the framing rule in `contracts.rs`:
`"sha256:" + hex(SHA256(domain || 0x00 || JCS(dto)))`.

A passing run is contract evidence only. It says nothing about
deployed behavior, and conformance is claimed per family only from a
green run of the venue whose head contains the change.
