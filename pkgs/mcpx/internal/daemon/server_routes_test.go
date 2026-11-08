package daemon

import (
	"fmt"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync"
	"testing"
)

// postServerAdd sends one POST /v1/servers add for name.
func postServerAdd(s *Server, name string) *httptest.ResponseRecorder {
	body := fmt.Sprintf(`{"name":%q,"command":"/bin/true"}`, name)
	req := httptest.NewRequest(http.MethodPost, "/v1/servers", strings.NewReader(body))
	rec := httptest.NewRecorder()
	s.handleServersAdd(rec, req)
	return rec
}

// Concurrent live server adds must not lose each other's file edits:
// the handler holds the reload lock over write and reload (#2674),
// mirroring TestConcurrentAdapterAddsAllLand.
func TestConcurrentServerAddsAllLand(t *testing.T) {
	s, path := setupPersist(t, []byte(`{"mcpServers":{}}`+"\n"), false)
	const n = 8
	var wg sync.WaitGroup
	codes := make([]int, n)
	for i := range n {
		wg.Add(1)
		go func() {
			defer wg.Done()
			codes[i] = postServerAdd(s, fmt.Sprintf("s%d", i)).Code
		}()
	}
	wg.Wait()
	for i, code := range codes {
		if code != http.StatusOK {
			t.Fatalf("add s%d: status %d, want 200", i, code)
		}
	}
	got := readFile(t, path)
	for i := range n {
		if !strings.Contains(got, fmt.Sprintf(`"s%d"`, i)) {
			t.Fatalf("add s%d is missing from the file: %s", i, got)
		}
	}
}

// A concurrent add/remove mix must not resurrect a removed server or
// drop an unrelated one (#2674): the remove of "victim" races adds of
// fresh names; afterwards victim stays gone and every add landed.
func TestConcurrentServerAddRemoveMix(t *testing.T) {
	s, path := setupPersist(t,
		[]byte(`{"mcpServers":{"victim":{"command":"/bin/true"},"keeper":{"command":"/bin/true"}}}`+"\n"),
		false)
	const n = 6
	var wg sync.WaitGroup
	addCodes := make([]int, n)
	removeCode := make([]int, 1)
	wg.Add(1)
	go func() {
		defer wg.Done()
		req := httptest.NewRequest(http.MethodDelete, "/v1/servers/victim", nil)
		req.SetPathValue("name", "victim")
		rec := httptest.NewRecorder()
		s.handleServersRemove(rec, req)
		removeCode[0] = rec.Code
	}()
	for i := range n {
		wg.Add(1)
		go func() {
			defer wg.Done()
			addCodes[i] = postServerAdd(s, fmt.Sprintf("new%d", i)).Code
		}()
	}
	wg.Wait()
	if removeCode[0] != http.StatusOK {
		t.Fatalf("remove victim: status %d, want 200", removeCode[0])
	}
	for i, code := range addCodes {
		if code != http.StatusOK {
			t.Fatalf("add new%d: status %d, want 200", i, code)
		}
	}
	got := readFile(t, path)
	if strings.Contains(got, `"victim"`) {
		t.Fatalf("removed server was resurrected: %s", got)
	}
	if !strings.Contains(got, `"keeper"`) {
		t.Fatalf("unrelated server was dropped: %s", got)
	}
	for i := range n {
		if !strings.Contains(got, fmt.Sprintf(`"new%d"`, i)) {
			t.Fatalf("add new%d is missing from the file: %s", i, got)
		}
	}
}
