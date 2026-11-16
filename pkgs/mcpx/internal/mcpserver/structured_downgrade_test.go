package mcpserver_test

import (
	"strings"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/mcpserver"
)

// #1974 headline 7: 2025-06-18..2025-11-25 define structuredContent as an
// object only, while 2026-07-28 allows any JSON value. A non-object value
// must not reach an object-only host in the field; it is rendered into
// content instead, so the data survives in a shape the host can read.
// Oracle: the receiving revision's own CallToolResult schema
// (tools-structured-object-only / tools-structured-any-json conformance
// rows).
func TestNonObjectStructuredContentIsRenderedForObjectOnlyHosts(t *testing.T) {
	srv := mcpserver.New(nil, "mcpx", "test")
	result := map[string]any{
		"content":           []any{map[string]any{"type": "text", "text": "hello"}},
		"structuredContent": []any{1, 2, 3},
	}
	for _, version := range []string{"2025-06-18", "2025-11-25"} {
		out := protoJSON(t, srv.Downgrade(result, version))
		if strings.Contains(out, "structuredContent") {
			t.Errorf("%s is object-only and must not be sent a non-object structuredContent:\n%s", version, out)
		}
		if !strings.Contains(out, "1") || !strings.Contains(out, "3") {
			t.Errorf("%s: the data should survive rendered as text:\n%s", version, out)
		}
	}

	// An object value is exactly what those revisions define: kept.
	obj := map[string]any{
		"content":           []any{map[string]any{"type": "text", "text": "hello"}},
		"structuredContent": map[string]any{"n": 42},
	}
	for _, version := range []string{"2025-06-18", "2025-11-25", "2026-07-28"} {
		out := protoJSON(t, srv.Downgrade(obj, version))
		if !strings.Contains(out, "structuredContent") {
			t.Errorf("%s defines object structuredContent and should keep it:\n%s", version, out)
		}
	}

	// A scalar value is a non-object too.
	scalar := map[string]any{"structuredContent": "just text"}
	out := protoJSON(t, srv.Downgrade(scalar, "2025-11-25"))
	if strings.Contains(out, "structuredContent") {
		t.Errorf("a scalar structuredContent must not reach a 2025-11-25 host in the field:\n%s", out)
	}
	if !strings.Contains(out, "just text") {
		t.Errorf("the scalar should survive rendered as text:\n%s", out)
	}
}
