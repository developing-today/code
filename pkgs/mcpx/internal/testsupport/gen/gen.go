// Package gen is the generator core for mcpx's randomized tests
// (#1956, Approach C): hand-rolled generators drawing from one
// run-wide seed, behind a tiny Source interface, with math/rand/v2's
// ChaCha8 as the stream.
//
// Why hand-rolled (the issue's findings, briefly): native fuzz cannot
// be seeded from the run seed; testing/quick is frozen and draws from
// math/rand v1; rapid couples its output to its own version and seeds
// process-globally, which breaks per-test substreams under
// t.Parallel. ChaCha8's stream is specified (chacha8rand, C2SP), so
// it is the one part whose output is stable across Go releases —
// generators here draw ONLY Uint64() from it and do their own range
// reduction, keeping the coupling surface exactly that stream.
//
// Derivation (#1949 section 2, version v1):
//
//	root          = SHA-256("mcpx-test-seed/v1\x00" || seedID)
//	stream(t, g@v) = SHA-256(root || 0x00 || "g@v" || 0x00 || t) -> ChaCha8 key
//
// Adding, removing or reordering tests never shifts another test's
// inputs (streams are keyed by test name, not draw order), and
// changing one generator's version never shifts another's.
package gen

import (
	"crypto/sha256"
	"fmt"
	"math/bits"
	rand "math/rand/v2"
	"testing"
)

// Source is the one draw every generator makes. Implementations are
// deterministic: the same construction always yields the same stream.
type Source interface {
	Uint64() uint64
}

// Root hashes a run seed id into the derivation root. The seed id is
// never used as a key directly (#1949): it is an identifier (a UUIDv7
// in CI), the root is the key material.
func Root(seed string) [32]byte {
	h := sha256.New()
	h.Write([]byte("mcpx-test-seed/v1\x00"))
	h.Write([]byte(seed))
	var root [32]byte
	copy(root[:], h.Sum(nil))
	return root
}

// StreamKey derives one stream's ChaCha8 key: the stream of test
// testName using generator name@version under the given run seed.
func StreamKey(seed, testName, name string, version int) [32]byte {
	root := Root(seed)
	h := sha256.New()
	h.Write(root[:])
	h.Write([]byte{0})
	h.Write([]byte(fmt.Sprintf("%s@%d", name, version)))
	h.Write([]byte{0})
	h.Write([]byte(testName))
	var key [32]byte
	copy(key[:], h.Sum(nil))
	return key
}

// chaChaSource is a Source over one derived ChaCha8 stream.
type chaChaSource struct{ stream *rand.ChaCha8 }

func (s *chaChaSource) Uint64() uint64 { return s.stream.Uint64() }

// Stream returns the Source for one (test, generator@version) pair
// under a run seed. Same arguments, same stream, on every machine and
// every Go release (ChaCha8's stream is specified).
func Stream(seed, testName, name string, version int) Source {
	return &chaChaSource{stream: rand.NewChaCha8(StreamKey(seed, testName, name, version))}
}

// ForTest is Stream keyed by the calling test's own name — the common
// case, and the reason adding a test elsewhere cannot shift this
// test's inputs.
func ForTest(t *testing.T, seed, name string, version int) Source {
	t.Helper()
	return Stream(seed, t.Name(), name, version)
}

// BytesSource adapts a byte slice to a Source: Uint64s are read
// big-endian, eight bytes at a time, and past the end of the slice
// the source reads zeros (it never panics and never wraps). This is
// the fuzz adapter: a native fuzz target's []byte becomes a Source,
// so any generator in this package can be driven by fuzz data — and,
// through the seed corpus, by plain go test.
type BytesSource struct {
	b   []byte
	off int
}

// NewBytesSource returns a Source reading from b.
func NewBytesSource(b []byte) *BytesSource { return &BytesSource{b: b} }

func (s *BytesSource) Uint64() uint64 {
	var v uint64
	for i := 0; i < 8; i++ {
		v <<= 8
		if s.off < len(s.b) {
			v |= uint64(s.b[s.off])
			s.off++
		}
	}
	return v
}

// Bytes draws n bytes from src, big-endian per Uint64. Bytes over a
// BytesSource round-trips: Bytes(NewBytesSource(b), len(b)) equals b.
func Bytes(src Source, n int) []byte {
	if n <= 0 {
		return nil
	}
	out := make([]byte, 0, n)
	for len(out) < n {
		v := src.Uint64()
		for shift := 56; shift >= 0 && len(out) < n; shift -= 8 {
			out = append(out, byte(v>>uint(shift)))
		}
	}
	return out
}

// Intn draws a value in [0, n) by Lemire reduction of one Uint64 —
// the generator-side range reduction #1949 asks for, not a Rand
// method (whose distribution Go does not promise to keep stable).
// A non-positive n draws nothing and returns 0.
func Intn(src Source, n int) int {
	if n <= 0 {
		return 0
	}
	hi, _ := bits.Mul64(src.Uint64(), uint64(n))
	return int(hi)
}

// Sizes is the size-growing runner's schedule: input sizes doubling
// from 0 up to max. Growth small-to-large is the shrinking
// substitute: when a size fails, every smaller size has already
// passed, so the failing size is within a factor of two of the
// smallest failing one — report it and the case is already small.
func Sizes(max int) []int {
	if max < 0 {
		max = 0
	}
	out := []int{0}
	for n := 1; n < max; n *= 2 {
		out = append(out, n)
	}
	if out[len(out)-1] != max {
		out = append(out, max)
	}
	return out
}

// Run drives fn with generated byte inputs of growing size (see
// Sizes), drawing from src. The first failing size stops the run and
// is named in the failure.
func Run(t *testing.T, src Source, maxSize int, fn func(b []byte) error) {
	t.Helper()
	for _, size := range Sizes(maxSize) {
		if err := fn(Bytes(src, size)); err != nil {
			t.Fatalf("gen: size %d: %v (sizes grow small-to-large, so this is within 2x of the smallest failing size)", size, err)
		}
	}
}
