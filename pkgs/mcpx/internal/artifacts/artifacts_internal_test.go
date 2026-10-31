package artifacts

import (
	"database/sql"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"

	_ "modernc.org/sqlite"
)

// GC selects expired ids first and deletes them afterwards. With
// touch-on-read enabled, a Get landing between the selection and the
// delete renews the artifact; the delete must then NOT remove it.
// deleteExpired rechecks expiry atomically in the DELETE's WHERE.
func TestDeleteExpiredRechecksExpiryAtomically(t *testing.T) {
	s, err := Open(Options{Dir: t.TempDir(), TTL: time.Hour, IdleTTL: time.Hour, MaxAge: 24 * time.Hour})
	if err != nil {
		t.Fatal(err)
	}
	defer s.Close()
	m, err := s.Put(strings.NewReader("renewed"), PutOptions{Name: "a.txt"})
	if err != nil {
		t.Fatal(err)
	}
	now := time.Now()
	// The row expires -- this is the state GC's selection sees.
	if _, err := s.db.Exec(`UPDATE artifacts SET expires = ? WHERE id = ?`,
		now.Add(-time.Minute).UnixMilli(), m.ID); err != nil {
		t.Fatal(err)
	}
	// A reader renews it before GC's delete lands.
	if _, err := s.db.Exec(`UPDATE artifacts SET expires = ? WHERE id = ?`,
		now.Add(time.Hour).UnixMilli(), m.ID); err != nil {
		t.Fatal(err)
	}
	gone, err := s.deleteExpired(m.ID, now)
	if err != nil {
		t.Fatal(err)
	}
	if gone {
		t.Error("deleteExpired removed a renewed artifact")
	}
	if _, err := s.get(m.ID); err != nil {
		t.Errorf("renewed artifact should survive: %v", err)
	}
	body, _, err := s.Bytes(m.ID)
	if err != nil || string(body) != "renewed" {
		t.Errorf("renewed artifact lost its blob: body=%q err=%v", body, err)
	}
	// Expire it again with no renewal: now the delete must land.
	if _, err := s.db.Exec(`UPDATE artifacts SET expires = ? WHERE id = ?`,
		now.Add(-time.Minute).UnixMilli(), m.ID); err != nil {
		t.Fatal(err)
	}
	gone, err = s.deleteExpired(m.ID, now)
	if err != nil {
		t.Fatal(err)
	}
	if !gone {
		t.Error("deleteExpired should remove a still-expired artifact")
	}
	if _, err := s.get(m.ID); err == nil {
		t.Error("still-expired artifact should be gone")
	}
}

// Two openers racing to migrate a legacy store must both succeed:
// the loser of the ALTER race rechecks and finds the column present
// instead of failing Open with "duplicate column name".
func TestEnsureLastTouchedConcurrentOpeners(t *testing.T) {
	db, err := sql.Open("sqlite", filepath.Join(t.TempDir(), "legacy.db")+
		"?_pragma=journal_mode(WAL)&_pragma=busy_timeout(5000)")
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()
	if _, err := db.Exec(`CREATE TABLE artifacts (id TEXT PRIMARY KEY)`); err != nil {
		t.Fatal(err)
	}
	const openers = 16
	errs := make([]error, openers)
	var wg sync.WaitGroup
	for i := range openers {
		wg.Add(1)
		go func() {
			defer wg.Done()
			errs[i] = ensureLastTouched(db)
		}()
	}
	wg.Wait()
	for i, err := range errs {
		if err != nil {
			t.Errorf("opener %d: %v", i, err)
		}
	}
	has, err := hasLastTouchedColumn(db)
	if err != nil {
		t.Fatal(err)
	}
	if !has {
		t.Error("last_touched column missing after concurrent migration")
	}
}
