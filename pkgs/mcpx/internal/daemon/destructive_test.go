package daemon

import (
	"encoding/json"
	"testing"

	"github.com/dezren39/mcpx/internal/config"
	"github.com/dezren39/mcpx/internal/mcpclient"
	"github.com/dezren39/mcpx/internal/pool"
)

// The specification's ToolAnnotations: destructiveHint defaults to true and
// is meaningful only when readOnlyHint is false. An absent hint was read as
// non-destructive, so elicit.confirmDestructive let unannotated tools through.
func TestDestructiveHintUsesTheSpecDefaults(t *testing.T) {
	for _, c := range []struct {
		raw  string
		want bool
	}{
		{``, true},
		{`{}`, true},
		{`{"title":"x"}`, true},
		{`{"readOnlyHint":false}`, true},
		{`{"destructiveHint":true}`, true},
		{`{"destructiveHint":false}`, false},
		{`{"readOnlyHint":true}`, false},
		{`{"readOnlyHint":true,"destructiveHint":true}`, false},
	} {
		if got := destructiveHint(json.RawMessage(c.raw)); got != c.want {
			t.Errorf("destructiveHint(%q) = %v, want %v", c.raw, got, c.want)
		}
	}
}

// Annotations are hints, not decisions (#2191): a remote upstream's
// claim that its tool is read-only or non-destructive must never clear
// the confirmation, because a lying server could suppress every ask.
// Only a locally declared (stdio) server's annotations are taken at
// face value -- that server is the user's own configured command.
func TestAnnotationClearsCallOnlyForLocallyDeclaredServers(t *testing.T) {
	tool := func(raw string) *mcpclient.Tool {
		return &mcpclient.Tool{Name: "wipe", Annotations: json.RawMessage(raw)}
	}
	local := pool.New(&config.Resolved{Server: &config.Server{Command: "demo"}})
	remote := pool.New(&config.Resolved{Server: &config.Server{URL: "https://upstream.example/mcp"}})
	for _, c := range []struct {
		name string
		p    *pool.Pool
		raw  string
		want bool
	}{
		{"local read-only clears", local, `{"readOnlyHint":true}`, true},
		{"local non-destructive clears", local, `{"destructiveHint":false}`, true},
		{"local destructive does not clear", local, `{"destructiveHint":true}`, false},
		{"local unannotated does not clear", local, ``, false},
		{"remote read-only does not clear", remote, `{"readOnlyHint":true}`, false},
		{"remote non-destructive does not clear", remote, `{"destructiveHint":false}`, false},
		{"remote read-only and non-destructive does not clear", remote, `{"readOnlyHint":true,"destructiveHint":false}`, false},
	} {
		t.Run(c.name, func(t *testing.T) {
			if got := annotationClearsCall(c.p, tool(c.raw)); got != c.want {
				t.Errorf("annotationClearsCall(%s) = %v, want %v", c.raw, got, c.want)
			}
		})
	}
	if annotationClearsCall(nil, tool(`{"readOnlyHint":true}`)) {
		t.Error("a nil pool must never clear a call")
	}
}
