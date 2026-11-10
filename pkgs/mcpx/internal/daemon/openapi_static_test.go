package daemon

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"testing"
)

// #1883 item 4 (via #2515): the static variant of /v1/openapi.json
// returns only the ops-table paths; the live default still merges the
// running daemon's per-tool paths.
func TestOpenAPIStaticVariantOmitsLiveToolPaths(t *testing.T) {
	s := intentTestServer()

	get := func(target string) map[string]any {
		t.Helper()
		rec := httptest.NewRecorder()
		s.handleOpenAPI(rec, httptest.NewRequest(http.MethodGet, target, nil))
		if rec.Code != http.StatusOK {
			t.Fatalf("GET %s: status %d", target, rec.Code)
		}
		var doc map[string]any
		if err := json.Unmarshal(rec.Body.Bytes(), &doc); err != nil {
			t.Fatalf("GET %s: decode: %v", target, err)
		}
		paths, ok := doc["paths"].(map[string]any)
		if !ok {
			t.Fatalf("GET %s: no paths object", target)
		}
		return paths
	}

	live := get("/v1/openapi.json")
	if _, ok := live["/v1/call/weather/forecast"]; !ok {
		t.Errorf("live document is missing the per-tool path /v1/call/weather/forecast")
	}

	for _, target := range []string{"/v1/openapi.json?static=1", "/v1/openapi.json?static=true"} {
		paths := get(target)
		if _, ok := paths["/v1/call/weather/forecast"]; ok {
			t.Errorf("%s: static document contains a live per-tool path", target)
		}
		if len(paths) == 0 {
			t.Errorf("%s: static document has no ops-table paths at all", target)
		}
	}
}
