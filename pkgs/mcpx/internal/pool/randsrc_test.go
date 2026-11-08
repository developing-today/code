package pool

import (
	"crypto/rand"
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

// The production default must stay crypto/rand: these ids are
// access control, and a silent swap of the default would make them
// guessable (#2640).
func TestRandSourceDefaultIsCryptoRand(t *testing.T) {
	if RandSource != rand.Reader {
		t.Fatal("production default RandSource is not crypto/rand")
	}
}

// Two runs seeded identically mint identical ids: the #2640
// acceptance test at the package level.
func TestNewTraceIDSeededReproducesIDs(t *testing.T) {
	swapRandSource(t, randsrc.Seeded([]byte("run-seed"), "pool"))
	first := newTraceID("srv")
	swapRandSource(t, randsrc.Seeded([]byte("run-seed"), "pool"))
	if second := newTraceID("srv"); second != first {
		t.Fatalf("same seed minted different ids: %q vs %q", first, second)
	}
	swapRandSource(t, randsrc.Seeded([]byte("other-seed"), "pool"))
	if third := newTraceID("srv"); third == first {
		t.Fatalf("different seed minted the same id %q", first)
	}
}
