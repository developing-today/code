package execsvc

import (
	"strings"
	"testing"
)

// CODE-16 (#1937): returned stdout is capped at exec.maxOutputBytes
// with a visible marker; under-cap output is byte-identical.

func TestTruncateStdoutOverCap(t *testing.T) {
	full := strings.Repeat("a", 1000)
	out, truncated := truncateStdout(full, 200)
	if !truncated {
		t.Fatal("expected truncated=true")
	}
	if len(out) > 200 {
		t.Fatalf("capped output is %d bytes, over the 200-byte cap", len(out))
	}
	if !strings.HasPrefix(out, strings.Repeat("a", 50)) {
		t.Fatal("capped output does not keep the head of the original")
	}
	if !strings.Contains(out, "stdout truncated") {
		t.Fatalf("capped output carries no truncation marker: %q", out[len(out)-120:])
	}
	// The marker names how much was dropped: 1000 total, kept is
	// whatever prefix survived, dropped = 1000 - kept.
	if !strings.Contains(out, "of 1000 bytes") {
		t.Fatalf("marker does not name the original size: %q", out)
	}
}

func TestTruncateStdoutUnderCapUnchanged(t *testing.T) {
	in := "short answer\n"
	out, truncated := truncateStdout(in, 200)
	if truncated || out != in {
		t.Fatalf("under-cap output changed: truncated=%v out=%q", truncated, out)
	}
}

func TestTruncateStdoutExactCapUnchanged(t *testing.T) {
	in := strings.Repeat("b", 200)
	out, truncated := truncateStdout(in, 200)
	if truncated || out != in {
		t.Fatalf("exact-cap output changed: truncated=%v len=%d", truncated, len(out))
	}
}

func TestTruncateStdoutZeroLimitDisables(t *testing.T) {
	in := strings.Repeat("c", 5000)
	out, truncated := truncateStdout(in, 0)
	if truncated || out != in {
		t.Fatal("limit 0 must disable the cap")
	}
}

func TestTruncateStdoutSmallLimitHardBound(t *testing.T) {
	// A positive limit smaller than the marker itself is still a
	// hard bound (Codex P2 on PR 2979: limit 1 or 64 returned the
	// full 108-byte marker).
	full := strings.Repeat("a", 1000)
	for _, limit := range []int64{1, 10, 64, 100} {
		out, truncated := truncateStdout(full, limit)
		if !truncated {
			t.Fatalf("limit %d: expected truncated=true", limit)
		}
		if int64(len(out)) > limit {
			t.Fatalf("limit %d: capped output is %d bytes, over the cap", limit, len(out))
		}
	}
}

func TestTruncateStdoutInvalidByteInPrefix(t *testing.T) {
	// An invalid UTF-8 byte inside the retained prefix must not drag
	// the cut back to before it (Codex P2 on PR 2979: the old code
	// validated the whole shrinking prefix per dropped byte --
	// quadratic -- and silently dropped everything after the byte).
	in := strings.Repeat("a", 500) + "\xff" + strings.Repeat("b", 500)
	out, truncated := truncateStdout(in, 800)
	if !truncated {
		t.Fatal("expected truncated=true")
	}
	if len(out) > 800 {
		t.Fatalf("capped output is %d bytes, over the 800-byte cap", len(out))
	}
	if !strings.Contains(out, "stdout truncated") {
		t.Fatal("no marker in capped output")
	}
	if !strings.Contains(out, strings.Repeat("b", 50)) {
		t.Fatal("content after the invalid byte was dropped")
	}
}

func TestTruncateStdoutUTF8Boundary(t *testing.T) {
	in := strings.Repeat("é", 500) // 2 bytes each, 1000 bytes total
	out, truncated := truncateStdout(in, 201)
	if !truncated {
		t.Fatal("expected truncated=true")
	}
	if len(out) > 201 {
		t.Fatalf("capped output is %d bytes, over the 201-byte cap", len(out))
	}
	if !strings.Contains(out, "stdout truncated") {
		t.Fatal("no marker in capped output")
	}
}
