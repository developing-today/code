package mcpclient_test

import (
	"net/http"
	"net/http/httptest"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/mcpclient"
)

// Fleet context passthrough (build 4/5, #14190): the originating queue
// task id rides an MCPX-mediated tool call to the far side -- as
// X-Fleet-* headers where headers travel (Streamable HTTP) and as
// dev.fleet/* _meta on every transport -- with no model in the loop.
// Absence is representable: no ctx and no env stamps nothing, not an error.

func fleetEchoTool() []any {
	return []any{map[string]any{"name": "echo", "inputSchema": map[string]any{"type": "object"}}}
}

func fleetMetaOf(t *testing.T, s seen) map[string]any {
	t.Helper()
	p, _ := s.body["params"].(map[string]any)
	if p == nil {
		return nil
	}
	m, _ := p["_meta"].(map[string]any)
	return m
}

func TestFleetPassthroughModern(t *testing.T) {
	rec := &recorder{}
	srv := modernHTTP(t, rec, fleetEchoTool, func(s seen) (int, any) {
		return 200, result(s.id, map[string]any{"content": []any{}})
	})
	c := dialURL(t, srv.URL, mcpclient.ForceModern, mcpclient.Options{})

	ctx := mcpclient.WithFleetCtx(ctx5(), mcpclient.FleetCtx{TaskID: "22547", TaskName: "mcpx-passthrough"})
	if _, err := c.CallTool(ctx, "echo", map[string]any{}); err != nil {
		t.Fatal(err)
	}
	calls := rec.all("tools/call")
	if len(calls) == 0 {
		t.Fatal("far side saw no tools/call")
	}
	last := calls[len(calls)-1]
	if got := last.header.Get(mcpclient.FleetHeaderTaskID); got != "22547" {
		t.Errorf("X-Fleet-Task-Id = %q, want %q", got, "22547")
	}
	if got := last.header.Get(mcpclient.FleetHeaderTaskName); got != "mcpx-passthrough" {
		t.Errorf("X-Fleet-Task-Name = %q, want %q", got, "mcpx-passthrough")
	}
	meta := fleetMetaOf(t, last)
	if meta[mcpclient.FleetMetaTaskID] != "22547" {
		t.Errorf("_meta %q = %v, want %q", mcpclient.FleetMetaTaskID, meta[mcpclient.FleetMetaTaskID], "22547")
	}
	if meta[mcpclient.FleetMetaTaskName] != "mcpx-passthrough" {
		t.Errorf("_meta %q = %v, want %q", mcpclient.FleetMetaTaskName, meta[mcpclient.FleetMetaTaskName], "mcpx-passthrough")
	}
}

func TestFleetPassthroughLegacy(t *testing.T) {
	rec := &recorder{}
	srv := httptest.NewServer(&legacyHTTP{rec: rec, version: "2025-11-25", onCall: func(w http.ResponseWriter, s seen) {
		writeJSON(w, result(s.id, map[string]any{"content": []any{}}))
	}})
	t.Cleanup(srv.Close)
	c := dialURL(t, srv.URL, mcpclient.ForceLegacy, mcpclient.Options{})

	ctx := mcpclient.WithFleetCtx(ctx5(), mcpclient.FleetCtx{TaskID: "22547"})
	if _, err := c.CallTool(ctx, "echo", nil); err != nil {
		t.Fatal(err)
	}
	calls := rec.all("tools/call")
	if len(calls) == 0 {
		t.Fatal("far side saw no tools/call")
	}
	last := calls[len(calls)-1]
	if got := last.header.Get(mcpclient.FleetHeaderTaskID); got != "22547" {
		t.Errorf("X-Fleet-Task-Id = %q, want %q", got, "22547")
	}
	if meta := fleetMetaOf(t, last); meta[mcpclient.FleetMetaTaskID] != "22547" {
		t.Errorf("legacy _meta %q = %v, want %q", mcpclient.FleetMetaTaskID, meta[mcpclient.FleetMetaTaskID], "22547")
	}
}

func TestFleetEnvFallbackFarSide(t *testing.T) {
	t.Setenv("FLEET_CTX_TASK_ID", "22547")
	t.Setenv("FLEET_CTX_TASK_NAME", "mcpx-passthrough")
	rec := &recorder{}
	srv := modernHTTP(t, rec, fleetEchoTool, func(s seen) (int, any) {
		return 200, result(s.id, map[string]any{"content": []any{}})
	})
	c := dialURL(t, srv.URL, mcpclient.ForceModern, mcpclient.Options{})

	// No ctx: the stdio/no-header path resolves the process env.
	if _, err := c.CallTool(ctx5(), "echo", map[string]any{}); err != nil {
		t.Fatal(err)
	}
	calls := rec.all("tools/call")
	last := calls[len(calls)-1]
	if got := last.header.Get(mcpclient.FleetHeaderTaskID); got != "22547" {
		t.Errorf("env X-Fleet-Task-Id = %q, want %q", got, "22547")
	}
	if meta := fleetMetaOf(t, last); meta[mcpclient.FleetMetaTaskID] != "22547" {
		t.Errorf("env _meta %q = %v, want %q", mcpclient.FleetMetaTaskID, meta[mcpclient.FleetMetaTaskID], "22547")
	}
}

func TestFleetAbsenceStampsNothing(t *testing.T) {
	t.Setenv("FLEET_CTX_TASK_ID", "")
	t.Setenv("FLEET_CTX_TASK_NAME", "")
	rec := &recorder{}
	srv := modernHTTP(t, rec, fleetEchoTool, func(s seen) (int, any) {
		return 200, result(s.id, map[string]any{"content": []any{}})
	})
	c := dialURL(t, srv.URL, mcpclient.ForceModern, mcpclient.Options{})

	if _, err := c.CallTool(ctx5(), "echo", map[string]any{}); err != nil {
		t.Fatal(err)
	}
	calls := rec.all("tools/call")
	last := calls[len(calls)-1]
	if got := last.header.Get(mcpclient.FleetHeaderTaskID); got != "" {
		t.Errorf("absent X-Fleet-Task-Id = %q, want none", got)
	}
	if meta := fleetMetaOf(t, last); len(meta) != 0 {
		// _meta may carry protocol keys (version, log level); it must
		// carry no fleet keys.
		for _, k := range []string{mcpclient.FleetMetaTaskID, mcpclient.FleetMetaTaskName} {
			if _, ok := meta[k]; ok {
				t.Errorf("absent _meta carries %q: %v", k, meta)
			}
		}
	}
	if f := mcpclient.FleetCtxFromEnv(); !f.Empty() {
		t.Errorf("FleetCtxFromEnv = %+v, want empty", f)
	}
}
