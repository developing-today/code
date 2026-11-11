package recipes_test

import (
	"encoding/json"
	"math"
	"strings"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/recipes"
	"github.com/developing-today/code/pkgs/mcpx/internal/schemaval"
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

func TestRenderValidatesTypedPlaceholderValues(t *testing.T) {
	tests := []struct {
		name        string
		typeOf      recipes.Type
		value       any
		want        string
		wantErrType string
	}{
		{name: "number rejects boolean", typeOf: recipes.TypeNumber, value: true, wantErrType: "number"},
		{name: "number rejects Inf string", typeOf: recipes.TypeNumber, value: "Inf", wantErrType: "number"},
		{name: "number rejects NaN string", typeOf: recipes.TypeNumber, value: "NaN", wantErrType: "number"},
		{name: "number rejects hexadecimal float", typeOf: recipes.TypeNumber, value: "0x10p0", wantErrType: "number"},
		{name: "number rejects separators", typeOf: recipes.TypeNumber, value: "1_000", wantErrType: "number"},
		{name: "number rejects leading plus", typeOf: recipes.TypeNumber, value: "+3", wantErrType: "number"},
		{name: "number rejects missing integer", typeOf: recipes.TypeNumber, value: ".5", wantErrType: "number"},
		{name: "number rejects positive infinity", typeOf: recipes.TypeNumber, value: math.Inf(1), wantErrType: "number"},
		{name: "number rejects negative infinity", typeOf: recipes.TypeNumber, value: math.Inf(-1), wantErrType: "number"},
		{name: "number rejects NaN", typeOf: recipes.TypeNumber, value: math.NaN(), wantErrType: "number"},
		{name: "string rejects number", typeOf: recipes.TypeString, value: 42, wantErrType: "string"},
		{name: "string rejects boolean", typeOf: recipes.TypeString, value: true, wantErrType: "string"},
		{name: "string rejects array", typeOf: recipes.TypeString, value: []any{"a"}, wantErrType: "string"},
		{name: "boolean rejects number", typeOf: recipes.TypeBoolean, value: 1, wantErrType: "boolean"},
		{name: "integer number control", typeOf: recipes.TypeNumber, value: 3, want: "3"},
		{name: "float number control", typeOf: recipes.TypeNumber, value: 3.5, want: "3.5"},
		{name: "JSON integer string control", typeOf: recipes.TypeNumber, value: "3", want: "3"},
		{name: "JSON decimal string control", typeOf: recipes.TypeNumber, value: "-3.5", want: "-3.5"},
		{name: "JSON exponent string control", typeOf: recipes.TypeNumber, value: "1e2", want: "1e2"},
		{name: "string control is quoted", typeOf: recipes.TypeString, value: "hello", want: `"hello"`},
		{name: "boolean control", typeOf: recipes.TypeBoolean, value: true, want: "true"},
		{name: "boolean string path stays canonical", typeOf: recipes.TypeBoolean, value: "TRUE", want: "true"},
		{name: "JSON object control", typeOf: recipes.TypeJSON, value: map[string]any{"ok": true}, want: `{"ok":true}`},
		{name: "JSON string control stays raw", typeOf: recipes.TypeJSON, value: `{"ok":true}`, want: `{"ok":true}`},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			source := "// @param value:" + string(tt.typeOf) + "\nconst result = @value;"
			r := recipes.Parse("literal", "/tmp/literal.ts", source)
			out, err := r.Render(map[string]any{"value": tt.value})
			if tt.wantErrType != "" {
				if err == nil || !strings.Contains(err.Error(), "value is a "+tt.wantErrType) {
					t.Fatalf("Render error = %v, want error for %s", err, tt.wantErrType)
				}
				return
			}
			if err != nil {
				t.Fatalf("Render: %v", err)
			}
			if !strings.Contains(out, "const result = "+tt.want+";") {
				t.Fatalf("Render output does not contain expected literal %q:\n%s", tt.want, out)
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

func TestRenderValidatesAndEncodesDeclaredDefaults(t *testing.T) {
	cases := []struct {
		name, typ, def, want string
		wantErr              bool
	}{
		{"invalid number identifier", "number", "banana", "", true},
		{"unquoted string", "string", "hello", `value: "hello"`, false},
		{"invalid boolean identifier", "boolean", "yes", "", true},
		{"number expression", "number", "1+1", "", true},
		{"number control", "number", "30", "value: 30", false},
		{"quoted string control", "string", `"hi"`, `value: "hi"`, false},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			src := "// @param value:" + tc.typ + " = " + tc.def + "\nawait demo.run({ value: @value });"
			r := recipes.Parse("demo", "/tmp/demo.ts", src)
			out, err := r.Render(nil)
			if tc.wantErr {
				if err == nil {
					t.Fatalf("Render accepted invalid %s default %q; output:\n%s", tc.typ, tc.def, out)
				}
				if !strings.Contains(err.Error(), "value") || !strings.Contains(err.Error(), tc.typ) {
					t.Errorf("error %q should name the placeholder and expected type %q", err, tc.typ)
				}
				return
			}
			if err != nil {
				t.Fatalf("Render: %v", err)
			}
			if !strings.Contains(out, tc.want) {
				t.Errorf("Render output =\n%s\nwant it to contain %q", out, tc.want)
			}
		})
	}
}

// Codex review on #2646 (4422039950): a TypeJSON placeholder accepts
// any JSON value (arrays included), so FormSchema must not declare
// "object" for it -- with broker-side schema validation (#1954) that
// declaration refused answers the recipe renders fine.
func TestFormSchemaJSONPlaceholderAcceptsAnyJSONValue(t *testing.T) {
	schema := recipes.FormSchema([]recipes.Placeholder{
		{Name: "items", Type: recipes.TypeJSON, Required: true},
		{Name: "label", Type: recipes.TypeString, Required: true},
	})
	var doc struct {
		Properties map[string]map[string]any `json:"properties"`
	}
	if err := json.Unmarshal(schema, &doc); err != nil {
		t.Fatalf("FormSchema output does not parse: %v", err)
	}
	if _, constrained := doc.Properties["items"]["type"]; constrained {
		t.Errorf("TypeJSON property carries a type constraint: %v", doc.Properties["items"])
	}
	if got := doc.Properties["label"]["type"]; got != "string" {
		t.Errorf("TypeString property type = %v, want string", got)
	}
	compiled, err := schemaval.Compile(schema)
	if err != nil {
		t.Fatalf("FormSchema output does not compile: %v", err)
	}
	if err := compiled.ValidateJSON([]byte(`{"items": [1, 2], "label": "x"}`)); err != nil {
		t.Errorf("array answer for a TypeJSON placeholder refused: %v", err)
	}
	if err := compiled.ValidateJSON([]byte(`{"items": {"a": 1}, "label": "x"}`)); err != nil {
		t.Errorf("object answer for a TypeJSON placeholder refused: %v", err)
	}
}

// Oracle: the rendered script stays valid in its own language (PR #8317,
// #1933 slice 2). Render consumes an @param declaration line by keeping
// it as a comment; a #-style declaration (Python/shell) must keep its #
// marker -- rewriting it with // produced "//# @param ..." as the first
// line, which is a syntax error in Python.
func TestRenderKeepsHashCommentMarkerOnConsumedDeclarations(t *testing.T) {
	r := recipes.Parse("greet", "/tmp/greet.py", "# @param name:string   who to greet\nprint(\"hello \" + @name)\n")
	out, err := r.Render(map[string]any{"name": "world"})
	if err != nil {
		t.Fatal(err)
	}
	first, _, _ := strings.Cut(out, "\n")
	if !strings.HasPrefix(first, "#") || strings.Contains(first, "//") {
		t.Errorf("the consumed declaration must stay a # comment, got %q", first)
	}
	if !strings.Contains(out, `print("hello " + "world")`) {
		t.Errorf("the placeholder must be substituted as a JSON literal:\n%s", out)
	}
}

// Oracle: shell-safe argument handling (Codex P1 on PR #8317). A
// shell-backed recipe renders its values as single-quoted shell
// words, inside which the shell performs no command substitution:
// a schema-valid $(...) argument stays data, and an embedded single
// quote cannot break out of the word.
func TestRenderShellValuesStayData(t *testing.T) {
	r := recipes.Parse("greet", "/tmp/greet.sh", "#!/bin/sh\n# @param name:string   who to greet\nprintf 'hello %s' @name\n")
	out, err := r.Render(map[string]any{"name": "$(touch /tmp/pwned)"})
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(out, "printf 'hello %s' '$(touch /tmp/pwned)'") {
		t.Errorf("shell value must render as one single-quoted word:\n%s", out)
	}
	out, err = r.Render(map[string]any{"name": "o'clock"})
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(out, "'o'\\''clock'") {
		t.Errorf("an embedded quote must be closed, escaped and reopened:\n%s", out)
	}
}

// Oracle: Python-compatible literals (Codex P2 on PR #8317). A
// Python recipe renders booleans and null with Python's own
// spellings, including inside JSON-typed values, and an omitted
// optional parameter is None, not the JavaScript undefined the
// Python runtime would reject with a NameError.
func TestRenderPythonSpellsPythonLiterals(t *testing.T) {
	src := "# @param enabled:boolean\n# @param cfg:json\n# @param note:string?\nprint(@enabled, @cfg, @note)\n"
	r := recipes.Parse("check", "/tmp/check.py", src)
	out, err := r.Render(map[string]any{
		"enabled": true,
		"cfg":     map[string]any{"a": true, "b": nil, "n": []any{false}},
	})
	if err != nil {
		t.Fatal(err)
	}
	want := "print(True, {\"a\": True, \"b\": None, \"n\": [False]}, None)"
	if !strings.Contains(out, want) {
		t.Errorf("rendered = %q, want it to contain %q", out, want)
	}
}

// The JavaScript spelling is untouched for a script with no Python
// or shell evidence: the language-aware rendering must not change
// what existing TypeScript recipes produce.
func TestRenderTypeScriptKeepsJSONLiterals(t *testing.T) {
	r := recipes.Parse("greet", "/tmp/greet.ts", "// @param name:string\nconsole.log(@name)\n")
	out, err := r.Render(map[string]any{"name": "$(touch /tmp/pwned)"})
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(out, `console.log("$(touch /tmp/pwned)")`) {
		t.Errorf("a TypeScript recipe keeps its JSON string literal:\n%s", out)
	}
}
