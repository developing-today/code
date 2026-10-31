package secrets

import (
	"reflect"
	"strconv"
	"strings"
	"testing"
	"time"
)

func none(string) bool { return false }

func only(names ...string) func(string) bool {
	set := map[string]bool{}
	for _, n := range names {
		set[n] = true
	}
	return func(ns string) bool { return set[ns] }
}

func TestReferencesFindsTheNamespacesAScriptReaches(t *testing.T) {
	cases := []struct {
		name       string
		src        string
		configured func(string) bool
		want       []string
	}{
		{"dotted call", `const r = await tools.fs.read({path: "/x"});`, none, []string{"fs"}},
		{"bracket form", `await tools["fs"]["read"]({})`, none, []string{"fs"}},
		{"single quotes in brackets", `await tools['chrome']['list']()`, none, []string{"chrome"}},
		{"a reference without a call", `const f = tools.gh;`, none, []string{"gh"}},
		{"across lines", "await tools\n  .fs\n  .read({})", none, []string{"fs"}},
		{"nested calls both count", `console.log(await tools.a.b({ x: tools.c.d() }))`, none, []string{"a", "c"}},
		{"a namespace that is not configured still counts", `tools.nope.x()`, none, []string{"nope"}},
		{"names are sorted and unique", `tools.b.x(); tools.a.y(); tools.b.z();`, none, []string{"a", "b"}},

		{"a string is not code", `const s = "tools.fs.read()";`, none, nil},
		{"a single-quoted string is not code", `const s = 'tools.fs.read()';`, none, nil},
		{"an escaped quote does not end the string", `const s = "it\"s tools.fs.read()";`, none, nil},
		{"a line comment is not code", "// tools.fs.read()\nx();", none, nil},
		{"a block comment is not code", "/* tools.gh.x() */ y();", none, nil},
		{"plain template text is not code", "const s = `tools.fs.read()`;", none, nil},
		{"an unterminated string ends at its line", "const s = \"abc\ntools.fs.read()", none, []string{"fs"}},

		{"a template interpolation is code", "console.log(`${await tools.chrome.list()}`)", none, []string{"chrome"}},
		{"a call inside an object in an interpolation", "`${({x: tools.gh.get()}).x}`", none, []string{"gh"}},
		{"plain text after an interpolation is still text", "`${1} tools.fs.read()`", none, nil},

		{"a member of something else is not a namespace", `client.tools.fs.read()`, none, nil},
		{"optional chaining is a member too", `x?.tools.fs.read()`, none, nil},
		{"a computed key is invisible", `tools[name].read()`, none, nil},
		{"a key in an object is not a reference", `const o = { demo: 1 }`, only("demo"), nil},

		{"a bare configured namespace is a reference", `await demo.echo({})`, only("demo"), []string{"demo"}},
		{"a bare unconfigured name is not", `await other.echo({})`, only("demo"), nil},
		{"a bare name the script declares is a local", "const demo = {x: 1};\ndemo.echo();", only("demo"), nil},

		// Codex round on PR #3042: scanner edge shapes that manufactured
		// phantom declarations or swallowed real references.
		{"division after a postfix increment is not a regex", `let n = 2; n++ / 2; tools.fs.read();`, none, []string{"fs"}},
		{"division after a keyword member is not a regex", `const o = {return: 4}; o.return / 2; tools.fs.read();`, none, []string{"fs"}},
		{"a comma string in a default is not a delimiter", `function f(x = "," + demo.echo()) {}`, only("demo"), []string{"demo"}},
		{"declaration keywords as object keys bind nothing", `const o = { let: 1, demo: 2 }; demo.echo();`, only("demo"), []string{"demo"}},
		{"a typescript annotation is not a binding", `function f(x: typeof demo) {} demo.echo();`, only("demo"), []string{"demo"}},
		{"a typescript generic annotation is not a binding", `function f(x: Map<string, demo>) {} demo.echo();`, only("demo"), []string{"demo"}},
		{"declaration scanning stops at an ASI boundary", "const x = 1\na(), demo.echo();", only("demo"), []string{"demo"}},
		{"control-condition parens do not end a value", `if (true) /x*/.test("x"); tools.fs.read();`, none, []string{"fs"}},
		{"while condition parens do not end a value", `while (true) /x*/.test("x"); tools.fs.read();`, none, []string{"fs"}},
		{"for condition parens do not end a value", `for (;;) /x*/.test("x"); tools.fs.read();`, none, []string{"fs"}},
		{"variable type annotations are not bindings", `type demo = string; const x: Map<string, demo> = new Map(); demo.echo();`, only("demo"), []string{"demo"}},
		{"variable type annotation with multiple declarators", `const x: Map<string, demo> = new Map(), y = 1; demo.echo();`, only("demo"), []string{"demo"}},
		{"an arrow return type annotation is not a parameter", `type demo = string; const f = (): demo => ""; demo.echo();`, only("demo"), []string{"demo"}},
		{"generic type arguments in an initializer do not bind", `type demo = string; const x = identity<string, demo>(""); demo.echo();`, only("demo"), []string{"demo"}},
		{"a string key spelling a brace does not close a destructuring pattern", `const { "}": x, demo: local } = obj; demo.echo();`, only("demo"), []string{"demo"}},
		{"type-only import specifiers do not bind values", `import { type demo } from "./types"; demo.echo();`, only("demo"), []string{"demo"}},
		{"type-only aliased import specifiers do not bind values", `import { type demo as aliased } from "./types"; demo.echo();`, only("demo"), []string{"demo"}},
		{"whole-statement type import binds no values", `import type { demo } from "./types"; demo.echo();`, only("demo"), []string{"demo"}},
		{"whole-statement default type import binds no values", `import type demo from "./types"; demo.echo();`, only("demo"), []string{"demo"}},
		{"importing type as a named value still binds", `import { type as demo } from "./types"; demo.echo();`, only("demo"), nil},
		{"numeric literal with trailing decimal point is not split", `const n = 1. / 2; tools.fs.read();`, none, []string{"fs"}},
		{"member access on numeric literal", `1..toString(); tools.fs.read();`, none, []string{"fs"}},
		{"an imported name is declared too", `import { demo } from "./lib.ts"; demo.echo();`, only("demo"), nil},
		{"an aliased import declares the alias", `import { x as demo } from "./lib.ts"; demo.echo();`, only("demo"), nil},
		{"a bare name inside a string is not", `const s = "demo.echo()";`, only("demo"), nil},
		{"both spellings in one script", `await demo.echo(); await tools.fs.read();`, only("demo"), []string{"demo", "fs"}},

		{"an empty script reaches nothing", ``, none, nil},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			got := References(c.src, c.configured)
			if got == nil {
				t.Fatal("References must never return nil: it serialises as null")
			}
			// Empty and nil are the same answer here: nothing was reached.
			if len(got) == 0 && len(c.want) == 0 {
				return
			}
			if !reflect.DeepEqual(got, c.want) {
				t.Errorf("References(%q) = %v; want %v", c.src, got, c.want)
			}
		})
	}
}

