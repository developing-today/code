// Package seedstore validates the mcpx run-seed store (#1953):
// pkgs/mcpx/testdata/seeds/v1, one compact-JSON file per seed, filed
// by the month of the date embedded in the seed's UUIDv7 timestamp.
//
// The store is written only by the seed collector workflow and read
// by replay tooling later; this package is the schema's enforcement
// point, so a corrupt record fails the guard test in CI instead of
// failing a replay months after the run's logs have expired.
package seedstore

import (
	"encoding/hex"
	"encoding/json"
	"fmt"
	"regexp"
	"strings"
	"time"
)

// Record is one stored seed: the file
// testdata/seeds/v1/<yyyy>/<mm>/<seed>.json, as one line of compact
// JSON. Field order in the file is the struct order (the collector
// writes them in this order too).
type Record struct {
	Seed       string `json:"seed"`
	Commit     string `json:"commit"`
	Ref        string `json:"ref"`
	Event      string `json:"event"`
	Run        string `json:"run"`
	Attempt    int    `json:"attempt"`
	Result     string `json:"result"`
	Derivation string `json:"derivation"`
	// Generators is the sha256 of the generator name@version manifest
	// at Commit (#1958); empty until generator manifests land.
	Generators string `json:"generators"`
	Date       string `json:"date"`
}

// results are the GitHub Actions run conclusions a record may carry.
var results = map[string]bool{
	"success": true, "failure": true, "cancelled": true,
	"skipped": true, "timed_out": true, "action_required": true,
	"neutral": true, "startup_failure": true, "stale": true,
}

var generatorsHash = regexp.MustCompile(`^sha256:[0-9a-f]{64}$`)

// Timestamp returns the minting time embedded in a UUIDv7 seed id:
// the first 48 bits are Unix milliseconds. The id must be a
// well-formed version-7 UUID (variant bits 10xx included).
func Timestamp(seed string) (time.Time, error) {
	h := strings.ReplaceAll(seed, "-", "")
	if len(h) != 32 || seed != fmt.Sprintf("%s-%s-%s-%s-%s", h[0:8], h[8:12], h[12:16], h[16:20], h[20:32]) {
		return time.Time{}, fmt.Errorf("seed %q is not a canonical UUID", seed)
	}
	raw, err := hex.DecodeString(h)
	if err != nil {
		return time.Time{}, fmt.Errorf("seed %q is not hex: %w", seed, err)
	}
	if raw[6]>>4 != 0x7 {
		return time.Time{}, fmt.Errorf("seed %q is not version 7", seed)
	}
	if raw[8]>>6 != 0b10 {
		return time.Time{}, fmt.Errorf("seed %q does not carry the RFC variant bits", seed)
	}
	ms := int64(raw[0])<<40 | int64(raw[1])<<32 | int64(raw[2])<<24 |
		int64(raw[3])<<16 | int64(raw[4])<<8 | int64(raw[5])
	return time.UnixMilli(ms).UTC(), nil
}

// Validate checks one record against the store's rules, including the
// two cross-checks that make the store self-describing: the filename
// is the seed id, and the record's date is the UTC date of the
// timestamp embedded in that id.
func Validate(rec Record, filename string) error {
	ts, err := Timestamp(rec.Seed)
	if err != nil {
		return err
	}
	if filename != rec.Seed+".json" {
		return fmt.Errorf("filename %q does not match seed id %q", filename, rec.Seed)
	}
	if want := ts.Format("2006-01-02"); rec.Date != want {
		return fmt.Errorf("record date %q does not match the date %q embedded in seed %s", rec.Date, want, rec.Seed)
	}
	if len(rec.Commit) != 40 {
		return fmt.Errorf("commit %q is not a full SHA", rec.Commit)
	}
	if _, err := hex.DecodeString(rec.Commit); err != nil {
		return fmt.Errorf("commit %q is not hex: %w", rec.Commit, err)
	}
	if rec.Ref == "" || rec.Event == "" || rec.Derivation == "" {
		return fmt.Errorf("record for seed %s leaves ref, event or derivation empty", rec.Seed)
	}
	if !strings.HasPrefix(rec.Run, "https://") || !strings.Contains(rec.Run, "/actions/runs/") {
		return fmt.Errorf("run %q is not an Actions run URL", rec.Run)
	}
	if rec.Attempt < 1 {
		return fmt.Errorf("attempt %d is not a run attempt number", rec.Attempt)
	}
	if !results[rec.Result] {
		return fmt.Errorf("result %q is not an Actions run conclusion", rec.Result)
	}
	if rec.Generators != "" && !generatorsHash.MatchString(rec.Generators) {
		return fmt.Errorf("generators %q is not a sha256: manifest hash", rec.Generators)
	}
	return nil
}

// Parse decodes one stored record file's contents.
func Parse(contents []byte) (Record, error) {
	var rec Record
	if err := json.Unmarshal(contents, &rec); err != nil {
		return Record{}, fmt.Errorf("seed record does not parse: %w", err)
	}
	return rec, nil
}
