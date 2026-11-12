<!-- Moved verbatim from AGENTS.md by task 15116 (issue #8430) so the root rulebook fits its 8,000-byte budget. -->

## Operator script argument contract (#3352)

Every argument is accounted for before anything happens. An operator
script (deploys, installs, `bin/` tools, `pm-stack` scripts and
exporters, Go tools) declares its whole command line — boolean flags,
value options, positionals — and parses strictly against that
declaration, before any subprocess, network, file write, lock, or
deploy step. Anything unaccounted for is a usage error naming the
offending argument, with a non-zero exit (2 where the tool has no
established usage code). Unknown flag, stray positional, and typo'd
flag are the same event; `--opt=value` either works identically to
`--opt value` or is refused, never a silent fall-through to a
different operation; a present-but-valueless option is a usage error,
never the default; a parsed safety flag (`--dry-run`, `--status`) is
honoured on every code path. Parsing is per-tool (no shared library);
new operator scripts copy the strict pattern from a covered sibling
(`provider-catalog-sync.mjs`, `append-usage-snapshots.ts`) and state
the contract in their header comment. Deliberate exceptions are
named, with their reason, in the #3353 closing inventory.

