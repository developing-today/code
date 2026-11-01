package daemon

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/dezren39/mcpx/internal/artifacts"
)

// #2327: a per-request ttl on POST /v1/artifacts may shorten retention
// but never extend it past the operator's artifacts.ttl. The store
// here is opened with a 2h ceiling (not the 24h default) so the test
// proves the handler reads the operator's configured value.
func TestHandleArtifactPutTTLCeiling(t *testing.T) {
	store, err := artifacts.Open(artifacts.Options{Dir: t.TempDir(), TTL: 2 * time.Hour})
	if err != nil {
		t.Fatal(err)
	}
	defer store.Close()
	s := &Server{}
	s.artifactStore = store
	s.artifactsOnce.Do(func() {})

	put := func(target string) *httptest.ResponseRecorder {
		req := httptest.NewRequest(http.MethodPost, target, strings.NewReader("bytes"))
		rec := httptest.NewRecorder()
		s.handleArtifactPut(rec, req)
		return rec
	}
	stored := func() int {
		list, err := store.List(artifacts.Filter{})
		if err != nil {
			t.Fatal(err)
		}
		return len(list)
	}

	// Over the ceiling: 400 naming both values, and nothing stored.
	rec := put("/v1/artifacts?name=over.txt&ttl=876000h")
	if rec.Code != http.StatusBadRequest {
		t.Fatalf("over-ceiling ttl: status %d, want 400 (body %q)", rec.Code, rec.Body.String())
	}
	if body := rec.Body.String(); !strings.Contains(body, "876000") || !strings.Contains(body, "artifacts.ttl") || !strings.Contains(body, "2h") {
		t.Fatalf("over-ceiling ttl: body must name the requested ttl and the ceiling, got %q", body)
	}
	if n := stored(); n != 0 {
		t.Fatalf("over-ceiling ttl stored %d artifacts, want 0", n)
	}

	cases := []struct {
		name    string
		target  string
		wantTTL time.Duration
	}{
		{"under ceiling", "/v1/artifacts?name=under.txt&ttl=1h", time.Hour},
		{"at ceiling", "/v1/artifacts?name=at.txt&ttl=2h", 2 * time.Hour},
		{"no ttl uses operator default", "/v1/artifacts?name=default.txt", 2 * time.Hour},
		{"negative ttl uses operator default", "/v1/artifacts?name=neg.txt&ttl=-1h", 2 * time.Hour},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			rec := put(tc.target)
			if rec.Code != http.StatusCreated {
				t.Fatalf("status %d, want 201 (body %q)", rec.Code, rec.Body.String())
			}
			var meta artifacts.Meta
			if err := json.Unmarshal(rec.Body.Bytes(), &meta); err != nil {
				t.Fatalf("decode meta: %v", err)
			}
			got := meta.Expires.Sub(meta.Created)
			if diff := got - tc.wantTTL; diff < -time.Second || diff > time.Second {
				t.Fatalf("stored ttl %s, want %s", got, tc.wantTTL)
			}
		})
	}
	if n := stored(); n != len(cases) {
		t.Fatalf("stored %d artifacts, want %d", n, len(cases))
	}
}
