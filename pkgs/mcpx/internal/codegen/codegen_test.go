package codegen_test

import (
	"encoding/json"
	"strings"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/codegen"
)

func typeOf(t *testing.T, schemaJSON string) string {
	t.Helper()
	return strings.TrimSpace(codegen.TypeFor(json.RawMessage(schemaJSON), ""))
}

func TestPrimitiveTypes(t *testing.T) {
	cases := map[string]string{
		`{"type":"string"}`:  "string",
		`{"type":"integer"}`: "number",
		`{"type":"number"}`:  "number",
		`{"type":"boolean"}`: "boolean",
		`{"type":"null"}`:    "null",
	}
	for in, want := range cases {
		if got := typeOf(t, in); got != want {
			t.Errorf("%s => %q, want %q", in, got, want)
		}
	}
}

func TestNullableTypeArrayBecomesUnion(t *testing.T) {
	if got := typeOf(t, `{"type":["integer","null"]}`); got != "number | null" {
		t.Fatalf("got %q", got)
	}
}

func TestEnumBecomesLiteralUnion(t *testing.T) {
	got := typeOf(t, `{"type":"string","enum":["a","b","c"]}`)
	if got != `"a" | "b" | "c"` {
		t.Fatalf("got %q", got)
	}
}

func TestConstBecomesLiteral(t *testing.T) {
	if got := typeOf(t, `{"const":"fixed"}`); got != `"fixed"` {
		t.Fatalf("got %q", got)
	}
}

func TestArrayOfUnionGetsParentheses(t *testing.T) {
	got := typeOf(t, `{"type":"array","items":{"type":["string","null"]}}`)
	if got != "(string | null)[]" {
		t.Fatalf("got %q", got)
	}
}

func TestTupleViaPrefixItems(t *testing.T) {
	got := typeOf(t, `{"type":"array","prefixItems":[{"type":"string"},{"type":"number"}]}`)
	if got != "[string, number]" {
		t.Fatalf("got %q", got)
	}
}

func TestObjectRequiredAndOptional(t *testing.T) {
	got := typeOf(t, `{
	  "type":"object",
	  "properties":{"a":{"type":"string"},"b":{"type":"number"}},
	  "required":["a"]
	}`)
	if !strings.Contains(got, "a: string;") {
		t.Errorf("required prop should not be optional: %s", got)
	}
	if !strings.Contains(got, "b?: number;") {
		t.Errorf("unlisted prop should be optional: %s", got)
	}
}

func TestNonIdentifierPropertyIsQuoted(t *testing.T) {
	got := typeOf(t, `{"type":"object","properties":{"a-b":{"type":"string"}},"required":["a-b"]}`)
	if !strings.Contains(got, `"a-b": string;`) {
		t.Fatalf("got %s", got)
	}
}

func TestRefResolution(t *testing.T) {
	got := typeOf(t, `{
	  "type":"object",
	  "properties":{"p":{"$ref":"#/$defs/Point"}},
	  "required":["p"],
	  "$defs":{"Point":{"type":"object","properties":{"x":{"type":"number"}},"required":["x"]}}
	}`)
	if !strings.Contains(got, "x: number;") {
		t.Fatalf("$ref was not resolved: %s", got)
	}
}

func TestRecursiveRefTerminates(t *testing.T) {
	got := typeOf(t, `{
	  "type":"object",
	  "properties":{"next":{"$ref":"#/$defs/Node"}},
	  "$defs":{"Node":{"type":"object","properties":{"next":{"$ref":"#/$defs/Node"}}}}
	}`)
	if got == "" {
		t.Fatal("recursive schema produced nothing")
	}
	if !strings.Contains(got, "unknown") {
		t.Fatalf("a cycle should bottom out in unknown: %s", got)
	}
}

func TestAnyOfBecomesUnion(t *testing.T) {
	got := typeOf(t, `{"anyOf":[{"type":"string"},{"type":"number"}]}`)
	if got != "string | number" {
		t.Fatalf("got %q", got)
	}
}

func TestAllOfBecomesIntersection(t *testing.T) {
	got := typeOf(t, `{"allOf":[{"type":"object","properties":{"a":{"type":"string"}},"required":["a"]},{"type":"object","properties":{"b":{"type":"number"}},"required":["b"]}]}`)
	if !strings.Contains(got, "&") {
		t.Fatalf("got %q", got)
	}
}

