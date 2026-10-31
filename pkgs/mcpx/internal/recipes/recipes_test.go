package recipes_test

import (
	"encoding/json"
	"strings"
	"testing"

	"github.com/dezren39/mcpx/internal/recipes"
)

const stale = `// Close stale issues in a repository.
// @param repo:string        which repository, as owner/name
// @param days:number = 30   how old counts as stale
// @param dryRun:boolean?    report without closing

const found = await demo.search({ repo: @repo, olderThanDays: @days });
if (!@dryRun) await demo.close({ ids: found });
`

func parse(t *testing.T) recipes.Recipe {
	t.Helper()
	return recipes.Parse("close-stale", "/tmp/close-stale.ts", stale)
}

func TestADeclarationBecomesATypedPlaceholder(t *testing.T) {
	r := parse(t)
	if r.Summary != "Close stale issues in a repository." {
		t.Errorf("summary = %q", r.Summary)
	}
	byName := map[string]recipes.Placeholder{}
	for _, p := range r.Placeholders {
		byName[p.Name] = p
	}
	if len(byName) != 3 {
		t.Fatalf("expected three placeholders, got %+v", r.Placeholders)
	}
	if p := byName["repo"]; p.Type != recipes.TypeString || !p.Required ||
		p.Description != "which repository, as owner/name" {
		t.Errorf("repo = %+v", p)
	}
	if p := byName["days"]; p.Type != recipes.TypeNumber || p.Required ||
		p.Default == nil || *p.Default != "30" {
		t.Errorf("days = %+v", p)
	}
	if p := byName["dryRun"]; p.Type != recipes.TypeBoolean || p.Required {
		t.Errorf("dryRun = %+v", p)
	}
}

func TestTheToolsARecipeCallsAreRecorded(t *testing.T) {
	// Matching free text to a recipe leans on this: somebody describing a
	// task uses the words the tools are named with.
	r := parse(t)
	if strings.Join(r.Tools, ",") != "demo.close,demo.search" {
		t.Errorf("tools = %v", r.Tools)
	}
}

func TestValuesAreSubstitutedAsJSONNotAsText(t *testing.T) {
	// A recipe runs with the user's credentials. Pasting raw text in would
	// make every placeholder an injection site.
	r := parse(t)
	out, err := r.Render(map[string]any{"repo": `me/thing"); evil(("`, "dryRun": true})
	if err != nil {
		t.Fatal(err)
	}
	if strings.Contains(out, `evil((`) && !strings.Contains(out, `\"`) {
		t.Fatalf("the value was not encoded:\n%s", out)
	}
	if !strings.Contains(out, "olderThanDays: 30") {
		t.Errorf("the default should be used when nothing was given:\n%s", out)
	}
	if !strings.Contains(out, "if (!true)") {
		t.Errorf("the boolean should be a boolean:\n%s", out)
	}
	if strings.Contains(out, "@repo") {
		t.Errorf("a placeholder survived:\n%s", out)
	}
}

func TestRenderRefusesWhenSomethingRequiredIsMissing(t *testing.T) {
	if _, err := parse(t).Render(nil); err == nil {
		t.Fatal("a recipe with an unfilled required hole must not render")
	}
}

func TestMissingListsOnlyWhatCannotBeDefaulted(t *testing.T) {
	missing := parse(t).Missing(nil)
	if len(missing) != 1 || missing[0].Name != "repo" {
		t.Fatalf("missing = %+v", missing)
	}
}

func TestTheFormCarriesDescriptionsAndDefaults(t *testing.T) {
	// Every question needs an answer for the case where nobody answers, and
	// the schema is where an answerer can see what that would be.
	schema := recipes.FormSchema(parse(t).Placeholders)
	var doc struct {
		Properties map[string]map[string]any `json:"properties"`
		Required   []string                  `json:"required"`
	}
	if err := json.Unmarshal(schema, &doc); err != nil {
		t.Fatal(err)
	}
	if doc.Properties["days"]["default"] != float64(30) {
		t.Errorf("days default = %v", doc.Properties["days"]["default"])
	}
	if doc.Properties["repo"]["description"] == nil {
		t.Error("the description should reach whoever answers")
	}
	if len(doc.Required) != 1 || doc.Required[0] != "repo" {
		t.Errorf("required = %v", doc.Required)
	}
}

func TestAnUndeclaredReferenceIsStillAPlaceholder(t *testing.T) {
	r := recipes.Parse("x", "/tmp/x.ts", "await demo.echo({ message: @who });")
	if len(r.Placeholders) != 1 || r.Placeholders[0].Name != "who" {
		t.Fatalf("placeholders = %+v", r.Placeholders)
	}
}

func TestAnEmailAddressIsNotAPlaceholder(t *testing.T) {
	r := recipes.Parse("x", "/tmp/x.ts", `const to = "someone@example.com";`)
	if len(r.Placeholders) != 0 {
		t.Fatalf("placeholders = %+v", r.Placeholders)
	}
}

