# Working on mcpx source

These are for changing mcpx, not using it — the user guide is one level up in
`../AGENTS.md`.

- **Never `go test ./...` locally.** It is CI's command. Run the package you
  changed: `go test ./internal/<pkg> -count=1`. e2e is `./internal/e2e`; name
  the tests (`-run 'TestThat'`) rather than running the package.
- **A bug gets a failing-first test in the same PR**, covering its
  permutations, not just the one input that tripped it.
- **A bug that is a class of bug gets a guard**: iterate the registry or table
  so the test fails for every member, present and future.
- **Flakes:** `-race -shuffle=on -count=5` on tests you changed. Never retry
  to get green; fix it or quarantine with an issue and an expiry.
- **Read CI before merging.** Reproduce a failure with the package and test
  name from the log, `-count=1 -v`.
- **Docs you add are part of the book.** A new markdown file needs an entry in
  `../docs/compendium.manifest.json` or `go test ./internal/settings` fails.

Details, commands and per-package flake notes: `../docs/testing.md`.
