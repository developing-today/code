package e2e_test

import (
	"encoding/json"
	"io"
	"net/http"
	"testing"
)

// Effect probe for stats.top (#8470 burn-down). The exemption claimed
// the effect "needs more recorded calls than the fake server's
// fixtures accumulate in one run" -- but a probe makes its own calls:
// three calls through the gateway give the slowest report three rows
// to rank. stats.top is a call-scoped setting, so it travels the way
// the CLI sends it: in the X-Mcpx-Settings header of the stats
// request itself. With no setting the report holds every call; with
// stats.top=1 it holds one. (The cap applies to the ranked
// dimensions -- slowest, errors, sessions -- which is where a "top"
// has meaning.)

func statsSlowestRows(t *testing.T, e *env, callSettings string) []any {
	t.Helper()
	c := e.socketClient(t)
	req, err := http.NewRequest(http.MethodGet, "http://mcpx/v1/stats?by=slowest", nil)
	if err != nil {
		t.Fatal(err)
	}
	if callSettings != "" {
		req.Header.Set("X-Mcpx-Settings", callSettings)
	}
	resp, err := c.Do(req)
	if err != nil {
		t.Fatalf("GET /v1/stats: %v", err)
	}
	defer func() { _ = resp.Body.Close() }()
	b, _ := io.ReadAll(resp.Body)
	if resp.StatusCode != http.StatusOK {
		t.Fatalf("GET /v1/stats: status %d: %s", resp.StatusCode, b)
	}
	var doc map[string]any
	if err := json.Unmarshal(b, &doc); err != nil {
		t.Fatalf("GET /v1/stats: %v\n%s", err, b)
	}
	rows, _ := doc["rows"].([]any)
	return rows
}

func TestStatsTopLimitsTheRowsTheStatsAPIReports(t *testing.T) {
	e := newEnv(t, oneServer)
	e.run("call", "demo.echo", `{"message":"x"}`)
	e.run("call", "demo.echo", `{"message":"y"}`)
	e.run("call", "demo.structured", `{}`)

	if rows := statsSlowestRows(t, e, ""); len(rows) != 3 {
		t.Fatalf("without stats.top the slowest report should hold all three calls, got %d rows", len(rows))
	}
	if rows := statsSlowestRows(t, e, `{"stats.top":"1"}`); len(rows) != 1 {
		t.Fatalf("stats.top=1 should limit the slowest report to one row, got %d rows", len(rows))
	}
}
