package artifacts_test

import (
	"database/sql"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/dezren39/mcpx/internal/artifacts"
	_ "modernc.org/sqlite"
)

func open(t *testing.T, opts artifacts.Options) *artifacts.Store {
	t.Helper()
	if opts.Dir == "" {
		opts.Dir = t.TempDir()
	}
	if opts.TTL == 0 {
		opts.TTL = time.Hour
	}
	s, err := artifacts.Open(opts)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { s.Close() })
	return s
}

func TestPutStoresAndReadsBack(t *testing.T) {
	s := open(t, artifacts.Options{})
	m, err := s.Put(strings.NewReader("hello"), artifacts.PutOptions{Name: "greeting.txt", Run: "r1"})
	if err != nil {
		t.Fatal(err)
	}
	if m.Size != 5 {
		t.Errorf("size = %d", m.Size)
	}
	if !strings.HasPrefix(m.ID, "art-") {
		t.Errorf("id should be a handle, got %q", m.ID)
	}
	if m.URI != "mcpx://artifacts/"+m.ID {
		t.Errorf("uri = %q", m.URI)
	}
	if m.Mime != "text/plain; charset=utf-8" {
		t.Errorf("the extension should decide the type, got %q", m.Mime)
	}
	body, _, err := s.Bytes(m.ID)
	if err != nil {
		t.Fatal(err)
	}
	if string(body) != "hello" {
		t.Errorf("body = %q", body)
	}
}

// The handle must not be the hash. A content hash is guessable by anyone who
// can guess the content, and the handle is the whole of the access control
// until OAuth scopes land.
func TestTheHandleIsNotTheContentHash(t *testing.T) {
	s := open(t, artifacts.Options{})
	m, err := s.Put(strings.NewReader("hello"), artifacts.PutOptions{Name: "a.txt"})
	if err != nil {
		t.Fatal(err)
	}
	if strings.Contains(m.ID, m.SHA256) || strings.Contains(m.SHA256, strings.TrimPrefix(m.ID, "art-")) {
		t.Errorf("the id %q leaks the hash %q", m.ID, m.SHA256)
	}
	second, err := s.Put(strings.NewReader("hello"), artifacts.PutOptions{Name: "a.txt"})
	if err != nil {
		t.Fatal(err)
	}
	if second.ID == m.ID {
		t.Error("two registrations of the same bytes should get different handles")
	}
	if second.SHA256 != m.SHA256 {
		t.Error("the same bytes should share one content address")
	}
}

func TestIdenticalContentIsStoredOnce(t *testing.T) {
	dir := t.TempDir()
	s := open(t, artifacts.Options{Dir: dir})
	for i := 0; i < 3; i++ {
		if _, err := s.Put(strings.NewReader("same"), artifacts.PutOptions{Name: "x.bin"}); err != nil {
			t.Fatal(err)
		}
	}
	var blobs int
	_ = filepath.Walk(filepath.Join(dir, "blobs"), func(_ string, info os.FileInfo, err error) error {
		if err == nil && !info.IsDir() {
			blobs++
		}
		return nil
	})
	if blobs != 1 {
		t.Errorf("three registrations of one body should keep one blob, found %d", blobs)
	}
	used, err := s.Used()
	if err != nil {
		t.Fatal(err)
	}
	if used != 4 {
		t.Errorf("usage should count distinct content, got %d", used)
	}
}

func TestPerArtifactLimitIsAnErrorNotATruncation(t *testing.T) {
	s := open(t, artifacts.Options{MaxBytes: 4})
	_, err := s.Put(strings.NewReader("much too long"), artifacts.PutOptions{Name: "big"})
	var tooLarge artifacts.ErrTooLarge
	if err == nil {
		t.Fatal("over the limit should fail")
	}
	if !asErr(err, &tooLarge) {
		t.Fatalf("want ErrTooLarge, got %T: %v", err, err)
	}
	list, _ := s.List(artifacts.Filter{})
	if len(list) != 0 {
		t.Errorf("a refused artifact should not be registered: %+v", list)
	}
}

