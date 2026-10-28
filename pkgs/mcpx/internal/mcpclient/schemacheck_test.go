package mcpclient_test

import (
	"context"
	"encoding/json"
	"errors"
	"strings"
	"sync/atomic"
	"testing"
	"time"

	"github.com/dezren39/mcpx/internal/mcpclient"
)

// schemaServer serves one tools/list page carrying the given tools and
// counts tools/call requests. callResult is what tools/call returns.
type schemaServer struct {
	*scriptedServer
	calls atomic.Int64
}

func newSchemaServer(t *testing.T, tools []any, callResult map[string]any) (*schemaServer, *mcpclient.Client, chan mcpclient.Warning) {
	t.Helper()
	ss := &schemaServer{}
	ss.scriptedServer = newScripted(func(method string, params map[string]any) any {
		switch method {
		case "server/discover":
			return modernDiscover(params)
		case "tools/list":
			return map[string]any{"resultType": "complete", "tools": tools}
		case "tools/call":
			ss.calls.Add(1)
			return callResult
		}
		return nil
	})
	c := dialModern(t, ss.scriptedServer, mcpclient.Options{})
	warned := make(chan mcpclient.Warning, 16)
	c.Subscribe(mcpclient.Notifications{OnWarning: func(w mcpclient.Warning) { warned <- w }})
	return ss, c, warned
}

func toolFixture(name, input, output string) map[string]any {
	t := map[string]any{"name": name}
	if input != "" {
		t["inputSchema"] = json.RawMessage(input)
	}
	if output != "" {
		t["outputSchema"] = json.RawMessage(output)
	}
	return t
}

const echoInput = `{"type":"object","properties":{"count":{"type":"integer"},"label":{"type":"string"}},"required":["count"],"additionalProperties":false}`

func TestCallToolRejectsInvalidArgumentsBeforeTheTransport(t *testing.T) {
	ss, c, _ := newSchemaServer(t,
		[]any{toolFixture("echo", echoInput, "")},
		map[string]any{"resultType": "complete", "content": []any{}})
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	if _, err := c.ListTools(ctx); err != nil {
		t.Fatal(err)
	}
	for name, args := range map[string]map[string]any{
		"wrong type":       {"count": "three"},
		"missing required": {"label": "x"},
		"additional prop":  {"count": 1, "nope": true},
	} {
		_, err := c.CallTool(ctx, "echo", args)
		var ipe *mcpclient.InvalidParamsError
		if !errors.As(err, &ipe) {
			t.Fatalf("%s: error is %T (%v), want InvalidParamsError", name, err, err)
		}
		if ipe.RPCCode() != -32602 {
			t.Fatalf("%s: RPCCode %d, want -32602", name, ipe.RPCCode())
		}
		if !strings.Contains(err.Error(), "echo") {
			t.Fatalf("%s: error does not name the tool: %v", name, err)
		}
	}
	if got := ss.calls.Load(); got != 0 {
		t.Fatalf("upstream received %d tools/call requests for invalid arguments, want 0", got)
	}

	// Valid arguments reach the upstream exactly once.
	if _, err := c.CallTool(ctx, "echo", map[string]any{"count": 2, "label": "ok"}); err != nil {
		t.Fatal(err)
	}
	if got := ss.calls.Load(); got != 1 {
		t.Fatalf("upstream received %d tools/call requests for valid arguments, want 1", got)
	}
}

// A tool whose inputSchema fails through a combinator (anyOf) must
// surface the -32602 InvalidParamsError, never panic: the validation
// error tree for combinators has interior units with no message
// (regression from the PR #1596 conformance crash, #1483).
func TestCallToolCombinatorSchemaFailureReturnsInvalidParams(t *testing.T) {
	ss, c, _ := newSchemaServer(t,
		[]any{toolFixture("pick", `{"type":"object","properties":{"mode":{"anyOf":[{"const":"a"},{"const":"b"}]}},"required":["mode"]}`, "")},
		map[string]any{"resultType": "complete", "content": []any{}})
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	if _, err := c.ListTools(ctx); err != nil {
		t.Fatal(err)
	}
	_, err := c.CallTool(ctx, "pick", map[string]any{"mode": "c"})
	var ipe *mcpclient.InvalidParamsError
	if !errors.As(err, &ipe) {
		t.Fatalf("error is %T (%v), want InvalidParamsError", err, err)
	}
	if ipe.RPCCode() != -32602 {
		t.Fatalf("RPCCode %d, want -32602", ipe.RPCCode())
	}
	if !strings.Contains(err.Error(), "$.mode") {
		t.Fatalf("error does not name the failing path: %v", err)
	}
	if got := ss.calls.Load(); got != 0 {
		t.Fatalf("upstream received %d tools/call requests for invalid arguments, want 0", got)
	}
}

func TestListToolsExcludesUncompilableSchemasWithWarning(t *testing.T) {
	_, c, warned := newSchemaServer(t,
		[]any{
			toolFixture("good", echoInput, ""),
			toolFixture("bad-dialect", `{"$schema":"http://json-schema.org/draft-03/schema#","type":"object"}`, ""),
			toolFixture("bad-ref", `{"type":"object","properties":{"x":{"$ref":"https://example.com/ext.json"}}}`, ""),
		},
		map[string]any{"resultType": "complete", "content": []any{}})
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	tools, err := c.ListTools(ctx)
	if err != nil {
		t.Fatal(err)
	}
	if len(tools) != 1 || tools[0].Name != "good" {
		t.Fatalf("listed tools = %v, want only [good]", tools)
	}
	got := map[string]string{}
	for i := 0; i < 2; i++ {
		select {
		case w := <-warned:
			got[w.Tool] = w.Reason
		case <-time.After(2 * time.Second):
			t.Fatalf("only %d warnings received: %v", len(got), got)
		}
	}
	if !strings.Contains(got["bad-dialect"], "dialect") {
		t.Errorf("bad-dialect warning = %q, want the dialect named", got["bad-dialect"])
	}
	if !strings.Contains(got["bad-ref"], "$ref") {
		t.Errorf("bad-ref warning = %q, want the $ref named", got["bad-ref"])
	}
}

