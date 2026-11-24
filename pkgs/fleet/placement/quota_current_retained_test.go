package placement

import (
	"os"
	"strings"
	"testing"
	"time"
)

// TestReadQuotaCurrentRetainedEntries pins the retained-entry rule of
// issue #2732's wiring slice: when a gatherer tick fails, markRetained
// (pm-stack/aggregator/src/quota-gather-map.ts) carries the previous
// entry over with state "ok" plus a retainedFromFailedTick stamp. The
// aggregator's own reader classifies such an entry as stale by
// definition (#1268 in quota-current.ts entryState) — the provider's
// latest run failed, however recent the retained measuredAt is. This
// reader must agree: a retained entry never supplies a number and never
// shadows an older genuine ok read, so the seating path cannot silently
// proceed on failed-tick data.
func TestReadQuotaCurrentRetainedEntries(t *testing.T) {
	now := time.Date(2026, 11, 23, 16, 0, 0, 0, time.UTC)
	maxAge := 60 * time.Minute

	retained := func(serviceID, measuredAt string, pct string) string {
		return `{"providerId":"` + serviceID + `","providerName":"` + serviceID + `","serviceId":"` + serviceID + `","state":"ok","measuredAt":"` + measuredAt + `","source":"api","retainedFromFailedTick":"2026-11-23T15:55:00Z","windows":[{"name":"monthly","value":null,"pctOfLimit":` + pct + `,"valueLabel":null,"windowSeconds":null,"resetsAt":null,"measuredAt":"` + measuredAt + `","source":"api","state":"ok"}]}`
	}
	genuine := func(serviceID, measuredAt string, pct string) string {
		return `{"providerId":"` + serviceID + `","providerName":"` + serviceID + `","serviceId":"` + serviceID + `","state":"ok","measuredAt":"` + measuredAt + `","source":"api","windows":[{"name":"monthly","value":null,"pctOfLimit":` + pct + `,"valueLabel":null,"windowSeconds":null,"resetsAt":null,"measuredAt":"` + measuredAt + `","source":"api","state":"ok"}]}`
	}
	writeFile := func(t *testing.T, entries string) string {
		t.Helper()
		path := t.TempDir() + "/quota-current.json"
		doc := `{"version":1,"generatedAt":"2026-11-23T16:00:00Z","source":"gatherer","providers":[` + entries + `]}`
		if err := os.WriteFile(path, []byte(doc), 0o600); err != nil {
			t.Fatalf("write fixture: %v", err)
		}
		return path
	}

	t.Run("sole retained entry is stale, never a number", func(t *testing.T) {
		path := writeFile(t, retained("chatgpt-plan", "2026-11-23T15:50:00Z", "98.4"))
		_, _, err := ReadQuotaCurrent(path, "chatgpt-plan", now, maxAge)
		if err == nil {
			t.Fatal("expected a stale-read error for a read retained after a failed tick")
		}
		if !strings.Contains(err.Error(), "stale") {
			t.Fatalf("retained-read error %q does not classify the read as stale", err)
		}
	})

	t.Run("retained entry does not shadow an older genuine ok read", func(t *testing.T) {
		entries := genuine("chatgpt-plan", "2026-11-23T15:30:00Z", "50.0") + "," +
			retained("chatgpt-plan", "2026-11-23T15:50:00Z", "98.4")
		path := writeFile(t, entries)
		pct, readAt, err := ReadQuotaCurrent(path, "chatgpt-plan", now, maxAge)
		if err != nil {
			t.Fatalf("ReadQuotaCurrent: %v", err)
		}
		if pct != 50.0 {
			t.Fatalf("pct = %v, want the genuine read's 50.0, not the retained 98.4", pct)
		}
		if got := readAt.UTC().Format(time.RFC3339); got != "2026-11-23T15:30:00Z" {
			t.Fatalf("readAt = %s, want the genuine read's 2026-11-23T15:30:00Z", got)
		}
	})
}
