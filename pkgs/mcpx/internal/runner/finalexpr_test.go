package runner

import "testing"

// CODE-19 (#1937): the final top-level expression of a snippet is
// rewritten to report its value; everything else is left alone.
func TestRewriteFinalExpression(t *testing.T) {
	cases := []struct {
		name string
		src  string
		want string
		ok   bool
	}{
		{"bare expression", "1 + 2", "__mcpxFinal(await (1 + 2));", true},
		{"parenthesized expression", "(1 + 2)", "__mcpxFinal(await ((1 + 2)));", true},
		{"array literal", "[1, 2]", "__mcpxFinal(await ([1, 2]));", true},
		{"parenthesized object literal", "({ answer: 42 })", "__mcpxFinal(await (({ answer: 42 })));", true},
		{"await call", "await ns.tool({})", "__mcpxFinal(await (await ns.tool({})));", true},
		{"after statements", "const a = 1;\na + 2;", "const a = 1;\n__mcpxFinal(await (a + 2));", true},
		{"no semicolons", "const a = 1\na + 2\n", "const a = 1\n__mcpxFinal(await (a + 2));\n", true},
		{"ends with declaration", "const a = 1;", "", false},
		{"ends with function declaration", "function f() { return 1; }", "", false},
		{"default export abstains", "export default () => 1;\nfoo();", "", false},
		{"named export then expression", "export const x = 1;\nfoo();", "export const x = 1;\n__mcpxFinal(await (foo()));", true},
		{"string with semicolon", `const s = "a;b"; s.length`, `const s = "a;b"; __mcpxFinal(await (s.length));`, true},
		{"template with expr", "const s = `x${1 + 2}y`; s", "const s = `x${1 + 2}y`; __mcpxFinal(await (s));", true},
		{"regex with semicolon", `const r = /a;b/; r.test("a;b")`, `const r = /a;b/; __mcpxFinal(await (r.test("a;b")));`, true},
		{"object literal statement abstains", "{ a: 1 }", "", false},
		{"block then expression", "if (x) { y(); }\nfoo();", "if (x) { y(); }\n__mcpxFinal(await (foo()));", true},
		{"trailing line comment kept", "foo() // done", "__mcpxFinal(await (foo())); // done", true},
		{"chained call across lines", "foo()\n  .bar();", "__mcpxFinal(await (foo()\n  .bar()));", true},
		{"multiline call args", "foo(\n  1,\n  2\n);", "__mcpxFinal(await (foo(\n  1,\n  2\n)));", true},
		{"label abstains", "outer: for (;;) { break outer; }", "", false},
		{"import then expression", "import { x } from \"./m.ts\";\nx();", "import { x } from \"./m.ts\";\n__mcpxFinal(await (x()));", true},
		{"unterminated string abstains", "const s = \"abc", "", false},
		{"empty", "", "", false},
		{"only a comment", "// nothing here", "", false},
		{"assignment expression", "x = 5;", "__mcpxFinal(await (x = 5));", true},
		{"division not regex", "const q = a / b; q", "const q = a / b; __mcpxFinal(await (q));", true},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			got, ok := rewriteFinalExpression(tc.src)
			if ok != tc.ok {
				t.Fatalf("ok = %v, want %v (got %q)", ok, tc.ok, got)
			}
			if tc.ok && got != tc.want {
				t.Errorf("got  %q\nwant %q", got, tc.want)
			}
			if !tc.ok && got != tc.src {
				t.Errorf("abstention changed the source: got %q want %q", got, tc.src)
			}
		})
	}
}
