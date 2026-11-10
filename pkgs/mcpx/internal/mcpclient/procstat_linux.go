package mcpclient

import (
	"bytes"
	"fmt"
	"os"
	"strconv"
	"strings"
)

// procStat reads a process's start time and state from /proc/<pid>/stat.
// The start time is in clock ticks since boot.
func procStat(pid int) (uint64, byte, error) {
	raw, err := os.ReadFile("/proc/" + strconv.Itoa(pid) + "/stat")
	if err != nil {
		return 0, 0, err
	}
	// The command name is parenthesised and may contain spaces or parens,
	// so fields are counted from the last ')'.
	i := bytes.LastIndexByte(raw, ')')
	if i < 0 {
		return 0, 0, fmt.Errorf("unparseable /proc/%d/stat", pid)
	}
	fields := strings.Fields(string(raw[i+1:]))
	// fields[0] is field 3 (state); starttime is field 22.
	if len(fields) < 20 || len(fields[0]) != 1 {
		return 0, 0, fmt.Errorf("unparseable /proc/%d/stat", pid)
	}
	start, err := strconv.ParseUint(fields[19], 10, 64)
	if err != nil {
		return 0, 0, fmt.Errorf("unparseable /proc/%d/stat: %w", pid, err)
	}
	return start, fields[0][0], nil
}