func TestQuotaRefusesNewContentButNotDuplicates(t *testing.T) {
	s := open(t, artifacts.Options{Quota: 10})
	if _, err := s.Put(strings.NewReader("0123456789"), artifacts.PutOptions{Name: "a"}); err != nil {
		t.Fatal(err)
	}
	// The same bytes again free no space by being refused, so they are not.
	if _, err := s.Put(strings.NewReader("0123456789"), artifacts.PutOptions{Name: "b"}); err != nil {
		t.Errorf("a duplicate should not hit the quota: %v", err)
	}
	_, err := s.Put(strings.NewReader("x"), artifacts.PutOptions{Name: "c"})
	var quota artifacts.ErrQuota
	if !asErr(err, &quota) {
		t.Fatalf("want ErrQuota, got %v", err)
	}
}

func TestPutFileHardlinksRatherThanCopies(t *testing.T) {
	dir := t.TempDir()
	src := filepath.Join(dir, "shot.png")
	if err := os.WriteFile(src, []byte("PNGDATA"), 0o600); err != nil {
		t.Fatal(err)
	}
	s := open(t, artifacts.Options{})
	m, err := s.PutFile(src, artifacts.PutOptions{})
	if err != nil {
		t.Fatal(err)
	}
	if m.Name != "shot.png" {
		t.Errorf("the name should default to the file's, got %q", m.Name)
	}
	if !sameInode(t, src, s.BodyPath(m)) {
		t.Error("a file on the same filesystem should be linked, not copied")
	}
}

func TestExportNeverOverwrites(t *testing.T) {
	s := open(t, artifacts.Options{})
	out := t.TempDir()
	var paths []string
	for _, body := range []string{"one", "two", "three"} {
		m, err := s.Put(strings.NewReader(body), artifacts.PutOptions{Name: "shot.png"})
		if err != nil {
			t.Fatal(err)
		}
		p, err := s.Export(m, out)
		if err != nil {
			t.Fatal(err)
		}
		paths = append(paths, p)
	}
	seen := map[string]bool{}
	for i, p := range paths {
		if seen[p] {
			t.Fatalf("export %d reused %s; a second file called shot.png is a second file", i, p)
		}
		seen[p] = true
		if filepath.Ext(p) != ".png" {
			t.Errorf("the suffix should go before the extension, got %s", p)
		}
	}
	if _, err := os.Stat(filepath.Join(out, "shot-1.png")); err != nil {
		t.Errorf("expected shot-1.png: %v", err)
	}
}

func TestExpiredArtifactsAreCollected(t *testing.T) {
	s := open(t, artifacts.Options{TTL: -time.Second})
	if _, err := s.Put(strings.NewReader("gone soon"), artifacts.PutOptions{Name: "x"}); err != nil {
		t.Fatal(err)
	}
	n, err := s.GC(time.Now())
	if err != nil {
		t.Fatal(err)
	}
	if n != 1 {
		t.Errorf("collected %d", n)
	}
	list, _ := s.List(artifacts.Filter{})
	if len(list) != 0 {
		t.Errorf("still listed: %+v", list)
	}
}

func TestDeleteKeepsABodyAnotherRegistrationShares(t *testing.T) {
	s := open(t, artifacts.Options{})
	a, _ := s.Put(strings.NewReader("shared"), artifacts.PutOptions{Name: "a"})
	b, _ := s.Put(strings.NewReader("shared"), artifacts.PutOptions{Name: "b"})
	if err := s.Delete(a.ID); err != nil {
		t.Fatal(err)
	}
	if _, _, err := s.Bytes(b.ID); err != nil {
		t.Fatalf("the surviving registration lost its body: %v", err)
	}
	if err := s.Delete(b.ID); err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(s.BodyPath(b)); !os.IsNotExist(err) {
		t.Error("the last registration should take the body with it")
	}
}

