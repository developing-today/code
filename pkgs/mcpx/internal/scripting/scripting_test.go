package scripting_test

import (
	"context"
	"encoding/json"
	"strings"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/diagnose"
	"github.com/developing-today/code/pkgs/mcpx/internal/scripting"
)

func sampleCatalog() *diagnose.Catalog {
	return &diagnose.Catalog{
		Tools: []diagnose.Tool{
			{
				Namespace: "github",
				Name:      "list_issues",
				Func:      "listIssues",
				Shape: diagnose.ShapeOf(json.RawMessage(`{
					"type": "object",
					"properties": {
						"owner": {"type": "string"},
						"repo": {"type": "string"},
						"state": {"type": "string"}
					},
					"required": ["owner", "repo"]
				}`)),
			},
			{
				Namespace: "fs",
				Name:      "read_file",
				Func:      "readFile",
				Shape: diagnose.ShapeOf(json.RawMessage(`{
					"type": "object",
					"properties": {
						"path": {"type": "string"}
					},
					"required": ["path"]
				}`)),
			},
		},
	}
}

func TestValidationOnValidScript(t *testing.T) {
	src := `
const issues = await tools.github.listIssues({ owner: "dezren39", repo: "mcpx" });
console.log("Found issues:", issues.length);
`
	res := scripting.Validate(src, sampleCatalog())
	if !res.Valid {
		t.Fatalf("expected valid script, got: %s", res.Summary())
	}
	if len(res.Questions) > 0 {
		t.Fatalf("unexpected questions for valid script: %+v", res.Questions)
	}
}

func TestValidationDetectsSyntaxAndUnbalancedDelimiters(t *testing.T) {
	cases := []struct {
		name string
		src  string
		err  string
	}{
		{
			name: "unclosed paren",
			src:  `const x = (1 + 2;`,
			err:  "unclosed delimiter '('",
		},
		{
			name: "mismatched delimiter",
			src:  `const obj = { a: [1, 2} ];`,
			err:  "mismatched delimiter",
		},
		{
			name: "unclosed string",
			src:  "const s = \"hello world\nconst y = 2;",
			err:  "unclosed string literal",
		},
	}

	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			res := scripting.Validate(tc.src, nil)
			if res.Valid {
				t.Fatalf("expected invalid result for %s", tc.name)
			}
			found := false
			for _, d := range res.Diagnostics {
				if strings.Contains(d.Message, tc.err) {
					found = true
					break
				}
			}
			if !found {
				t.Fatalf("expected diagnostic containing %q, got: %s", tc.err, res.Summary())
			}
		})
	}
}