// The effective schema for a ruled tool (#1986) keeps the upstream
// properties and required list at the root and adds the rules under
// allOf. The rendered type must keep the root fields: dropping them
// would hide required arguments (path, owner) behind the one field a
// rule happens to constrain.
func TestAllOfKeepsSiblingRootProperties(t *testing.T) {
	got := typeOf(t, `{
	  "type":"object",
	  "properties":{"path":{"type":"string"},"owner":{"type":"string"}},
	  "required":["path","owner"],
	  "allOf":[{"type":"object","properties":{"owner":{"enum":["my-org"]}},"required":["owner"]}]
	}`)
	for _, want := range []string{"path: string;", "owner: string;", `owner: "my-org";`, "&"} {
		if !strings.Contains(got, want) {
			t.Errorf("rendered type %q missing %q", got, want)
		}
	}
}

func TestAdditionalPropertiesBecomesIndexSignature(t *testing.T) {
	got := typeOf(t, `{"type":"object","properties":{"a":{"type":"string"}},"additionalProperties":{"type":"number"}}`)
	if !strings.Contains(got, "[key: string]: number;") {
		t.Fatalf("got %s", got)
	}
}

func TestEmptyObjectBecomesRecord(t *testing.T) {
	if got := typeOf(t, `{"type":"object"}`); got != "Record<string, unknown>" {
		t.Fatalf("got %q", got)
	}
}

func TestArgsTypeMarksNoRequiredAsOptional(t *testing.T) {
	_, optional := codegen.ArgsTypeFor(json.RawMessage(`{"type":"object","properties":{"a":{"type":"string"}}}`), "")
	if !optional {
		t.Fatal("a schema with no required props should allow omitting the argument")
	}
	_, optional = codegen.ArgsTypeFor(json.RawMessage(`{"type":"object","properties":{"a":{"type":"string"}},"required":["a"]}`), "")
	if optional {
		t.Fatal("a schema with a required prop must demand the argument")
	}
}

func TestToolFuncName(t *testing.T) {
	cases := map[string]string{
		"take_screenshot": "take_screenshot",
		"fancy-name":      "fancy_name",
		"9lives":          "_9lives",
		"a.b":             "a_b",
	}
	for in, want := range cases {
		if got := codegen.ToolFuncName(in); got != want {
			t.Errorf("ToolFuncName(%q) = %q, want %q", in, got, want)
		}
	}
}

func TestDescriptionBecomesJSDoc(t *testing.T) {
	got := typeOf(t, `{"type":"object","properties":{"a":{"type":"string","description":"the thing"}},"required":["a"]}`)
	if !strings.Contains(got, "/** the thing */") {
		t.Fatalf("got %s", got)
	}
}

func TestCommentTerminatorIsEscaped(t *testing.T) {
	got := typeOf(t, `{"type":"object","properties":{"a":{"type":"string","description":"ends with */ oops"}},"required":["a"]}`)
	if strings.Contains(got, "*/ oops") {
		t.Fatalf("a description must not be able to close its own comment: %s", got)
	}
}

func sampleNamespace() codegen.Namespace {
	return codegen.Namespace{
		Name:        "demo",
		Server:      "demo-server",
		Description: "a demo",
		Tools: []codegen.Tool{
			{
				Name:        "fancy-name",
				Description: "does a thing",
				InputSchema: json.RawMessage(`{"type":"object","properties":{"x":{"type":"string"}},"required":["x"]}`),
			},
			{
				Name:        "noargs",
				InputSchema: json.RawMessage(`{"type":"object","properties":{}}`),
			},
		},
	}
}

func TestDeclarationsShape(t *testing.T) {
	out := codegen.Declarations([]codegen.Namespace{sampleNamespace()})
	for _, want := range []string{
		"declare namespace demo {",
		"function fancy_name(args: {",
		"function noargs(args?: Record<string, unknown>): Promise<ToolResult>;",
		"/** a demo */",
	} {
		if !strings.Contains(out, want) {
			t.Errorf("declarations missing %q:\n%s", want, out)
		}
	}
}

func TestModuleShape(t *testing.T) {
	out := codegen.Module([]codegen.Namespace{sampleNamespace()}, "http://127.0.0.1:1234", "sess-1")
	for _, want := range []string{
		`const DEFAULT_ENDPOINT = "http://127.0.0.1:1234";`,
		`const DEFAULT_SESSION = "sess-1";`,
		`export const demo = {`,
		`return __call("demo-server", "fancy-name", args);`,
		`export const tools = {`,
		`export default tools;`,
		`env("MCPX_SESSION")`,
	} {
		if !strings.Contains(out, want) {
			t.Errorf("module missing %q", want)
		}
	}
}

