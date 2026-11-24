package logstore

import (
	"path/filepath"
	"testing"
	"time"
)

func newIdentityStore(t *testing.T) *FeedbackStore {
	t.Helper()
	fs, err := NewFeedbackStore(filepath.Join(t.TempDir(), "feedback.db"))
	if err != nil {
		t.Fatalf("NewFeedbackStore: %v", err)
	}
	t.Cleanup(func() { _ = fs.Close() })
	return fs
}

func recordAndScore(t *testing.T, fs *FeedbackStore, rec InteractionRecord, params FeedbackParams) {
	t.Helper()
	if err := fs.RecordInteraction(rec); err != nil {
		t.Fatalf("RecordInteraction(%s): %v", rec.TraceID, err)
	}
	params.TraceID = rec.TraceID
	if err := fs.ProvideFeedback(params); err != nil {
		t.Fatalf("ProvideFeedback(%s): %v", rec.TraceID, err)
	}
}

func f64(v float64) *float64 { return &v }

// Scoring tool A must move A's multipliers only: a longer tool name that A is
// a prefix of, and unrelated tools, keep neutral multipliers — and A's own
// average must not be diluted by the longer tool's rows.
func TestToolIdentityExactMatching(t *testing.T) {
	fs := newIdentityStore(t)

	recordAndScore(t, fs, InteractionRecord{
		TraceID: "a", Timestamp: time.Now(), Source: "call",
		Input: "shot", ToolsUsed: []string{"browser.screenshot"},
	}, FeedbackParams{Score: 0.0, Target: TargetExecution, ExecutionScore: f64(0.0)})

	recordAndScore(t, fs, InteractionRecord{
		TraceID: "a-suffix", Timestamp: time.Now(), Source: "call",
		Input: "shot again", ToolsUsed: []string{"browser.screenshot2"},
	}, FeedbackParams{Score: 1.0, Target: TargetExecution, ExecutionScore: f64(1.0)})

	recordAndScore(t, fs, InteractionRecord{
		TraceID: "other", Timestamp: time.Now(), Source: "call",
		Input: "read", ToolsUsed: []string{"files.read"},
	}, FeedbackParams{Score: 1.0, Target: TargetExecution, ExecutionScore: f64(1.0)})

	if exec, _ := fs.ToolFeedbackAdjustment("browser.screenshot"); exec != 0.5 {
		t.Fatalf("browser.screenshot execMult = %v, want 0.5 (only its own 0.0 row)", exec)
	}
	if exec, _ := fs.ToolFeedbackAdjustment("browser.screenshot2"); exec != 1.5 {
		t.Fatalf("browser.screenshot2 execMult = %v, want 1.5 (only its own 1.0 row)", exec)
	}
	if exec, retr := fs.ToolFeedbackAdjustment("browser.screensho"); exec != 1.0 || retr != 1.0 {
		t.Fatalf("prefix-only name got (%v, %v), want neutral (1.0, 1.0)", exec, retr)
	}
	if exec, retr := fs.ToolFeedbackAdjustment("never.seen"); exec != 1.0 || retr != 1.0 {
		t.Fatalf("unrelated tool got (%v, %v), want neutral (1.0, 1.0)", exec, retr)
	}
}

