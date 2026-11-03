package logging

import (
	"crypto/rand"
	"errors"
	"io"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/randsrc"
)

func swapRandSource(t *testing.T, s io.Reader) {
	t.Helper()
	old := RandSource
	RandSource = s
	t.Cleanup(func() { RandSource = old })
}

func TestRandSourceDefaultIsCryptoRand(t *testing.T) {
	if RandSource != rand.Reader {
		t.Fatal("production default RandSource is not crypto/rand")
	}
}

type errReader struct{}

func (errReader) Read([]byte) (int, error) { return 0, errors.New("no randomness") }

// Two runs seeded identically mint identical ids: the #2640
// acceptance test at the package level.
func TestNewTraceIDSeededReproducesIDs(t *testing.T) {
	swapRandSource(t, randsrc.Seeded([]byte("run-seed"), "logging"))
	first := NewTraceID("trc")
	swapRandSource(t, randsrc.Seeded([]byte("run-seed"), "logging"))
	if second := NewTraceID("trc"); second != first {
		t.Fatalf("same seed minted different ids: %q vs %q", first, second)
	}
	swapRandSource(t, randsrc.Seeded([]byte("other-seed"), "logging"))
	if third := NewTraceID("trc"); third == first {
		t.Fatalf("different seed minted the same id %q", first)
	}
}

// A failing source keeps the documented fallback: logging must not
// stop because randomness did.
func TestNewTraceIDFailingSourceFallsBack(t *testing.T) {
	swapRandSource(t, errReader{})
	if got := NewTraceID("trc"); got != "trc-0000000000000000" {
		t.Fatalf("failing source: got %q, want the zero fallback", got)
	}
}
