package launcher_test

import (
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/launcher"
)

func write(t *testing.T, dir, name, body string) string {
	t.Helper()
	path := filepath.Join(dir, name)
	if err := os.WriteFile(path, []byte(body), 0o644); err != nil {
		t.Fatal(err)
	}
	return path
}

func TestAFileCanDeclareItsPlaceholderInAComment(t *testing.T) {
	// A comment works in any language and cannot affect what the file does
	// at runtime, which makes it the safest of the three spellings.
	dir := t.TempDir()
	p := write(t, dir, "thing.ts", "// @mcpx:placeholder banner\nlog.info(\"hi\");\n")
	defs, err := launcher.LoadDefinitions([]string{p})
	if err != nil {
		t.Fatal(err)
	}
	if len(defs) != 1 || defs[0].Name != "banner" {
		t.Fatalf("got %+v", defs)
	}
	if strings.Contains(defs[0].Body, "@mcpx:placeholder") {
		t.Error("the declaration should not be part of the body")
	}
	if !strings.Contains(defs[0].Body, `log.info("hi")`) {
		t.Errorf("the body should survive: %q", defs[0].Body)
	}
}

func TestADeclarationCanNameItsParameters(t *testing.T) {
	dir := t.TempDir()
	p := write(t, dir, "wrap.ts", "// @mcpx:placeholder wrap(inner, label)\nrun(@label, () => { @inner });\n")
	defs, err := launcher.LoadDefinitions([]string{p})
	if err != nil {
		t.Fatal(err)
	}
	if strings.Join(defs[0].Params, ",") != "inner,label" {
		t.Errorf("got %v", defs[0].Params)
	}
}

func TestAnExportedConstantAlsoDeclares(t *testing.T) {
	// Visible to tooling, unlike a comment.
	dir := t.TempDir()
	p := write(t, dir, "x.ts", "export const MCPX_PLACEHOLDER = \"metrics\";\nsend();\n")
	defs, err := launcher.LoadDefinitions([]string{p})
	if err != nil {
		t.Fatal(err)
	}
	if defs[0].Name != "metrics" {
		t.Errorf("got %q", defs[0].Name)
	}
}

func TestTheFilenameDeclaresWhenNothingElseDoes(t *testing.T) {
	// Least ceremony for a directory of small fragments, and unambiguous
	// because a filename is already unique within its directory.
	dir := t.TempDir()
	p := write(t, dir, "report.ts", "emit(summary());\n")
	defs, err := launcher.LoadDefinitions([]string{p})
	if err != nil {
		t.Fatal(err)
	}
	if defs[0].Name != "report" {
		t.Errorf("got %q", defs[0].Name)
	}
}

func TestADeclaredPlaceholderIsUsableFromATemplate(t *testing.T) {
	dir := t.TempDir()
	p := write(t, dir, "banner.ts", "// @mcpx:placeholder banner\nBANNER;\n")
	defs, _ := launcher.LoadDefinitions([]string{p})

	fill, err := launcher.Bind(launcher.Fill{launcher.Entry: "E"}, defs)
	if err != nil {
		t.Fatal(err)
	}
	got, err := launcher.Expand(
		launcher.Template{Text: "@banner\n@entry", Name: "t"}, fill,
		launcher.Options{Defs: defs})
	if err != nil {
		t.Fatal(err)
	}
	if got != "BANNER;\nE" {
		t.Errorf("got %q", got)
	}
}

func TestADeclaredPlaceholderReceivesArgumentsByName(t *testing.T) {
	dir := t.TempDir()
	p := write(t, dir, "wrap.ts", "// @mcpx:placeholder wrap(inner)\ntry { @inner } catch (e) { log.error(\"caught\", e as Error); }\n")
	defs, _ := launcher.LoadDefinitions([]string{p})
	fill, _ := launcher.Bind(launcher.Fill{launcher.Entry: "ENTRY"}, defs)

	got, err := launcher.Expand(
		launcher.Template{Text: "@wrap(entry)", Name: "t"}, fill,
		launcher.Options{Defs: defs})
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(got, "try { ENTRY }") {
		t.Errorf("the named parameter should bind: %q", got)
	}
}

