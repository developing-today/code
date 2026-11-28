package main

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/developing-today/code/pkgs/fleet/placement"
)

func TestReadQuotaCurrentWithB4FallbackUsesFreshRead(t *testing.T) {
	now := time.Date(2026, 11, 23, 16, 0, 0, 0, time.UTC)
	path := writeQuotaFixture(t, `{"version":1,"providers":[{"serviceId":"opencode-go.popesdrew","state":"ok","measuredAt":"2026-11-23T15:50:00Z","windows":[{"name":"monthly","pctOfLimit":50.0}]}]}`)
	pct, readAt, b4, err := readQuotaCurrentWithB4Fallback(path, "opencode-go.popesdrew", now, 60*time.Minute)
	if err != nil {
		t.Fatalf("readQuotaCurrentWithB4Fallback: %v", err)
	}
	if b4 || pct != 50.0 || !readAt.Equal(time.Date(2026, 11, 23, 15, 50, 0, 0, time.UTC)) {
		t.Fatalf("got pct=%v readAt=%s b4=%v; want fresh 50%% read", pct, readAt, b4)
	}
}

func TestReadQuotaCurrentWithB4FallbackUsesLastKnownAfterRetry(t *testing.T) {
	now := time.Date(2026, 11, 23, 16, 0, 0, 0, time.UTC)
	path := writeQuotaFixture(t, `{"version":1,"providers":[{"serviceId":"opencode-go.popesdrew","state":"stale","measuredAt":"2026-11-23T14:00:00Z","retainedFromFailedTick":"2026-11-23T15:45:00Z","windows":[{"name":"monthly","pctOfLimit":98.4}]}]}`)
	pct, readAt, b4, err := readQuotaCurrentWithB4Fallback(path, "opencode-go.popesdrew", now, 60*time.Minute)
	if err != nil {
		t.Fatalf("readQuotaCurrentWithB4Fallback: %v", err)
	}
	if !b4 || pct != 98.4 || !readAt.Equal(time.Date(2026, 11, 23, 14, 0, 0, 0, time.UTC)) {
		t.Fatalf("got pct=%v readAt=%s b4=%v; want B4 98.4%% last-known read", pct, readAt, b4)
	}
}

func TestReadQuotaCurrentWithB4FallbackFailsWithoutAnyLastKnownMeasurement(t *testing.T) {
	now := time.Date(2026, 11, 23, 16, 0, 0, 0, time.UTC)
	path := writeQuotaFixture(t, `{"version":1,"providers":[{"serviceId":"opencode-go.popesdrew","state":"error","measuredAt":"2026-11-23T15:55:00Z","windows":[]}]}`)
	_, _, b4, err := readQuotaCurrentWithB4Fallback(path, "opencode-go.popesdrew", now, 60*time.Minute)
	if err == nil || b4 || !strings.Contains(err.Error(), "no usable B4 last-known state") {
		t.Fatalf("got b4=%v err=%v; want no-last-known error", b4, err)
	}
}

// TestApplyB4FallbackRecordsReliance pins the JSON-record oracle: fallback
// input must carry its original read time and the B4 marker.
func TestApplyB4FallbackRecordsReliance(t *testing.T) {
	readAt := time.Date(2026, 11, 23, 14, 0, 0, 0, time.UTC)
	pct := 98.4
	var answers placement.Answers
	applyB4Fallback(&answers, pct, readAt, "opencode-go.popesdrew")
	if !answers.PoolReadUnobtainable {
		t.Fatal("B4 fallback marker was not set")
	}
	if answers.PoolMonthlyPct == nil || *answers.PoolMonthlyPct != pct {
		t.Fatalf("PoolMonthlyPct = %v, want %v", answers.PoolMonthlyPct, pct)
	}
	if answers.PoolReadAt == nil || !answers.PoolReadAt.Equal(readAt) {
		t.Fatalf("PoolReadAt = %v, want %s", answers.PoolReadAt, readAt)
	}
	if answers.PoolID != "opencode-go.popesdrew" {
		t.Fatalf("PoolID = %q, want account-specific pool", answers.PoolID)
	}
}

func writeQuotaFixture(t *testing.T, contents string) string {
	t.Helper()
	path := filepath.Join(t.TempDir(), "quota-current.json")
	if err := os.WriteFile(path, []byte(contents), 0o600); err != nil {
		t.Fatalf("write quota fixture: %v", err)
	}
	return path
}
