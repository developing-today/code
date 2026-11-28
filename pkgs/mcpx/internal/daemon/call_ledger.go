package daemon

import (
	"os"
	"path/filepath"
	"strings"
	"time"
)

// pruneCallLedger keeps eight calendar days of daily JSONL files. FileSink
// separately caps each active file and retains eight size/line/age rotations;
// together these bounds prevent old daily files from accumulating forever.
func pruneCallLedger(dir string, now time.Time) {
	entries, err := os.ReadDir(dir)
	if err != nil {
		return
	}
	today := time.Date(now.Year(), now.Month(), now.Day(), 0, 0, 0, 0, now.Location())
	cutoff := today.AddDate(0, 0, -7)
	for _, entry := range entries {
		name := entry.Name()
		if !strings.HasPrefix(name, "mcpx-") || !strings.HasSuffix(name, ".jsonl") {
			continue
		}
		stem := strings.TrimSuffix(strings.TrimPrefix(name, "mcpx-"), ".jsonl")
		if len(stem) < len("2006-01-02") {
			continue
		}
		day, err := time.Parse("2006-01-02", stem[:len("2006-01-02")])
		if err != nil || (len(stem) > len("2006-01-02") && stem[len("2006-01-02")] != '-') {
			continue
		}
		if day.Before(cutoff) {
			_ = os.Remove(filepath.Join(dir, name))
		}
	}
}