// Rows stored before canonicalisation keep the colon spelling; they must
// still answer queries in either spelling, and new records written with the
// colon spelling are stored canonically.
func TestToolIdentityHistoricalColonSpelling(t *testing.T) {
	fs := newIdentityStore(t)

	// A legacy row, written straight to the table the way pre-change code did.
	if _, err := fs.db.Exec(`
		INSERT INTO interactions (trace_id, ts_unix_ms, source, input_text, tools_used, score, target, execution_score)
		VALUES ('legacy', ?, 'call', 'shot', 'browser:screenshot', 0.0, 'execution', 0.0)
	`, time.Now().UnixMilli()); err != nil {
		t.Fatalf("insert legacy row: %v", err)
	}
	if exec, _ := fs.ToolFeedbackAdjustment("browser.screenshot"); exec != 0.5 {
		t.Fatalf("dot query over colon row: execMult = %v, want 0.5", exec)
	}
	if exec, _ := fs.ToolFeedbackAdjustment("browser:screenshot"); exec != 0.5 {
		t.Fatalf("colon query over colon row: execMult = %v, want 0.5", exec)
	}

	// A new record written with the colon spelling is stored canonically.
	if err := fs.RecordInteraction(InteractionRecord{
		TraceID: "new", Timestamp: time.Now(), Source: "call",
		Input: "shot", ToolsUsed: []string{"browser:screenshot"},
	}); err != nil {
		t.Fatalf("RecordInteraction: %v", err)
	}
	got, err := fs.Get("new")
	if err != nil {
		t.Fatalf("Get: %v", err)
	}
	if len(got.ToolsUsed) != 1 || got.ToolsUsed[0] != "browser.screenshot" {
		t.Fatalf("stored identity = %v, want [browser.screenshot]", got.ToolsUsed)
	}
}

// A view's function name and the server identity behind it are aliases for
// the same tool: feedback recorded under the server spelling must answer a
// ranking query that supplies the alias, and must not leak to the view name
// without it.
func TestToolIdentityNamespaceAlias(t *testing.T) {
	fs := newIdentityStore(t)

	recordAndScore(t, fs, InteractionRecord{
		TraceID: "srv", Timestamp: time.Now(), Source: "call",
		Input: "shot", ToolsUsed: []string{"browser:screenshot"},
	}, FeedbackParams{Score: 1.0, Target: TargetRetrieval, RetrievalScore: f64(1.0)})

	if _, retr := fs.ToolFeedbackAdjustment("chrome.screenshot", "browser.screenshot"); retr != 1.5 {
		t.Fatalf("aliased query retrMult = %v, want 1.5", retr)
	}
	if _, retr := fs.ToolFeedbackAdjustment("chrome.screenshot"); retr != 1.0 {
		t.Fatalf("unaliased query retrMult = %v, want neutral 1.0", retr)
	}
}

// Offered tools are not invocations: execution feedback on an interaction
// that only offered a tool must not move its execution multiplier, while
// retrieval feedback on the offered set does move the retrieval multiplier
// and feeds correlated queries.
func TestOfferedToolsAreNotInvocations(t *testing.T) {
	fs := newIdentityStore(t)

	recordAndScore(t, fs, InteractionRecord{
		TraceID: "offered-exec", Timestamp: time.Now(), Source: "proxy",
		Input: "take a screenshot", ToolsOffered: []string{"browser.screenshot"},
	}, FeedbackParams{Score: 0.0, Target: TargetExecution, ExecutionScore: f64(0.0)})

	if exec, _ := fs.ToolFeedbackAdjustment("browser.screenshot"); exec != 1.0 {
		t.Fatalf("offered-only execution feedback moved execMult to %v, want 1.0", exec)
	}

	recordAndScore(t, fs, InteractionRecord{
		TraceID: "offered-retr", Timestamp: time.Now(), Source: "proxy",
		Input: "take a screenshot", ToolsOffered: []string{"browser.screenshot"},
	}, FeedbackParams{Score: 1.0, Target: TargetRetrieval, RetrievalScore: f64(1.0)})

	if _, retr := fs.ToolFeedbackAdjustment("browser.screenshot"); retr != 1.5 {
		t.Fatalf("offered retrieval feedback: retrMult = %v, want 1.5", retr)
	}
	queries := fs.FindCorrelatedQueries("browser.screenshot", 0.8)
	if len(queries) != 1 || queries[0] != "take a screenshot" {
		t.Fatalf("correlated queries = %v, want [take a screenshot]", queries)
	}

	got, err := fs.Get("offered-retr")
	if err != nil {
		t.Fatalf("Get: %v", err)
	}
	if len(got.ToolsUsed) != 0 || len(got.ToolsOffered) != 1 || got.ToolsOffered[0] != "browser.screenshot" {
		t.Fatalf("round trip = used %v offered %v, want used [] offered [browser.screenshot]", got.ToolsUsed, got.ToolsOffered)
	}
}
