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

// Two item reads can interleave read-compute-write: both readers
// snapshot the same metadata, the later reader commits first, and the
// earlier reader's touch lands second. The SQL guard in touch makes
// the update monotonic, so the stale touch must neither persist nor
// return the older deadline -- reverting the guard to an unconditional
// update makes this test fail on both counts.
func TestTouchStaleReaderDoesNotRegressRenewal(t *testing.T) {
	s, err := Open(Options{Dir: t.TempDir(), TTL: time.Hour, IdleTTL: 2 * time.Hour, MaxAge: 24 * time.Hour})
	if err != nil {
		t.Fatal(err)
	}
	defer s.Close()
	m, err := s.Put(strings.NewReader("hello"), PutOptions{Name: "a.txt"})
	if err != nil {
		t.Fatal(err)
	}
	// Two stale snapshots of the same pre-renewal state.
	earlierSnap, err := s.get(m.ID)
	if err != nil {
		t.Fatal(err)
	}
	laterSnap, err := s.get(m.ID)
	if err != nil {
		t.Fatal(err)
	}
	later := m.Created.Add(30 * time.Minute).Truncate(time.Millisecond)
	earlier := m.Created.Add(10 * time.Minute).Truncate(time.Millisecond)
	// The later touch commits first.
	if err := s.touch(&laterSnap, later); err != nil {
		t.Fatal(err)
	}
	wantExpires := later.Add(2 * time.Hour)
	if laterSnap.LastTouched.UnixMilli() != later.UnixMilli() || laterSnap.Expires.UnixMilli() != wantExpires.UnixMilli() {
		t.Fatalf("later touch: lastTouched=%v expires=%v, want %v / %v",
			laterSnap.LastTouched, laterSnap.Expires, later, wantExpires)
	}
	// The earlier (stale) touch lands second: it must keep the newer
	// deadline and last-touch, in what it returns...
	if err := s.touch(&earlierSnap, earlier); err != nil {
		t.Fatal(err)
	}
	if earlierSnap.LastTouched.UnixMilli() != later.UnixMilli() {
		t.Errorf("stale touch returned lastTouched %v, want the newer %v", earlierSnap.LastTouched, later)
	}
	if earlierSnap.Expires.UnixMilli() != wantExpires.UnixMilli() {
		t.Errorf("stale touch returned expires %v, want the newer %v", earlierSnap.Expires, wantExpires)
	}
	if earlierSnap.ExpiresAt.UnixMilli() != wantExpires.UnixMilli() {
		t.Errorf("stale touch returned expiresAt %v, want %v", earlierSnap.ExpiresAt, wantExpires)
	}
	// ...and in what is persisted.
	persisted, err := s.get(m.ID)
	if err != nil {
		t.Fatal(err)
	}
	if persisted.LastTouched.UnixMilli() != later.UnixMilli() || persisted.Expires.UnixMilli() != wantExpires.UnixMilli() {
		t.Errorf("persisted lastTouched=%v expires=%v, want %v / %v",
			persisted.LastTouched, persisted.Expires, later, wantExpires)
	}
}

// Two processes can hold the same store open with different
// artifacts.idleTtl values (the daemon and a CLI, say). Both snapshot
// the artifact; the longer-window touch commits first; the
// shorter-window touch lands second, from its stale snapshot, with a
// LATER last_touched (so its WHERE guard passes) but a SMALLER
// computed deadline. The update must never move expires backwards:
// MAX in the UPDATE keeps the longer renewal, and the touch returns
// the persisted state. Reverting to a plain SET makes this fail.
func TestTouchShorterWindowDoesNotRegressLongerRenewal(t *testing.T) {
	s, err := Open(Options{Dir: t.TempDir(), TTL: time.Hour, IdleTTL: 2 * time.Hour, MaxAge: 24 * time.Hour})
	if err != nil {
		t.Fatal(err)
	}
	defer s.Close()
	m, err := s.Put(strings.NewReader("hello"), PutOptions{Name: "a.txt"})
	if err != nil {
		t.Fatal(err)
	}
	// Two snapshots of the same pre-renewal state.
	longSnap, err := s.get(m.ID)
	if err != nil {
		t.Fatal(err)
	}
	shortSnap, err := s.get(m.ID)
	if err != nil {
		t.Fatal(err)
	}
	later := m.Created.Add(30 * time.Minute).Truncate(time.Millisecond)
	if err := s.touch(&longSnap, later); err != nil {
		t.Fatal(err)
	}
	wantExpires := later.Add(2 * time.Hour)
	// The shorter-window peer touches from its stale snapshot: later
	// in wall-clock terms, but renewing by less.
	s.opts.IdleTTL = 30 * time.Minute
	shortNow := later.Add(10 * time.Minute)
	if err := s.touch(&shortSnap, shortNow); err != nil {
		t.Fatal(err)
	}
	if shortSnap.Expires.UnixMilli() != wantExpires.UnixMilli() {
		t.Errorf("shorter-window touch returned expires %v, want the longer renewal %v", shortSnap.Expires, wantExpires)
	}
	persisted, err := s.get(m.ID)
	if err != nil {
		t.Fatal(err)
	}
	if persisted.Expires.UnixMilli() != wantExpires.UnixMilli() {
		t.Errorf("persisted expires=%v, want %v (never regress)", persisted.Expires, wantExpires)
	}
	if persisted.LastTouched.UnixMilli() != shortNow.UnixMilli() {
		t.Errorf("persisted lastTouched=%v, want %v (the later touch still counts)", persisted.LastTouched, shortNow)
	}
}