func TestMentionsInCommentsAndStringsAreNotPlaceholders(t *testing.T) {
	// #1670: refs() always promised to ignore comments and strings and
	// never did, so a thank-you in a comment became a required input and
	// Render spliced the value inside string literals. Each source below
	// mentions @name somewhere it is not code; none may yield a
	// placeholder, and rendering must leave the source untouched.
	sources := map[string]string{
		"line comment":       "// thanks @alice for the review\nawait demo.echo({ ok: true });",
		"trailing comment":   "await demo.echo({ ok: true }); // cc @alice",
		"block comment":      "/* ping @alice */\nawait demo.echo({ ok: true });",
		"block comment span": "/* ping\n@alice\n*/\nawait demo.echo({ ok: true });",
		"double quoted":      "const note = \"ask @admin first\";\nconsole.log(note);",
		"single quoted":      "const note = 'ask @admin first';\nconsole.log(note);",
		"escaped quote":      "const note = \"ask \\\"@admin\\\" first\";\nconsole.log(note);",
		"template literal":   "const note = `ask @admin first`;\nconsole.log(note);",
		"template span":      "const note = `ask\n@admin first`;\nconsole.log(note);",
	}
	for name, src := range sources {
		t.Run(name, func(t *testing.T) {
			r := recipes.Parse("x", "/tmp/x.ts", src)
			if len(r.Placeholders) != 0 {
				t.Fatalf("placeholders = %+v", r.Placeholders)
			}
			out, err := r.Render(map[string]any{"alice": "x", "admin": "root"})
			if err != nil {
				t.Fatalf("render: %v", err)
			}
			if out != src+"\n" {
				t.Fatalf("render changed a comment/string mention:\n got %q\nwant %q", out, src+"\n")
			}
		})
	}
}

func TestACodeReferenceBesideAStringMentionStillWorks(t *testing.T) {
	// The string mention is prose; the code-position reference beside it
	// is the placeholder, and only it is substituted.
	src := "const note = \"ask @who first\";\nawait demo.echo({ message: @who });"
	r := recipes.Parse("x", "/tmp/x.ts", src)
	if len(r.Placeholders) != 1 || r.Placeholders[0].Name != "who" {
		t.Fatalf("placeholders = %+v", r.Placeholders)
	}
	out, err := r.Render(map[string]any{"who": "hi"})
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(out, `const note = "ask @who first";`) {
		t.Errorf("the string literal was rewritten:\n%s", out)
	}
	if !strings.Contains(out, `message: "hi"`) {
		t.Errorf("the code reference was not substituted:\n%s", out)
	}
}

func TestAToolMarkerIsNotAPlaceholder(t *testing.T) {
	r := recipes.Parse("x", "/tmp/x.ts",
		"// @tool false\nawait demo.echo({ message: \"hi\" });")
	if !r.OptOut {
		t.Error("@tool false should opt the recipe out")
	}
	if len(r.Placeholders) != 0 {
		t.Fatalf("the @tool marker is metadata, not an input: placeholders = %+v", r.Placeholders)
	}
}

func TestACRLFLineContinuationKeepsTheStringOpen(t *testing.T) {
	// Codex P2 on PR 2957: in a single- or double-quoted string, a
	// backslash continuation on a CRLF line escaped only the \r, so
	// the \n ended the string early and the real closing quote
	// opened a new one -- the code after it, @who included, was
	// scanned as string text and never substituted.
	for _, q := range []string{"\"", "'"} {
		src := "const note = " + q + "hello\\\r\nworld" + q + "; await demo.echo({ message: @who });"
		r := recipes.Parse("x", "/tmp/x.ts", src)
		if len(r.Placeholders) != 1 || r.Placeholders[0].Name != "who" {
			t.Fatalf("quote %s: placeholders = %+v", q, r.Placeholders)
		}
		out, err := r.Render(map[string]any{"who": "hi"})
		if err != nil {
			t.Fatalf("quote %s: render: %v", q, err)
		}
		if !strings.Contains(out, `message: "hi"`) {
			t.Errorf("quote %s: the code reference was not substituted:\n%q", q, out)
		}
		if !strings.Contains(out, "hello\\\r\nworld") {
			t.Errorf("quote %s: the continued string was rewritten:\n%q", q, out)
		}
	}
}