func TestShadowingABuiltInIsRefused(t *testing.T) {
	// A template that reads correctly and means something else is the worst
	// kind of surprise.
	dir := t.TempDir()
	p := write(t, dir, "x.ts", "// @mcpx:placeholder entry\nNOPE;\n")
	defs, _ := launcher.LoadDefinitions([]string{p})
	if _, err := launcher.Bind(launcher.Fill{}, defs); err == nil {
		t.Fatal("redefining @entry should be refused")
	} else if !strings.Contains(err.Error(), "built in") {
		t.Errorf("the reason should be stated: %v", err)
	}
}

func TestTwoFilesClaimingOneNameIsRefused(t *testing.T) {
	dir := t.TempDir()
	a := write(t, dir, "a.ts", "// @mcpx:placeholder same\nA;\n")
	b := write(t, dir, "b.ts", "// @mcpx:placeholder same\nB;\n")
	defs, _ := launcher.LoadDefinitions([]string{a, b})
	if _, err := launcher.Bind(launcher.Fill{}, defs); err == nil {
		t.Fatal("two declarations of one name cannot be ordered")
	} else if !strings.Contains(err.Error(), "a.ts") || !strings.Contains(err.Error(), "b.ts") {
		t.Errorf("both files should be named: %v", err)
	}
}

func TestADeclarationCanGiveAParameterADefault(t *testing.T) {
	// #1852: `label = "unnamed"` declares a parameter named label whose
	// default is the value after the `=`, not a parameter literally
	// named `label = "unnamed"`.
	dir := t.TempDir()
	p := write(t, dir, "timed.ts", "// @mcpx:placeholder timed(body, label = \"unnamed\")\nrun(@label, () => { @body });\n")
	defs, err := launcher.LoadDefinitions([]string{p})
	if err != nil {
		t.Fatal(err)
	}
	if strings.Join(defs[0].Params, ",") != "body,label" {
		t.Errorf("params: got %v", defs[0].Params)
	}
	if defs[0].Defaults["label"] != `"unnamed"` {
		t.Errorf("defaults: got %v", defs[0].Defaults)
	}
}

func TestADefaultContainingACommaDoesNotCorruptTheSplit(t *testing.T) {
	dir := t.TempDir()
	p := write(t, dir, "timed.ts", "// @mcpx:placeholder timed(body, label = \"a, b\", mode = fast)\nrun(@label);\n")
	defs, err := launcher.LoadDefinitions([]string{p})
	if err != nil {
		t.Fatal(err)
	}
	if strings.Join(defs[0].Params, ",") != "body,label,mode" {
		t.Errorf("params: got %v", defs[0].Params)
	}
	if defs[0].Defaults["label"] != `"a, b"` || defs[0].Defaults["mode"] != "fast" {
		t.Errorf("defaults: got %v", defs[0].Defaults)
	}
}

func TestADefaultAppliesWhenTheUseOmitsTheArgument(t *testing.T) {
	dir := t.TempDir()
	p := write(t, dir, "timed.ts", "// @mcpx:placeholder timed(body, label = \"unnamed\")\nrun(@label, () => { @body });\n")
	defs, _ := launcher.LoadDefinitions([]string{p})
	fill, _ := launcher.Bind(launcher.Fill{launcher.Entry: "ENTRY"}, defs)

	expand := func(text string) string {
		got, err := launcher.Expand(launcher.Template{Text: text, Name: "t"}, fill, launcher.Options{Defs: defs})
		if err != nil {
			t.Fatal(err)
		}
		return got
	}
	if got := expand("@timed(entry)"); got != `run("unnamed", () => { ENTRY });` {
		t.Errorf("omitted argument should bind the default: got %q", got)
	}
	if got := expand(`@timed(entry, "custom")`); got != `run("custom", () => { ENTRY });` {
		t.Errorf("an explicit argument should win over the default: got %q", got)
	}
	if got := expand("@timed"); !strings.Contains(got, `"unnamed"`) {
		t.Errorf("a bare use omits every argument, so the default should still bind: got %q", got)
	}
}

