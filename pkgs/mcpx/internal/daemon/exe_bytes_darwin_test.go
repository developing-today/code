package daemon

import (
	"bytes"
	"encoding/binary"
	"fmt"
	"os"
	"testing"

	"golang.org/x/sys/unix"
)

// Darwin has no /proc. The closest equivalent is the child's exec path
// from kern.procargs2 (layout: int32 argc, then the NUL-terminated exec
// path, then argv). Reading that path gives the program the child was
// started from; the exec-change test replaces the file at the path
// before restarting, so the restarted child's path content is the new
// program — the same comparison the linux /proc read makes.
func exeBytes(t *testing.T, pid int) []byte {
	t.Helper()
	raw, err := unix.SysctlRaw(fmt.Sprintf("kern.procargs2.%d", pid))
	if err != nil {
		t.Fatalf("sysctl kern.procargs2.%d: %v", pid, err)
	}
	if len(raw) < 5 {
		t.Fatalf("kern.procargs2.%d: short reply (%d bytes)", pid, len(raw))
	}
	_ = binary.LittleEndian.Uint32(raw[:4]) // argc; the exec path follows it
	rest := raw[4:]
	end := bytes.IndexByte(rest, 0)
	if end <= 0 {
		t.Fatalf("kern.procargs2.%d: no exec path in reply", pid)
	}
	b, err := os.ReadFile(string(rest[:end]))
	if err != nil {
		t.Fatalf("read child exec path %q: %v", rest[:end], err)
	}
	return b
}