func TestValidationRegexLiteralsAreOpaque(t *testing.T) {
	valid := []struct {
		name string
		src  string
	}{
		{"regex double quote", "const re = /\"/;\nconst ok = re.test(\"x\");\n"},
		{"regex escaped open paren", "const re = /\\(/;\nconst ok = re.test(\"(\");\n"},
		{"regex close brace", "const re = /}/;\nconst ok = re.test(\"}\");\n"},
		{"regex comment-like", "const re = /\\/*/;\nconst ok = re.test(\"/*\");\n"},
		{"regex apostrophe", "const re = /'/;\nconst ok = re.test(\"'\");\n"},
		{"regex char class delimiter", "const re = /[)]/;\nconst ok = re.test(\")\");\n"},
		{"regex slash in char class", "const re = /[/]/;\n"},
		{"regex negated class bracket", "const re = /[^]]/;\n"},
		{"regex with flags then division", "const re = /a+/gi;\nconst q = 10 / 2 / 5;\n"},
		{"regex as call argument", "const ok = test(/}/, s);\n"},
		{"division chain", "const q = a / b / c;\n"},
		{"division assign", "let x = 10;\nx /= 2;\n"},
		{"regex empty class", "const re = /[]/;\nconst ok = re.test(\"x\");\n"},
		{"regex negated empty class", "const re = /[^]/;\nconst ok = re.test(\"x\");\n"},
		{"division after postfix increment", "let n = 4;\nconst ratio = n++ / 2;\n"},
		{"division after postfix decrement", "let n = 4;\nconst ratio = n-- / 2;\n"},
		{"division after trailing-dot number", "const ratio = 4. / 2;\n"},
		{"division after leading-dot number", "const ratio = .5 / 2;\n"},
		{"division after non-null assertion", "const n: number | undefined = 4;\nconst ratio = n! / 2;\n"},
		{"regex after if control header", "if (true) /a+/.test(\"aa\");\n"},
		{"regex after statement block", "{} /a+/.test(\"aa\");\n"},
		{"division after expression parens", "const ratio = (1 + 2) / 2;\n"},
		{"division after object literal parens", "const ratio = ({}) / 2;\n"},
		{"regex after return keyword", "function f() { return /a+/.test(\"aa\"); }\n"},
		{"division after keyword-named property", "const obj = {return: 4};\nconst ratio = obj.return / 2;\n"},
		{"division after keyword-named method call", "const obj = {catch: () => 4};\nconst ratio = obj.catch() / 2;\n"},
		{"division after function expression", "const n = function() {} / 2;\n"},
		{"division after class expression", "const n = class {} / 2;\n"},
		{"division after named class expression", "const n = class Foo extends Bar {} / 2;\n"},
		{"regex after function declaration", "function f() {}\n/a+/.test(\"aa\");\n"},
		{"division after non-breaking space", "const ratio = 4\u00a0/ 2;\n"},
		{"division after vertical tab and form feed", "const ratio = 4\v\f/ 2;\n"},
		{"division after BOM whitespace", "const ratio = 4\ufeff/ 2;\n"},
		{"division after line separator", "const ratio = 4\u2028/ 2;\n"},
		{"line separator inside double quoted string", "const s = \"a\u2028b\";\n"},
		{"paragraph separator inside single quoted string", "const s = 'a\u2029b';\n"},
		{"string line continuation backslash CR", "const s = \"a\\\rb\";\n"},
		{"string line continuation backslash CRLF", "const s = \"a\\\r\nb\";\n"},
		{"string line continuation backslash line separator", "const s = \"a\\\u2028b\";\n"},
		{"division after async function assignment in heritage callback", "class C extends {C:()=>{const x=async function(){} /2;},Base:Object}.Base {}\n"},
		{"division after named async function assignment in heritage callback", "class C extends {C:()=>{const x=async function named(){} /2;},Base:Object}.Base {}\n"},
		{"division after async generator assignment in heritage callback", "class C extends {C:()=>{const x=async function*(){} /2;},Base:Object}.Base {}\n"},
		{"division after async function assignment", "const x = async function(){} / 2;\n"},
		{"regex after for-of keyword", "for (const match of /a+/.exec(\"aa\")) {}\n"},
		{"regex after in keyword operator", "const ok = \"source\" in /a+/;\n"},
		{"division after void object literal", "const q = void {} / 2;\n"},
		{"division after delete object literal", "const q = delete {} / 2;\n"},
		{"division after await object literal", "const q = await {} / 2;\n"},
		{"division after yield object literal", "function* g() { const q = yield {} / 2; }\n"},
		{"division after identifier named of", "const of = 4;\nconst ratio = of / 2;\n"},
		{"division after combining-mark identifier", "const a\u0301 = 4;\nconst q = a\u0301 / 2;\n"},
		{"division after ZWNJ identifier", "const a\u200c = 4;\nconst q = a\u200c / 2;\n"},
		{"regex after block following typeof string statement", "typeof \"\"; {} /a+/.test(\"aa\");\n"},
		{"division after function expression with param default object", "const n = function(a = {}) {} / 2;\n"},
		{"regex after switch case block", "switch (1) { case 1: {} /a+/.test(\"aa\"); }\n"},
		{"division after less-than object literal", "const q = 1 < {} / 2;\n"},
		{"division after greater-than object literal", "const q = 1 > {} / 2;\n"},
		{"regex after debugger statement newline", "debugger\n/a+/.test(\"aa\");\n"},
		{"regex after break statement newline", "while (true) { break\n/a+/.test(\"aa\"); }\n"},
		{"division after generic as assertion", "const q = 4 as Extract<number, number> / 2;\n"},
		{"division after generic satisfies assertion", "const q = 4 satisfies Extract<number, number> / 2;\n"},
		{"division after plain as assertion", "const q = x as number / 2;\n"},
		{"object after ternary colon in block", "if (true) { const o = true ? {a: 1} : {}; const q = 4 / 2; }\n"},
		{"division after function expression with nested callback default", "const n = function(a = () => { return 1; }) {} / 2;\n"},
		{"regex after relational less-than following as assertion", "const q = 4 as number < /a+/.source.length;\n"},
		{"regex after relational less-than following satisfies assertion", "const q = 4 satisfies number < /a+/.source.length;\n"},
		{"regex in switch case after optional chain discriminant", "const obj = {x:1}; switch (obj?.x) { case 1: {} /a+/.test(\"aa\"); }\n"},
		{"regex after instanceof operator", "const ok = \"a\" instanceof /a+/.constructor;\n"},
		{"division after object literal following division", "const q = 4 / {} / 2;\n"},
		{"regex after generic class declaration", "class C<T> {} /a+/.test(\"aa\");\n"},
		{"regex after generic interface declaration", "interface C<T> {} /a+/.test(\"aa\");\n"},
		{"division after structural as assertion", "const q = 4 as { toFixed(): string } / 2;\n"},
		{"division after structural satisfies assertion", "const q = 4 satisfies { toFixed(): string } / 2;\n"},
		{"regex after for-await header", "async function f() { for await (const x of [1]) /a+/.test(\"aa\"); }\n"},
		{"hashbang line before script", "#!/usr/bin/env -S deno run --allow-read=/tmp\nconst re = /a+/;\nconst ok = re.test(\"aa\");\n"},
		{"division after object with class-named property", "const obj = {class: 1, x: {} / 2};\n"},
		{"division after object with function-named property", "const obj = {function: 1, x: {} / 2};\n"},
		{"division after ternary with callback semicolon", "function f(){const q = true ? (()=>{return 1;})() : {} / 2;} f();\n"},
		{"division after structural as assertion split after as", "const q = 4 as\n{ toFixed(): string } / 2;\n"},
		{"division after generic as assertion split after as", "const q = 4 as\nExtract<number, number> / 2;\n"},
		{"regex in default export expression", "export default /a+/.test(\"aa\") ? () => 1 : () => 0;\n"},
		{"regex after arrow body on next line", "const f = () => {}\n/a+/.test(\"aa\");\n"},
		{"regex after async arrow body on next line", "const f = async () => {}\n/a+/.test(\"aa\");\n"},
		{"division after object with spaced class property", "const obj = {class : 1, x: {} / 2};\n"},
		{"division after object with commented class property", "const obj = {class /*c*/ : 1, x: {} / 2};\n"},
		{"regex in switch case after nullish discriminant", "switch (null ?? 1) { case 1: {} /a+/.test(\"aa\"); }\n"},
		{"division after nested function expression in param default", "const n = function(a = function() {} / 2) {} / 2;\n"},
		{"division after nested class expression in param default", "const n = function(a = class {} / 2) {} / 2;\n"},
		{"regex after block following string statement", "const x = \"x\"\n{} /a+/.test(\"aa\");\n"},
		{"regex after block following regex statement", "const x = /x/\n{} /a+/.test(\"aa\");\n"},
		{"regex as class heritage expression", "class C extends /a+/.constructor {}\n"},
		{"regex as class expression heritage", "const C = class extends /a+/.constructor {};\n"},
		{"division after spread object operand", "const obj = {...{} / 2};\n"},
		{"division after spread string operand member new", "const a = {...\"x\".new / 2};\n"},
		{"division after spread template operand member new", "const a = {...`x`.new / 2};\n"},
		{"division after spread regex operand member new", "const a = {.../x/.new / 2};\n"},
		{"regex after side-effect import newline", "import \"node:fs\"\n/a+/.test(\"aa\");\n"},
		{"regex after named import newline", "import { readFile } from \"node:fs\"\n/a+/.test(\"aa\");\n"},
		{"regex after re-export newline", "export { readFile } from \"node:fs\"\n/a+/.test(\"aa\");\n"},
		{"regex after prefix increment on new line", "let n = 1; n\n++/a+/.lastIndex;\n"},
		{"division after same-line postfix increment", "let n = 4;\nconst ratio = n++ / 2;\n"},
		{"regex as new operand", "const r = new /a+/.constructor(\"x\");\n"},
		{"division after class expression with object heritage", "const q = class extends {C: Object}.C {} / 2;\n"},
		{"division after function expression with debugger default", "const f = function(a = () => {debugger\n}) {} / 2;\n"},
		{"division after spread object across comment", "const obj = {... /*c*/ {} / 2};\n"},
		{"regex after prefix increment across comment after line break", "let n=1; n\n/*comment*/++/a+/.lastIndex;\n"},
		{"division after braced unicode escape identifier", "const \\u{61} = 4; const q = \\u{61} / 2;\n"},
		{"division after hex unicode escape identifier", "const \\u0061 = 4; const q = \\u0061 / 2;\n"},
		{"division after TypeScript instantiation expression", "function f<T>() { return 4; } const q = f<number> / 2;\n"},
		{"division after TypeScript instantiation on tagged template", "const q = tag`x`<number> / 2;\n"},
		{"division after TypeScript instantiation on tagged template with comment", "const q = tag`x` /* c */ <number> / 2;\n"},
		{"division after TypeScript instantiation call", "function f<T>() { return 4; } const q = f<number>() / 2;\n"},
		{"division after TypeScript instantiation on digit-ending identifier", "const q = f1<number> / 2;\n"},
		{"division after TypeScript instantiation on digit-ending member", "const q = obj.f1<number> / 2;\n"},
		{"division after TypeScript instantiation on escaped digit-ending identifier", "const q = f\\u{61}1<number> / 2;\n"},
		{"division after TypeScript instantiation on escaped digit-ending member", "const q = obj.f\\u{61}1<number> / 2;\n"},
		{"relational comparison after numeric literal stays relational", "const ok = 1 < 2; const q = 4 / 2;\n"},
		{"division after private keyword-named member", "class C { #return=4; f(){ return this.#return / 2; } }\n"},
		{"division after object with import-named property", "const obj = { import: \"foo\"\n / 2 };\n"},
		{"comparison chain with capitalised operand stays relational", "const ok = a < Foo > b;\nconst q = 4 / 2;\n"},
		{"regex after relational chain with capitalised middle operand", "const yes = 1 < Number > /[)]/.source.length;\n"},
		{"regex after relational chain with trailing-dot numeric operand", "const yes = 1. < Number > /[)]/.source.length;\n"},
		{"regex after relational chain with newline before comparison", "const yes = 1\n< Number > /[)]/.source.length;\n"},
		{"statement block inside function in heritage object", "class C extends {C: function(){ {} /[}]/.test('}'); }}.C {}\n"},
		{"division after class expression with nested object heritage", "const C = class extends {x:{C:Object}}.x.C {} / 2;\n"},
		{"division after class expression with arrow body in heritage object", "const C = class extends {C:()=>{}, Base:Object}.Base {} / 2;\n"},
		{"division after class expression with object inside arrow body in heritage object", "const C = class extends {C:()=>{const a={};}, Base:Object}.Base {} / 2;\n"},
		{"regex after class declaration with nested object heritage", "class C extends {x:{C:Object}}.x.C {} /a+/.test(\"aa\");\n"},
		{"postfix increment after escaped identifier at line start", "let a=1;\n\\u0061++ / 2;\n"},
		{"postfix decrement after escaped identifier at line start", "let a=1;\n\\u0061-- / 2;\n"},
		{"regex after class declaration following property named new", "const obj={new:1}; obj.new\nclass C {} /[)]/.test(')');\n"},
		{"regex after function declaration following property named new", "const obj={new:1}; obj.new\nfunction f() {} /[)]/.test(')');\n"},
		{"division after nested class expression in heritage array", "class C extends {x:[class extends {C:Object}.C {} / 2],Base:Object}.Base {}\n"},
		{"division after nested class with new-operand heritage in heritage array", "class C extends {x:[class extends new {C:Object}.C {} / 2]}.x {}\n"},
		{"division after nested class with comment-separated new-operand heritage", "class C extends {x:[class extends new /*c*/ {C:Object}.C {} / 2]}.x {}\n"},
		{"division after nested class with async function heritage operand", "class C extends {x:[class extends async function(){} {} / 2]}.x {}\n"},
		{"division after nested class with named async function heritage operand", "class C extends {x:[class extends async function foo(){} {} / 2]}.x {}\n"},
		{"division after nested class with async generator heritage operand", "class C extends {x:[class extends async function*(){} {} / 2]}.x {}\n"},
		{"division after class expression with direct async function heritage", "const q = class extends async function(){} {} / 2;\n"},
		{"division after nested class with new async function heritage operand", "class C extends {x:[class extends new async function(){} {} / 2]}.x {}\n"},
		{"division after nested class with named new async function heritage operand", "class C extends {x:[class extends new async function foo(){} {} / 2]}.x {}\n"},
		{"division after nested class with new async generator heritage operand", "class C extends {x:[class extends new async function*(){} {} / 2]}.x {}\n"},
		{"division after class expression with direct new async function heritage", "const q = class extends new async function(){} {} / 2;\n"},
		{"regex after declaration following new async split by line break", "new async\nfunction f() {} /[}]/.test(\"}\");\n"},
		{"regex after declaration following new async split by CRLF", "new async\r\nfunction f() {} /[}]/.test(\"}\");\n"},
		{"regex after declaration following new async split by lone carriage return", "new async\rfunction f() {} /[}]/.test(\"}\");\n"},
		{"regex after declaration following new async split by line separator", "new async\u2028function f() {} /[}]/.test(\"}\");\n"},
		{"regex after declaration following new async split by paragraph separator", "new async\u2029function f() {} /[}]/.test(\"}\");\n"},
		{"regex after declaration following new async split by block comment with lone carriage return", "new async /*\r*/ function f() {} /[}]/.test(\"}\");\n"},
		{"regex after declaration following new async split by block comment with line separator", "new async /*\u2028*/ function f() {} /[}]/.test(\"}\");\n"},
		{"regex after declaration following new async split by line comment ended by carriage return", "new async // c\rfunction f() {} /[}]/.test(\"}\");\n"},
		{"regex after declaration following new async split by line comment ended by paragraph separator", "new async // c\u2029function f() {} /[}]/.test(\"}\");\n"},
		{"regex after declaration following new async split by block comment with line break", "new async /*\n*/ function f() {} /[}]/.test(\"}\");\n"},
		{"regex after declaration following new async split by line comment", "new async // c\nfunction f() {} /[}]/.test(\"}\");\n"},
		{"regex after named declaration following new async split by line break", "new async\nfunction foo() {} /[}]/.test(\"}\");\n"},
		{"regex after generator declaration following new async split by line break", "new async\nfunction* f() {} /[}]/.test(\"}\");\n"},
		{"division after nested class with nested object in new-operand heritage", "class C extends {x:[class extends new {C:{D:Object}}.C {} / 2]}.x {}\n"},
		{"division after class expression with new-operand heritage", "const q = class extends new {C:Object}.C {} / 2;\n"},
		{"division after nested class body after member-named new heritage", "class C extends {x:[class extends obj.new {} / 2]}.x {}\n"},
		{"division after roman-numeral identifier", "const \u21601 = 4;\nconst q = \u21601 / 2;\n"},
		{"division after roman-numeral member identifier", "const q = obj.\u21601 / 2;\n"},
		{"division after roman-numeral generic instantiation", "const q = \u21601<number> / 2;\n"},
		{"division after roman-numeral member generic instantiation", "const q = obj.\u21601<number> / 2;\n"},
		{"division after nested class expression as heritage property", "const C = class extends {x: class extends {C:Object}.C {} / 2, Base:Object}.Base {} / 2;\n"},
		{"division after instantiation following block comment with inner opener text", "const q = f /* 1 /* text */<number> / 2;\n"},
		{"division after instantiation with earlier string containing comment opener", "const s = \"1/*\"; const q = f /* comment */<number> / 2;\n"},
		{"division after instantiation following two separate block comments", "const q = f /* a */ /* b */<number> / 2;\n"},
		{"division after instantiation following line comment ending in digit", "const q = f // 1\n<number> / 2;\n"},
		{"division after instantiation on member following line comment ending in digit", "const q = obj.f // 1\n<number> / 2;\n"},
		{"division after instantiation following line comment ending in digit with trailing spaces", "const q = f // 1   \n<number> / 2;\n"},
		{"division after instantiation following line comment ending in digit with trailing tab", "const q = f // 1\t\n<number> / 2;\n"},
		{"division after instantiation on member following line comment ending in digit with trailing spaces and tab", "const q = obj.f // 1 \t \n<number> / 2;\n"},
		{"division after instantiation on digit-ending identifier following line comment with trailing spaces", "const q = f1 // 1  \n<number> / 2;\n"},
		{"division after instantiation on digit-ending identifier following line comment", "const q = f1 // 1\n<number> / 2;\n"},
		{"division inside nested class heritage with nested object operand", "class C extends {x:[class extends {x:{}}.x {} / 2]}.x {}\n"},
		{"division after nested class heritage expression with nested object operand", "const q = class extends {x:[class extends {x:{y:{}}}.x {} / 2]}.x {} / 2;\n"},
		{"division inside nested class heritage with arrow body in nested object", "class C extends {x:[class extends {x:{f:()=>{}}}.x {} / 2]}.x {}\n"},
		{"division after nested class heritage array operand with object second", "class C extends {x:[class extends [0,{C:Object}][1].C {} / 2]}.x {}\n"},
		{"division after nested class heritage array operand with object first", "class C extends {x:[class extends [{C:Object},0][0].C {} / 2]}.x {}\n"},
		{"division after nested class with class-expression heritage in heritage array", "class C extends {x:[class extends class {} {} / 2]}.x {}\n"},
		{"division after nested class with function-expression heritage in heritage array", "class C extends {x:[class extends function(){} {} / 2]}.x {}\n"},
		{"division after nested class with named class-expression heritage in heritage array", "class C extends {x:[class extends class D {} {} / 2]}.x {}\n"},
		{"division after nested class with nonempty function heritage body in heritage array", "class C extends {x:[class extends function(){ {} } {} / 2]}.x {}\n"},
		{"division after nested class with class-method heritage in heritage array", "class C extends {x:[class extends class {m(){}} {} / 2]}.x {}\n"},
		{"division after class expression with class-expression heritage", "const C = class extends class {} {} / 2;\n"},
		{"division after class expression with function-expression heritage", "const C = class extends function() {} {} / 2;\n"},
		{"division after instantiation with block comment before angle bracket", "const q = f1/*c*/<number> / 2;\n"},
		{"division after instantiation with two block comments before angle bracket", "const q = f1/*a*//*b*/<number> / 2;\n"},
		{"division after TypeScript instantiation on middle-dot digit-ending identifier", "const q = f\u00b71<number> / 2;\n"},
		{"division after TypeScript instantiation on middle-dot digit-ending member", "const q = obj.f\u00b71<number> / 2;\n"},
		{"division after TypeScript instantiation on ano-teleia digit-ending identifier", "const q = f\u03871<number> / 2;\n"},
		{"division after TypeScript instantiation on script-p digit-ending identifier", "const q = \u21181<number> / 2;\n"},
		{"division after spread new class expression", "const a={...new class {} / 2};\n"},
		{"division after spread new function expression", "const a={...new function() {} / 2};\n"},
	}
	for _, tc := range valid {
		t.Run(tc.name, func(t *testing.T) {
			res := scripting.Validate(tc.src, nil)
			if !res.Valid {
				t.Fatalf("expected valid script, got: %s", res.Summary())
			}
		})
	}

	invalid := []struct {
		name string
		src  string
		err  string
	}{
		{"unclosed regex before newline", "const re = /abc\nconst x = 1;\n", "unclosed regex literal"},
		{"unclosed regex before carriage return", "const re = /abc\rconst x = 1;\n", "unclosed regex literal"},
		{"unclosed regex at EOF", "const re = /abc", "unclosed regex literal"},
		{"unclosed paren after division", "const q = a / b;\nconst x = (1;\n", "unclosed delimiter '('"},
		{"unclosed string after division", "const q = a / b;\nconst s = \"oops\n", "unclosed string literal"},
		{"unclosed class body after nested heritage object", "const C = class extends {x:{C:Object}}.x.C {\n", "unclosed delimiter '{'"},
	}
	for _, tc := range invalid {
		t.Run(tc.name, func(t *testing.T) {
			res := scripting.Validate(tc.src, nil)
			if res.Valid {
				t.Fatalf("expected invalid result for %s", tc.name)
			}
			found := false
			for _, d := range res.Diagnostics {
				if strings.Contains(d.Message, tc.err) {
					found = true
					break
				}
			}
			if !found {
				t.Fatalf("expected diagnostic containing %q, got: %s", tc.err, res.Summary())
			}
		})
	}
}

