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

// Review cases (PR #9344): TypeScript declaration tails are not
// expressions, and a newline after an operand-taking keyword does
// not split the expression it belongs to.
func TestRewriteFinalExpressionReviewCases(t *testing.T) {
	cases := []struct {
		name string
		src  string
		want string
		ok   bool
	}{
		{"ts type tail abstains", "const a = 1;\ntype Answer = number;", "", false},
		{"ts interface tail abstains", "interface Answer { value: number }", "", false},
		{"ts enum tail abstains", "enum Colour { Red, Green }", "", false},
		{"new across a newline", "class C {}\nnew\nC()", "class C {}\n__mcpxFinal(await (new\nC()));", true},
		{"typeof across a newline", "typeof\nfoo", "__mcpxFinal(await (typeof\nfoo));", true},
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

// Review cases, round 2 (PR #9344): blank lines and comments do
// not end a statement whose next token continues it, relational
// operators continue an expression, and a comment may sit between
// export and default.
func TestRewriteFinalExpressionReview2Cases(t *testing.T) {
	cases := []struct {
		name string
		src  string
		want string
		ok   bool
	}{
		{"blank line call chain", "const f = () => (x) => x + 1;\nf()\n\n(2)", "const f = () => (x) => x + 1;\n__mcpxFinal(await (f()\n\n(2)));", true},
		{"relational continuation", "1\n< 2", "__mcpxFinal(await (1\n< 2));", true},
		{"export comment default abstains", "export /* entry */ default () => 42;\n1", "", false},
		{"comment then chain", "foo()\n// note\n.bar();", "__mcpxFinal(await (foo()\n// note\n.bar()));", true},
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

// Review cases, round 3 (PR #9344): update operators respect
// statement boundaries, export lists can declare the default
// export, a newline inside a block comment abstains, and a regex
// may open the statement after a block.
func TestRewriteFinalExpressionReview3Cases(t *testing.T) {
	cases := []struct {
		name string
		src  string
		want string
		ok   bool
	}{
		{"postfix update then statement", "let i = 0;\ni++\ni", "let i = 0;\ni++\n__mcpxFinal(await (i));", true},
		{"prefix update statement", "let i = 0;\ni\n++i", "let i = 0;\ni\n__mcpxFinal(await (++i));", true},
		{"export list default abstains", "const answer = () => 42;\nexport { answer as default };\n1", "", false},
		{"block comment newline abstains", "console.log('start') /* one\ntwo */ 42", "", false},
		{"regex after block", "if (true) {} /a;b/.test('a;b')", "if (true) {} __mcpxFinal(await (/a;b/.test('a;b')));", true},
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

// Review cases, CodeRabbit (PR #9344): an unbraced control-flow
// body stays inside its statement across newlines, and a
// declaration split after its keyword is not cut in half. All of
// these must come through untouched: the final span is not an
// expression statement, and a wrong split would corrupt a valid
// program.
func TestRewriteFinalExpressionControlCases(t *testing.T) {
	cases := []string{
		"if (true)\n  1\nelse\n  2",
		"if (ready)\n  go();",
		"for (let i = 0; i < 3; i++)\n  tick();",
		"while (more)\n  step();",
		"const\nanswer = 42",
		"let\nx = 1",
	}
	for _, src := range cases {
		if got, ok := rewriteFinalExpression(src); ok || got != src {
			t.Errorf("rewriteFinalExpression(%q) = %q, %v; want untouched", src, got, ok)
		}
	}
}

// Review cases, round 4 (PR #9344): a statement may end with a
// regex literal or a postfix assertion, an object literal inside
// an unbraced body is not a block boundary, and a regex may open
// an unbraced body.
func TestRewriteFinalExpressionReview4Cases(t *testing.T) {
	transform := []struct{ name, src, want string }{
		{"regex statement then expression", "/a/\n42", "/a/\n__mcpxFinal(await (42));"},
		{"postfix assertion then expression", "const obj = { answer: 42 };\nobj.answer!\n42", "const obj = { answer: 42 };\nobj.answer!\n__mcpxFinal(await (42));"},
		{"regex final statement", "foo();\n/a/.test('a')", "foo();\n__mcpxFinal(await (/a/.test('a')));"},
	}
	for _, tc := range transform {
		t.Run(tc.name, func(t *testing.T) {
			got, ok := rewriteFinalExpression(tc.src)
			if !ok || got != tc.want {
				t.Errorf("got %q, %v\nwant %q", got, ok, tc.want)
			}
		})
	}
	untouched := []string{
		"let x; if (true) x = {} || 42",
		"if (true) /a;b/.test('a;b')",
	}
	for _, src := range untouched {
		if got, ok := rewriteFinalExpression(src); ok || got != src {
			t.Errorf("rewriteFinalExpression(%q) = %q, %v; want untouched", src, got, ok)
		}
	}
}

// Review cases, round 5 (PR #9344): assertion keywords stay
// attached to their types across a newline, and an async
// declaration with an intervening comment is still a declaration.
func TestRewriteFinalExpressionReview5Cases(t *testing.T) {
	if got, ok := rewriteFinalExpression("1 as\nnumber"); !ok || got != "__mcpxFinal(await (1 as\nnumber));" {
		t.Errorf("as across a newline: got %q, %v; want the whole assertion wrapped", got, ok)
	}
	src := "console.log(await f()); async /* helper */ function f() { return 42 }"
	if got, ok := rewriteFinalExpression(src); ok || got != src {
		t.Errorf("async comment declaration: got %q, %v; want untouched", got, ok)
	}
}