func TestListFiltersByRunOldestLast(t *testing.T) {
	s := open(t, artifacts.Options{})
	for _, r := range []string{"r1", "r2", "r1"} {
		if _, err := s.Put(strings.NewReader(r+"x"), artifacts.PutOptions{Name: "n", Run: r}); err != nil {
			t.Fatal(err)
		}
	}
	got, err := s.List(artifacts.Filter{Run: "r1"})
	if err != nil {
		t.Fatal(err)
	}
	if len(got) != 2 {
		t.Fatalf("want 2 for r1, got %d", len(got))
	}
	for _, m := range got {
		if m.Run != "r1" {
			t.Errorf("leaked %s", m.Run)
		}
	}
}

func TestMissingArtifactSaysItMayHaveExpired(t *testing.T) {
	s := open(t, artifacts.Options{})
	_, err := s.Get("art-nope")
	if err == nil || !strings.Contains(err.Error(), "expired") {
		t.Errorf("want an expiry hint, got %v", err)
	}
}

// --- Touch-on-read opt-in (#2548, decision 0006) ---

// With idleTtl unset, an item read changes nothing: expiry stays at
// put + ttl exactly, and lastTouched stays at creation.
func TestItemReadDoesNotRenewByDefault(t *testing.T) {
	s := open(t, artifacts.Options{TTL: time.Hour})
	m, err := s.Put(strings.NewReader("hello"), artifacts.PutOptions{Name: "a.txt"})
	if err != nil {
		t.Fatal(err)
	}
	got, err := s.Get(m.ID)
	if err != nil {
		t.Fatal(err)
	}
	if got.Expires.UnixMilli() != m.Expires.UnixMilli() {
		t.Errorf("default expiry must not move on read: put %v, read %v", m.Expires, got.Expires)
	}
	if got.LastTouched.UnixMilli() != m.Created.UnixMilli() {
		t.Errorf("lastTouched should stay at creation when the opt-in is off, got %v", got.LastTouched)
	}
	if got.ExpiresAt != got.Expires {
		t.Errorf("expiresAt should mirror expires, got %v vs %v", got.ExpiresAt, got.Expires)
	}
	if got.TTLRemainingMs <= 0 || got.TTLRemainingMs > int64(time.Hour/time.Millisecond) {
		t.Errorf("ttlRemainingMs should be within the TTL, got %d", got.TTLRemainingMs)
	}
}

// With idleTtl set, an item read renews: expires becomes
// lastTouched + idleTtl (past the absolute put + ttl), and the
// renewal is persisted, not just rendered.
func TestItemReadRenewsWhenIdleTtlSet(t *testing.T) {
	s := open(t, artifacts.Options{TTL: time.Hour, IdleTTL: 2 * time.Hour, MaxAge: 24 * time.Hour})
	m, err := s.Put(strings.NewReader("hello"), artifacts.PutOptions{Name: "a.txt"})
	if err != nil {
		t.Fatal(err)
	}
	before := time.Now()
	got, err := s.Get(m.ID)
	if err != nil {
		t.Fatal(err)
	}
	if !got.Expires.After(m.Expires) {
		t.Fatalf("read should renew past the absolute expiry %v, got %v", m.Expires, got.Expires)
	}
	want := got.LastTouched.Add(2 * time.Hour)
	if got.Expires.Sub(want) > time.Second || want.Sub(got.Expires) > time.Second {
		t.Errorf("expires should be lastTouched + idleTtl (%v), got %v", want, got.Expires)
	}
	if got.LastTouched.Before(before.Add(-time.Second)) || got.LastTouched.After(time.Now().Add(time.Second)) {
		t.Errorf("lastTouched should be the read time, got %v", got.LastTouched)
	}
	// A collection read afterwards must show the renewed deadline
	// without renewing it further.
	list, err := s.List(artifacts.Filter{})
	if err != nil {
		t.Fatal(err)
	}
	if len(list) != 1 || list[0].Expires.UnixMilli() != got.Expires.UnixMilli() {
		t.Errorf("renewal should persist: list shows %+v, get showed %v", list, got.Expires)
	}
	if list[0].LastTouched.UnixMilli() != got.LastTouched.UnixMilli() {
		t.Errorf("list must not touch: lastTouched moved from %v to %v", got.LastTouched, list[0].LastTouched)
	}
}

