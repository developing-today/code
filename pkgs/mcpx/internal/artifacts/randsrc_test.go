package artifacts

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
func TestNewIDSeededReproducesIDs(t *testing.T) {
	swapRandSource(t, randsrc.Seeded([]byte("run-seed"), "artifacts"))
	first := NewID()
	swapRandSource(t, randsrc.Seeded([]byte("run-seed"), "artifacts"))
	if second := NewID(); second != first {
		t.Fatalf("same seed minted different ids: %q vs %q", first, second)
	}
	swapRandSource(t, randsrc.Seeded([]byte("other-seed"), "artifacts"))
	if third := NewID(); third == first {
		t.Fatalf("different seed minted the same id %q", first)
	}
}
