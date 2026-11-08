package preflight_test

import (
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/preflight"
)

// The shape `deno info --json` (deno 2.9.7) actually emits for a script
// importing two missing files and one present one: the graph exits 0, and
// each missing module is an entry with an error and no kind, joined back to
// the as-written specifier and its span through the importer's dependency.
const denoInfoFixture = `{
  "version": 1,
  "roots": ["file:///work/main.ts"],
  "modules": [
    {
      "kind": "esm",
      "specifier": "file:///work/main.ts",
      "local": "/work/main.ts",
      "dependencies": [
        {"specifier": "./missing-a.ts", "code": {"specifier": "file:///work/missing-a.ts", "span": {"start": {"line": 0, "character": 16}, "end": {"line": 0, "character": 32}}}},
        {"specifier": "./present.ts", "code": {"specifier": "file:///work/present.ts", "span": {"start": {"line": 1, "character": 16}, "end": {"line": 1, "character": 30}}}},
        {"specifier": "./missing-b.ts", "code": {"specifier": "file:///work/missing-b.ts", "span": {"start": {"line": 2, "character": 16}, "end": {"line": 2, "character": 32}}}}
      ]
    },
    {"specifier": "file:///work/missing-a.ts", "error": "Module not found \"file:///work/missing-a.ts\"."},
    {"specifier": "file:///work/missing-b.ts", "error": "Module not found \"file:///work/missing-b.ts\"."},
    {"kind": "esm", "specifier": "file:///work/present.ts", "local": "/work/present.ts"}
  ],
  "redirects": {},
  "npmPackages": {}
}`

func TestParseGraphFindsEveryMissingSpecifier(t *testing.T) {
	g, err := preflight.ParseGraph([]byte(denoInfoFixture))
	if err != nil {
		t.Fatal(err)
	}
	if len(g.Missing) != 2 {
		t.Fatalf("both missing specifiers should be reported, got %d: %+v", len(g.Missing), g.Missing)
	}
	if g.Missing[0].Specifier != "./missing-a.ts" || g.Missing[1].Specifier != "./missing-b.ts" {
		t.Errorf("specifiers as written: %+v", g.Missing)
	}
	span := g.Missing[0].Span
	if span == nil || span.Path != "/work/main.ts" || span.Line != 1 || span.Character != 17 {
		t.Errorf("span should locate the specifier in the importer, 1-based: %+v", span)
	}
	if len(g.LocalFiles) != 2 {
		t.Errorf("local files for the dynamic scan: %v", g.LocalFiles)
	}
}

func TestReportGraphReportsAllMissingInOnePass(t *testing.T) {
	g, err := preflight.ParseGraph([]byte(denoInfoFixture))
	if err != nil {
		t.Fatal(err)
	}
	r := preflight.ReportGraph(g, nil, preflight.DynamicWarn)
	if r.Err() == nil {
		t.Fatal("unresolved imports should be fatal in report mode")
	}
	for _, want := range []string{"./missing-a.ts", "./missing-b.ts", "/work/main.ts:1:17"} {
		if !strings.Contains(r.Err().Error(), want) {
			t.Errorf("the one-pass report should name %s: %v", want, r.Err())
		}
	}
}

func TestComputedDynamicImportProducesAWarning(t *testing.T) {
	src := "const name = 'x';\nconst m = await import(\"./\" + name);\nconst r = require(someVar);\n"
	dyn := preflight.ScanDynamicImports("main.ts", src)
	if len(dyn) != 2 {
		t.Fatalf("both computed edges should be found, got %+v", dyn)
	}
	if dyn[0].Kind != "import" || dyn[0].Line != 2 {
		t.Errorf("first: %+v", dyn[0])
	}
	if dyn[1].Kind != "require" || dyn[1].Line != 3 {
		t.Errorf("second: %+v", dyn[1])
	}
	r := preflight.ReportGraph(&preflight.Graph{}, dyn, preflight.DynamicWarn)
	if r.Err() != nil {
		t.Errorf("warn is the default: a computed import should not stop the run: %v", r.Err())
	}
	if len(r.Warnings()) != 2 {
		t.Errorf("but it should be said, twice: %v", r.Warnings())
	}
	r = preflight.ReportGraph(&preflight.Graph{}, dyn, preflight.DynamicFail)
	if r.Err() == nil {
		t.Error("fail mode should make a computed import fatal")
	}
	r = preflight.ReportGraph(&preflight.Graph{}, dyn, preflight.DynamicIgnore)
	if len(r.Problems) != 0 {
		t.Errorf("ignore means ignore: %v", r.Problems)
	}
}

