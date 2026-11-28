package e2e_test

import (
	"fmt"
	"os"
	"strings"
)

// exited reports whether a process is gone or a zombie; processExists alone
// counts a zombie as alive.
func exited(pid int) bool {
	b, err := os.ReadFile(fmt.Sprintf("/proc/%d/stat", pid))
	if err != nil {
		return true
	}
	i := strings.LastIndexByte(string(b), ')')
	return i+2 < len(b) && b[i+2] == 'Z'
}
