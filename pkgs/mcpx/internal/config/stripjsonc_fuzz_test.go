package config_test

import (
	"bytes"
	"testing"

	"github.com/dezren39/mcpx/internal/config"
	"github.com/dezren39/mcpx/internal/testsupport/gen"
)

// FuzzStripJSONC is the proof target for the testsupport/gen fuzz
// adapter (#1956): the fuzzer's bytes become a gen.BytesSource, the
// generator draws the input from that source, and StripJSONC — the
// JSONC comment stripper every config load passes through — must
// never panic, never grow its input, and be deterministic. The seed
// corpus (f.Add below and testdata/fuzz/FuzzStripJSONC) runs as
// ordinary tests under plain go test ./...; a real -fuzz run is the
// soak job's business (#1985), not this target's.
func FuzzStripJSONC(f *testing.F) {
	f.Add([]byte(`{"a": 1} // trailing comment`))
	f.Add([]byte("/* block */ {\"url\": \"https://example.com//not-a-comment\"}"))
	f.Add([]byte("{\"s\": \"escaped \\\" quote // still a string\"} // real comment"))
	f.Add([]byte("no comments at all"))
	f.Add([]byte{0x00, 0x01, '/', '/', '\n', '"', 0xff})
	f.Fuzz(func(t *testing.T, data []byte) {
		// The input length comes from len(data), NOT a draw from the
		// source: drawing a size first (the old gen.Intn call) consumed
		// the first eight payload bytes as a length, so the seed corpus
		// never reached StripJSONC intact (Codex P2 on PR #2636).
		// Bytes over a BytesSource round-trips, so the full payload —
		// corpus seeds included — reaches the stripper unchanged.
		src := gen.NewBytesSource(data)
		input := gen.Bytes(src, len(data))
		out := config.StripJSONC(input)
		if len(out) > len(input) {
			t.Fatalf("StripJSONC grew its input: %d bytes in, %d out", len(input), len(out))
		}
		if again := config.StripJSONC(input); !bytes.Equal(again, out) {
			t.Fatal("StripJSONC is not deterministic on the same input")
		}
	})
}
