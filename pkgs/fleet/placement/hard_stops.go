package placement

import (
	"encoding/json"
	"fmt"
	"os"
)

// ReadHardStops is the machine-readable steward hard-stop producer for
// issue #2732 proposal step 2: the seating path checks a steward-written
// hard-stops file instead of relying on dispatcher-log prose. It reports
// whether the named pool is under a steward hard-stop.
//
// File contract (version 1):
//
//	{"version":1,"stops":{"<serviceId>":{"stopped":true,"note":"...","issuedAt":"..."}}}
//
// A pool with no entry — or an entry with stopped false — is not
// stopped. A missing, unreadable, or malformed file is an error, never
// a silent "not stopped": a seating path that asked for hard-stop
// enforcement must fail loudly rather than seat unguarded.
func ReadHardStops(path, pool string) (bool, error) {
	raw, err := os.ReadFile(path)
	if err != nil {
		return false, fmt.Errorf("hard-stops: read %s: %w", path, err)
	}
	var doc hardStopsDoc
	if err := json.Unmarshal(raw, &doc); err != nil {
		return false, fmt.Errorf("hard-stops: parse %s: %w", path, err)
	}
	if doc.Version != 1 {
		return false, fmt.Errorf("hard-stops: %s: unsupported version %d, want 1", path, doc.Version)
	}
	entry, ok := doc.Stops[pool]
	if !ok {
		return false, nil
	}
	return entry.Stopped, nil
}

type hardStopsDoc struct {
	Version int                      `json:"version"`
	Stops   map[string]hardStopEntry `json:"stops"`
}

type hardStopEntry struct {
	Stopped  bool   `json:"stopped"`
	Note     string `json:"note,omitempty"`
	IssuedAt string `json:"issuedAt,omitempty"`
}