// Renewal is bounded: however often an artifact is read, its deadline
// never passes created + maxAge.
func TestRenewalIsBoundedByMaxAge(t *testing.T) {
	s := open(t, artifacts.Options{TTL: time.Hour, IdleTTL: 2 * time.Hour, MaxAge: 90 * time.Minute})
	m, err := s.Put(strings.NewReader("hello"), artifacts.PutOptions{Name: "a.txt"})
	if err != nil {
		t.Fatal(err)
	}
	got, err := s.Get(m.ID)
	if err != nil {
		t.Fatal(err)
	}
	ceiling := m.Created.Add(90 * time.Minute)
	if got.Expires.UnixMilli() != ceiling.UnixMilli() {
		t.Errorf("expires should be clamped to created + maxAge (%v), got %v", ceiling, got.Expires)
	}
}

// Touch never shortens an explicit per-artifact lifetime, and never
// revives an expired artifact.
func TestTouchNeverShortensOrRevives(t *testing.T) {
	s := open(t, artifacts.Options{TTL: time.Hour, IdleTTL: time.Minute, MaxAge: 24 * time.Hour})
	m, err := s.Put(strings.NewReader("hello"), artifacts.PutOptions{Name: "a.txt", TTL: 3 * time.Hour})
	if err != nil {
		t.Fatal(err)
	}
	got, err := s.Get(m.ID)
	if err != nil {
		t.Fatal(err)
	}
	if got.Expires.UnixMilli() != m.Expires.UnixMilli() {
		t.Errorf("explicit lifetime should stand: put %v, read %v", m.Expires, got.Expires)
	}

	short := open(t, artifacts.Options{TTL: time.Millisecond, IdleTTL: time.Hour, MaxAge: 24 * time.Hour})
	dead, err := short.Put(strings.NewReader("bye"), artifacts.PutOptions{Name: "b.txt"})
	if err != nil {
		t.Fatal(err)
	}
	time.Sleep(5 * time.Millisecond)
	got, err = short.Get(dead.ID)
	if err != nil {
		t.Fatal(err)
	}
	if got.Expires.UnixMilli() != dead.Expires.UnixMilli() {
		t.Errorf("an expired artifact must not be revived: was %v, read shows %v", dead.Expires, got.Expires)
	}
	n, err := short.GC(time.Now())
	if err != nil {
		t.Fatal(err)
	}
	if n != 1 {
		t.Errorf("GC should still collect the expired artifact, collected %d", n)
	}
}

// A store written before the touch-on-read opt-in has last_touched = 0
// on its rows. That sentinel must render as the creation time, not as
// 1970: time.UnixMilli(0) is not the zero Time.
func TestMigratedRowsReportCreationAsLastTouched(t *testing.T) {
	dir := t.TempDir()
	s := open(t, artifacts.Options{Dir: dir})
	m, err := s.Put(strings.NewReader("hello"), artifacts.PutOptions{Name: "a.txt"})
	if err != nil {
		t.Fatal(err)
	}
	if err := s.Close(); err != nil {
		t.Fatal(err)
	}
	db, err := sql.Open("sqlite", filepath.Join(dir, "index.db"))
	if err != nil {
		t.Fatal(err)
	}
	if _, err := db.Exec(`UPDATE artifacts SET last_touched = 0 WHERE id = ?`, m.ID); err != nil {
		t.Fatal(err)
	}
	if err := db.Close(); err != nil {
		t.Fatal(err)
	}
	s2 := open(t, artifacts.Options{Dir: dir})
	list, err := s2.List(artifacts.Filter{})
	if err != nil {
		t.Fatal(err)
	}
	if len(list) != 1 {
		t.Fatalf("want 1 artifact, got %d", len(list))
	}
	if list[0].LastTouched.UnixMilli() != m.Created.UnixMilli() {
		t.Errorf("migrated row lastTouched = %v, want creation %v", list[0].LastTouched, m.Created)
	}
}