func TestCallToolOutputValidationIsWarnOnly(t *testing.T) {
	const out = `{"type":"object","properties":{"total":{"type":"integer"}},"required":["total"]}`
	result := map[string]any{"resultType": "complete",
		"content":           []any{},
		"structuredContent": map[string]any{"total": "not-a-number"}}
	_, c, warned := newSchemaServer(t,
		[]any{toolFixture("sum", `{"type":"object"}`, out)}, result)
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	if _, err := c.ListTools(ctx); err != nil {
		t.Fatal(err)
	}
	raw, err := c.CallTool(ctx, "sum", map[string]any{})
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(raw), "not-a-number") {
		t.Fatalf("result was not delivered unchanged: %s", raw)
	}
	select {
	case w := <-warned:
		if w.Tool != "sum" || !strings.Contains(w.Reason, "outputSchema") {
			t.Fatalf("warning = %+v, want an outputSchema warning for sum", w)
		}
	case <-time.After(2 * time.Second):
		t.Fatal("no warning recorded for a structuredContent violation")
	}
}

// BenchmarkCallToolValidateArgs measures argument validation on a
// representative nested schema (~5 levels, ~20 properties, modelled on
// a catalog tool). The triage budget for #1483: <=100us/op median,
// <=1ms worst per call.
func BenchmarkCallToolValidateArgs(b *testing.B) {
	const nested = `{"type":"object","properties":{
		"name":{"type":"string"},"count":{"type":"integer"},
		"window":{"type":"object","properties":{"start":{"type":"string"},"end":{"type":"string"},"tz":{"type":"string"}},"required":["start","end"]},
		"filters":{"type":"array","items":{"type":"object","properties":{"field":{"type":"string"},"op":{"type":"string"},"value":{"type":"string"}},"required":["field","op"]}},
		"opts":{"type":"object","properties":{"deep":{"type":"object","properties":{"deeper":{"type":"object","properties":{"leaf":{"type":"integer"},"flag":{"type":"boolean"},"mode":{"type":"string"}},"required":["leaf"]},"level":{"type":"integer"}},"required":["deeper"]},"verbose":{"type":"boolean"}}},
		"tags":{"type":"array","items":{"type":"string"}},
		"a":{"type":"string"},"b":{"type":"string"},"c":{"type":"string"},"d":{"type":"string"},"e":{"type":"string"}
	},"required":["name","count"]}`
	ss := &schemaServer{}
	ss.scriptedServer = newScripted(func(method string, params map[string]any) any {
		switch method {
		case "server/discover":
			return modernDiscover(params)
		case "tools/list":
			return map[string]any{"resultType": "complete", "tools": []any{toolFixture("bench", nested, "")}}
		case "tools/call":
			return map[string]any{"resultType": "complete", "content": []any{}}
		}
		return nil
	})
	ctx := context.Background()
	c, err := mcpclient.NewWithOptions(ctx, ss.scriptedServer, mcpclient.Options{
		ClientName: "mcpx", ClientVersion: "bench", Preference: mcpclient.ForceModern})
	if err != nil {
		b.Fatal(err)
	}
	defer c.Close()
	if _, err := c.ListTools(ctx); err != nil {
		b.Fatal(err)
	}
	args := map[string]any{
		"name": "x", "count": 3,
		"window":  map[string]any{"start": "a", "end": "b", "tz": "UTC"},
		"filters": []any{map[string]any{"field": "f", "op": "=", "value": "1"}},
		"opts":    map[string]any{"deep": map[string]any{"deeper": map[string]any{"leaf": 1, "flag": true, "mode": "m"}, "level": 2}},
	}
	b.ResetTimer()
	for i := 0; i < b.N; i++ {
		if _, err := c.CallTool(ctx, "bench", args); err != nil {
			b.Fatal(err)
		}
	}
}

// An explicit null for a property the schema does not require is an
// omission, not a type violation (SEP-2243: a null parameter's header
// is omitted, and the conformance suite calls a boolean-typed optional
// parameter with null). A null for a required property is still
// validated as sent and fails here, because the schema does not admit
// null for it.
func TestCallToolNullForOptionalPropertyIsAnOmission(t *testing.T) {
	ss, c, _ := newSchemaServer(t,
		[]any{toolFixture("opts", `{"type":"object","properties":{"region":{"type":"string"},"verbose":{"type":"boolean"}},"required":["region"],"additionalProperties":false}`, "")},
		map[string]any{"resultType": "complete", "content": []any{}})
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	if _, err := c.ListTools(ctx); err != nil {
		t.Fatal(err)
	}
	if _, err := c.CallTool(ctx, "opts", map[string]any{"region": "us-east1", "verbose": nil}); err != nil {
		t.Fatalf("null for optional property: %v", err)
	}
	if got := ss.calls.Load(); got != 1 {
		t.Fatalf("upstream received %d tools/call requests, want 1", got)
	}
	_, err := c.CallTool(ctx, "opts", map[string]any{"region": nil})
	var ipe *mcpclient.InvalidParamsError
	if !errors.As(err, &ipe) {
		t.Fatalf("null for required property: error is %T (%v), want InvalidParamsError", err, err)
	}
	if got := ss.calls.Load(); got != 1 {
		t.Fatalf("upstream received %d tools/call requests after the rejected call, want 1", got)
	}
}
