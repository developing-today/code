package elicit_test

import (
	"encoding/json"
	"os"
	"path/filepath"
	"sync"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/elicit"
)

func perm(t *testing.T, path string) os.FileMode {
	t.Helper()
	info, err := os.Stat(path)
	if err != nil {
		t.Fatal(err)
	}
	return info.Mode().Perm()
}

func TestOpenAtCreatesOwnerOnlyStore(t *testing.T) {
	state := t.TempDir()
	b, err := elicit.OpenAt(state, filepath.Join(state, "logs", "elicit.db"))
	if err != nil {
		t.Fatal(err)
	}
	defer b.Close()

	db := elicit.StorePath(state)
	if got := perm(t, filepath.Dir(db)); got != 0o700 {
		t.Errorf("store dir mode = %04o, want 0700", got)
	}
	if got := perm(t, db); got != 0o600 {
		t.Errorf("store file mode = %04o, want 0600", got)
	}
	// The shared logs directory is not created, let alone tightened:
	// it keeps its public mode and other tenants.
	if _, err := os.Stat(filepath.Join(state, "logs")); !os.IsNotExist(err) {
		t.Errorf("OpenAt created the logs dir; err = %v", err)
	}
}

func TestOpenAtMigratesLegacyStore(t *testing.T) {
	state := t.TempDir()
	legacy := filepath.Join(state, "logs", "elicit.db")
	if err := os.MkdirAll(filepath.Dir(legacy), 0o755); err != nil {
		t.Fatal(err)
	}
	lb, err := elicit.Open(legacy)
	if err != nil {
		t.Fatal(err)
	}
	r := ask(t, lb, elicit.Request{Session: "s1"})
	if err := lb.Close(); err != nil {
		t.Fatal(err)
	}

	b, err := elicit.OpenAt(state, legacy)
	if err != nil {
		t.Fatal(err)
	}
	defer b.Close()

	if _, err := os.Stat(legacy); !os.IsNotExist(err) {
		t.Errorf("legacy store still in place after migration; err = %v", err)
	}
	if got := perm(t, elicit.StorePath(state)); got != 0o600 {
		t.Errorf("migrated store mode = %04o, want 0600", got)
	}
	pending, err := b.Pending(elicit.Filter{})
	if err != nil {
		t.Fatal(err)
	}
	found := false
	for _, p := range pending {
		if p.ID == r.ID {
			found = true
		}
	}
	if !found {
		t.Fatalf("pending question %s did not survive migration: %+v", r.ID, pending)
	}
	// And it is still answerable from the migrated store.
	if err := b.Respond(elicit.Answer{
		ID: r.ID, Action: elicit.Accept,
		Content: json.RawMessage(`{"repo":"me/thing"}`), By: "test",
	}); err != nil {
		t.Fatalf("answering the migrated question: %v", err)
	}
}

func TestOpenAtTightensLooseModes(t *testing.T) {
	state := t.TempDir()
	b, err := elicit.OpenAt(state, "")
	if err != nil {
		t.Fatal(err)
	}
	if err := b.Close(); err != nil {
		t.Fatal(err)
	}
	db := elicit.StorePath(state)
	if err := os.Chmod(filepath.Dir(db), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.Chmod(db, 0o644); err != nil {
		t.Fatal(err)
	}

	b2, err := elicit.OpenAt(state, "")
	if err != nil {
		t.Fatal(err)
	}
	defer b2.Close()
	if got := perm(t, filepath.Dir(db)); got != 0o700 {
		t.Errorf("store dir mode after re-open = %04o, want 0700", got)
	}
	if got := perm(t, db); got != 0o600 {
		t.Errorf("store file mode after re-open = %04o, want 0600", got)
	}
}

func TestOpenAtConcurrentOpens(t *testing.T) {
	state := t.TempDir()
	legacy := filepath.Join(state, "logs", "elicit.db")
	const n = 8
	var wg sync.WaitGroup
	errs := make([]error, n)
	brokers := make([]*elicit.Broker, n)
	for i := range n {
		wg.Add(1)
		go func() {
			defer wg.Done()
			b, err := elicit.OpenAt(state, legacy)
			brokers[i] = b
			errs[i] = err
		}()
	}
	wg.Wait()
	for i := range n {
		if errs[i] != nil {
			t.Errorf("concurrent open %d: %v", i, errs[i])
		}
		if brokers[i] != nil {
			brokers[i].Close()
		}
	}
}

func TestOpenAtConcurrentMigrationPreservesUncheckpointedWAL(t *testing.T) {
	// A daemon that crashes leaves its newest questions only in the
	// legacy store's WAL: the main database file does not have them
	// until a checkpoint, which a crash never runs. Build that state
	// by snapshotting a live legacy store's files before Close can
	// checkpoint, then race first opens of the new location against
	// the snapshot: no opener may see the destination before its WAL
	// arrives, or the question is lost for good.
	src := t.TempDir()
	srcLegacy := filepath.Join(src, "logs", "elicit.db")
	if err := os.MkdirAll(filepath.Dir(srcLegacy), 0o755); err != nil {
		t.Fatal(err)
	}
	lb, err := elicit.Open(srcLegacy)
	if err != nil {
		t.Fatal(err)
	}
	r := ask(t, lb, elicit.Request{Session: "s1"})
	wal, err := os.ReadFile(srcLegacy + "-wal")
	if err != nil || len(wal) == 0 {
		t.Fatalf("live legacy store has no WAL to snapshot: err=%v len=%d", err, len(wal))
	}
	main, err := os.ReadFile(srcLegacy)
	if err != nil {
		t.Fatal(err)
	}

	state := t.TempDir()
	legacy := filepath.Join(state, "logs", "elicit.db")
	if err := os.MkdirAll(filepath.Dir(legacy), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(legacy, main, 0o644); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(legacy+"-wal", wal, 0o644); err != nil {
		t.Fatal(err)
	}
	if err := lb.Close(); err != nil {
		t.Fatal(err)
	}

	const n = 8
	var wg sync.WaitGroup
	errs := make([]error, n)
	brokers := make([]*elicit.Broker, n)
	for i := range n {
		wg.Add(1)
		go func() {
			defer wg.Done()
			b, err := elicit.OpenAt(state, legacy)
			brokers[i] = b
			errs[i] = err
		}()
	}
	wg.Wait()
	for i := range n {
		if errs[i] != nil {
			t.Errorf("concurrent migrating open %d: %v", i, errs[i])
		}
		if brokers[i] != nil {
			brokers[i].Close()
		}
	}

	b, err := elicit.OpenAt(state, legacy)
	if err != nil {
		t.Fatal(err)
	}
	defer b.Close()
	pending, err := b.Pending(elicit.Filter{})
	if err != nil {
		t.Fatal(err)
	}
	found := false
	for _, q := range pending {
		if q.ID == r.ID {
			found = true
		}
	}
	if !found {
		t.Fatalf("question %s that lived only in the legacy WAL did not survive concurrent migration: %+v", r.ID, pending)
	}
}
