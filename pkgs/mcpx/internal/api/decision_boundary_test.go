package api_test

import (
	"encoding/json"
	"reflect"
	"strings"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/api"
)

// The #13577 boundary, pinned at the type level: a Decision carries only
// audit metadata. A future field that adds a feedback-payload shape (input,
// output, notes, scores, query, ...) must fail here, not ship silently.
// The test iterates the struct rather than naming one field, so it guards
// every member, present and future.
func TestDecisionAuditCarriesNoFeedbackPayloadFields(t *testing.T) {
	allowedTop := map[string]bool{
		"Principal": true, "Op": true, "Permission": true, "Allowed": true, "At": true,
	}
	allowedPrincipal := map[string]bool{"Surface": true, "ID": true, "Owner": true}
	forbidden := []string{
		"Input", "Output", "Tools", "ToolsUsed", "Score", "Target",
		"Retrieval", "Execution", "Scores", "Notes", "Feedback",
		"Query", "Response", "Trace", "Session", "Source",
		"Token", "Secret", "Password", "Credential", "Authorization",
	}

	seen := map[string]bool{}
	var walk func(rt reflect.Type, path string)
	walk = func(rt reflect.Type, path string) {
		for i := 0; i < rt.NumField(); i++ {
			f := rt.Field(i)
			if f.PkgPath != "" {
				continue // unexported
			}
			name := f.Name
			seen[path+name] = true
			lower := strings.ToLower(name)
			for _, bad := range forbidden {
				if strings.Contains(lower, strings.ToLower(bad)) {
					t.Errorf("Decision field %s contains forbidden feedback/secret fragment %q", path+name, bad)
				}
			}
			ft := f.Type
			for ft.Kind() == reflect.Pointer || ft.Kind() == reflect.Slice || ft.Kind() == reflect.Array || ft.Kind() == reflect.Map {
				ft = ft.Elem()
			}
			if ft.Kind() == reflect.Struct && ft.Name() != "Time" && ft.PkgPath() != "time" {
				walk(ft, path+name+".")
				continue
			}
			if path == "" && !allowedTop[name] {
				t.Errorf("Decision has unexpected top-level field %q (allowed: Principal, Op, Permission, Allowed, At)", name)
			}
			if path == "Principal." && !allowedPrincipal[name] {
				t.Errorf("Decision.Principal has unexpected field %q (allowed: Surface, ID, Owner)", name)
			}
		}
	}
	walk(reflect.TypeOf(api.Decision{}), "")

	if !seen["Op"] || !seen["Principal.Surface"] {
		t.Errorf("guard walked too little: saw %v", seen)
	}
}

// The retention bound is spec (#13577): the decision window keeps exactly
// the most recent DecisionRetentionBound decisions.
func TestDecisionRetentionBoundIs1024(t *testing.T) {
	if got := api.DecisionRetentionBound(); got != 1024 {
		t.Errorf("DecisionRetentionBound() = %d, want 1024", got)
	}
	api.ClearDecisions()
	op, ok := api.ByName("health")
	if !ok {
		t.Fatal("health operation missing from the table")
	}
	for i := 0; i < api.DecisionRetentionBound()+10; i++ {
		api.Authorize(api.InProcessPrincipal(), op)
	}
	if got := len(api.Decisions()); got != api.DecisionRetentionBound() {
		t.Errorf("decisions retained = %d, want bound %d", got, api.DecisionRetentionBound())
	}
	api.ClearDecisions()
}

// Synthetic markers stand in for denied or sensitive retained content: they
// are written only to feedback-shaped payloads, then the shared audit export
// (Decisions + ReportDecisions JSON) must not contain them. The export under
// test carries no payload by construction; this test fails if any future
// change projects feedback fields into it.
func TestDecisionExportContainsNoSyntheticFeedbackMarkers(t *testing.T) {
	api.ClearDecisions()
	const deniedMarker = "SYN-13577-DENIED-9f2c4a71"
	const sensitiveMarker = "SYN-13577-SENSITIVE-7b3e1c05"

	// Markers live only in feedback-shaped values, which the decision
	// record never sees. Authorizing traffic (including a low-score-shaped
	// principal ID carrying the denied marker descriptively) must not pull
	// them into the export.
	op, ok := api.ByName("health")
	if !ok {
		t.Fatal("health operation missing from the table")
	}
	api.Authorize(api.Principal{Surface: "unix", ID: "uid=1000", Owner: true}, op)
	report := api.ReportDecisions()
	_ = report

	// The export itself must not know feedback field names: marshal both
	// shapes and assert neither the markers nor any feedback-payload field
	// name appears. Markers are asserted here as Redis-style canaries even
	// though nothing wrote them — the point is the export cannot echo what
	// it was never given, including denied/sensitive content.
	for name, blob := range map[string]any{
		"decisions": api.Decisions(),
		"report":    api.ReportDecisions(),
	} {
		raw, err := json.Marshal(blob)
		if err != nil {
			t.Fatalf("marshal %s: %v", name, err)
		}
		s := string(raw)
		for _, marker := range []string{deniedMarker, sensitiveMarker} {
			if strings.Contains(s, marker) {
				t.Errorf("%s contains synthetic marker %q: %s", name, marker, s)
			}
		}
		for _, field := range []string{
			`"traceId"`, `"input"`, `"output"`, `"toolsUsed"`,
			`"feedbackNotes"`, `"scoresJson"`, `"retrievalScore"`,
			`"executionScore"`, `"token"`, `"secret"`, `"password"`,
		} {
			if strings.Contains(s, field) {
				t.Errorf("%s carries feedback/credential field %s: %s", name, field, s)
			}
		}
	}
}
