package mcpclient_test

import (
	"context"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/mcpclient"
)

// #1602: the client surfaces upstream ttlMs/cacheScope hints instead of
// discarding them, aggregating across pages.

func TestListToolsWithHintsAggregatesPages(t *testing.T) {
	s := newScripted(func(method string, params map[string]any) any {
		switch method {
		case "server/discover":
			return modernDiscover(params)
		case "tools/list":
			if params["cursor"] == nil {
				return map[string]any{
					"resultType": "complete",
					"tools":      []any{map[string]any{"name": "a"}},
					"nextCursor": "p2",
					"ttlMs":      5000,
					"cacheScope": "public",
				}
			}
			return map[string]any{
				"resultType": "complete",
				"tools":      []any{map[string]any{"name": "b"}},
				"ttlMs":      2000,
				"cacheScope": "private",
			}
		}
		return nil
	})
	c := dialModern(t, s, mcpclient.Options{})
	tools, hints, err := c.ListToolsWithHints(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	if len(tools) != 2 {
		t.Fatalf("tools = %v, want both pages", tools)
	}
	// Minimum TTL across pages wins; private on any page marks the list.
	if !hints.HasTTL || hints.TTLMs != 2000 || !hints.Private {
		t.Fatalf("hints = %+v, want {TTLMs:2000 HasTTL:true Private:true}", hints)
	}

	// The wrapper keeps its old shape and result.
	wrapped, err := c.ListTools(context.Background())
	if err != nil || len(wrapped) != 2 {
		t.Fatalf("ListTools wrapper = %v, %v", wrapped, err)
	}
}

func TestListResourcesWithHintsPagesTemplates(t *testing.T) {
	s := newScripted(func(method string, params map[string]any) any {
		switch method {
		case "server/discover":
			return modernDiscover(params)
		case "resources/list":
			return map[string]any{
				"resultType": "complete",
				"resources":  []any{map[string]any{"uri": "demo://a", "name": "a"}},
				"ttlMs":      9000,
				"cacheScope": "public",
			}
		case "resources/templates/list":
			if params["cursor"] == nil {
				return map[string]any{
					"resultType":        "complete",
					"resourceTemplates": []any{map[string]any{"uriTemplate": "demo://t/{id}", "name": "t1"}},
					"nextCursor":        "t2",
					"ttlMs":             3000,
				}
			}
			return map[string]any{
				"resultType":        "complete",
				"resourceTemplates": []any{map[string]any{"uriTemplate": "demo://u/{id}", "name": "t2"}},
			}
		}
		return nil
	})
	c := dialModern(t, s, mcpclient.Options{})
	res, hints, err := c.ListResourcesWithHints(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	// The template from page 2 must surface -- before #1602 templates were
	// fetched in a single unpaged call and page 2 was silently dropped.
	names := map[string]bool{}
	for _, r := range res {
		names[r.Name] = true
	}
	if !names["a"] || !names["t1"] || !names["t2"] {
		t.Fatalf("resources = %v, want a, t1 and page-2 template t2", res)
	}
	// Aggregation spans both lists: min declared TTL is the templates' 3000.
	if !hints.HasTTL || hints.TTLMs != 3000 || hints.Private {
		t.Fatalf("hints = %+v, want {TTLMs:3000 HasTTL:true Private:false}", hints)
	}
}

func TestListPromptsWithHintsAbsent(t *testing.T) {
	s := newScripted(func(method string, params map[string]any) any {
		switch method {
		case "server/discover":
			return modernDiscover(params)
		case "prompts/list":
			return map[string]any{
				"resultType": "complete",
				"prompts":    []any{map[string]any{"name": "p1"}},
			}
		}
		return nil
	})
	c := dialModern(t, s, mcpclient.Options{})
	prompts, hints, err := c.ListPromptsWithHints(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	if len(prompts) != 1 {
		t.Fatalf("prompts = %v", prompts)
	}
	// Absent must stay distinguishable from an explicit ttlMs of 0.
	if hints.HasTTL || hints.TTLMs != 0 || hints.Private {
		t.Fatalf("hints = %+v, want zero CacheHints for absent hints", hints)
	}
}