func TestLiteralDynamicImportIsNotComputed(t *testing.T) {
	src := "const m = await import(\"./literal.ts\");\nconst n = await import(`./plain.ts`);\n"
	if dyn := preflight.ScanDynamicImports("a.ts", src); len(dyn) != 0 {
		t.Errorf("literal dynamic imports are static edges, not warnings: %+v", dyn)
	}
}

func TestImportsOffIsANoOp(t *testing.T) {
	r := preflight.CheckImports(context.Background(), "/bin/deno", "/nope.ts", preflight.ImportsOff, preflight.DynamicWarn, 0)
	if len(r.Problems) != 0 {
		t.Errorf("off means off: %v", r.Problems)
	}
}

func TestCheckImportsSaysSoWhenDenoIsMissing(t *testing.T) {
	r := preflight.CheckImports(context.Background(), "", "/x.ts", preflight.ImportsReport, preflight.DynamicWarn, 0)
	if len(r.Problems) != 1 || r.Err() != nil {
		t.Fatalf("a check that did not run is a warning, not a failure: %v", r.Problems)
	}
	if !strings.Contains(r.Problems[0].What, "no deno binary") {
		t.Errorf("got %q", r.Problems[0].What)
	}
}

func TestTextModeReportsBareSpecifiersAsUnverifiable(t *testing.T) {
	src := "import { x } from \"some-package\";\nimport rel from \"./rel.ts\";\nimport \"npm:thing\";\nconst m = await import(\"./\" + y);\n"
	r := preflight.CheckImportsText("remote-script", src, preflight.DynamicWarn)
	if r.Err() != nil {
		t.Fatalf("text mode verifies nothing, so nothing is fatal: %v", r.Err())
	}
	var joined []string
	for _, p := range r.Problems {
		joined = append(joined, p.What)
	}
	all := strings.Join(joined, "\n")
	for _, want := range []string{"some-package", "npm:thing", "cannot be verified", "cannot be resolved statically"} {
		if !strings.Contains(all, want) {
			t.Errorf("want %q in:\n%s", want, all)
		}
	}
	if strings.Contains(all, "./rel.ts\" is visible") {
		t.Errorf("relative imports are the caller's own files, not bare specifiers:\n%s", all)
	}
}

// Codex on PR #2575: deno can store a resolution failure on the dependency
// edge itself ({error, span}, no resolved specifier, no broken module).
// The edge must still be reported, with its span.
func TestParseGraphFindsEdgeLevelErrors(t *testing.T) {
	const fixture = `{
	  "version": 1,
	  "roots": ["file:///work/main.ts"],
	  "modules": [
	    {
	      "kind": "esm",
	      "specifier": "file:///work/main.ts",
	      "local": "/work/main.ts",
	      "dependencies": [
	        {"specifier": "unmapped-pkg", "code": {"error": "Unknown package unmapped-pkg", "span": {"start": {"line": 0, "character": 16}, "end": {"line": 0, "character": 30}}}}
	      ]
	    }
	  ]
	}`
	g, err := preflight.ParseGraph([]byte(fixture))
	if err != nil {
		t.Fatal(err)
	}
	if len(g.Missing) != 1 {
		t.Fatalf("the edge-level failure should be reported, got %+v", g.Missing)
	}
	ms := g.Missing[0]
	if ms.Specifier != "unmapped-pkg" || !strings.Contains(ms.Err, "Unknown package") {
		t.Errorf("missing entry should carry the as-written specifier and deno's error: %+v", ms)
	}
	if ms.Span == nil || ms.Span.Line != 1 || ms.Span.Character != 17 {
		t.Errorf("span should come from the edge, 1-based: %+v", ms.Span)
	}
}