func TestModuleCallsUseServerNameNotNamespace(t *testing.T) {
	// The namespace is a TypeScript identifier; the wire call must use the
	// real server name so a renamed namespace still routes correctly.
	out := codegen.Module([]codegen.Namespace{sampleNamespace()}, "http://x", "")
	if strings.Contains(out, `__call("demo",`) {
		t.Fatal("call should use the server name, not the namespace")
	}
}

func TestModuleExposesPathHelpers(t *testing.T) {
	out := codegen.Module([]codegen.Namespace{sampleNamespace()}, "http://x", "")
	for _, want := range []string{
		"export interface PathPair",
		"export const paths = {",
		"export function here(meta: { url: string }): PathPair",
		"export function hereDir(meta: { url: string }): PathPair",
		`env("MCPX_CWD")`,
		`env("MCPX_ENTRY")`,
		`env("MCPX_SCRIPT_DIRS")`,
	} {
		if !strings.Contains(out, want) {
			t.Errorf("module missing %q", want)
		}
	}
	// here() must be a function, not a constant: a constant would resolve once
	// inside the client and report the client's path to every importer.
	if strings.Contains(out, "export const here") {
		t.Error("here must be a function so each importing file gets its own path")
	}
}

func TestServerInstructionsAreRenderedAboveSignatures(t *testing.T) {
	n := sampleNamespace()
	n.Instructions = "Call list_pages first to obtain a pageId.\n\nSecond paragraph."
	out := codegen.Declarations([]codegen.Namespace{n})
	if !strings.Contains(out, "Server guidance:") {
		t.Fatalf("instructions should be labelled:\n%s", out)
	}
	if !strings.Contains(out, "Call list_pages first") {
		t.Fatalf("instructions missing:\n%s", out)
	}
	// Must sit above the namespace, not inside it.
	if strings.Index(out, "Call list_pages first") > strings.Index(out, "declare namespace") {
		t.Error("instructions should precede the namespace body")
	}
}

func TestNamespaceWithoutInstructionsKeepsTheShortForm(t *testing.T) {
	out := codegen.Declarations([]codegen.Namespace{sampleNamespace()})
	if !strings.Contains(out, "/** a demo */") {
		t.Fatalf("a description-only namespace should stay on one line:\n%s", out)
	}
	if strings.Contains(out, "Server guidance") {
		t.Error("no instructions means no guidance block")
	}
}

func TestInstructionsCannotCloseTheComment(t *testing.T) {
	n := sampleNamespace()
	n.Instructions = "ends with */ oops"
	out := codegen.Declarations([]codegen.Namespace{n})
	if strings.Contains(out, "*/ oops") {
		t.Fatalf("instructions must not be able to close their own comment:\n%s", out)
	}
}

func TestPreludeRendersAboveServerGuidance(t *testing.T) {
	n := sampleNamespace()
	n.Prelude = "pageId comes from list_pages."
	n.Instructions = "Generic server blurb."
	out := codegen.Declarations([]codegen.Namespace{n})
	if !strings.Contains(out, "Notes:") || !strings.Contains(out, "pageId comes from list_pages") {
		t.Fatalf("prelude missing:\n%s", out)
	}
	// The operator's note is the more specific statement and must come first.
	if strings.Index(out, "Notes:") > strings.Index(out, "Server guidance:") {
		t.Errorf("prelude should precede server guidance:\n%s", out)
	}
}

func TestPreludeAloneStillProducesABlock(t *testing.T) {
	n := sampleNamespace()
	n.Prelude = "only a note"
	out := codegen.Declarations([]codegen.Namespace{n})
	if !strings.Contains(out, "only a note") {
		t.Fatalf("prelude missing with no instructions:\n%s", out)
	}
}

func TestPreludeCannotCloseTheComment(t *testing.T) {
	n := sampleNamespace()
	n.Prelude = "danger */ here"
	out := codegen.Declarations([]codegen.Namespace{n})
	if strings.Contains(out, "*/ here") {
		t.Fatalf("prelude must be escaped:\n%s", out)
	}
}

