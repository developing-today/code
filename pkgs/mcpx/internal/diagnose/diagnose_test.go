package diagnose_test

import (
	"encoding/json"
	"strings"
	"testing"
	"time"

	"github.com/dezren39/mcpx/internal/diagnose"
)

func catalog() diagnose.Catalog {
	return diagnose.Catalog{
		Tools: []diagnose.Tool{
			{
				Namespace: "demo", Name: "create_issue", Func: "create_issue",
				Shape: diagnose.Shape{
					Props:    map[string]string{"title": "string", "options": "object"},
					Required: []string{"options", "title"},
				},
			},
			{
				Namespace: "demo", Name: "list_issues", Func: "list_issues",
				Shape: diagnose.Shape{Props: map[string]string{"state": "string"}},
			},
		},
		Changes: map[string][]diagnose.Change{
			"demo.create_issue": {{
				When: time.Date(2026, 9, 20, 0, 0, 0, 0, time.UTC),
				Kind: diagnose.ChangeArgRequired, Tool: "demo.create_issue",
				Field: "options", What: "`options` became required",
			}},
		},
	}
}

func TestACallWithNoArgumentsIsExplainedByWhatChanged(t *testing.T) {
	// The whole point of the package: not "the arguments are wrong" but
	// which argument, when it changed, and the smallest call that works.
	ds := diagnose.Script(`await demo.create_issue();`, catalog())
	if len(ds) != 1 {
		t.Fatalf("expected one diagnostic, got %d: %+v", len(ds), ds)
	}
	d := ds[0]
	if d.Kind != diagnose.KindMissingArgument || !d.Fatal {
		t.Errorf("kind = %s fatal = %v", d.Kind, d.Fatal)
	}
	if d.Changed == nil || !strings.Contains(d.Changed.What, "became required") {
		t.Errorf("the history should explain it: %+v", d.Changed)
	}
	text := d.String()
	for _, want := range []string{"2026-09-20", "you wrote", "minimum", "options", "title"} {
		if !strings.Contains(text, want) {
			t.Errorf("rendered diagnostic is missing %q:\n%s", want, text)
		}
	}
}

func TestAMissingKeyInAnObjectLiteralIsNamed(t *testing.T) {
	ds := diagnose.Script(`await demo.create_issue({ title: "x" });`, catalog())
	if len(ds) != 1 || ds[0].Field != "options" {
		t.Fatalf("expected one diagnostic about options, got %+v", ds)
	}
}

func TestASpreadStopsTheGuessing(t *testing.T) {
	// A spread can supply anything, so a missing-argument diagnostic would
	// be a claim mcpx cannot support.
	ds := diagnose.Script(`await demo.create_issue({ ...defaults });`, catalog())
	if len(ds) != 0 {
		t.Fatalf("a spread should silence the check, got %+v", ds)
	}
}

func TestAnUnknownToolSuggestsTheClosestName(t *testing.T) {
	ds := diagnose.Script(`await demo.list_issue({});`, catalog())
	if len(ds) != 1 || ds[0].Kind != diagnose.KindUnknownTool {
		t.Fatalf("expected an unknown-tool diagnostic, got %+v", ds)
	}
	if !strings.Contains(ds[0].Message, "list_issues") {
		t.Errorf("the closest name should be offered: %s", ds[0].Message)
	}
}

func TestCallsInStringsAndCommentsAreNotCalls(t *testing.T) {
	src := `
// await demo.create_issue();
const s = "await demo.create_issue()";
/* await demo.create_issue(); */
const t = ` + "`await demo.create_issue()`" + `;
`
	if ds := diagnose.Script(src, catalog()); len(ds) != 0 {
		t.Fatalf("nothing here is a call: %+v", ds)
	}
}

func TestUnknownNamespacesAreLeftAlone(t *testing.T) {
	// console.log and JSON.parse look exactly like namespace calls.
	if ds := diagnose.Script(`console.log(JSON.parse("{}"));`, catalog()); len(ds) != 0 {
		t.Fatalf("somebody else's code is not this package's business: %+v", ds)
	}
}