func TestValidationDetectsMissingToolArgumentsAndAsksQuestions(t *testing.T) {
	src := `
const res = await tools.github.listIssues();
`
	cat := sampleCatalog()
	res := scripting.Validate(src, cat)
	if res.Valid {
		t.Fatalf("expected invalid result for missing required arguments")
	}
	if len(res.Questions) == 0 {
		t.Fatalf("expected clarification questions for missing required argument")
	}
	q := res.Questions[0]
	if q.Field != "owner" && q.Field != "repo" {
		t.Fatalf("expected question for owner or repo, got: %s", q.Field)
	}
}

type mockProvider struct {
	response string
	err      error
}

func (m *mockProvider) Name() string { return "mock" }
func (m *mockProvider) Complete(ctx context.Context, req scripting.CompletionRequest) (string, error) {
	if m.err != nil {
		return "", m.err
	}
	return m.response, nil
}

func TestRepairSucceedsOnValidFix(t *testing.T) {
	brokenSrc := `const x = (1 + 2;`
	validFixed := `const x = (1 + 2);`

	mock := &mockProvider{response: validFixed}
	ctx := context.Background()

	result, err := scripting.Repair(ctx, brokenSrc, nil, nil, nil, mock)
	if err != nil {
		t.Fatalf("repair should succeed: %v", err)
	}
	if result.RepairedSource != validFixed {
		t.Fatalf("expected repaired source %q, got %q", validFixed, result.RepairedSource)
	}
}

