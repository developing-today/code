package preflight_test

import (
	"context"
	"strings"
	"testing"

	"github.com/dezren39/mcpx/internal/preflight"
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
