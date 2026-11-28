package cli

import (
	"bufio"
	"io"
)

// newStreamScanner reads an event stream line by line with lineMax
// (http.streamBufferMax) as a hard line ceiling (#11308).
// bufio.Scanner only consults its max when the buffer must grow past
// its initial capacity, so an initial buffer (http.streamBufferInit)
// larger than the max would silently raise the ceiling. The initial
// capacity is therefore clamped to the max: the ceiling always holds,
// and init only tunes how much is allocated up front.
func newStreamScanner(r io.Reader, initCap, lineMax int64) *bufio.Scanner {
	if initCap > lineMax {
		initCap = lineMax
	}
	if initCap < 0 {
		initCap = 0
	}
	sc := bufio.NewScanner(r)
	sc.Buffer(make([]byte, 0, initCap), int(lineMax))
	return sc
}
