package scripting_test

import (
	"context"
	"encoding/json"
	"strings"
	"testing"

	"github.com/dezren39/mcpx/internal/diagnose"
	"github.com/dezren39/mcpx/internal/scripting"
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
		{"unclosed regex at EOF", "const re = /abc", "unclosed regex literal"},
		{"unclosed paren after division", "const q = a / b;\nconst x = (1;\n", "unclosed delimiter '('"},
		{"unclosed string after division", "const q = a / b;\nconst s = \"oops\n", "unclosed string literal"},
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
