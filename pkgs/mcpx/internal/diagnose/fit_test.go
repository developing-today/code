package diagnose_test

import (
	"strings"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/diagnose"
)

// fitCatalog is the #1927 rung-1 shape-fit playground: a search tool
// whose argument is "term", a query tool whose argument is "query",
// and a list tool with the olderThanDays parameter the issue names.
func fitCatalog() diagnose.Catalog {
	return diagnose.Catalog{
		Tools: []diagnose.Tool{
			{
				Namespace: "demo", Name: "search", Func: "search",
				Shape: diagnose.Shape{
					Props:    map[string]string{"term": "string"},
					Required: []string{"term"},
				},
			},
			{
				Namespace: "demo", Name: "query", Func: "query",
				Shape: diagnose.Shape{
					Props:    map[string]string{"query": "string"},
					Required: []string{"query"},
				},
			},
			{
				Namespace: "demo", Name: "list", Func: "list",
				Shape: diagnose.Shape{
					Props:    map[string]string{"olderThanDays": "integer", "limit": "integer"},
					Required: []string{"olderThanDays"},
				},
			},
		},
	}
}

func TestCallErrorSuggestsTheToolWhoseSchemaTheArgumentsFit(t *testing.T) {
	// search -> query: the names sent satisfy exactly one other
	// tool's schema, so that tool is the deterministic suggestion.
	ds := diagnose.CallError(diagnose.CallErrorInput{
		Namespace: "demo", Tool: "search",
		Args: []byte(`{"query":"x"}`), Code: diagnose.InvalidParams,
		Message: "invalid params",
	}, fitCatalog())
	if len(ds) != 1 {
		t.Fatalf("diagnostics = %d, want 1: %+v", len(ds), ds)
	}
	d := ds[0]
	if !strings.Contains(d.Message, "demo.query") || !strings.Contains(d.Message, "fit") {
		t.Errorf("message should name the fitting tool: %q", d.Message)
	}
	if !strings.Contains(d.Fix, "demo.query") || !strings.Contains(d.Fix, "query") {
		t.Errorf("fix should be the minimal call to the fitting tool: %q", d.Fix)
	}
}

func TestCallErrorSuggestsTheUniquePrefixedParameter(t *testing.T) {
	// olderThan -> olderThanDays: a supplied name that prefixes
	// exactly one declared parameter.
	ds := diagnose.CallError(diagnose.CallErrorInput{
		Namespace: "demo", Tool: "list",
		Args: []byte(`{"olderThan":3}`), Code: diagnose.InvalidParams,
		Message: "invalid params",
	}, fitCatalog())
	if len(ds) != 1 {
		t.Fatalf("diagnostics = %d, want 1: %+v", len(ds), ds)
	}
	d := ds[0]
	if d.Field != "olderThanDays" {
		t.Errorf("field = %q, want olderThanDays", d.Field)
	}
	if !strings.Contains(d.Fix, "olderThanDays") {
		t.Errorf("fix should use the declared name: %q", d.Fix)
	}
}

func TestCallErrorSuggestsNothingWhenTwoToolsFit(t *testing.T) {
	cat := fitCatalog()
	cat.Tools = append(cat.Tools, diagnose.Tool{
		Namespace: "demo", Name: "lookup", Func: "lookup",
		Shape: diagnose.Shape{
			Props:    map[string]string{"query": "string"},
			Required: []string{"query"},
		},
	})
	ds := diagnose.CallError(diagnose.CallErrorInput{
		Namespace: "demo", Tool: "search",
		Args: []byte(`{"query":"x"}`), Code: diagnose.InvalidParams,
		Message: "invalid params",
	}, cat)
	for _, d := range ds {
		if strings.Contains(d.Message, "fit") {
			t.Fatalf("ambiguous fit must suggest nothing, got %+v", ds)
		}
	}
}

func TestCallErrorSuggestsNothingWhenAPrefixIsAmbiguous(t *testing.T) {
	cat := fitCatalog()
	for i := range cat.Tools {
		if cat.Tools[i].Name == "list" {
			cat.Tools[i].Shape.Props["olderThanHours"] = "integer"
		}
	}
	ds := diagnose.CallError(diagnose.CallErrorInput{
		Namespace: "demo", Tool: "list",
		Args: []byte(`{"olderThan":3}`), Code: diagnose.InvalidParams,
		Message: "invalid params",
	}, cat)
	for _, d := range ds {
		if strings.Contains(d.Message, "prefixes") {
			t.Fatalf("ambiguous prefix must suggest nothing, got %+v", ds)
		}
	}
}

func TestCallErrorFitDoesNotFireWhenNamesFitTheCalledTool(t *testing.T) {
	// A plain missing-required call keeps its existing diagnostic:
	// the fit only speaks when a supplied name fits nothing here.
	ds := diagnose.CallError(diagnose.CallErrorInput{
		Namespace: "demo", Tool: "search",
		Args: []byte(`{}`), Code: diagnose.InvalidParams,
		Message: "invalid params",
	}, fitCatalog())
	if len(ds) != 1 || !strings.Contains(ds[0].Message, "term is required") {
		t.Fatalf("want the missing-required diagnostic, got %+v", ds)
	}
}
