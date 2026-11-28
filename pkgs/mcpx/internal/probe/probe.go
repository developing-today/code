// Package probe holds the mcpx runtime capability probes: the one-line
// TypeScript programs whose per-runtime results are the evidence base
// for the feature-comparison matrix in docs/runtimes.md (issue #2129).
//
// The probes are deliberately tiny and dependency-free apart from the
// package-resolution fixture the test builds. probe_test.go runs every
// probe under deno, bun and node with the same look-up-and-skip pattern
// as the e2e runtime tests (testsupport.RequireBinary, so CI's
// MCPX_TEST_REQUIRE turns a missing runtime into a failure, not a
// silent skip). If a runtime's behaviour changes, the test fails and
// the docs matrix must be updated in the same change -- that assertion
// is the guard against the matrix going stale.
package probe

// Probe is one capability check: a one-line TypeScript file, the token
// its stdout must contain, and the exit code it must finish with.
type Probe struct {
	Name     string
	File     string
	Source   string
	Want     string
	WantCode int
}

// RunProbes are the five one-line probes run under every runtime.
var RunProbes = []Probe{
	{
		Name:   "env",
		File:   "env.ts",
		Source: `console.log("probe env=" + ((globalThis as any).Deno?.env?.get?.("MCPX_PROBE_TOKEN") ?? (globalThis as any).process?.env?.MCPX_PROBE_TOKEN ?? "missing"));`,
		Want:   "probe env=probe-token",
	},
	{
		Name:   "fetch",
		File:   "fetch.ts",
		Source: `console.log("probe fetch=" + typeof fetch);`,
		Want:   "probe fetch=function",
	},
	{
		Name:   "fs",
		File:   "fs.ts",
		Source: `const { readFileSync } = await import("node:fs"); console.log("probe fs=" + readFileSync(new URL("./data.txt", import.meta.url), "utf8").trim());`,
		Want:   "probe fs=probe-data",
	},
	{
		Name:   "json",
		File:   "json.ts",
		Source: `console.log("probe json=" + JSON.stringify(JSON.parse('{"a":1}')));`,
		Want:   `probe json={"a":1}`,
	},
	{
		Name:     "exit",
		File:     "exit.ts",
		Source:   `console.log("probe exit=now"); const g = globalThis as any; if (g.Deno?.exit) g.Deno.exit(3); else g.process.exit(3);`,
		Want:     "probe exit=now",
		WantCode: 3,
	},
}

// PackageProbe imports a bare specifier resolved from a node_modules
// directory the test creates next to the probe file.
var PackageProbe = Probe{
	Name:   "package",
	File:   "pkg.ts",
	Source: `import { greeting } from "probe-pkg"; console.log("probe pkg=" + greeting);`,
	Want:   "probe pkg=hello-from-probe-pkg",
}

// TypeCheckGood is well-typed; TypeCheckBad assigns a string to a
// number. A runtime with a static type checker must accept the first
// and reject the second before running either.
const (
	TypeCheckGood = "typecheck-good.ts"
	TypeCheckBad  = "typecheck-bad.ts"

	TypeCheckGoodSource = `const n: number = 41; console.log("probe type=" + (n + 1));`
	TypeCheckBadSource  = `const n: number = "not a number"; console.log("probe type=" + n);`
)

// TypeChecks reports whether each runtime statically type-checks a
// script before running it, as asserted by the type-check probes:
// deno does (deno check), bun and node strip types without checking.
var TypeChecks = map[string]bool{
	"deno": true,
	"bun":  false,
	"node": false,
}

// Runtimes is the probe matrix's column set, in docs order.
var Runtimes = []string{"deno", "bun", "node"}

// RunArgs is the argv (after the binary) that runs a script file under
// the named runtime, mirroring the kind table in docs/runtimes.md.
// Probes run unsandboxed (--allow-all on deno) because they measure
// runtime capability, not mcpx's permission enforcement.
func RunArgs(runtime, file string) []string {
	switch runtime {
	case "deno":
		return []string{"run", "--quiet", "--no-check", "--allow-all", file}
	case "bun":
		return []string{"run", file}
	case "node":
		return []string{"--no-warnings", "--experimental-transform-types", file}
	}
	return nil
}