func TestRepairRejectsStillBrokenCode(t *testing.T) {
	brokenSrc := `const x = (1 + 2;`
	stillBroken := `const x = [1 + 2;`

	mock := &mockProvider{response: stillBroken}
	ctx := context.Background()

	_, err := scripting.Repair(ctx, brokenSrc, nil, nil, nil, mock)
	if err == nil {
		t.Fatalf("repair must reject candidate that fails validation")
	}
	if !strings.Contains(err.Error(), "candidate repair failed validation") {
		t.Fatalf("expected error mentioning validation failure, got: %v", err)
	}
}

func TestSynthesizeProducesScript(t *testing.T) {
	synthCode := `
const data = await tools.fs.readFile({ path: "/tmp/test.txt" });
console.log(data);
`
	mock := &mockProvider{response: synthCode}
	ctx := context.Background()

	res, err := scripting.Synthesize(ctx, "read /tmp/test.txt", sampleCatalog(), mock)
	if err != nil {
		t.Fatalf("synthesize failed: %v", err)
	}
	if !strings.Contains(res, "readFile") {
		t.Fatalf("expected readFile in synthesized script, got: %s", res)
	}
}

func TestResolveProviderNamedDrivers(t *testing.T) {
	// 1. opencode with custom model
	pOpenCode := scripting.ResolveProvider(scripting.ProviderConfig{
		Type:  "opencode",
		Model: "claude-code/claude-haiku-5-5",
	}, nil)
	if pOpenCode == nil || pOpenCode.Name() != "opencode" {
		t.Fatalf("expected opencode provider, got: %v", pOpenCode)
	}

	// 2. claude
	pClaude := scripting.ResolveProvider(scripting.ProviderConfig{
		Type:  "claude",
		Model: "claude-haiku-5-5",
	}, nil)
	if pClaude == nil || pClaude.Name() != "claude" {
		t.Fatalf("expected claude provider, got: %v", pClaude)
	}

	// 3. codex
	pCodex := scripting.ResolveProvider(scripting.ProviderConfig{
		Type: "codex",
	}, nil)
	if pCodex == nil || pCodex.Name() != "codex" {
		t.Fatalf("expected codex provider, got: %v", pCodex)
	}

	// 4. antigravity / agy
	pAgy := scripting.ResolveProvider(scripting.ProviderConfig{
		Type: "agy",
	}, nil)
	if pAgy == nil || pAgy.Name() != "antigravity" {
		t.Fatalf("expected antigravity provider, got: %v", pAgy)
	}

	// 5. auto-detection default (no config). Which harness is installed is a
	// fact of the host, so the test names one rather than asking LookPath: a
	// machine with neither opencode nor claude, such as the nix build sandbox,
	// otherwise gets no provider at all.
	t.Setenv("OPENCODE_BINARY", "opencode")
	pAuto := scripting.ResolveProvider(scripting.ProviderConfig{}, nil)
	if pAuto == nil {
		t.Fatalf("expected auto-detected provider, got nil")
	}
	if pAuto.Name() != "opencode" && pAuto.Name() != "claude" {
		t.Fatalf("expected auto provider to be opencode or claude, got %s", pAuto.Name())
	}
}

// The numeric-token guard consults the recorded block-comment
// ranges for every '<': with a long comment history it must stay
// correct (and, since the ranges are binary-searched, fast) --
// Codex P2 on #3354 measured the old linear scan at ~25x slower
// on 60k comments.
func TestValidationNumericGuardWithLongCommentHistory(t *testing.T) {
	var b strings.Builder
	b.WriteString("let a = 1, b = 2;\n")
	for i := 0; i < 20000; i++ {
		b.WriteString("/*c*/ a < b;\n")
	}
	b.WriteString("const q = f1<number> / 2;\n")
	res := scripting.Validate(b.String(), nil)
	if !res.Valid {
		t.Fatalf("expected valid script, got: %s", res.Summary())
	}
}