func TestACompoundDefaultSurvivesTheSplit(t *testing.T) {
	// Codex review, PR #2442: splitArgs tracked only parentheses and
	// quotes, so a default that was an array, object or backtick string
	// containing commas was cut at those commas — `value = [1, 2]`
	// declared parameters `value` and `2]`.
	dir := t.TempDir()
	p := write(t, dir, "wrap.ts", "// @mcpx:placeholder wrap(value = [1, 2], meta = {a: 1, b: 2}, text = `x, y`)\nrun(@value, @meta, @text);\n")
	defs, err := launcher.LoadDefinitions([]string{p})
	if err != nil {
		t.Fatal(err)
	}
	if strings.Join(defs[0].Params, ",") != "value,meta,text" {
		t.Errorf("params: got %v", defs[0].Params)
	}
	want := map[string]string{"value": "[1, 2]", "meta": "{a: 1, b: 2}", "text": "`x, y`"}
	for name, def := range want {
		if defs[0].Defaults[name] != def {
			t.Errorf("default %s: got %q, want %q (all: %v)", name, defs[0].Defaults[name], def, defs[0].Defaults)
		}
	}
	fill, _ := launcher.Bind(launcher.Fill{}, defs)
	got, err := launcher.Expand(launcher.Template{Text: "@wrap()", Name: "t"}, fill, launcher.Options{Defs: defs})
	if err != nil {
		t.Fatal(err)
	}
	if got != "run([1, 2], {a: 1, b: 2}, `x, y`);" {
		t.Errorf("compound defaults should expand verbatim: got %q", got)
	}
}

func TestDeclarationTextAfterTheListIsNotPartOfIt(t *testing.T) {
	// Codex review, PR #2442: the declaration capture ran greedily to
	// the last ')' on the line, so trailing explanatory text carrying
	// parentheses became part of a parameter name and @wrap(entry)
	// stopped substituting @value.
	dir := t.TempDir()
	p := write(t, dir, "wrap.ts", "// @mcpx:placeholder wrap(value) // e.g. wrap(entry)\nrun(@value);\n")
	defs, err := launcher.LoadDefinitions([]string{p})
	if err != nil {
		t.Fatal(err)
	}
	if strings.Join(defs[0].Params, ",") != "value" {
		t.Errorf("params: got %v", defs[0].Params)
	}
	fill, _ := launcher.Bind(launcher.Fill{launcher.Entry: "ENTRY"}, defs)
	got, err := launcher.Expand(launcher.Template{Text: "@wrap(entry)", Name: "t"}, fill, launcher.Options{Defs: defs})
	if err != nil {
		t.Fatal(err)
	}
	if got != "run(ENTRY);" {
		t.Errorf("got %q", got)
	}
}

func TestADefaultCanReferenceAnOuterPlaceholder(t *testing.T) {
	// Codex review, PR #2442: explicit arguments have their leading @
	// stripped by readRef, but defaults bypassed that normalisation —
	// wrap(entry = @entry) bound entry to the literal text "@entry",
	// a self-reference the depth bound then refused.
	dir := t.TempDir()
	p := write(t, dir, "wrap.ts", "// @mcpx:placeholder wrap(entry = @entry)\nrun(@entry);\n")
	defs, _ := launcher.LoadDefinitions([]string{p})
	fill, _ := launcher.Bind(launcher.Fill{launcher.Entry: "ENTRY"}, defs)
	for _, use := range []string{"@wrap()", "@wrap(@entry)"} {
		got, err := launcher.Expand(launcher.Template{Text: use, Name: "t"}, fill, launcher.Options{Defs: defs})
		if err != nil {
			t.Fatalf("%s: %v", use, err)
		}
		if got != "run(ENTRY);" {
			t.Errorf("%s: got %q", use, got)
		}
	}
}