// Codex on PR #2575: the dynamic-import scanner must not treat a regex
// literal or a property method call as a module load — both became fatal
// under script.importsOnDynamic=fail.
func TestScanDynamicImportsSkipsRegexesAndPropertyCalls(t *testing.T) {
	src := "const matcher = /require(foo)/;\n" +
		"const other = /import\\(bar\\)/g;\n" +
		"obj.require(value);\n" +
		"obj.import(value);\n"
	if dyn := preflight.ScanDynamicImports("a.ts", src); len(dyn) != 0 {
		t.Fatalf("regex literals and property calls are not module loads, got %+v", dyn)
	}
	// A real computed import in the same file is still found.
	src += "import(name);\n"
	dyn := preflight.ScanDynamicImports("a.ts", src)
	if len(dyn) != 1 || dyn[0].Kind != "import" {
		t.Fatalf("the real dynamic import should survive, got %+v", dyn)
	}
	// Division is not a regex: a / b stays scannable after it.
	src2 := "const q = a / b;\nrequire(name);\n"
	if dyn := preflight.ScanDynamicImports("b.ts", src2); len(dyn) != 1 {
		t.Fatalf("division must not swallow the file, got %+v", dyn)
	}
}

// Codex round 2 on PR #2575: a line comment at the start of a line was
// scanned as an empty regex followed by code, so a commented-out require
// was reported as a computed import.
func TestScanDynamicImportsSkipsComments(t *testing.T) {
	src := "// require(foo)\n/* import(bar) */\nconst x = 1; // import(baz)\n"
	if dyn := preflight.ScanDynamicImports("a.ts", src); len(dyn) != 0 {
		t.Fatalf("commented-out calls are not module loads, got %+v", dyn)
	}
	src += "require(name);\n"
	if dyn := preflight.ScanDynamicImports("a.ts", src); len(dyn) != 1 {
		t.Fatalf("the real require after the comments should survive, got %+v", dyn)
	}
}

// Codex round 2 on PR #2575: ')' completes a value, so the slashes in
// (a + b) / (await import(name)).default / 2 divide -- the computed
// import between them must not be swallowed as regex contents.
func TestScanDynamicImportsFindsImportBetweenDivisions(t *testing.T) {
	src := "const x = (a + b) / (await import(name)).default / 2;\n"
	dyn := preflight.ScanDynamicImports("a.ts", src)
	if len(dyn) != 1 || dyn[0].Kind != "import" {
		t.Fatalf("the import between the divisions should be found, got %+v", dyn)
	}
}

// Codex round 3 on PR #2575: comments inside an import() argument list
// are not part of the argument -- a commented literal stays literal, and
// a comment before a + does not hide the concatenation.
func TestScanDynamicImportsClassifiesCommentedArguments(t *testing.T) {
	src := "import(/* webpackIgnore: true */ \"./literal.ts\");\n"
	if dyn := preflight.ScanDynamicImports("a.ts", src); len(dyn) != 0 {
		t.Fatalf("a literal import with a leading comment is static, got %+v", dyn)
	}
	src2 := "import(\"./\" /* comment */ + name);\n"
	if dyn := preflight.ScanDynamicImports("b.ts", src2); len(dyn) != 1 {
		t.Fatalf("the concatenation behind the comment is computed, got %+v", dyn)
	}
}