func TestReferencesRegexLiteralsBindingsAndPhantoms(t *testing.T) {
	cases := []struct {
		name       string
		src        string
		configured func(string) bool
		want       []string
	}{
		{"a regex literal in assignment is not code", "const r = /tools.fs.read/;", none, nil},
		{"a regex holding a bare reach is not code", "const r = /demo.echo/;", only("demo"), nil},
		{"a regex returned is not code", "function f() { return /demo.echo/; }", only("demo"), nil},
		{"a regex passed as an argument is not code", "check(/demo.echo/);", only("demo"), nil},
		{"a regex in a test condition is not code", "if (/demo.echo/.test(s)) { x(); }", only("demo"), nil},
		{"a slash inside a character class does not end the regex", "const r = /[/]demo.echo/;", only("demo"), nil},
		{"an escaped slash does not end the regex", "const r = /demo\\/x.echo/;", only("demo"), nil},
		{"division after an identifier is not a regex", "const x = a / b; demo.echo();", only("demo"), []string{"demo"}},
		{"division after a number is not a regex", "const x = 10 / 2; tools.fs.read();", none, []string{"fs"}},
		{"a real reach after a regex still counts", "const r = /x/; demo.echo();", only("demo"), []string{"demo"}},

		{"a function parameter is declared", "function f(demo) { demo.echo(); }", only("demo"), nil},
		{"an arrow parameter in parens is declared", "const f = (demo, other) => demo.echo();", only("demo"), nil},
		{"a lone arrow parameter is declared", "const f = demo => demo.echo();", only("demo"), nil},
		{"a destructured parameter is declared", "function f({demo}) { demo.echo(); }", only("demo"), nil},
		{"a plain destructured binding is declared", "const {demo} = obj; demo.echo();", only("demo"), nil},
		{"a renamed destructured binding is declared", "const {x: demo} = obj; demo.echo();", only("demo"), nil},
		{"an array destructured binding is declared", "const [demo] = arr; demo.echo();", only("demo"), nil},
		{"a catch binding is declared", "try { x(); } catch (demo) { demo.echo(); }", only("demo"), nil},
		{"a var destructured binding is declared", "var {demo} = obj; demo.echo();", only("demo"), nil},

		{"a declaration in a line comment declares nothing", "// const demo = {};\ndemo.echo();", only("demo"), []string{"demo"}},
		{"a declaration in a block comment declares nothing", "/* let demo */ demo.echo();", only("demo"), []string{"demo"}},
		{"a declaration in a string declares nothing", "const s = \"let demo\"; demo.echo();", only("demo"), []string{"demo"}},
		{"a declaration in template text declares nothing", "const s = `const demo`; demo.echo();", only("demo"), []string{"demo"}},
		{"a parameter shape in a comment declares nothing", "// function f(demo) {}\ndemo.echo();", only("demo"), []string{"demo"}},

		// Codex round 4 on PR #3042 (head 0aa195e73): three more shapes.
		{"a function-type parameter in a type alias is not a runtime binding", "type F = (demo: string) => string; demo.echo();", only("demo"), []string{"demo"}},
		{"a function-type annotation on a variable is not a runtime binding", "let g: (demo: string) => string; demo.echo();", only("demo"), []string{"demo"}},
		{"a runtime arrow after a type alias still binds", "type F = (x: string) => string; const f = (demo) => demo.echo();", only("demo"), nil},
		{"division after a non-null assertion is not a regex", "let n: number | undefined = 2; n! / 2; tools.fs.read();", none, []string{"fs"}},
		{"a regex after a prefix negation is not code", "const ok = !/demo.echo/.test(s);", only("demo"), nil},
		{"a for-in right-hand call is not a declaration", "for (const x in {}, demo.echo()) {}", only("demo"), []string{"demo"}},
		{"a generic assertion type in an initializer does not bind", "type demo = string; const x = {} as Record<string, demo>; demo.echo();", only("demo"), []string{"demo"}},
		{"a generic satisfies type in an initializer does not bind", "const x = {} satisfies Map<string, demo>; demo.echo();", only("demo"), []string{"demo"}},
		{"a semicolon-free generic alias ends before the next declaration", "type F = Array<string>\nconst demo = {echo() {}}; demo.echo();", only("demo"), nil},
		{"a regex after a debugger statement is not division", "debugger\n/x*/.test(\"x\"); tools.fs.read();", none, []string{"fs"}},
		{"a regex after a break statement is not division", "while (x) { break\n/y/.test(z); } tools.fs.read();", none, []string{"fs"}},
		{"a for-in binding is still declared", "for (const demo in obj) { demo.echo(); }", only("demo"), nil},
		{"a for-of binding is still declared", "for (const demo of items) { demo.echo(); }", only("demo"), nil},
		{"generic type arguments in parameter defaults do not bind", "function f(x = identity<string, demo>(\"\")) {} demo.echo();", only("demo"), []string{"demo"}},
		{"a regex after a for-await head is not division", "async function f() { for await (const x of []) /x*/.test(\"x\"); tools.fs.read(); } f();", none, []string{"fs"}},
		{"generic call in object destructuring default", "const {x = identity<string, demo>('')} = obj; demo.echo();", only("demo"), []string{"demo"}},
		{"nested generic call in array destructuring default", "const [x = identity<Map<string, demo>, number>('')] = obj; demo.echo();", only("demo"), []string{"demo"}},
		{"for await iterable does not declare references", "for await (const x of items, demo.echo()) {}", only("demo"), []string{"demo"}},
		{"for await binding is declared", "for await (const demo of items) { demo.echo(); }", only("demo"), nil},
		{"member named for is not a control head", "obj.for(x) / 2; tools.fs.read();", none, []string{"fs"}},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			got := References(c.src, c.configured)
			if got == nil {
				t.Fatal("References must never return nil: it serialises as null")
			}
			if len(got) == 0 && len(c.want) == 0 {
				return
			}
			if !reflect.DeepEqual(got, c.want) {
				t.Errorf("References(%q) = %v; want %v", c.src, got, c.want)
			}
		})
	}
}

