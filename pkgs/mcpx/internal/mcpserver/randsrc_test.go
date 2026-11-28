package mcpserver

import (
	"bytes"
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

type errReader struct{}

func (errReader) Read([]byte) (int, error) { return 0, errors.New("no randomness") }

func TestRandSourceDefaultIsCryptoRand(t *testing.T) {
	if RandSource != rand.Reader {
		t.Fatal("production default RandSource is not crypto/rand")
	}
}

func TestNewSessionIDSeededReproducesIDs(t *testing.T) {
	swapRandSource(t, randsrc.Seeded([]byte("run-seed"), "mcpserver"))
	first := newSessionID()
	swapRandSource(t, randsrc.Seeded([]byte("run-seed"), "mcpserver"))
	if second := newSessionID(); second != first {
		t.Fatalf("same seed minted different session ids: %q vs %q", first, second)
	}
	swapRandSource(t, randsrc.Seeded([]byte("other-seed"), "mcpserver"))
	if third := newSessionID(); third == first {
		t.Fatalf("different seed minted the same session id %q", first)
	}
}

// A deterministic source never fails, so the sess-0 fallback needs
// its own injected failing reader (#2640 review brief).
func TestNewSessionIDFailingSourceFallsBack(t *testing.T) {
	swapRandSource(t, errReader{})
	if got := newSessionID(); got != "sess-0" {
		t.Fatalf("failing source: got %q, want sess-0", got)
	}
}

func TestStateSignerSeededKeyReproduces(t *testing.T) {
	swapRandSource(t, randsrc.Seeded([]byte("run-seed"), "mcpserver"))
	a := newStateSigner()
	swapRandSource(t, randsrc.Seeded([]byte("run-seed"), "mcpserver"))
	b := newStateSigner()
	if len(a.key) == 0 {
		t.Fatal("seeded signer has no key")
	}
	if !bytes.Equal(a.key, b.key) {
		t.Fatal("same seed produced different signer keys")
	}
}

func TestStateSignerFailingSourceHasNoKey(t *testing.T) {
	swapRandSource(t, errReader{})
	s := newStateSigner()
	if _, err := s.mint("call", "binding"); !errors.Is(err, ErrNoStateKey) {
		t.Fatalf("mint with no key: err = %v, want ErrNoStateKey", err)
	}
}