// Codex round 3 on PR #2575: a template literal's text is not code, but
// its ${...} interpolations are -- a computed import inside one must be
// found, at its real position in the file.
func TestScanDynamicImportsFindsImportsInTemplateInterpolations(t *testing.T) {
	src := "const text = `${await import(name)}`;\n"
	dyn := preflight.ScanDynamicImports("a.ts", src)
	if len(dyn) != 1 || dyn[0].Kind != "import" || dyn[0].Line != 1 {
		t.Fatalf("the interpolated import should be found on line 1, got %+v", dyn)
	}
	src2 := "const text = `line one\n${import(name)}`;\n"
	dyn2 := preflight.ScanDynamicImports("b.ts", src2)
	if len(dyn2) != 1 || dyn2[0].Line != 2 {
		t.Fatalf("the interpolated import should be found on line 2, got %+v", dyn2)
	}
	// A template with no interpolation hides nothing and reports nothing.
	src3 := "const text = `import(name)`;\n"
	if dyn := preflight.ScanDynamicImports("c.ts", src3); len(dyn) != 0 {
		t.Fatalf("template literal text is not code, got %+v", dyn)
	}
}

// Codex on PR #2575: the graph check must run with the executing
// runtime's resolution configuration. Only resolution flags are
// forwarded, in both forms; run-only flags are left behind.
func TestResolutionArgsForwardsOnlyResolutionFlags(t *testing.T) {
	in := []string{"--allow-all", "--import-map", "map.json",
		"--config=deno.json", "--quiet", "--node-modules-dir=auto",
		"--frozen", "--unstable-kv", "--lock", "deno.lock"}
	want := []string{"--import-map", "map.json", "--config=deno.json",
		"--node-modules-dir=auto", "--frozen", "--lock", "deno.lock"}
	if got := preflight.ResolutionArgs(in); strings.Join(got, " ") != strings.Join(want, " ") {
		t.Fatalf("ResolutionArgs = %v, want %v", got, want)
	}
	if got := preflight.ResolutionArgs(nil); len(got) != 0 {
		t.Fatalf("ResolutionArgs(nil) = %v, want empty", got)
	}
}

// Codex round (PR #2575): a string followed by anything that continues
// the expression is computed, not a static edge -- only ')' or ','
// (import options) terminate the argument.
func TestScanDynamicImportsComputedBeyondConcatenation(t *testing.T) {
	dyn := preflight.ScanDynamicImports("a.ts", "const m = await import(\"./\".concat(name));\n")
	if len(dyn) != 1 {
		t.Fatalf("import(\"./\".concat(name)): got %+v, want 1 dynamic import", dyn)
	}
	dyn = preflight.ScanDynamicImports("a.ts", "const m = await import(\"./part\".slice(1));\n")
	if len(dyn) != 1 {
		t.Fatalf("import(\"./part\".slice(1)): got %+v, want 1 dynamic import", dyn)
	}
	// A literal with an options argument is still a static edge.
	if dyn := preflight.ScanDynamicImports("a.ts", "const m = await import(\"./x.ts\", { with: { type: \"json\" } });\n"); len(dyn) != 0 {
		t.Fatalf("import with options: got %+v, want no dynamic imports", dyn)
	}
	if dyn := preflight.ScanDynamicImports("a.ts", "const m = await import(\"./x.ts\");\n"); len(dyn) != 0 {
		t.Fatalf("plain literal: got %+v, want no dynamic imports", dyn)
	}
}