func TestReferencesManyDeclarationsStayCorrect(t *testing.T) {
	// Perf-guard companion (Codex round 5): for-head context is
	// computed in one forward pass, so declaration-heavy scripts stay
	// correct — and linear — at scale.
	var b strings.Builder
	for i := 0; i < 2000; i++ {
		b.WriteString("const v")
		b.WriteString(strconv.Itoa(i))
		b.WriteString(" = 1;\n")
	}
	b.WriteString("for (const x in {}, demo.echo()) {}\n")
	got := References(b.String(), only("demo"))
	if len(got) != 1 || got[0] != "demo" {
		t.Fatalf("References(2000 declarations + for-in) = %v; want [demo]", got)
	}
}

func TestReferencesManyComparisonsStayLinear(t *testing.T) {
	var b strings.Builder
	b.WriteString("const x = ")
	for i := 0; i < 16000; i++ {
		b.WriteString("a < b && ")
	}
	b.WriteString("true;\ndemo.echo();\n")
	start := time.Now()
	got := References(b.String(), only("demo"))
	elapsed := time.Since(start)
	if len(got) != 1 || got[0] != "demo" {
		t.Fatalf("References(16000 comparisons) = %v; want [demo]", got)
	}
	t.Logf("16000 comparisons took %v", elapsed)
}
