package cli

import (
	"bufio"
	"errors"
	"strings"
	"testing"
)

// #11308: with streamBufferInit > streamBufferMax, a line over the
// documented ceiling must still end the stream instead of passing.
func TestStreamScannerCeilingHoldsWhenInitExceedsMax(t *testing.T) {
	long := strings.Repeat("x", 100)
	for _, tc := range []struct {
		name         string
		initCap, max int64
	}{
		{"init below max", 16, 64},
		{"init above max", 4096, 64},
		{"init equal max", 64, 64},
	} {
		t.Run(tc.name, func(t *testing.T) {
			sc := newStreamScanner(strings.NewReader("data: ok\n"+long+"\n"), tc.initCap, tc.max)
			if !sc.Scan() || sc.Text() != "data: ok" {
				t.Fatalf("first line: got %q, err %v", sc.Text(), sc.Err())
			}
			if sc.Scan() {
				t.Fatalf("a %d-byte line passed a %d-byte ceiling", len(long), tc.max)
			}
			if !errors.Is(sc.Err(), bufio.ErrTooLong) {
				t.Fatalf("want bufio.ErrTooLong, got %v", sc.Err())
			}
		})
	}
}

func TestStreamScannerReadsLinesUnderTheCeiling(t *testing.T) {
	sc := newStreamScanner(strings.NewReader("a\nbb\nccc\n"), 4096, 64)
	var got []string
	for sc.Scan() {
		got = append(got, sc.Text())
	}
	if sc.Err() != nil || strings.Join(got, ",") != "a,bb,ccc" {
		t.Fatalf("got %v, err %v", got, sc.Err())
	}
}
