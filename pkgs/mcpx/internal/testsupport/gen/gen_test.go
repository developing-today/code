package gen_test

import (
	"bytes"
	"fmt"
	"testing"

	"github.com/dezren39/mcpx/internal/testsupport/gen"
)

// goldenSeed is a fixed test vector. It pins the GENERATOR, not a
// test of mcpx: the same seed in a real run is minted fresh per run
// (#1949), never this constant.
const goldenSeed = "01a18b4a-e218-708e-9085-29de6dff0f9c"

// TestGoldenStreamStability is the "avoid changing how seed
// generates data" guard (#1949 section 3, #1956): the exact output
// sequence of one stream is pinned below. If this test fails, the
// stream's output changed — which is only ever legal as a new
// generator VERSION (or a derivation version), never as a silent
// edit: stored seeds replay by these numbers. Bump the version,
// update the golden under the new version, and leave the old stream
// reproducible.
func TestGoldenStreamStability(t *testing.T) {
	want := []uint64{
		0xc949d5314505e5da,
		0x530b11e618fa28dd,
		0xbfc0257c49db0c05,
		0xfda4b6fcb1234da4,
		0xd9f534b8b6fe4981,
		0xfa8b21439d027cad,
	}
	src := gen.Stream(goldenSeed, "golden", "bytes", 1)
	for i, w := range want {
		if got := src.Uint64(); got != w {
			t.Fatalf("draw %d = %#016x, want %#016x: the bytes@1 stream changed; that is a new version, not an edit", i, got, w)
		}
	}
	// Bytes emits the same stream big-endian.
	wantBytes := []byte{0xc9, 0x49, 0xd5, 0x31, 0x45, 0x05, 0xe5, 0xda, 0x53, 0x0b, 0x11, 0xe6, 0x18, 0xfa, 0x28, 0xdd}
	if got := gen.Bytes(gen.Stream(goldenSeed, "golden", "bytes", 1), 16); !bytes.Equal(got, wantBytes) {
		t.Fatalf("Bytes = % x, want % x", got, wantBytes)
	}
}

// TestStreamsAreIndependent: a different test name, generator or
// version is a different stream; the same triple is the same stream.
func TestStreamsAreIndependent(t *testing.T) {
	first := func(src gen.Source) uint64 { return src.Uint64() }
	base := first(gen.Stream(goldenSeed, "golden", "bytes", 1))
	if got := first(gen.Stream(goldenSeed, "golden", "bytes", 1)); got != base {
		t.Fatal("the same stream key drew a different first value")
	}
	for _, other := range []gen.Source{
		gen.Stream(goldenSeed, "other-test", "bytes", 1),
		gen.Stream(goldenSeed, "golden", "ints", 1),
		gen.Stream(goldenSeed, "golden", "bytes", 2),
		gen.Stream("another-seed", "golden", "bytes", 1),
	} {
		if got := first(other); got == base {
			t.Fatal("a different stream key drew the same first value")
		}
	}
}

// TestBytesSourceReadsZerosPastTheEnd pins the fuzz adapter's
// contract: big-endian draws, zero padding, no panic, no wrap.
func TestBytesSourceReadsZerosPastTheEnd(t *testing.T) {
	src := gen.NewBytesSource([]byte{0x01, 0x02, 0x03})
	if got := src.Uint64(); got != 0x0102030000000000 {
		t.Fatalf("first draw = %#016x, want the padded big-endian read", got)
	}
	if got := src.Uint64(); got != 0 {
		t.Fatalf("draw past the end = %#016x, want 0", got)
	}
	// Round-trip: Bytes over a BytesSource reproduces the input.
	in := []byte("mcpx seed corpus input")
	if got := gen.Bytes(gen.NewBytesSource(in), len(in)); !bytes.Equal(got, in) {
		t.Fatalf("round trip = %q, want %q", got, in)
	}
}

func TestIntnAndSizes(t *testing.T) {
	src := gen.Stream(goldenSeed, "golden", "ints", 1)
	for i := 0; i < 1000; i++ {
		if v := gen.Intn(src, 7); v < 0 || v >= 7 {
			t.Fatalf("Intn(7) = %d, out of range", v)
		}
	}
	if got := gen.Intn(src, 0); got != 0 {
		t.Fatalf("Intn(0) = %d, want 0", got)
	}
	want := []int{0, 1, 2, 4, 8, 16, 20}
	got := gen.Sizes(20)
	if fmt.Sprint(got) != fmt.Sprint(want) {
		t.Fatalf("Sizes(20) = %v, want %v", got, want)
	}
}

// TestRunGrowsAndReports exercises the runner's happy path; its
// failure path is the Fatalf in Run itself, exercised whenever a
// consumer's property breaks.
func TestRunGrowsAndReports(t *testing.T) {
	var sizes []int
	gen.Run(t, gen.Stream(goldenSeed, "golden", "bytes", 1), 8, func(b []byte) error {
		sizes = append(sizes, len(b))
		return nil
	})
	if fmt.Sprint(sizes) != fmt.Sprint(gen.Sizes(8)) {
		t.Fatalf("runner used sizes %v, want the Sizes schedule %v", sizes, gen.Sizes(8))
	}
}