// Codex round (PR #2575): when the graph subprocess fails, its stderr
// explanation must reach the caller, not just "exit status 1".
func TestCheckImportsPreservesGraphStderr(t *testing.T) {
	dir := t.TempDir()
	entry := filepath.Join(dir, "main.ts")
	if err := os.WriteFile(entry, []byte("import \"./dep.ts\";\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	stub := filepath.Join(dir, "deno-stub")
	script := "#!/bin/sh\necho 'stub graph failure: the module is not listed' >&2\nexit 1\n"
	if err := os.WriteFile(stub, []byte(script), 0o755); err != nil {
		t.Fatal(err)
	}
	rep := preflight.CheckImportsResolved(context.Background(), stub, entry,
		preflight.ImportsReport, preflight.DynamicWarn, 10*time.Second, nil, dir)
	err := rep.Err()
	if err == nil {
		t.Fatal("want a fatal report from the failing graph subprocess")
	}
	if !strings.Contains(err.Error(), "the module is not listed") {
		t.Fatalf("error = %q, want the subprocess stderr preserved", err.Error())
	}
}

func TestScanDynamicImportsSkipsMethodDeclarations(t *testing.T) {
	src := "class Loader { require(value) { return value; } }\n" +
		"const o = { import(name) { return name; } };\n"
	if dyn := preflight.ScanDynamicImports("a.ts", src); len(dyn) != 0 {
		t.Fatalf("method declarations are not module loads, got %v", dyn)
	}
	src2 := "class Loader { require(value) { return value; } }\nconst m = require(name);\n"
	if dyn := preflight.ScanDynamicImports("a.ts", src2); len(dyn) != 1 {
		t.Fatalf("the real require call after the class should be found, got %v", dyn)
	}
}

func TestScanDynamicImportsFindsImportAfterRegexInTemplate(t *testing.T) {
	src := "const text = `${/}/.test(s) ? await import(name) : ''}`;\n"
	dyn := preflight.ScanDynamicImports("a.ts", src)
	if len(dyn) != 1 {
		t.Fatalf("the regex's } must not close the interpolation early, got %v", dyn)
	}
}

// Codex round (PR #2575): ASI separates a call statement from the block
// that follows it, so a brace alone does not make `import(name)` a method
// definition -- the call must still be reported.
func TestScanDynamicImportsCallBeforeAsiSeparatedBlock(t *testing.T) {
	src := "await import(name)\n{ console.log('block'); }\n"
	if dyn := preflight.ScanDynamicImports("a.ts", src); len(dyn) != 1 {
		t.Fatalf("a call before an ASI-separated block is not a method, got %v", dyn)
	}
	// The inverse must still hold: a method whose body opens on the next
	// line is a declaration, not a call.
	src2 := "class Loader {\n  require(value)\n  { return value; }\n}\n"
	if dyn := preflight.ScanDynamicImports("a.ts", src2); len(dyn) != 0 {
		t.Fatalf("a method body on the next line is still a declaration, got %v", dyn)
	}
}

// Codex round (PR #2575): the graph subprocess runs where the script will
// run, so a relative --config in the runtime arguments resolves against
// the same directory at preflight as at execution.
func TestCheckImportsRunsInExecutionWorkingDirectory(t *testing.T) {
	dir := t.TempDir()
	execDir := t.TempDir()
	entry := filepath.Join(dir, "main.ts")
	if err := os.WriteFile(entry, []byte("import \"./dep.ts\";\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	marker := filepath.Join(dir, "cwd")
	stub := filepath.Join(dir, "deno-stub")
	script := "#!/bin/sh\npwd -P > '" + marker + "'\necho '{\"modules\":[]}'\n"
	if err := os.WriteFile(stub, []byte(script), 0o755); err != nil {
		t.Fatal(err)
	}
	rep := preflight.CheckImportsResolved(context.Background(), stub, entry,
		preflight.ImportsReport, preflight.DynamicWarn, 10*time.Second, nil, execDir)
	if err := rep.Err(); err != nil {
		t.Fatalf("unexpected report: %v", err)
	}
	got, err := os.ReadFile(marker)
	if err != nil {
		t.Fatal(err)
	}
	want, err := filepath.EvalSymlinks(execDir)
	if err != nil {
		t.Fatal(err)
	}
	if strings.TrimSpace(string(got)) != want {
		t.Fatalf("graph subprocess ran in %q, want the execution directory %q",
			strings.TrimSpace(string(got)), want)
	}
}