func TestALineCommentEndsAtEveryJavaScriptLineTerminator(t *testing.T) {
	// Codex P2 on PR 2957: JavaScript ends a // comment at CR,
	// U+2028 and U+2029 as well as LF. The scanner ended it only at
	// LF, so code after a standalone CR was lexed as comment text
	// and its placeholder was never discovered or substituted.
	for name, term := range map[string]string{
		"CR":     "",
		"U+2028": " ",
		"U+2029": " ",
	} {
		t.Run(name, func(t *testing.T) {
			src := "// note" + term + "await demo.echo({ message: @who });"
			r := recipes.Parse("x", "/tmp/x.ts", src)
			if len(r.Placeholders) != 1 || r.Placeholders[0].Name != "who" {
				t.Fatalf("placeholders = %+v", r.Placeholders)
			}
			out, err := r.Render(map[string]any{"who": "hi"})
			if err != nil {
				t.Fatal(err)
			}
			if !strings.Contains(out, `message: "hi"`) {
				t.Errorf("the code reference after the terminator was not substituted:\n%q", out)
			}
		})
	}
}

func TestADeclarationLineCannotLexTheBody(t *testing.T) {
	// Codex P2 on PR 2957: refs and Render skip declaration lines,
	// but the shared scan lexed their prose as script, so a /* in a
	// @param description opened a block comment over the body.
	src := "# @param path:string -- glob such as /tmp/*\nawait demo.echo({ path: @path });"
	r := recipes.Parse("x", "/tmp/x.ts", src)
	out, err := r.Render(map[string]any{"path": "/tmp/x"})
	if err != nil {
		t.Fatalf("render: %v", err)
	}
	if !strings.Contains(out, `path: "/tmp/x"`) {
		t.Errorf("the declaration prose swallowed the body reference:\n%s", out)
	}
	// The undeclared-reference form: the swallowed name must still
	// be discovered as a placeholder.
	src = "# @param note:string -- see /* details\nawait demo.echo({ message: @who });"
	r = recipes.Parse("x", "/tmp/x.ts", src)
	names := map[string]bool{}
	for _, p := range r.Placeholders {
		names[p.Name] = true
	}
	if !names["who"] {
		t.Fatalf("placeholders = %+v, want who among them", r.Placeholders)
	}
	out, err = r.Render(map[string]any{"note": "n", "who": "hi"})
	if err != nil {
		t.Fatalf("render: %v", err)
	}
	if !strings.Contains(out, `message: "hi"`) {
		t.Errorf("the code reference was not substituted:\n%s", out)
	}
}

// TestJSONDefaultsWithBracketsInsideStrings pins #1671: a JSON default
// whose string values contain bracket characters must survive Parse
// whole — splitDefault counted brackets with no string state, so the
// default was truncated at the first bracket inside a string, the
// remainder polluted the description, and FormSchema silently dropped
// the (now invalid) default. The table covers the permutations: closing
// brackets, opening brackets (which inflated depth the other way,
// swallowing the description), escaped quotes (which kill a naive
// string-skip without escape handling), and escaped backslashes.
func TestJSONDefaultsWithBracketsInsideStrings(t *testing.T) {
	cases := []struct {
		name, def, desc string
	}{
		{"cfg", `{"note": "a}b"}`, "the config"},
		{"list", `["a]b", "c"]`, "the list"},
		{"nested", `[{"k": "v]"}]`, "the items"},
		{"esc", `{"note": "a\"}b"}`, "escaped quote"},
		{"open", `{"q": "a{b[c"}`, "openers in string"},
		{"bslash", `{"p": "a\\b}c"}`, "escaped backslash"},
	}
	var src strings.Builder
	src.WriteString("// Demo recipe.\n")
	for _, c := range cases {
		src.WriteString("// @param " + c.name + ":json = " + c.def + " " + c.desc + "\n")
	}
	src.WriteString("\nconst x = await demo.run({ cfg: @cfg });\n")
	r := recipes.Parse("demo", "/tmp/demo.ts", src.String())
	byName := map[string]recipes.Placeholder{}
	for _, ph := range r.Placeholders {
		byName[ph.Name] = ph
	}
	for _, c := range cases {
		ph, ok := byName[c.name]
		if !ok {
			t.Errorf("%s: placeholder missing", c.name)
			continue
		}
		if ph.Default == nil || *ph.Default != c.def {
			t.Errorf("%s: default = %v, want %s", c.name, ph.Default, c.def)
		}
		if ph.Description != c.desc {
			t.Errorf("%s: description = %q, want %q", c.name, ph.Description, c.desc)
		}
		if ph.Default != nil {
			var v any
			if err := json.Unmarshal([]byte(*ph.Default), &v); err != nil {
				t.Errorf("%s: default is not valid JSON: %v", c.name, err)
			}
		}
	}
	// The defaults must also reach the form schema: a headless caller
	// that never answers is run on them.
	var doc struct {
		Properties map[string]map[string]any `json:"properties"`
	}
	if err := json.Unmarshal(recipes.FormSchema(r.Placeholders), &doc); err != nil {
		t.Fatal(err)
	}
	for _, c := range cases {
		if _, ok := doc.Properties[c.name]["default"]; !ok {
			t.Errorf("%s default missing from form schema: %v", c.name, doc.Properties[c.name])
		}
	}
}
