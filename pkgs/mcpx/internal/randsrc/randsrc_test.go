package randsrc_test

import (
	"bytes"
	"crypto/rand"
	"io"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/randsrc"
)

func readN(t *testing.T, s randsrc.Source, n int) []byte {
	t.Helper()
	b := make([]byte, n)
	if _, err := io.ReadFull(s, b); err != nil {
		t.Fatalf("read: %v", err)
	}
	return b
}

// The acceptance test for #2640 at the source level: two sources
// built from the same seed produce the same stream.
func TestSeededSameSeedSameStream(t *testing.T) {
	a := readN(t, randsrc.Seeded([]byte("run-seed"), "pkg"), 64)
	b := readN(t, randsrc.Seeded([]byte("run-seed"), "pkg"), 64)
	if !bytes.Equal(a, b) {
		t.Fatalf("same seed+label streams differ:\n%x\n%x", a, b)
	}
}

func TestSeededSeedAndLabelSeparateStreams(t *testing.T) {
	base := readN(t, randsrc.Seeded([]byte("run-seed"), "pkg"), 32)
	otherSeed := readN(t, randsrc.Seeded([]byte("other-seed"), "pkg"), 32)
	otherLabel := readN(t, randsrc.Seeded([]byte("run-seed"), "other"), 32)
	if bytes.Equal(base, otherSeed) {
		t.Fatal("different seeds produced the same stream")
	}
	if bytes.Equal(base, otherLabel) {
		t.Fatal("different labels produced the same stream")
	}
}

func TestSeededReadSizesSliceOneStream(t *testing.T) {
	whole := readN(t, randsrc.Seeded([]byte("run-seed"), "pkg"), 48)
	s := randsrc.Seeded([]byte("run-seed"), "pkg")
	var chunked []byte
	for _, n := range []int{1, 7, 8, 31, 1} {
		chunked = append(chunked, readN(t, s, n)...)
	}
	if !bytes.Equal(whole, chunked) {
		t.Fatalf("chunked reads diverged from one read:\n%x\n%x", whole, chunked)
	}
}

func TestCryptoIsCryptoRand(t *testing.T) {
	if randsrc.Crypto != rand.Reader {
		t.Fatal("the production source is not crypto/rand")
	}
}