func TestToolsPrefixedCallsAreFound(t *testing.T) {
	ds := diagnose.Script(`await tools.demo.create_issue();`, catalog())
	if len(ds) != 1 {
		t.Fatalf("the tools.ns.fn spelling should be recognised: %+v", ds)
	}
}

func TestAnUndeclaredArgumentIsAWarningNotAFailure(t *testing.T) {
	ds := diagnose.Script(`await demo.list_issues({ nope: 1 });`, catalog())
	if len(ds) != 1 || ds[0].Kind != diagnose.KindUnknownArgument {
		t.Fatalf("expected an unknown-argument diagnostic, got %+v", ds)
	}
	if ds[0].Fatal {
		t.Error("servers accept undocumented arguments all the time; this must not stop a run")
	}
}

func TestInvalidParamsIsMappedBackToAField(t *testing.T) {
	ds := diagnose.CallError(diagnose.CallErrorInput{
		Namespace: "demo", Tool: "create_issue",
		Args:    json.RawMessage(`{"title":"x"}`),
		Code:    diagnose.InvalidParams,
		Message: "invalid params",
	}, catalog())
	if len(ds) != 1 || ds[0].Field != "options" {
		t.Fatalf("expected the missing field to be named, got %+v", ds)
	}
	if ds[0].Changed == nil {
		t.Error("the history should be attached to an upstream failure too")
	}
}

func TestAnErrorThatIsNotAboutTheSchemaIsNotExplainedAway(t *testing.T) {
	ds := diagnose.CallError(diagnose.CallErrorInput{
		Namespace: "demo", Tool: "list_issues",
		Code: -32000, Message: "upstream is down",
	}, catalog())
	if len(ds) != 0 {
		t.Fatalf("this package has nothing to add to a transport failure: %+v", ds)
	}
}

func TestADroppedConnectionIsNotReadAsASchemaComplaint(t *testing.T) {
	// mcpx's own client reports a server that died mid-call as -32000
	// "connection closed: ...". The text match used to read "unexpected" as
	// "expected" and blame the arguments for a crash.
	ds := diagnose.CallError(diagnose.CallErrorInput{
		Namespace: "demo", Tool: "create_issue", Args: json.RawMessage(`{}`),
		Code: -32000, Message: "connection closed: unexpected EOF",
	}, catalog())
	if len(ds) != 0 {
		t.Fatalf("a dropped connection is not an argument error: %+v", ds)
	}
}

func TestAFailureThatIsNotAboutArgumentsNamesNoMissingNamespace(t *testing.T) {
	// The daemon calls this for a namespace whose schemas may not have been
	// read yet. "no server is configured" would be false there, and it was
	// the answer for every failure, whatever the server said.
	ds := diagnose.CallError(diagnose.CallErrorInput{
		Namespace: "fresh", Tool: "anything",
		Code: -32603, Message: "internal error: backend unavailable",
	}, catalog())
	if len(ds) != 0 {
		t.Fatalf("an internal error says nothing about the namespace: %+v", ds)
	}
}

func TestAnUnknownToolIsStillNamedWhenTheServerSaysSo(t *testing.T) {
	ds := diagnose.CallError(diagnose.CallErrorInput{
		Namespace: "demo", Tool: "create_isue",
		Code: diagnose.InvalidParams, Message: "unknown tool create_isue",
	}, catalog())
	if len(ds) != 1 || ds[0].Kind != diagnose.KindUnknownTool {
		t.Fatalf("expected an unknown-tool diagnostic, got %+v", ds)
	}
}

func TestShapeOfReadsRequiredAndTypes(t *testing.T) {
	sh := diagnose.ShapeOf(json.RawMessage(
		`{"type":"object","properties":{"a":{"type":"string"},"b":{"type":["integer","null"]}},"required":["a"]}`))
	if sh.Props["a"] != "string" || sh.Props["b"] != "integer|null" {
		t.Errorf("types = %+v", sh.Props)
	}
	if len(sh.Required) != 1 || sh.Required[0] != "a" {
		t.Errorf("required = %v", sh.Required)
	}
}

