package seed

import (
	"crypto/sha256"
	"encoding/binary"
	"strings"
	"testing"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/seedstore"
	"github.com/developing-today/code/pkgs/mcpx/internal/testsupport/gen"
)

// Oracle: seedstore.Timestamp is the store's independent UUIDv7
// parser (#1953) — a Mint output it cannot parse or that does not
// embed ~now is a format bug by the store's own rules.
func TestMintProducesCanonicalUUIDv7(t *testing.T) {
	before := time.Now().UnixMilli()
	id := Mint()
	after := time.Now().UnixMilli()
	ts, err := seedstore.Timestamp(id)
	if err != nil {
		t.Fatalf("seedstore.Timestamp(%q): %v", id, err)
	}
	if ms := ts.UnixMilli(); ms < before || ms > after {
		t.Fatalf("Mint timestamp %d outside [%d, %d]", ms, before, after)
	}
	if id != strings.ToLower(id) || len(id) != 36 {
		t.Fatalf("Mint = %q, want canonical lowercase 36-char form", id)
	}
	if other := Mint(); other == id {
		t.Fatalf("two Mint calls returned the same id %q", id)
	}
}

// Oracle: the spec formula restated from #1949 section 2 with raw
// crypto/sha256 — not via gen — so a drift in either the helper or
// the derivation root breaks this test.
func TestShuffleMatchesSpecFormula(t *testing.T) {
	for _, id := range []string{"01a0f101-2378-7c1c-9dc1-5a1ae1571eca", "replay-seed_1.2"} {
		rootH := sha256.New()
		rootH.Write([]byte("mcpx-test-seed/v1\x00"))
		rootH.Write([]byte(id))
		root := rootH.Sum(nil)
		h := sha256.Sum256(append(root, []byte("shuffle")...))
		want := int64(binary.BigEndian.Uint64(h[:8]) & (1<<63 - 1))
		if got := Shuffle(id); got != want {
			t.Fatalf("Shuffle(%q) = %d, want %d per #1949 section 2", id, got, want)
		}
		if got := Shuffle(id); got < 0 {
			t.Fatalf("Shuffle(%q) = %d, must be a non-negative int63 for -shuffle", id, got)
		}
	}
	if Shuffle("seed-a") == Shuffle("seed-b") {
		t.Fatal("distinct seeds derived the same shuffle value")
	}
}

// Oracle: gen.Stream is the landed derivation (#1956); the helper's
// Source must be that exact stream for (seed, t.Name(), gen@ver).
func TestSourceIsGenStreamForTest(t *testing.T) {
	const id = "01a0f101-2378-7c1c-9dc1-5a1ae1571eca"
	t.Setenv(EnvVar, id)
	got := Source(t, "widget", 3)
	want := gen.Stream(id, t.Name(), "widget", 3)
	for i := 0; i < 8; i++ {
		if g, w := got.Uint64(), want.Uint64(); g != w {
			t.Fatalf("draw %d: Source = %d, gen.Stream = %d", i, g, w)
		}
	}
}

func TestValidIDs(t *testing.T) {
	for _, id := range []string{"01a0f101-2378-7c1c-9dc1-5a1ae1571eca", "a", "Seed_1.2-3", strings.Repeat("x", 128)} {
		if !Valid(id) {
			t.Errorf("Valid(%q) = false, want true", id)
		}
	}
	for _, id := range []string{"", "has space", "slash/sep", "plus+sign", strings.Repeat("x", 129)} {
		if Valid(id) {
			t.Errorf("Valid(%q) = true, want false", id)
		}
	}
}

func TestResolveFrom(t *testing.T) {
	env := func(m map[string]string) func(string) string {
		return func(k string) string { return m[k] }
	}
	// Set + valid: used as-is, not minted.
	id, minted, err := resolveFrom(env(map[string]string{EnvVar: "replay-me_1"}))
	if err != nil || minted || id != "replay-me_1" {
		t.Fatalf("set: got (%q, %v, %v)", id, minted, err)
	}
	// Set + invalid: error, never a silent replacement.
	if _, _, err := resolveFrom(env(map[string]string{EnvVar: "bad seed"})); err == nil {
		t.Fatal("invalid id: want error")
	}
	// Unset in CI: fail closed, both signals.
	for _, m := range []map[string]string{{"CI": "true"}, {"GITHUB_ACTIONS": "true"}} {
		if _, _, err := resolveFrom(env(m)); err == nil {
			t.Fatalf("CI %v: want fail-closed error", m)
		}
	}
	// Unset locally: minted, and the minted id is a valid v7.
	id, minted, err = resolveFrom(env(map[string]string{}))
	if err != nil || !minted {
		t.Fatalf("local: got (%q, %v, %v)", id, minted, err)
	}
	if _, err := seedstore.Timestamp(id); err != nil {
		t.Fatalf("locally minted id %q is not UUIDv7: %v", id, err)
	}
	// CI detection is exact: CI=false is a local run.
	if _, minted, err := resolveFrom(env(map[string]string{"CI": "false"})); err != nil || !minted {
		t.Fatalf("CI=false: got minted=%v err=%v, want a local mint", minted, err)
	}
	// A set seed wins even in CI (that is the seed job's whole job).
	id, minted, err = resolveFrom(env(map[string]string{EnvVar: "ci-seed", "CI": "true"}))
	if err != nil || minted || id != "ci-seed" {
		t.Fatalf("CI with seed: got (%q, %v, %v)", id, minted, err)
	}
}
