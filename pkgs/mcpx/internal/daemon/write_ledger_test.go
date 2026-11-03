package daemon

import (
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

func TestWriteLedgerPersistsRefusalAndClear(t *testing.T) {
	dir := t.TempDir()
	ledger, err := OpenWriteLedger(dir)
	if err != nil {
		t.Fatal(err)
	}
	want := unknownWriteRecord{
		Server:     "github",
		Tool:       "create_issue",
		ArgsSHA256: strings.Repeat("a", 64),
		At:         time.Date(2026, time.November, 2, 12, 30, 0, 0, time.UTC),
		Surface:    "registry",
	}
	if err := ledger.Record(want); err != nil {
		t.Fatal(err)
	}

	// The block must survive a daemon restart, not just live in Registry memory.
	restarted, err := OpenWriteLedger(dir)
	if err != nil {
		t.Fatal(err)
	}
	got, blocked := restarted.Lookup("github")
	if !blocked {
		t.Fatal("recorded unknown outcome did not survive reopening the ledger")
	}
	if got != want {
		t.Fatalf("Lookup() = %#v, want %#v", got, want)
	}
	// The file contains only the digest, never the original arguments.
	data, err := os.ReadFile(filepath.Join(dir, writeLedgerFilename))
	if err != nil {
		t.Fatal(err)
	}
	if strings.Contains(string(data), "private-argument") {
		t.Fatalf("ledger persisted original arguments: %s", data)
	}

	cleared, err := restarted.Clear("github")
	if err != nil {
		t.Fatal(err)
	}
	if !cleared {
		t.Fatal("Clear() = false, want true for an existing record")
	}
	if _, blocked := restarted.Lookup("github"); blocked {
		t.Fatal("server remained blocked after Clear")
	}

	clearedAgain, err := OpenWriteLedger(dir)
	if err != nil {
		t.Fatal(err)
	}
	if _, blocked := clearedAgain.Lookup("github"); blocked {
		t.Fatal("cleared record returned after reopening the ledger")
	}
}

func TestWriteClearRouteRemovesLedgerBlock(t *testing.T) {
	dir := t.TempDir()
	ledger, err := OpenWriteLedger(dir)
	if err != nil {
		t.Fatal(err)
	}
	if err := ledger.Record(unknownWriteRecord{
		Server: "github", Tool: "create_issue", ArgsSHA256: strings.Repeat("c", 64),
		At: time.Now().UTC(), Surface: "registry",
	}); err != nil {
		t.Fatal(err)
	}
	s := &Server{reg: &Registry{writeLedger: ledger}}
	mux := http.NewServeMux()
	s.routesV1Ops(mux)
	rec := httptest.NewRecorder()
	mux.ServeHTTP(rec, httptest.NewRequest(http.MethodPost, "/v1/writes/github/clear", nil))
	if rec.Code != http.StatusOK {
		t.Fatalf("POST clear status = %d, body %s; want 200", rec.Code, rec.Body)
	}
	if _, blocked := ledger.Lookup("github"); blocked {
		t.Fatal("clear route returned success but the server remained blocked")
	}
}

// Two configuration-specific daemons share one state directory (Paths.State
// is not keyed by ForConfig), so an unkeyed ledger filename let the second
// daemon's save replace the first daemon's block. Each configuration gets
// its own file.
func TestWriteLedgerIsolatedPerConfig(t *testing.T) {
	dir := t.TempDir()
	first, err := OpenWriteLedger(dir, "/projects/one/.mcpx.json")
	if err != nil {
		t.Fatal(err)
	}
	second, err := OpenWriteLedger(dir, "/projects/two/.mcpx.json")
	if err != nil {
		t.Fatal(err)
	}
	if err := first.Record(unknownWriteRecord{
		Server: "github", Tool: "create_issue", ArgsSHA256: strings.Repeat("a", 64),
		At: time.Now().UTC(), Surface: "registry",
	}); err != nil {
		t.Fatal(err)
	}
	if err := second.Record(unknownWriteRecord{
		Server: "linear", Tool: "create_issue", ArgsSHA256: strings.Repeat("b", 64),
		At: time.Now().UTC(), Surface: "registry",
	}); err != nil {
		t.Fatal(err)
	}

	reopenedFirst, err := OpenWriteLedger(dir, "/projects/one/.mcpx.json")
	if err != nil {
		t.Fatal(err)
	}
	reopenedSecond, err := OpenWriteLedger(dir, "/projects/two/.mcpx.json")
	if err != nil {
		t.Fatal(err)
	}
	if _, blocked := reopenedFirst.Lookup("github"); !blocked {
		t.Fatal("second configuration's ledger overwrote the first configuration's block")
	}
	if _, blocked := reopenedFirst.Lookup("linear"); blocked {
		t.Fatal("first configuration's ledger holds the second configuration's block")
	}
	if _, blocked := reopenedSecond.Lookup("linear"); !blocked {
		t.Fatal("second configuration's block did not survive reopening")
	}
	if _, blocked := reopenedSecond.Lookup("github"); blocked {
		t.Fatal("second configuration's ledger holds the first configuration's block")
	}
}
