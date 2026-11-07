package logstore

import (
	"math"
	"path/filepath"
	"strconv"
	"testing"
	"time"
)

// parseScoreParam replicates handleInteractionsList's min/max parsing
// (internal/daemon/routes_feedback.go): a failed parse leaves the filter
// field unset; a successful one sets it, no finiteness check.
func parseScoreParam(s string) *float64 {
	if s == "" {
		return nil
	}
	if val, err := strconv.ParseFloat(s, 64); err == nil {
		return &val
	}
	return nil
}

func TestProbeLaneBFeedbackScoreFilter(t *testing.T) {
	fs, err := NewFeedbackStore(filepath.Join(t.TempDir(), "fb.db"))
	if err != nil {
		t.Fatal(err)
	}
	for i, sc := range []float64{0.2, 0.5, 0.9} {
		s := sc
		err := fs.RecordInteraction(InteractionRecord{
			TraceID:   string(rune('a' + i)),
			Timestamp: time.UnixMilli(int64(1000 + i)),
			Source:    "probe",
			Input:     "x",
			Score:     &s,
		})
		if err != nil {
			t.Fatal(err)
		}
	}
	count := func(f InteractionFilter) int {
		list, err := fs.List(f)
		if err != nil {
			t.Fatalf("List error: %v", err)
		}
		return len(list)
	}
	t.Logf("total (no filter): %d", count(InteractionFilter{}))
	t.Logf("minScore=0.4 control (expect 2): %d", count(InteractionFilter{MinScore: parseScoreParam("0.4")}))
	t.Logf("minScore=abc filterSet=%v rows=%d", parseScoreParam("abc") != nil, count(InteractionFilter{MinScore: parseScoreParam("abc")}))
	nan := parseScoreParam("NaN")
	t.Logf("minScore=NaN filterSet=%v isNaN=%v rows=%d", nan != nil, nan != nil && math.IsNaN(*nan), count(InteractionFilter{MinScore: nan}))
	t.Logf("minScore=Inf rows=%d", count(InteractionFilter{MinScore: parseScoreParam("Inf")}))
	t.Logf("minScore=1e999 filterSet=%v rows=%d", parseScoreParam("1e999") != nil, count(InteractionFilter{MinScore: parseScoreParam("1e999")}))
	t.Logf("maxScore=xyz filterSet=%v rows=%d", parseScoreParam("xyz") != nil, count(InteractionFilter{MaxScore: parseScoreParam("xyz")}))
	t.Logf("maxScore=NaN rows=%d", count(InteractionFilter{MaxScore: parseScoreParam("NaN")}))
}
