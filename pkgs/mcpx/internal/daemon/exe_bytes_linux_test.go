package daemon

import (
	"fmt"
	"os"
	"testing"
)

// Linux: /proc/<pid>/exe is the inode the child is actually executing,
// even after that path has been replaced on disk.
func exeBytes(t *testing.T, pid int) []byte {
	t.Helper()
	b, err := os.ReadFile(fmt.Sprintf("/proc/%d/exe", pid))
	if err != nil {
		t.Fatalf("read /proc/%d/exe: %v", pid, err)
	}
	return b
}