func TestToolMetadataNeverEmitsNull(t *testing.T) {
	// A nil Go slice marshals to JSON null, and the emitted type says
	// string[]. One parameterless tool therefore made the generated client
	// fail to type check -- and with it every script, whatever the script
	// said, so --typecheck could not report the mistakes it exists to find.
	for _, schema := range []string{``, `{}`, `{"type":"object"}`, `{"type":"object","properties":{}}`} {
		out := codegen.Module([]codegen.Namespace{{
			Name:   "demo",
			Server: "demo",
			Tools:  []codegen.Tool{{Name: "go", InputSchema: json.RawMessage(schema)}},
		}}, "", "")
		if strings.Contains(out, `"params": null`) || strings.Contains(out, `"required": null`) {
			t.Errorf("schema %q emitted a null where string[] is declared:\n%s", schema, out)
		}
	}
}

func TestOutputSchemaTypeGeneration(t *testing.T) {
	outSchema := `{"type":"object","properties":{"status":{"type":"string"},"count":{"type":"number"}}}`
	n := codegen.Namespace{
		Name:   "worker",
		Server: "worker-srv",
		Tools: []codegen.Tool{
			{
				Name:         "do_job",
				Description:  "perform background job",
				InputSchema:  json.RawMessage(`{"type":"object","properties":{"jobId":{"type":"string"}}}`),
				OutputSchema: json.RawMessage(outSchema),
			},
		},
	}

	decls := codegen.Declarations([]codegen.Namespace{n})
	if !strings.Contains(decls, "Promise<Attached<{") || !strings.Contains(decls, "status?: string") || !strings.Contains(decls, "count?: number") {
		t.Fatalf("declarations missing typed return signature:\n%s", decls)
	}

	mod := codegen.Module([]codegen.Namespace{n}, "http://127.0.0.1:1234", "sess-1")
	if !strings.Contains(mod, "Promise<Attached<{") || !strings.Contains(mod, "export type Attached<T, R = RawToolResult> = T & { raw: R };") {
		t.Fatalf("module missing typed return signature or Attached export:\n%s", mod)
	}
}

// Argument rules (#1986) carry their required lists under allOf (via
// EffectiveSchema $defs copies), not at the root: a rule that requires
// an argument must still make the generated parameter mandatory, or
// generated clients pass strict type checking for calls the gate
// necessarily rejects (Codex P2 on PR #2855, round 5).
func TestArgsTypeMarksRuleRequiredAsMandatory(t *testing.T) {
	_, optional := codegen.ArgsTypeFor(json.RawMessage(`{"type":"object","properties":{"a":{"type":"string"}},"allOf":[{"required":["a"]}]}`), "")
	if optional {
		t.Fatal("an allOf member requiring a prop must demand the argument object")
	}
	_, optional = codegen.ArgsTypeFor(json.RawMessage(`{"type":"object","properties":{"a":{"type":"string"}},"$defs":{"argumentrule0$doc":{"type":"object","required":["a"]}},"allOf":[{"$ref":"#/$defs/argumentrule0$doc"}]}`), "")
	if optional {
		t.Fatal("a $ref'd rule requiring a prop must demand the argument object")
	}
	_, optional = codegen.ArgsTypeFor(json.RawMessage(`{"type":"object","properties":{"a":{"type":"string"}},"anyOf":[{"required":["a"]}]}`), "")
	if !optional {
		t.Fatal("an anyOf branch requiring a prop must not demand the argument object")
	}
}

// #1864 item 4: the generated clients carry the run's call settings
// (MCPX_SETTINGS, set by the daemon or the CLI from the caller's
// X-Mcpx-Settings header) back to the daemon on every call, so a
// per-call pool.callTimeout governs a script's calls too.
func TestGeneratedClientsForwardCallSettings(t *testing.T) {
	ts := codegen.Module(nil, "http://127.0.0.1:0", "session")
	if n := strings.Count(ts, `"x-mcpx-settings": env("MCPX_SETTINGS")`); n < 2 {
		t.Fatalf("TypeScript client: x-mcpx-settings present %d times, want at least 2 (__call and batch)", n)
	}
	py := codegen.PythonModule(nil, "http://127.0.0.1:0", "session")
	if !strings.Contains(py, `("x-mcpx-settings", os.environ.get("MCPX_SETTINGS", ""))`) {
		t.Fatal("Python client: x-mcpx-settings header missing")
	}
}
