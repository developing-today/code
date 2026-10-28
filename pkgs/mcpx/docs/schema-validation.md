# Schema validation (`internal/schemaval`)

mcpx validates tool and elicitation schemas with the `internal/schemaval`
package: a JSON Schema **2020-12** validator (wrapping
`github.com/santhosh-tekuri/jsonschema/v6`, its only import site in this
module) with an explicit dialect policy. It exists so Phases 2–3 of the
schemaval design can compile schemas at list time and enforce them at
call time; the core package itself changes no call paths.

Division of labour: `internal/mcpspec` validates MCP **protocol frames**
and is test-only; `schemaval` validates **tool/elicitation schemas** and
is the validator the binary carries.

## Dialect policy

| Declared `$schema`                                       | Result                                                                                      |
| -------------------------------------------------------- | ------------------------------------------------------------------------------------------- |
| _(absent)_                                               | Treated as 2020-12                                                                          |
| 2020-12 (`https://json-schema.org/draft/2020-12/schema`) | Supported                                                                                   |
| 2019-09 (`https://json-schema.org/draft/2019-09/schema`) | Supported                                                                                   |
| draft-07 (`http://json-schema.org/draft-07/schema#`)     | Supported                                                                                   |
| draft-06 (`http://json-schema.org/draft-06/schema#`)     | Supported                                                                                   |
| draft-04 (`http://json-schema.org/draft-04/schema#`)     | Supported                                                                                   |
| anything else                                            | Compile fails with a typed `UnsupportedDialectError`, distinct from a generic compile error |

## Rules

- **No network, ever.** The compiler's loader refuses every URL. A
  schema may reference only itself (internal `$ref`s into its own
  `$defs` and the like, or its own `$id`). Any other reference —
  internal-but-missing or external — fails compilation with a typed
  `UnresolvedRefError`. This is the `json-schema-no-network-ref`
  conformance rule, enforced at compile time.
- **Size bound.** A schema document larger than **1 MiB**
  (1,048,576 bytes) fails compilation with a typed
  `SchemaTooLargeError`. The largest legitimate schema documents in
  this repo (the vendored protocol schemas) are under 200 KiB.
- **Formats are annotations, not assertions, in v1.** A value that
  violates a `format` keyword still validates; format checking may be
  turned on deliberately in a later phase, never by accident.
- **Integer fidelity.** Instances are decoded without going through
  `float64`, so integers past 2^53 validate exactly (bounds,
  `multipleOf`, enum membership).
- **Errors are flat and path-carrying.** A failed validation returns
  one line per problem in the form `$.count: got string, want integer`,
  so a caller can show a tool user exactly what to fix.