func TestACallInsideAnotherCallIsStillACall(t *testing.T) {
	// The regression: the scanner used to skip past a call's arguments, so
	// console.log(await demo.create_issue()) was invisible -- which is how
	// most scripts are actually written.
	ds := diagnose.Script(`console.log(String(await demo.create_issue()));`, catalog())
	if len(ds) != 1 || ds[0].Tool != "demo.create_issue" {
		t.Fatalf("expected the nested call to be diagnosed, got %+v", ds)
	}
}

// fieldCatalog is a tool whose declared arguments are the ones the
// #1646 misattribution shapes collide over: short names that hide
// inside longer ordinary words ("id"/"valid" in "invalid", "name" in
// "unnamed").
func fieldCatalog() diagnose.Catalog {
	return diagnose.Catalog{
		Tools: []diagnose.Tool{
			{
				Namespace: "demo", Name: "update_record", Func: "update_record",
				Shape: diagnose.Shape{
					Props: map[string]string{
						"id": "string", "valid": "boolean", "name": "string",
						"query": "string", "x": "string", "user_id": "string",
					},
				},
			},
		},
	}
}

func TestAFieldNamedOnlyInsideAnotherWordIsNotBlamed(t *testing.T) {
	// #1646 BUG-A/BUG-B/BUG-C and the contract controls pinned by the
	// triage on the issue: token boundaries first, quote preference
	// second, longest name within a class, no separator folding.
	cases := []struct {
		name, message, want string
	}{
		{"BUG-A: id inside invalid names nothing", "the value is invalid", ""},
		{"BUG-B: name inside unnamed names nothing", "unnamed field rejected", ""},
		{"BUG-C: valid inside invalid loses to the named id", "invalid value for id", "id"},
		{"BUG-C quoted: valid inside invalid loses to the quoted id", "invalid value: 'id' must be a string", "id"},
		{"control: double-quoted query", `argument "query" is malformed`, "query"},
		{"control: single-quoted x", "argument 'x' has the wrong type", "x"},
		{"control: bare x is required phrasing", "x is required", "x"},
		{"quote preference beats a longer bare token", "valid syntax but 'id' is the wrong one", "id"},
		{"longest wins within the quoted class", `field "user_id" is bad`, "user_id"},
		{"longest wins within the bare class", "user_id is bad", "user_id"},
		{"a hyphen is a token boundary for id", "user-id is not allowed", "id"},
		{"matching stays case-sensitive", "ID is missing", ""},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			ds := diagnose.CallError(diagnose.CallErrorInput{
				Namespace: "demo", Tool: "update_record",
				Code:    diagnose.InvalidParams,
				Message: c.message,
			}, fieldCatalog())
			if len(ds) != 1 {
				t.Fatalf("expected one diagnostic, got %+v", ds)
			}
			if ds[0].Field != c.want {
				t.Errorf("message %q: field = %q, want %q", c.message, ds[0].Field, c.want)
			}
		})
	}
}

func TestASeparatorVariantDoesNotNameTheUnderscoredProp(t *testing.T) {
	// Contract point 4 in isolation: with only user_id declared, the
	// hyphenated spelling in the message must not be folded into it.
	cat := diagnose.Catalog{
		Tools: []diagnose.Tool{
			{
				Namespace: "demo", Name: "update_record", Func: "update_record",
				Shape: diagnose.Shape{
					Props: map[string]string{"user_id": "string"},
				},
			},
		},
	}
	ds := diagnose.CallError(diagnose.CallErrorInput{
		Namespace: "demo", Tool: "update_record",
		Code:    diagnose.InvalidParams,
		Message: "user-id is not allowed",
	}, cat)
	if len(ds) != 1 || ds[0].Field != "" {
		t.Fatalf("expected the generic diagnostic, got %+v", ds)
	}
	ds = diagnose.CallError(diagnose.CallErrorInput{
		Namespace: "demo", Tool: "update_record",
		Code:    diagnose.InvalidParams,
		Message: "user_id is not allowed",
	}, cat)
	if len(ds) != 1 || ds[0].Field != "user_id" {
		t.Fatalf("expected user_id to be named, got %+v", ds)
	}
}
