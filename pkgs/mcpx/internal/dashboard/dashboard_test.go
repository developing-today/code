package dashboard

import (
	"strings"
	"testing"
)

// TestDashboardHTMLElicitationRefreshPreservesInput guards issue #3223:
// loadElicitations must not rebuild the elicitation list on a refresh
// whose pending set is unchanged, and must preserve in-progress field
// values keyed by (request id, field name) when the set does change.
// The behavioral check (type a value, refresh, assert it survives) is
// exercised against the rendered page in the fix PR for #3223; this
// test pins the mechanisms that behavior depends on so a future edit
// cannot silently drop them.
func TestDashboardHTMLElicitationRefreshPreservesInput(t *testing.T) {
	html := HTML("test")
	for _, want := range []string{
		// Change detection: signature of the pending set as last rendered.
		"lastElicitSignature",
		"signature === lastElicitSignature",
		// Value/focus capture and restore across a forced re-render.
		"captureElicitState(list)",
		"restoreElicitState(list, saved)",
	} {
		if !strings.Contains(html, want) {
			t.Errorf("dashboard HTML missing elicitation preservation mechanism %q", want)
		}
	}
}
